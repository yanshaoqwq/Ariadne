using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Input;
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
/// U181：模态弹窗挡不住顶栏 + 焦点管理六缺口。
///
/// # 六条前提的核实结论（2026-08-18 逐条复核，报告写于 2026-08-19 之前的形态已漂）
///
/// **A（P1）模态遮罩不覆盖顶栏 —— 成立，但形态已变**。
/// 报告说遮罩是 `Grid.Row="1"` + `RowSpan="3"`；实测现在是 `RowSpan="2"`
/// （`MainWindow.axaml:448-450`）—— 因为 U208-G 把诊断横幅移进内容区当浮层，
/// 外层网格从 4 行缩成 3 行（`:42` `RowDefinitions="54,Auto,*"`）。
/// **结论不变**：顶栏在 `Grid.Row="0"`，遮罩覆盖 1..2 ⇒ 第 0 行既没被压暗也没被拦命中。
/// 顶栏里有项目切换 Button + MenuFlyout（`:93-136`，含新建/打开/切项目/离开项目）
/// 与三个窗口控制键（`:190-212`）。
/// 另两条根因同样成立：全仓 `IsEnabled="False"` 计数 = **0**；
/// `OnDialogScrimPressed`（`MainWindow.axaml.cs:285`）只拦指针且只拦落在自己身上的，
/// 全仓 **零处** Avalonia `KeyboardNavigation` 附加属性
/// （grep 命中的 `CanvasKeyboardNavigationHelpers` 是同名标识符，不是那个附加属性），
/// 唯一 `IsTabStop` 在 `SettingsPageView.axaml:590`（报告记的 534 已漂）。
///
/// **B（P2）关闭弹窗不还原焦点 —— 成立**。
/// `DialogService.cs` 的 `ShowAsync` 只有 `finally { ActiveDialog = null; }`，
/// 零焦点保存点；全仓 `FocusManager` 调用只在 `WorkspacePageView.axaml.cs` 两处只读判定。
///
/// **C（P2）画布节点焦点环是死样式 —— 成立**。
/// `WorkspacePageView.axaml:17-19` 的 `Border.keyboard-target:focus` 设
/// `BoxShadow 0 0 0 3 #B32E726B`，而 `:21-31` 的 `Grid.canvas-node Border.node-card`
/// 在同一 `UserControl.Styles` **更靠后**又设 `BoxShadow`，`:32-35`（pointerover）、
/// `:69-72`（selected）各再设一次。节点卡（`:598` `x:Name="NodeKeyboardFocusHost"`）
/// 挂 `Classes="keyboard-target node-keyboard-target node-card"` ⇒ 三条都命中它。
/// 按本仓已钉死的规则（同优先级按文档顺序、后者胜）焦点环永不生效。
/// ⚠️ 这条**不是**「缺资源键静默失效」那一型：`#B32E726B` 是字面量，键根本不参与；
/// 也不是「填充/描边语义错配」（`BoxShadow` 无此二义）。是纯粹的声明顺序覆盖，
/// 所以判据必须量**渲染后的 `BoxShadow` 实际值**才能区分。
///
/// **D（P3）`Ariadne.FocusRing` 零消费 —— 成立**。
/// `grep -rw` 排除 `bin/`+`obj/` 后全仓仅 4 处，全在定义与写入侧：
/// `AriadneTheme.axaml:121`（亮）/`:370`（暗）定义 Brush，
/// `ThemeApplication.cs:56`（`OverlayBrushKeys` 名单）/`:299`（`SetBrush` 写入）。
/// **零处 `{DynamicResource Ariadne.FocusRing}` 引用**，C# 侧也无字符串键读取
/// ⇒ 个性化换强调色时焦点指示纹丝不动。色值 `Ariadne.Color.FocusRing`
/// 在 `:53`（#2E726B）/`:306`（#6FB9AD）。
///
/// **E（P2）键盘连线起点自己被淡出、且无橡皮筋 —— 成立**。
/// `WorkspacePageViewModel.cs` 的 `BeginPortDragHighlight` 遍历**全部** `Nodes`
/// 含起点节点自己；`SetPortDragHighlight` 的 false 分支把五个端口 Opacity 打到 `0.22`，
/// 而 `TryEvaluateConnection` 对同节点必然失败 ⇒ 起点节点**所有**端口一起变淡。
/// 且 `OnPortKeyDown`（`WorkspacePageView.axaml.cs:2158`）**不调** `UpdateRubberBand`，
/// 而鼠标路径 `OnPortPointerPressed`（`:2123`）调了 ⇒ 键盘路径零起点指示。
///
/// **F（P3）`RightPanelTogglePill` 可聚焦但零焦点可见性 —— 成立**。
/// `Controls/RightPanelTogglePill.axaml:10` 有 `Focusable="True"`，
/// 但整个文件**无 `UserControl.Styles`、无任何 `:focus` 规则**；
/// 主题里搜 `TogglePill` 只命中 `AriadneTheme.axaml:2609` 一句注释，
/// 主题全部 14 处 `:focus` 选择器无一针对它。
///
/// # 判据取向
///
/// 焦点与模态类判据**一律落在运行态**：headless 起真窗口、真 `Focus()`、
/// 真 `RaiseEvent` 发 `KeyEventArgs`、断言 `FocusManager.GetFocusedElement()` 的实际归属，
/// 或断言用户可见结果（`CurrentPage` / `ProjectRoot` / 渲染后的 `BoxShadow`）。
/// **不断言「XAML 里设了某个属性」** —— 那是读代码不是读结果。
/// </summary>
[Collection("GlobalDialogService")]
public sealed class ModalFocusContainmentTests
{
    // ── A：模态隔离 ──────────────────────────────────────────

