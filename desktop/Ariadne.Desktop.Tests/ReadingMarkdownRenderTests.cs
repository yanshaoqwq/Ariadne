using System.Reflection;
using System.Text;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Documents;
using Avalonia.Headless;
using Avalonia.Media;
using Avalonia.Threading;
using Avalonia.VisualTree;
using AvaloniaEdit;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Controls;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 本文件 6 条用例**共用一个 Avalonia 运行时**的宿主。
///
/// # 为什么必须共用（本机 3.8GB 内存的硬约束）
///
/// 原实现在每条用例里 `HeadlessUnitTestSession.StartNew(...)` + `using` —— 6 条
/// 就是 6 次「起一整个 Avalonia 运行时再拆掉」。本机实测**跑不完**：可用内存
/// 掉到约 200MB 后 testhost 被静默杀掉，vstest 父进程挂住不返回，
/// 日志停在「正在启动测试执行」⇒ 极易误判成「用例挂死」或「还在跑」。
///
/// ⚠️ 区分 OOM 与挂死的判据（别再走一遍我走过的弯路）：
/// **OOM 时可用内存会跌到 200MB 附近、随后 testhost 消失、内存回弹**；
/// 挂死时内存平稳。我用这条判据确认了是 OOM 而非挂死。
///
/// # 隔离等级仍取 PerTest（**试过 PerAssembly，会挂死**）
///
/// 直觉上「共用会话就该配 PerAssembly」，实测**不行**：本机跑 6 条时
/// 常驻内存稳在约 2GB、8 分钟无任何进展，符合「挂死」而非 OOM 的特征
/// （OOM 会先跌到约 200MB 再让 testhost 消失、内存回弹；挂死时内存平稳）。
/// 这正是项目记忆里那条「headless 挂不挂看实体化顺序」——
/// App 级状态跨用例不重置时，某条用例的实体化顺序会把运行时卡住。
///
/// ⇒ 现在的组合是**一个会话 + PerTest 隔离**：省掉 6 次「起/拆运行时」的开销，
/// 同时让每次 Dispatch 重建 App，既避免跨用例污染、也让内存逐条回收。
/// ⚠️ 别再把它「优化」成 PerAssembly，那条路已经验证过是死的。
///
/// # 为什么用 IClassFixture 而不是 static 字段
///
/// static 字段没人 Dispose，会留下一个 Avalonia 会话线程；若它不是后台线程，
/// 进程退不出 —— 那会把「测试跑不完」这个症状**原样复现出来**，只是换了成因。
/// <c>IClassFixture</c> 的语义正好是「本类所有用例共用一个实例、跑完即 Dispose」。
/// </summary>
public sealed class ReadingMarkdownRenderSession : IDisposable
{
    public ReadingMarkdownRenderSession() =>
        Session = HeadlessUnitTestSession.StartNew(
            typeof(SharedHeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);

    public HeadlessUnitTestSession Session { get; }

    public void Dispose() => Session.Dispose();

    private static class SharedHeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }
}

