using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Threading;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U204 的**渲染取证**。静态守卫（<see cref="ScaleTokenUsageTests"/>）能证
/// 「字面量换成了 token 引用」，**证不了那些字号在屏幕上真的生效**。
///
/// <para>
/// ⚠️ 这个缺口在本项目是实的，而且恰好命中这次改动的形态：
/// <b>缺资源键静默失效</b> —— <c>DynamicResource</c> 拼错键名时既不报错也不回落，
/// 表现是「那个属性整个失效」。这次一口气把 76 处尺寸字面量换成 token 引用，
/// 任何一个键名写错都会让那处控件回落到继承字号/零描边，
/// 而所有静态判据（「引用了 token」）照样全绿。
/// </para>
///
/// <para>
/// ⚠️ 另一个只有渲染才抓得到的：<c>BindingPriority.LocalValue</c>(0) 最高，
/// 元素上的内联属性压过所有样式层且**静默**。D 条把 <c>.subtitle</c> 的两处
/// 上下文覆盖删掉之后，若某处元素上还内联写着 <c>FontSize</c>，
/// 拆类就等于没拆 —— 只有量真实的 <c>FontSize</c> 才看得出来。
/// </para>
/// </summary>
public sealed class ScaleTokenRenderSession : IDisposable
{
    public ScaleTokenRenderSession() =>
        Session = HeadlessUnitTestSession.StartNew(
            typeof(ScaleHeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);

    public HeadlessUnitTestSession Session { get; }

    public void Dispose() => Session.Dispose();

    private static class ScaleHeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }
}

[Collection("AvaloniaHeadless")]
public sealed class ReadingSurfaceRenderTests : IClassFixture<ScaleTokenRenderSession>
{
    private readonly ScaleTokenRenderSession _session;

    public ReadingSurfaceRenderTests(ScaleTokenRenderSession session) => _session = session;

