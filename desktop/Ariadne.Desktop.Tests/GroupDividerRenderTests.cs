using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Media;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U205 的**渲染取证**：分组线不能只是「XAML 里有那个 Border」。
///
/// <para>
/// <see cref="ButtonClusterGroupingTests"/> 已经守住「判为多组的簇逐个都有
/// <c>Border.group-divider</c>」，但那是**静态文本判据** —— 它证不了那条线
/// 在屏幕上真的有宽度、有颜色、比按钮矮一截。本项目已有先例说明这个缺口有多实：
/// U148 / U152 / U207-C 三次都是「样式写对了但被内联属性静默压过」
/// （<c>BindingPriority</c>：<c>LocalValue=0</c> 数值最小、优先级最高，
/// 元素上的内联属性压过所有样式层，**不报错、不回落**）。
/// </para>
///
/// <para>
/// ⚠️ 另一个只有真实渲染才抓得到的形态：<b>缺资源键静默失效</b>。
/// 三个 token（<c>Ariadne.Stroke.Hairline</c> / <c>Ariadne.Group.DividerInset</c> /
/// <c>Ariadne.BorderMuted</c>）任何一个拼错，<c>DynamicResource</c> 都
/// 既不报错也不回落 —— 结果是宽度 0（线整个不见）而所有静态判据照样全绿。
/// </para>
/// </summary>
public sealed class GroupDividerRenderSession : IDisposable
{
    public GroupDividerRenderSession() =>
        Session = HeadlessUnitTestSession.StartNew(
            typeof(DividerHeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);

    public HeadlessUnitTestSession Session { get; }

    public void Dispose() => Session.Dispose();

    private static class DividerHeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }
}

[Collection("AvaloniaHeadless")]
public sealed class GroupDividerRenderTests : IClassFixture<GroupDividerRenderSession>
{
    private readonly GroupDividerRenderSession _session;

    public GroupDividerRenderTests(GroupDividerRenderSession session) => _session = session;

    /// <summary>
    /// 分组线必须**真的画出来**：有非零宽度、有可见底色、且比相邻按钮矮。
    ///
    /// # 判据为什么是这三条
    ///
    /// - **宽度 &gt; 0**：三个 token 任一拼错都会让它塌成 0 宽，而静态判据全绿。
    /// - **底色不透明**：`Background` 没生效时线是透明的 —— 占位却看不见，
    ///   这比没有线更坏（版式已经为它让了空间，作者却得不到那个分组信息）。
    /// - **比按钮矮**：满高的线会读成「表格列线」而不是「组边界」，
    ///   这是主题注释里那句「上下各内缩 5 让线比按钮矮一截」的可执行形态。
    ///   ⚠️ 少了这一条，「把 DividerInset 改成 0」不会有任何东西变红。
    /// </summary>
    [Fact]
    public async Task GroupDivider_RendersAsAVisibleHairlineShorterThanTheButtons()
    {
        await RunAsync(async () =>
        {
            var cluster = new StackPanel
            {
                Orientation = Avalonia.Layout.Orientation.Horizontal,
                Spacing = 8,
                // ⚠️ **必须 Top 对齐**，否则本用例量的是宿主而不是产品。
                // 我第一版没写，结果 StackPanel 被窗口竖向拉满，
                // `VerticalAlignment=Stretch` 的分组线跟着长到 110px（按钮 32px），
                // 用例报「线比按钮高」——**那是我的宿主不真实，不是产品缺陷**：
                // 真实页面里这些簇都在 `Grid` 的一行内（`SettingsPageView.axaml:532`
                // 的 `RowDefinitions="Auto,…"`），行高由按钮撑出来，Stretch 才是对的。
                // ⇒ 渲染取证的宿主必须复现「决定尺寸的那个约束」，
                // 否则测出来的是测试脚手架的形状。
                VerticalAlignment = Avalonia.Layout.VerticalAlignment.Top,
            };
            var left = new Button { Content = "保存" };
            var divider = new Border();
            divider.Classes.Add("group-divider");
            var right = new Button { Content = "吊销" };
            cluster.Children.Add(left);
            cluster.Children.Add(divider);
            cluster.Children.Add(right);

            var window = new Window { Width = 320, Height = 120, Content = cluster };
            window.Show();
            await DrainAsync();
            window.UpdateLayout();
            await DrainAsync();

            try
            {
                // 自检：按钮本身得先真的排布出来，否则下面比的是一堆 0。
                Assert.True(
                    left.Bounds.Height > 8 && right.Bounds.Height > 8,
                    $"按钮没排布出来（左 {left.Bounds.Height} 右 {right.Bounds.Height}）——"
                    + "布局没跑，本用例的一切结论都无效。");

                Assert.True(
                    divider.Bounds.Width > 0,
                    "分组线宽度为 0 —— 三个 token（Stroke.Hairline / Group.DividerInset /"
                    + " BorderMuted）任一拼错都会这样，而 DynamicResource **不报错也不回落**。");

                var fill = divider.Background as ISolidColorBrush;
                Assert.True(
                    fill is not null && fill.Color.A > 0,
                    $"分组线没有可见底色（Background={divider.Background?.ToString() ?? "null"}）——"
                    + "它占了版式空间却看不见，比没有线更坏。");

                var buttonHeight = Math.Max(left.Bounds.Height, right.Bounds.Height);
                Assert.True(
                    divider.Bounds.Height < buttonHeight,
                    $"分组线高 {divider.Bounds.Height:0.##} ≥ 按钮高 {buttonHeight:0.##} ——"
                    + "满高的线会读成「表格列线」而不是「组边界」"
                    + "（主题里 Group.DividerInset 的上下内缩就是为这个）。");
            }
            finally
            {
                window.Content = null;
                window.Close();
                await Task.Yield();
            }
        });
    }

    /// <summary>
    /// ⚠️ **必须用有返回值的那个 <c>Dispatch</c> 重载**（<c>Func&lt;Task&lt;T&gt;&gt;</c>）。
    /// 写成 <c>Dispatch(body, ct)</c>（<c>Func&lt;Task&gt;</c> 重载）时
    /// **整个用例体一次都不执行**，连 <c>Assert.Fail</c> 都报绿。
    /// `AiPanelAndDiagnosticLayoutTests` / `ReadingMarkdownRenderTests` 都把这个坑
    /// 写在了注释里 —— 复用先例时，先例的前提写在它的注释里，不会跟着被抄的代码走。
    /// </summary>
    private Task RunAsync(Func<Task> body) =>
        _session.Session.Dispatch(async () =>
        {
            await body();
            return true;
        }, CancellationToken.None);

    private static async Task DrainAsync()
    {
        for (var i = 0; i < 16; i++)
        {
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Render);
        }
    }
}