/// <summary>
/// U203：阅读态必须**渲染** Markdown，而不是把 `#` / `**` / `>` / 三连短横线
/// 原样印在稿纸上。
///
/// # 判据为什么必须落在渲染产物上
///
/// 缺陷版本里**没有任何一行代码是错的**：没有失败的解析、没有异常、没有 TODO，
/// 缺的是一整个环节。所以「解析器能解析出 Heading」这种断言毫无价值——
/// 缺陷版本压根不调用解析器，那种用例在修复前后**都是绿的**。
/// 本文件的主判据是：**阅读态真实视觉树里，可见文本不含 `#`，而标题文字仍在**。
/// 这一条直接对应作者眼睛看到的东西，且摘掉接线必红（变异测试已验证）。
///
/// # 两态刻意不一致
///
/// 修改态（`ae:TextEditor`）编辑的是**源码**，那里必须仍看得到 `#`。
/// 下面 <see cref="EditMode_StillShowsRawMarkdownSource"/> 钉住这一点——
/// 否则下一个人会为了「一致」把渲染也塞进编辑器，那会让作者改不动标记。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class ReadingMarkdownRenderTests : IClassFixture<ReadingMarkdownRenderSession>
{
    private readonly ReadingMarkdownRenderSession _session;

    public ReadingMarkdownRenderTests(ReadingMarkdownRenderSession session) => _session = session;

    /// <summary>取材于 U203 报告里实拍的那一章：标题 + 加粗 + 引用 + 分隔线。</summary>
    private const string MarkdownBody = """
        # 第1章 雨夜的来信

        她把信折成很小的一块，塞进袖口。**雨还在下**，屋檐的水线连成一片。

        > 我知道你会来。这句话她读了三遍。

        ---

        - 第一件事：把灯熄掉
        - 第二件事：等他敲门

        ## 后半夜

        窗外传来*极轻*的脚步声。
        """;

    /// <summary>
    /// **主用例**：阅读态渲染出的可见文本里不得有 `#`，且标题文字必须仍在。
    ///
    /// 缺陷版本下第一块的文本就是原始正文，含 `# 第1章 雨夜的来信`，必红。
    /// </summary>
    [Fact]
    public async Task ReadMode_RendersHeadings_WithoutLeavingHashMarkersOnThePage()
    {
        await RunHeadlessAsync(async () =>
        {
            var (window, view) = await OpenAsync(MarkdownBody, editMode: false);
            try
            {
                var visible = VisibleReadingText(view);

                Assert.DoesNotContain("#", visible, StringComparison.Ordinal);
                Assert.Contains("第1章 雨夜的来信", visible, StringComparison.Ordinal);
                Assert.Contains("后半夜", visible, StringComparison.Ordinal);
                // 正文一个字都不能丢：渲染标记时**吃掉字符**比不渲染更糟——
                // 不渲染只是丑，吃字符是作者的稿子少了一截而他不会立刻发现。
                Assert.Contains("她把信折成很小的一块", visible, StringComparison.Ordinal);
                Assert.Contains("窗外传来极轻的脚步声", visible, StringComparison.Ordinal);
            }
            finally
            {
                await CloseAsync(window);
            }
        });
    }

    /// <summary>
    /// 标题必须**看起来**像标题：字号与字重都要与正文不同。
    ///
    /// 只断言「没有 `#`」是不够的——把行首井号剥掉、字号不变（U203 报告里的路 C）
    /// 同样能过那一条，但那会让长篇的章节层级整体丢失。
    /// </summary>
    [Fact]
    public async Task ReadMode_HeadingIsVisuallyDistinctFromBodyText()
    {
        await RunHeadlessAsync(async () =>
        {
            var (window, view) = await OpenAsync(MarkdownBody, editMode: false);
            try
            {
                var heading = ReadingBlocks(view)
                    .FirstOrDefault(block => block.Classes.Contains("md-h1"));
                var paragraph = ReadingBlocks(view)
                    .FirstOrDefault(block => block.Classes.Contains("md-paragraph"));

                Assert.True(heading is not null, "阅读态没有渲染出一级标题控件");
                Assert.True(paragraph is not null, "阅读态没有渲染出正文段落控件");

                Assert.True(
                    heading!.FontSize > paragraph!.FontSize,
                    $"标题字号 {heading.FontSize} 必须大于正文 {paragraph.FontSize}——"
                        + "只剥井号不分级，等于把章节层级抹平（U203 的路 C，已否决）");
                Assert.NotEqual(paragraph.FontWeight, heading.FontWeight);

                // 字号必须来自主题 token，不能是控件里写死的数字。
                Assert.True(
                    heading.TryFindResource(
                        "Ariadne.Reading.Heading1Size", heading.ActualThemeVariant, out var token)
                    && token is double expected
                    && Math.Abs(heading.FontSize - expected) < 0.01,
                    "一级标题字号必须取 Ariadne.Reading.Heading1Size");
            }
            finally
            {
                await CloseAsync(window);
            }
        });
    }

    /// <summary>
    /// 修改态仍显示源码。**这不是缺陷，是产品决策**：那边在改稿。
    /// </summary>
    [Fact]
    public async Task EditMode_StillShowsRawMarkdownSource()
    {
        await RunHeadlessAsync(async () =>
        {
            var (window, view) = await OpenAsync(MarkdownBody, editMode: true);
            try
            {
                var editor = view.FindControl<TextEditor>("DocumentEditor");
                Assert.NotNull(editor);
                var text = editor!.Document?.Text ?? string.Empty;

                Assert.Contains("# 第1章", text, StringComparison.Ordinal);
                Assert.Contains("**雨还在下**", text, StringComparison.Ordinal);
                Assert.Contains("> 我知道你会来", text, StringComparison.Ordinal);
            }
            finally
            {
                await CloseAsync(window);
            }
        });
    }

    /// <summary>阅读态所有已实体化正文片段的可见文本（按视觉顺序拼接）。</summary>
    private static string VisibleReadingText(WorksPageView view)
    {
        var builder = new StringBuilder();
        foreach (var block in ReadingBlocks(view))
        {
            builder.AppendLine(SegmentText(block));
        }
        return builder.ToString();
    }

    /// <summary>
    /// 引用块、分隔线、列表都必须真的成为版面元素，而不是留着 `>` / 三连短横 / `-`。
    ///
    /// 判据取「有那个控件 + 标记字符不在可见文本里」两条同时成立：
    /// 只看控件存在，剥不干净标记也能过；只看文本，把标记删掉不渲染也能过。
    /// </summary>
    [Fact]
    public async Task ReadMode_RendersQuoteRuleAndList_AsLayoutNotMarkers()
    {
        await RunHeadlessAsync(async () =>
        {
            var (window, view) = await OpenAsync(MarkdownBody, editMode: false);
            try
            {
                var visible = VisibleReadingText(view);
                Assert.DoesNotContain(">", visible, StringComparison.Ordinal);
                Assert.DoesNotContain("**", visible, StringComparison.Ordinal);
                Assert.Contains("我知道你会来", visible, StringComparison.Ordinal);
                Assert.Contains("第一件事：把灯熄掉", visible, StringComparison.Ordinal);

                var borders = view.GetVisualDescendants().OfType<Border>().ToList();
                Assert.Contains(borders, border => border.Classes.Contains("md-quote"));
                Assert.Contains(borders, border => border.Classes.Contains("md-rule"));

                // 引用文字与正文必须有色差，否则「这是引用」在视觉上不成立。
                var quote = ReadingBlocks(view).First(b => b.Classes.Contains("md-quote-text"));
                var body = ReadingBlocks(view).First(b => b.Classes.Contains("md-paragraph"));
                Assert.NotEqual(body.Foreground?.ToString(), quote.Foreground?.ToString());

                // 列表项的圆点是独立的标记控件（不在可选中文本里 ⇒ Ctrl+C 不会带走它）。
                var markers = view.GetVisualDescendants()
                    .OfType<TextBlock>()
                    .Where(t => t.Classes.Contains("md-list-marker"))
                    .ToList();
                Assert.True(markers.Count >= 2, $"两条列表项应有两个标记，实得 {markers.Count}");
            }
            finally
            {
                await CloseAsync(window);
            }
        });
    }

    /// <summary>
    /// 加粗必须真的加粗：`**雨还在下**` 那一段里应存在一个 Bold 的 Run，
    /// 且 `*` 一个都不剩。
    ///
    /// 这条守的是行内层。段落走的是 Inlines 那条路（与纯文本段落**不是**同一条
    /// 代码路径），只测块级会漏掉整整一半。
    /// </summary>
    [Fact]
    public async Task ReadMode_BoldRunsAreActuallyBold()
    {
        await RunHeadlessAsync(async () =>
        {
            var (window, view) = await OpenAsync(MarkdownBody, editMode: false);
            try
            {
                var runs = ReadingBlocks(view)
                    .SelectMany(block => block.Inlines?.OfType<Run>() ?? Enumerable.Empty<Run>())
                    .ToList();

                var bold = runs.FirstOrDefault(run => run.FontWeight == FontWeight.Bold);
                Assert.True(bold is not null, "没有任何加粗 Run —— `**雨还在下**` 没被渲染");
                Assert.Equal("雨还在下", bold!.Text);

                var italic = runs.FirstOrDefault(run => run.FontStyle == FontStyle.Italic);
                Assert.True(italic is not null, "没有任何斜体 Run —— `*极轻*` 没被渲染");
                Assert.Equal("极轻", italic!.Text);
            }
            finally
            {
                await CloseAsync(window);
            }
        });
    }

    /// <summary>
    /// 段内换行必须走 <see cref="LineBreak"/> inline，**不能把 `\n` 塞进 Run**。
    ///
    /// 项目已知踩坑：AvaloniaEdit 的 `FormattedTextElement` 遇 `\n` 会静默截断，
    /// 「多行塞进一个 inline」是同一种赌注。这里钉住的是可测的那一层：
    /// 每个 Run 的文本都不含 `\n`，而拼起来的可见文本仍保有换行
    /// ⇒ 复制出来的正文行数才是对的（阅读态是可选中复制的）。
    ///
    /// # ⚠️ 本条**刻意不开窗**，别「统一」成上面那几条的 OpenAsync 写法
    ///
    /// 首版是走 `OpenAsync` 的（整个 WorksPageView 挂进 Window 再 UpdateLayout），
    /// 症状：**本机 headless 下这一条挂死**，150s 超时也不返回，而且它会把整批
    /// 6 条一起拖死 —— 表现为「日志停在『正在启动测试执行』」，
    /// 与 OOM **完全同形**（我先误判成 OOM 查了三轮）。
    ///
    /// 逐条单跑确认：另外 5 条各 5~7 秒就绿，只有这一条挂。它与那 5 条的唯一结构差异是
    /// **段落内同时有 `LineBreak` 和格式化 Run**（那 5 条用的正文里，带格式的段落都是单行）。
    ///
    /// **已证实是 headless-only，不是产品缺陷**：用真实生产控件 `MarkdownReaderBlock`
    /// + 真实 `AriadneTheme` 在**真实 X11** 上渲染同一形状，布局正常走完 ——
    /// 段A（含 LineBreak + 加粗）高 60px = 两行 × 30px 行高，与对照段B（纯文本两行）
    /// 完全一致，位图也正常落盘。所以产品是好的，是本机 headless 的文本布局在
    /// 「Inlines 含 LineBreak 且控件在窗口视觉树里」这个组合上转不出来。
    ///
    /// ⇒ 改法是**去掉窗口**：本条的全部判据都是 inline **结构**
    /// （LineBreak 的存在、Run 里没有 `\n`、可见文本的拼接结果），
    /// 这些是 `MarkdownReaderBlock` 建子控件时就定了的，**不需要布局**。
    /// 不开窗既保住了守卫，又绕开了 headless 那个坑。
    /// ⚠️ 不要为了「和别的用例一致」把它改回 OpenAsync —— 那会让整批测试再次挂死。
    /// ⚠️ 也不要改成 `[Fact(Skip = ...)]`：这条守的正是「段落内既有换行又有格式」
    /// 这个最容易出错的组合，Skip 等于让守卫消失。
    /// </summary>
    [Fact]
    public async Task ReadMode_UsesLineBreakInlines_NotNewlinesInsideRuns()
    {
        // 两行一段（中间无空行），第二行带加粗 ⇒ 整段走 Inlines 那条路。
        const string body = "第一行没有格式。\n第二行有**加粗**。";
        await RunHeadlessAsync(() =>
        {
            // 直接建生产控件、不挂窗口：判据在结构层，布局无关（见上面注释）。
            var reader = new MarkdownReaderBlock { Source = body };
            var paragraph = Assert.Single(reader.SelectableSegments);
            var inlines = paragraph.Inlines?.ToList() ?? new List<Inline>();
            Assert.NotEmpty(inlines);

            foreach (var run in inlines.OfType<Run>())
            {
                Assert.DoesNotContain("\n", run.Text ?? string.Empty, StringComparison.Ordinal);
            }
            Assert.Contains(inlines, inline => inline is LineBreak);

            // 加粗真的加粗了（这段走的是 Inlines 那条路，与纯文本段不是同一条代码路径）。
            Assert.Contains(inlines.OfType<Run>(), run => run.FontWeight == FontWeight.Bold);

            // 换行仍在可见文本里 ⇒ SelectedText / 复制拿到的行结构是对的。
            var text = SegmentText(paragraph);
            Assert.Contains("\n", text, StringComparison.Ordinal);
            Assert.Equal("第一行没有格式。\n第二行有加粗。", text);
            return Task.CompletedTask;
        });
    }

    /// <summary>
    /// 片段可见文本。**先 Text 再 Inlines.Text**：用了 Inlines 之后
    /// `TextBlock.Text` 恒为 null（Avalonia 12.0.5 实测），反过来纯文本段落的
    /// Inlines 是空集合。顺序反了会把带格式的段落读成空串，
    /// 于是「不含 `#`」这条断言变成对空串取真——用例会静默失效。
    /// </summary>
    private static string SegmentText(SelectableTextBlock block) =>
        !string.IsNullOrEmpty(block.Text) ? block.Text! : block.Inlines?.Text ?? string.Empty;

    /// <summary>阅读态的正文片段：DataContext 必须是正文块 VM（排除对照栏等其他 reading 文本）。</summary>
    private static IReadOnlyList<SelectableTextBlock> ReadingBlocks(WorksPageView view) =>
        view.GetVisualDescendants()
            .OfType<SelectableTextBlock>()
            .Where(block => block.DataContext is DocumentBlockViewModel)
            .ToList();

    /// <summary>
    /// 把 ViewModel 先置成「树已加载 + 已打开文档 + 指定模式」再挂进窗口。
    /// 顺序抄自 <c>WorksReadModeScrollTests</c>：这样首次布局的绑定求值就能读到
    /// ShowDocumentChrome=true，正文区才真正参与布局。
    /// </summary>
    private static async Task<(Window Window, WorksPageView View)> OpenAsync(string text, bool editMode)
    {
        var viewModel = new WorksPageViewModel(DisplayNameService.LoadDefault(), NoopBackend.Create());
        var type = typeof(WorksPageViewModel);
        type.GetField("_currentDocumentId", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(viewModel, "chapters/chapter-01.md");
        var stateType = type.Assembly.GetTypes().First(candidate => candidate.Name == "WorksTreeLoadState");
        type.GetField("_worksTreeState", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(viewModel, Enum.Parse(stateType, "Content"));
        viewModel.IsEditMode = editMode;
        viewModel.DocumentContent = text;

        var view = new WorksPageView { DataContext = viewModel };
        var window = new Window { Width = 1400, Height = 900, Content = view };
        window.Show();
        await DrainAsync();
        window.UpdateLayout();
        await DrainAsync();
        return (window, view);
    }

    /// <summary>
    /// 在共用的 headless 会话里跑用例体。
    ///
    /// ⚠️ **必须用返回值的那个 <c>Dispatch</c> 重载**（`Func&lt;Task&lt;T&gt;&gt;`）。
    /// `Func&lt;Task&gt;` 那个重载会把断言失败连同异常一起吞掉、测试照样报绿 ——
    /// `ControlSurfaceThemingTests` 的注释记着这个坑：它曾让整整一个文件变成空测，
    /// 连 `Assert.True(false)` 都是绿的。别「顺手简化」成 `Dispatch(body, ct)`。
    /// </summary>
    private Task RunHeadlessAsync(Func<Task> body) =>
        _session.Session.Dispatch(async () =>
        {
            await body();
            return true;
        }, CancellationToken.None);

    private static async Task CloseAsync(Window window)
    {
        window.Content = null;
        window.Close();
        await DrainAsync();
    }

    private static async Task DrainAsync()
    {
        for (var i = 0; i < 16; i++)
        {
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Render);
        }
    }

    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() => Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == $"get_{nameof(IAriadneBackendClient.HasProjectRoot)}")
            {
                return false;
            }

            var returnType = targetMethod?.ReturnType;
            if (returnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (returnType is not null
                && returnType.IsGenericType
                && returnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var inner = returnType.GetGenericArguments()[0];
                var value = inner.IsValueType ? Activator.CreateInstance(inner) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(inner)
                    .Invoke(null, new[] { value });
            }
            return null;
        }
    }
}
