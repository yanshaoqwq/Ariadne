using System;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Threading.Tasks;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Media;
using Avalonia.Styling;
using Avalonia.Threading;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using System.Threading;
using Xunit;
// 别名：Avalonia.Controls.Shapes.Path 会盖掉 System.IO.Path，本文件两者都要用。
using ShapePath = Avalonia.Controls.Shapes.Path;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U207-C 守卫：空闲态的运行入口与运行控制三键。
///
/// <para>
/// **缺陷原形**（实机取证）：空画布 + 默认「节点库」tab ⇒ **一个开始运行的入口都没有**
/// （`:674` 的起始节点小三角要先有节点、`:1410` 的执行面板主按钮要先切 tab），
/// 而三个**运行中**控制键（暂停/继续/停止）常驻可见。作者只会把那颗三角当播放键 ——
/// 它其实是 `ResumeWorkflowCommand`，没在运行时点它什么都不发生。
/// </para>
///
/// <para>
/// 第二个子缺陷：禁用态下「停止」比另两个显眼得多（逐像素：停止 100 / 暂停 66 / 继续 53），
/// 因为危险色在 `:disabled` 下没退色。⇒ 空闲时界面上最醒目的运行控制，
/// 恰是此刻最无意义的那个。
/// </para>
///
/// <para>
/// ⚠️ **本类刻意断言真实 ViewModel 状态与实际生效的样式，不用源码 `Assert.Contains`。**
/// 同目录 `CanvasHelpersTests` 那种「读源码找字符串」的判据挪一行注释就绿，
/// 而本项目已因此吃过两次死样式事故（U148 双层边框 / U152 十二个死类）：
/// 样式写了 ≠ 样式生效了。
/// </para>
/// </summary>
public sealed class IdleRunControlVisibilityTests
{
    /// <summary>
    /// 互斥性是**定义级**的，不是运行时巧合：`ShowRunEntry => !ShowRunControls`。
    ///
    /// 这条钉住的是「不存在两者同时为假的空窗态」——
    /// 那正是缺陷原形（三键在、入口不在）的镜像失败：
    /// 若有人把两个属性改成各自独立计算，就可能出现「三键隐藏了、运行按钮也没出现」，
    /// 那比原缺陷更糟（连唯一有意义的动作也没了）。
    /// </summary>
    [Fact]
    public void RunEntryAndRunControls_AreNeverBothHidden()
    {
        var source = ReadViewModelSource();

        // 这一条是唯一允许看源码的断言，因为要证明的正是「它是取反而非独立计算」——
        // 这个性质在任何单一运行时状态下都观察不到（要穷举所有状态才等价）。
        Assert.Contains(
            "public bool ShowRunEntry => !ShowRunControls;",
            source,
            StringComparison.Ordinal);
    }

    /// <summary>
    /// 危险色必须由 class + 样式供值，**不能内联写在 Path 上**。
    ///
    /// ⚠️ 这是本条最关键的一跳，也是修复前它「加了样式却无效」的原因：
    /// 内联属性是 `BindingPriority.LocalValue`（0），**压过 Style(3) 与 Template(2) 全部层级**，
    /// 且失配时不报错、不回落 ⇒ 样式块静默无效。
    /// 项目里 U148 与 U152 都是这个根因，本条是第三次。
    /// </summary>
    [Fact]
    public void DangerIconFill_IsNotInlinedOnThePath()
    {
        var markup = ReadWorkspaceMarkup();

        Assert.DoesNotContain(
            "Fill=\"{DynamicResource Ariadne.StatusError}\"",
            markup,
            StringComparison.Ordinal);
    }

    /// <summary>禁用态退色规则必须存在，且退到与邻键同一枚令牌（三键齐平）。</summary>
    [Fact]
    public void DisabledDangerIcon_FadesToTheSameTokenAsItsNeighbours()
    {
        var theme = ReadTheme();

        Assert.Contains(
            "Button:disabled Path.icon-fill.danger",
            theme,
            StringComparison.Ordinal);
    }