    /// <summary>
    /// A-1（布局判据，两态之差）：**弹窗开着时，顶栏的项目切换键必须被遮罩盖住。**
    ///
    /// 判据落在实测坐标 + 真实命中：取项目切换键在窗口坐标系里的中心点，
    /// 断言该点落在顶栏遮罩的矩形内、且遮罩此刻真的参与命中测试。
    /// **不断言「XAML 里 RowSpan 是几」** —— 原缺陷恰恰是「遮罩 IsVisible 为 true
    /// 但不管用」，读属性读不出这件事。
    ///
    /// 反向态（弹窗关着）一并钉住：遮罩不得拦命中，否则作者平时就点不到项目切换键了。
    /// </summary>
    [Fact]
    public async Task A1_DialogOpen_ScrimCoversTheTitleBarRow()
    {
        await RunWindowAsync(async harness =>
        {
            var scrim = harness.RequireControl<Border>("TitleBarDialogScrim");
            var projectButton = harness.ProjectMenuButton;

            // 反向态先量：关着的时候遮罩不许拦命中。
            Assert.False(
                scrim.IsHitTestVisible,
                "弹窗没开时顶栏遮罩就在拦命中 ⇒ 作者平时点不到项目切换键，"
                + "把一个缺陷换成了更糟的一个。");

            var pending = harness.OpenDialogAsync();
            await Drain();

            Assert.True(
                scrim.IsHitTestVisible,
                "弹窗开着而顶栏遮罩不拦命中 ⇒ 遮罩只是一层像素，正是原缺陷。");

            // 实测坐标：项目切换键中心必须落在遮罩矩形内。
            var buttonCenter = projectButton.TranslatePoint(
                new Point(projectButton.Bounds.Width / 2, projectButton.Bounds.Height / 2),
                harness.Window);
            Assert.True(buttonCenter.HasValue, "项目切换键没测量 ⇒ 坐标判据无效（前提自检）");

            var scrimTopLeft = scrim.TranslatePoint(default, harness.Window);
            Assert.True(scrimTopLeft.HasValue, "顶栏遮罩没测量 ⇒ 坐标判据无效（前提自检）");
            var scrimRect = new Rect(scrimTopLeft!.Value, scrim.Bounds.Size);

            Assert.True(
                scrimRect.Contains(buttonCenter!.Value),
                $"项目切换键中心 {buttonCenter} 不在顶栏遮罩 {scrimRect} 内 ⇒ 顶栏没被盖住。");

            harness.CancelDialog();
            await pending;
        });
    }

    /// <summary>
    /// A-2（用户可见结果判据）：**弹窗开着时，对顶栏项目切换键发一次真实鼠标按下，
    /// 不许打开项目菜单。**
    ///
    /// 走 `IHeadlessWindow.MouseDown` 而不是 `button.Command.Execute()` 或
    /// `RaiseEvent` —— 缺陷正出在**命中测试**这一层（遮罩盖不到第 0 行），
    /// 绕过它就等于不测。判据取 `Button.Flyout.IsOpen`：那正是报告里
    /// 「菜单会弹在弹窗之上，作者可以直接新建/打开另一个项目」的第一步。
    ///
    /// 反向态一并钉住：弹窗**关着**时同一次点击必须真的把菜单打开。
    /// 缺了它，「把项目切换键 IsEnabled 常关」也能让正向全绿。
    /// </summary>
    [Fact]
    public async Task A2_DialogOpen_TopBarProjectMenuCannotChangeProject()
    {
        await RunWindowAsync(async harness =>
        {
            var projectButton = harness.ProjectMenuButton;
            var center = projectButton.TranslatePoint(
                new Point(projectButton.Bounds.Width / 2, projectButton.Bounds.Height / 2),
                harness.Window);
            Assert.True(center.HasValue, "项目切换键没测量 ⇒ 点击判据无效（前提自检）");

            // 反向态：弹窗关着时这一点击**必须**能开菜单，否则正向判据没有意义。
            harness.ClickAt(center!.Value);
            await Drain();
            Assert.True(
                projectButton.Flyout?.IsOpen == true,
                "弹窗没开时点项目切换键都开不出菜单 ⇒ 本用例的点击路径没走到实处（前提自检）");
            projectButton.Flyout!.Hide();
            await Drain();

            var pending = harness.OpenDialogAsync();
            await Drain();

            harness.ClickAt(center!.Value);
            await Drain();

            Assert.False(
                projectButton.Flyout?.IsOpen == true,
                "「离开前要不要保存」开着时还能点开项目菜单 ⇒ 作者能在没回答前直接换项目，"
                + "未保存的正文会丢。这是 U181-A 的原始症状。");

            harness.CancelDialog();
            await pending;
        });
    }

    /// <summary>
    /// A-3（焦点归属判据）：**弹窗开着时反复按 Tab，焦点不许跑到弹窗外面。**
    ///
    /// 判据取 `FocusManager.GetFocusedElement()` 的**实际归属** ——
    /// 断言它始终落在弹窗宿主 `ContentControl` 的子树内。
    /// **不断言「XAML 里设了 TabNavigation」**，那是读代码不是读结果。
    ///
    /// 按 20 次而不是 1 次：弹窗内部本来就有好几个可聚焦控件，按一次
    /// 当然还在里面。要压到「循环回头」之后才知道它是不是真的被关住了。
    /// </summary>
    [Fact]
    public async Task A3_DialogOpen_TabCannotEscapeToSidebarNavigation()
    {
        await RunWindowAsync(async harness =>
        {
            var pending = harness.OpenDialogAsync();
            await Drain();

            var host = harness.DialogHost;
            Assert.True(
                harness.FocusedElement is Control initial
                    && IsWithin(initial, host),
                "弹窗打开后焦点没落在弹窗里 ⇒ U64 的默认聚焦断了，本用例的起点不成立（前提自检）");

            for (var step = 0; step < 20; step++)
            {
                harness.PressKey(Key.Tab);
                await Drain();

                var focused = harness.FocusedElement as Control;
                Assert.True(
                    focused is not null && IsWithin(focused, host),
                    $"第 {step + 1} 次 Tab 之后焦点跑到了弹窗外面"
                    + $"（{focused?.GetType().Name ?? "null"}）⇒ 没有焦点陷阱，"
                    + "作者能 Tab 到侧栏导航项并按 Enter 换页。");
            }

            harness.CancelDialog();
            await pending;
        });
    }

