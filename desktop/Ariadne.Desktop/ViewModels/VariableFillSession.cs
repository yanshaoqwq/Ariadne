using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;

namespace Ariadne.Desktop.ViewModels;

/// <summary>AI 提议的一条变量取值改动；新旧值都留着，diff 区要同时显示。</summary>
public sealed record VariableFillChange(string Name, string OldText, string NewText);

/// <summary>
/// 一次「AI 填变量值」的提议：捕获发起时的全部取值快照 + 解析出来的改动。
///
/// 与 <see cref="QuickEditSession"/> 同一套守卫思路（捕获快照 → 应用前比对 →
/// 不符则整体作废）。之所以不直接复用那个类：它绑的是「文档 id + 版本 + 正文 +
/// 选区」这四件事，变量这边一个都没有——硬套进去只能给每个字段塞假值，
/// 那样守卫就失效了（假值永远相等，等于没守）。共用的是**判据的形状**，不是字段。
/// </summary>
public sealed record VariableFillSession(
    IReadOnlyDictionary<string, string> Snapshot,
    IReadOnlyList<VariableFillChange> Changes)
{
    /// <summary>
    /// 快照是否仍与当前表单一致。
    ///
    /// 比整张表而不是只比将被改动的那几个变量：句式渲染出来的那句话由**所有**变量
    /// 决定，作者在等 AI 回话期间改了任何一个，他看到的 diff 就不再是他将得到的结果。
    /// </summary>
    public bool MatchesCurrent(IReadOnlyDictionary<string, string> current)
    {
        if (current.Count != Snapshot.Count)
        {
            return false;
        }

        foreach (var (name, text) in Snapshot)
        {
            if (!current.TryGetValue(name, out var live)
                || !string.Equals(live, text, StringComparison.Ordinal))
            {
                return false;
            }
        }

        return true;
    }

    /// <summary>
    /// diff 文本：一条改动渲染成「- 名字：旧值」「+ 名字：新值」两行。
    ///
    /// 刻意产出与后端 quick_edit 同前缀的文本，好让
    /// <see cref="QuickEditDiffLineViewModel"/> 原样解析、复用同一套着色——
    /// 差别只在这里的行是变量而不是正文行，视图层无需知道这件事。
    /// 旧值为空时写成占位符「（空）」：两行都空白读起来像渲染坏了。
    /// </summary>
    public string BuildDiffText(Func<string, string> blankPlaceholder)
    {
        var builder = new StringBuilder();
        foreach (var change in Changes)
        {
            var old = change.OldText.Length == 0 ? blankPlaceholder(change.Name) : change.OldText;
            var next = change.NewText.Length == 0 ? blankPlaceholder(change.Name) : change.NewText;
            builder.Append("- ").Append(change.Name).Append('：').Append(old).Append('\n');
            builder.Append("+ ").Append(change.Name).Append('：').Append(next).Append('\n');
        }

        return builder.ToString();
    }
}

/// <summary>
/// 撤销态：只在「应用后取值再没被动过」时允许还原，否则会盖掉作者后续的手改。
/// 与 <see cref="QuickEditUndoState"/> 同一条规则。
/// </summary>
public sealed record VariableFillUndoState(
    IReadOnlyDictionary<string, string> AppliedValues,
    IReadOnlyDictionary<string, string> PreviousValues)
{
    public bool CanUndo(IReadOnlyDictionary<string, string> current)
    {
        if (current.Count != AppliedValues.Count)
        {
            return false;
        }

        foreach (var (name, text) in AppliedValues)
        {
            if (!current.TryGetValue(name, out var live)
                || !string.Equals(live, text, StringComparison.Ordinal))
            {
                return false;
            }
        }

        return true;
    }
}

/// <summary>解析 AI 回复的结果：可用改动 + 被拒条目名（要让作者看见，不能静默丢）。</summary>
public sealed record VariableFillParseResult(
    IReadOnlyList<VariableFillChange> Changes,
    IReadOnlyList<string> RejectedNames);

