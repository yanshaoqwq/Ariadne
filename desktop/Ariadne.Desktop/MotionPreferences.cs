namespace Ariadne.Desktop;

/// <summary>
/// 进程内统一的动态效果偏好。视觉层只订阅这一处，避免页面各自维护第二状态源。
/// </summary>
public static class MotionPreferences
{
    private static bool _reduceMotion;

    public static event EventHandler? Changed;

    public static bool ReduceMotion => _reduceMotion;

    public static void Apply(bool reduceMotion)
    {
        if (_reduceMotion == reduceMotion)
        {
            return;
        }

        _reduceMotion = reduceMotion;
        Changed?.Invoke(null, EventArgs.Empty);
    }
}