    /// <summary>
    /// A-4（**反向钉死，不可省**）：**弹窗内部 Tab 仍然要能循环到不同控件上。**
    ///
    /// 缺了这条，「把弹窗里所有控件的 `IsTabStop` 关掉」也能让 A-3 全绿 ——
    /// 那是把一个缺陷换成一个更糟的：键盘用户完全操作不了弹窗。
    ///
    /// 判据取「20 次 Tab 里，落在**弹窗子树内**的聚焦对象至少有 2 个不同的」。
    /// 不取「每次都变」：弹窗内可聚焦控件数量有限，循环时必然重复。
    ///
    /// ⚠️ **"弹窗子树内"这个限定是变异测试逼出来的，不是我一开始就写对的。**
    /// 首版只数"不同的聚焦对象"，于是变异（给弹窗按钮加 `IsTabStop="False"`，
    /// 模拟那个假修法）之后它**仍然绿** ——
    /// 因为焦点跑到弹窗**外面**去了，外面的控件当然也是"不同的对象"。
    /// 那一版等于把 A-3 的失效顺手当成了 A-4 的通过条件，两条一起废掉。
    /// 加上子树限定后同一变异立刻转红。
    /// </summary>
    [Fact]
    public async Task A4_DialogOpen_TabStillCyclesInsideTheDialog()
    {
        await RunWindowAsync(async harness =>
        {
            var pending = harness.OpenDialogAsync();
            await Drain();

            var host = harness.DialogHost;
            var seen = new HashSet<object>(ReferenceEqualityComparer.Instance);
            if (harness.FocusedElement is Control start && IsWithin(start, host))
            {
                seen.Add(start);
            }

            for (var step = 0; step < 20; step++)
            {
                harness.PressKey(Key.Tab);
                await Drain();
                if (harness.FocusedElement is Control focused && IsWithin(focused, host))
                {
                    seen.Add(focused);
                }
            }

            Assert.True(
                seen.Count >= 2,
                $"弹窗内 20 次 Tab 只碰到 {seen.Count} 个**弹窗内**可聚焦对象 ⇒ 焦点陷阱做成了"
                + "「谁也别动」（或者焦点整个跑出去了），键盘用户操作不了弹窗。"
                + "这比 Tab 跑出去更糟。");

            harness.CancelDialog();
            await pending;
        });
    }

    /// <summary>
    /// A-5（产品定夺判据）：**窗口控制键（最小化/最大化/关闭）在弹窗开着时仍可点。**
    ///
    /// 这是报告里明确要求产品定夺的那一处，结论取「保留窗口控制」：
    /// 把关闭键也禁掉会让弹窗成为退出应用的唯一出路，用户没法用系统手势退出。
    /// 所以顶栏遮罩只覆盖前 3 列（品牌轨 + 项目切换 + 预算），不覆盖第 3 列。
    ///
    /// 判据取「三个键的中心点都落在顶栏遮罩矩形**之外**」——
    /// 若有人图省事把遮罩改成整行覆盖，这条会立刻红。
    /// </summary>
    [Fact]
    public async Task A5_DialogOpen_WindowControlsStayReachable()
    {
        await RunWindowAsync(async harness =>
        {
            var pending = harness.OpenDialogAsync();
            await Drain();

            var scrim = harness.RequireControl<Border>("TitleBarDialogScrim");
            var scrimTopLeft = scrim.TranslatePoint(default, harness.Window);
            Assert.True(scrimTopLeft.HasValue, "顶栏遮罩没测量 ⇒ 坐标判据无效（前提自检）");
            var scrimRect = new Rect(scrimTopLeft!.Value, scrim.Bounds.Size);

            var controls = harness.WindowControls;
            Assert.Equal(3, controls.Count);

            foreach (var control in controls)
            {
                var center = control.TranslatePoint(
                    new Point(control.Bounds.Width / 2, control.Bounds.Height / 2),
                    harness.Window);
                Assert.True(center.HasValue, "窗口控制键没测量 ⇒ 坐标判据无效（前提自检）");
                Assert.False(
                    scrimRect.Contains(center!.Value),
                    $"窗口控制键中心 {center} 被顶栏遮罩 {scrimRect} 盖住了 ⇒ 弹窗成了退出应用的"
                    + "唯一出路，用户没法用系统手势关窗。产品定夺是「只禁业务入口，保留窗口控制」。");
            }

            harness.CancelDialog();
            await pending;
        });
    }

    // ── B：关闭侧焦点还原 ────────────────────────────────────

    /// <summary>
    /// B-1（焦点归属判据）：**关掉弹窗后焦点回到打开它的那个控件。**
    ///
    /// 判据刻意取「回到**那一个**控件」而不是「焦点非 null」：
    /// 后者在焦点被随便扔到窗口第一个可聚焦控件时同样成立，而那正是原缺陷的体感
    /// （作者的手位没了，得从头 Tab 一遍）。
    ///
    /// 触发控件取顶栏的项目切换键 —— 它是真实的弹窗触发者之一，
    /// 且在弹窗关闭后仍然在树上（"仍在树上"这个前提成立，B-2 管另一半）。
    /// </summary>
    [Fact]
    public async Task B1_DialogClosed_FocusReturnsToTheControlThatOpenedIt()
    {
        await RunWindowAsync(async harness =>
        {
            var opener = harness.ProjectMenuButton;
            Assert.True(opener.Focus(), "触发控件拿不到焦点 ⇒ 本用例的起点不成立（前提自检）");
            await Drain();
            Assert.Same(opener, harness.FocusedElement);

            var pending = harness.OpenDialogAsync();
            await Drain();

            // 弹窗打开后焦点必然已经离开触发控件（U64 的默认聚焦），否则本用例没测到东西。
            Assert.NotSame(opener, harness.FocusedElement);

            harness.CancelDialog();
            await pending;
            await Drain();

            Assert.Same(opener, harness.FocusedElement);
        });
    }

    /// <summary>
    /// B-2（**边界条件，不可省**）：**触发控件已从树上摘除时，还原动作不许抛异常，
    /// 也不许把焦点扔到别处。**
    ///
    /// 这是报告明确点出的坑：弹窗的结果常常就是把那个控件所在的页面换掉
    /// （「离开前要不要保存」答完就换页了），对已摘除的元素调 `Focus()`
    /// 会静默失败或把焦点扔到奇怪的地方。
    ///
    /// 构造方式：拿一个**从未挂进窗口**的按钮当"触发者"聚焦不了，所以改为
    /// 先把一个按钮挂进树、聚焦、开弹窗，再在弹窗关闭前把它摘下来。
    /// 判据取「不抛 + 焦点没落到那个已摘除的控件上」。
    /// </summary>
    [Fact]
    public async Task B2_OpenerDetached_RestoreDoesNotThrowOrStealFocus()
    {
        await RunWindowAsync(async harness =>
        {
            // 借诊断横幅那块常驻 Panel 挂一个临时按钮：它在内容列里，摘除不影响别处。
            var host = harness.RequireControl<Border>("DiagnosticBanner").Parent as Panel;
            Assert.True(host is not null, "找不到可挂载的宿主 Panel ⇒ 本用例构造不出场景（前提自检）");

            var opener = new Button { Content = "U181-B2", Focusable = true };
            host!.Children.Add(opener);
            await Drain();
            Assert.True(opener.Focus(), "临时触发控件拿不到焦点 ⇒ 前提不成立");
            await Drain();

            var pending = harness.OpenDialogAsync();
            await Drain();

            // 弹窗还开着时把触发控件摘掉 —— 模拟「弹窗的结果就是换页」。
            host.Children.Remove(opener);
            await Drain();

            harness.CancelDialog();
            await pending;
            await Drain();

            Assert.NotSame(opener, harness.FocusedElement);
        });
    }

