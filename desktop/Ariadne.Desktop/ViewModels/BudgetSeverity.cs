namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 顶栏预算条的紧急度分档（U194-E）。
///
/// 缺陷版本里预算条**一路青绿到 100%**：余量剩 $0.50 时和剩 $50 时长得一样，
/// 作者只有在「点运行被后端拒绝」那一刻才知道额度到头了——界面从「一切正常」
/// 直接跳到「被拒」，中间没有任何预告。
///
/// 分档只描述**语义**，具体色由主题的 `ProgressBar.budget-*` /
/// `TextBlock.budget-*` 样式给（颜色一律走 `Ariadne.*` 令牌，零魔法值）。
/// </summary>
public enum BudgetSeverity
{
    /// <summary>余量充足，或未设限额（日预算 0 = 不设上限，U112）。</summary>
    Normal,

    /// <summary>接近上限，提前预告。</summary>
    Warning,

    /// <summary>余量低于一次调用的量级，随时会被后端预算门拒绝。</summary>
    Error,
}
