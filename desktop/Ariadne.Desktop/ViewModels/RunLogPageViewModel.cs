using System.Collections.ObjectModel;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// <summary>Mutually exclusive list load states (U72 / 00A).</summary>
public enum PageLoadState
{
    IdleNeedProject,
    Loading,
    Error,
    Empty,
    Content,
}

public sealed class RunLogPageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private string _searchQuery = string.Empty;
    private string _selectedLevel = string.Empty;
    private string _statusText = string.Empty;
    private PageLoadState _loadState = PageLoadState.Loading;
    private string _errorText = string.Empty;
    private int _loadGeneration;

    public RunLogPageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        Logs = new ObservableCollection<RunLogItemViewModel>();
        LevelOptions = new ObservableCollection<RunLogLevelOption>
        {
            new(string.Empty, displayNames.Text("ui.run_log.all_levels")),
            new("info", displayNames.Text("ui.level.info")),
            new("warning", displayNames.Text("ui.level.warning")),
            new("error", displayNames.Text("ui.level.error")),
        };
        SearchCommand = new RelayCommand(() => _ = RefreshAsync());
        RefreshCommand = new RelayCommand(() => _ = RefreshAsync());
        MarkReadCommand = new RelayCommand(() => _ = MarkReadAsync());
        _ = RefreshAsync();
    }

    public string Title => _displayNames.Text("ui.run_log.title");

    public string SearchPlaceholder => _displayNames.Text("ui.run_log.search.placeholder");

    public string AllLevelsText => _displayNames.Text("ui.run_log.all_levels");

    public string RefreshText => _displayNames.Text("ui.common.refresh");

    public string SearchText => _displayNames.Text("ui.common.search");

    public string MarkReadText => _displayNames.Text("ui.run_log.mark_read");

    public string EmptyText => _displayNames.Text("ui.run_log.empty");
    public string EmptyTitle => _loadState == PageLoadState.IdleNeedProject
        ? _displayNames.Text("ui.empty.need_project.title")
        : _displayNames.Text("ui.empty.run_log.title");
    public string EmptyHint => _loadState == PageLoadState.IdleNeedProject
        ? _displayNames.Text("ui.empty.need_project.hint")
        : _displayNames.Text("ui.empty.run_log.hint");
    public string ErrorTitle => _displayNames.Text("ui.run_log.error.title");
    public string LoadingText => _displayNames.Text("ui.common.loading");

    public bool HasLogs => _loadState == PageLoadState.Content && Logs.Count > 0;
    public bool IsLogListEmpty => _loadState == PageLoadState.Empty || _loadState == PageLoadState.IdleNeedProject;
    public bool IsLoading => _loadState == PageLoadState.Loading;
    public bool IsError => _loadState == PageLoadState.Error;
    public bool ShowEmpty => IsLogListEmpty && !IsLoading && !IsError;
    public bool ShowContent => HasLogs;

    public string LevelInfoText => _displayNames.Text("ui.level.info");

    public string LevelWarningText => _displayNames.Text("ui.level.warning");

    public string LevelErrorText => _displayNames.Text("ui.level.error");

    public ObservableCollection<RunLogItemViewModel> Logs { get; }

    public ObservableCollection<RunLogLevelOption> LevelOptions { get; }

    public RelayCommand SearchCommand { get; }

    public RelayCommand RefreshCommand { get; }

    public RelayCommand MarkReadCommand { get; }

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

    public string ErrorText
    {
        get => _errorText;
        private set => SetProperty(ref _errorText, value);
    }

    public PageLoadState LoadState
    {
        get => _loadState;
        private set
        {
            if (SetProperty(ref _loadState, value))
            {
                OnPropertyChanged(nameof(HasLogs));
                OnPropertyChanged(nameof(IsLogListEmpty));
                OnPropertyChanged(nameof(IsLoading));
                OnPropertyChanged(nameof(IsError));
                OnPropertyChanged(nameof(ShowEmpty));
                OnPropertyChanged(nameof(ShowContent));
                OnPropertyChanged(nameof(EmptyTitle));
                OnPropertyChanged(nameof(EmptyHint));
            }
        }
    }

    private async Task RefreshAsync()
    {
        var gen = ++_loadGeneration;
        if (!_backend.HasProjectRoot)
        {
            Logs.Clear();
            ErrorText = string.Empty;
            StatusText = string.Empty;
            LoadState = PageLoadState.IdleNeedProject;
            return;
        }

        LoadState = PageLoadState.Loading;
        StatusText = LoadingText;
        try
        {
            var level = string.IsNullOrWhiteSpace(SelectedLevel) ? null : SelectedLevel;
            var logs = await _backend.QueryRunLogsAsync(level, SearchQuery).ConfigureAwait(true);
            if (gen != _loadGeneration)
            {
                return;
            }

            Logs.Clear();
            foreach (var log in logs)
            {
                Logs.Add(new RunLogItemViewModel(log));
            }
            ErrorText = string.Empty;
            if (Logs.Count == 0)
            {
                LoadState = PageLoadState.Empty;
                StatusText = EmptyText;
            }
            else
            {
                LoadState = PageLoadState.Content;
                StatusText = $"{Logs.Count}";
            }
        }
        catch (Exception ex)
        {
            if (gen != _loadGeneration)
            {
                return;
            }

            // U72: keep previous content when possible; never demote errors to Empty.
            ErrorText = UserFacingError.Format(ex, _displayNames);
            StatusText = ErrorText;
            LoadState = PageLoadState.Error;
            // Do not Logs.Clear() — preserve last good snapshot for diagnosis.
        }
    }

    private async Task MarkReadAsync()
    {
        try
        {
            await _backend.MarkRunLogsReadAsync().ConfigureAwait(true);
            await RefreshAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
            ErrorText = StatusText;
            LoadState = PageLoadState.Error;
        }
    }
}

public sealed record RunLogLevelOption(string Value, string Label);

public sealed class RunLogItemViewModel
{
    public RunLogItemViewModel(UiRunLogEntry entry)
    {
        LogId = entry.LogId;
        TimestampMs = entry.TimestampMs;
        Kind = entry.Kind;
        Level = entry.Level;
        Message = entry.Message;
        TimestampText = FormatTimestamp(entry.TimestampMs);
        var level = entry.Level.ToLowerInvariant();
        LevelBrushKey = level switch
        {
            "error" => "error",
            "warning" or "warn" => "warning",
            _ => "info",
        };
        IsError = LevelBrushKey == "error";
        IsWarning = LevelBrushKey == "warning";
        IsInfo = LevelBrushKey == "info";
    }

    public string LogId { get; }
    public long TimestampMs { get; }
    public string Kind { get; }
    public string Level { get; }
    public string Message { get; }
    public string TimestampText { get; }
    public string LevelBrushKey { get; }
    public bool IsError { get; }
    public bool IsWarning { get; }
    public bool IsInfo { get; }

    private static string FormatTimestamp(long ms)
    {
        try
        {
            return DateTimeOffset.FromUnixTimeMilliseconds(ms).LocalDateTime.ToString("yyyy-MM-dd HH:mm:ss");
        }
        catch
        {
            return ms.ToString();
        }
    }
}
