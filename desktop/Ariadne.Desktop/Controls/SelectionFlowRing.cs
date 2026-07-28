using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// 节点选中环：一道贴着卡片外缘的圆角描边，上面叠加一圈「流动的」高亮。
/// - 常驻底环 = 主题强调色半透，保证选中态始终可读；
/// - 高亮 = 强调色衍生的亮色，沿环身旋转扫过，形成流动渐变（灵动来源）；
/// - 颜色全部从主题 token 解析（强调色 + 由它提亮的衍生色），零硬编码；
/// - 仅在选中(IsActive)且未偏好减少动态时跑帧循环；减少动态时退化为静态底环。
/// </summary>
public sealed class SelectionFlowRing : Control
{
    /// <summary>圆角半径（与节点卡片一致，内部再按描边外扩做同心）。</summary>
    public static readonly StyledProperty<double> CornerRadiusProperty =
        AvaloniaProperty.Register<SelectionFlowRing, double>(nameof(CornerRadius), 12d);

    /// <summary>是否处于选中态。只有选中才绘制并跑流动动画。</summary>
    public static readonly StyledProperty<bool> IsActiveProperty =
        AvaloniaProperty.Register<SelectionFlowRing, bool>(nameof(IsActive));

    public double CornerRadius
    {
        get => GetValue(CornerRadiusProperty);
        set => SetValue(CornerRadiusProperty, value);
    }

    public bool IsActive
    {
        get => GetValue(IsActiveProperty);
        set => SetValue(IsActiveProperty, value);
    }

    private const double StrokeWidth = 2.5;
    private bool _isAttached;
    private bool _frameQueued;
    private double _phase;

    // 画笔/画刷只随主题变化，平时复用；高亮刷每帧按相位重建（渐变方向在转）。
    private Pen? _basePen;
    private Color _accent;
    private Color _accentBright;

    public SelectionFlowRing()
    {
        IsHitTestVisible = false;
    }

    static SelectionFlowRing()
    {
        AffectsRender<SelectionFlowRing>(IsActiveProperty, CornerRadiusProperty);
    }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == IsActiveProperty)
        {
            // 选中态切换：开始/停止流动循环，并重绘。
            InvalidateVisual();
            QueueNextFrame();
        }
    }

    public override void Render(DrawingContext context)
    {
        base.Render(context);
        if (!IsActive)
        {
            return;
        }

        var width = Bounds.Width;
        var height = Bounds.Height;
        if (width < 8 || height < 8)
        {
            return;
        }

        EnsureColors();

        // 描边以控件外缘为中心：内缩半线宽，圆角按外扩做同心。
        var rect = new Rect(
            StrokeWidth / 2,
            StrokeWidth / 2,
            width - StrokeWidth,
            height - StrokeWidth);
        var radius = CornerRadius + StrokeWidth;
        var rounded = new RoundedRect(rect, radius);

        // 1) 常驻底环：强调色半透，选中态的稳定可读基底。
        context.DrawRectangle(null, _basePen, rounded);

        // 2) 流动高亮：旋转的线性渐变在环身上扫过一段亮带。
        //    减少动态时不画这层（底环已足够表达选中）。
        if (!MotionPreferences.ReduceMotion)
        {
            var angle = _phase * 1.6; // 转速（弧度/秒），舒缓不晃眼
            var dx = Math.Cos(angle);
            var dy = Math.Sin(angle);
            var highlight = new LinearGradientBrush
            {
                StartPoint = new RelativePoint(0.5 - dx * 0.62, 0.5 - dy * 0.62, RelativeUnit.Relative),
                EndPoint = new RelativePoint(0.5 + dx * 0.62, 0.5 + dy * 0.62, RelativeUnit.Relative),
            };
            var dim = Color.FromArgb(0x00, _accentBright.R, _accentBright.G, _accentBright.B);
            highlight.GradientStops.Add(new GradientStop(dim, 0));
            highlight.GradientStops.Add(new GradientStop(_accentBright, 0.5));
            highlight.GradientStops.Add(new GradientStop(dim, 1));
            var highlightPen = new Pen(highlight, StrokeWidth + 0.6);
            context.DrawRectangle(null, highlightPen, rounded);
        }
    }

    /// <summary>解析主题强调色并派生提亮色；缓存到底环画笔，主题换色时重建。</summary>
    private void EnsureColors()
    {
        if (_basePen is not null)
        {
            return;
        }

        _accent = AppIconPainter.ResolveColor("Ariadne.AccentPrimary", Color.FromRgb(0x2E, 0x72, 0x6B));
        _accentBright = Lighten(_accent, 0.55);
        _basePen = new Pen(new SolidColorBrush(Color.FromArgb(0x8C, _accent.R, _accent.G, _accent.B)), StrokeWidth);
    }

    /// <summary>把主题强调色向白提亮 amount（0-1），得到高亮衍生色（不引入新硬编码色）。</summary>
    private static Color Lighten(Color color, double amount)
    {
        byte Lerp(byte channel) => (byte)(channel + (255 - channel) * amount);
        return Color.FromRgb(Lerp(color.R), Lerp(color.G), Lerp(color.B));
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
        if (!_isAttached || !IsActive || _frameQueued || MotionPreferences.ReduceMotion)
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
            if (!_isAttached || !IsActive || MotionPreferences.ReduceMotion)
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
        // 主题换色：丢弃缓存画笔，下一帧按新 token 重建。
        _basePen = null;
        InvalidateVisual();
    }
}
