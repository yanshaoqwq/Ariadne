namespace Ariadne.Desktop.Controls;

/// <summary>
/// U203 行内解析：`**粗**` / `*斜*` / `***粗斜***` / <c>`代码`</c> / `\*` 转义。
///
/// 拆成独立文件是因为它有一条与块级完全不同的核心性质：
/// **任何无法配对的标记都必须原样留在文本里**。吃掉字符比不渲染更糟——
/// 不渲染只是丑，吃字符是作者的正文丢了一截，而且他不会立刻发现。
/// </summary>
public static partial class MarkdownReadingParser
{
    /// <summary>可被 `\` 转义的字符。刻意只有这三个：只对本解析器真正会消费的标记生效，
    /// 否则散文里正常出现的 `\` 会被静默吃掉（例如写 Windows 路径）。</summary>
    private const string EscapableChars = "*`\\";

    /// <summary>
    /// 把多行合成一个行内序列，行间插 <see cref="MarkdownInlineKind.LineBreak"/>。
    ///
    /// **逐行解析、不跨行配对**：一是小说里跨行的 `*` 几乎都是巧合而非强调，
    /// 二是跨行配对会让「上一段末尾一个星号」把下面整段变成斜体（作者极难归因）。
    /// </summary>
    private static IReadOnlyList<MarkdownInline> ParseInlineLines(IReadOnlyList<string> lines)
    {
        var result = new List<MarkdownInline>();
        for (var index = 0; index < lines.Count; index++)
        {
            if (index > 0)
            {
                result.Add(MarkdownInline.Break());
            }
            ParseInlineLine(lines[index], result);
        }
        return result;
    }

    private static void ParseInlineLine(string line, List<MarkdownInline> sink)
    {
        var buffer = new System.Text.StringBuilder();
        var cursor = 0;
        while (cursor < line.Length)
        {
            var ch = line[cursor];

            // ① 转义：`\*` → 字面 `*`。只吃我们会消费的标记字符。
            if (ch == '\\' && cursor + 1 < line.Length && EscapableChars.Contains(line[cursor + 1]))
            {
                buffer.Append(line[cursor + 1]);
                cursor += 2;
                continue;
            }

            // ② 行内代码：反引号里的内容原样保留，不再做强调解析。
            if (ch == '`' && TryMatchCode(line, cursor, out var codeText, out var codeEnd))
            {
                FlushText(buffer, sink);
                sink.Add(new MarkdownInline(MarkdownInlineKind.Code, codeText));
                cursor = codeEnd;
                continue;
            }

            // ③ 强调：`***` > `**` > `*`，长的先试，否则 `***粗斜***` 会被当成
            //    「粗体 + 一个裸星号」。
            if (ch == '*' && TryMatchEmphasis(line, cursor, out var kind, out var inner, out var end))
            {
                FlushText(buffer, sink);
                // 强调内部仍可能有行内代码/转义，递归一层即可（强调不嵌套强调，
                // 那是 CommonMark 里最容易写出无限回溯的地方，小说也用不到）。
                ParseEmphasisContent(kind, inner, sink);
                cursor = end;
                continue;
            }

            buffer.Append(ch);
            cursor++;
        }
        FlushText(buffer, sink);
    }

    private static void FlushText(System.Text.StringBuilder buffer, List<MarkdownInline> sink)
    {
        if (buffer.Length > 0)
        {
            sink.Add(MarkdownInline.PlainText(buffer.ToString()));
            buffer.Clear();
        }
    }

    /// <summary>强调内部只再认行内代码与转义，认不出的原样并入。</summary>
    private static void ParseEmphasisContent(
        MarkdownInlineKind kind,
        string inner,
        List<MarkdownInline> sink)
    {
        var buffer = new System.Text.StringBuilder();
        var cursor = 0;
        while (cursor < inner.Length)
        {
            if (inner[cursor] == '\\' && cursor + 1 < inner.Length
                && EscapableChars.Contains(inner[cursor + 1]))
            {
                buffer.Append(inner[cursor + 1]);
                cursor += 2;
                continue;
            }
            if (inner[cursor] == '`' && TryMatchCode(inner, cursor, out var codeText, out var codeEnd))
            {
                if (buffer.Length > 0)
                {
                    sink.Add(new MarkdownInline(kind, buffer.ToString()));
                    buffer.Clear();
                }
                sink.Add(new MarkdownInline(MarkdownInlineKind.Code, codeText));
                cursor = codeEnd;
                continue;
            }
            buffer.Append(inner[cursor]);
            cursor++;
        }
        if (buffer.Length > 0)
        {
            sink.Add(new MarkdownInline(kind, buffer.ToString()));
        }
    }

    /// <summary>匹配 <c>`code`</c>；配不上返回 false（此时那个反引号原样显示）。</summary>
    private static bool TryMatchCode(string line, int start, out string text, out int end)
    {
        text = string.Empty;
        end = start;
        var close = line.IndexOf('`', start + 1);
        // 空的 `` 不算代码：配对成功却渲染出零内容，等于吃掉两个字符。
        if (close <= start + 1)
        {
            return false;
        }
        text = line[(start + 1)..close];
        end = close + 1;
        return true;
    }

    /// <summary>
    /// 匹配 `*` 系强调。
    ///
    /// 两条边界规则（都是为散文服务，不是照抄 CommonMark）：
    /// 开标记后必须紧跟非空白、闭标记前必须紧跟非空白 ⇒ `2 * 3 * 4` 不会变斜体。
    /// </summary>
    private static bool TryMatchEmphasis(
        string line,
        int start,
        out MarkdownInlineKind kind,
        out string inner,
        out int end)
    {
        kind = MarkdownInlineKind.Text;
        inner = string.Empty;
        end = start;

        var runLength = 0;
        while (start + runLength < line.Length && line[start + runLength] == '*' && runLength < 3)
        {
            runLength++;
        }
        var marker = new string('*', runLength);
        var contentStart = start + runLength;
        if (contentStart >= line.Length || char.IsWhiteSpace(line[contentStart]))
        {
            return false;
        }

        var search = contentStart;
        while (true)
        {
            var close = line.IndexOf(marker, search, StringComparison.Ordinal);
            if (close < 0 || close <= contentStart)
            {
                return false;
            }
            // 闭标记前一个字符不能是空白，否则 `* 3 *` 这种算式会被当强调。
            if (!char.IsWhiteSpace(line[close - 1]))
            {
                inner = line[contentStart..close];
                kind = runLength switch
                {
                    3 => MarkdownInlineKind.BoldItalic,
                    2 => MarkdownInlineKind.Bold,
                    _ => MarkdownInlineKind.Italic,
                };
                end = close + runLength;
                return true;
            }
            search = close + marker.Length;
        }
    }
}