    // ── C + D：焦点环真的画出来，且颜色来自主题令牌 ─────────

    /// <summary>
    /// C-1（渲染态属性判据）：**焦点落在节点卡上时，它渲染出来的 `BoxShadow`
    /// 必须与失焦时不同，且等于主题的焦点环令牌。**
    ///
    /// 判据刻意量**渲染后的属性值**，不量 XAML 文本 —— 原缺陷正是
    /// 「类名写了、样式写了，但被同文件更靠后的三条 node-card BoxShadow 盖掉」，
    /// 既有守卫（`WorkspaceCanvas08Tests` W4）只断言 XAML 里存在类名字符串，
    /// 样式死掉照样全绿（U152 同型）。
    ///
    /// ⚠️ 这条缺陷**不是**「缺资源键静默失效」那一型（原值 `#B32E726B` 是字面量，
    /// 键根本不参与），也不是「填充/描边语义错配」（`BoxShadow` 无此二义）。
    /// 是纯粹的声明顺序覆盖。所以判据取「两态之差 + 等于令牌值」两个都要：
    /// 前者证明焦点态真的生效了，后者证明它取的是主题色而非又一处魔法值。
    /// </summary>
    [Fact]
    public async Task C1_FocusedNodeCard_RendersAVisibleFocusRing()
    {
        await RunCanvasAsync(async harness =>
        {
            var card = harness.NodeCard;
            var resting = card.BoxShadow;
            var restingBorderBrush = card.BorderBrush;
            var restingThickness = card.BorderThickness;

            Assert.True(card.Focus(), "节点卡拿不到焦点 ⇒ 本用例没走到现场（前提自检）");
            // ⚠️ 必须等过渡跑完再量：node-card 上挂着 160ms 的 BoxShadowsTransition，
            // 只 Drain 一轮会量到**插值中间态**（实测 spread=1.77、alpha=0x75，
            // 都在起止值之间）。这不是"焦点环没生效"，而是我量早了 ——
            // 若照这个中间值去改产品，会把一个已经好了的东西改坏。
            await SettleTransitionsAsync();

            Assert.NotEqual(resting.ToString(), card.BoxShadow.ToString());

            // ⚠️ 真正**看得见**的那一环是 BorderBrush/BorderThickness，不是 BoxShadow。
            // BoxShadow 画在控件边界之外，而节点卡所在的 canvas-node 单元格定高、
            // 外溢会被裁掉 —— 探针实测 8px 的环只画出约 1px 发丝线。
            // 所以判据必须落在描边上，否则"环画了但看不见"照样全绿（首版就是这样）。
            AssertFocusRingBorder(card, restingBorderBrush, restingThickness);
        });
    }

    /// <summary>
    /// C-2（**必经路径判据**）：**节点同时"被选中 + 有焦点"时焦点环仍要在。**
    ///
    /// 这不是补充场景，而是方向键导航的**唯一**路径：
    /// `OnNodeCardKeyDown` 里 `SelectNode` 与 `FocusNodeCard` 是成对调用，
    /// 所以键盘移动焦点必然落进 `.selected` 分支。
    /// 只测"未选中 + 有焦点"会让修复看起来成立而实际路径仍然瞎 ——
    /// `.selected` 那条 BoxShadow 是三个竞争者里最强的一个。
    /// </summary>
    [Fact]
    public async Task C2_SelectedAndFocused_StillShowsTheFocusRing()
    {
        await RunCanvasAsync(async harness =>
        {
            var card = harness.NodeCard;

            // 复刻方向键路径：先选中，再聚焦。
            harness.ViewModel.SelectNode(harness.ViewModel.Nodes[0]);
            await SettleTransitionsAsync();
            var selectedResting = card.BoxShadow;
            var selectedBorderBrush = card.BorderBrush;
            var selectedThickness = card.BorderThickness;

            Assert.True(card.Focus(), "节点卡拿不到焦点 ⇒ 本用例没走到现场（前提自检）");
            await SettleTransitionsAsync();

            Assert.NotEqual(selectedResting.ToString(), card.BoxShadow.ToString());
            // `.selected` 那条也设 BorderBrush（NodeSelected），所以这里同时验证
            // 焦点态**压过**了选中态的描边色 —— 那正是方向键路径的必经分支。
            AssertFocusRingBorder(card, selectedBorderBrush, selectedThickness);
        });
    }

