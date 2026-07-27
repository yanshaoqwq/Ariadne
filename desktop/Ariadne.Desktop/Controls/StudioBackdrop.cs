using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.VisualTree;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// 欢迎页的生成式叙事轨迹。图形随主题着色，并遵守全局“减少动态效果”偏好。
/// </summary>
public sealed class StudioBackdrop : Control
{
    private static readonly Color SignalColor = Color.FromRgb(0xE8, 0x68, 0x4A);
    private bool _isAttached;
    private bool _frameQueued;
    private double _phase;

    // Render 跑在 RequestAnimationFrame 循环里，画笔只随主题变化，不必每帧重建。
    private Pen? _gridPen;
    private Pen? _axisPen;
    private Pen? _primaryPen;
    private Pen? _secondaryPen;
    private Pen? _markPen;
    private Pen? _markerRingPen;
    private IBrush? _markerBrush;

    public StudioBackdrop()
    {
        IsHitTestVisible = false;
        ClipToBounds = true;
    }

    public override void Render(DrawingContext context)
    {
        base.Render(context);

        var width = Bounds.Width;
        var height = Bounds.Height;
        if (width < 240 || height < 180)
        {
            return;
        }

        EnsurePens();

        DrawEditorialGrid(context, width, height);
        DrawStoryPaths(context, width, height);
        DrawRegistrationMarks(context, width, height);
    }

    /// <summary>
    /// 懒解析主题色并构建全部画笔；只在首帧和主题换色后各执行一次。
    /// </summary>
    private void EnsurePens()
    {
        if (_gridPen is not null)
        {
            return;
        }

        var accent = AppIconPainter.ResolveColor(
            "Ariadne.AccentPrimary",
            Color.FromRgb(0x35, 0x6F, 0x68));
        var ink = AppIconPainter.ResolveColor(
            "Ariadne.TextPrimary",
            Color.FromRgb(0x18, 0x20, 0x20));
        // 信号色同样从主题 Ariadne.Signal 解析，不再直接用硬编码常量作绘制色
        // （常量仅在资源缺失时兜底，与全代码库 ResolveColor 模式一致）。
        var signal = AppIconPainter.ResolveColor("Ariadne.Signal", SignalColor);

        _gridPen = new Pen(new SolidColorBrush(WithAlpha(ink, 0x0C)), 1);
        _axisPen = new Pen(new SolidColorBrush(WithAlpha(ink, 0x18)), 1);
        _primaryPen = new Pen(new SolidColorBrush(WithAlpha(accent, 0x44)), 1.4);
        _secondaryPen = new Pen(new SolidColorBrush(WithAlpha(ink, 0x20)), 1);
        _markPen = new Pen(new SolidColorBrush(WithAlpha(accent, 0x52)), 1);
        _markerRingPen = new Pen(new SolidColorBrush(WithAlpha(signal, 0x58)), 1);
        _markerBrush = new SolidColorBrush(signal);
    }

