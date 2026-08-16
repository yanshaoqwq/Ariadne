using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Threading;
using Avalonia.VisualTree;
using AvaloniaEdit;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U151：同一份正文在阅读态与修改态必须是**同一套排版**。
///
/// 缺陷形态：字体族与字号都对齐了，唯独漏了行高——阅读态
/// （`SelectableTextBlock.reading`）是 30px，修改态（`ae:TextEditor`）停在
/// AvaloniaEdit 默认的 `LineHeightFactor=1.16` ⇒ 约 20.2px。点一下「修改」
/// 整篇行距收紧约三分之一、一屏 20 行累计位移近 200px。
/// 用户以为自己滚动位置乱了，其实是版面换了。
///
/// **判据落在「同一段文本在两态下的总高是否接近」**，而不是
/// 「`LineHeightFactor` 等于 1.7204」——后者只是把一个魔数抄进测试：
/// `Ariadne.Font.Reading` 是 CJK 衬线 fallback 链，换机器落到不同字体时
/// 度量行高就变，写死系数的用例会在别的机器上反过来拦住正确的实现。
///
/// 两态的高度都从**真实 WorksPageView 里的真实控件**上量，不在测试里镜像一份
/// 生产的反解逻辑——镜像公式会跟着生产代码漂移，且缺陷若出在「算对了但没赋值」
/// 那一步，镜像版照样全绿。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class ReadingEditingParityTests
{
    /// 探针正文的行数。20 行 ≈ 一屏，与审查报告里的实测规模一致。
    private const int ProbeLineCount = 20;

    /// <summary>
    /// **主用例**：同样 20 行正文，阅读态与修改态的渲染总高差不得超过 5%。
    ///
    /// 缺陷版本下修改态约为阅读态的 67%（差 33%），必红。
    /// </summary>
    [Fact]
    public async Task SameParagraphs_RenderToNearlyEqualHeight_InBothModes()
    {
        await RunHeadlessAsync(async () =>
        {
            var readingPerLine = await MeasureReadingLineHeightAsync();
            var editingPerLine = await MeasureEditorLineHeightAsync();

            // 前置：两边都真的量到了高度，否则这条用例在比两个 0。
            Assert.True(readingPerLine > 0, "阅读态没有量到行高，用例本身失效");
            Assert.True(editingPerLine > 0, "修改态没有量到行高，用例本身失效");

            var readingTotal = readingPerLine * ProbeLineCount;
            var editingTotal = editingPerLine * ProbeLineCount;
            var drift = Math.Abs(readingTotal - editingTotal) / readingTotal;

            Assert.True(
                drift <= 0.05,
                $"两态排版必须一致：阅读态 {ProbeLineCount} 行共 {readingTotal:F2}px "
                    + $"（每行 {readingPerLine:F4}）vs 修改态 {editingTotal:F2}px"
                    + $"（每行 {editingPerLine:F4}），相差 {drift:P1}，上限 5%。"
                    + "行高不统一时「切换视图保留阅读位置」在数学上做不到。");
        });
    }

    /// <summary>
    /// 单行高必须等于主题 token，而不是某个巧合数字。
    ///
    /// 有了总高用例还要这一条：总高可能被「行数凑巧不同 + 每行高度不同」抵消到看起来接近。
    /// 期望值从 `Ariadne.Reading.LineHeight` 现取，测试里不出现 30。
    /// </summary>
    [Fact]
    public async Task EditorLineHeight_EqualsTheSharedReadingToken()
    {
        await RunHeadlessAsync(async () =>
        {
            var (window, view) = await OpenAsync(BuildProbeText(ProbeLineCount), editMode: true);
            try
            {
                var expected = ReadingLineHeightToken(view);
                var actual = FindEditor(view).TextArea.TextView.DefaultLineHeight;

                // 容差 0.5px：字体度量是浮点数，反解后不可能位位相等；
                // 缺陷版本差 9.77px，这个容差拦得住。
                Assert.True(
                    Math.Abs(actual - expected) <= 0.5,
                    $"编辑器单行高 {actual:F4}px 必须对齐阅读态 token {expected:F4}px");
            }
            finally
            {
                await CloseAsync(window);
            }
        });
    }

    /// <summary>
    /// 阅读态那一侧也必须引 token，而不是再写一个 30。
    ///
    /// 守的是「单一定义处」：U151 的成因就是同一个数被两处各写一遍、其中一处忘了改。
    /// 若有人把样式里的 `LineHeight` 改回字面量，上面两条仍会全绿（值相同），
    /// 但漂移的种子已经埋回去了——所以这条同时断言渲染值与样式来源。
    /// </summary>
    [Fact]
    public async Task ReadingStyle_TakesLineHeightFromTheSharedToken()
    {
        await RunHeadlessAsync(async () =>
        {
            var (window, view) = await OpenAsync(BuildProbeText(ProbeLineCount), editMode: false);
            try
            {
                var expected = ReadingLineHeightToken(view);
                var block = FindReadingBlocks(view).FirstOrDefault();
                Assert.NotNull(block);

                Assert.Equal(expected, block!.LineHeight);
            }
            finally
            {
                await CloseAsync(window);
            }
        });

        // 值相等时行为断言看不出区别，只有源码断言能拦住「又抄了一遍」。
        var theme = File.ReadAllText(ResolveDesktopFile("Resources", "Styles", "AriadneTheme.axaml"));
        var readingStyles = ExtractLineHeightStyleBlocks(theme, ".reading");
        Assert.NotEmpty(readingStyles);
        foreach (var style in readingStyles)
        {
            Assert.Contains("Ariadne.Reading.LineHeight", style, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// **变异点**：系数必须由「当前字体的度量」反解，不能是写死的常量。
    ///
    /// 做法：把字号翻倍，并换一个新的 `TextEditorOptions` 强制 AvaloniaEdit
    /// 丢弃缓存的字体度量（`OnOptionsChanged` 会以 null 属性名走一遍
    /// `InvalidateDefaultTextMetrics`）。这样度量确实变了：
    /// - 若实现按度量反解 ⇒ 系数随之变小，单行高仍落回 token；
    /// - 若实现写死了系数（例如 1.7204）⇒ 单行高跟着字号翻倍，彻底跑偏。
    ///
    /// 这一条覆盖的是「换机器落到不同字体」那个真实风险——headless 里换不了字体，
    /// 但「度量变了系数要跟着变」是同一个性质。
    /// </summary>
    [Fact]
    public async Task EditorLineHeight_IsDerivedFromCurrentFontMetrics_NotAHardcodedFactor()
    {
        await RunHeadlessAsync(async () =>
        {
            var (window, view) = await OpenAsync(BuildProbeText(ProbeLineCount), editMode: true);
            try
            {
                var expected = ReadingLineHeightToken(view);
                var editor = FindEditor(view);
                var baselineMetric = CurrentFontMetricHeight(editor);
                Assert.True(baselineMetric > 0, "基线度量为 0，用例本身失效");

                editor.FontSize *= 2;
                // 换 Options 实例强制丢弃缓存度量：只改字号时是否重算依赖 AvaloniaEdit
                // 内部实现，这里不做假设，直接走它明确会重算的那条路。
                editor.Options = new TextEditorOptions();
                await DrainAsync();
                window.UpdateLayout();
                await DrainAsync();

                var scaledMetric = CurrentFontMetricHeight(editor);
                // 前置：度量真的变了，否则这条用例没在测反解。
                Assert.True(
                    scaledMetric > baselineMetric * 1.5,
                    $"字号翻倍后字体度量高应显著变大（原 {baselineMetric:F4} → 现 {scaledMetric:F4}），"
                        + "没变说明这条用例的前提不成立");

                var actual = editor.TextArea.TextView.DefaultLineHeight;
                Assert.True(
                    Math.Abs(actual - expected) <= 0.5,
                    $"度量变化后单行高仍应落在 token {expected:F4}px 上，实际 {actual:F4}px——"
                        + "跑偏说明系数是写死的，而不是按当前字体反解的");
            }
            finally
            {
                await CloseAsync(window);
            }
        });
    }

    /// <summary>
    /// 源码守卫：不得给 `ae:TextEditor` 内联写 `LineHeight`。
    ///
    /// 这是本编号「最容易改错」的那条路——AvaloniaEdit 自己排版、**不读**
    /// `TextBlock.LineHeight`，内联写了**静默无效**却看起来已经处理过了。
    /// 行为用例拦不住它（写了也没效果，高度不会变），只有源码断言能拦。
    /// </summary>
    [Fact]
    public void DocumentEditorMarkup_DoesNotSetLineHeightInline()
    {
        var markup = File.ReadAllText(ResolveDesktopFile("Views", "WorksPageView.axaml"));
        var start = markup.IndexOf("<ae:TextEditor", StringComparison.Ordinal);
        Assert.True(start > 0, "找不到 ae:TextEditor 声明，用例本身失效");
        var end = markup.IndexOf("/>", start, StringComparison.Ordinal);
        Assert.True(end > start, "ae:TextEditor 声明没有闭合，用例本身失效");

        Assert.DoesNotContain("LineHeight", markup[start..end], StringComparison.Ordinal);
    }

    /// <summary>
    /// U152 的一部分：死样式 `TextBox.document-editor` 必须已删除。
    ///
    /// 它是 U151 被漏掉的**直接原因**——里面设了 `LineHeight=30`，
    /// 让人以为正文编辑器行高已统一，而真正在用的 `ae:TextEditor` 根本没设。
    /// 死样式会冒充「已完成的工作」，危害不在渲染而在判断。
    ///
    /// 断言只针对**选择器与类挂载**，不禁止注释里提到这个名字：
    /// 记录「这里曾有一条死样式、为什么删」正是防止它被补回来的手段。
    /// </summary>
    [Fact]
    public void DeadDocumentEditorStyle_IsGone()
    {
        const string deadClass = "document-editor";
        var theme = File.ReadAllText(ResolveDesktopFile("Resources", "Styles", "AriadneTheme.axaml"));

        Assert.DoesNotContain($"Selector=\"TextBox.{deadClass}\"", theme, StringComparison.Ordinal);

        // 反向对照：全仓也不该有人挂这个类（若有，说明删早了、删错了）。
        foreach (var file in EnumerateDesktopSources())
        {
            var content = File.ReadAllText(file);
            Assert.DoesNotContain($"Classes=\"{deadClass}\"", content, StringComparison.Ordinal);
            Assert.DoesNotContain($"Classes.Add(\"{deadClass}\")", content, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// 阅读态每行实际占多高：量**两段不同行数的正文的高度差**。
    ///
    /// `(H(20 行) − H(10 行)) / 10` 天然消掉了块的 padding / margin / 首末行额外留白，
    /// 得到的就是纯粹的「多一行多占多少」。
    ///
    /// 为什么不用更直接的做法：
    /// - 读 `LineHeight` 属性 ⇒ 那是**声明值**，缺陷若出在「声明了但没生效」
    ///   （被后声明的通用样式盖掉，U154 那一类）就看不出来；
    /// - 除以 `TextLayout.TextLines.Count` ⇒ 要依赖 `TextLayout` 这个内部渲染细节，
    ///   而差分法只用公开的 `Bounds.Height`，换 Avalonia 版本也不会失效。
    /// </summary>
    private static async Task<double> MeasureReadingLineHeightAsync()
    {
        var tall = await MeasureReadingBlockHeightAsync(ProbeLineCount);
        var shortHalf = await MeasureReadingBlockHeightAsync(ProbeLineCount / 2);

        var deltaLines = ProbeLineCount - (ProbeLineCount / 2);
        Assert.True(
            tall > shortHalf,
            $"{ProbeLineCount} 行必须比 {ProbeLineCount / 2} 行高（实测 {tall:F2} vs {shortHalf:F2}），"
                + "否则这条用例没在测行高");

        return (tall - shortHalf) / deltaLines;
    }

    private static async Task<double> MeasureReadingBlockHeightAsync(int lineCount)
    {
        var (window, view) = await OpenAsync(BuildProbeText(lineCount), editMode: false);
        try
        {
            var block = FindReadingBlocks(view).FirstOrDefault();
            Assert.NotNull(block);
            var height = block!.Bounds.Height;
            Assert.True(height > 0, "阅读块还没完成布局，量不到高度");
            return height;
        }
        finally
        {
            await CloseAsync(window);
        }
    }

    /// <summary>修改态每行实际占多高：AvaloniaEdit 自己排版，这个值就是它的行高。</summary>
    private static async Task<double> MeasureEditorLineHeightAsync()
    {
        var (window, view) = await OpenAsync(BuildProbeText(ProbeLineCount), editMode: true);
        try
        {
            var editor = FindEditor(view);
            // 前置：编辑器确实拿到了同一份正文，否则两态比的不是同一段文本。
            Assert.Equal(ProbeLineCount, editor.Document?.LineCount);
            return editor.TextArea.TextView.DefaultLineHeight;
        }
        finally
        {
            await CloseAsync(window);
        }
    }

    /// <summary>反解出字体自身的度量行高：DefaultLineHeight 已含当前系数。</summary>
    private static double CurrentFontMetricHeight(TextEditor editor) =>
        editor.TextArea.TextView.DefaultLineHeight / editor.Options.LineHeightFactor;

    private static double ReadingLineHeightToken(Control control)
    {
        Assert.True(
            control.TryFindResource("Ariadne.Reading.LineHeight", control.ActualThemeVariant, out var resource),
            "主题里找不到 Ariadne.Reading.LineHeight，期望值无从取得");
        return Assert.IsType<double>(resource);
    }

    private static TextEditor FindEditor(WorksPageView view)
    {
        var editor = view.FindControl<TextEditor>("DocumentEditor");
        Assert.NotNull(editor);
        return editor!;
    }

    private static IEnumerable<SelectableTextBlock> FindReadingBlocks(WorksPageView view) =>
        view.GetVisualDescendants()
            .OfType<SelectableTextBlock>()
            .Where(block => block.Classes.Contains("reading")
                            && block.DataContext is DocumentBlockViewModel);

    /// <summary>
    /// 把 ViewModel 先置成「树已加载 + 已打开文档 + 指定模式」再挂进窗口，
    /// 这样首次布局的绑定求值就能读到 ShowDocumentChrome=true，
    /// 正文区（含 ae:TextEditor）才真正参与布局。顺序抄自 WorksReadModeScrollTests。
    /// </summary>
    private static async Task<(Window Window, WorksPageView View)> OpenAsync(string text, bool editMode)
    {
        var viewModel = new WorksPageViewModel(DisplayNameService.LoadDefault(), NoopBackend.Create());
        var type = typeof(WorksPageViewModel);
        type.GetField("_currentDocumentId", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(viewModel, "chapters/chapter-01.md");
        SetWorksTreeStateToContent(viewModel);
        viewModel.IsEditMode = editMode;
        viewModel.DocumentContent = text;

        var view = new WorksPageView { DataContext = viewModel };
        var window = new Window { Width = 1400, Height = 900, Content = view };
        window.Show();
        await DrainAsync();
        // 显式跑一遍布局：本用例的判据是**实测高度**，而高度只有 arrange 之后才有意义。
        // 光靠 drain 依赖 headless 下的渲染节拍，显式一次更确定。
        window.UpdateLayout();
        await DrainAsync();
        return (window, view);
    }

    private static void SetWorksTreeStateToContent(WorksPageViewModel viewModel)
    {
        var type = typeof(WorksPageViewModel);
        var stateType = type.GetNestedType("WorksTreeLoadState", BindingFlags.NonPublic)
                        ?? type.Assembly.GetTypes().First(candidate => candidate.Name == "WorksTreeLoadState");
        type.GetField("_worksTreeState", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(viewModel, Enum.Parse(stateType, "Content"));
    }

    /// <summary>
    /// 20 行正文，每行 21 个汉字——刻意短到**不会自动换行**
    /// （版心 576px / 16px ≈ 36 字），这样「文档行数」与「视觉行数」一致，两态才可比。
    /// 若让它换行，阅读态与编辑器的断行算法差异会混进高度差里，
    /// 那时用例失败原因就说不清是行高还是断行了。
    /// </summary>
    private static string BuildProbeText(int lineCount) =>
        string.Join(
            "\n",
            Enumerable.Range(0, lineCount).Select(i => $"第{i:D2}行" + new string('文', 18)));

    private static IEnumerable<string> EnumerateDesktopSources()
    {
        var root = ResolveDesktopRoot();
        foreach (var pattern in new[] { "*.axaml", "*.cs" })
        {
            foreach (var file in Directory.EnumerateFiles(root, pattern, SearchOption.AllDirectories))
            {
                // obj/bin 里是生成物，会把已删掉的样式以旧副本形式带回来。
                var sep = Path.DirectorySeparatorChar;
                if (file.Contains($"{sep}obj{sep}", StringComparison.Ordinal)
                    || file.Contains($"{sep}bin{sep}", StringComparison.Ordinal))
                {
                    continue;
                }
                yield return file;
            }
        }
    }

    /// <summary>取出所有「选择器含指定片段且设了行高」的 Style 块原文。</summary>
    private static List<string> ExtractLineHeightStyleBlocks(string theme, string selectorFragment)
    {
        var blocks = new List<string>();
        var cursor = 0;
        while (true)
        {
            var start = theme.IndexOf("<Style Selector=", cursor, StringComparison.Ordinal);
            if (start < 0)
            {
                break;
            }
            var end = theme.IndexOf("</Style>", start, StringComparison.Ordinal);
            if (end < 0)
            {
                break;
            }

            var block = theme[start..end];
            var selectorEnd = block.IndexOf('>');
            if (selectorEnd > 0
                && block[..selectorEnd].Contains(selectorFragment, StringComparison.Ordinal)
                && block.Contains("LineHeight", StringComparison.Ordinal))
            {
                blocks.Add(block);
            }
            cursor = end + 1;
        }
        return blocks;
    }

    private static string ResolveDesktopRoot()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return Path.Combine(dir!, "Ariadne.Desktop");
    }

    private static string ResolveDesktopFile(params string[] segments) =>
        Path.Combine(new[] { ResolveDesktopRoot() }.Concat(segments).ToArray());

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

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
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
