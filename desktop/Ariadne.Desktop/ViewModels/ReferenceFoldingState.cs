using System.Collections.Generic;
using System.Linq;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// U150：提示词编辑器里 `{{ref:...}}` 的**折叠/展开状态**。
///
/// # 两种展开语义不是同一个开关
///
/// 用户明确要求过：**「对 AI 展开，对人类还是做成可 Ctrl 左键点开收起」**。
/// - **对 AI**：`{{ref:...}}` 在送进请求体**之前**必须变成真正的原文
///   （`core/src/workflow/integration.rs` 的 `expand_prompt_content_references`，
///   13-B 已落地）。那是**功能正确性**要求——占位符字面量进请求体是安全缺口，
///   Auto Mode 的审计 LLM 会在「审的是占位符」的前提下给出虚假通过。
/// - **对人**：默认**折叠**成一行摘要，Ctrl+左键手动展开/收起。那是**可读性**
///   要求——一段被引的大纲会把提示词模板撑成几十行，编辑器里就没法看了。
///
/// **不要把两者做成同一个开关。** 本类只管后者，它对送出去的文本零影响：
/// 编辑器的 `Document.Text` 始终是**原始模板**（含 `{{ref:...}}` 字面量），
/// 折叠只是**呈现层的显示变换**。这条不变式是本设计的地基——
/// 一旦折叠改写了文档文本，用户保存的就不是他写的东西了。
///
/// # 为什么状态存在这里而不是控件里
///
/// 折叠状态要在「重新解析」之后存活：用户在别处敲一个字，整段文本重新扫描，
/// 但已经展开的那几条引用不该被打回折叠态——否则每敲一个字都会闪回去。
/// 所以状态按**引用的身份**（document_id + 定位串）而不是按偏移量记：
/// 偏移量在每次编辑后都变，拿它做 key 等于每次编辑都丢状态。
/// </summary>
public sealed class ReferenceFoldingState
{
    /// <summary>已被用户手动展开的引用身份集合。</summary>
    private readonly HashSet<string> _expanded = new(StringComparer.Ordinal);

    /// <summary>
    /// 一条引用在编辑器里的呈现。
    /// </summary>
    /// <param name="Start">占位符在**当前文本**里的起始偏移（UTF-16）。</param>
    /// <param name="End">占位符结束偏移（半开）。</param>
    /// <param name="Identity">跨编辑稳定的身份，用作折叠状态的 key。</param>
    /// <param name="IsExpanded">true = 显示原文，false = 显示折叠摘要。</param>
    /// <param name="IsValid">语法是否合法；非法引用不可展开（没有原文可显示）。</param>
    /// <param name="CollapsedLabel">折叠态显示的文字。</param>
    public sealed record Segment(
        int Start,
        int End,
        string Identity,
        bool IsExpanded,
        bool IsValid,
        string CollapsedLabel);

    /// <summary>
    /// 跨编辑稳定的引用身份。
    ///
    /// **刻意不含偏移量**：在文本开头敲一个字，后面每条引用的偏移都变，
    /// 但它们还是「同一条引用」，展开状态必须跟着。
    ///
    /// 同一份文档的**不同行段**算不同引用（`#L2-L3` 与 `#L9-L9` 各自独立折叠），
    /// 所以身份要含定位串。**版本锚定不含**：`@v=abc` 换成 `@v=def` 指的还是
    /// 同一段正文，只是作者更新了锚点，没有理由把它打回折叠。
    /// </summary>
    public static string IdentityOf(ContentReferenceSyntax.Occurrence occurrence) =>
        occurrence.Locator switch
        {
            ContentReferenceSyntax.LocatorKind.Lines =>
                $"{occurrence.DocumentId}#L{occurrence.RangeStart}-L{occurrence.RangeEnd}",
            ContentReferenceSyntax.LocatorKind.Bytes =>
                $"{occurrence.DocumentId}@{occurrence.RangeStart}-{occurrence.RangeEnd}",
            _ => occurrence.DocumentId,
        };

