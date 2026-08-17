using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U150：提示词编辑器里 `{{ref:...}}` 的折叠/展开状态。
///
/// **纯逻辑、无视觉树** —— 这是刻意的：本机 Avalonia headless 起不来
/// （`HeadlessUnitTestSession.StartNew` 挂住，干净 HEAD 上既有用例同样如此），
/// 而折叠状态的全部性质都不依赖控件。能做成纯逻辑就别放进视觉树，
/// 那样它在任何环境都跑得动、也跑得快。
/// </summary>
public sealed class ReferenceFoldingStateTests
{
    private const string Outline =
        "本章接住上一章的雨。\n{{ref:chapters/chapter-01.md#L2-L3}}\n注意克制。\n"
        + "对照那一段的节奏：\n{{ref:chapters/chapter-01.md#L9-L9}}\n";

    /// <summary>默认全部折叠：一段被引的大纲会把模板撑成几十行。</summary>
    [Fact]
    public void EverythingStartsCollapsed()
    {
        var state = new ReferenceFoldingState();
        var segments = state.Project(Outline);

        Assert.Equal(2, segments.Count);
        Assert.All(segments, segment => Assert.False(segment.IsExpanded));
    }

    /// <summary>
    /// **主性质：展开状态必须跨编辑存活。**
    ///
    /// 用户展开一条引用读原文，然后在**别处**敲一个字——那条引用不该被打回折叠。
    /// 这就是身份不含偏移量的理由：在文本开头插入一个字符，后面每条引用的
    /// `Start`/`End` 都变，但它们还是「同一条引用」。
    ///
    /// 判据取「编辑后仍为展开」而不是「Identity 不含数字」：
    /// 后者是对实现的断言，前者是用户能感知的性质——
    /// 换一种身份算法只要保住这条性质就仍然正确。
    /// </summary>
    [Fact]
    public void ExpansionSurvivesEditsElsewhereInTheText()
    {
        var state = new ReferenceFoldingState();
        var before = ContentReferenceSyntax.Parse(Outline);
        Assert.True(state.Toggle(before[0]));

        // 在**开头**插入文字：后面所有偏移都平移了。
        var edited = "（改标题）" + Outline;
        var segments = state.Project(edited);

        Assert.Equal(2, segments.Count);
        Assert.True(
            segments[0].IsExpanded,
            "在别处编辑后展开态丢了 ⇒ 用户每敲一个字，展开的引用就闪回折叠");
        Assert.False(segments[1].IsExpanded, "没被展开的那条不该跟着展开");

        // 偏移确实变了——否则这条用例没在测「跨偏移变化存活」。
        Assert.NotEqual(before[0].Start, segments[0].Start);
    }

    /// <summary>同一份文档的不同行段是**两条独立**引用，各自折叠。</summary>
    [Fact]
    public void SameDocumentDifferentRangesFoldIndependently()
    {
        var state = new ReferenceFoldingState();
        var occurrences = ContentReferenceSyntax.Parse(Outline);
        // 前置：两条引用确实指向同一份文档，否则这条用例测的是别的东西。
        Assert.Equal(occurrences[0].DocumentId, occurrences[1].DocumentId);

        state.Toggle(occurrences[1]);
        var segments = state.Project(Outline);

        Assert.False(segments[0].IsExpanded);
        Assert.True(segments[1].IsExpanded);
    }

    /// <summary>
    /// 版本锚定变了**不该**打回折叠：`@v=abc` → `@v=def` 指的还是同一段正文，
    /// 只是作者更新了锚点。
    /// </summary>
    [Fact]
    public void ChangingTheVersionAnchorKeepsTheExpansion()
    {
        var state = new ReferenceFoldingState();
        var before = "{{ref:a.md#L1-L2@v=abc}}";
        state.Toggle(ContentReferenceSyntax.Parse(before)[0]);

        var after = state.Project("{{ref:a.md#L1-L2@v=def}}");
        Assert.True(Assert.Single(after).IsExpanded);
    }

    /// <summary>
    /// 语法非法的引用**不可展开**：它没有原文可显示。
    ///
    /// 展开一个语法错误只会显示一片空白，用户以为是自己点坏了。
    /// 折叠标签这时给**原因**而不是路径——用户要修的是语法。
    /// </summary>
    [Fact]
    public void MalformedReferencesCannotBeExpanded()
    {
        var state = new ReferenceFoldingState();
        var text = "{{ref:a.md#L0-L3}}";
        var occurrence = Assert.Single(ContentReferenceSyntax.Parse(text));
        Assert.False(occurrence.IsValid);

        Assert.False(state.Toggle(occurrence), "非法引用不该切换成展开");

        var segment = Assert.Single(state.Project(text));
        Assert.False(segment.IsExpanded);
        Assert.False(segment.IsValid);
        Assert.Equal(occurrence.ParseError, segment.CollapsedLabel);
    }

    /// <summary>
    /// 折叠标签只取文件名，不要完整路径。
    ///
    /// `chapters/第三卷/chapter-42.md` 在编辑器里占掉半行，而作者认得出
    /// `chapter-42.md`。标签长了就把它旁边的大纲文字挤走，
    /// 而作者要读的是自己写的那句话。
    /// </summary>
    [Fact]
    public void CollapsedLabelUsesFileNameNotFullPath()
    {
        var state = new ReferenceFoldingState();
        var segment = Assert.Single(
            state.Project("{{ref:chapters/第三卷/chapter-42.md#L120-L145}}"));

        Assert.Equal("chapter-42.md L120-145", segment.CollapsedLabel);
        Assert.DoesNotContain("第三卷", segment.CollapsedLabel);
    }

    /// <summary>
    /// ⚠️ **折叠标签与「给 AI 看的展开标记」不是同一套字符串**。
    ///
    /// 给 AI 的是 `[提供的正文参考：…][正文参考结束]`——要让模型知道
    /// 这段是引来的、边界在哪。给人的是一行尽量短的摘要。
    /// U150 文档特意点了这一条，写成同一套会让编辑器里出现一行给模型看的标记。
    /// </summary>
    [Fact]
    public void CollapsedLabelIsNotTheAiFacingMarker()
    {
        var state = new ReferenceFoldingState();
        var segment = Assert.Single(state.Project("{{ref:a.md#L1-L2}}"));

        Assert.DoesNotContain("提供的正文参考", segment.CollapsedLabel);
        Assert.DoesNotContain("正文参考结束", segment.CollapsedLabel);
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
        Assert.Equal(
            "b.md",
            hit!.DocumentId);
        // 边界前一个偏移仍属第一条。
        Assert.Equal("a.md", ReferenceFoldingState.HitTest(occurrences, boundary - 1)!.DocumentId);
        // 越过末尾就没有命中——不能返回最后一条。
        Assert.Null(ReferenceFoldingState.HitTest(occurrences, text.Length));
    }

    /// <summary>切到别的节点时全部收起：上一个节点的展开态带过来会让人困惑。</summary>
    [Fact]
    public void CollapseAllResetsEverything()
    {
        var state = new ReferenceFoldingState();
        foreach (var occurrence in ContentReferenceSyntax.Parse(Outline))
        {
            state.Toggle(occurrence);
        }
        Assert.All(state.Project(Outline), segment => Assert.True(segment.IsExpanded));

        state.CollapseAll();
        Assert.All(state.Project(Outline), segment => Assert.False(segment.IsExpanded));
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
