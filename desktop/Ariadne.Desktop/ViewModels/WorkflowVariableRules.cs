using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 工作流变量的纯逻辑层：声明解析、空值判定、摘要句式渲染、启动载荷组装。
///
/// 不依赖 Avalonia，便于单测。判定规则必须与 core 侧
/// (`contracts/workflow.rs` 的 variable_value_is_blank / render_summary_template)
/// 保持一致——两侧不一致会让前端放行、后端拒绝，作者看到自相矛盾的结果。
/// </summary>
public static class WorkflowVariableRules
{
    /// <summary>变量类型；与 core 的 WorkflowVariableKind 同名同义。</summary>
    public const string KindNumber = "number";
    public const string KindString = "string";
    public const string KindBoolean = "boolean";

    /// <summary>
    /// 判定取值对 required 而言是否算空白。
    ///
    /// 只有 null 与空白字符串算空：0 与 false 是合法取值（第 0 章合法，
    /// 不润色也合法）。判据是「替换进 {{var.x}} 后会不会在稿子里留个洞」，
    /// 不是「值看起来重不重要」。
    /// </summary>
    public static bool IsBlank(object? value) => value switch
    {
        null => true,
        string text => string.IsNullOrWhiteSpace(text),
        _ => false,
    };

    /// <summary>
    /// 渲染摘要句式：把 {{var.名字}} 替换成当前取值。
    ///
    /// 与提示词渲染的关键差异：缺失取值**就地留空**而不是报错。折叠行是给人看的
    /// 预览，required 变量留空时那个空洞本身就是「稿子里会缺这块」的提示。
    /// 非 var. 命名空间原样保留，便于排查写错的句式。
    /// </summary>
    public static string RenderSummary(string? template, IReadOnlyDictionary<string, object?> values)
    {
        if (string.IsNullOrEmpty(template))
        {
            return string.Empty;
        }

        var rendered = new StringBuilder(template.Length);
        var index = 0;
        while (index < template.Length)
        {
            var open = template.IndexOf("{{", index, System.StringComparison.Ordinal);
            if (open < 0)
            {
                rendered.Append(template, index, template.Length - index);
                break;
            }

            rendered.Append(template, index, open - index);
            var close = template.IndexOf("}}", open + 2, System.StringComparison.Ordinal);
            if (close < 0)
            {
                // 未闭合：剩余按字面量输出，不吞掉作者写的内容。
                rendered.Append(template, open, template.Length - open);
                break;
            }

            var name = template[(open + 2)..close].Trim();
            if (name.StartsWith("var.", System.StringComparison.Ordinal))
            {
                var variable = name[4..].Trim();
                if (values.TryGetValue(variable, out var value))
                {
                    rendered.Append(FormatValue(value));
                }
                // 无取值：留空。
            }
            else
            {
                rendered.Append(template, open, close + 2 - open);
            }

            index = close + 2;
        }

        return rendered.ToString();
    }

    /// <summary>
    /// 取值的显示文本。字符串取原文（不带 JSON 引号，否则会渲染成《"雪落时"》），
    /// 布尔与数字取不受区域设置影响的固定写法。
    /// </summary>
    public static string FormatValue(object? value) => value switch
    {
        null => string.Empty,
        string text => text,
        bool flag => flag ? "true" : "false",
        double number => number.ToString("R", CultureInfo.InvariantCulture),
        _ => System.Convert.ToString(value, CultureInfo.InvariantCulture) ?? string.Empty,
    };

    /// <summary>
    /// 清洗 AI 生成的句式：剥掉代码块围栏、首尾引号、多余空行。
    ///
    /// 提示词已经说了「只回这一句话本身」，但模型仍常裹上 ```/「」/引号 ——
    /// 与其指望它每次都守规矩，不如在入口处收口。只取第一行非空内容：
    /// 句式本就是一句话，模型多写的解释一律丢弃。
    /// </summary>
    public static string CleanGeneratedSummary(string? answer)
    {
        if (string.IsNullOrWhiteSpace(answer))
        {
            return string.Empty;
        }

        var lines = answer.Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n');
        foreach (var raw in lines)
        {
            var line = raw.Trim();
            if (line.Length == 0 || line.StartsWith("```", StringComparison.Ordinal))
            {
                continue;
            }

            // 首尾成对的引号（含中日文引号）一律剥掉
            line = line.Trim('"', '\'', '「', '」', '『', '』', '“', '”', '‘', '’').Trim();
            if (line.Length > 0)
            {
                return line;
            }
        }

        return string.Empty;
    }

    /// <summary>
    /// 把输入框文本按声明类型转成后端期望的取值。
    ///
    /// 返回 false 表示这段文本不是该类型的合法值——此时不能静默塞个默认值，
    /// 那会让作者以为填进去了。
    /// </summary>
    public static bool TryParse(string kind, string? text, out object? value)
    {
        value = null;
        var trimmed = text?.Trim() ?? string.Empty;
        switch (kind)
        {
            case KindNumber:
                if (trimmed.Length == 0)
                {
                    return true; // 空 = 未填；由 required 校验决定能否启动
                }
                if (double.TryParse(trimmed, NumberStyles.Float, CultureInfo.InvariantCulture, out var number))
                {
                    value = number;
                    return true;
                }
                return false;

            case KindBoolean:
                if (trimmed.Length == 0)
                {
                    return true;
                }
                if (bool.TryParse(trimmed, out var flag))
                {
                    value = flag;
                    return true;
                }
                return false;

            default:
                // 字符串：空白也保留原样，交给 IsBlank 判定
                value = text ?? string.Empty;
                return true;
        }
    }
}
