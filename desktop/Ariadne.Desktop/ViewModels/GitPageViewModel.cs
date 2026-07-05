using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// Git 页 ViewModel：树形版本历史 + 存档详情栏。
/// 本轮只承载视觉骨架文案，后端接线（get_git_branch_graph / create_checkpoint 等）留待交互阶段。
public sealed class GitPageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private bool _isRightPanelOpen = true;

    public GitPageViewModel(DisplayNameService displayNames)
    {
        _displayNames = displayNames;
        ToggleRightPanelCommand = new RelayCommand(() => IsRightPanelOpen = !IsRightPanelOpen);
    }

    public string ToggleRightPanelText => _displayNames.Text("ui.action.toggle_right_panel");

    /// 右侧栏开合状态；收起后由悬浮左向箭头重新展开。
    public bool IsRightPanelOpen
    {
        get => _isRightPanelOpen;
        set => SetProperty(ref _isRightPanelOpen, value);
    }

    public RelayCommand ToggleRightPanelCommand { get; }

    public string Title => _displayNames.Text("ui.git.title");

    public string Description => _displayNames.Text("ui.git.desc");

    public string RefreshText => _displayNames.Text("ui.common.refresh");

    public string CheckpointPlaceholder => _displayNames.Text("ui.git.checkpoint.placeholder");

    public string CreateCheckpointText => _displayNames.Text("ui.git.create_checkpoint");

    public string BranchGraphText => _displayNames.Text("ui.git.branch_graph");

    public string DetailsText => _displayNames.Text("ui.git.details");

    public string NoSelectionText => _displayNames.Text("ui.git.no_selection");

    public string EmptyText => _displayNames.Text("ui.git.empty");

    public string RestoreBranchNameText => _displayNames.Text("ui.git.restore_branch_name");

    public string RestoreNewBranchText => _displayNames.Text("ui.git.restore_new_branch");

    public string SummaryLabel => _displayNames.Text("ui.git.summary");

    public string AuthorLabel => _displayNames.Text("ui.git.author");

    public string TimeLabel => _displayNames.Text("ui.git.time");
}
