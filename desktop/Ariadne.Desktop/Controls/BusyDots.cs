using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// U178-F：**全应用唯一的通用「正在进行」指示基元**。
///
/// 立项理由是一次全仓实测：`ProgressRing` / `IsIndeterminate` / `Skeleton` /
/// `Shimmer` 四个关键字命中数**全为 0**，唯一的 `ProgressBar` 是顶栏预算条
/// （那是「已花多少」的量表，不是加载指示）。于是任何耗时操作期间，
/// 用户能看到的唯一变化是一行文字换了内容——这正是「需要特别等」的观感来源。
///
/// **为什么自绘而不是用 FluentTheme 的 ProgressBar(IsIndeterminate)**：
/// 三条都是硬约束层面的：
/// (1) 颜色——Fluent 的进度条色来自它自己的 `SystemAccentColor` 一族，
///     不走 `Ariadne.*` token，深浅主题与个性化换色都跟不上；
/// (2) 形状——本项目的加载点位大多是「一行文字旁边」，需要的是紧凑圆环/圆点，
///     而不是一条撑满宽度的横条；
/// (3) ReduceMotion——Fluent 的不定态动画没有任何总闸可关，
///     而本项目把「减少动效」当作用户偏好在守（`MotionPreferences`）。
///
/// **动效形态：三点依次呼吸，不是旋转圆环。** 取舍理由：
/// 旋转需要每帧重算几何并整体重绘；三点只是三个圆的不透明度在变，
/// 每帧成本几乎为零。在这台机器上（U159 的教训：重挂载路径上的开销才是大头）
/// 加载指示常常和「正在重建的视图树」同时出现，指示器自己必须足够便宜。
///
/// 帧循环与门控完全照抄同目录 <see cref="SelectionFlowRing"/> 的既有范式：
/// 只在 <see cref="IsActive"/> 且未偏好减少动态时跑帧；两个条件在
/// 排帧与回调里**各查一次**（回调是下一帧才执行的，期间状态可能已变）。
/// </summary>
public sealed class BusyDots : Control
{
    /// <summary>是否正在进行。false 时完全不绘制、也不排帧。</summary>
    public static readonly StyledProperty<bool> IsActiveProperty =
        AvaloniaProperty.Register<BusyDots, bool>(nameof(IsActive));

    /// <summary>单点直径。默认 5px：与 caption 字号（11–12px）并排时不抢视线。</summary>
    public static readonly StyledProperty<double> DotDiameterProperty =
        AvaloniaProperty.Register<BusyDots, double>(nameof(DotDiameter), 5d);

    public bool IsActive
    {
        get => GetValue(IsActiveProperty);
        set => SetValue(IsActiveProperty, value);
    }

    public double DotDiameter
    {
        get => GetValue(DotDiameterProperty);
        set => SetValue(DotDiameterProperty, value);
    }

    /// 点数固定 3：这是「进行中」在各家产品里的通行手势，多一点少一点都只是噪声。
    private const int DotCount = 3;

    /// 点间距 = 直径的 0.9 倍。比 1.0 略紧，三点读作一个整体而不是三个独立圆点。
    private const double GapRatio = 0.9;

    /// <summary>
    /// 一轮呼吸的周期（秒）。1.1s 是**刻意落在 120–180ms 过渡尺度之外**的：
    /// 那个区间是「一次状态跃迁」的时长，而这里是持续循环的节律——
    /// 循环动画取那么快会变成焦躁的闪烁，反而让人觉得系统在挣扎。
    /// </summary>
    private const double CycleSeconds = 1.1;

    /// 相邻两点的相位差（弧度）。三点各差 1/3 个周期，形成「波沿着点列走」的读感。
    private static readonly double PhaseStep = Math.PI * 2 / DotCount;

    private bool _isAttached;
    private bool _frameQueued;
    private double _phase;
    private Color _dotColor;
    private IBrush? _brush;

    public BusyDots()
    {
        // 纯指示、不接受交互：挡住下面的按钮会让「加载中」变成「点不动」。
        IsHitTestVisible = false;
    }

    static BusyDots()
    {
        AffectsRender<BusyDots>(IsActiveProperty, DotDiameterProperty);
        AffectsMeasure<BusyDots>(DotDiameterProperty);
    }

    protected override Size MeasureOverride(Size availableSize)
    {
        var d = Math.Max(1, DotDiameter);
        // 宽 = 三个点 + 两个间隙；高 = 一个点。父级据此排版，不必各处写死尺寸。
        return new Size(d * DotCount + d * GapRatio * (DotCount - 1), d);
    }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == IsActiveProperty)
        {
            // 相位归零：下次开始时总是从同一个姿态起步，避免「接着上次的乱相位」。
            _phase = 0;
            QueueNextFrame();
        }
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

    private void OnMotionPreferenceChanged(object? sender, EventArgs e)
    {
        // 关掉动效后要再画一帧：否则停在某个中间相位上，三点亮度参差不齐。
        InvalidateVisual();
        QueueNextFrame();
    }

    private void OnThemeColorsChanged()
    {
        _brush = null;
        InvalidateVisual();
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
        topLevel.RequestAnimationFrame(_ =>
        {
            _frameQueued = false;
            // 再查一遍：这个回调是下一帧才跑的，期间可能已经 detach 或关掉了动效。
            if (!_isAttached || !IsActive || MotionPreferences.ReduceMotion)
            {
                return;
            }

            _phase += Math.PI * 2 / (CycleSeconds * 60);
            if (_phase > Math.PI * 2)
            {
                _phase -= Math.PI * 2;
            }

            InvalidateVisual();
            QueueNextFrame();
        });
    }

    public override void Render(DrawingContext context)
    {
        if (!IsActive)
        {
            return;
        }

        if (_brush is null)
        {
            // 走 token：次级文字色。加载指示是辅助信息，不该和正文同重，
            // 更不该用强调色（那是「可操作」的语义）。
            _dotColor = AppIconPainter.ResolveColor(
                "Ariadne.TextSecondary", Color.FromRgb(0x5B, 0x64, 0x69));
            _brush = new SolidColorBrush(_dotColor);
        }

        var d = Math.Max(1, DotDiameter);
        var r = d / 2;
        var step = d + d * GapRatio;
        var cy = Bounds.Height / 2;

        for (var i = 0; i < DotCount; i++)
        {
            // 减少动效时全部取中间亮度：形态还在（三点仍是「进行中」的读感），
            // 只是不动。整体隐藏会让偏好动效的用户失去唯一的进度反馈。
            var wave = MotionPreferences.ReduceMotion
                ? 0d
                : Math.Sin(_phase - i * PhaseStep);
            // 0.35–1.0：下限不取 0，否则点会完全消失、看起来是「少了一个点」而不是「暗下去」。
            var opacity = 0.675 + wave * 0.325;
            var cx = r + i * step;
            using (context.PushOpacity(opacity))
            {
                context.DrawEllipse(_brush, null, new Point(cx, cy), r, r);
            }
        }
    }
}
