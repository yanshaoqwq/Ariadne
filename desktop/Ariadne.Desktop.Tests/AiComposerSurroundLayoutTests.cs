using System.Reflection;
using Avalonia;
using Avalonia.Automation;
using Avalonia.Controls;
using Avalonia.Controls.Primitives;
using Avalonia.Headless;
using Avalonia.LogicalTree;
using Avalonia.Media;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Controls;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U213-C / U213-D：项目 AI 输入面**周围那一圈**的两条形态约束。
///
/// 两条合一个文件，因为它们钉的是同一块地方的两个层次：
/// - **D**：宿主那一层不许再叠一份带边线的内边距（用户原话「下面聊天框输出框
///   外面竟然又套了一层，难看死了」）；
/// - **C**：跨章知识查询不许再占对话流顶端，改为输入框**下方**的悬浮工具栏
///   （用户原话「不应该塞在顶端，应该在输入框下面悬浮着做工具栏」）。
///
/// ⚠️ **判据一律落在实体化后的控件上，不查 XAML 文本**。理由是本仓已记的两个坑：
/// 源码断言会被注释文本假命中（且变异标记复述被断言的字符串会让变异全绿），
/// 而「两页是否一致」这种性质在文本层只能证明「两处写了同样的字」，
/// 证不了「两处算出了同样的值」——`Classes` 走样式层，值来自主题字典。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class AiComposerSurroundLayoutTests
{
    /// <summary>
    /// U213-D：作品页输入面宿主**不许有边线**，且与工作区那一侧逐项一致。
    ///
    /// # 缺陷形态
    ///
    /// 作品页宿主原是 `Padding="12,10"` + `BorderThickness="0,1,0,0"`，
    /// 而 composer 自己就带完整 chrome（圆角 + 描边 + 12,10 内边距）⇒
    /// 从上往下是 发丝线 → 12/10 → composer 的框 → 又 12/10 → 文字，
    /// 视觉上「框里又一个框」。
    ///
    /// # 为什么判据必须包含「与工作区相等」这一半
    ///
    /// 报告的前提是「两页要同批改，否则会漂移」；**实测那个前提是反的**——
    /// 两页早已漂移，工作区那侧用 `dock-rail-header` 淡底色分层、零边框，是对的。
    /// 所以修复不是「两页一起换个新写法」，而是**把作品页对齐到工作区**。
    /// 只断言「作品页没有边线」会漏掉另一半：把两页各自改成两种「都没有边线」的
    /// 不同写法（比如一页透明底、一页淡底），观感照样不一致而用例全绿。
    ///
    /// # 三条断言各自不可省
    ///
    /// 1. 作品页宿主四边 `BorderThickness` 全 0 —— 那条发丝线是「双层」观感的根源；
    /// 2. 两页宿主的 `Background` 是**同一个颜色**且**不是透明**
    ///    —— 分层信号必须还在（去掉边线不等于去掉分层），且两页同款；
    /// 3. 两页宿主的 `Padding` 相等 —— 外缘留白同宽，composer 的框在两页里
    ///    与栏边缘的距离一致。
    /// </summary>
    [Fact]
    public async Task U213D_WorksComposerHost_HasNoHairline_AndMatchesWorkspace()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var names = DisplayNameService.LoadDefault();
            var window = new Window { Width = 1400, Height = 900 };
            window.Show();

            // 两页放进**同一个**窗口依次挂载：本机 3.8G，两个 headless 会话会被静默 OOM
            // （`AiPanelAndDiagnosticLayoutTests` 的注释记过）。
            var worksVm = new WorksPageViewModel(
                names, DispatchProxy.Create<IAriadneBackendClient, SoftBackend>());
            worksVm.IsRightPanelOpen = true;
            worksVm.IsNavTreeTab = false; // 项目 AI 页，输入面才在这一支里
            var worksView = new WorksPageView { DataContext = worksVm };
            window.Content = worksView;
            await DrainAsync();
            window.UpdateLayout();
            await DrainAsync();
            var worksHost = ResolveComposerHost(worksView);
            var worksThickness = worksHost.BorderThickness;
            var worksPadding = worksHost.Padding;
            var worksBackground = worksHost.Background as ISolidColorBrush;

            var workspaceVm = new WorkspacePageViewModel(
                names, DispatchProxy.Create<IAriadneBackendClient, SoftBackend>());
            workspaceVm.IsRightPanelOpen = true;
            Assert.True(
                workspaceVm.IsProjectAiTab,
                "工作区右栏默认不在项目 AI 页 ⇒ 本用例取到的不是同一块地方，判据无效。");
            var workspaceView = new WorkspacePageView { DataContext = workspaceVm };
            window.Content = workspaceView;
            await DrainAsync();
            window.UpdateLayout();
            await DrainAsync();
            var workspaceHost = ResolveComposerHost(workspaceView);

            // 自检：两页真的各取到了一个宿主，且不是同一个对象（否则下面比的是自己和自己）。
            Assert.NotSame(worksHost, workspaceHost);

            // 判据 1：作品页那条发丝线必须消失。
            Assert.Equal(new Thickness(0), worksThickness);

            // 判据 2：分层底色仍在，且两页同色。
            var workspaceBackground = workspaceHost.Background as ISolidColorBrush;
            Assert.NotNull(worksBackground);
            Assert.NotNull(workspaceBackground);
            Assert.True(
                worksBackground!.Color.A > 0,
                "作品页宿主底色全透明 ⇒ 边线去掉了，分层信号也一起没了（输入区与对话流糊成一片）。");
            Assert.Equal(workspaceBackground!.Color, worksBackground.Color);

            // 判据 3：外缘留白同宽。
            Assert.Equal(workspaceHost.Padding, worksPadding);

            window.Content = null;
            window.Close();
            await DrainAsync();
            return true;
        }, CancellationToken.None);
    }

    /// <summary>
    /// U213-C：搜索入口在输入框**下方**，且是「不占地的小图标」。
    ///
    /// # 用户否掉的是位置本身
    ///
    /// 原话：「搜索应该是小图标放在合适的位置，**不应该塞在顶端，应该在输入框
    /// 下面悬浮着做工具栏**」。所以判据的核心是**相对位置**与**占地**两件事，
    /// 光断言「入口还在」是不够的——它在顶端时也一直都在。
    ///
    /// # 四条断言
    ///
    /// 1. 折叠态入口存在（`icon-btn` 那颗搜索键），且它的顶边**低于** composer 的底边
    ///    —— 「在输入框下面」这句话的字面意思，也是原形态（顶端）必然违反的一条；
    /// 2. 它只占一行图标高（≤ 一个图标按钮的高度 + 余量）
    ///    —— 原形态六个纵向元素占了上百像素；
    /// 3. 静息态下浮层**没有实体化的内容**（`Popup.Child` 未挂进可视树）
    ///    —— 「悬浮」的含义是不参与布局，用 `Panel + IsVisible` 那条路会占高度；
    /// 4. 折叠态整条工具栏**没有底色**（它浮在输入框下方，不是又一张卡片）。
    ///
    /// ⚠️ 判据取 `TranslatePoint` 到同一宿主的坐标而不是 `Bounds.Y`：
    /// 两者分属不同父容器，`Bounds` 是相对各自父级的，直接比会比出错的结论。
    /// </summary>
    [Fact]
    public async Task U213C_KnowledgeLookupEntry_SitsBelowTheComposer_AsACollapsedIcon()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var names = DisplayNameService.LoadDefault();
            var panel = new ProjectAiPanel
            {
                DataContext = new WorkspacePageViewModel(
                    names, DispatchProxy.Create<IAriadneBackendClient, SoftBackend>()),
            };
            // 宽度取 318：与 `AiPanelAndDiagnosticLayoutTests` 同一量级（右栏实测宽度）。
            var window = new Window { Width = 318, Height = 320, Content = panel };
            window.Show();
            await DrainAsync();
            window.UpdateLayout();
            await DrainAsync();

            try
            {
                var composer = panel.GetVisualDescendants().OfType<ProjectAiComposer>().FirstOrDefault();
                Assert.NotNull(composer);
                var toggle = panel.GetVisualDescendants().OfType<Button>()
                    .FirstOrDefault(button => button.Name == "KnowledgeLookupToggle");
                Assert.NotNull(toggle);

                // 自检：两者都真的布局过了，否则下面比的是 0 和 0。
                Assert.True(
                    composer!.Bounds.Height > 20 && toggle!.Bounds.Height > 4,
                    $"控件没量出高度（composer={composer.Bounds.Height} toggle={toggle!.Bounds.Height}）"
                    + " —— 布局没跑，位置判据无效。");

                var composerBottom = composer.TranslatePoint(
                    new Point(0, composer.Bounds.Height), panel)!.Value.Y;
                var toggleTop = toggle.TranslatePoint(default, panel)!.Value.Y;

                // 判据 1：在输入框**下方**。
                Assert.True(
                    toggleTop >= composerBottom,
                    $"搜索入口顶边 y={toggleTop} 高于输入框底边 y={composerBottom} ⇒ "
                    + "它又跑到输入框上方去了（用户否掉的正是「塞在顶端」这个位置，U213-C）。");

                // 判据 2：只占一行图标高。
                Assert.True(
                    toggle.Bounds.Height <= 40,
                    $"折叠态入口高 {toggle.Bounds.Height}px ⇒ 不再是「一个小图标」，"
                    + "常驻占地又回来了（原形态是六个纵向元素）。");

                // 判据 3：静息态浮层不参与布局。
                //
                // ⚠️ 顺序要紧：**先**断言「浮层内容没有实体化进可视树」，
                // 再断言「那个 Popup 确实在」。反过来写的话，把 Popup 换成
                // `Panel + IsVisible`（真正要防的那种退化）会先撞上
                // `Assert.Single(Popup)` 的「集合为空」——用例照样红，
                // 但红的原因变成「找不到 Popup」而不是「浮层占了布局」，
                // 判据于是证明不了自己声称的那条性质（变异测试当场发现）。
                Assert.Empty(panel.GetVisualDescendants().OfType<AutoCompleteBox>());
                var popup = Assert.Single(panel.GetLogicalDescendants().OfType<Popup>());
                Assert.False(popup.IsOpen);

                // 判据 4：工具栏那一条不许有底色（不是又一张卡片，U213-E 同一条）。
                var rail = toggle.GetVisualAncestors().OfType<StackPanel>().First();
                Assert.True(
                    rail.Background is null
                    || (rail.Background is ISolidColorBrush fill && fill.Color.A == 0),
                    "工具栏刷上了底色 ⇒ 它正在变成一张卡片，而用户要的是「悬浮」。");
            }
            finally
            {
                window.Content = null;
                window.Close();
                await DrainAsync();
            }

            return true;
        }, CancellationToken.None);
    }

    /// <summary>
    /// U213-C 展开态：浮层真的开得出来，且**横向落在输入框那一栏之内**。
    ///
    /// # 为什么折叠态那条用例不够
    ///
    /// 它断言的是「静息态浮层没实体化」。那条在「浮层压根打不开」时**照样全绿**
    /// —— 永远不实体化当然满足「静息态不实体化」。两条合起来才是完整性质：
    /// 关着的时候不占地，点开之后有内容。
    ///
    /// # 🔴 判据换过一次：**展开方向断言不出来，别再试**
    ///
    /// 我先写的是「浮层底边不低于输入框顶边 ⇒ 它向上弹」。变异测试证否了它：
    /// 把 `Placement` 从 `TopEdgeAlignedRight` 改成 `BottomEdgeAlignedRight`
    /// （字面意思是向下弹），用例**照样全绿**。
    /// 原因是 Avalonia 的 popup 定位自带翻转：本控件停在整栏最下沿，
    /// 下方放不下 340px 的浮层，于是它**无论声明哪个方向都会被翻到上方**。
    /// ⇒ 「向上弹」在生产几何下不是我的代码决定的，是框架保证的；
    /// 为它写断言只会得到一条永远绿的装饰（AGENTS.md 记的第三种情形：
    /// 那段代码本来就没有行为效果）。`TopEdgeAligned*` 保留为**意图声明**，
    /// 但它的价值不在这条用例里。
    ///
    /// # 真正由我的代码决定、且实机两次踩到的是**横向**
    ///
    /// 框架不会替你修横向溢出。实测两版都错：
    /// 锚在图标 + 左对齐 → 浮层从最左的图标向右展开，**溢出窗口右缘**；
    /// 锚在图标 + 右对齐 → 相对图标右缘对齐，整块往左跑、**压在正文编辑器上**。
    /// 只有「锚在满栏宽的 composer + 右对齐」才让它落在本栏内。
    /// ⇒ 判据取「浮层的横向区间包含在 composer 的横向区间里（含容差）」。
    ///
    /// # 判据
    ///
    /// 1. 拨 `IsPanelOpen` 后输入框与关闭键**真的实体化**（浮层内容挂上了树）；
    /// 2. 浮层横向不越出 composer 那一栏（本条是本用例的本体）；
    /// 3. 关闭命令一执行，内容退出可视树（不是「藏起来但还占着」）。
    /// </summary>
    [Fact]
    public async Task U213C_KnowledgeLookupPopup_OpensWithinTheComposerColumn()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var names = DisplayNameService.LoadDefault();
            var viewModel = new WorkspacePageViewModel(
                names, DispatchProxy.Create<IAriadneBackendClient, SoftBackend>());
            var panel = new ProjectAiPanel { DataContext = viewModel };
            // ⚠️ **必须复现「右栏贴在宽窗口右缘」这个宿主几何**，不能把 panel
            // 直接当 Content。两条理由都是变异测试逼出来的：
            //
            // (1) 竖向：Avalonia 在「上方放不下」时会把浮层自动翻下去。panel
            //     贴窗口顶时它上面只有 ~71px ⇒ 用例会报「在向下展开」，
            //     而那是**测试宿主自己的形状**造成的，不是产品缺陷
            //     （本仓已记这一类：取证宿主会量到自己的形状）。
            // (2) 横向：窗口只有 318px 宽时，浮层无论锚在哪都会被框架**贴边夹住**
            //     ⇒ 「没越栏」恒成立，判据变成装饰（把锚点改回小图标，全绿）。
            //     真实缺陷只在「右栏是宽窗口里的一条窄栏」时才显形——
            //     那时浮层有充足的左侧空间可以跑出去。
            //
            // ⇒ 照抄生产结构：宽窗口 + 右侧窄栏 + 栏内 `RowDefinitions="*,Auto"`。
            var rail = new Grid { RowDefinitions = new RowDefinitions("*,Auto") };
            rail.Children.Add(panel);
            Grid.SetRow(panel, 1);
            var page = new Grid
            {
                ColumnDefinitions = new ColumnDefinitions("*,318"),
            };
            page.Children.Add(rail);
            Grid.SetColumn(rail, 1);
            var window = new Window { Width = 1400, Height = 900, Content = page };
            window.Show();
            await DrainAsync();
            window.UpdateLayout();
            await DrainAsync();

            try
            {
                var composer = panel.GetVisualDescendants().OfType<ProjectAiComposer>().First();
                // ⚠️ 坐标一律取**屏幕坐标**：浮层在自己的顶层（`PopupRoot`）里，
                // 与宿主不共享坐标系，`TranslatePoint(..., panel)` 跨不过去（返回 null）。
                var composerLeft = composer.PointToScreen(default).X;
                var composerRight = composer
                    .PointToScreen(new Point(composer.Bounds.Width, 0)).X;
                // 自检：composer 真的量出了宽度，否则下面的区间是个点。
                Assert.True(
                    composerRight - composerLeft > 100,
                    $"composer 没量出宽度（{composerLeft}..{composerRight}）—— 区间判据无效。");

                viewModel.KnowledgeLookup.IsPanelOpen = true;
                await DrainAsync();
                window.UpdateLayout();
                await DrainAsync();

                var popup = panel.GetLogicalDescendants().OfType<Popup>().First();
                Assert.True(popup.IsOpen, "拨了 IsPanelOpen 而浮层没开 ⇒ IsOpen 没绑上。");

                // 判据 1：表单真的在浮层里实体化了。
                var input = popup.GetLogicalDescendants().OfType<AutoCompleteBox>().FirstOrDefault();
                Assert.NotNull(input);
                Assert.Contains(
                    popup.GetLogicalDescendants().OfType<Button>(),
                    button => Equals(
                        button.GetValue(AutomationProperties.NameProperty),
                        viewModel.KnowledgeLookup.CloseText));

                // 判据 2（本体）：浮层横向落在 composer 那一栏之内。
                // ⚠️ 直接取 `Popup.Child`，**不要**对 popup 调
                // `GetVisualDescendants()`：Popup 自己不在宿主的可视树里
                // （它有独立顶层 `PopupRoot`），那样什么都取不到
                // （实测「Sequence contains no matching element」）。
                // 也不要走 `Popup.Host`——那在 12.0.5 里是显式接口实现
                // （`Avalonia.Controls.Diagnostics.IPopupHostProvider`），
                // `Popup` 类型上压根没有这个公开成员，写了编译不过。
                var shell = Assert.IsType<Border>(popup.Child);
                Assert.Contains("glass-dialog", shell.Classes);
                var shellLeft = shell.PointToScreen(default).X;
                var shellRight = shell.PointToScreen(new Point(shell.Bounds.Width, 0)).X;
                // 自检：浮层真的量出了宽度，否则「没越栏」是因为它没渲染。
                Assert.True(
                    shell.Bounds.Width > 100 && shell.Bounds.Height > 40,
                    $"浮层没量出尺寸（{shell.Bounds.Width}×{shell.Bounds.Height}）—— 判据无效。");

                // 容差 2px：屏幕坐标经过一次 DPI 取整，逐像素相等不是可靠判据。
                Assert.True(
                    shellRight <= composerRight + 2,
                    $"浮层右缘 x={shellRight} 越出输入框右缘 x={composerRight} ⇒ "
                    + "它正在溢出右栏（第一版就是这样被切掉一条，U213-C）。");
                Assert.True(
                    shellLeft >= composerLeft - 2,
                    $"浮层左缘 x={shellLeft} 越出输入框左缘 x={composerLeft} ⇒ "
                    + "它压到了正文编辑器上，读起来像「从别处飞来的一块板」（U213-C）。");

                // 判据 3：关掉后内容退出可视树，不是藏着还占位。
                viewModel.KnowledgeLookup.ClosePanelCommand.Execute(null);
                await DrainAsync();
                window.UpdateLayout();
                await DrainAsync();
                Assert.False(popup.IsOpen);
                Assert.Empty(panel.GetVisualDescendants().OfType<AutoCompleteBox>());
            }
            finally
            {
                viewModel.KnowledgeLookup.IsPanelOpen = false;
                await DrainAsync();
                window.Content = null;
                window.Close();
                await DrainAsync();
            }

            return true;
        }, CancellationToken.None);
    }

    // ---------- 定位：宿主 = 直接包着 ProjectAiPanel 的那个 Border ----------

    /// <summary>
    /// 取「Child 就是 <see cref="ProjectAiPanel"/>」的那个 Border。
    ///
    /// 用父子关系而不是 <c>x:Name</c> 定位：两页各写一份宿主，给它们起名字等于
    /// 要求两处名字也保持一致，而「宿主漂移」正是本文件要防的东西——
    /// 判据自己就不该依赖一个会跟着漂移的锚点。
    /// </summary>
    private static Border ResolveComposerHost(Control root)
    {
        var panel = root.GetVisualDescendants().OfType<ProjectAiPanel>().FirstOrDefault();
        Assert.NotNull(panel);
        var host = panel!.GetVisualAncestors().OfType<Border>().FirstOrDefault();
        Assert.NotNull(host);
        return host!;
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    /// <summary>
    /// 后端一律抛。本文件只测视觉层次，页面处于错误态照样把控件实体化；
    /// 返回「成功的默认值」会让 VM 走进真实加载流程并等更多调用而卡死
    /// （<see cref="InputSurfaceStyleTests"/> 的注释记过这个坑）。
    /// ⚠️ <c>DispatchProxy</c> 宿主类不能 sealed。
    /// </summary>
    private class SoftBackend : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == $"get_{nameof(IAriadneBackendClient.HasProjectRoot)}")
            {
                return false;
            }

            throw new NotSupportedException(targetMethod?.Name);
        }
    }

    /// <summary>把布局与样式两级都放空，确保取到的是应用过主题后的值。</summary>
    private static async Task DrainAsync()
    {
        for (var i = 0; i < 12; i++)
        {
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Background);
        }

        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
    }
}
