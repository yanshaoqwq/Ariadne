using System.Xml.Linq;
using Avalonia;
using Avalonia.Media;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U161：画布缩放后「渲染位置」必须与「命中/连线换算出的位置」一致。
///
/// **缺陷背景**：节点层与连线层挂了 `TransformGroup(Scale, Translate)` 但没设
/// `RenderTransformOrigin`。Avalonia 的默认值是 `50%,50%`（中心，本文件实测），
/// 而全代码库的屏幕↔逻辑换算一律用 `screen = logical × zoom + offset`
/// （`CanvasViewportSession.ToLogical` 的逆），该式**仅在原点为左上时成立**。
/// 于是缩放≠1 时渲染与命中相差 `(-(zoom-1)·层宽/2, -(zoom-1)·层高/2)`。
///
/// **判据为什么是「算出来的坐标」而不是「属性出现过」**：
/// `Assert.Contains("RenderTransformOrigin", axaml)` 在值被写成 `50%,50%`
/// 时照样绿——那正是缺陷本身的形状。所以这里读出**实际值**（读不到就按
/// Avalonia 默认的中心处理），用**真实 Avalonia 矩阵**复刻施加过程，
/// 再比对两条路径的结果。写错值会红，删掉属性也会红。
///
/// **诚实边界**：本文件不过 XAML 编译，也不实例化控件
/// （本机 Avalonia headless 会卡死，见 CLAUDE.md 记录）。它覆盖的是
/// 「那两层的变换原点与换算公式是否自洽」；`.axaml` 能否编译由
/// `dotnet build` 负责，两者互补。改 `.axaml` 后仍必须 build。
/// </summary>
public sealed class CanvasZoomTransformOriginTests
{
    private const string CanvasNamespace = "https://github.com/avaloniaui";
    private static readonly XNamespace Xaml = CanvasNamespace;
    private static readonly XNamespace XamlX = "http://schemas.microsoft.com/winfx/2006/xaml";

    /// <summary>
    /// 参与坐标换算的两层。**新增内容层时必须加进来**——
    /// 漏一层就是漏一个坐标系，症状与本缺陷相同。
    /// </summary>
    public static TheoryData<string> ContentLayerNames => new()
    {
        "NodesItemsControl",
        "EdgesItemsControl",
    };

    /// <summary>
    /// 缩放档位。**刻意不含 1.0**：错位量是 `-(zoom-1)·尺寸/2`，
    /// 在 zoom=1 时恰为零，缺陷存在时也会通过。
    /// 把它放进参数表会让这份覆盖变成装饰品。
    /// 取值覆盖 `CanvasViewportHelpers` 的 MinZoom(0.25)–MaxZoom(2.5) 两端。
    /// </summary>
    private static readonly double[] Zooms = { 0.25, 0.5, 1.5, 2.5 };

    [Theory]
    [MemberData(nameof(ContentLayerNames))]
    public void ContentLayer_DrawnPosition_MatchesHitTestConversionAtEveryZoom(string layerName)
    {
        var origin = ResolveRenderTransformOrigin(layerName);

        // 内容层尺寸取一个真实量级（ComputeNodeLayerSize 在 1200x800 视口下的量级）。
        // 尺寸必须非零：错位量与尺寸成正比，用 0 会让缺陷消失。
        const double layerWidth = 1200;
        const double layerHeight = 800;
        var logical = new Point(300, 200);

        foreach (var zoom in Zooms)
        {
            // 平移取非零值，确保测的不只是缩放那一半。
            const double offsetX = 40;
            const double offsetY = -25;

            var drawn = ComposeLayerMatrix(zoom, offsetX, offsetY, layerWidth, layerHeight, origin)
                .Transform(logical);

            // 代码其余各处（命中、框选、橡皮筋、点阵）共用的换算式。
            var expected = new Point(
                (logical.X * zoom) + offsetX,
                (logical.Y * zoom) + offsetY);

            Assert.Equal(expected.X, drawn.X, 6);
            Assert.Equal(expected.Y, drawn.Y, 6);
        }
    }

    /// <summary>
    /// 钉住「Avalonia 默认原点是中心」这个前提本身。
    ///
    /// 若某个版本把默认值改成左上，本用例会红——那时上面两条的
    /// 显式 `0%,0%` 就变成冗余，可以决定是否移除。**没有这条的话，
    /// 未来有人看到显式属性会以为是废话而删掉**，缺陷随即复现。
    /// </summary>
    [Fact]
    public void AvaloniaDefaultRenderTransformOrigin_IsCenter_WhichIsWhyExplicitTopLeftIsRequired()
    {
        var defaultOrigin = Visual.RenderTransformOriginProperty.GetDefaultValue(typeof(Visual));

        Assert.Equal(RelativePoint.Center, defaultOrigin);
        Assert.NotEqual(RelativePoint.TopLeft, defaultOrigin);
    }

