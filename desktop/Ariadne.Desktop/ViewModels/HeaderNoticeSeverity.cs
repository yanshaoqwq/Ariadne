namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 顶栏通知的紧急度分档（U194-B）。
///
/// ## 原缺陷
///
/// 三类紧急度完全不同的消息写进**同一个** `NotificationText`：
/// 「版本 0.x」（纯信息）、初始化失败（要决策）、「批量保存第 3 页失败」（**数据风险**）。
/// 同一位置、同一 `subtle` 灰、同一 260px 截断、旁边一个 Fill 写死
/// `Ariadne.TextSubtle` 的 `Ellipse`（恒定灰，不随严重度变）
/// ⇒ 作者分不出哪条要立刻处理，且**后写的静默覆盖先写的**：
/// 「保存失败」被随后一条「版本号」顶掉，那条消息就此消失。
///
/// ## 为什么分档而不是直接做 toast
///
/// 报告（U194-B）建议的终点是**分渠道**（信息类留顶栏、需决策的走更显眼处），
/// 但分级是它的前置：没有 severity，任何渠道路由都无从判断该走哪条。
/// 且分级不新增控件、不抢焦点，与 U194-D 留档的「后台事件不弹窗是健康的」不冲突。
///
/// ## 排序有语义
///
/// 数值递增 = 紧急度递增，`NotifyHeader` 的覆盖判定直接比这个值
/// （见 `MainWindowViewModel.NotifyHeader`）。往中间插档要同时复核那处比较。
/// </summary>
public enum HeaderNoticeSeverity
{
    /// <summary>纯信息，看不看都不影响作者的下一步（版本号、维护阶段）。</summary>
    Info,

    /// <summary>要作者知道、但没有数据风险（离开项目失败、外链打不开）。</summary>
    Warning,

    /// <summary>有数据风险或功能已不可用（正文未落盘、后端起不来）。</summary>
    Error,
}
