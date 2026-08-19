using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U150 / U201-A：提示词编辑器里 `{{ref:...}}` 的**预览状态**。
///
/// **纯逻辑、无视觉树** —— 这是刻意的：预览状态的全部性质都不依赖控件。
/// 能做成纯逻辑就别放进视觉树，那样它在任何环境都跑得动、也跑得快。
/// 浮层是否真的开出来那一半在 `PromptReferenceFoldingViewTests`（真实控件）。
///
/// # ⚠️ 本文件在 U201-A 被整体改写，别照旧版理解
///
/// 旧版钉的是**错误极性**：`EverythingStartsCollapsed` 断言「默认全部折叠」
/// （即默认屏幕上看不到 `{{ref:` 字面量），而那正是缺陷本体——
/// 默认态吃掉了可编辑性，作者无从照抄语法写第二条引用、想改行段得先「展开」。
///
/// 现在的性质是：**文本流永远是字面量**，预览是另一层的开合。
/// 所以这里不再有「折叠态」这个概念，只有「预览开没开」。
/// </summary>
public sealed class ReferenceFoldingStateTests
{
    private const string Outline =
        "本章接住上一章的雨。\n{{ref:chapters/chapter-01.md#L2-L3}}\n注意克制。\n"
        + "对照那一段的节奏：\n{{ref:chapters/chapter-01.md#L9-L9}}\n";

    /// <summary>被引文档的正文，供预览取用。行号从 1 起数。</summary>
    private const string ReferencedDocument =
        "第一行：雨停了。\n第二行：她把伞收起来。\n第三行：门在身后合上。\n第四行：走廊很长。\n";

    /// <summary>
    /// **默认没有任何预览打开**，而这**不等于**「默认折叠」。
    ///
    /// 区别是本轮缺陷的核心：默认态下屏幕上是完整的 `{{ref:...}}` 字面量
    /// （由 AvaloniaEdit 原生渲染，本类根本不参与），作者可以照抄语法、直接改行段。
    /// 旧版把「默认」做成了折叠摘要，等于默认只读、编辑要先解锁。
    /// </summary>
    [Fact]
    public void NoPreviewIsOpenByDefaultAndTheLiteralStaysIntact()
    {
        var state = new ReferenceFoldingState();
        var segments = state.Project(Outline);

        Assert.Equal(2, segments.Count);
        Assert.All(segments, segment => Assert.False(segment.IsPreviewOpen));
        Assert.False(state.IsAnyPreviewOpen);
        Assert.Null(state.OpenPreviewBody);

        // 投影里每条段落都必须**精确覆盖占位符本身**——因为文本流不再被替换，
        // 这些偏移的唯一用途就是命中测试。切偏了就会点不到（或点到别处）。
        foreach (var segment in segments)
        {
            Assert.Equal("{{ref:", Outline.Substring(segment.Start, 6));
            Assert.Equal("}}", Outline.Substring(segment.End - 2, 2));
        }
    }

    /// <summary>
    /// **主性质：预览打开后必须跨编辑存活。**
    ///
    /// 用户开着预览读原文，然后在**别处**敲一个字——预览不该被关掉。
    /// 这就是身份不含偏移量的理由：在文本开头插入一个字符，后面每条引用的
    /// `Start`/`End` 都变，但它们还是「同一条引用」。
    /// </summary>
    [Fact]
    public void OpenPreviewSurvivesEditsElsewhereInTheText()
    {
        var state = new ReferenceFoldingState();
        var before = ContentReferenceSyntax.Parse(Outline);
        Assert.True(state.OpenPreview(before[0], "第二行：她把伞收起来。\n"));

        // 在**开头**插入文字：后面所有偏移都平移了。
        var edited = "（改标题）" + Outline;
        var segments = state.Project(edited);

        Assert.Equal(2, segments.Count);
        Assert.True(
            segments[0].IsPreviewOpen,
            "在别处编辑后预览被关掉了 ⇒ 用户每敲一个字，正在读的预览就闪一下没了");
        Assert.False(segments[1].IsPreviewOpen, "没开预览的那条不该跟着开");

        // 偏移确实变了——否则这条用例没在测「跨偏移变化存活」。
        Assert.NotEqual(before[0].Start, segments[0].Start);
    }

    /// <summary>同一份文档的不同行段是**两条独立**引用，预览各自独立。</summary>
    [Fact]
    public void SameDocumentDifferentRangesPreviewIndependently()
    {
        var state = new ReferenceFoldingState();
        var occurrences = ContentReferenceSyntax.Parse(Outline);
        // 前置：两条引用确实指向同一份文档，否则这条用例测的是别的东西。
        Assert.Equal(occurrences[0].DocumentId, occurrences[1].DocumentId);

        state.OpenPreview(occurrences[1], "第九行。\n");
        var segments = state.Project(Outline);

        Assert.False(segments[0].IsPreviewOpen);
        Assert.True(segments[1].IsPreviewOpen);
    }