    /// <summary>
    /// 反向证明本测试有鉴别力：**按默认的中心原点算，必定与换算式不符**。
    ///
    /// 这条是内建的变异测试。若哪天上面的主用例因为断言写错而变成空测，
    /// 这条仍会指出「中心原点是错的」这个事实仍然成立——
    /// 两条同时绿才说明覆盖真的在。
    /// </summary>
    [Fact]
    public void CenterOrigin_ProvablyBreaksTheConversion_SoTheTestHasDiscriminatingPower()
    {
        const double layerWidth = 1200;
        const double layerHeight = 800;
        const double zoom = 2.0;
        var logical = new Point(300, 200);

        var withCenter = ComposeLayerMatrix(zoom, 0, 0, layerWidth, layerHeight, RelativePoint.Center)
            .Transform(logical);
        var withTopLeft = ComposeLayerMatrix(zoom, 0, 0, layerWidth, layerHeight, RelativePoint.TopLeft)
            .Transform(logical);
        var expected = new Point(logical.X * zoom, logical.Y * zoom);

        // 左上原点 == 换算式。
        Assert.Equal(expected.X, withTopLeft.X, 6);
        Assert.Equal(expected.Y, withTopLeft.Y, 6);

        // 中心原点 != 换算式，且差值等于解析式 -(zoom-1)·尺寸/2。
        Assert.Equal(-(zoom - 1) * layerWidth / 2, withCenter.X - expected.X, 6);
        Assert.Equal(-(zoom - 1) * layerHeight / 2, withCenter.Y - expected.Y, 6);
    }

    // ════════════════════════════════════════════════════════
    // 工具
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 复刻 Avalonia 施加 `RenderTransform` 的实际做法：
    /// 平移到 origin → 应用变换 → 平移回去。
    ///
    /// 变换本体与 `WorkspacePageView.axaml` 里那两层同构：
    /// `TransformGroup(ScaleTransform(zoom), TranslateTransform(offset))`。
    /// 用真实的 `TransformGroup` / `Matrix` 而非自己手推矩阵——
    /// 手推等于把被测对象的实现抄一遍，抄错了两边一起错。
    /// </summary>
    private static Matrix ComposeLayerMatrix(
        double zoom, double offsetX, double offsetY,
        double layerWidth, double layerHeight, RelativePoint origin)
    {
        var group = new TransformGroup();
        group.Children.Add(new ScaleTransform(zoom, zoom));
        group.Children.Add(new TranslateTransform(offsetX, offsetY));

        var pixels = origin.ToPixels(new Size(layerWidth, layerHeight));
        return Matrix.CreateTranslation(-pixels.X, -pixels.Y)
               * group.Value
               * Matrix.CreateTranslation(pixels.X, pixels.Y);
    }

    /// <summary>
    /// 从 shipped `.axaml` 读出某层的 `RenderTransformOrigin` 实际值。
    ///
    /// **读不到就返回 Avalonia 默认值**（中心），而不是抛异常或假定左上——
    /// 「属性缺失」正是缺陷的原始形态，必须让它走进和「写错值」相同的失败路径。
    ///
    /// 用 XML 解析而非正则：XAML 是 XML，正则会被属性顺序、换行、
    /// 注释里的同名字符串绊倒。
    /// </summary>
    private static RelativePoint ResolveRenderTransformOrigin(string layerName)
    {
        var path = ResolveDesktopSource("Views", "WorkspacePageView.axaml");
        var element = XDocument.Load(path)
            .Descendants(Xaml + "ItemsControl")
            .SingleOrDefault(node => (string?)node.Attribute(XamlX + "Name") == layerName);

        Assert.NotNull(element);

        // 前提校验：这一层确实挂了缩放变换。若哪天变换被移走，
        // 本用例就不再测的是真实路径了——必须当场失败而不是继续算一个无关的矩阵。
        var hasScale = element!
            .Descendants(Xaml + "ScaleTransform")
            .Any(scale => (string?)scale.Attribute("ScaleX") == "{Binding CanvasZoom}");
        Assert.True(
            hasScale,
            $"{layerName} 上没有绑定 CanvasZoom 的 ScaleTransform——"
            + "本用例假定的变换路径已不存在，判据失效，需要同步更新测试");

        var raw = (string?)element.Attribute("RenderTransformOrigin");
        return string.IsNullOrWhiteSpace(raw)
            ? Visual.RenderTransformOriginProperty.GetDefaultValue(typeof(Visual))
            : RelativePoint.Parse(raw);
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 12 && walk is not null; i++)
        {
            var candidate = Path.Combine(
                new[] { walk.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (Directory.Exists(candidate) || File.Exists(candidate))
            {
                return candidate;
            }

            walk = walk.Parent;
        }

        throw new FileNotFoundException(string.Join('/', parts));
    }
}
