namespace Ariadne.Desktop.Controls;

/// <summary>阅读态识别出的块级语义。小说正文用得到的就这几种。</summary>
public enum MarkdownSegmentKind
{
    /// <summary>普通段落。连续的非空行合成一段，行间以 <see cref="MarkdownInlineKind.LineBreak"/> 分隔。</summary>
    Paragraph,
    /// <summary>ATX 标题（`#`~`######`）。</summary>
    Heading,
    /// <summary>引用块（`>`）。连续引用行合成一段。</summary>
    Quote,
    /// <summary>分隔线（`---` / `***` / `___`）。</summary>
    ThematicBreak,
    /// <summary>列表项。有序/无序都走这一种，序号或圆点在 <see cref="MarkdownSegment.ListMarker"/>。</summary>
    ListItem,
    /// <summary>围栏代码块（```）。块内**不做**行内解析。</summary>
    CodeBlock,
}

/// <summary>行内片段类型。刻意只有这几种——见 <see cref="MarkdownReadingParser"/> 的支持清单。</summary>
public enum MarkdownInlineKind
{
    Text,
    Bold,
    Italic,
    BoldItalic,
    Code,
    /// <summary>段内换行。<see cref="Text"/> 恒为空串。</summary>
    LineBreak,
}

/// <summary>行内片段。</summary>
public sealed record MarkdownInline(MarkdownInlineKind Kind, string Text)
{
    public static MarkdownInline PlainText(string text) => new(MarkdownInlineKind.Text, text);
    public static MarkdownInline Break() => new(MarkdownInlineKind.LineBreak, string.Empty);
}

/// <summary>
/// 一个块级片段。
/// </summary>
/// <param name="Kind">块级语义。</param>
/// <param name="HeadingLevel">标题级别 1–6；非标题为 0。</param>
/// <param name="ListMarker">列表项前导标记（无序为圆点，有序保留作者写的序号）；非列表为 null。</param>
/// <param name="Inlines">行内片段序列。<see cref="MarkdownSegmentKind.ThematicBreak"/> 为空。</param>
public sealed record MarkdownSegment(
    MarkdownSegmentKind Kind,
    int HeadingLevel,
    string? ListMarker,
    IReadOnlyList<MarkdownInline> Inlines)
{
    /// <summary>该片段渲染后**可见**的纯文本（标记字符已剥离，段内换行为 \n）。</summary>
    public string VisibleText => string.Concat(
        Inlines.Select(inline => inline.Kind == MarkdownInlineKind.LineBreak ? "\n" : inline.Text));

    /// <summary>是否所有行内片段都是无格式文本（可走 <c>TextBlock.Text</c> 直出这条低风险路径）。</summary>
    public bool IsPlainText => Inlines.All(inline =>
        inline.Kind is MarkdownInlineKind.Text or MarkdownInlineKind.LineBreak);
}