    /// <summary>
    /// 版本锚定变了**不该**关掉预览：`@v=abc` → `@v=def` 指的还是同一段正文，
    /// 只是作者更新了锚点。
    /// </summary>
    [Fact]
    public void ChangingTheVersionAnchorKeepsThePreviewOpen()
    {
        var state = new ReferenceFoldingState();
        var before = "{{ref:a.md#L1-L2@v=abc}}";
        state.OpenPreview(ContentReferenceSyntax.Parse(before)[0], "正文两行。\n第二行。\n");

        var after = state.Project("{{ref:a.md#L1-L2@v=def}}");
        Assert.True(Assert.Single(after).IsPreviewOpen);
    }

    /// <summary>
    /// **U201-B：取不到正文 ⇒ 不进入预览态。**
    ///
    /// 语法对但文档不存在时，旧版照样能「展开」——展开出一片空白，
    /// 而用户以为是自己点坏了。现在 `OpenPreview` 的签名本身要求先有正文，
    /// 传 null 直接被拒。
    ///
    /// ⚠️ 判据同时钉住「拒绝之后状态没被污染」：只断言返回值的话，
    /// 一个「先设好 identity 再返回 false」的实现照样全绿，而那时预览
    /// 会显示上一条引用的正文（或空），比直接不开更糟。
    /// </summary>
    [Fact]
    public void PreviewIsRefusedWhenTheBodyCannotBeFetched()
    {
        var state = new ReferenceFoldingState();
        var occurrence = Assert.Single(ContentReferenceSyntax.Parse("{{ref:chapters/gone.md#L1-L2}}"));
        Assert.True(occurrence.IsValid, "前置：语法必须是合法的，否则这条测的是语法门而不是 B 条");

        Assert.False(
            state.OpenPreview(occurrence, null),
            "取不到正文却开了预览 ⇒ 用户会看到一片空白，以为是自己点坏了");

        Assert.False(state.IsAnyPreviewOpen);
        Assert.Null(state.OpenPreviewBody);
        Assert.False(state.IsPreviewOpenFor(occurrence));
    }

    /// <summary>
    /// **空串正文允许开预览**——它与「取不到」不是同一件事。
    ///
    /// 被引的那几行确实可以是空行。把空串也当失败会让作者以为文档丢了，
    /// 而真实情况是他引到了一段空白（那本身就是他要知道的信息）。
    /// </summary>
    [Fact]
    public void EmptyBodyStillOpensBecauseItIsNotTheSameAsUnavailable()
    {
        var state = new ReferenceFoldingState();
        var occurrence = Assert.Single(ContentReferenceSyntax.Parse("{{ref:a.md#L1-L1}}"));

        Assert.True(state.OpenPreview(occurrence, string.Empty));
        Assert.True(state.IsAnyPreviewOpen);
        Assert.Equal(string.Empty, state.OpenPreviewBody);
    }

    /// <summary>
    /// 语法非法的引用**不可预览**：它连 document_id 都没解析出来。
    ///
    /// 标签这时给**原因**而不是路径——用户要修的是语法，给他看文档 ID 没用。
    /// </summary>
    [Fact]
    public void MalformedReferencesCannotBePreviewed()
    {
        var state = new ReferenceFoldingState();
        var text = "{{ref:a.md#L0-L3}}";
        var occurrence = Assert.Single(ContentReferenceSyntax.Parse(text));
        Assert.False(occurrence.IsValid);

        // 即便调用方硬塞一段正文进来也不能开——规则收在这里，不靠调用点自觉。
        Assert.False(state.OpenPreview(occurrence, "硬塞的正文"), "非法引用不该开预览");

        var segment = Assert.Single(state.Project(text));
        Assert.False(segment.IsPreviewOpen);
        Assert.False(segment.IsValid);
        Assert.Equal(occurrence.ParseError, segment.PreviewLabel);
    }

    /// <summary>
    /// **同一条再点一次 = 收起**：`IsPreviewOpenFor` 要如实回答「这条开着吗」，
    /// 控件靠它决定该开还是该关。答错会让 Ctrl+左键变成单向的（关不掉）。
    /// </summary>
    [Fact]
    public void IsPreviewOpenForDistinguishesTheOpenOneFromTheRest()
    {
        var state = new ReferenceFoldingState();
        var occurrences = ContentReferenceSyntax.Parse(Outline);
        state.OpenPreview(occurrences[0], "正文。\n");

        Assert.True(state.IsPreviewOpenFor(occurrences[0]));
        Assert.False(state.IsPreviewOpenFor(occurrences[1]));

        state.ClosePreview();
        Assert.False(state.IsPreviewOpenFor(occurrences[0]));
        Assert.False(state.IsAnyPreviewOpen);
        Assert.Null(state.OpenPreviewBody);
    }