    /// <summary>
    /// D-1（消费者存在性判据）：**`Ariadne.FocusRing` 必须有真实引用点。**
    ///
    /// 原缺陷：令牌**有定义**（`AriadneTheme.axaml` 亮/暗各一份）、
    /// **有覆盖写入**（`ThemeApplication` 的 `OverlayBrushKeys` + `SetBrush`），
    /// 但全仓**零处 `{DynamicResource Ariadne.FocusRing}`** ⇒ 那条
    /// 「焦点环随主题」的接线是空转的，作者换强调色时焦点指示纹丝不动。
    /// 按项目既定判定标准（完全体之后仍零消费者的契约就是没用的契约）
    /// 要么接线要么删 —— 这里选接线，因为焦点色本来就该是可个性化维度，
    /// `UI组件状态表.md` 也早写了「focus 统一使用全局 focus 环」。
    ///
    /// ⚠️ 扫描时必须排除 `bin/` 与 `obj/`：那里有构建副本，会把"零消费"
    /// 假装成"有消费"（本仓的死代码扫描器已因类似原因自伤两次）。
    /// </summary>
    [Fact]
    public void D1_FocusRingToken_HasRealConsumers()
    {
        var root = ResolveDesktopRoot();
        var consumers = new List<string>();

        foreach (var file in Directory.EnumerateFiles(root, "*.*", SearchOption.AllDirectories))
        {
            var relative = Path.GetRelativePath(root, file);
            if (relative.Contains($"bin{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
                || relative.Contains($"obj{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            {
                continue;
            }
            var extension = Path.GetExtension(file);
            if (extension is not (".axaml" or ".cs"))
            {
                continue;
            }

            var text = File.ReadAllText(file);
            // 只认**引用**形态，不认定义（`x:Key="Ariadne.FocusRing"`）
            // 也不认写入侧（ThemeApplication 里的字符串键）。
            if (text.Contains("{DynamicResource Ariadne.FocusRing}", StringComparison.Ordinal)
                || text.Contains("{StaticResource Ariadne.FocusRing}", StringComparison.Ordinal))
            {
                consumers.Add(relative);
            }
        }

        Assert.True(
            consumers.Count > 0,
            "Ariadne.FocusRing 仍然零消费者 ⇒「焦点环随主题」是空转的接线。"
            + "按项目判定标准：要么接线，要么删。");
    }

    /// <summary>
    /// D-2（**跟随判据**）：**个性化换强调色后，焦点环令牌真的跟着变。**
    ///
    /// 这条比 D-1 强一层：D-1 只证"有人引用它"，这条证"改了强调色之后
    /// 焦点环的实际值确实不同了"。若有人把焦点环写成一处硬编码，
    /// D-1 可能仍绿（别处引用了 FocusRing），这条会红。
    ///
    /// 判据取 `Ariadne.Shadow.FocusRing`（焦点环真正的承载键）在
    /// `WriteThreeColorOverlay` 前后不同，且 `Ariadne.FocusRing` 画刷同步跟随。
    /// </summary>
    [Fact]
    public async Task D2_FocusRingFollowsAccentOverride()
    {
        await RunAppAsync(() =>
        {
            var app = Application.Current!;
            var resources = app.Resources;
            // ⚠️ 查询**必须带 ThemeVariant**：Ariadne.Shadow.FocusRing 与
            // Ariadne.FocusRing 都定义在 ResourceDictionary.ThemeDictionaries 的
            // Light/Dark 两份里，不带变体查不到 —— 会误报「令牌不存在」，
            // 也就是红在**基线**断言而不是我要测的那条。本仓已记这一条
            // 「测试基建缺陷伪装成产品缺陷」，我这轮又踩了一次。
            var variant = app.ActualThemeVariant;

            Assert.True(
                app.TryGetResource("Ariadne.Shadow.FocusRing", variant, out var before)
                    && before is BoxShadows,
                "起点就取不到 Ariadne.Shadow.FocusRing ⇒ 判据无效（前提自检）");

            // 刻意取一个与预设青绿差得很远的品牌色（洋红），
            // 否则"派生出来的值恰好一样"会让这条假绿（二次变异那条教训）。
            ThemeApplication.WriteThreeColorOverlay(
                isDark: false,
                main: Color.FromRgb(0xFA, 0xFA, 0xF7),
                surface: Color.FromRgb(0xFF, 0xFF, 0xFF),
                brand: Color.FromRgb(0xC0, 0x1E, 0x8A));

            Assert.True(
                app.TryGetResource("Ariadne.Shadow.FocusRing", variant, out var after)
                    && after is BoxShadows,
                "换色后 Ariadne.Shadow.FocusRing 不见了 ⇒ 覆盖写坏了键");

            Assert.NotEqual(before!.ToString(), after!.ToString());

            // 画刷侧也必须跟着走（它是 .connect-origin / 胶囊焦点边的取色处）。
            Assert.True(
                app.TryGetResource("Ariadne.FocusRing", variant, out var brush)
                    && brush is ISolidColorBrush solid,
                "换色后取不到 Ariadne.FocusRing 画刷");
            Assert.Equal(Color.FromRgb(0xC0, 0x1E, 0x8A), ((ISolidColorBrush)brush!).Color);

            // 不手动还原覆盖层：会话是 PerTest 隔离的，`Application.Current.Resources`
            // 不跨用例泄漏；而 `ClearOverlay` 是私有的，为测试把它开成 public
            // 正是死代码扫描器会标记的那种"测试专用公开 API"。
            return Task.CompletedTask;
        });
    }

    // ── E：键盘连线起点可见 ─────────────────────────────────

    /// <summary>
    /// E-1（用户可见结果判据）：**选好连线起点后，起点端口自己不许被淡出。**
    ///
    /// 原缺陷：`BeginPortDragHighlight` 遍历**全部** Nodes（含起点节点自己），
    /// 而 `TryEvaluateConnection` 对同节点必然判 Self 失败 ⇒ 起点节点的
    /// 所有端口一起走 false 分支、Opacity 打到 0.22。作者选完起点，
    /// 那个起点反而**比别的端口更淡**。
    ///
    /// 判据取 Opacity 与「已选为起点」标记两个：前者是作者眼里的直接现象，
    /// 后者保证它有一个**形状上**可辨的态（一圈实边）而不只是"没变淡"——
    /// 与 U178 那批「状态不能只靠颜色/明暗」一致。
    /// </summary>
    [Fact]
    public async Task E1_KeyboardConnectStart_SourcePortStaysFullyVisible()
    {
        await RunCanvasAsync(async harness =>
        {
            var viewModel = harness.ViewModel;
            var source = viewModel.Nodes[0];

            viewModel.BeginPortDragHighlight(
                source.Id, NodePortKind.Data, NodePortDirection.Out);
            await Drain();

            Assert.Equal(1.0, source.PortDataOutOpacity);
            Assert.True(
                source.PortDataOutIsOrigin,
                "起点端口没有「已选为起点」标记 ⇒ 只是不淡了，作者仍分不清"
                + "「这个是我刚选的起点」和「这个是可以落线的目标」。");

            // 同节点其余端口**仍应**淡出：它们确实不可连（同节点），
            // 保留淡出才让「哪些能落」这件事继续成立。
            Assert.Equal(0.22, source.PortDataInOpacity);
            Assert.False(source.PortDataInIsOrigin);

            // 取消后标记必须清干净，否则那圈实边会留在画布上假装连线还在进行。
            viewModel.EndPortDragHighlight();
            await Drain();
            Assert.False(source.PortDataOutIsOrigin);
            Assert.Equal(1.0, source.PortDataInOpacity);
        });
    }

    /// <summary>
    /// E-2（运行态判据）：**键盘选好起点后，橡皮筋真的画出来了。**
    ///
    /// 原缺陷：鼠标路径（`OnPortPointerPressed`）选完起点就 `UpdateRubberBand`，
    /// 键盘路径（`OnPortKeyDown`）**什么都不画** ⇒ 键盘连线零起点指示。
    ///
    /// 判据取橡皮筋 `Path` 的 `IsVisible` + `Data` 非空（真的有几何），
    /// 不取「调了某个函数」。走真实按键事件把焦点/路由两层都过一遍。
    /// 反向一并钉住：取消连线后橡皮筋必须消失 —— 留着就是"假指示"，比零指示更糟。
    /// </summary>
    [Fact]
    public async Task E2_KeyboardConnectStart_DrawsTheRubberBand()
    {
        await RunCanvasAsync(async harness =>
        {
            var band = harness.RubberBand;
            Assert.False(band.IsVisible, "起点还没选，橡皮筋就已经可见（前提自检）");

            Assert.True(
                harness.PressEnterOnDataOutPort(),
                "找不到可聚焦的数据出端口 ⇒ 本用例的入口不存在（前提自检）");
            await Drain();

            Assert.True(
                band.IsVisible && band.Data is not null,
                "键盘选完连线起点后橡皮筋没画出来 ⇒ 作者看不出线从哪个端口出发，"
                + "只有状态条一句「已选择连线起点」。");

            harness.CancelKeyboardConnection();
            await Drain();
            Assert.False(
                band.IsVisible,
                "取消连线后橡皮筋还留在画布上 ⇒ 把「零指示」换成了「假指示」。");
        });
    }

    /// <summary>
    /// E-3（**反向钉死，不可省**）：**别的节点上不兼容的端口仍然要淡出。**
    ///
    /// 缺了这条，最省事的假修法是「谁都不淡出」—— E-1 会全绿，
    /// 而「哪些端口能落线」这个真正有用的提示被一起删掉了。
    /// </summary>
    [Fact]
    public async Task E3_PointerDrag_StillDimsIncompatiblePortsOnOtherNodes()
    {
        await RunCanvasAsync(async harness =>
        {
            var viewModel = harness.ViewModel;
            viewModel.AddNodeAt("writer", 700, 200);
            await Drain();
            Assert.True(viewModel.Nodes.Count >= 2, "需要两个节点（前提自检）");

            var source = viewModel.Nodes[0];
            var other = viewModel.Nodes[1];

            // 从数据出口起线：别的节点上「数据出口」不是合法落点，必须淡出。
            viewModel.BeginPortDragHighlight(
                source.Id, NodePortKind.Data, NodePortDirection.Out);
            await Drain();

            Assert.Equal(0.22, other.PortDataOutOpacity);
            Assert.False(
                other.PortDataOutIsOrigin,
                "别的节点上的端口被标成了连线起点 ⇒ 起点标记算错了对象");
            // 而合法落点（数据入口）必须亮着，否则等于把提示整个关掉。
            Assert.Equal(1.0, other.PortDataInOpacity);
        });
    }

    // ── F：右栏胶囊焦点可见性 ───────────────────────────────

    /// <summary>
    /// F-1（渲染态两态之差）：**Tab 停在右栏开合胶囊上时屏幕必须有变化。**
    ///
    /// 原缺陷：`RightPanelTogglePill.axaml` 有 `Focusable="True"` 且接了
    /// Enter/Space，但整个文件无 `UserControl.Styles`、无任何 `:focus` 规则，
    /// 主题里也搜不到针对它的选择器 ⇒ 焦点停上去**屏幕毫无变化**，
    /// 作者不知道此刻按 Enter 会发生什么。三个页面都在用它。
    ///
    /// 判据量壳体 Border 的 `BorderBrush` / `BorderThickness` / `BoxShadow`
    /// 三者的两态之差，并要求描边色等于 `Ariadne.FocusRing`
    /// （U69 只管 AutomationProperties.Name，不覆盖视觉焦点）。
    ///
    /// ⚠️ 为什么必须量渲染值而不是读 XAML：本仓「样式挂了但看不见」有两种成因
    /// —— 缺资源键静默失效（DynamicResource 引用不存在的键不报错也不回落，
    /// 属性直接消失）、以及填充/描边语义错配。读 XAML 两种都看不出来。
    /// </summary>
    [Fact]
    public async Task F1_TogglePillFocused_ChangesSomethingVisible()
    {
        await RunAppAsync(async () =>
        {
            var pill = new RightPanelTogglePill { AccessibleName = "U181-F" };
            var window = new Window { Width = 400, Height = 300, Content = pill };
            window.Show();
            await Drain();

            try
            {
                var shell = pill.GetVisualDescendants().OfType<Border>()
                    .FirstOrDefault(border => border.Name == "PillShell");
                Assert.True(shell is not null, "胶囊里找不到 PillShell ⇒ 判据的观测点不存在");

                var restingBrush = shell!.BorderBrush;
                var restingThickness = shell.BorderThickness;
                var restingShadow = shell.BoxShadow;

                Assert.True(pill.Focus(), "胶囊拿不到焦点 ⇒ 本用例没走到现场（前提自检）");
                await Drain();

                Assert.True(
                    !Equals(restingBrush, shell.BorderBrush)
                        || restingThickness != shell.BorderThickness
                        || restingShadow.ToString() != shell.BoxShadow.ToString(),
                    "焦点落在右栏开合胶囊上，壳体的描边色/描边宽/投影**一个都没变** ⇒ "
                    + "屏幕毫无变化，作者不知道按 Enter 会发生什么。");

                Assert.True(
                    shell.TryFindResource("Ariadne.FocusRing", shell.ActualThemeVariant, out var token)
                        && token is ISolidColorBrush expected
                        && shell.BorderBrush is ISolidColorBrush actual
                        && expected.Color == actual.Color,
                    "焦点描边色不是 Ariadne.FocusRing ⇒ 又写了一处组件自己的 focus 颜色，"
                    + "违反「focus 统一使用全局 focus 环」的约定。");
            }
            finally
            {
                window.Content = null;
                window.Close();
                await Drain();
            }
        });
    }

    // ── 脚手架 ──────────────────────────────────────────────

    /// <summary>
    /// 真窗口宿主。刻意用 `MainWindow` 而不是把内容塞进一个裸 `Window`：
    /// 顶栏遮罩、全局遮罩、侧栏导航、弹窗宿主的相对位置**就是本条要验的东西**，
    /// 换宿主等于把被测对象换掉（本仓已记「渲染取证的宿主会量到自己的形状」）。
    /// </summary>
    private sealed class WindowHarness
    {
        public required MainWindow Window { get; init; }
        public required MainWindowViewModel ViewModel { get; init; }

        public T RequireControl<T>(string name) where T : Control
        {
            var found = Window.GetVisualDescendants().OfType<T>()
                .FirstOrDefault(control => control.Name == name);
            Assert.True(found is not null, $"视觉树里找不到 {name} ⇒ 判据的观测点不存在");
            return found!;
        }

        /// <summary>顶栏那个带 MenuFlyout 的项目切换键（唯一挂 Flyout 的顶栏按钮）。</summary>
        public Button ProjectMenuButton =>
            Window.GetVisualDescendants().OfType<Button>()
                .First(button => button.Flyout is MenuFlyout);

        /// <summary>三个窗口控制键（最小化 / 最大化 / 关闭）。</summary>
        public IReadOnlyList<Button> WindowControls =>
            Window.GetVisualDescendants().OfType<Button>()
                .Where(button => button.Classes.Contains("window-control"))
                .ToList();

        /// <summary>弹窗宿主（常驻 ContentControl，`dialog-panel` 类）。</summary>
        public ContentControl DialogHost =>
            Window.GetVisualDescendants().OfType<ContentControl>()
                .First(host => host.Classes.Contains("dialog-panel"));

        public IInputElement? FocusedElement => Window.FocusManager?.GetFocusedElement();

        /// <summary>
        /// 发一次**真实**鼠标按下+抬起。走 `IHeadlessWindow` 而不是 `RaiseEvent`：
        /// 缺陷出在命中测试这一层（遮罩盖不到顶栏），`RaiseEvent` 会把它绕过去。
        /// </summary>
        public void ClickAt(Point pointInWindow)
        {
            Window.MouseDown(pointInWindow, MouseButton.Left);
            Window.MouseUp(pointInWindow, MouseButton.Left);
        }

        /// <summary>
        /// 发一个真实的按键（从 Window 出发，走完整焦点与路由链路）。
        ///
        /// 用 `KeyPressQwerty` 而不是 `KeyPress`：后者在 Avalonia 12.0.5 里
        /// 要求显式传 `PhysicalKey`，Qwerty 那个重载会按标准键盘布局自己映射。
        /// </summary>
        public void PressKey(Key key, RawInputModifiers modifiers = RawInputModifiers.None)
        {
            Window.KeyPressQwerty(MapToPhysical(key), modifiers);
            Window.KeyReleaseQwerty(MapToPhysical(key), modifiers);
        }

        private static PhysicalKey MapToPhysical(Key key) => key switch
        {
            Key.Tab => PhysicalKey.Tab,
            Key.Enter => PhysicalKey.Enter,
            Key.Escape => PhysicalKey.Escape,
            Key.Space => PhysicalKey.Space,
            _ => throw new NotSupportedException($"本用例还没给 {key} 建物理键映射"),
        };

        /// <summary>开一个普通确认弹窗，返回它的 completion（测试末尾要 await 掉）。</summary>
        public Task<int> OpenDialogAsync()
        {
            var dialog = new ConfirmDialogViewModel(
                "U181",
                "U181 焦点隔离判据用的弹窗",
                new[]
                {
                    new DialogButton("确定", DialogButtonVariant.Primary, 0),
                    new DialogButton("取消", DialogButtonVariant.Subtle, 1),
                })
            {
                // ⚠️ `CancelResultIndex` **必须显式给 >= 0 的值**：
                // `ConfirmDialogViewModel.Cancel()` 在它 < 0 时**什么都不做**（默认就是 -1），
                // 于是 `RequestCancelActive()` 关不掉弹窗、`Completion` 永不完成，
                // 用例挂死在末尾那个 await 上 —— 我第一版就这么挂了两轮，
                // 而**挂死看起来和「测试很慢」一模一样**，在这台跑不完全量的机器上
                // 极易被误判成环境问题。
                ConfirmResultIndex = 0,
                CancelResultIndex = 1,
            };
            return DialogService.Current.ConfirmAsync(dialog);
        }

        public void CancelDialog() => DialogService.Current.RequestCancelActive();
    }

    private static async Task RunWindowAsync(Func<WindowHarness, Task> body)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);

        await session.Dispatch(
            async () =>
            {
                var names = DisplayNameService.LoadDefault();
                DialogService.Initialize(names);
                var viewModel = new MainWindowViewModel(
                    names,
                    DispatchProxy.Create<IAriadneBackendClient, SoftBackendProxy>());
                // Window 不能塞进另一个 Window 的 Content ⇒ 直接 Show MainWindow。
                var window = new MainWindow { Width = 1400, Height = 900, DataContext = viewModel };
                window.Show();
                await Drain();

                try
                {
                    await body(new WindowHarness { Window = window, ViewModel = viewModel });
                }
                finally
                {
                    window.Close();
                    await Drain();
                }

                return true;
            },
            CancellationToken.None);
    }

