using Ariadne.Desktop.Controls;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U203 解析层：**不支持的语法必须原样显示，一个字符都不许吃**。
///
/// ⚠️ 这个文件本身**证明不了缺陷已修**（缺陷版本压根不调用解析器，
/// 解析层测得再全也照样全绿）——渲染产物那一层的判据在
/// <see cref="ReadingMarkdownRenderTests"/>。这里守的是另一件事：
/// 解析器**不要弄坏正文**。吃字符比不渲染更糟：不渲染只是丑，
/// 吃字符是作者的稿子少了一截，而他不会立刻发现。
/// </summary>
public sealed class MarkdownReadingParserTests
{
    /// <summary>
    /// 把解析结果还原成「作者会看到的文字」，用于逐条比对有没有吃字符。
    ///
    /// ⚠️ **必须把 <see cref="MarkdownSegment.ListMarker"/> 也拼回来**：它是渲染时
    /// 印在版面上的独立标记控件（<c>MarkdownReaderBlock.BuildListItem</c>），
    /// 不在 <c>VisibleText</c> 里。漏掉它会让「作者看到的文字」少一截，
    /// 于是列表项在这里一律显示成「吃掉了行首标记」——那是**用例的**错觉，
    /// 不是产品的行为。（这条本身就是首轮跑出来的一个假红。）
    /// </summary>
    private static string Visible(string source) =>
        string.Join("\n", MarkdownReadingParser.Parse(source).Select(segment =>
            segment.ListMarker is { } marker
                ? $"{marker} {segment.VisibleText}"
                : segment.VisibleText));

    [Theory]
    // 链接/图片/表格/HTML：不支持 ⇒ 原样。
    [InlineData("看这里 [第三章](chapters/03.md) 有交代。")]
    [InlineData("![封面](cover.png)")]
    [InlineData("| 姓名 | 年龄 |")]
    [InlineData("<b>加粗</b>")]
    // `_` 一律当普通字符：小说里 snake_case 与文件名远比 _强调_ 常见。
    [InlineData("文件名是 chapter_01_final.md")]
    [InlineData("变量 snake_case_name 出现在正文里")]
    // 配不上的标记：留在原处。
    [InlineData("她说了一句*没有闭合的话")]
    [InlineData("三个星号但没内容 *** 后面还有字")]
    [InlineData("反引号没闭合 `select * from")]
    // 散文里的算式：两侧有空格的 `*` 不是强调。
    [InlineData("每章 2 * 3 * 4 个场景")]
    // `#` 后无空格不是标题；光秃秃的 `####` 也不是（否则那一行会凭空消失）。
    [InlineData("#标签不是标题")]
    [InlineData("####")]
    public void UnsupportedOrUnmatchedSyntax_SurvivesCharacterForCharacter(string line)
    {
        Assert.Equal(line, Visible(line));
    }

    /// <summary>
    /// 任务列表 `- [ ]` **不支持**，但它是「支持的语法 + 不支持的扩展」叠在一行上，
    /// 所以不能塞进上面那条「逐字符原样」的 Theory 里：
    ///
    /// - `- ` 是**支持**的无序列表标记 ⇒ 被消费，改印成 `•`（见 <c>ListMarkerOf</c> 注释：
    ///   作者写 `-` 还是 `*` 是源码风格，不该泄漏到成书上）；
    /// - `[ ]` 那个复选框**不支持** ⇒ 原样留在正文里，一个字符不少。
    ///
    /// 钉住这个区分是有意义的：如果哪天有人给列表加了任务列表支持，
    /// 这条会红，提醒他去确认「复选框渲染成什么」而不是静默把 `[ ]` 吃掉。
    /// </summary>
    [Fact]
    public void TaskListCheckbox_IsUnsupported_AndSurvivesVerbatim()
    {
        var segment = Assert.Single(MarkdownReadingParser.Parse("- [ ] 待办事项"));

        Assert.Equal(MarkdownSegmentKind.ListItem, segment.Kind);
        Assert.Equal("•", segment.ListMarker);
        Assert.Equal("[ ] 待办事项", segment.VisibleText);
    }

