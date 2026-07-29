using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media;
using Avalonia.VisualTree;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// 图标悬停自转行为：指针进入按钮后，按钮内的图标绕自身中心
/// 「先快速转一圈 → 短暂停顿 → 之后持续慢速匀转」，指针离开立即复位。
/// 用 RequestAnimationFrame 逐帧驱动（Avalonia 的样式关键帧动画不支持
/// RenderTransform，且主题禁止无限时长动画声明）。
/// 尊重全局“减少动态效果”偏好：开启时完全不转。
/// </summary>
public static class HoverSpin
{
    /// <summary>挂在按钮上启用悬停自转（通过样式 Setter 附加，不逐个写在标记里）。</summary>
    public static readonly AttachedProperty<bool> EnableProperty =
        AvaloniaProperty.RegisterAttached<Control, bool>("Enable", typeof(HoverSpin));

    // 节奏参数：0~FastEnd 快转一整圈（CubicEaseOut），到 PauseEnd 停住，之后匀速慢转。
    private const double FastEndSeconds = 0.5;
    private const double PauseEndSeconds = 0.85;
    private const double SlowDegreesPerSecond = 180;

    public static bool GetEnable(Control control) => control.GetValue(EnableProperty);

    public static void SetEnable(Control control, bool value) => control.SetValue(EnableProperty, value);

    static HoverSpin()
    {
        EnableProperty.Changed.AddClassHandler<Control>((control, args) =>
        {
            if (args.NewValue is true)
            {
                control.PointerEntered += OnPointerEntered;
                control.PointerExited += OnPointerExited;
            }
            else
            {
                control.PointerEntered -= OnPointerEntered;
                control.PointerExited -= OnPointerExited;
                StopSpin(control);
            }
        });
    }

    /// <summary>每个按钮的进行中自转状态：目标图标 + 世代号（离开即失效）。</summary>
    private sealed class SpinState
    {
        public Visual? Target;
        public int Generation;
        public bool Running;
    }

    private static readonly AttachedProperty<SpinState?> StateProperty =
        AvaloniaProperty.RegisterAttached<Control, SpinState?>("State", typeof(HoverSpin));

    private static void OnPointerEntered(object? sender, PointerEventArgs e)
    {
        if (sender is not Control control || MotionPreferences.ReduceMotion)
        {
            return;
        }

        var state = control.GetValue(StateProperty);
        if (state is null)
        {
            state = new SpinState();
            control.SetValue(StateProperty, state);
        }

        // 目标 = 按钮内第一个图标 Path（保存按钮里角标 Ellipse 不跟转，更稳）。
        state.Target ??= control.GetVisualDescendants().OfType<Avalonia.Controls.Shapes.Path>()
            .FirstOrDefault(path => path.Classes.Contains("icon"));
        if (state.Target is null || state.Running)
        {
            return;
        }

        state.Generation++;
        state.Running = true;
        var generation = state.Generation;
        var topLevel = TopLevel.GetTopLevel(control);
        if (topLevel is null)
        {
            state.Running = false;
            return;
        }

        var startedAt = TimeSpan.MinValue;
        void Frame(TimeSpan timestamp)
        {
            if (state.Generation != generation || state.Target is null)
            {
                return; // 指针已离开：停帧，复位已在 StopSpin 做过
            }

            if (startedAt == TimeSpan.MinValue)
            {
                startedAt = timestamp;
            }

            var t = (timestamp - startedAt).TotalSeconds;
            double angle;
            if (t < FastEndSeconds)
            {
                // 快转一圈：CubicEaseOut，起势迅猛后收
                var p = t / FastEndSeconds;
                angle = 360 * (1 - Math.Pow(1 - p, 3));
            }
            else if (t < PauseEndSeconds)
            {
                angle = 360; // 停顿
            }
            else
            {
                angle = 360 + (t - PauseEndSeconds) * SlowDegreesPerSecond; // 持续慢转
            }

            state.Target.RenderTransform = new RotateTransform(angle % 360);
            topLevel.RequestAnimationFrame(Frame);
        }

        topLevel.RequestAnimationFrame(Frame);
    }

    private static void OnPointerExited(object? sender, PointerEventArgs e)
    {
        if (sender is Control control)
        {
            StopSpin(control);
        }
    }

    private static void StopSpin(Control control)
    {
        var state = control.GetValue(StateProperty);
        if (state is null)
        {
            return;
        }

        state.Generation++; // 使进行中的帧循环失效
        state.Running = false;
        if (state.Target is not null)
        {
            state.Target.RenderTransform = null; // 立即复位到 0°
        }
    }
}