    /// <summary>画布宿主：C / E 两组判据都要一个真实实体化的节点卡与端口。</summary>
    private sealed class CanvasHarness
    {
        public required WorkspacePageView View { get; init; }
        public required WorkspacePageViewModel ViewModel { get; init; }

        /// <summary>节点卡本体（焦点宿主，挂 keyboard-target/node-card 三个类）。</summary>
        public Border NodeCard =>
            View.GetVisualDescendants().OfType<Border>()
                .First(border => border.Name == "NodeKeyboardFocusHost");

        /// <summary>
        /// 橡皮筋 Path（键盘/鼠标连线的起点指示）。
        /// ⚠️ 必须写全 `Avalonia.Controls.Shapes.Path`：裸 `Path` 在本文件里
        /// 解析成 `System.IO.Path`（静态类），编译期直接报 CS0722。
        /// </summary>
        public Avalonia.Controls.Shapes.Path RubberBand =>
            View.GetVisualDescendants().OfType<Avalonia.Controls.Shapes.Path>()
                .First(path => path.Name == "RubberBandPath");

        /// <summary>
        /// 对一个真实的数据出端口发 Enter。走 `RaiseEvent` 让 `OnPortKeyDown`
        /// 的 `ReferenceEquals(sender, e.Source)` 闸门成立 —— 那道闸是产品逻辑，
        /// 绕过它就测不到真实路径。
        /// </summary>
        public bool PressEnterOnDataOutPort()
        {
            var port = View.GetVisualDescendants().OfType<Border>()
                .FirstOrDefault(border =>
                    border.Focusable
                    && border.Tag is string tag
                    && tag.StartsWith("data|out", StringComparison.Ordinal));
            if (port is null)
            {
                return false;
            }

            port.Focus();
            port.RaiseEvent(new KeyEventArgs
            {
                RoutedEvent = InputElement.KeyDownEvent,
                Key = Key.Enter,
                Source = port,
            });
            return true;
        }

