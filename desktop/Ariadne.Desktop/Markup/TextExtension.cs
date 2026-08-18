using Avalonia.Markup.Xaml;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.Markup;

/// <summary>
/// 在 XAML 里直接取一条本地化文案：<c>{loc:Text ui.workspace.run}</c>。
///
/// **为什么需要它（U159 路 C-1）**：节点卡片模板里有 18 处
/// <c>{Binding $parent[UserControl].DataContext.XxxTip}</c> 绑的是纯静态文案
/// （10 个 <c>ToolTip.Tip</c> + 8 个 <c>AutomationProperties.Name</c>）。
/// 祖先绑定的实现（<c>LogicalAncestorElementNode</c> → <c>ControlLocator.Track</c>）
/// 每个都要建一个 <c>ControlTracker</c>、订阅 <c>AttachedToLogicalTree</c> +
/// <c>DetachedFromLogicalTree</c> 两个事件，并在 <c>Update()</c> 里跑一次
/// <c>GetLogicalAncestors().Where(...).ElementAtOrDefault(level)</c>——
/// **10 层深的 LINQ 全祖先链遍历**。
///
/// 关键在于它订阅的正是 attach/detach ⇒ **这笔成本恰好落在「切回画布页时
/// 重新挂载」这条路径上**，也就是 U159 那 5~7 秒的来源
/// （实测基线：每节点边际 72.4ms，60 节点多付 4.3 秒）。
///
/// **为什么可以安全地不响应语言切换**：`WorkspacePageViewModel` 全文
/// **没有订阅 `DisplayNameService.LanguageChanged`**，也没有 `RefreshLocalizedText`
/// （MainWindow / Welcome / ProjectAutomationState 三处才有）。
/// 也就是说这些属性的 getter 虽然每次都读服务，但**没有任何 `OnPropertyChanged`
/// 会通知它们** ⇒ 切语言后画布页文案本来就不刷新。
/// 脱绑不损失任何现有行为——它只是不再为一个**从不发生的通知**付订阅代价。
///
/// ⚠️ 如果将来给画布页补上语言切换刷新，**这个扩展要连带改**
/// （改成返回一个轻量绑定，或让页面在语言变更时重建）。
/// 那时的正确做法**不是**改回 `$parent[UserControl]` 祖先绑定——
/// 直接绑到节点自身的 VM 属性即可，节点 VM 本来就在每个卡片的 DataContext 上。
/// </summary>
public sealed class TextExtension : MarkupExtension
{
    public TextExtension()
    {
    }

    public TextExtension(string key)
    {
        Key = key;
    }

    /// <summary>display_name.json 里的 key。</summary>
    public string Key { get; set; } = string.Empty;

    public override object ProvideValue(IServiceProvider serviceProvider)
    {
        // 走 DisplayNameService.Current 而不是注入实例：markup extension 没有
        // DataContext，而 Current 在 App 启动时就 Initialize 过了。
        // 缺 key 时 Text() 返回 "[key]"，与绑定路径的行为一致，便于自查。
        return DisplayNameService.Current.Text(Key);
    }
}