    [Fact]
    public void SupportedSyntax_StripsOnlyTheMarkers()
    {
        Assert.Equal("第1章 雨夜", Visible("# 第1章 雨夜"));
        Assert.Equal("小节", Visible("### 小节 ###"));
        Assert.Equal("我知道你会来", Visible("> 我知道你会来"));
        Assert.Equal("加粗", Visible("**加粗**"));
        Assert.Equal("斜体", Visible("*斜体*"));
        Assert.Equal("粗斜", Visible("***粗斜***"));
        Assert.Equal("代码", Visible("`代码`"));
        // 转义：`\*` 应显示为一个字面星号，而不是把反斜杠也留下。
        Assert.Equal("字面*星号", Visible(@"字面\*星号"));
    }

    [Fact]
    public void BlockKinds_AreClassifiedAsExpected()
    {
        var segments = MarkdownReadingParser.Parse(
            "# 标题\n\n正文一\n正文二\n\n> 引用\n\n---\n\n- 项目\n\n1. 有序");

        Assert.Collection(
            segments.Select(s => s.Kind),
            k => Assert.Equal(MarkdownSegmentKind.Heading, k),
            k => Assert.Equal(MarkdownSegmentKind.Paragraph, k),
            k => Assert.Equal(MarkdownSegmentKind.Quote, k),
            k => Assert.Equal(MarkdownSegmentKind.ThematicBreak, k),
            k => Assert.Equal(MarkdownSegmentKind.ListItem, k),
            k => Assert.Equal(MarkdownSegmentKind.ListItem, k));

        // 连续非空行合成**一个**段落并保留行结构：中文小说里一行就是一段，
        // 合并成一行（CommonMark 的软换行语义）会把分段全部抹掉。
        Assert.Equal("正文一\n正文二", segments[1].VisibleText);

        // 有序列表保留作者写的序号（小说里的编号常刻意不从 1 开始）。
        Assert.Equal("1.", segments[5].ListMarker);
        Assert.Equal("•", segments[4].ListMarker);
    }

    [Fact]
    public void CodeFence_KeepsContentVerbatim_AndDoesNotParseInlineMarkers()
    {
        var segments = MarkdownReadingParser.Parse("```\n值 = a * b\n**不是加粗**\n```");

        var code = Assert.Single(segments);
        Assert.Equal(MarkdownSegmentKind.CodeBlock, code.Kind);
        Assert.Equal("值 = a * b\n**不是加粗**", code.VisibleText);
    }

    /// <summary>
    /// U203 报告点名的前提：块可能被硬切在**行中间**。
    ///
    /// 后半截若恰好以 `-` / `#` 开头，按行首标记识别就会把半句话渲染成
    /// 列表项或标题。`continuesPreviousLine` 让首行强制当普通文本。
    /// </summary>
    [Fact]
    public void ContinuationBlock_DoesNotReadBlockMarkersOnItsFirstLine()
    {
        const string tail = "- 这其实是上一行被切开的后半截\n\n# 这一行是真的标题";

        var naive = MarkdownReadingParser.Parse(tail);
        Assert.Equal(MarkdownSegmentKind.ListItem, naive[0].Kind);

        var continued = MarkdownReadingParser.Parse(tail, continuesPreviousLine: true);
        Assert.Equal(MarkdownSegmentKind.Paragraph, continued[0].Kind);
        // 原样保留，包括那个减号——它是正文的一部分，不是标记。
        Assert.Equal("- 这其实是上一行被切开的后半截", continued[0].VisibleText);
        // 后续行仍正常识别：整块降级会让长段落里的标题全部失效。
        Assert.Equal(MarkdownSegmentKind.Heading, continued[1].Kind);
    }