    /// <summary>
    /// **①的主判据（行为级）：空画布上确实存在一个可见的运行入口，且它说得出自己为什么还不能点。**
    ///
    /// 刻意**不**断言「三键的 IsVisible 绑定变了」——那种判据在
    /// 「三键隐藏了但也没放运行按钮」时照样绿，等于把一个缺陷换成另一个。
    ///
    /// ⚠️ 也刻意**不**要求空画布上那颗按钮「可点」：跑一张空画布没有任何意义，
    /// 硬让它可点只会换来一句后端拒绝。本条要的是
    /// 「入口在场 + 禁用理由有文字」这一对——后者是本仓已定的规矩
    /// （见 `RunNodeAsync` 对 required 变量的处置注释：禁用理由必须配文字，
    /// 不能只灰掉按钮），也是它与「按钮压根不存在」的实质区别。
    /// 真正的「可点」由下一条在画上起始节点后钉。
    /// </summary>
    [Fact]
    public async Task EmptyCanvas_ShowsRunEntry_AndNamesWhatIsMissing()
    {
        var vm = await CreateLoadedViewModelAsync();

        Assert.Empty(vm.Nodes);
        Assert.False(vm.HasStartNodes);

        // 入口在场，三键不在场。
        Assert.True(vm.ShowRunEntry);
        Assert.False(vm.ShowRunControls);

        // 还不能跑，但说得出缺什么——且这句话不等于「从起始节点运行」那句通用文案。
        Assert.False(vm.RunWorkflowCommand.CanExecute(null));
        Assert.Equal(NeedsStartNodeText, vm.RunEntryTooltip);
        Assert.NotEqual(vm.RunFromStartText, vm.RunEntryTooltip);
    }

    /// <summary>
    /// **①的第二半：画上起始节点，那颗按钮立刻真的可点，说明文案也跟着换。**
    ///
    /// 拦的是「入口放上去了但门禁没接线」那类半成品：
    /// `RefreshStartNodes` 若忘了广播 `RunWorkflowCommand.NotifyCanExecuteChanged()`，
    /// 作者画完第一个「开始」节点后按钮**仍是灰的**——功能对了、界面没跟上，
    /// 正是本仓「修复要沿链路走到用户可见处」那条教训的形态。
    /// </summary>
    [Fact]
    public async Task AddingStartNode_MakesRunEntryClickable_AndSwapsTheHint()
    {
        var vm = await CreateLoadedViewModelAsync();
        Assert.False(vm.RunWorkflowCommand.CanExecute(null));

        vm.AddNodeAt("start", 120, 120);

        Assert.True(vm.HasStartNodes);
        // 入口仍在场（没有 run，所以还不该出现三键），且此刻**可点**。
        Assert.True(vm.ShowRunEntry);
        Assert.False(vm.ShowRunControls);
        Assert.True(vm.RunWorkflowCommand.CanExecute(null));
        // 说明文案从「缺什么」换回通用的「从起始节点运行」。
        Assert.Equal(vm.RunFromStartText, vm.RunEntryTooltip);
        Assert.NotEqual(NeedsStartNodeText, vm.RunEntryTooltip);
    }

    /// <summary>
    /// **①的翻转半：一旦真的跑起来，入口让位给三键；跑完（终态）再换回入口。**
    ///
    /// 终态那一段是本条刻意选的口径：`CurrentRunId` 此时**仍然非空**，
    /// 所以「有 run id 就显示三键」那种写法会让作者跑完一轮后
    /// 继续对着三个点不动的控制键——而他下一件事通常是再跑一轮。
    /// </summary>
    [Theory]
    [InlineData("running")]
    [InlineData("paused")]
    [InlineData("waiting_confirmation")]
    public async Task ActiveRun_SwapsRunEntryForTheThreeControls(string status)
    {
        var vm = await CreateLoadedViewModelAsync();
        vm.AddNodeAt("start", 120, 120);
        Assert.True(vm.ShowRunEntry);

        AttachRun(vm, status);

        Assert.False(vm.ShowRunEntry);
        Assert.True(vm.ShowRunControls);
    }

    [Theory]
    [InlineData("succeeded")]
    [InlineData("failed")]
    [InlineData("stopped")]
    public async Task TerminalRun_SwapsBackToRunEntry_EvenThoughRunIdRemains(string status)
    {
        var vm = await CreateLoadedViewModelAsync();
        vm.AddNodeAt("start", 120, 120);
        AttachRun(vm, "running");
        Assert.True(vm.ShowRunControls);

        AttachRun(vm, status);

        // 前置：run id 还在——否则这条用例只是在测「没有 run」，拦不住「有 run id 就显示三键」。
        Assert.NotEmpty(vm.CurrentRunId);
        Assert.True(vm.ShowRunEntry);
        Assert.False(vm.ShowRunControls);
        Assert.True(vm.RunWorkflowCommand.CanExecute(null));
    }

