using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// 运行日志页 ViewModel：可检索事件流。
/// 本轮只承载视觉骨架文案，后端接线（query_run_logs / mark_run_logs_read）留待交互阶段。
public sealed class RunLogPageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;

    public RunLogPageViewModel(DisplayNameService displayNames)
    {
        _displayNames = displayNames;
    }

    public string Title => _displayNames.Text("ui.run_log.title");

    public string SearchPlaceholder => _displayNames.Text("ui.run_log.search.placeholder");

    public string AllLevelsText => _displayNames.Text("ui.run_log.all_levels");

    public string RefreshText => _displayNames.Text("ui.common.refresh");

    public string EmptyText => _displayNames.Text("ui.run_log.empty");

    public string LevelInfoText => _displayNames.Text("ui.level.info");

    public string LevelWarningText => _displayNames.Text("ui.level.warning");

    public string LevelErrorText => _displayNames.Text("ui.level.error");
}
