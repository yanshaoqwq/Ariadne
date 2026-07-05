using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// 作品页 ViewModel：章节树 + 阅读/修改器 + 右栏（导航/项目 AI）。
/// 本轮只承载视觉骨架文案，后端接线（get_works_tree / get_document_content 等）留待交互阶段。
public sealed class WorksPageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private bool _isRightPanelOpen = true;
    private bool _isNavTreeTab = true;

    public WorksPageViewModel(DisplayNameService displayNames)
    {
        _displayNames = displayNames;
        ToggleRightPanelCommand = new RelayCommand(() => IsRightPanelOpen = !IsRightPanelOpen);
        ShowNavTreeCommand = new RelayCommand(() => IsNavTreeTab = true);
        ShowProjectAiCommand = new RelayCommand(() => IsNavTreeTab = false);
    }

    public string ToggleRightPanelText => _displayNames.Text("ui.action.toggle_right_panel");

    /// 右侧栏开合状态；收起后由悬浮左向箭头重新展开。
    public bool IsRightPanelOpen
    {
        get => _isRightPanelOpen;
        set => SetProperty(ref _isRightPanelOpen, value);
    }

    public RelayCommand ToggleRightPanelCommand { get; }

    /// 右栏标签：true=导航树（含章节树/大纲），false=项目 AI。
    public bool IsNavTreeTab
    {
        get => _isNavTreeTab;
        set
        {
            if (SetProperty(ref _isNavTreeTab, value))
            {
                OnPropertyChanged(nameof(IsProjectAiTab));
            }
        }
    }

    public bool IsProjectAiTab => !_isNavTreeTab;

    public RelayCommand ShowNavTreeCommand { get; }

    public RelayCommand ShowProjectAiCommand { get; }

    public string SidebarTitle => _displayNames.Text("ui.works.sidebar.title");

    public string ImportText => _displayNames.Text("ui.works.import_manuscript");

    public string ExportText => _displayNames.Text("ui.works.export_combined");

    public string ReadModeText => _displayNames.Text("ui.works.read_mode");

    public string EditModeText => _displayNames.Text("ui.works.edit_mode");

    public string SaveText => _displayNames.Text("ui.common.save");

    public string OutlineText => _displayNames.Text("ui.works.outline");

    public string NavTreeText => _displayNames.Text("ui.works.nav_tree");

    public string ProjectAiText => _displayNames.Text("ui.works.project_ai");

    public string NoDocumentText => _displayNames.Text("ui.works.no_document_selected");

    public string EmptyIndexText => _displayNames.Text("ui.works.empty_index");

    public string QuickAiHint => _displayNames.Text("ui.works.quick_ai_hint");

    public string ProjectAiPlaceholder => _displayNames.Text("ui.works.project_ai.placeholder");
}
