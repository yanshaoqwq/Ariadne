using System.Collections.ObjectModel;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

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
        LevelOptions = new ObservableCollection<RunLogLevelOption>
        {
            new(string.Empty, displayNames.Text("ui.run_log.all_levels")),
            new("info", displayNames.Text("ui.level.info")),
            new("warning", displayNames.Text("ui.level.warning")),
            new("error", displayNames.Text("ui.level.error")),
        };
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

    public ObservableCollection<RunLogLevelOption> LevelOptions { get; }

    public RelayCommand RefreshCommand { get; }

    public string SearchQuery
    {
        get => _searchQuery;
        set => SetProperty(ref _searchQuery, value);
    }

    public string SelectedLevel
    {
        get => _selectedLevel;
        set
        {
            if (SetProperty(ref _selectedLevel, value))
            {
                _ = RefreshAsync();
            }
        }
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
            var level = string.IsNullOrWhiteSpace(SelectedLevel) ? null : SelectedLevel;
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

public sealed record RunLogLevelOption(string Value, string Label);
