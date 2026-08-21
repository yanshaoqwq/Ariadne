using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Presenters;
using Avalonia.Controls.Primitives;
using Avalonia.Headless;
using Avalonia.Media;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Controls;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U208-E / U208-G 的**布局判据**：两条都是「像素位置错了」，
/// 所以判据必须落在真实实体化后的 <c>Bounds</c> 上，不能落在「XAML 里有没有那个属性」。
///
/// # 为什么本文件只起**一个** Avalonia 运行时
///
/// 本机 3.8GB 内存。每条用例各起一个 headless 会话会把 testhost 静默 OOM 掉
/// （症状：日志停在「正在启动测试执行」不动）。沿用
/// <see cref="ReadingMarkdownRenderTests"/> 已验证过的组合：
/// **一个会话 + PerTest 隔离**。⚠️ 别改成 PerAssembly，那条路那边验证过是死的。
/// </summary>
public sealed class AiPanelAndDiagnosticLayoutSession : IDisposable
{
    public AiPanelAndDiagnosticLayoutSession() =>
        Session = HeadlessUnitTestSession.StartNew(
            typeof(LayoutHeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);

    public HeadlessUnitTestSession Session { get; }

    public void Dispose() => Session.Dispose();

    private static class LayoutHeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }
}

[Collection("AvaloniaHeadless")]
public sealed class AiPanelAndDiagnosticLayoutTests
    : IClassFixture<AiPanelAndDiagnosticLayoutSession>
{
    private readonly AiPanelAndDiagnosticLayoutSession _session;

    public AiPanelAndDiagnosticLayoutTests(AiPanelAndDiagnosticLayoutSession session) =>
        _session = session;

    /// <summary>
    /// U208-E → **U213-E 之后判据反转**：Auto Mode 这一行**不许再有任何底色**。
    ///
    /// # 本条原来断言什么，为什么现在必须反过来
    ///
    /// 原判据是「选中态琥珀色块的右缘与状态文字之间留 ≥8px 余白」——
    /// 实机测得缺陷版本只有 1px，字压在色块边缘上（U208-E）。那条判据是对的，
    /// 但它的**前提**（这一行有一块琥珀底、里面有一行状态文字）已经被 U213-E 移除：
    /// 用户原话「要做的自动模式悬浮也变成了难看的卡片，做成悬浮文字和开关」，
    /// 于是 `Button.subtle` + `SelectedClass.IsSelected` 换成了
    /// 「`TextBlock` 标签 + `ToggleSwitch`」，底色与状态文字**双双不存在了**。
    ///
    /// ⚠️ **改成反向钉住而不是删掉这条用例**：删掉的话，「哪天有人为了强调开启态
    /// 又给这一行刷个底」不会有任何东西变红，而那正是用户明确否掉的形态。
    /// 本仓已记：产品改了要同批改用例，且正解是**反转判据**、不是移除断言。
    ///
    /// # 判据：三条各自不可省
    ///
    /// 1. 这一行里**没有** `Button`（不再是「整块可点」——那是当初长成卡片的根源）；
    /// 2. 有一个 `ToggleSwitch`，且它的 `IsChecked` 真的跟着开启态（否则只是摆件）；
    /// 3. 这一行的任何可视元素都**没有** `Ariadne.AccentLight` 底
    ///    —— 这一条才是「不许再变回卡片」的本体。
    /// </summary>
    [Fact]
    public async Task U213E_AutoModeRow_IsALabelWithASwitchAndCarriesNoFill()
    {
        await RunAsync(async () =>
        {
            var (window, panel) = await OpenAutoModePanelAsync(autoModeEnabled: true);
            try
            {
                // 判据 1：不再是「整块可点」。
                Assert.Empty(panel.GetVisualDescendants().OfType<Button>()
                    .Where(candidate => candidate.Classes.Contains("subtle")));

                // 判据 2：开关在，且真的反映开启态。
                var toggle = Assert.Single(
                    panel.GetVisualDescendants().OfType<ToggleSwitch>());
                Assert.True(
                    toggle.IsChecked,
                    "AutoMode 已启用，而开关停在关的位置 ⇒ IsEnabledRequest 没绑上，"
                    + "开关变成了一个不反映状态的摆件。");

                // 判据 3（本体）：这一行里不许出现强调色底。
                // ⚠️ 取主题资源**必须带 ActualThemeVariant**：不带变体的
                // `FindResource(key)` 在这里返回 null（实测），而「Avalonia 缺资源键
                // 静默失效」会让 Assert.NotNull 把「取资源方式不对」误报成「主题缺键」。
                Assert.True(
                    toggle.TryFindResource(
                        "Ariadne.AccentLight", toggle.ActualThemeVariant, out var fillToken)
                    && fillToken is ISolidColorBrush,
                    "主题里取不到 Ariadne.AccentLight —— 本条的比对基准不存在。");
                var forbidden = ((ISolidColorBrush)fillToken!).Color;

                foreach (var visual in panel.GetVisualDescendants().OfType<Control>())
                {
                    // ToggleSwitch 自己的**开态轨道**就该是强调色，跳过它的内部。
                    if (visual == toggle || toggle.GetVisualDescendants().Contains(visual))
                    {
                        continue;
                    }
                    // ⚠️ `Background` 不在 `Control` 上，它由 `TemplatedControl` /
                    // `Panel` / `Border` 各自单独定义 —— 所以要按类型分别取，
                    // 不能对基类反射一个统一的 BackgroundProperty（那样连编译都过不了）。
                    var background = visual switch
                    {
                        Border border => border.Background,
                        Panel layoutPanel => layoutPanel.Background,
                        TemplatedControl templated => templated.Background,
                        _ => null,
                    };
                    if (background is ISolidColorBrush brush && brush.Color == forbidden)
                    {
                        Assert.Fail(
                            $"{visual.GetType().Name} 又刷上了 Ariadne.AccentLight 底 ⇒ "
                            + "AutoMode 这一行正在变回用户否掉的那块满宽琥珀卡片（U213-E）。");
                    }
                }
            }
            finally
            {
                await CloseAsync(window);
            }
        });
    }

    /// <summary>
    /// U208-G：诊断横幅出现**不得改变页面内容与左栏导航的 Y 起点**。
    ///
    /// # 判据为什么取「同一棵树的两个状态之差」
    ///
    /// 实机测得左栏「作品」高亮块顶边：无横幅 y=255、有横幅 y=301、展开详情 y=342
    /// ⇒ 下推 46px / 再推 41px。代价不是"不好看"：作者点「展开详情」时**他刚点的
    /// 那颗按钮已经移到别处**，下一次点击落到别的控件上（报告作者本人误点到项目菜单）。
    /// 所以判据必须是「横幅可见与不可见两态下，同一个控件的窗口坐标相同」，
    /// 而不是「XAML 里横幅在第几行」——后者换个写法就绿，且不保证真的不推挤。
    ///
    /// # 另外两条是 U181 的镜像
    ///
    /// U181 是「模态弹窗挡不住顶栏」；本条反过来：横幅**不能去挡顶栏**
    /// （否则作者关不掉窗口），也不能盖住左栏导航（那等于把"下推"换成"遮挡"，
    /// 判据过了而问题没解决）。两条都落在实测坐标上。
    /// </summary>
    [Fact]
    public async Task U208G_DiagnosticBanner_DoesNotShiftContentOrCoverChrome()
    {
        await RunAsync(async () =>
        {
            var names = DisplayNameService.LoadDefault();
            var backend = System.Reflection.DispatchProxy
                .Create<IAriadneBackendClient, AlwaysProjectOpenBackend>();
            var viewModel = new MainWindowViewModel(names, backend);
            var window = new Views.MainWindow { DataContext = viewModel, Width = 1200, Height = 800 };
            window.Show();
            await DrainAsync();

            try
            {
                viewModel.ClearDiagnosticCommand.Execute(null);
                await RelayoutAsync(window);
                Assert.False(viewModel.HasDiagnostic);

                var rail = window.GetVisualDescendants().OfType<Border>()
                    .First(border => border.Classes.Contains("app-rail"));
                var pageHost = window.GetVisualDescendants()
                    .OfType<TransitioningContentControl>()
                    .First(host => host.Name == "PageHost");

                var railBaseline = TopInWindow(rail, window);
                var pageBaseline = TopInWindow(pageHost, window);
                // 自检：基线本身必须是个真实布局过的值，否则下面比的是 0 和 0。
                Assert.True(railBaseline > 0 && pageBaseline > 0,
                    $"基线坐标不合理（rail={railBaseline} page={pageBaseline}）—— 布局没跑，判据无效。");

                viewModel.Observe(UserFacingError.FromException(
                    BackendException.FromIpcPayload("validation", "u208g probe detail")));
                await RelayoutAsync(window);
                Assert.True(viewModel.HasDiagnostic);

                var banner = window.GetVisualDescendants().OfType<Border>()
                    .First(border => border.Name == "DiagnosticBanner");
                // 自检：横幅真的量出高度了，否则「不推挤」是因为它压根没显示。
                Assert.True(banner.IsVisible && banner.Bounds.Height > 8,
                    $"横幅没渲染出高度（IsVisible={banner.IsVisible} h={banner.Bounds.Height}）"
                    + " —— 此时「Y 不变」是假绿。");

                Assert.True(
                    Math.Abs(TopInWindow(rail, window) - railBaseline) <= 0.5,
                    $"横幅出现把左栏导航从 y={railBaseline} 推到了 "
                    + $"y={TopInWindow(rail, window)}（实机缺陷版本下推 46px，U208-G）。");
                Assert.True(
                    Math.Abs(TopInWindow(pageHost, window) - pageBaseline) <= 0.5,
                    $"横幅出现把内容区从 y={pageBaseline} 推到了 "
                    + $"y={TopInWindow(pageHost, window)}（U208-G）。");

                // U181 镜像 1：不得压住顶栏（窗口控制键在第 1 行，高 54）。
                var bannerTopLeft = banner.TranslatePoint(default, window);
                Assert.NotNull(bannerTopLeft);
                Assert.True(bannerTopLeft!.Value.Y >= 54,
                    $"横幅顶边 y={bannerTopLeft.Value.Y} 压到了顶栏（54px）—— 会挡住关闭/最大化键。");
                // U181 镜像 2：不得盖住左栏导航。
                var railRight = TopLeftInWindow(rail, window).X + rail.Bounds.Width;
                Assert.True(bannerTopLeft.Value.X >= railRight - 0.5,
                    $"横幅左边 x={bannerTopLeft.Value.X} 盖住了左栏（右缘 x={railRight}）——"
                    + "那是把「下推」换成「遮挡」，判据过了而问题没解决。");
            }
            finally
            {
                window.Content = null;
                window.Close();
                await DrainAsync();
            }
        });
    }

    private static double TopInWindow(Control control, Window window) =>
        TopLeftInWindow(control, window).Y;

    private static Point TopLeftInWindow(Control control, Window window)
    {
        var point = control.TranslatePoint(default, window);
        Assert.NotNull(point);
        return point!.Value;
    }

    private static async Task RelayoutAsync(Window window)
    {
        await DrainAsync();
        window.UpdateLayout();
        await DrainAsync();
    }

    // ---------- 宿主与桩 ----------

    /// <summary>
    /// 起一个只装 <see cref="ProjectAiPanel"/> 的窗口。
    ///
    /// 宽度取 318：实机测得 AutoMode 按钮约 318px 宽（色块 293px + 宿主内边距），
    /// 用同一量级才能复现"字顶到边"的那个几何关系。
    /// </summary>
    private static async Task<(Window Window, ProjectAiPanel Panel)> OpenAutoModePanelAsync(
        bool autoModeEnabled)
    {
        var names = DisplayNameService.LoadDefault();
        var backend = System.Reflection.DispatchProxy
            .Create<IAriadneBackendClient, AlwaysProjectOpenBackend>();
        var automation = new ProjectAutomationState(names, backend);
        automation.ApplyBackendValue(autoModeEnabled);

        var panel = new ProjectAiPanel
        {
            DataContext = new AutoModeHostStub(automation, names),
        };
        var window = new Window
        {
            Width = 318,
            Height = 220,
            Content = panel,
        };
        window.Show();
        await DrainAsync();
        window.UpdateLayout();
        await DrainAsync();
        return (window, panel);
    }

    /// <summary>
    /// 面板只用到 <c>ProjectAutomation</c> 与几个文案属性。
    /// <c>x:CompileBindings="False"</c> 下缺失的绑定只是空值，不会抛。
    /// </summary>
    private sealed class AutoModeHostStub
    {
        private readonly DisplayNameService _names;

        public AutoModeHostStub(ProjectAutomationState automation, DisplayNameService names)
        {
            ProjectAutomation = automation;
            _names = names;
        }

        public ProjectAutomationState ProjectAutomation { get; }

        public string ProjectAiMessage { get; set; } = string.Empty;

        public string ProjectAiPlaceholder => _names.Text("ui.works.project_ai.placeholder");

        public string ProjectAiText => _names.Text("ui.works.project_ai");
    }

    /// <summary>
    /// <see cref="ProjectAutomationState"/> 只在 <c>HasProjectRoot</c> 为真时
    /// 才让 ToggleCommand 可用；此外本用例不碰任何后端调用。
    /// </summary>
    public class AlwaysProjectOpenBackend : System.Reflection.DispatchProxy
    {
        protected override object? Invoke(System.Reflection.MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }
            throw new NotSupportedException(targetMethod?.Name ?? "unknown");
        }
    }

    /// <summary>
    /// 在共用的 headless 会话里跑用例体。
    ///
    /// ⚠️ **必须用有返回值的那个 <c>Dispatch</c> 重载**（<c>Func&lt;Task&lt;T&gt;&gt;</c>）。
    /// 我在本文件第一版上原样踩了这个坑：写成 `Dispatch(body, default)`
    /// （`Func&lt;Task&gt;` 重载）之后，**整个用例体一次都没执行**，
    /// 连 `Assert.Fail(...)` 都报绿 —— 我先后用「变异（摘掉修复）」和
    /// 「把阈值改成 1000」两次探针才定位到，白跑三轮。
    /// `ReadingMarkdownRenderTests:386` 与 `ControlSurfaceThemingTests`
    /// **都已经把这个坑写在注释里了**，而我抄了它们的用例体、helper 自己凭印象写。
    /// ⇒ 复用先例时，先例的**前提写在它的注释里**，不会跟着被抄的那段代码走。
    /// 别「顺手简化」回 `Dispatch(body, ct)`。
    /// </summary>
    private Task RunAsync(Func<Task> body) =>
        _session.Session.Dispatch(async () =>
        {
            await body();
            return true;
        }, CancellationToken.None);

    /// <summary>把布局与渲染两级优先级都放空，确保 Bounds 已经是排布后的值。</summary>
    private static async Task DrainAsync()
    {
        for (var i = 0; i < 16; i++)
        {
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Render);
        }
    }

    private static async Task CloseAsync(Window window)
    {
        window.Close();
        await Task.Yield();
    }
}