/// <summary>
/// 「AI 填变量值」的纯逻辑层：组装请求正文、解析回复、判定可用改动。
///
/// 不依赖 Avalonia，便于单测；与 <see cref="WorkflowVariableRules"/> 同一条边界。
/// </summary>
public static class VariableFillProtocol
{
    /// <summary>
    /// 组装发给项目空间 AI 的消息：作者的指令 + 变量清单 + 回复格式约定。
    ///
    /// **回复格式约定刻意写在 C# 里，而不是 prompt_list.json**：它与下面的
    /// <see cref="Parse"/> 是一对咬合的齿轮，格式一改解析就静默失灵——
    /// 把它放进可编辑的提示词资源，等于把解析器的前提交给别处随手改。
    /// prompt_list.json 管的是「AI 该以什么身份、什么笔法说话」（如
    /// workflow.variable_summary），那类措辞才该可编辑。
    ///
    /// 作者的指令本身就是这次改写的提示词——与作品页快捷改写同一个位置、
    /// 同一个含义（说明框里写「下一章」，AI 就把 chapter 递增）。
    /// </summary>
    public static string BuildMessage(
        string instruction,
        IReadOnlyList<WorkflowVariableViewModel> variables)
    {
        var builder = new StringBuilder();
        builder.Append(instruction.Trim()).Append('\n').Append('\n');
        builder.Append("请据此给出这次运行要用的工作流变量取值。\n");
        builder.Append("变量清单（名字、类型、当前值）：\n");
        foreach (var variable in variables)
        {
            builder
                .Append("- ")
                .Append(variable.Name)
                .Append('（')
                .Append(variable.Kind)
                .Append("）当前值：")
                .Append(variable.Text.Length == 0 ? "（空）" : variable.Text)
                .Append('\n');
        }

        builder.Append('\n');
        builder.Append("回复格式：每行一个「变量名=取值」，只写需要改的变量，");
        builder.Append("不要解释、不要引号、不要代码块。布尔写 true/false，数字只写数字。\n");
        return builder.ToString();
    }

    /// <summary>
    /// 解析回复，逐条按声明类型校验。
    ///
    /// 三类条目都不静默吞掉：
    /// - 未声明的名字 / hidden 变量 → 记进 RejectedNames（hidden 不进表单，就不该被这条路改）
    /// - 类型不合法（number 收到「第三章」）→ 记进 RejectedNames，绝不塞进表单让作者以为填对了
    /// - 与当前值相同 → 不算改动，也不算被拒（否则 diff 里全是没变的行）
    /// </summary>
    public static VariableFillParseResult Parse(
        string? answer,
        IReadOnlyList<WorkflowVariableViewModel> variables)
    {
        var changes = new List<VariableFillChange>();
        var rejected = new List<string>();
        if (string.IsNullOrWhiteSpace(answer))
        {
            return new VariableFillParseResult(changes, rejected);
        }

        var byName = new Dictionary<string, WorkflowVariableViewModel>(StringComparer.Ordinal);
        foreach (var variable in variables)
        {
            byName[variable.Name] = variable;
        }

        var seen = new HashSet<string>(StringComparer.Ordinal);
        var lines = answer.Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n');
        foreach (var raw in lines)
        {
            var line = raw.Trim();
            // 代码块围栏与空行跳过：提示里说了不要，但模型仍常裹上（同 CleanGeneratedSummary 的取舍）。
            if (line.Length == 0 || line.StartsWith("```", StringComparison.Ordinal))
            {
                continue;
            }

            // 列表符号一并剥掉：模型很爱写成「- chapter=4」。
            line = line.TrimStart('-', '*', '·', ' ').Trim();
            var separator = line.IndexOfAny(new[] { '=', '：', ':' });
            if (separator <= 0)
            {
                continue;
            }

            var name = line[..separator].Trim();
            var value = line[(separator + 1)..].Trim().Trim('"', '\'', '「', '」', '“', '”');
            if (name.Length == 0 || !seen.Add(name))
            {
                // 同名重复只认第一条：后一条无从判断是修正还是幻觉，取先出现的更可预测。
                continue;
            }

            if (!byName.TryGetValue(name, out var variable))
            {
                rejected.Add(name);
                continue;
            }

            if (!WorkflowVariableRules.TryParse(variable.Kind, value, out _))
            {
                rejected.Add(name);
                continue;
            }

            if (string.Equals(variable.Text, value, StringComparison.Ordinal))
            {
                continue;
            }

            changes.Add(new VariableFillChange(name, variable.Text, value));
        }

        return new VariableFillParseResult(changes, rejected);
    }

    /// <summary>当前表单取值的快照（名字 → 文本），供守卫比对与撤销还原。</summary>
    public static Dictionary<string, string> CaptureValues(
        IReadOnlyList<WorkflowVariableViewModel> variables)
    {
        var snapshot = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var variable in variables)
        {
            snapshot[variable.Name] = variable.Text;
        }

        return snapshot;
    }
}
