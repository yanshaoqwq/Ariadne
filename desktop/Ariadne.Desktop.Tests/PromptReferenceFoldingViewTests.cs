using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Input;
using Avalonia.Threading;
using Ariadne.Desktop.Controls;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U150 / U115：提示词编辑器的**呈现层**——折叠、Ctrl+左键展开、占位符高亮。
///
/// # 与 `ReferenceFoldingStateTests` 的分工
///
/// 那一份测的是**状态模型**（纯逻辑，无视觉树）：投影、身份、跨编辑存活。
/// 它全绿而功能仍然不可用——因为呈现层根本没接（U150 的地基已落但没接线）。
/// 这正是「测试全绿 ≠ 功能可用」的那一类：状态模型自洽，但没人调它。
///
/// 所以本文件的判据一律落在**真实控件**上：真实 `PromptTemplateEditor` 实例、
/// 真实 `TextDocument`、真实 `PointerPressedEventArgs`（含 Ctrl 修饰键）。
/// 断言的是「屏幕上显示的是折叠摘要还是原文」，不是「内部集合里有没有那个 key」——
/// 后者在「状态改了但没重画」时照样全绿，而那时用户点了没反应。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class PromptReferenceFoldingViewTests
{
    /// 两条引用，指向同一份文档的不同行段。
    private const string Template =
        "先读这一段：\n{{ref:chapters/chapter-01.md#L2-L3}}\n再对照节奏：\n"
        + "{{ref:chapters/chapter-01.md#L9-L9}}\n";

    /// <summary>
    /// **默认是折叠态。**
    ///
    /// 这是「对人折叠」那一半的全部意义：一段被引的大纲会把提示词模板撑成几十行，
    /// 编辑器里就没法看自己写的那句话了。
    ///
    /// 判据取控件的 `CurrentSegments`（呈现投影）而不是状态对象——
    /// 缺陷完全可以是「Project 算对了但控件从没调它」，那时投影是空的、
    /// 屏幕上是原始 `{{ref:...}}`，而只断言状态对象的用例看不出来。
    /// </summary>
    [Fact]
    public async Task ReferencesStartCollapsedInTheRealEditor()
    {
        await RunHeadlessAsync(() =>
        {
            var editor = new PromptTemplateEditor { BoundText = Template };

            Assert.Equal(2, editor.CurrentSegments.Count);
            Assert.All(
                editor.CurrentSegments,
                segment => Assert.False(
                    segment.IsExpanded,
                    "提示词编辑器一打开就是展开态 ⇒ 一段被引的大纲会把模板撑成几十行，"
                    + "作者看不到自己写的指示"));
            return Task.CompletedTask;
        });
    }

    /// <summary>
    /// **主用例：Ctrl+左键展开，再点一次收起。**
    ///
    /// 这是用户提过两次的那条（「而且还要做 ctrl 点击展开/收回，我猜这个也没做」）。
    ///
    /// 判据落在控件的**呈现投影**上（`CurrentSegments[i].IsExpanded`）——
    /// 那是屏幕上显示折叠摘要还是原文的直接依据。少了「命中测试 → Toggle → 重投影」
    /// 任何一环这条都红。
    ///
    /// ⚠️ **坐标换算那一段没有被覆盖**，代价要说清：AvaloniaEdit 的 `TextArea` 在
    /// headless 下**从不被 arrange**（实测 `TextArea.Bounds` 恒 0×0，而外层 editor
    /// 是 520×320），于是 `TextView.VisualLines` 为空、`GetPosition` 一律返回 null。
    /// 硬要在测试里走坐标只会得到一条「因为环境而永远红」的用例。
    /// 那半段由真机开窗验证；这里覆盖的是缺陷真正所在的另一半
    /// （此前这一半**完全不存在**：Ctrl+左键全仓零实现）。
    /// </summary>
    [Fact]
    public async Task CtrlLeftClickExpandsThenCollapsesTheReferenceUnderTheCursor()
    {
        await RunHeadlessAsync(() =>
        {
            var editor = new PromptTemplateEditor { BoundText = Template };

            // 点在**第一条**引用内部（起点 +3，落在 `{{ref:` 里，仍属该占位符）。
            var first = ContentReferenceSyntax.Parse(Template)[0];
            Assert.True(
                editor.ToggleReferenceAtOffset(first.Start + 3),
                "偏移落在第一条引用内部却没命中 ⇒ HitTest 没接上（或投影里没有这条）");

            Assert.True(
                editor.CurrentSegments[0].IsExpanded,
                "Ctrl+左键之后第一条引用仍是折叠态 ⇒ 用户提了两次的手势还是没接上");
            Assert.False(
                editor.CurrentSegments[1].IsExpanded,
                "只点了第一条，第二条不该跟着展开");

            // 再点一次同一处：收起。「展开/收回」是一个手势的两个方向，
            // 只做展开等于做了一半——收不回去的话模板永远是几十行。
            editor.ToggleReferenceAtOffset(first.Start + 3);
            Assert.False(
                editor.CurrentSegments[0].IsExpanded,
                "再点一次没收起 ⇒ 展开是单向的，模板会一直撑着");

            return Task.CompletedTask;
        });
    }

    /// <summary>
    /// **不带 Ctrl 的左键不展开**，且**点在引用之外不展开**。
    ///
    /// 这两条一起构成手势的「负空间」。少了它们，一个把所有点击都当展开的实现
    /// 也能让上面那条主用例全绿——而那种实现会让用户每次定位光标都误触折叠。
    ///
    /// 修饰键那一半走**真实的处理器**（真实 `PointerPressedEventArgs`），
    /// 因为「Ctrl 判断」正是它的职责所在；偏移那一半走接缝。
    /// </summary>
    [Fact]
    public async Task PlainClickAndClicksOutsideReferencesDoNotToggle()
    {
        await RunHeadlessAsync(() =>
        {
            var editor = new PromptTemplateEditor { BoundText = Template };
            var first = ContentReferenceSyntax.Parse(Template)[0];

            // ① 不带 Ctrl：处理器必须原样放过（连坐标换算都不该走）。
            editor.OnEditorPointerPressed(editor, PressArgs(editor, KeyModifiers.None));
            Assert.All(
                editor.CurrentSegments,
                segment => Assert.False(
                    segment.IsExpanded,
                    "不带 Ctrl 的左键也展开了 ⇒ 用户每次定位光标都会误触折叠"));

            // ② 带 Ctrl 但点在引用之外（第 0 个字符，在第一条引用之前）。
            Assert.False(
                editor.ToggleReferenceAtOffset(0),
                "点在引用之外却报命中 ⇒ HitTest 的区间判定错了");
            Assert.All(
                editor.CurrentSegments,
                segment => Assert.False(segment.IsExpanded));

            // 前置自检：偏移 0 确实在第一条引用之前，否则 ② 测的是别的东西。
            Assert.True(first.Start > 0);
            return Task.CompletedTask;
        });
    }

    /// <summary>
    /// 造一个左键按下事件。
    ///
    /// 坐标给 0,0：本用例只关心**修饰键那一道门**，而 headless 下坐标换算无论
    /// 给什么都得不到有效偏移（`TextArea` 不被 arrange）。给 0,0 是如实表达
    /// 「这里不测坐标」，比编一个看起来精确的坐标诚实。
    /// </summary>
    private static PointerPressedEventArgs PressArgs(
        PromptTemplateEditor editor,
        KeyModifiers modifiers)
        => new(
            editor,
            // Pointer 在 Avalonia.Input 与 System.Reflection 下同名，必须写全限定名。
            new Avalonia.Input.Pointer(1, PointerType.Mouse, isPrimary: true),
            editor.TextArea.TextView,
            new Point(0, 0),
            timestamp: 0,
            new PointerPointProperties(
                RawInputModifiers.LeftMouseButton,
                PointerUpdateKind.LeftButtonPressed),
            modifiers);

    /// <summary>
    /// 折叠摘要**不能**长得像给 AI 的展开标记。
    ///
    /// 给 AI 的是 `[提供的正文参考：…]…[正文参考结束]`（`core/src/rag/reference.rs`
    /// 的 `EXPANSION_OPEN_PREFIX`）——它要让模型知道这段是引来的、边界在哪。
    /// 两者撞形状的话，用户看到编辑器里那一行会以为**这就是发出去的内容**，
    /// 从而以为引用没被展开。U150 文档特意点了这一条。
    /// </summary>
    [Fact]
    public void CollapsedMarkerIsVisuallyDistinctFromTheAiFacingMarker()
    {
        var segment = Assert.Single(
            new ReferenceFoldingState().Project("{{ref:chapters/chapter-42.md#L120-L145}}"));
        var marker = ReferenceFoldingElementGenerator.CollapsedText(segment);

        Assert.Contains("chapter-42.md", marker, StringComparison.Ordinal);
        Assert.DoesNotContain("提供的正文参考", marker, StringComparison.Ordinal);
        Assert.DoesNotContain("正文参考结束", marker, StringComparison.Ordinal);
        // 也不能保留原始 `{{ref:` 字面量——那等于没折叠。
        Assert.DoesNotContain("{{ref:", marker, StringComparison.Ordinal);
    }

    /// <summary>
    /// U115：占位符分档必须与**后端的实际行为**一致。
    ///
    /// 三档的分界不是审美选择，是照 `core/src/rag/prompt_template.rs` 的
    /// `resolve_variable` 抄的：
    /// - 已知命名空间 / 固定别名 → 后端能解析 ⇒ 合法档
    /// - `skill.` → 后端**明确拒绝**（`namespace skill is deprecated`）⇒ 错误档
    /// - 裸名 → 后端去 inputs 里找，编辑期无从判断 ⇒ **待确认档，不是错误档**
    ///
    /// 最后那一条是本设计的关键。把裸名标红会更「醒目」，但那是**误报**——
    /// 裸名走 inputs 回落是后端支持的写法，官方模板里就有。
    /// 误报比漏报危险：它训练用户忽略颜色，之后真正的 `skill.` 错误也就看不见了。
    ///
    /// ⚠️ `节点提示词` 这条别名必须留在合法档：存量工作流的节点 config 里存的就是它
    /// （U149），标成未知等于告诉用户「你已保存的工作流全都写错了」。
    /// </summary>
    [Theory]
    // 引用：合法与非法（行号 0 是 1-based 违例）
    [InlineData("{{ref:a.md#L1-L2}}", PromptPlaceholderSyntax.PlaceholderKind.Reference)]
    [InlineData("{{ref:a.md#L0-L2}}", PromptPlaceholderSyntax.PlaceholderKind.MalformedReference)]
    // 已知命名空间
    [InlineData("{{input.outline}}", PromptPlaceholderSyntax.PlaceholderKind.KnownVariable)]
    [InlineData("{{var.章节标题}}", PromptPlaceholderSyntax.PlaceholderKind.KnownVariable)]
    [InlineData("{{template.foo}}", PromptPlaceholderSyntax.PlaceholderKind.KnownVariable)]
    // U149 兼容别名：新名与旧名都得算合法
    [InlineData("{{角色设定}}", PromptPlaceholderSyntax.PlaceholderKind.KnownVariable)]
    [InlineData("{{节点提示词}}", PromptPlaceholderSyntax.PlaceholderKind.KnownVariable)]
    // 后端确定会拒的
    [InlineData("{{skill.web_search}}", PromptPlaceholderSyntax.PlaceholderKind.RejectedVariable)]
    [InlineData("{{}}", PromptPlaceholderSyntax.PlaceholderKind.RejectedVariable)]
    [InlineData("{{input.}}", PromptPlaceholderSyntax.PlaceholderKind.RejectedVariable)]
    // 裸名：待确认，**不是**错误
    [InlineData("{{本章大纲}}", PromptPlaceholderSyntax.PlaceholderKind.UnverifiableVariable)]
    public void PlaceholderKindsMatchWhatTheBackendActuallyDoes(
        string text,
        PromptPlaceholderSyntax.PlaceholderKind expected)
    {
        var placeholder = Assert.Single(PromptPlaceholderSyntax.Parse(text));
        Assert.Equal(expected, placeholder.Kind);
    }

    /// <summary>
    /// 未闭合的 `{{` 不记成占位符。
    ///
    /// 用户打字打到一半时 `{{` 必然短暂未闭合，为它闪一次红是纯噪音。
    /// 与 `ContentReferenceSyntax.Parse` 和后端 `render_prompt_template` 同一取舍。
    /// </summary>
    [Fact]
    public void UnclosedPlaceholdersAreNotHighlighted()
    {
        Assert.Empty(PromptPlaceholderSyntax.Parse("正在写 {{input.outl"));
        // 但同一段里**已闭合**的那个仍要认出来。
        Assert.Single(PromptPlaceholderSyntax.Parse("{{input.a}} 然后 {{input."));
    }

    /// <summary>
    /// **子类必须把 StyleKey 指回 `TextEditor`，否则整个编辑器渲染成一片空白。**
    ///
    /// Avalonia 按控件的 StyleKey 找 `ControlTheme`，默认是实际类型。AvaloniaEdit
    /// dll 里那份 theme **键在 `TextEditor` 上**，子类查不到 ⇒ 没有模板 ⇒
    /// 连 `TextArea` 都不被实体化，屏幕上什么都没有，而且**不报任何错**。
    ///
    /// 这个缺陷**真的发生过**（本轮第一版就是），而且是**开窗截图才看出来的**：
    /// headless 下 `TextArea` 本来就不被 arrange，「没有模板」与「有模板但没布局」
    /// 在测试里长得一模一样。所以这里断言的是**类型元数据**而不是渲染结果——
    /// 那是这条性质在无渲染环境下唯一还能验的形态，代价是它只挡「改回默认 StyleKey」
    /// 这一种回归，挡不住 theme 那边换键名。
    /// </summary>
    [Fact]
    public async Task StyleKeyPointsAtTextEditorSoTheControlThemeIsFound()
    {
        // 必须在 headless 会话里实体化：TextEditor 的构造要 IFontManagerImpl，
        // 裸 new 会抛 "Unable to locate 'Avalonia.Platform.IFontManagerImpl'"。
        await RunHeadlessAsync(() =>
        {
            var styleKey = typeof(PromptTemplateEditor)
                .GetProperty("StyleKeyOverride", BindingFlags.Instance | BindingFlags.NonPublic)
                ?.GetValue(new PromptTemplateEditor());

            Assert.Equal(
                typeof(AvaloniaEdit.TextEditor),
                styleKey);
            return Task.CompletedTask;
        });
    }

    private static async Task RunHeadlessAsync(Func<Task> body)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            await body();
            return true;
        }, CancellationToken.None);
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }
}