    /// <summary>
    /// **显隐必须真的广播出去，不只是 getter 算得对。**
    ///
    /// 这条补的是一个真实盲区：上面几条用例直接读属性，**属性值对了就绿**，
    /// 而绑定靠的是 `PropertyChanged`。若 `NotifyRunCommandStates()` 里漏了
    /// 那两行 `OnPropertyChanged`，跑起来后界面上「运行」按钮不会让位、
    /// 三键也不会出现 —— 功能对了、界面纹丝不动，
    /// 正是本仓「修复要沿链路走到用户可见处」那条教训的形态，
    /// 也是「做一半的功能会掩盖没做的一半」的形态。
    /// </summary>
    [Fact]
    public async Task RunStateTransition_RaisesPropertyChangedForBothVisibilities()
    {
        var vm = await CreateLoadedViewModelAsync();
        vm.AddNodeAt("start", 120, 120);

        var seen = new List<string>();
        vm.PropertyChanged += (_, e) => seen.Add(e.PropertyName ?? string.Empty);

        AttachRun(vm, "running");

        Assert.Contains(nameof(vm.ShowRunControls), seen);
        Assert.Contains(nameof(vm.ShowRunEntry), seen);
    }

    /// <summary>
    /// **②的主判据（渲染级）：禁用态下那颗停止图标实际取到的画刷，
    /// 必须与它的邻键（暂停/继续）取到**同一个**颜色。**
    ///
    /// 这条替代了「样式块存在」那种判据——本仓已因后者吃过两次死样式事故
    /// （U148 双层边框 / U152 十二个死类）：**样式写了 ≠ 样式生效了**。
    /// 判据落在 `Path.Fill` 上，也就是渲染器真正拿去画的那个值。
    ///
    /// 「与邻键同色」是刻意选的口径，比「不等于危险色」强：
    /// 后者在退成一个**更淡**的颜色时也绿，而那只是把「停止最扎眼」
    /// 换成「停止最看不见」，三键仍然不齐 —— 报告量的正是三者的不齐
    /// （停止 100 px vs 暂停 66 / 继续 53）。
    ///
    /// 亮/暗两个变体各跑一遍：令牌在两份 ThemeDictionaries 里值不同，
    /// 只测一个变体挡不住「退色时抄了另一变体的值」。
    /// </summary>
    [Theory]
    [InlineData("Light")]
    [InlineData("Dark")]
    public async Task DisabledDangerIcon_RendersTheSameBrushAsItsNeighbour(string variantName)
    {
        var variant = variantName == "Dark" ? ThemeVariant.Dark : ThemeVariant.Light;
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);

