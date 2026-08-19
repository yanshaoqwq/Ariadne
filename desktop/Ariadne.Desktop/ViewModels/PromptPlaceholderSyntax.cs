using System.Collections.Generic;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// U115：提示词模板里 `{{...}}` **变量占位符**的前端词法层。
///
/// # 与 <see cref="ContentReferenceSyntax"/> 的分工
///
/// 两者扫的是**同一对花括号**，但语义不同，所以必须分开：
/// - `{{ref:文档#L1-L2}}` 是**正文引用**——展开成小说原文，由 `ContentReferenceSyntax`
///   负责（它还要出定位串、版本锚定这些引用独有的信息）。
/// - `{{input.outline}}` / `{{角色设定}}` 是**变量占位符**——展开成运行期的值，
///   由本类负责。
///
/// 本类扫**全部** `{{...}}`（含引用），因为高亮层要的是「哪些字符是占位符」这一条，
/// 不区分种类就画不出「引用是一种颜色、变量是另一种」。种类由
/// <see cref="PlaceholderKind"/> 给出，引用那一类的细节仍去问 `ContentReferenceSyntax`。
///
/// # 为什么这里的校验只到「形状」，不到「值」
///
/// 权威解析在 `core/src/rag/prompt_template.rs` 的 `resolve_variable`。它能报
/// `{{input.outline}}` 未解析，是因为它手上有**运行期的 inputs**。编辑器里没有——
/// 用户还没运行，输入是什么要等连线与上游产出才知道。
///
/// 所以这里只判命名空间形状：`input.` / `system.` / `param.` / `var.` / `template.`
/// 以及 `角色设定` 那一组固定别名是**已知形状**；`skill.` 是后端**明确拒绝**的
/// 废弃命名空间（`prompt template namespace skill is deprecated`），编辑期就该标红；
/// 裸名（`{{本章大纲}}`）后端会去 inputs 里找，找不到才 fail-loud——
/// **编辑期无从判断，因此给「待确认」而不是「错」**。
///
/// 把裸名一律标红会更「醒目」，但那是**误报**：裸名走 inputs 回落是后端支持的写法，
/// 官方模板里就有。误报比漏报危险——它会训练用户忽略颜色，那之后真正的
/// `skill.` 错误也就看不见了。
/// </summary>
public static class PromptPlaceholderSyntax
{
    /// <summary>占位符种类；决定高亮用哪一档颜色。</summary>
    public enum PlaceholderKind
    {
        /// <summary>`{{ref:...}}` 正文引用，且语法合法。</summary>
        Reference,

        /// <summary>`{{ref:...}}` 但语法非法（行号为 0、区间倒置…）。</summary>
        MalformedReference,

        /// <summary>命名空间已知的变量：`input.` / `system.` / `param.` / `var.` / `template.` / 固定别名。</summary>
        KnownVariable,

        /// <summary>裸名变量：后端会去 inputs 里找。编辑期无从判断，属「待确认」而非「错」。</summary>
        UnverifiableVariable,

        /// <summary>后端会明确拒绝的写法：空占位符、`skill.` 废弃命名空间。</summary>
        RejectedVariable,
    }

    /// <summary>扫到的一个占位符。偏移量口径是 UTF-16（与 AvaloniaEdit 的 TextDocument 一致）。</summary>
    /// <param name="Start">起点，含 `{{`。</param>
    /// <param name="End">终点（半开），含 `}}`。</param>
    /// <param name="Body">花括号之间的内容，已 trim。</param>
    /// <param name="Kind">种类。</param>
    public sealed record Placeholder(int Start, int End, string Body, PlaceholderKind Kind);

    /// <summary>
    /// 与 `resolve_variable` 一一对应的固定别名。
    ///
    /// `节点提示词` 是**必须保留的兼容别名**（U149）：存量工作流的节点 config 里存的是
    /// 渲染前的模板字符串，只认新名 `角色设定` 会让所有已保存的工作流在下一次运行时
    /// fail-loud。编辑器这边把它标成「未知」同样是误报。
    /// </summary>
    private static readonly string[] FixedAliases =
    {
        "角色设定",
        "prompt.角色设定",
        "节点提示词",
        "prompt.节点提示词",
    };

    /// <summary>命名空间前缀白名单，与 `resolve_variable` 的 `strip_prefix` 分支同一份。</summary>
    private static readonly string[] KnownPrefixes =
    {
        "input.",
        "system.",
        "param.",
        "var.",
        "template.",
    };

    /// <summary>
    /// 扫描全部 `{{...}}`，按出现顺序返回。
    ///
    /// **未闭合的 `{{` 不记**——与 `ContentReferenceSyntax.Parse` 和后端
    /// `render_prompt_template` 同一取舍：它没有确定的结束位置，划不出高亮区间。
    /// 用户打字打到一半时 `{{` 必然短暂未闭合，为它闪一次红是纯噪音。
    /// </summary>
    public static IReadOnlyList<Placeholder> Parse(string? text)
    {
        var found = new List<Placeholder>();
        if (string.IsNullOrEmpty(text))
        {
            return found;
        }

        var cursor = 0;
        while (cursor < text.Length)
        {
            var open = text.IndexOf("{{", cursor, StringComparison.Ordinal);
            if (open < 0)
            {
                break;
            }

            var bodyStart = open + 2;
            var close = text.IndexOf("}}", bodyStart, StringComparison.Ordinal);
            if (close < 0)
            {
                break;
            }

            var end = close + 2;
            found.Add(new Placeholder(
                open,
                end,
                text[bodyStart..close].Trim(),
                Classify(text[bodyStart..close].Trim())));
            cursor = end;
        }

        return found;
    }

    /// <summary>判种类。引用那一类转交 <see cref="ContentReferenceSyntax"/>，不在这里重写一遍语法。</summary>
    private static PlaceholderKind Classify(string body)
    {
        if (body.Length == 0)
        {
            // 后端：`prompt template variable cannot be empty`。
            return PlaceholderKind.RejectedVariable;
        }

        // `ref:` 交给引用词法层判定——那边已经有完整的行号/版本校验与 19 条回归，
        // 在这里再写一遍等于给同一条语法造第二个真相。
        if (body.StartsWith("ref:", StringComparison.Ordinal))
        {
            var occurrence = ContentReferenceSyntax
                .Parse(ContentReferenceSyntax.Open + body["ref:".Length..] + ContentReferenceSyntax.Close);
            return occurrence.Count == 1 && occurrence[0].IsValid
                ? PlaceholderKind.Reference
                : PlaceholderKind.MalformedReference;
        }

        if (body.StartsWith("skill.", StringComparison.Ordinal))
        {
            // 后端：`prompt template namespace skill is deprecated; use template`。
            // 这是**确定**会失败的写法，编辑期标红不是误报。
            return PlaceholderKind.RejectedVariable;
        }

        foreach (var alias in FixedAliases)
        {
            if (string.Equals(body, alias, StringComparison.Ordinal))
            {
                return PlaceholderKind.KnownVariable;
            }
        }

        foreach (var prefix in KnownPrefixes)
        {
            if (body.StartsWith(prefix, StringComparison.Ordinal))
            {
                // 前缀后面必须有名字：`{{input.}}` 后端一定报未解析。
                return body.Length > prefix.Length
                    ? PlaceholderKind.KnownVariable
                    : PlaceholderKind.RejectedVariable;
            }
        }

        // 裸名：后端去 inputs 里找。编辑期没有 inputs ⇒ 只能说「待确认」。
        return PlaceholderKind.UnverifiableVariable;
    }
}
