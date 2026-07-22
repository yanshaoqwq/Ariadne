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
    ContentError,
}

public sealed class RunLogPageViewModel : ViewModelBase, IProjectDataReloadable, ILocalizedUiAware
{
    private const int PageSize = 100;
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private string _searchQuery = string.Empty;
    private string _selectedLevel = string.Empty;
    private string _selectedKind = string.Empty;
    private string _workflowIdFilter = string.Empty;
    private string _runIdFilter = string.Empty;
    private string _nodeIdFilter = string.Empty;
    private string _statusText = string.Empty;
    private PageLoadState _loadState = PageLoadState.Loading;
    private string _errorText = string.Empty;
    private bool _hasMore;
    private bool _isLoadingMore;
    private bool _isMarkingRead;
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
        KindOptions = new ObservableCollection<RunLogKindOption>
        {
            new(string.Empty, displayNames.Text("ui.run_log.all_kinds")),
            new("node", displayNames.Text("ui.run_log.kind.node")),
            new("tool", displayNames.Text("ui.run_log.kind.tool")),
            new("provider", displayNames.Text("ui.run_log.kind.provider")),
            new("cost", displayNames.Text("ui.run_log.kind.cost")),
            new("confirmation", displayNames.Text("ui.run_log.kind.confirmation")),
            new("error", displayNames.Text("ui.run_log.kind.error")),
            new("diagnostic", displayNames.Text("ui.run_log.kind.diagnostic")),
        };
        SearchCommand = new RelayCommand(() => _ = RefreshAsync());
        RefreshCommand = new RelayCommand(() => _ = RefreshAsync());
        LoadMoreCommand = new RelayCommand(() => _ = LoadMoreAsync(), () => HasMore && !IsLoadingMore);
        MarkReadCommand = new RelayCommand(() => _ = MarkReadAsync(), () => !IsMarkingRead && Logs.Any(log => log.IsUnread));
    }

    public string Title => _displayNames.Text("ui.run_log.title");

    public string SearchPlaceholder => _displayNames.Text("ui.run_log.search.placeholder");

    public string AllLevelsText => _displayNames.Text("ui.run_log.all_levels");

    public string RefreshText => _displayNames.Text("ui.common.refresh");

    public string SearchText => _displayNames.Text("ui.common.search");

    public string MarkReadText => _displayNames.Text(HasActiveFilter
        ? "ui.run_log.mark_read.filtered"
        : "ui.run_log.mark_read.all");

    public string KindFilterText => _displayNames.Text("ui.run_log.filter.kind");
    public string WorkflowFilterText => _displayNames.Text("ui.run_log.filter.workflow");
    public string RunFilterText => _displayNames.Text("ui.run_log.filter.run");
    public string NodeFilterText => _displayNames.Text("ui.run_log.filter.node");
    public string LoadMoreText => _displayNames.Text("ui.run_log.load_more");
    public string LoadingMoreText => _displayNames.Text("ui.run_log.loading_more");

    public string EmptyText => _displayNames.Text("ui.run_log.empty");
    public string EmptyTitle => _loadState == PageLoadState.IdleNeedProject
        ? _displayNames.Text("ui.empty.need_project.title")
        : _displayNames.Text(HasActiveFilter ? "ui.run_log.filtered_empty.title" : "ui.empty.run_log.title");
    public string EmptyHint => _loadState == PageLoadState.IdleNeedProject
        ? _displayNames.Text("ui.empty.need_project.hint")
        : _displayNames.Text(HasActiveFilter ? "ui.run_log.filtered_empty.hint" : "ui.empty.run_log.hint");
    public string ErrorTitle => _displayNames.Text("ui.run_log.error.title");
    public string LoadingText => _displayNames.Text("ui.common.loading");

    public bool HasLogs => Logs.Count > 0;
    public bool IsLogListEmpty => _loadState == PageLoadState.Empty || _loadState == PageLoadState.IdleNeedProject;
    public bool IsLoading => _loadState == PageLoadState.Loading;
    public bool IsError => _loadState == PageLoadState.Error || _loadState == PageLoadState.ContentError;
    public bool IsStandaloneError => _loadState == PageLoadState.Error;
    public bool IsContentError => _loadState == PageLoadState.ContentError;
    public bool ShowEmpty => IsLogListEmpty && !IsLoading && !IsError;
    public bool ShowContent => HasLogs && (_loadState == PageLoadState.Content || _loadState == PageLoadState.ContentError);
    public bool HasActiveFilter => !string.IsNullOrWhiteSpace(SearchQuery)
        || !string.IsNullOrWhiteSpace(SelectedLevel)
        || !string.IsNullOrWhiteSpace(SelectedKind)
        || !string.IsNullOrWhiteSpace(WorkflowIdFilter)
        || !string.IsNullOrWhiteSpace(RunIdFilter)
        || !string.IsNullOrWhiteSpace(NodeIdFilter);

    public string LevelInfoText => _displayNames.Text("ui.level.info");

    public string LevelWarningText => _displayNames.Text("ui.level.warning");

    public string LevelErrorText => _displayNames.Text("ui.level.error");

    public ObservableCollection<RunLogItemViewModel> Logs { get; }

    public ObservableCollection<RunLogLevelOption> LevelOptions { get; }

    public ObservableCollection<RunLogKindOption> KindOptions { get; }

    public RelayCommand SearchCommand { get; }

    public RelayCommand RefreshCommand { get; }

    public RelayCommand MarkReadCommand { get; }

    public RelayCommand LoadMoreCommand { get; }

    public string SearchQuery
    {
        get => _searchQuery;
        set
        {
            if (SetProperty(ref _searchQuery, value))
            {
                NotifyFilterChanged();
            }
        }
    }

    public string SelectedLevel
    {
        get => _selectedLevel;
        set
        {
            if (SetProperty(ref _selectedLevel, value))
            {
                NotifyFilterChanged();
                _ = RefreshAsync();
            }
        }
    }

    public string SelectedKind
    {
        get => _selectedKind;
        set
        {
            if (SetProperty(ref _selectedKind, value))
            {
                NotifyFilterChanged();
                _ = RefreshAsync();
            }
        }
    }

    public string WorkflowIdFilter
    {
        get => _workflowIdFilter;
        set
        {
            if (SetProperty(ref _workflowIdFilter, value))
            {
                NotifyFilterChanged();
            }
        }
    }

    public string RunIdFilter
    {
        get => _runIdFilter;
        set
        {
            if (SetProperty(ref _runIdFilter, value))
            {
                NotifyFilterChanged();
            }
        }
    }

    public string NodeIdFilter
    {
        get => _nodeIdFilter;
        set
        {
            if (SetProperty(ref _nodeIdFilter, value))
            {
                NotifyFilterChanged();
            }
        }
    }

    public bool HasMore
    {
        get => _hasMore;
        private set
        {
            if (SetProperty(ref _hasMore, value))
            {
                LoadMoreCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public bool IsLoadingMore
    {
        get => _isLoadingMore;
        private set
        {
            if (SetProperty(ref _isLoadingMore, value))
            {
                LoadMoreCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public bool IsMarkingRead
    {
        get => _isMarkingRead;
        private set
        {
            if (SetProperty(ref _isMarkingRead, value))
            {
                MarkReadCommand.NotifyCanExecuteChanged();
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
                OnPropertyChanged(nameof(IsStandaloneError));
                OnPropertyChanged(nameof(IsContentError));
                OnPropertyChanged(nameof(ShowEmpty));
                OnPropertyChanged(nameof(ShowContent));
                OnPropertyChanged(nameof(EmptyTitle));
                OnPropertyChanged(nameof(EmptyHint));
            }
        }
    }

    private async Task RefreshAsync(CancellationToken cancellationToken = default)
    {
        var gen = ++_loadGeneration;
        if (!_backend.HasProjectRoot)
        {
            Logs.Clear();
            HasMore = false;
            ErrorText = string.Empty;
            StatusText = string.Empty;
            LoadState = PageLoadState.IdleNeedProject;
            return;
        }

        LoadState = PageLoadState.Loading;
        StatusText = LoadingText;
        try
        {
            var logs = await _backend.QueryRunLogsAsync(
                BuildQuery(limit: PageSize + 1),
                cancellationToken).ConfigureAwait(true);
            cancellationToken.ThrowIfCancellationRequested();
            if (gen != _loadGeneration)
            {
                return;
            }

            var page = logs.Take(PageSize).ToArray();
            Logs.Clear();
            foreach (var log in page)
            {
                Logs.Add(new RunLogItemViewModel(log, _displayNames));
            }
            HasMore = logs.Count > PageSize;
            MarkReadCommand.NotifyCanExecuteChanged();
            ErrorText = string.Empty;
            if (Logs.Count == 0)
            {
                LoadState = PageLoadState.Empty;
                StatusText = EmptyText;
            }
            else
            {
                LoadState = PageLoadState.Content;
                StatusText = _displayNames.Format("ui.run_log.result_count", new Dictionary<string, string>
                {
                    ["count"] = Logs.Count.ToString(),
                });
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
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
            LoadState = Logs.Count > 0 ? PageLoadState.ContentError : PageLoadState.Error;
            // Do not Logs.Clear() — preserve last good snapshot for diagnosis.
        }
    }

    private async Task LoadMoreAsync()
    {
        if (!HasMore || IsLoadingMore || Logs.Count == 0)
        {
            return;
        }

        var generation = _loadGeneration;
        var cursor = Logs[^1];
        IsLoadingMore = true;
        try
        {
            var logs = await _backend.QueryRunLogsAsync(
                BuildQuery(cursor.TimestampMs, cursor.LogId, PageSize + 1)).ConfigureAwait(true);
            if (generation != _loadGeneration)
            {
                return;
            }

            foreach (var log in logs.Take(PageSize))
            {
                if (Logs.All(existing => !string.Equals(existing.LogId, log.LogId, StringComparison.Ordinal)))
                {
                    Logs.Add(new RunLogItemViewModel(log, _displayNames));
                }
            }
            HasMore = logs.Count > PageSize;
            ErrorText = string.Empty;
            LoadState = PageLoadState.Content;
            StatusText = _displayNames.Format("ui.run_log.result_count", new Dictionary<string, string>
            {
                ["count"] = Logs.Count.ToString(),
            });
            MarkReadCommand.NotifyCanExecuteChanged();
        }
        catch (Exception ex)
        {
            if (generation == _loadGeneration)
            {
                ErrorText = UserFacingError.Format(ex, _displayNames);
                StatusText = ErrorText;
                LoadState = PageLoadState.ContentError;
            }
        }
        finally
        {
            IsLoadingMore = false;
        }
    }

    public Task ReloadProjectDataAsync(CancellationToken cancellationToken = default) => RefreshAsync(cancellationToken);

    public void DeactivateProjectData()
    {
        Interlocked.Increment(ref _loadGeneration);
        HasMore = false;
    }

    private async Task MarkReadAsync()
    {
        if (IsMarkingRead)
        {
            return;
        }
        IsMarkingRead = true;
        try
        {
            var updated = await _backend.MarkRunLogsReadAsync(BuildQuery()).ConfigureAwait(true);
            await RefreshAsync().ConfigureAwait(true);
            StatusText = _displayNames.Format("ui.run_log.mark_read.done", new Dictionary<string, string>
            {
                ["count"] = updated.ToString(),
            });
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
            ErrorText = StatusText;
            LoadState = Logs.Count > 0 ? PageLoadState.ContentError : PageLoadState.Error;
        }
        finally
        {
            IsMarkingRead = false;
        }
    }

    private RunLogQuery BuildQuery(long? afterTimestampMs = null, string? afterLogId = null, int? limit = null)
    {
        return new RunLogQuery(
            NullIfWhiteSpace(SelectedKind),
            NullIfWhiteSpace(SelectedLevel),
            NullIfWhiteSpace(WorkflowIdFilter),
            NullIfWhiteSpace(RunIdFilter),
            NullIfWhiteSpace(NodeIdFilter),
            NullIfWhiteSpace(SearchQuery),
            afterTimestampMs,
            afterLogId,
            limit,
            Descending: true);
    }

    private void NotifyFilterChanged()
    {
        OnPropertyChanged(nameof(HasActiveFilter));
        OnPropertyChanged(nameof(MarkReadText));
        OnPropertyChanged(nameof(EmptyTitle));
        OnPropertyChanged(nameof(EmptyHint));
    }

    private static string? NullIfWhiteSpace(string? value)
    {
        return string.IsNullOrWhiteSpace(value) ? null : value.Trim();
    }

    public void RefreshLocalizedUi()
    {
        LevelOptions[0] = new RunLogLevelOption(string.Empty, _displayNames.Text("ui.run_log.all_levels"));
        LevelOptions[1] = new RunLogLevelOption("info", _displayNames.Text("ui.level.info"));
        LevelOptions[2] = new RunLogLevelOption("warning", _displayNames.Text("ui.level.warning"));
        LevelOptions[3] = new RunLogLevelOption("error", _displayNames.Text("ui.level.error"));
        KindOptions[0] = new RunLogKindOption(string.Empty, _displayNames.Text("ui.run_log.all_kinds"));
        for (var index = 1; index < KindOptions.Count; index++)
        {
            var value = KindOptions[index].Value;
            KindOptions[index] = new RunLogKindOption(value, _displayNames.Text($"ui.run_log.kind.{value}"));
        }
        foreach (var log in Logs)
        {
            log.RefreshLocalizedUi(_displayNames);
        }
        OnPropertyChanged(string.Empty);
    }
}

public sealed record RunLogLevelOption(string Value, string Label);

public sealed record RunLogKindOption(string Value, string Label);

public sealed class RunLogItemViewModel : ViewModelBase
{
    public RunLogItemViewModel(UiRunLogEntry entry, DisplayNameService? displayNames = null)
    {
        var names = displayNames ?? DisplayNameService.Current;
        LogId = entry.LogId;
        TimestampMs = entry.TimestampMs;
        Kind = entry.Kind;
        Level = entry.Level;
        Message = entry.Message;
        WorkflowId = entry.WorkflowId;
        RunId = entry.RunId;
        NodeId = entry.NodeId;
        IsUnread = entry.Unread;
        TimestampText = FormatTimestamp(entry.TimestampMs);
        RefreshLocalizedUi(names);
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
    public string? WorkflowId { get; }
    public string? RunId { get; }
    public string? NodeId { get; }
    public string TimestampText { get; }
    public string KindText { get; private set; } = string.Empty;
    public string ContextText { get; private set; } = string.Empty;
    public string UnreadText { get; private set; } = string.Empty;
    public bool HasContext { get; private set; }
    public bool IsUnread { get; }
    public string LevelBrushKey { get; }
    public bool IsError { get; }
    public bool IsWarning { get; }
    public bool IsInfo { get; }

    internal void RefreshLocalizedUi(DisplayNameService names)
    {
        KindText = names.Text($"ui.run_log.kind.{Kind.ToLowerInvariant()}");
        UnreadText = names.Text("ui.run_log.unread");
        var context = new List<string>();
        AddContext(context, names, "ui.run_log.context.workflow", WorkflowId);
        AddContext(context, names, "ui.run_log.context.run", RunId);
        AddContext(context, names, "ui.run_log.context.node", NodeId);
        ContextText = string.Join(" · ", context);
        HasContext = context.Count > 0;
        OnPropertyChanged(string.Empty);
    }

    private static void AddContext(
        ICollection<string> target,
        DisplayNameService names,
        string key,
        string? value)
    {
        if (!string.IsNullOrWhiteSpace(value))
        {
            target.Add(names.Format(key, new Dictionary<string, string> { ["id"] = value }));
        }
    }

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