    /// <summary>
    /// 按当前文本算出每条引用该怎么显示。
    ///
    /// 每次文本变化后重算。**不修改文本**——返回的是呈现指令，
    /// 文档内容始终是用户写的那份原始模板。
    /// </summary>
    public IReadOnlyList<Segment> Project(string? text)
    {
        // 绝大多数提示词没有引用，先挡一次省掉整趟扫描。
        if (!ContentReferenceSyntax.ContainsReference(text))
        {
            return Array.Empty<Segment>();
        }

        return ContentReferenceSyntax.Parse(text)
            .Select(occurrence =>
            {
                var identity = IdentityOf(occurrence);
                return new Segment(
                    occurrence.Start,
                    occurrence.End,
                    identity,
                    // 非法引用一律折叠：它没有可展开的原文，
                    // 展开一个语法错误只会显示一片空白，用户以为是自己点坏了。
                    IsExpanded: occurrence.IsValid && _expanded.Contains(identity),
                    occurrence.IsValid,
                    CollapsedLabelFor(occurrence));
            })
            .ToList();
    }

    /// <summary>
    /// 切换某条引用的展开态；返回切换后是否为展开。
    ///
    /// **非法引用不可展开**，调用方无需自己判断——把这个规则收在这里，
    /// 免得每个调用点各写一遍（那正是规则漂移的起点）。
    /// </summary>
    public bool Toggle(ContentReferenceSyntax.Occurrence occurrence)
    {
        if (!occurrence.IsValid)
        {
            return false;
        }

        var identity = IdentityOf(occurrence);
        if (_expanded.Remove(identity))
        {
            return false;
        }
        _expanded.Add(identity);
        return true;
    }

    /// <summary>全部收起。切到别的节点时调用——上一个节点的展开态带过来会让人困惑。</summary>
    public void CollapseAll() => _expanded.Clear();

    /// <summary>命中测试：给定光标偏移，落在哪条引用上（没有则 null）。</summary>
    ///
    /// <remarks>
    /// 用**半开区间**判定：`[Start, End)`。占位符紧邻时（`}}{{ref:`）
    /// 闭区间会让边界那一个偏移同时命中两条，点击行为随实现顺序而变。
    /// </remarks>
    public static ContentReferenceSyntax.Occurrence? HitTest(
        IReadOnlyList<ContentReferenceSyntax.Occurrence> occurrences,
        int offset) =>
        occurrences.FirstOrDefault(item => offset >= item.Start && offset < item.End);

    /// <summary>
    /// 折叠态显示的文字。
    ///
    /// ⚠️ **与「给 AI 看的展开标记」不是同一套字符串**（U150 文档特意点过这一条）。
    /// 给 AI 的是 `[提供的正文参考：xx章 章节标题名]…[正文参考结束]`——它要让模型
    /// 知道这段是引来的、边界在哪。给人的是一行**尽量短**的摘要：编辑器里一屏
    /// 只有几十列，摘要长了就把它旁边的大纲文字挤走，而作者要读的是自己写的那句话。
    ///
    /// 非法引用显示原因而不是路径：这时用户要修的是语法，给他看「文档ID」没用。
    /// </summary>
    private static string CollapsedLabelFor(ContentReferenceSyntax.Occurrence occurrence)
    {
        if (!occurrence.IsValid)
        {
            return occurrence.ParseError ?? "引用写法有误";
        }

        // 只取文件名：`chapters/第三卷/chapter-42.md` 在编辑器里占掉半行，
        // 而作者认得出 `chapter-42.md`。完整路径在悬停提示里给。
        var name = occurrence.DocumentId;
        var slash = name.LastIndexOfAny(new[] { '/', '\\' });
        if (slash >= 0 && slash + 1 < name.Length)
        {
            name = name[(slash + 1)..];
        }

        return occurrence.Locator switch
        {
            ContentReferenceSyntax.LocatorKind.Lines =>
                $"{name} L{occurrence.RangeStart}-{occurrence.RangeEnd}",
            ContentReferenceSyntax.LocatorKind.Bytes => $"{name} @{occurrence.RangeStart}",
            _ => name,
        };
    }
}