    /// <summary>
    /// U204-D 渲染取证：三个文字类在**真实上下文里**各自渲染成一个确定字号。
    ///
    /// # 宿主为什么要真的套 Border.empty-state / Border.inspector-group
    ///
    /// 那两条上下文覆盖正是 D 条的病灶。把 TextBlock 裸放在窗口里，
    /// 覆盖规则不会触发 —— 用例会「证明」问题不存在。
    /// ⇒ 渲染取证的宿主必须复现**决定尺寸的那个约束**，
    /// 这里就是「祖先是那两个 Border 类」。
    /// </summary>
    [Fact]
    public async Task TextClasses_RenderOneSizeEachInsideTheirRealAncestors()
    {
        await RunAsync(async () =>
        {
            // 三种上下文各放一份 .subtitle，外加两个拆出来的类。
            var plainSubtitle = new TextBlock { Text = "普通小标题" };
            plainSubtitle.Classes.Add("subtitle");

            var emptySubtitle = new TextBlock { Text = "空态里的小标题" };
            emptySubtitle.Classes.Add("subtitle");
            var emptyTitle = new TextBlock { Text = "空态标题" };
            emptyTitle.Classes.Add("empty-title");
            var emptyHost = new Border();
            emptyHost.Classes.Add("empty-state");
            emptyHost.Child = new StackPanel { Children = { emptySubtitle, emptyTitle } };

            var inspectorSubtitle = new TextBlock { Text = "检查器里的小标题" };
            inspectorSubtitle.Classes.Add("subtitle");
            var inspectorLabel = new TextBlock { Text = "分区标签" };
            inspectorLabel.Classes.Add("inspector-label");
            var inspectorHost = new Border();
            inspectorHost.Classes.Add("inspector-group");
            inspectorHost.Child = new StackPanel { Children = { inspectorSubtitle, inspectorLabel } };

            var root = new StackPanel
            {
                VerticalAlignment = Avalonia.Layout.VerticalAlignment.Top,
                Children = { plainSubtitle, emptyHost, inspectorHost },
            };
            var window = new Window { Width = 480, Height = 400, Content = root };
            window.Show();
            await DrainAsync();
            window.UpdateLayout();
            await DrainAsync();

            try
            {
                // 自检：文字得先真的排布出来，否则下面比的是一堆继承默认值。
                Assert.True(
                    plainSubtitle.Bounds.Height > 4,
                    $"文字没排布出来（高 {plainSubtitle.Bounds.Height}）——本用例的结论全部无效。");

                var subtitleToken = ScaleTokenPaths.ThemeDouble("Ariadne.Size.Subtitle");
                var titleToken = ScaleTokenPaths.ThemeDouble("Ariadne.Size.Title");
                var microToken = ScaleTokenPaths.ThemeDouble("Ariadne.Size.Micro");

                // ① `.subtitle` 在三种上下文里必须是**同一个**字号。
                //    这是 D 条的核心判据：修复前这三个数是 16 / 20 / 11。
                Assert.Equal(subtitleToken, plainSubtitle.FontSize);
                Assert.Equal(subtitleToken, emptySubtitle.FontSize);
                Assert.Equal(subtitleToken, inspectorSubtitle.FontSize);

                // ② 拆出来的两个类各自落在自己的 token 上（而不是回落到继承值）。
                //    ⚠️ 这一条是「键名拼错」的唯一探测器：拼错时 FontSize 会
                //    静默继承祖先的值，而静态判据看到的仍是「引用了 token」。
                Assert.Equal(titleToken, emptyTitle.FontSize);
                Assert.Equal(microToken, inspectorLabel.FontSize);

                // ③ 层级关系必须真的分开。少了这条，把三个 token 都改成 16
                //    也能让 ① ② 全绿 —— 那是一致的无层级。
                Assert.True(
                    emptyTitle.FontSize > plainSubtitle.FontSize,
                    $"空态标题 {emptyTitle.FontSize} 不大于普通小标题 {plainSubtitle.FontSize}");
                Assert.True(
                    inspectorLabel.FontSize < plainSubtitle.FontSize,
                    $"检查器分区标签 {inspectorLabel.FontSize} 不小于普通小标题 {plainSubtitle.FontSize}");
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
    /// U204-A 渲染取证：换成 token 的字号 / 圆角 / 描边**真的画出来了**。
    ///
    /// 用真实页面文件里那几种写法各取一个代表：`FontSize` 走 <c>Ariadne.Size.Caption</c>、
    /// `BorderThickness` 走 <c>Ariadne.Stroke.HairlineAll</c>、
    /// `CornerRadius` 走 <c>Ariadne.Radius.Large</c>。
    ///
    /// ⚠️ <c>BorderThickness</c> 那条尤其必要：它是这次唯一**换了目标类型**的
    /// token（<c>Thickness</c> 而非 <c>x:Double</c>）。若误用了同名的
    /// <c>Ariadne.Stroke.Hairline</c>（<c>x:Double</c>），<c>DynamicResource</c>
    /// 不跑类型转换器 ⇒ 描边**整个消失**，而 XAML 里那行字看起来完全正确。
    /// </summary>
    [Fact]
    public async Task ScaleTokensOnControls_ResolveToTheThemeValues()
    {
        await RunAsync(async () =>
        {
            var text = new TextBlock { Text = "12px 说明文字" };
            var border = new Border { Child = text };
            // 用与页面文件同一条路子取值：DynamicResource → 主题字典。
            border.SetValue(
                Border.BorderThicknessProperty,
                ResolveRequired<Thickness>("Ariadne.Stroke.HairlineAll"));
            border.SetValue(
                Border.CornerRadiusProperty,
                ResolveRequired<CornerRadius>("Ariadne.Radius.Large"));
            text.SetValue(
                TextBlock.FontSizeProperty,
                ResolveRequired<double>("Ariadne.Size.Caption"));

            var window = new Window { Width = 320, Height = 160, Content = border };
            window.Show();
            await DrainAsync();
            window.UpdateLayout();
            await DrainAsync();

            try
            {
                Assert.True(border.Bounds.Height > 4, "Border 没排布出来，结论无效。");

                var hairline = ScaleTokenPaths.ThemeThickness("Ariadne.Stroke.HairlineAll");
                Assert.Equal(hairline[0], border.BorderThickness.Left);
                Assert.Equal(hairline[1], border.BorderThickness.Top);
                Assert.True(
                    border.BorderThickness.Left > 0,
                    "描边宽度为 0 —— 这正是「把 Thickness token 写成 x:Double token」的症状："
                    + "DynamicResource 不跑类型转换器，静默失效。");

                Assert.Equal(
                    ScaleTokenPaths.ThemeCornerRadius("Ariadne.Radius.Large"),
                    border.CornerRadius.TopLeft);
                Assert.Equal(
                    ScaleTokenPaths.ThemeDouble("Ariadne.Size.Caption"),
                    text.FontSize);
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
    /// U204-B 渲染取证：<c>WorksPageViewModel</c> 的稿纸宽**真的从主题 token 读**，
    /// 而不是走那个恰好同值的兜底常量。
    ///
    /// # 判据为什么必须换成一个「绝不会等于兜底值」的数
    ///
    /// 主题里那个 token 当前是 720，兜底常量也是 720 ⇒ 直接断言
    /// 「VM 返回 720」在**修复前后完全一样**，那是空测。
    /// 本项目有先例（「变异全绿也可能是遗留状态」）：只有把资源改成一个
    /// 绝不存在于代码里的值、再看 VM 是否跟随，才能区分「真的读了」
    /// 与「值来自别处」。这里取 913（非圆整、不等于任何现有尺度）。
    /// </summary>
    [Fact]
    public async Task DocumentSurfaceMaxWidth_FollowsTheThemeTokenNotTheFallback()
    {
        await RunAsync(async () =>
        {
            var app = Application.Current;
            Assert.NotNull(app);

            var themeValue = ScaleTokenPaths.ThemeDouble("Ariadne.Reading.SurfaceMaxWidth");
            var vm = new WorksPageViewModel(
                DisplayNameService.LoadDefault(),
                DispatchProxy.Create<IAriadneBackendClient, ScaleTokenSoftBackend>());

            // 先证「常态下取的是主题值」。
            Assert.Equal(themeValue, vm.DocumentSurfaceMaxWidth);

            // 再把资源换成一个绝不等于兜底常量的值，VM 必须跟随。
            // 覆盖写在应用级字典（优先于 Styles.Resources），用完必须还原 ——
            // headless session 是 Collection 级共享的。
            const double probe = 913d;
            var hadOverride = app!.Resources.ContainsKey("Ariadne.Reading.SurfaceMaxWidth");
            try
            {
                app.Resources["Ariadne.Reading.SurfaceMaxWidth"] = probe;
                await DrainAsync();

                Assert.Equal(probe, vm.DocumentSurfaceMaxWidth);
            }
            finally
            {
                if (!hadOverride)
                {
                    app.Resources.Remove("Ariadne.Reading.SurfaceMaxWidth");
                }
                else
                {
                    app.Resources["Ariadne.Reading.SurfaceMaxWidth"] = themeValue;
                }

                await DrainAsync();
            }

            // 还原后必须回到主题值 —— 少了这条，上面那次覆盖可能泄漏给别的用例。
            Assert.Equal(themeValue, vm.DocumentSurfaceMaxWidth);
        });
    }

    private static T ResolveRequired<T>(string key)
    {
        var app = Application.Current;
        Assert.NotNull(app);
        Assert.True(
            app!.TryGetResource(key, app.ActualThemeVariant, out var value),
            $"主题里查不到资源键「{key}」——DynamicResource 遇到这种情况**不报错也不回落**，"
            + "生产里的症状是那个属性整个失效。");
        Assert.IsType<T>(value);
        return (T)value!;
    }

    /// <summary>
    /// ⚠️ **必须用有返回值的那个 <c>Dispatch</c> 重载**（<c>Func&lt;Task&lt;T&gt;&gt;</c>）。
    /// 写成 <c>Func&lt;Task&gt;</c> 重载时**整个用例体一次都不执行**，
    /// 连 <c>Assert.Fail</c> 都报绿（`GroupDividerRenderTests` 把这个坑写在注释里）。
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

    /// 本用例只关心稿纸宽度这一个属性，后端一律不许被调 ——
    /// 抛而不是返回默认值：默认值会让「VM 悄悄发了个请求」变成静默通过。
    private class ScaleTokenSoftBackend : DispatchProxy
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
}
