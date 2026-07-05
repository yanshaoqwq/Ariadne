using System.Collections.ObjectModel;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// 运行日志页 ViewModel：可检索事件流。
/// 本轮只承载视觉骨架文案，后端接线（query_run_logs / mark_run_logs_read）留待交互阶段。
public sealed class RunLogPageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private string _searchQuery = string.Empty;
    private string _selectedLevel = string.Empty;
    private string _statusText = string.Empty;

    public RunLogPageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        Logs = new ObservableCollection<UiRunLogEntry>();
        RefreshCommand = new RelayCommand(() => _ = RefreshAsync());
        _ = RefreshAsync();
    }

    public string Title => _displayNames.Text("ui.run_log.title");

    public string SearchPlaceholder => _displayNames.Text("ui.run_log.search.placeholder");

    public string AllLevelsText => _displayNames.Text("ui.run_log.all_levels");

    public string RefreshText => _displayNames.Text("ui.common.refresh");

    public string EmptyText => _displayNames.Text("ui.run_log.empty");

    public string LevelInfoText => _displayNames.Text("ui.level.info");

    public string LevelWarningText => _displayNames.Text("ui.level.warning");

    public string LevelErrorText => _displayNames.Text("ui.level.error");

    public ObservableCollection<UiRunLogEntry> Logs { get; }

    public RelayCommand RefreshCommand { get; }

    public string SearchQuery
    {
        get => _searchQuery;
        set => SetProperty(ref _searchQuery, value);
    }

    public string SelectedLevel
    {
        get => _selectedLevel;
        set => SetProperty(ref _selectedLevel, value);
    }

    public string StatusText
    {
        get => _statusText;
        set => SetProperty(ref _statusText, value);
    }

    private async Task RefreshAsync()
    {
        try
        {
            var level = SelectedLevel switch
            {
                "info" => "info",
                "warning" => "warning",
                "error" => "error",
                _ => null,
            };
            var logs = await _backend.QueryRunLogsAsync(level, SearchQuery).ConfigureAwait(true);
            Logs.Clear();
            foreach (var log in logs)
            {
                Logs.Add(log);
            }
            StatusText = Logs.Count == 0 ? EmptyText : $"{Logs.Count}";
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }
}
