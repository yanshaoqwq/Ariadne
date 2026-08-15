using System.Reflection;
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
using Ariadne.Desktop.Views;
using Xunit;
// 别名而非 `using Avalonia.Controls.Shapes;`：那个命名空间里的 `Path`
// 会盖掉 `System.IO.Path`，本文件要用 Path.Combine 定位源码。
using ShapePath = Avalonia.Controls.Shapes.Path;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U155：画布拖线（橡皮筋）的颜色必须来自主题令牌，不能写死。
///
/// 缺陷形态：`BrushForPortKind` 把三种引脚色抄成十六进制字面量
/// `#8B939D`/`#7C3AED`/`#2E726B`，而这三枚令牌在主题里**亮暗各有一套值**。
/// 抄下来的三个值恰是「亮色 2 个 + 暗色 1 个」的混合——
/// **任何单一主题下都不完全正确**，换主题时线的颜色纹丝不动。
///
/// 判据刻意取「**橡皮筋 Path 实际拿到的 Color** 是否等于当前变体下
/// `Ariadne.Color.Edge*` 令牌的值」，而不是：
/// - ❌「函数返回非 null」——缺陷版本也返回非 null
/// - ❌「源码里出现了 ResolveColor / TryFindResource」——缓存成 static 后照样绿
/// - ❌「亮色下取到的值等于某个写死的期望十六进制」——那只是把同一个魔数抄到测试里，
///      主题改色时测试会反过来拦住正确的改动
///
/// 期望值一律**从主题字典现取**，测试里零颜色字面量。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class PortBrushThemeTests
{
    /// <summary>三种引脚各自应当命中的主题令牌（Color 键，Brush 键去掉 `.Color` 段）。</summary>
    public static TheoryData<NodePortKind, string> PortKindTokens => new()
    {
        { NodePortKind.Data, "Ariadne.Color.EdgeData" },
        { NodePortKind.Control, "Ariadne.Color.EdgeControl" },
        { NodePortKind.Communication, "Ariadne.Color.EdgeCommunication" },
    };

    /// <summary>
    /// 亮色变体下，橡皮筋取到的色必须等于**亮色字典**里对应令牌的值。
    ///
    /// 这一条单独拦 `Control`：缺陷版本给它抄的是暗色值，亮色主题下必红。
    /// </summary>
    [Theory]
    [MemberData(nameof(PortKindTokens))]
    public async Task LightVariant_RubberBandStroke_EqualsLightThemeToken(NodePortKind kind, string colorKey)
    {
        await RunWithViewAsync(async view =>
        {
            SetVariant(ThemeVariant.Light);
            await DrainAsync();

            var expected = ResolveTokenColor(view, colorKey, ThemeVariant.Light);
            var actual = DrawRubberBandColor(view, kind);

            Assert.Equal(expected, actual);
        });
    }

    /// <summary>
    /// 暗色变体下，橡皮筋取到的色必须等于**暗色字典**里对应令牌的值。
    ///
    /// 这一条单独拦 `Communication` 与 `Data`：缺陷版本给它们抄的是亮色值，暗色主题下必红。
    /// 与上一条合起来，就没有任何一个主题能让缺陷版本全绿——这正是本编号的性质。
    /// </summary>
    [Theory]
    [MemberData(nameof(PortKindTokens))]
    public async Task DarkVariant_RubberBandStroke_EqualsDarkThemeToken(NodePortKind kind, string colorKey)
    {
        await RunWithViewAsync(async view =>
        {
            SetVariant(ThemeVariant.Dark);
            await DrainAsync();

            var expected = ResolveTokenColor(view, colorKey, ThemeVariant.Dark);
            var actual = DrawRubberBandColor(view, kind);

            Assert.Equal(expected, actual);
        });
    }

    /// <summary>
    /// **主变异点：切主题后重新拖出的线，颜色必须跟着变。**
    ///
    /// 这一条拦的是「取了令牌但缓存成 static」那类改法——
    /// 那种写法在单一变体下取值正确、上面两条也能全绿，
    /// 但进程内只算一次，用户换主题后线仍是旧色，缺陷实质未修。
    ///
    /// 亮/暗两套字典里三枚令牌的值**都不相同**（见 AriadneTheme.axaml 的
    /// ThemeDictionaries），所以「切了变体颜色却没变」只可能是取值被缓存了。
    /// </summary>
    [Theory]
    [MemberData(nameof(PortKindTokens))]
    public async Task SwitchingVariant_RepaintsRubberBandWithNewColor(NodePortKind kind, string colorKey)
    {
        await RunWithViewAsync(async view =>
        {
            SetVariant(ThemeVariant.Light);
            await DrainAsync();
            var lightExpected = ResolveTokenColor(view, colorKey, ThemeVariant.Light);
            var lightActual = DrawRubberBandColor(view, kind);

            SetVariant(ThemeVariant.Dark);
            await DrainAsync();
            var darkExpected = ResolveTokenColor(view, colorKey, ThemeVariant.Dark);
            var darkActual = DrawRubberBandColor(view, kind);

            // 前置：两套字典的值确实不同，否则这条用例不构成证据。
            Assert.NotEqual(lightExpected, darkExpected);

            Assert.Equal(lightExpected, lightActual);
            Assert.Equal(darkExpected, darkActual);
            // 最终判据落在「用户看到的那根线」：切主题后重画，颜色必须真的变了。
            Assert.NotEqual(lightActual, darkActual);
        });
    }

    /// <summary>
    /// 个性化强调色（`ThemeApplication` 写的应用级覆盖层）也必须被橡皮筋读到。
    ///
    /// `Ariadne.EdgeData` 在覆盖清单 `OverlayBrushKeys` 里，用户自定义强调色时
    /// 它会被改写成新的强调色。若橡皮筋读的是 ThemeDictionaries 里的预设值
    /// （或干脆写死），rose/amber/violet 这些主题下就会有一根青绿线横在画布上。
    /// </summary>
    [Fact]
    public async Task CustomAccentOverlay_IsPickedUpByRubberBand()
    {
        await RunWithViewAsync(async view =>
        {
            SetVariant(ThemeVariant.Light);
            await DrainAsync();
            var presetData = DrawRubberBandColor(view, NodePortKind.Data);

            // 走真实的个性化路径：violet 预设 + 自定义品牌色，而不是手写资源。
            ThemeApplication.Apply(
                "violet",
                mainHex: null,
                surfaceHex: null,
                brandHex: null);
            await DrainAsync();

            var overlayExpected = ResolveTokenColor(view, "Ariadne.EdgeData", ThemeVariant.Light);
            var overlayActual = DrawRubberBandColor(view, NodePortKind.Data);

            // 前置：覆盖层确实改了 EdgeData，否则这条用例没在测覆盖层。
            Assert.NotEqual(presetData, overlayExpected);
            Assert.Equal(overlayExpected, overlayActual);
        });
    }

    /// <summary>
    /// 源码守卫：这条路径上不得再出现颜色字面量。
    ///
    /// 上面的行为用例已足以拦住「取错变体」和「缓存」两类改法，但拦不住
    /// 「先查令牌、查不到就 `Color.Parse` 一个写死的设计值」——那个分支在测试里
    /// 永远走不到，却会在真实缺资源时把魔数画到画布上。所以额外钉一条源码断言。
    ///
    /// 断言收窄到 `UpdateRubberBand`…`ResetEdgeDrag` 这一段（引脚取色的全部实现），
    /// 不去管整个文件——画布代码另有 agent 在改，收窄边界避免误伤。
    /// </summary>
    [Fact]
    public void PortBrushResolution_ContainsNoColorLiterals()
    {
        var source = File.ReadAllText(ResolveViewSourcePath());
        var start = source.IndexOf("private void UpdateRubberBand(", StringComparison.Ordinal);
        var end = source.IndexOf("private void ResetEdgeDrag(", StringComparison.Ordinal);
        Assert.True(start > 0 && end > start, "取色实现的边界锚点没找到，用例本身失效了");

        var region = source[start..end];

        Assert.DoesNotContain("Color.Parse", region, StringComparison.Ordinal);
        Assert.DoesNotContain("Color.FromRgb", region, StringComparison.Ordinal);
        Assert.DoesNotContain("Color.FromArgb", region, StringComparison.Ordinal);
        // 连注释里都不留十六进制值：注释里的魔数会被下一个人当成「设计值」抄走。
        Assert.DoesNotMatch("#[0-9A-Fa-f]{6}", region);

        // 三枚令牌 key 必须都出现，否则「没有字面量」可能只是因为整段被删了。
        Assert.Contains("Ariadne.EdgeControl", region, StringComparison.Ordinal);
        Assert.Contains("Ariadne.EdgeCommunication", region, StringComparison.Ordinal);
        Assert.Contains("Ariadne.EdgeData", region, StringComparison.Ordinal);
    }

    /// <summary>
    /// 走真实的橡皮筋绘制路径取色：设置引脚种类 → 调 `UpdateRubberBand`
    /// → 读 `RubberBandPath.Stroke` 的实际颜色。
    ///
    /// 刻意不直接反射调取色函数：那样断言的是「函数返回什么」，
    /// 而用户看到的是「Path 上最终挂了什么笔刷」。缺陷若出在赋值那一步
    /// （例如忘了把新取的笔刷写回 Stroke），只测函数照样全绿。
    /// </summary>
    private static Color DrawRubberBandColor(WorkspacePageView view, NodePortKind kind)
    {
        var type = typeof(WorkspacePageView);
        type.GetField("_edgeSourceKind", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(view, kind);
        type.GetField("_rubberBandStartCanvas", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(view, new Point(40, 40));
        type.GetMethod("UpdateRubberBand", BindingFlags.Instance | BindingFlags.NonPublic)!
            .Invoke(view, new object?[] { new Point(220, 160) });

        var path = view.FindControl<ShapePath>("RubberBandPath");
        Assert.NotNull(path);
        var stroke = path!.Stroke as ISolidColorBrush;
        Assert.NotNull(stroke);
        return stroke!.Color;
    }

    /// <summary>当前期望色 = 主题字典在指定变体下对该令牌的取值（不写死任何数）。</summary>
    private static Color ResolveTokenColor(WorkspacePageView view, string colorKey, ThemeVariant variant)
    {
        Assert.True(
            view.TryFindResource(colorKey, variant, out var resource),
            $"主题里找不到 {colorKey}（{variant}），期望值无从取得");

        return resource switch
        {
            Color color => color,
            ISolidColorBrush brush => brush.Color,
            _ => throw new Xunit.Sdk.XunitException($"{colorKey} 不是颜色资源：{resource?.GetType().Name}"),
        };
    }

    private static void SetVariant(ThemeVariant variant)
    {
        Assert.NotNull(Application.Current);
        Application.Current!.RequestedThemeVariant = variant;
    }

    private static async Task RunWithViewAsync(Func<WorkspacePageView, Task> body)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var viewModel = new WorkspacePageViewModel(
                DisplayNameService.LoadDefault(),
                DispatchProxy.Create<IAriadneBackendClient, SoftBackendProxy>());
            var view = new WorkspacePageView { DataContext = viewModel };
            var window = new Window { Width = 1200, Height = 800, Content = view };
            window.Show();
            await DrainAsync();

            try
            {
                await body(view);
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

    private static string ResolveViewSourcePath()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return Path.Combine(dir!, "Ariadne.Desktop", "Views", "WorkspacePageView.axaml.cs");
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
                return typeof(Task)
                    .GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }

            return targetMethod?.ReturnType.IsValueType == true
                ? Activator.CreateInstance(targetMethod.ReturnType)
                : null;
        }
    }
}
