using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// 模板市场页 ViewModel：搜索 + 标签 + 卡片网格 + 权限确认。
/// 本轮只承载视觉骨架文案，后端接线（search_templates / install_template 等）留待交互阶段。
public sealed class TemplateMarketPageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;

    public TemplateMarketPageViewModel(DisplayNameService displayNames)
    {
        _displayNames = displayNames;
    }

    public string Title => _displayNames.Text("ui.template.title");

    public string OnlineSearchText => _displayNames.Text("ui.template.online_search");

    public string SearchPlaceholder => _displayNames.Text("ui.template.search.placeholder");

    public string SearchText => _displayNames.Text("ui.common.search");

    public string EmptyText => _displayNames.Text("ui.template.empty");

    public string ImportText => _displayNames.Text("ui.common.import");

    public string PermissionText => _displayNames.Text("ui.template.permission");

    public string BackToTopText => _displayNames.Text("ui.common.back_to_top");

    public string LoadMoreText => _displayNames.Text("ui.common.load_more");
}