    [Fact]
    public void EmptyAndWhitespaceInput_ProduceNoSegments()
    {
        Assert.Empty(MarkdownReadingParser.Parse(null));
        Assert.Empty(MarkdownReadingParser.Parse(string.Empty));
        Assert.Empty(MarkdownReadingParser.Parse("\n\n\n"));
    }

    /// <summary>
    /// CRLF 正文必须与 LF 正文解析结果一致。
    ///
    /// 作者的稿子可能是 Windows 写的、软件跑在 Linux 上（本项目的常态）。
    /// 按平台换行符切行会让整篇变成一行，标题一个都识别不出来。
    /// </summary>
    [Fact]
    public void CrlfSource_ParsesTheSameAsLf()
    {
        var lf = MarkdownReadingParser.Parse("# 标题\n\n正文");
        var crlf = MarkdownReadingParser.Parse("# 标题\r\n\r\n正文");

        Assert.Equal(lf.Count, crlf.Count);
        Assert.Equal(lf[0].VisibleText, crlf[0].VisibleText);
        Assert.Equal(lf[1].VisibleText, crlf[1].VisibleText);
    }

    /// <summary>
    /// 中文正文的字符边界：多字节字符里不许切开（CLAUDE.md §3）。
    ///
    /// 判据取「渲染出的可见文本 == 原文去掉标记后的字符序列」，
    /// 并显式比对码位数——切在代理对中间会得到替换字符而不是异常，
    /// 只比字符串长度看不出来。
    /// </summary>
    [Fact]
    public void MultiByteCharacters_AreNeverSplit()
    {
        // emoji 是代理对（UTF-16 两个 char），CJK 扩展区同理。
        const string body = "# 标题🌧️雨\n\n正文𠀋字与🌊浪";
        var visible = Visible(body);

        Assert.Contains("标题🌧️雨", visible, StringComparison.Ordinal);
        Assert.Contains("正文𠀋字与🌊浪", visible, StringComparison.Ordinal);
        Assert.DoesNotContain("�", visible, StringComparison.Ordinal);
    }

    /// <summary>
    /// U203：虚拟化切块**不许切在行中间**（除非那一行本身超过块粒度）。
    ///
    /// 原实现在「窗口里有换行、但都落在目标位置之前」时直接按字符数硬切，
    /// 于是标题行会被切成两半。这里造的正文正是那个形状：
    /// 一个短标题行后面跟一个超长段落。
    /// </summary>
    [Fact]
    public void BlockSplitting_NeverCutsInsideAHeadingLine()
    {
        // 3000 字的开场段 + 标题行 + 6000 字的长段：硬切窗口（4000–6000）里
        // 唯一的换行落在 3000 一带，旧实现会在 4000 处切进那个长段。
        var content = new string('文', 3_000) + "\n# 第2章 长夜\n" + new string('风', 6_000);
        var vm = MarkdownRenderTestHarness.SeedReadingViewModel(content);

        Assert.True(vm.DocumentBlocks.Count > 1, "正文必须被切成多块，否则这条用例无意义");
        foreach (var block in vm.DocumentBlocks)
        {
            if (block.StartsMidLine)
            {
                continue;
            }
            var segments = MarkdownReadingParser.Parse(block.Text, block.StartsMidLine);
            Assert.DoesNotContain(
                segments,
                segment => segment.VisibleText.Contains('#', StringComparison.Ordinal));
        }

        // 标题必须在某一块里被**完整**识别成标题，而不是散成两段正文。
        var headings = vm.DocumentBlocks
            .SelectMany(block => MarkdownReadingParser.Parse(block.Text, block.StartsMidLine))
            .Where(segment => segment.Kind == MarkdownSegmentKind.Heading)
            .ToList();
        var heading = Assert.Single(headings);
        Assert.Equal("第2章 长夜", heading.VisibleText);
    }
}
