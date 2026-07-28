using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// 节点画布的低对比点阵背景。点随画布缩放/平移联动，密度随缩放自适应分档，
/// 为节点排布提供空间参照（Blueprint/Figma 式编辑器质感），但不喧宾夺主。
/// 取色走 <c>Ariadne.CanvasGrid</c> 主题 token，随亮/暗与个性化主题实时切换；
/// 主题切换通过 <see cref="AppIconPainter.IconColorsChanged"/> 触发重解析。
/// </summary>
public sealed class CanvasGridBackground : Control
{
    // 画布逻辑网格基准间距（设备无关像素）。自适应分档围绕它上下翻档。
    private const double BaseSpacing = 26.0;

    // 屏幕上点间距的可读区间（dip）：小于下限则间距翻倍，大于上限则减半，
    // 保证无论缩放到哪一级，点密度都落在“看得清又不密”的范围。
    private const double MinScreenSpacing = 20.0;
    private const double MaxScreenSpacing = 46.0;

    // 点半径（dip），常量屏幕尺寸：缩小时不糊、放大时不变成色块。
    private const double DotRadius = 1.15;

    private double _zoom = 1.0;
    private double _offsetX;
    private double _offsetY;
    private IBrush? _dotBrush;

    // 平铺点阵刷：整片背景一次画完，免去按点发出成千上万次 DrawEllipse。
    // 只随间距分档与主题换色重建。
    private TileBrush? _tileBrush;
    private double _tileSpacing;

    public CanvasGridBackground()
    {
        // 纯背景层：不拦截命中（画布指针事件要落到下面的节点/空白），并按宿主边界裁剪。
        IsHitTestVisible = false;
        ClipToBounds = true;
    }

    /// <summary>
    /// 写入当前画布视口（缩放 + 平移），有变化才触发重绘。
    /// 由工作区页面在平移/缩放统一漏斗（ApplyCanvasViewportState）里调用，保证与节点层严格同帧。
    /// </summary>
    public void SetViewport(double zoom, double offsetX, double offsetY)
    {
        if (Math.Abs(zoom - _zoom) < 1e-9
            && Math.Abs(offsetX - _offsetX) < 1e-9
            && Math.Abs(offsetY - _offsetY) < 1e-9)
        {
            return;
        }

        _zoom = Math.Max(0.05, zoom);
        _offsetX = offsetX;
        _offsetY = offsetY;
        InvalidateVisual();
    }

    public override void Render(DrawingContext context)
    {
        base.Render(context);

        var width = Bounds.Width;
        var height = Bounds.Height;
        if (width < 2 || height < 2)
        {
            return;
        }

        // 懒解析一次点刷，主题切换时置空后在下一帧重建（见 OnThemeColorsChanged）。
        _dotBrush ??= new SolidColorBrush(
            AppIconPainter.ResolveColor("Ariadne.CanvasGrid", Color.FromRgb(0xCC, 0xD5, 0xD2)));
        var brush = _dotBrush;

        // 自适应逻辑间距：让「逻辑间距 × 缩放」落在屏幕可读区间内。
        var logical = BaseSpacing;
        var screen = logical * _zoom;
        while (screen < MinScreenSpacing)
        {
            logical *= 2;
            screen = logical * _zoom;
        }

        while (screen > MaxScreenSpacing)
        {
            logical *= 0.5;
            screen = logical * _zoom;
        }

        // 点位于画布逻辑坐标 k·logical 处 → 屏幕坐标 k·screen + offset。
        // 首个可见点取 offset 对 screen 的正模，保证整片点阵随平移连续滚动、对齐逻辑原点。
        var startX = PositiveModulo(_offsetX, screen);
        var startY = PositiveModulo(_offsetY, screen);
        if (double.IsNaN(startX) || double.IsNaN(startY))
        {
            return;
        }

        if (_tileBrush is null || Math.Abs(_tileSpacing - screen) > 1e-9)
        {
            _tileBrush = CreateTileBrush(brush, screen);
            _tileSpacing = screen;
        }

        // 点画在 tile 正中心，故原点平移半格才能让点心落在 startX + k·screen 上；
        // 四周各多铺一格覆盖边缘，溢出部分由 ClipToBounds 裁掉。
        using (context.PushTransform(
                   Matrix.CreateTranslation(startX - screen / 2, startY - screen / 2)))
        {
            context.DrawRectangle(
                _tileBrush,
                null,
                new Rect(-screen, -screen, width + (screen * 3), height + (screen * 3)));
        }
    }

    /// <summary>
    /// 构造一个边长为 <paramref name="spacing"/>、中心含单个点的平铺刷。
    /// </summary>
    private static TileBrush CreateTileBrush(IBrush dotBrush, double spacing)
    {
        var center = spacing / 2;
        return new DrawingBrush(new GeometryDrawing
        {
            Brush = dotBrush,
            Geometry = new EllipseGeometry(new Rect(
                center - DotRadius,
                center - DotRadius,
                DotRadius * 2,
                DotRadius * 2)),
        })
        {
            TileMode = TileMode.Tile,
            Stretch = Stretch.None,
            SourceRect = new RelativeRect(0, 0, spacing, spacing, RelativeUnit.Absolute),
            DestinationRect = new RelativeRect(0, 0, spacing, spacing, RelativeUnit.Absolute),
        };
    }

    protected override void OnAttachedToVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnAttachedToVisualTree(e);
        AppIconPainter.IconColorsChanged += OnThemeColorsChanged;
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        AppIconPainter.IconColorsChanged -= OnThemeColorsChanged;
        base.OnDetachedFromVisualTree(e);
    }

    private void OnThemeColorsChanged()
    {
        // 主题/个性化换色：丢弃旧刷，下一帧按新 token 重建。
        _dotBrush = null;
        _tileBrush = null;
        InvalidateVisual();
    }

    private static double PositiveModulo(double value, double modulo)
    {
        var result = value % modulo;
        return result < 0 ? result + modulo : result;
    }
}
