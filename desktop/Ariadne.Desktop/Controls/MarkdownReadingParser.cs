namespace Ariadne.Desktop.Controls;

/// <summary>
/// U203：阅读态的 Markdown 块级/行内识别器（纯逻辑，不依赖 Avalonia，可单测）。
///
/// # 为什么自己写而不是装库
///
/// 本机 NuGet 缓存里**没有任何 Markdown 库**（Markdig / Markdown.Avalonia 都没有），
/// 且不保证有网络。更重要的是硬约束：多数 Avalonia Markdown 库带自己的样式体系或
/// HTML 渲染，会绕开 `Ariadne.*` token 另立一套视觉（U155 那类硬编码色值问题）。
/// 小说正文真正会用到的标记种类极少，自己识别可控。
///
/// # 支持 / 不支持（**不支持的一律原样显示，绝不吃字符**）
///
/// 支持：ATX 标题 `#`~`######`、引用 `>`、分隔线 `---`/`***`/`___`、
/// 无序列表 `-`/`*`/`+`、有序列表 `1.`/`1)`、`**粗**`、`*斜*`、`***粗斜***`、
/// 行内代码 <c>`code`</c>、围栏代码块 ```、反斜杠转义 `\*` `\`` `\\`。
///
/// 不支持（**原样显示**）：链接 `[t](u)`、图片、表格、脚注、HTML、
/// setext 标题（`===` 下划线）、嵌套列表层级、任务列表 `- [ ]`、YAML front matter。
/// **`_` 一律当普通字符**（不是漏做）：小说里 `snake_case`、`file_name.md`
/// 远比 `_强调_` 常见，支持它等于让文件名随机变斜体。
///
/// # 两条刻意的保守规则
///
/// ① `#` 后必须跟空格且有非空内容才算标题 —— 否则 `#tag`、光秃秃的 `####`
///    会被吃成空标题（作者会看到那一行凭空消失）。
/// ② `*` 强调的开标记后必须紧跟非空白、闭标记前必须紧跟非空白 ——
///    否则散文里的 `2 * 3 * 4` 会变成斜体。
///
/// # UTF-8 / 字符边界
///
/// 全程只按 `char` 与 `\n` 迭代，不做任何字节索引（CLAUDE.md §3）。
/// `Split('\n')` 在 UTF-16 上是安全的：`\n` 不可能是代理对的一半。
/// </summary>
public static partial class MarkdownReadingParser
{
    /// <summary>行首允许的缩进空格数上限（CommonMark 同为 3；再多就是缩进代码块的语义）。</summary>
    private const int MaxIndentSpaces = 3;

    /// <summary>
    /// 把一段正文解析为块级片段序列。
    /// </summary>
    /// <param name="text">块的原始正文（含 Markdown 标记）。</param>
    /// <param name="continuesPreviousLine">
    /// 本块的第一行是否是上一块某一行的**后半截**（虚拟化硬切在行中间时为 true）。
    /// 为 true 时首行强制当普通段落文本，不做块级标记识别——否则一个被切开的
    /// 长行的后半截若恰好以 `-` 开头，会被误当成列表项/分隔线。
    /// </param>
    public static IReadOnlyList<MarkdownSegment> Parse(string? text, bool continuesPreviousLine = false)
    {
        var segments = new List<MarkdownSegment>();
        if (string.IsNullOrEmpty(text))
        {
            return segments;
        }

        var lines = SplitLines(text);
        // 段落/引用是「连续同类行合成一段」，所以要攒一个缓冲；遇到异类行或空行才收口。
        var pending = new List<string>();
        var pendingKind = MarkdownSegmentKind.Paragraph;

        for (var index = 0; index < lines.Count; index++)
        {
            var line = lines[index];
            var forcePlain = continuesPreviousLine && index == 0;

            if (!forcePlain && TryTakeCodeFence(lines, ref index, out var codeSegment))
            {
                Flush(segments, pending, ref pendingKind);
                segments.Add(codeSegment);
                continue;
            }

            var kind = forcePlain ? MarkdownSegmentKind.Paragraph : ClassifyLine(line);

            // 空行只作分段信号，本身不产出可见内容（段间距由版面的 Margin 承担）。
            if (kind == MarkdownSegmentKind.Paragraph && IsBlank(line))
            {
                Flush(segments, pending, ref pendingKind);
                continue;
            }

            switch (kind)
            {
                case MarkdownSegmentKind.ThematicBreak:
                    Flush(segments, pending, ref pendingKind);
                    segments.Add(new MarkdownSegment(
                        MarkdownSegmentKind.ThematicBreak, 0, null, Array.Empty<MarkdownInline>()));
                    break;

                case MarkdownSegmentKind.Heading:
                    Flush(segments, pending, ref pendingKind);
                    segments.Add(BuildHeading(line));
                    break;

                case MarkdownSegmentKind.ListItem:
                    Flush(segments, pending, ref pendingKind);
                    segments.Add(BuildListItem(line));
                    break;

                case MarkdownSegmentKind.Quote:
                    if (pendingKind != MarkdownSegmentKind.Quote)
                    {
                        Flush(segments, pending, ref pendingKind);
                        pendingKind = MarkdownSegmentKind.Quote;
                    }
                    pending.Add(StripQuoteMarker(line));
                    break;

                default:
                    if (pendingKind != MarkdownSegmentKind.Paragraph)
                    {
                        Flush(segments, pending, ref pendingKind);
                    }
                    pending.Add(line);
                    break;
            }
        }

        Flush(segments, pending, ref pendingKind);
        return segments;
    }