    /// <summary>
    /// **同时只有一条预览**：开第二条时第一条自动让位。
    ///
    /// 屏幕上同时开两个浮层会互相遮挡，而作者一次只看一条。
    /// 少了这条性质，一个用集合记状态的实现会留下两个都「开着」的段，
    /// 而控件只有一个浮层 ⇒ 状态与屏幕不一致。
    /// </summary>
    [Fact]
    public void OpeningASecondPreviewReplacesTheFirst()
    {
        var state = new ReferenceFoldingState();
        var occurrences = ContentReferenceSyntax.Parse(Outline);

        state.OpenPreview(occurrences[0], "第一条的正文。\n");
        state.OpenPreview(occurrences[1], "第二条的正文。\n");

        var segments = state.Project(Outline);
        Assert.False(segments[0].IsPreviewOpen, "开了第二条，第一条还开着 ⇒ 两个浮层会互相遮挡");
        Assert.True(segments[1].IsPreviewOpen);
        Assert.Equal("第二条的正文。\n", state.OpenPreviewBody);
    }

    /// <summary>
    /// **U201-B 的另一半：预览显示的是「那几行」，不是整篇。**
    ///
    /// `#L2-L3` 就该给第 2、3 行。给整篇等于没回答「引的是哪几行」——
    /// 而那正是作者点开预览要确认的事（行号写对了吗）。
    ///
    /// 1-based 闭区间，与 Rust 侧 `to_source_span` 同口径。
    /// </summary>
    [Fact]
    public void PreviewSlicesJustTheReferencedLines()
    {
        var occurrence = Assert.Single(ContentReferenceSyntax.Parse("{{ref:a.md#L2-L3}}"));
        var slice = ReferenceFoldingState.SliceForPreview(ReferencedDocument, occurrence);

        Assert.Equal("第二行：她把伞收起来。\n第三行：门在身后合上。\n", slice);
        // 不能把首行也带进来（那是 0-based 与 1-based 搞混的经典症状）。
        Assert.DoesNotContain("第一行", slice);
        Assert.DoesNotContain("第四行", slice);
    }

    /// <summary>
    /// 单行引用给单行；末行没有换行符时也要能取到。
    ///
    /// 末行那一半单列出来是因为它是行切分最容易漏的边界：
    /// 按 `\n` 切时最后一段没有分隔符收尾，实现容易把它整个丢掉。
    /// </summary>
    [Theory]
    [InlineData("{{ref:a.md#L1-L1}}", "第一行\n")]
    [InlineData("{{ref:a.md#L3-L3}}", "末行无换行")]
    public void PreviewHandlesSingleLinesIncludingAnUnterminatedLastLine(
        string template,
        string expected)
    {
        var document = "第一行\n第二行\n末行无换行";
        var occurrence = Assert.Single(ContentReferenceSyntax.Parse(template));

        Assert.Equal(expected, ReferenceFoldingState.SliceForPreview(document, occurrence));
    }

    /// <summary>
    /// 行号越界**截断到文档末尾**而不是报错或给空。
    ///
    /// 正文改短之后旧引用越界是常态（后端同样是截断 + 记警告）。
    /// 为此让预览整个失败太脆：作者最需要预览的时刻恰恰是「行号好像不对了」。
    /// </summary>
    [Fact]
    public void PreviewClampsOutOfRangeLineNumbersToTheEndOfTheDocument()
    {
        var occurrence = Assert.Single(ContentReferenceSyntax.Parse("{{ref:a.md#L3-L99}}"));
        var slice = ReferenceFoldingState.SliceForPreview(ReferencedDocument, occurrence);

        Assert.Equal("第三行：门在身后合上。\n第四行：走廊很长。\n", slice);
    }

    /// <summary>
    /// 起点整个越到末尾之后 ⇒ 给**空串**，不给整篇。
    ///
    /// 给整篇会让「行号写错了」看起来像「引用没生效」，而两者的下一步动作完全不同
    /// （改行号 vs 查文档路径）。
    /// </summary>
    [Fact]
    public void PreviewGivesNothingWhenTheRangeStartsPastTheEnd()
    {
        var occurrence = Assert.Single(ContentReferenceSyntax.Parse("{{ref:a.md#L90-L99}}"));

        Assert.Equal(
            string.Empty,
            ReferenceFoldingState.SliceForPreview(ReferencedDocument, occurrence));
    }

