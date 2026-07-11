using System.Collections.ObjectModel;
using Avalonia.Media;
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
    public string EmptyTitle => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.run_log.title")
        : _displayNames.Text("ui.empty.need_project.title");
    public string EmptyHint => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.run_log.hint")
        : _displayNames.Text("ui.empty.need_project.hint");
    public bool HasLogs => Logs.Count > 0;
    public bool IsLogListEmpty => Logs.Count == 0;

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

    private async Task RefreshAsync()
    {
        if (!_backend.HasProjectRoot)
        {
            Logs.Clear();
            StatusText = string.Empty;
            OnPropertyChanged(nameof(HasLogs));
            OnPropertyChanged(nameof(IsLogListEmpty));
            OnPropertyChanged(nameof(EmptyTitle));
            OnPropertyChanged(nameof(EmptyHint));
            return;
        }

        try
        {
            var level = string.IsNullOrWhiteSpace(SelectedLevel) ? null : SelectedLevel;
            var logs = await _backend.QueryRunLogsAsync(level, SearchQuery).ConfigureAwait(true);
            Logs.Clear();
            foreach (var log in logs)
            {
                Logs.Add(new RunLogItemViewModel(log));
            }
            StatusText = Logs.Count == 0 ? EmptyText : $"{Logs.Count}";
            OnPropertyChanged(nameof(HasLogs));
            OnPropertyChanged(nameof(IsLogListEmpty));
        }
        catch
        {
            Logs.Clear();
            StatusText = string.Empty;
            OnPropertyChanged(nameof(HasLogs));
            OnPropertyChanged(nameof(IsLogListEmpty));
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
            StatusText = ex.Message;
        }
    }
}

public sealed record RunLogLevelOption(string Value, string Label);

public sealed class RunLogItemViewModel
{
    private static readonly IBrush ErrorBrush = new SolidColorBrush(Color.Parse("#DC2626"));
    private static readonly IBrush WarningBrush = new SolidColorBrush(Color.Parse("#D97706"));
    private static readonly IBrush InfoBrush = new SolidColorBrush(Color.Parse("#2563EB"));
    private static readonly IBrush ErrorBg = new SolidColorBrush(Color.Parse("#1FDC2626"));
    private static readonly IBrush WarningBg = new SolidColorBrush(Color.Parse("#1FD97706"));
    private static readonly IBrush InfoBg = new SolidColorBrush(Color.Parse("#1F2563EB"));

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
        LevelForeground = LevelBrushKey switch
        {
            "error" => ErrorBrush,
            "warning" => WarningBrush,
            _ => InfoBrush,
        };
        LevelBackground = LevelBrushKey switch
        {
            "error" => ErrorBg,
            "warning" => WarningBg,
            _ => InfoBg,
        };
    }

    public string LogId { get; }
    public long TimestampMs { get; }
    public string TimestampText { get; }
    public string Kind { get; }
    public string Level { get; }
    public string LevelBrushKey { get; }
    public IBrush LevelForeground { get; }
    public IBrush LevelBackground { get; }
    public string Message { get; }

    private static string FormatTimestamp(long timestampMs)
    {
        try
        {
            var dto = DateTimeOffset.FromUnixTimeMilliseconds(timestampMs).ToLocalTime();
            return dto.ToString("MM-dd HH:mm:ss");
        }
        catch
        {
            return timestampMs.ToString();
        }
    }
}
