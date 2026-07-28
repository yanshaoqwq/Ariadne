using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// 节点库拖拽指示点：拖动中是一枚主题色的脉动圆点（半径做放大/缩小呼吸，不做上下位移），
/// 松手时按 <see cref="ExpandProgress"/> 从圆点「展开」成节点卡片的轮廓，再由真实节点接管。
/// - 颜色全部从主题强调色 token 解析并派生，零硬编码；
/// - 只在附加到可视树且未偏好减少动态时跑帧循环；减少动态时退化为静态圆点/静态轮廓。
/// </summary>
public sealed class DragPulseDot : Control
{
    /// <summary>圆点静止半径（脉动在此半径上下摆动）。</summary>
    public static readonly StyledProperty<double> RadiusProperty =
        AvaloniaProperty.Register<DragPulseDot, double>(nameof(Radius), 9d);

    /// <summary>落定展开进度：0=纯圆点，1=完全展开成卡片轮廓。由 code-behind 逐帧推进。</summary>
    public static readonly StyledProperty<double> ExpandProgressProperty =
        AvaloniaProperty.Register<DragPulseDot, double>(nameof(ExpandProgress));

    /// <summary>展开目标尺寸（节点卡片轮廓的宽高）。</summary>
    public static readonly StyledProperty<Size> ExpandedSizeProperty =
        AvaloniaProperty.Register<DragPulseDot, Size>(nameof(ExpandedSize), new Size(232, 96));

    public double Radius
    {
        get => GetValue(RadiusProperty);
        set => SetValue(RadiusProperty, value);
    }

    public double ExpandProgress
    {
        get => GetValue(ExpandProgressProperty);
        set => SetValue(ExpandProgressProperty, value);
    }

    public Size ExpandedSize
    {
        get => GetValue(ExpandedSizeProperty);
        set => SetValue(ExpandedSizeProperty, value);
    }

    private bool _isAttached;
    private bool _frameQueued;
    private double _phase;

    // 画笔/画刷只随主题变化；缓存后复用，主题换色时丢弃重建。
    private Color _accent;
    private IBrush? _dotBrush;
    private IBrush? _haloBrush;
    private Pen? _outlinePen;
    private IBrush? _outlineFill;

    public DragPulseDot()
    {
        IsHitTestVisible = false;
    }

    static DragPulseDot()
    {
        AffectsRender<DragPulseDot>(RadiusProperty, ExpandProgressProperty, ExpandedSizeProperty);
    }

    public override void Render(DrawingContext context)
    {
        base.Render(context);
        EnsureBrushes();

        var center = new Point(Bounds.Width / 2, Bounds.Height / 2);
        var progress = Math.Clamp(ExpandProgress, 0, 1);

        if (progress <= 0.001)
        {
            // 纯拖动态：脉动圆点 + 一圈更淡的光环，呼吸只改半径。
            var breath = MotionPreferences.ReduceMotion ? 0 : Math.Sin(_phase * 4.2);
            var radius = Radius * (1 + breath * 0.18);
            context.DrawEllipse(_haloBrush, null, center, radius * 2.1, radius * 2.1);
            context.DrawEllipse(_dotBrush, null, center, radius, radius);
            return;
        }

        // 落定展开：圆点在两个维度上分别插值到卡片宽高，圆角从「整圆」收到卡片圆角。
        var eased = 1 - Math.Pow(1 - progress, 3); // CubicEaseOut，与全局撬起动效同族
        var target = ExpandedSize;
        var width = Lerp(Radius * 2, target.Width, eased);
        var height = Lerp(Radius * 2, target.Height, eased);
        var corner = Lerp(Radius, 12, eased);
        var rect = new Rect(
            center.X - width / 2,
            center.Y - height / 2,
            width,
            height);
        context.DrawRectangle(_outlineFill, _outlinePen, new RoundedRect(rect, corner));
    }

    private static double Lerp(double from, double to, double t) => from + (to - from) * t;

    /// <summary>解析主题强调色并派生点/光环/轮廓用的半透变体；主题换色时重建。</summary>
    private void EnsureBrushes()
    {
        if (_dotBrush is not null)
        {
            return;
        }

        _accent = AppIconPainter.ResolveColor("Ariadne.AccentPrimary", Color.FromRgb(0x2E, 0x72, 0x6B));
        _dotBrush = new SolidColorBrush(_accent).ToImmutable();
        _haloBrush = new SolidColorBrush(Color.FromArgb(0x2E, _accent.R, _accent.G, _accent.B)).ToImmutable();
        _outlineFill = new SolidColorBrush(Color.FromArgb(0x1F, _accent.R, _accent.G, _accent.B)).ToImmutable();
        _outlinePen = new Pen(
            new SolidColorBrush(Color.FromArgb(0xB0, _accent.R, _accent.G, _accent.B)).ToImmutable(),
            2);
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
            // 只有还在「圆点」阶段才需要逐帧重绘呼吸；展开由外部推进 ExpandProgress。
            if (ExpandProgress <= 0.001)
            {
                InvalidateVisual();
            }

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
        _dotBrush = null;
        _haloBrush = null;
        _outlineFill = null;
        _outlinePen = null;
        InvalidateVisual();
    }
}
