using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Input;
using Ariadne.Desktop.Controls;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U150 / U115 / U201：提示词编辑器的**呈现层**——字面量可见、Ctrl+左键预览、占位符高亮。
///
/// # ⚠️ 本文件在 U201-A 被整体改写（连注释一起），别照旧版理解
///
/// 旧版的判据**钉的正是错误极性**：它断言「默认所有段 `IsExpanded == false`」
/// 并把这称作「对人折叠那一半的全部意义」。那个默认态才是缺陷本体：
/// 屏幕上看不到 `{{ref:` 字面量 ⇒ 作者无从照抄语法写第二条同类引用、
/// 想改行段得先「展开」成源码 ⇒ 默认只读、编辑要先解锁。而这是个**编辑器**。
/// 旧注释必须一并删掉，留着会让下一个人把极性再改回去。
///
/// # 现在的性质
///
/// 1. 文本流**永远**是作者写的字面量（没有 ElementGenerator 插手）；
/// 2. Ctrl+左键弹浮层，浮层里是**被引正文**（不是占位符字面量、不是摘要）；
/// 3. 取不到正文 ⇒ 不进入预览态，但要有可读提示；
/// 4. 不带 Ctrl 的左键什么都不做。
///
/// # 与 `ReferenceFoldingStateTests` 的分工
///
/// 那一份测**状态模型**（纯逻辑，无视觉树）：投影、身份、跨编辑存活、行切片。
/// 它全绿而功能仍可能不可用——「状态改了但浮层没开」在只断言状态的用例里照样全绿，
/// 而那时用户点了 Ctrl+左键屏幕上什么都没发生。
/// 所以本文件的判据一律落在**真实控件**上：真实 `PromptTemplateEditor`、
/// 真实 `TextDocument`、真实 `PointerPressedEventArgs`、真实 `Flyout.IsOpen`。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class PromptReferenceFoldingViewTests
{
    /// 两条引用，指向同一份文档的不同行段。
    private const string Template =
        "先读这一段：\n{{ref:chapters/chapter-01.md#L2-L3}}\n再对照节奏：\n"
        + "{{ref:chapters/chapter-01.md#L9-L9}}\n";

    /// <summary>被引文档的正文。第 2、3 行是第一条引用该显示的内容。</summary>
    private const string ReferencedDocument =
        "第一行：雨停了。\n第二行：她把伞收起来。\n第三行：门在身后合上。\n第四行：走廊很长。\n";

    /// <summary>第一条引用（`#L2-L3`）预览时**应当**显示的正文。</summary>
    private const string ExpectedFirstPreview =
        "第二行：她把伞收起来。\n第三行：门在身后合上。\n";

    /// <summary>
    /// **判据 1：`{{ref:` 字面量在屏幕上始终可见——默认态、预览开着时、预览关掉后都一样。**
    ///
    /// 这是 U201-A 的本体。旧版默认把它替换成 `‹chapter-01.md L2-3›`，
    /// 于是作者：想写第二条同类引用无从照抄语法、想把 `#L2-L3` 改成 `#L2-L5`
    /// 得先「展开」回源码。默认只读、编辑要先解锁 —— 而这是个编辑器。
    ///
    /// # 判据为什么落在「本项目没有注册任何 ElementGenerator」上
    ///
    /// 「屏幕上看得见字面量」在 headless 下没法直接量：`TextArea` 从不被 arrange，
    /// `VisualLines` 恒为空，取不到真实渲染结果。
    /// 而 AvaloniaEdit 里的字符替换**只可能**经由 `TextView.ElementGenerators` 发生
    /// （`LineTransformers` 只改颜色不改字符）。
    /// ⇒ 「这个列表里没有我们的东西」与「文本流原样显示我们关心的那些字符」等价，
    /// 且它**恰好挡住回归**：任何人想重做行内折叠，第一步必然是往这个列表里加一个
    /// 本项目的生成器。
    ///
    /// ⚠️ 不能断言列表**为空**：AvaloniaEdit 的 `TextEditor` 构造时自带三个
    /// （`SingleCharacterElementGenerator` 控制字符框、`LinkElementGenerator`、
    /// `MailLinkElementGenerator`）。它们是框架自带的 URL/控制字符处理，
    /// 不碰 `{{ref:` 这种普通字符。断言为空会让这条用例**因为框架默认值而永远红**，
    /// 而那种红会被当成噪音关掉（连同它守的性质一起）。
    ///
    /// 代价说清：按**程序集**筛，所以它挡不住「有人把折叠生成器塞进 AvaloniaEdit 源码」
    /// 这种不会发生的事；挡得住的是本仓库里重新加回行内替换。
    /// </summary>
    [Fact]
    public async Task ReferenceLiteralStaysVisibleInTheTextFlowThroughoutTheWholeInteraction()
    {
        await RunHeadlessAsync(async () =>
        {
            var editor = NewEditor(_ => Task.FromResult<string?>(ReferencedDocument));

            // ① 默认态：本项目没有往文本流里插手。
            Assert.Empty(OurGenerators(editor));
            Assert.Equal(Template, editor.Document.Text);

            // ② 开着预览时仍然如此——预览在另一层，不该动文本流。
            var first = ContentReferenceSyntax.Parse(Template)[0];
            editor.ToggleReferenceAtOffset(first.Start + 3);
            await SettleAsync(editor);
            Assert.True(editor.IsPreviewOpen, "前置：预览必须真的开了，否则 ② 测的是默认态");
            Assert.Empty(OurGenerators(editor));
            Assert.Equal(Template, editor.Document.Text);

            // ③ 关掉预览之后也一样。
            editor.ToggleReferenceAtOffset(first.Start + 3);
            await SettleAsync(editor);
            Assert.False(editor.IsPreviewOpen);
            Assert.Empty(OurGenerators(editor));
            // 文档文本被预览改写 ⇒ 作者保存下来的不是他写的东西，而他看不出差别
            // （屏幕上显示的正是被改写后的样子）。这条不变式是整个设计的地基。
            Assert.Equal(Template, editor.Document.Text);

            // 前置自检：模板里确实有字面量可看，否则上面三条在空模板上也全绿。
            Assert.Contains("{{ref:", editor.Document.Text, StringComparison.Ordinal);
        });
    }

    /// <summary>
    /// 本项目注册的 ElementGenerator（框架自带的三个不算）。
    ///
    /// 按声明程序集筛而不是按类型名：类型名会被改（U201-A 就把那个
    /// `ReferenceFoldingElementGenerator` 连文件一起删了），而「来自本仓库」这个性质
    /// 对任何新写的生成器都成立。
    /// </summary>
    private static IReadOnlyList<AvaloniaEdit.Rendering.VisualLineElementGenerator> OurGenerators(
        PromptTemplateEditor editor)
        => editor.TextArea.TextView.ElementGenerators
            .Where(generator =>
                generator.GetType().Assembly == typeof(PromptTemplateEditor).Assembly)
            .ToList();

    /// <summary>
    /// **判据 2（主用例）：Ctrl+左键显示的是「被引正文」，不是占位符字面量、也不是摘要。**
    ///
    /// 这是 U201 的第二个独立错误。U150 报告的原始诉求是
    /// 「无法在编辑器里**预览它会展开成什么**」，而上一版 Ctrl+左键给的是
    /// `{{ref:chapters/chapter-01.md#L2-L3}}` 这串占位符原文
    /// ⇒ 真正的预览**从未实现**，作者仍然要跑一次工作流、花一次 LLM 钱
    /// 才知道自己引到的是哪几行。
    ///
    /// # 三条反向断言各挡一种「看起来做了」的实现
    ///
    /// - 不含 `{{ref:` ⇒ 挡住「展开成占位符原文」（上一版的实际行为）；
    /// - 不含 `‹` ⇒ 挡住「显示折叠摘要」（上上一版的默认态）；
    /// - 不含第 1/第 4 行 ⇒ 挡住「整篇塞进浮层」（那没回答「引的是哪几行」，
    ///   而行号对不对正是作者点开预览要确认的事）。
    ///
    /// 少了这三条，一个「浮层里显示占位符原文」的实现照样能让正向断言全绿。
    /// </summary>
    [Fact]
    public async Task CtrlLeftClickShowsTheReferencedBodyTextNotThePlaceholderOrASummary()
    {
        await RunHeadlessAsync(async () =>
        {
            var fetched = new List<string>();
            var editor = NewEditor(documentId =>
            {
                fetched.Add(documentId);
                return Task.FromResult<string?>(ReferencedDocument);
            });

            // 点在**第一条**引用内部（起点 +3，落在 `{{ref:` 里，仍属该占位符）。
            var first = ContentReferenceSyntax.Parse(Template)[0];
            Assert.True(
                editor.ToggleReferenceAtOffset(first.Start + 3),
                "偏移落在第一条引用内部却没命中 ⇒ HitTest 没接上（或投影里没有这条）");
            await SettleAsync(editor);

            // ① 浮层真的开了。判据取 `IsPreviewOpen`（内部读的是 `Flyout.IsOpen`）——
            // 只断言状态对象的话，「状态改了但浮层没开」照样全绿，而那时屏幕上没反应。
            Assert.True(
                editor.IsPreviewOpen,
                "Ctrl+左键之后预览没开 ⇒ 用户提了两次的手势还是没接上");

            // ② 浮层里是**被引正文**。
            Assert.Equal(ExpectedFirstPreview, editor.PreviewBodyText);

            // ③ 三条反向断言（各挡一种「看起来做了」的实现）。
            Assert.DoesNotContain("{{ref:", editor.PreviewBodyText!, StringComparison.Ordinal);
            Assert.DoesNotContain("‹", editor.PreviewBodyText!, StringComparison.Ordinal);
            Assert.DoesNotContain("第一行", editor.PreviewBodyText!, StringComparison.Ordinal);
            Assert.DoesNotContain("第四行", editor.PreviewBodyText!, StringComparison.Ordinal);

            // ④ 确实去取了那份文档（而不是凭空造出一段正文）。
            Assert.Equal(new[] { "chapters/chapter-01.md" }, fetched);

            // ⑤ 只开了点中的那一条；另一条不受影响。
            Assert.True(editor.CurrentSegments[0].IsPreviewOpen);
            Assert.False(editor.CurrentSegments[1].IsPreviewOpen, "只点了第一条，第二条不该跟着开");

            // ⑥ 再点一次同一处：收起。「展开/收回」是一个手势的两个方向，
            // 只做展开等于做了一半——关不掉的浮层会一直挡着作者要读的那一行。
            editor.ToggleReferenceAtOffset(first.Start + 3);
            await SettleAsync(editor);
            Assert.False(editor.IsPreviewOpen, "再点一次没收起 ⇒ 预览是单向的，浮层会一直挡着");
            // 收起走的是同步路径，**不该**为一个即将关掉的浮层再跑一次 IPC。
            Assert.Single(fetched);
        });
    }

    /// <summary>
    /// **判据 3（U201-B）：语法对但取不到正文 ⇒ 不进入预览态。**
    ///
    /// 上一版的 `IsValid` 只校验**语法**（`{{ref:...}}` 写法对不对），不校验文档
    /// 存不存在 ⇒ 引一份已删掉的章节照样能「展开」，展开出一片空白，
    /// 而作者以为是自己点坏了。
    ///
    /// 三种「取不到」的形态一起测，因为它们在代码里走**不同分支**、各自会漏：
    /// - 委托返回 null（文档不存在 / 后端拒绝）；
    /// - 委托抛异常（IPC 断了）——这一条尤其重要，它是 fire-and-forget 的
    ///   `async` 路径，漏掉 catch 会走 `UnobservedTaskException` 而不是显示提示；
    /// - 委托为 null（编辑器没接后端，也就是本轮的真实状态）。
    ///
    /// # 判据的两半都必须在
    ///
    /// 「不进入预览态」+「有可见提示」。只测前者的话，一个**静默无反应**的实现
    /// 也全绿——而那时作者会以为手势坏了并反复点击，真正的原因
    /// （文档路径写错了）没人告诉他。
    /// </summary>
    [Theory]
    [InlineData("returns-null")]
    [InlineData("throws")]
    [InlineData("no-provider")]
    public async Task PreviewIsRefusedWhenTheBodyCannotBeFetchedButTheReasonIsShown(string mode)
    {
        await RunHeadlessAsync(async () =>
        {
            var editor = NewEditor(mode switch
            {
                "returns-null" => _ => Task.FromResult<string?>(null),
                "throws" => _ => Task.FromException<string?>(new InvalidOperationException("IPC 断了")),
                _ => (Func<string, Task<string?>>?)null,
            });

            var first = ContentReferenceSyntax.Parse(Template)[0];
            // 前置：这条引用的**语法是合法的**，否则这条用例测的是语法门而不是 B 条。
            Assert.True(first.IsValid);

            // 命中仍要返回 true：事件得算 Handled，否则 TextArea 会把光标跳到点击处
            // 并清掉选区，而作者只是想看一眼引用。
            Assert.True(editor.ToggleReferenceAtOffset(first.Start + 3));
            await SettleAsync(editor);

            // ① 不进入预览态。
            Assert.False(
                editor.IsPreviewOpen,
                "取不到正文却进了预览态 ⇒ 作者看到一片空白，以为是自己点坏了");
            Assert.All(editor.CurrentSegments, segment => Assert.False(segment.IsPreviewOpen));

            // ② 但有**可读的**原因。不是静默无反应。
            var notice = editor.PreviewBodyText;
            Assert.False(
                string.IsNullOrWhiteSpace(notice),
                "取不到正文时什么都不显示 ⇒ 作者会以为手势坏了并反复点击");
            // 缺 key 时 `DisplayNameService.Text` 返回 `[key]`——那不是给人看的文案。
            Assert.DoesNotContain("[ui.node.prompt", notice!, StringComparison.Ordinal);
            // 提示里不该混进被引正文（那会让人以为预览成功了）。
            Assert.DoesNotContain("第二行", notice!, StringComparison.Ordinal);

            // ③ 再点一次同一条要**重新去取**，而不是被当成「收起」。
            // 少了这条，一个「把提示也算成已打开」的实现会让作者永远看不到正文：
            // 文档恢复之后他点第二次，得到的却是「收起」。
            Assert.True(editor.ToggleReferenceAtOffset(first.Start + 3));
            await SettleAsync(editor);
            Assert.False(editor.IsPreviewOpen);
            Assert.False(string.IsNullOrWhiteSpace(editor.PreviewBodyText));
        });
    }

    /// <summary>
    /// **判据 4：不带 Ctrl 的左键不预览，点在引用之外也不预览。**
    ///
    /// 这两条一起构成手势的「负空间」。少了它们，一个把所有点击都当预览的实现
    /// 也能让上面那些用例全绿——而那种实现会让作者每次定位光标都弹出一个浮层。
    ///
    /// 修饰键那一半走**真实的处理器**（真实 `PointerPressedEventArgs`），
    /// 因为「Ctrl 判断」正是它的职责所在；偏移那一半走接缝。
    /// </summary>
    [Fact]
    public async Task PlainClickAndClicksOutsideReferencesDoNotPreview()
    {
        await RunHeadlessAsync(async () =>
        {
            var fetched = 0;
            var editor = NewEditor(_ =>
            {
                fetched++;
                return Task.FromResult<string?>(ReferencedDocument);
            });
            var first = ContentReferenceSyntax.Parse(Template)[0];

            // ① 不带 Ctrl：处理器必须原样放过（连坐标换算都不该走）。
            editor.OnEditorPointerPressed(editor, PressArgs(editor, KeyModifiers.None));
            await SettleAsync(editor);
            Assert.False(
                editor.IsPreviewOpen,
                "不带 Ctrl 的左键也弹了预览 ⇒ 作者每次定位光标都会被浮层挡住");

            // ② 带 Ctrl 但点在引用之外（第 0 个字符，在第一条引用之前）。
            Assert.False(
                editor.ToggleReferenceAtOffset(0),
                "点在引用之外却报命中 ⇒ HitTest 的区间判定错了");
            await SettleAsync(editor);
            Assert.False(editor.IsPreviewOpen);
            Assert.All(editor.CurrentSegments, segment => Assert.False(segment.IsPreviewOpen));

            // ③ 两种都**没有发起取正文**。这条是关键：一个「先无条件取正文、
            // 再判断该不该显示」的实现会让每次点击都打一次 IPC，
            // 而症状（后端日志被点击刷满、输入发涩）离手势判断很远，极难联想到这里。
            Assert.Equal(0, fetched);

            // 前置自检：偏移 0 确实在第一条引用之前，否则 ② 测的是别的东西。
            Assert.True(first.Start > 0);
        });
    }

    /// <summary>
    /// 语法非法的引用：Ctrl+左键**不进入预览态**，但给出「写法有误」的原因。
    ///
    /// 与判据 3 分开是因为走**不同分支**：这一条在命中之后就被挡下，
    /// 连取正文都不该发起（连 document_id 都没解析出来，取什么？）。
    /// 而作者此刻要修的是语法——静默无反应会让他以为手势坏了，
    /// 真正的问题只是少打了一个字符。
    /// </summary>
    [Fact]
    public async Task MalformedReferenceShowsTheSyntaxReasonWithoutFetchingAnything()
    {
        await RunHeadlessAsync(async () =>
        {
            var fetched = 0;
            var editor = new PromptTemplateEditor
            {
                // 行号 0 违反 1-based ⇒ 语法非法。
                BoundText = "对照：\n{{ref:a.md#L0-L3}}\n",
                DocumentTextProvider = _ =>
                {
                    fetched++;
                    return Task.FromResult<string?>(ReferencedDocument);
                },
            };

            var occurrence = Assert.Single(ContentReferenceSyntax.Parse(editor.Document.Text));
            Assert.False(occurrence.IsValid, "前置：这条引用的语法必须是非法的");

            Assert.True(editor.ToggleReferenceAtOffset(occurrence.Start + 3));
            await SettleAsync(editor);

            Assert.False(editor.IsPreviewOpen, "语法非法却进了预览态 ⇒ 展开出一片空白");
            Assert.False(
                string.IsNullOrWhiteSpace(editor.PreviewBodyText),
                "语法非法时静默无反应 ⇒ 作者以为手势坏了，而真正要修的是语法");
            Assert.DoesNotContain("[ui.node.prompt", editor.PreviewBodyText!, StringComparison.Ordinal);
            Assert.Equal(0, fetched);
        });
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
    /// ⚠️ U201-A 之后这一层**格外重要**：着色现在是引用在编辑器里的**唯一**视觉标记
    /// （字面量不再被替换成摘要，形状上与普通文字无异）。着色挂不上的话，
    /// `{{ref:...}}` 就退回成一串看不出特殊的普通字符。
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
    /// 这个缺陷**真的发生过**，而且是**开窗截图才看出来的**：
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

            Assert.Equal(typeof(AvaloniaEdit.TextEditor), styleKey);
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
    /// 造一个编辑器，注入取正文的委托。
    ///
    /// `provider` 为 null 表示「没接后端」——那也是要覆盖的一种形态。
    /// </summary>
    private static PromptTemplateEditor NewEditor(Func<string, Task<string?>>? provider) =>
        new() { BoundText = Template, DocumentTextProvider = provider };

    /// <summary>
    /// 等预览那趟异步取正文真正落定。
    ///
    /// **必需**：`ToggleReferenceAtOffset` 不 await 取正文（点击处理器不能等在
    /// IPC 上），所以断言必须自己等——不等的话断言会在浮层开出来之前跑完，
    /// 用例红在「预览没开」上，而真实原因只是测试没等。
    ///
    /// ⚠️ **必须 await 控件交出来的那个 Task，不能靠 drain dispatcher 队列猜时序。**
    /// 本轮先写的就是 drain 版（`InvokeAsync` 跑两轮 Background 优先级），
    /// 它在 43 条混跑里**偶发失败两次**，而且症状是**失败信息为空**的红：
    /// 续体在 headless 会话拆掉之后才跑，断言异常没有归属的用例可挂。
    /// 单跑绿、混跑红 ⇒ 先查共享状态与时序，别当噪音重跑一次就过。
    /// 偶发红比没有用例更糟：它会被当噪音关掉，连同它守的性质一起。
    /// </summary>
    private static async Task SettleAsync(PromptTemplateEditor editor)
    {
        await editor.PendingPreview;
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

    /// <summary>
    /// U201-B 最后一环：预览委托必须在**生产 XAML 里**接到 VM。
    ///
    /// # 为什么单独一条，而且判据在源码文本上
    ///
    /// 上面所有行为用例都**直接给控件赋委托**（那是它们该做的——测的是预览逻辑），
    /// 于是「XAML 里忘了绑」这件事对它们完全不可见：
    /// 控件工作正常、43 条全绿，而**生产里 Ctrl+左键恒显示「预览暂不可用」**。
    ///
    /// 这正是本项目反复吃到的形态（U150 那一版停在这一环，U184-A、U193-A 同型）：
    /// **能力做好了、没接到用户看得见处**，而所有测试都在能力那一侧。
    ///
    /// ⇒ 判据只能落在「生产 XAML 的那一行」上。它挪一行注释就绿不了——
    /// 断言的是属性赋值本身。
    /// </summary>
    [Fact]
    public void ProductionMarkup_WiresThePreviewProviderToTheViewModel()
    {
        var markup = File.ReadAllText(ResolveDesktopFile("Views", "WorkspacePageView.axaml"));
        var stripped = System.Text.RegularExpressions.Regex.Replace(
            markup, "<!--.*?-->", string.Empty, System.Text.RegularExpressions.RegexOptions.Singleline);

        // 前置：编辑器本身还在这份 XAML 里（换控件/挪位置时这条要一起看）。
        Assert.Contains("ctl:PromptTemplateEditor", stripped, StringComparison.Ordinal);

        Assert.Contains(
            "DocumentTextProvider=\"{Binding ReferenceTextProvider}\"",
            stripped,
            StringComparison.Ordinal);
    }

    /// <summary>
    /// 配套：VM 那一侧真的有这个属性，且它真的走后端取文档。
    ///
    /// 少了这条，上面那条 XAML 断言在「绑到一个不存在的属性」时照样绿 ——
    /// Avalonia 绑定失败是**静默**的（见记忆 avalonia-missing-resource-key-fails-silently
    /// 的同族形态），屏幕上的症状与没接线完全一致。
    /// </summary>
    [Fact]
    public void ViewModel_ExposesTheProviderAndItGoesThroughTheBackend()
    {
        var source = File.ReadAllText(ResolveDesktopFile("ViewModels", "WorkspacePageViewModel.cs"));

        Assert.Contains("ReferenceTextProvider", source, StringComparison.Ordinal);
        // 必须真的问后端，不能是个返回 null 的空壳（那等于没接）。
        Assert.Contains("GetDocumentContentAsync", source, StringComparison.Ordinal);
    }

    private static string ResolveDesktopFile(params string[] parts)
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 10 && dir is not null; i++)
        {
            var candidate = Path.Combine(
                new[] { dir.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            dir = dir.Parent;
        }
        throw new FileNotFoundException(
            "找不到 " + string.Join("/", parts) + "，基准目录 " + AppContext.BaseDirectory);
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }
}