        /// <summary>取消键盘连线（走 Esc 那条真实路径）。</summary>
        public void CancelKeyboardConnection()
        {
            View.RaiseEvent(new KeyEventArgs
            {
                RoutedEvent = InputElement.KeyDownEvent,
                Key = Key.Escape,
                Source = View,
            });
        }
    }

    private static async Task RunCanvasAsync(Func<CanvasHarness, Task> body)
    {
        await RunAppAsync(async () =>
        {
            var names = DisplayNameService.LoadDefault();
            DialogService.Initialize(names);
            var viewModel = new WorkspacePageViewModel(
                names,
                DispatchProxy.Create<IAriadneBackendClient, SoftBackendProxy>());
            var view = new WorkspacePageView { DataContext = viewModel };
            var window = new Window { Width = 1400, Height = 900, Content = view };
            window.Show();
            await Drain();

            // 端口只在"精密控制"档才可见/可聚焦；节点必须真的加进去并实体化。
            viewModel.AddNodeAt("summarizer", 200, 200);
            await Drain();

            try
            {
                await body(new CanvasHarness { View = view, ViewModel = viewModel });
            }
            finally
            {
                window.Content = null;
                window.Close();
                await Drain();
            }
        });
    }

    /// <summary>只要一个 Avalonia 应用会话（不需要特定视图）时用这个。</summary>
    private static async Task RunAppAsync(Func<Task> body)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);

        await session.Dispatch(
            async () =>
            {
                await body();
                return true;
            },
            CancellationToken.None);
    }

    /// <summary>仓库里的 `desktop/` 目录（用于源码级扫描）。</summary>
    private static string ResolveDesktopRoot()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.True(dir is not null, "找不到 Ariadne.slnx ⇒ 无法定位 desktop/ 根（前提自检）");
        return dir!;
    }

    private static async Task Drain()
    {
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.SystemIdle);
    }

    /// <summary>
    /// 等属性过渡跑完再量渲染值。
    ///
    /// 画布节点卡挂着 160ms 的 `BoxShadowsTransition`；只 `Drain()` 一轮会量到
    /// **插值中间态**，看起来就像"焦点环取的不是主题令牌"。
    /// 300ms 给的是 160ms 过渡的近两倍余量 —— 取两倍而不是刚好 160，
    /// 是因为 headless 的时钟推进不保证与真实时间同步。
    /// </summary>
    private static async Task SettleTransitionsAsync()
    {
        await Drain();
        await Task.Delay(300);
        await Drain();
    }

    /// <summary>
    /// 焦点环的**可见**判据：描边色等于 `Ariadne.FocusRing`、描边变粗，且两者都与静息态不同。
    ///
    /// 为什么不验 `BoxShadow`：它画在控件边界之外，而节点卡所在的 canvas-node
    /// 单元格定高、外溢会被裁掉（探针实测 8px 环只剩约 1px）。
    /// 描边画在边界之内，永远不会被裁 —— 那才是作者真正看到的东西。
    /// </summary>
    private static void AssertFocusRingBorder(
        Border card, IBrush? restingBrush, Thickness restingThickness)
    {
        Assert.True(
            card.TryFindResource("Ariadne.FocusRing", card.ActualThemeVariant, out var token)
                && token is ISolidColorBrush,
            "主题里取不到 Ariadne.FocusRing ⇒ 焦点环颜色又回到了魔法值");

        Assert.True(
            card.BorderBrush is ISolidColorBrush actual
                && ((ISolidColorBrush)token!).Color == actual.Color,
            $"节点卡获得焦点后描边色不是 Ariadne.FocusRing（实际 {card.BorderBrush}）"
            + " ⇒ 焦点环没生效，或者又写了一处组件自己的 focus 颜色。");

        // ⚠️ **刻意不断言「描边色与静息态不同」**。
        // `Ariadne.FocusRing` 与 `Ariadne.NodeSelected` 都派生自强调色，
        // 所以"已选中"的节点获得焦点时**两个色值本来就相等** ——
        // 首版写了 NotEqual，红在这里，而产品是对的：
        // 按 `UI组件状态表.md`「focus 统一使用全局 focus 环，各组件不另写 focus 颜色」，
        // 焦点就该用那一个颜色；为了让断言好过而给焦点另配一个色，
        // 恰恰会违反那条约定（本仓已记「相关既有守卫失效时，正解是改用例而非改产品」）。
        //
        // ⇒ 区分靠**粗细**（这也正是「状态不能只靠颜色」要求的形状差异），
        // 下面这条因此是强判据而不是补充判据。
        Assert.True(
            card.BorderThickness.Left > restingThickness.Left,
            $"焦点态描边没变粗（{restingThickness.Left} → {card.BorderThickness.Left}）"
            + " ⇒ 与「已选中」态在屏幕上完全同形（两者描边色都派生自强调色），"
            + "作者分不出焦点在哪。");
        _ = restingBrush;
    }

    /// <summary>`candidate` 是否落在 `ancestor` 的视觉子树内（含它自己）。</summary>
    private static bool IsWithin(Control candidate, Control ancestor) =>
        ReferenceEquals(candidate, ancestor)
        || candidate.GetVisualAncestors().Any(node => ReferenceEquals(node, ancestor));

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    /// <summary>DispatchProxy 的宿主类不能 sealed（运行时要派生它）。</summary>
    private class SoftBackendProxy : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }
            if (targetMethod?.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (targetMethod?.ReturnType.IsGenericType == true
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                var value = resultType.IsValueType ? Activator.CreateInstance(resultType) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }
            return targetMethod?.ReturnType is { IsValueType: true } vt
                ? Activator.CreateInstance(vt)
                : null;
        }
    }
}