    /// <summary>
    /// 无定位段（整篇引用）与 byte 定位都给**整篇**。
    ///
    /// 整篇引用给整篇是定义。byte 定位（`@1024-2048`）刻意不切：那是 UTF-8 byte
    /// 区间而这里手上是 UTF-16 字符串，换算要重做一遍编码与边界校验，
    /// 而 byte 形态是工具生成的、人不手写。给整篇仍如实回答了「引的是哪篇」。
    /// </summary>
    [Theory]
    [InlineData("{{ref:a.md}}")]
    [InlineData("{{ref:a.md@0-6}}")]
    public void WholeAndByteLocatorsPreviewTheEntireDocument(string template)
    {
        var occurrence = Assert.Single(ContentReferenceSyntax.Parse(template));

        Assert.Equal(
            ReferencedDocument,
            ReferenceFoldingState.SliceForPreview(ReferencedDocument, occurrence));
    }

    /// <summary>
    /// 预览标签只取文件名，不要完整路径。
    ///
    /// `chapters/第三卷/chapter-42.md` 在浮层标题里占掉整行，而作者认得出
    /// `chapter-42.md`。
    /// </summary>
    [Fact]
    public void PreviewLabelUsesFileNameNotFullPath()
    {
        var state = new ReferenceFoldingState();
        var segment = Assert.Single(
            state.Project("{{ref:chapters/第三卷/chapter-42.md#L120-L145}}"));

        Assert.Equal("chapter-42.md L120-145", segment.PreviewLabel);
        Assert.DoesNotContain("第三卷", segment.PreviewLabel);
    }

    /// <summary>
    /// ⚠️ **预览标签与「给 AI 看的展开标记」不是同一套字符串**。
    ///
    /// 给 AI 的是 `[提供的正文参考：…][正文参考结束]`——要让模型知道
    /// 这段是引来的、边界在哪。给人的是浮层标题里一行尽量短的出处。
    /// 写成同一套会让作者以为浮层里那一行就是发出去的内容。
    /// </summary>
    [Fact]
    public void PreviewLabelIsNotTheAiFacingMarker()
    {
        var state = new ReferenceFoldingState();
        var segment = Assert.Single(state.Project("{{ref:a.md#L1-L2}}"));

        Assert.DoesNotContain("提供的正文参考", segment.PreviewLabel);
        Assert.DoesNotContain("正文参考结束", segment.PreviewLabel);
    }

    /// <summary>命中测试用**半开区间**：占位符紧邻时边界偏移不能同时命中两条。</summary>
    [Fact]
    public void HitTestUsesHalfOpenRangesSoAdjacentPlaceholdersDoNotOverlap()
    {
        var text = "{{ref:a.md}}{{ref:b.md}}";
        var occurrences = ContentReferenceSyntax.Parse(text);
        Assert.Equal(2, occurrences.Count);

        var boundary = occurrences[0].End;
        var hit = ReferenceFoldingState.HitTest(occurrences, boundary);

        Assert.NotNull(hit);
        Assert.Equal("b.md", hit!.DocumentId);
        // 边界前一个偏移仍属第一条。
        Assert.Equal("a.md", ReferenceFoldingState.HitTest(occurrences, boundary - 1)!.DocumentId);
        // 越过末尾就没有命中——不能返回最后一条。
        Assert.Null(ReferenceFoldingState.HitTest(occurrences, text.Length));
    }

    /// <summary>切到别的节点时关掉预览：上一个节点的预览带过来会让人困惑。</summary>
    [Fact]
    public void CollapseAllClosesThePreview()
    {
        var state = new ReferenceFoldingState();
        var occurrences = ContentReferenceSyntax.Parse(Outline);
        state.OpenPreview(occurrences[0], "正文。\n");
        Assert.True(state.IsAnyPreviewOpen);

        state.CollapseAll();
        Assert.All(state.Project(Outline), segment => Assert.False(segment.IsPreviewOpen));
        Assert.Null(state.OpenPreviewBody);
    }

    /// <summary>
    /// 没有引用的提示词返回空——绝大多数提示词都是这种，
    /// 走这条路时不该有任何扫描开销。
    /// </summary>
    [Theory]
    [InlineData("")]
    [InlineData("普通提示词，没有任何引用。")]
    [InlineData("有 {{本章大纲}} 这种变量占位符，但不是正文引用。")]
    public void TextWithoutReferencesProjectsNothing(string text)
    {
        Assert.Empty(new ReferenceFoldingState().Project(text));
    }
}