    /// <summary>把攒下的连续同类行收口成一个片段。</summary>
    private static void Flush(
        List<MarkdownSegment> segments,
        List<string> pending,
        ref MarkdownSegmentKind pendingKind)
    {
        if (pending.Count > 0)
        {
            segments.Add(new MarkdownSegment(pendingKind, 0, null, ParseInlineLines(pending)));
            pending.Clear();
        }
        pendingKind = MarkdownSegmentKind.Paragraph;
    }

    /// <summary>
    /// 按 `\n` 切行并去掉行尾 `\r`。
    ///
    /// 不用 `string.Split(Environment.NewLine)`：正文可能来自 Windows 写的文件而
    /// 运行在 Linux（本项目的常态），按平台换行符切会整篇不分行。
    /// </summary>
    private static List<string> SplitLines(string text)
    {
        var lines = new List<string>();
        foreach (var line in text.Split('\n'))
        {
            lines.Add(line.EndsWith('\r') ? line[..^1] : line);
        }
        return lines;
    }

    private static bool IsBlank(string line) => line.Trim().Length == 0;

    /// <summary>去掉行首至多 3 个 ASCII 空格/制表符，返回剩余部分。</summary>
    private static string TrimIndent(string line)
    {
        var cut = 0;
        while (cut < line.Length && cut < MaxIndentSpaces && (line[cut] == ' ' || line[cut] == '\t'))
        {
            cut++;
        }
        return line[cut..];
    }

    /// <summary>判定一行的块级语义。顺序有讲究：`- - -` 既像分隔线又像列表项，分隔线优先。</summary>
    private static MarkdownSegmentKind ClassifyLine(string line)
    {
        var body = TrimIndent(line);
        if (IsThematicBreak(body))
        {
            return MarkdownSegmentKind.ThematicBreak;
        }
        if (HeadingLevelOf(body) > 0)
        {
            return MarkdownSegmentKind.Heading;
        }
        if (body.StartsWith('>'))
        {
            return MarkdownSegmentKind.Quote;
        }
        if (ListMarkerOf(body) is not null)
        {
            return MarkdownSegmentKind.ListItem;
        }
        return MarkdownSegmentKind.Paragraph;
    }

    /// <summary>分隔线：3 个及以上同一个 `-`/`*`/`_`，其间只允许空格。</summary>
    private static bool IsThematicBreak(string body)
    {
        if (body.Length < 3)
        {
            return false;
        }
        var marker = body[0];
        if (marker != '-' && marker != '*' && marker != '_')
        {
            return false;
        }

        var count = 0;
        foreach (var ch in body)
        {
            if (ch == marker)
            {
                count++;
            }
            else if (ch != ' ' && ch != '\t')
            {
                return false;
            }
        }
        return count >= 3;
    }

    /// <summary>
    /// ATX 标题级别；不是标题返回 0。
    ///
    /// 刻意要求 `#` 后有空格**且**有非空内容：`#标签` 与光秃秃的 `####`
    /// 若当成标题，作者会看到那一行的字符凭空少掉（吃字符比不渲染更糟）。
    /// </summary>
    private static int HeadingLevelOf(string body)
    {
        var level = 0;
        while (level < body.Length && body[level] == '#')
        {
            level++;
        }
        if (level is < 1 or > 6 || level >= body.Length)
        {
            return 0;
        }
        if (body[level] != ' ' && body[level] != '\t')
        {
            return 0;
        }
        return body[(level + 1)..].Trim().Length == 0 ? 0 : level;
    }