    protected override void OnAttachedToVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnAttachedToVisualTree(e);
        _isAttached = true;
        MotionPreferences.Changed += OnMotionPreferenceChanged;
        AppIconPainter.IconColorsChanged += OnThemeColorsChanged;
        QueueNextFrame();
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        _isAttached = false;
        MotionPreferences.Changed -= OnMotionPreferenceChanged;
        AppIconPainter.IconColorsChanged -= OnThemeColorsChanged;
        base.OnDetachedFromVisualTree(e);
    }

    private void DrawEditorialGrid(DrawingContext context, double width, double height)
    {
        var startX = Math.Max(0, width * 0.44);
        const double step = 56;

        for (var x = startX; x <= width; x += step)
        {
            context.DrawLine(_gridPen!, new Point(x, 0), new Point(x, height));
        }

        for (var y = height % step; y <= height; y += step)
        {
            context.DrawLine(_gridPen!, new Point(startX, y), new Point(width, y));
        }

        context.DrawLine(_axisPen!, new Point(startX, 0), new Point(startX, height));
    }

    private void DrawStoryPaths(DrawingContext context, double width, double height)
    {
        var breathe = MotionPreferences.ReduceMotion ? 0 : Math.Sin(_phase * 0.7) * 7;
        var primary = CreateCurve(
            new Point(width * 0.08, height * 0.78),
            new Point(width * 0.34, height * 0.64 + breathe),
            new Point(width * 0.53, height * 0.20 - breathe),
            new Point(width * 0.92, height * 0.27));
        var secondary = CreateCurve(
            new Point(width * 0.30, height * 0.92),
            new Point(width * 0.48, height * 0.72 - breathe),
            new Point(width * 0.72, height * 0.86 + breathe),
            new Point(width * 0.98, height * 0.58));

        context.DrawGeometry(null, _primaryPen!, primary);
        context.DrawGeometry(null, _secondaryPen!, secondary);

        var progress = MotionPreferences.ReduceMotion
            ? 0.58
            : 0.5 + Math.Sin(_phase * 0.34) * 0.22;
        var marker = CubicPoint(
            new Point(width * 0.08, height * 0.78),
            new Point(width * 0.34, height * 0.64 + breathe),
            new Point(width * 0.53, height * 0.20 - breathe),
            new Point(width * 0.92, height * 0.27),
            progress);
        context.DrawRectangle(_markerBrush!, null, new Rect(marker.X - 3, marker.Y - 3, 6, 6));
        context.DrawRectangle(
            null,
            _markerRingPen!,
            new Rect(marker.X - 9, marker.Y - 9, 18, 18));
    }

    private void DrawRegistrationMarks(DrawingContext context, double width, double height)
    {
        DrawCorner(context, _markPen!, new Point(width * 0.50, height * 0.13), 16);
        DrawCorner(context, _markPen!, new Point(width * 0.88, height * 0.78), 22);
        DrawCorner(context, _markPen!, new Point(width * 0.63, height * 0.53), 10);
    }

    private static void DrawCorner(DrawingContext context, Pen pen, Point origin, double size)
    {
        context.DrawLine(pen, origin, new Point(origin.X + size, origin.Y));
        context.DrawLine(pen, origin, new Point(origin.X, origin.Y + size));
    }

    private static StreamGeometry CreateCurve(Point start, Point c1, Point c2, Point end)
    {
        var geometry = new StreamGeometry();
        using var stream = geometry.Open();
        stream.BeginFigure(start, false);
        stream.CubicBezierTo(c1, c2, end);
        stream.EndFigure(false);
        return geometry;
    }

    private static Point CubicPoint(Point p0, Point p1, Point p2, Point p3, double t)
    {
        var inverse = 1 - t;
        var x = inverse * inverse * inverse * p0.X
            + 3 * inverse * inverse * t * p1.X
            + 3 * inverse * t * t * p2.X
            + t * t * t * p3.X;
        var y = inverse * inverse * inverse * p0.Y
            + 3 * inverse * inverse * t * p1.Y
            + 3 * inverse * t * t * p2.Y
            + t * t * t * p3.Y;
        return new Point(x, y);
    }

    private static Color WithAlpha(Color color, byte alpha) =>
        Color.FromArgb(alpha, color.R, color.G, color.B);

    private void QueueNextFrame()
    {
        if (!_isAttached || _frameQueued || MotionPreferences.ReduceMotion)
        {
            return;
        }

        var topLevel = TopLevel.GetTopLevel(this);
        if (topLevel is null)
        {
            return;
        }

        _frameQueued = true;
        topLevel.RequestAnimationFrame(timestamp =>
        {
            _frameQueued = false;
            if (!_isAttached || MotionPreferences.ReduceMotion)
            {
                return;
            }

            _phase = timestamp.TotalSeconds;
            InvalidateVisual();
            QueueNextFrame();
        });
    }

    private void OnMotionPreferenceChanged(object? sender, EventArgs e)
    {
        InvalidateVisual();
        QueueNextFrame();
    }

    private void OnThemeColorsChanged()
    {
        // 主题/个性化换色：丢弃旧画笔，下一帧按新 token 重建。
        _gridPen = null;
        _axisPen = null;
        _primaryPen = null;
        _secondaryPen = null;
        _markPen = null;
        _markerRingPen = null;
        _markerBrush = null;
        InvalidateVisual();
    }
}