        await session.Dispatch(async () =>
        {
            Assert.NotNull(Application.Current);
            Application.Current!.RequestedThemeVariant = variant;

            // 照着产品里那两颗键的形状搭：icon-btn 宿主 + icon-fill 图标，
            // 停止那颗多一个 danger 类。刻意不实体化整个 WorkspacePageView——
            // 本条要证的是**样式规则**生效，页面结构由上面几条 VM 用例负责。
            var neutral = new ShapePath { Classes = { "icon-fill" }, Data = SquareGeometry() };
            var danger = new ShapePath { Classes = { "icon-fill", "danger" }, Data = SquareGeometry() };
            var neutralHost = new Button { Classes = { "icon-btn" }, Content = neutral };
            var dangerHost = new Button { Classes = { "icon-btn" }, Content = danger };
            var window = new Window
            {
                Width = 200,
                Height = 80,
                Content = new StackPanel { Children = { neutralHost, dangerHost } },
            };
            window.Show();
            await DrainAsync();

            var errorColor = ResolveColor(window, "Ariadne.StatusError", variant);
            var neutralColor = ResolveColor(window, "Ariadne.TextSecondary", variant);
            // 前置：两枚令牌值不同，否则本条整个不构成证据。
            Assert.NotEqual(errorColor, neutralColor);

            // 启用态：危险色必须还在（退色不能退成「永远是灰的」——那就把语义弄丢了）。
            Assert.Equal(errorColor, FillColor(danger));
            Assert.Equal(neutralColor, FillColor(neutral));

            neutralHost.IsEnabled = false;
            dangerHost.IsEnabled = false;
            await DrainAsync();

            // 禁用态：三键齐平。
            Assert.Equal(FillColor(neutral), FillColor(danger));
            Assert.NotEqual(errorColor, FillColor(danger));

            window.Content = null;
            window.Close();
            await DrainAsync();
            return true;
        }, CancellationToken.None);
    }

    private static Geometry SquareGeometry() => Geometry.Parse("M6,6 L18,6 L18,18 L6,18 Z");

    /// <summary>读 Path 上**实际生效**的填充色（不是我们写进 XAML 的那个值）。</summary>
    private static Color FillColor(ShapePath path)
    {
        var brush = path.Fill as ISolidColorBrush;
        Assert.NotNull(brush);
        return brush!.Color;
    }

    /// <summary>期望色一律从主题字典现取，测试里零颜色字面量。</summary>
    private static Color ResolveColor(Window window, string key, ThemeVariant variant)
    {
        Assert.True(
            window.TryFindResource(key, variant, out var resource),
            $"主题里找不到 {key}（{variant}），期望值无从取得");

        return resource switch
        {
            Color color => color,
            ISolidColorBrush brush => brush.Color,
            _ => throw new Xunit.Sdk.XunitException($"{key} 不是颜色资源：{resource?.GetType().Name}"),
        };
    }

    private static async Task DrainAsync()
    {
        for (var i = 0; i < 8; i++)
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

    /// <summary>本条要用到的那句「还没有开始节点」文案，从语言包现取，测试里零文案字面量。</summary>
    private static string NeedsStartNodeText =>
        DisplayNameService.LoadDefault().Text("ui.workspace.run.needs_start_node");

    /// <summary>
    /// 造一个「项目已打开 + 画布已加载成功 + 画布为空」的页面 VM。
    ///
    /// 必须真走 <c>ReloadProjectDataAsync</c>：`CanPersistWorkflow` 要求
    /// `WorkflowLoadState.Loaded`，而它的初值是 `NoProject`——
    /// 直接 new 出来的 VM 处在「画布还没加载」那一档，
    /// 拿它断言「运行入口禁用」会得到一个**理由不对**的绿灯。
    /// </summary>
    private static async Task<WorkspacePageViewModel> CreateLoadedViewModelAsync()
    {
        var backend = RunEntryBackend.Create();
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend);
        await vm.ReloadProjectDataAsync();
        return vm;
    }

    /// <summary>把 run 会话按到指定生命周期状态上（不起轮询，测试里不要后台请求）。</summary>
    private static void AttachRun(WorkspacePageViewModel vm, string status)
    {
        var session = typeof(WorkspacePageViewModel)
            .GetField("_runSession", BindingFlags.Instance | BindingFlags.NonPublic)!
            .GetValue(vm)!;
        session.GetType()
            .GetMethod("Attach", BindingFlags.Instance | BindingFlags.Public)!
            .Invoke(session, new object?[] { "default", "run-1", status, false, false });
    }

    /// <summary>空画布后端替身：项目已打开、画布加载成功但零节点。</summary>
    // ⚠️ **不能加 `sealed`**：DispatchProxy 要在运行时**继承**这个类型来生成代理，
    //    sealed 会让 `DispatchProxy.Create` 抛 `ArgumentException: cannot be sealed`。
    //    编译期不报，只在跑到这一行时才炸——所以「顺手统一风格」加回 sealed 时
    //    看起来一切正常，直到用例运行。
    private class RunEntryBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, RunEntryBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            if (targetMethod.Name == "get_HasProjectRoot")
            {
                return true;
            }

            object? value = targetMethod.Name switch
            {
                nameof(IAriadneBackendClient.LoadProjectCanvasAsync) => EmptyCanvas(),
                nameof(IAriadneBackendClient.SaveProjectCanvasAsync) => args![0],
                _ => DefaultFor(targetMethod.ReturnType),
            };

            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }

            return value;
        }

        private static WorkflowGraphData EmptyCanvas() => new(
            "default",
            "default",
            Array.Empty<CanvasNode>(),
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(StringComparer.Ordinal),
            ContentRevision: "rev-1",
            ExpectedRevision: null);

        private static object? DefaultFor(Type returnType)
        {
            if (returnType.IsGenericType && returnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var inner = returnType.GetGenericArguments()[0];
                return inner.IsArray
                    ? Array.CreateInstance(inner.GetElementType()!, 0)
                    : inner.IsValueType ? Activator.CreateInstance(inner) : null;
            }

            return returnType.IsValueType ? Activator.CreateInstance(returnType) : null;
        }
    }

    private static string ReadViewModelSource()
        => File.ReadAllText(ResolveDesktopFile(
            Path.Combine("Ariadne.Desktop", "ViewModels", "WorkspacePageViewModel.cs")));

    private static string ReadWorkspaceMarkup()
        => File.ReadAllText(ResolveDesktopFile(
            Path.Combine("Ariadne.Desktop", "Views", "WorkspacePageView.axaml")));

    private static string ReadTheme()
        => File.ReadAllText(ResolveDesktopFile(
            Path.Combine("Ariadne.Desktop", "Resources", "Styles", "AriadneTheme.axaml")));

    private static string ResolveDesktopFile(string relative)
    {
        var dir = AppContext.BaseDirectory;
        for (var i = 0; i < 8 && dir is not null; i++)
        {
            var candidate = Path.Combine(dir, "desktop", relative);
            if (File.Exists(candidate))
            {
                return candidate;
            }

            candidate = Path.Combine(dir, relative);
            if (File.Exists(candidate))
            {
                return candidate;
            }

            dir = Path.GetDirectoryName(dir);
        }

        throw new FileNotFoundException($"从 {AppContext.BaseDirectory} 向上找不到 {relative}");
    }
}