    /// <summary>列表标记；不是列表项返回 null。返回值即渲染时印出来的前导标记。</summary>
    private static string? ListMarkerOf(string body)
    {
        if (body.Length >= 2 && (body[0] == '-' || body[0] == '*' || body[0] == '+')
            && (body[1] == ' ' || body[1] == '\t'))
        {
            // 无序列表统一印圆点：作者写 `-` 还是 `*` 是源码风格，不该泄漏到成书上。
            return "•";
        }

        var digits = 0;
        while (digits < body.Length && digits < 9 && char.IsAsciiDigit(body[digits]))
        {
            digits++;
        }
        if (digits == 0 || digits + 1 >= body.Length)
        {
            return null;
        }
        if (body[digits] != '.' && body[digits] != ')')
        {
            return null;
        }
        if (body[digits + 1] != ' ' && body[digits + 1] != '\t')
        {
            return null;
        }
        // 有序列表**保留作者写的序号**：小说里的编号常常刻意不从 1 开始
        // （接着上一章数下去），自动重排会改掉作者的意思。
        return body[..(digits + 1)];
    }

    private static MarkdownSegment BuildHeading(string line)
    {
        var body = TrimIndent(line);
        var level = HeadingLevelOf(body);
        var content = body[level..].Trim();
        // CommonMark 的「闭合序列」：`## 标题 ##` 结尾那串 `#` 是装饰，不是内容。
        var trimmed = content.TrimEnd('#');
        if (trimmed.Length < content.Length && (trimmed.Length == 0 || trimmed[^1] == ' '))
        {
            content = trimmed.TrimEnd();
        }
        return new MarkdownSegment(
            MarkdownSegmentKind.Heading, level, null, ParseInlineLines(new[] { content }));
    }

    private static MarkdownSegment BuildListItem(string line)
    {
        var body = TrimIndent(line);
        var marker = ListMarkerOf(body);
        // ListMarkerOf 已判定过，这里只是让编译器满意；null 时退化成整行原样显示。
        var skip = marker == "•" ? 1 : marker?.Length ?? 0;
        var content = body[skip..].TrimStart(' ', '\t');
        return new MarkdownSegment(
            MarkdownSegmentKind.ListItem, 0, marker, ParseInlineLines(new[] { content }));
    }

    /// <summary>剥掉引用行的 `>` 与紧随的一个空格。多层 `>>` 只剥一层，余下的原样显示。</summary>
    private static string StripQuoteMarker(string line)
    {
        var body = TrimIndent(line);
        var rest = body[1..];
        return rest.StartsWith(' ') ? rest[1..] : rest;
    }

    /// <summary>
    /// 围栏代码块：从 ``` 行起到闭合 ``` 行止。
    /// </summary>
    /// <remarks>
    /// 块内**不做**行内解析（代码里的 `*` 是代码）。没有闭合围栏时把余下全部当代码，
    /// 与 CommonMark 一致——这也让「围栏被虚拟化切到下一块」不至于把标记吐回正文。
    /// </remarks>
    private static bool TryTakeCodeFence(
        IReadOnlyList<string> lines,
        ref int index,
        out MarkdownSegment segment)
    {
        segment = null!;
        var opening = TrimIndent(lines[index]);
        if (!opening.StartsWith("```", StringComparison.Ordinal))
        {
            return false;
        }

        var content = new List<MarkdownInline>();
        var cursor = index + 1;
        var first = true;
        while (cursor < lines.Count)
        {
            var body = TrimIndent(lines[cursor]);
            if (body.StartsWith("```", StringComparison.Ordinal))
            {
                break;
            }
            if (!first)
            {
                content.Add(MarkdownInline.Break());
            }
            content.Add(MarkdownInline.PlainText(lines[cursor]));
            first = false;
            cursor++;
        }

        segment = new MarkdownSegment(MarkdownSegmentKind.CodeBlock, 0, null, content);
        // 停在闭合围栏那一行；外层循环的 index++ 会跨过它。
        index = Math.Min(cursor, lines.Count - 1);
        return true;
    }
}
