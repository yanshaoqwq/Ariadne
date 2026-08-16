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
    private string _runIdFilter = string.Empty;
    private string _nodeIdFilter = string.Empty;
    private string _statusText = string.Empty;
    private PageLoadState _loadState = PageLoadState.Loading;
    private string _errorText = string.Empty;
    private bool _hasMore;
    private bool _isLoadingMore;
    private bool _isMarkingRead;
    private bool _isClearingFilters;
    private RunLogItemViewModel? _selectedLog;
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
        ClearFiltersCommand = new RelayCommand(ClearFilters, () => HasActiveFilter);
        CopySelectedCommand = new RelayCommand(() => _ = CopySelectedAsync(), () => SelectedLog is not null);
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
    // U137：WorkflowFilterText / RunFilterText / NodeFilterText 已删——
    // 它们是三个手打 ID 输入框的 placeholder，输入框删了它们就零引用。
    public string LoadMoreText => _displayNames.Text("ui.run_log.load_more");
    public string LoadingMoreText => _displayNames.Text("ui.run_log.loading_more");
    public string ClearFiltersText => _displayNames.Text("ui.run_log.clear_filters");
    public string CopySelectedText => _displayNames.Text("ui.run_log.copy_selected");

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
        || !string.IsNullOrWhiteSpace(RunIdFilter)
        || !string.IsNullOrWhiteSpace(NodeIdFilter);

    public ObservableCollection<RunLogItemViewModel> Logs { get; }

    /// <summary>
    /// 当前生效的 ID 筛选，以可移除 chip 呈现。
    ///
    /// U137：取代原来三个手打输入框。后端对 run_id / node_id 是**精确等值**匹配
    /// （同一条 SQL 里 message 用的是 LIKE），手敲差一个字符就是空结果、
    /// 且除了"没有结果"以外不给任何提示。取值只能来自用户点过的那条日志，
    /// 就不存在"敲错"这件事。
    /// </summary>
    public ObservableCollection<RunLogContextChipViewModel> ActiveIdFilters { get; } = new();

    public bool HasActiveIdFilters => ActiveIdFilters.Count > 0;

    public ObservableCollection<RunLogLevelOption> LevelOptions { get; }

    public ObservableCollection<RunLogKindOption> KindOptions { get; }

    public RelayCommand SearchCommand { get; }

    public RelayCommand RefreshCommand { get; }

    public RelayCommand MarkReadCommand { get; }

    public RelayCommand LoadMoreCommand { get; }

    public RelayCommand ClearFiltersCommand { get; }

    public RelayCommand CopySelectedCommand { get; }

    public Func<string, Task>? RequestCopyText { get; set; }

    public RunLogItemViewModel? SelectedLog
    {
        get => _selectedLog;
        set
        {
            if (SetProperty(ref _selectedLog, value))
            {
                CopySelectedCommand.NotifyCanExecuteChanged();
            }
        }
    }

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
                if (!_isClearingFilters)
                {
                    _ = RefreshAsync();
                }
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
                if (!_isClearingFilters)
                {
                    _ = RefreshAsync();
                }
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
            SelectedLog = null;
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
            SelectedLog = null;
            foreach (var log in page)
            {
                Logs.Add(CreateLogItem(log));
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
                    Logs.Add(CreateLogItem(log));
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
            // U137：workflow_id 恒传 null。桌面端 2026-07-20 起固定单画布约束
            // （DefaultWorkflowId = "default"），按它筛选没有任何区分度；
            // 原来那个输入框永远只有一个可能值，已删。
            null,
            NullIfWhiteSpace(RunIdFilter),
            NullIfWhiteSpace(NodeIdFilter),
            NullIfWhiteSpace(SearchQuery),
            afterTimestampMs,
            afterLogId,
            limit,
            Descending: true);
    }

    /// <summary>
    /// 构造日志条目并接上 ID 回填。
    ///
    /// U137：**收口成工厂方法**，因为条目在两处被创建（首屏加载 + 加载更多）。
    /// 各自 new 的话漏接一处，就会出现「前 100 条的 ID 能点、往下翻的点不动」——
    /// 这种只在特定路径失效的缺陷最难被发现。
    /// </summary>
    private RunLogItemViewModel CreateLogItem(UiRunLogEntry entry)
    {
        var item = new RunLogItemViewModel(entry, _displayNames);
        item.ApplyFilterRequest = ApplyIdFilter;
        return item;
    }

    private void NotifyFilterChanged()
    {
        OnPropertyChanged(nameof(HasActiveFilter));
        OnPropertyChanged(nameof(MarkReadText));
        OnPropertyChanged(nameof(EmptyTitle));
        OnPropertyChanged(nameof(EmptyHint));
        ClearFiltersCommand.NotifyCanExecuteChanged();
    }

    /// <summary>
    /// U137：点明细里的 ID chip 即回填筛选并立刻重查。
    ///
    /// 同一字段再点一次就换成新值（而不是叠加）——后端是等值匹配，
    /// 两个 run_id 同时生效的语义不存在。
    /// </summary>
    public void ApplyIdFilter(RunLogFilterField field, string value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return;
        }

        _isClearingFilters = true;
        try
        {
            switch (field)
            {
                case RunLogFilterField.RunId:
                    RunIdFilter = value;
                    break;
                case RunLogFilterField.NodeId:
                    NodeIdFilter = value;
                    break;
            }
        }
        finally
        {
            _isClearingFilters = false;
        }
        RebuildActiveIdFilters();
        _ = RefreshAsync();
    }

    /// <summary>U137：移除某个生效的 ID 筛选并重查。</summary>
    public void RemoveIdFilter(RunLogFilterField field)
    {
        _isClearingFilters = true;
        try
        {
            switch (field)
            {
                case RunLogFilterField.RunId:
                    RunIdFilter = string.Empty;
                    break;
                case RunLogFilterField.NodeId:
                    NodeIdFilter = string.Empty;
                    break;
            }
        }
        finally
        {
            _isClearingFilters = false;
        }
        RebuildActiveIdFilters();
        _ = RefreshAsync();
    }

    /// <summary>
    /// 重建筛选区的 chip 列表。
    ///
    /// 每次全量重建而不是增删单项：这个列表最多两条，重建的代价可以忽略，
    /// 而增量维护要处理「同字段换值」「清空全部」等分支，
    /// 漏一条就会出现「筛选还在生效但 chip 已经不见了」——用户无从知道
    /// 为什么列表是空的。
    /// </summary>
    private void RebuildActiveIdFilters()
    {
        ActiveIdFilters.Clear();
        AddActiveIdFilter(RunLogFilterField.RunId, "ui.run_log.context.run", RunIdFilter);
        AddActiveIdFilter(RunLogFilterField.NodeId, "ui.run_log.context.node", NodeIdFilter);
        OnPropertyChanged(nameof(HasActiveIdFilters));
    }

    private void AddActiveIdFilter(RunLogFilterField field, string key, string value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return;
        }

        var label = _displayNames.Format(key, new Dictionary<string, string> { ["id"] = value });
        ActiveIdFilters.Add(new RunLogContextChipViewModel(
            label,
            field,
            value,
            () => RemoveIdFilter(field)));
    }

    private void ClearFilters()
    {
        if (!HasActiveFilter)
        {
            return;
        }

        _isClearingFilters = true;
        try
        {
            SearchQuery = string.Empty;
            SelectedLevel = string.Empty;
            SelectedKind = string.Empty;
            RunIdFilter = string.Empty;
            NodeIdFilter = string.Empty;
        }
        finally
        {
            _isClearingFilters = false;
        }
        RebuildActiveIdFilters();
        _ = RefreshAsync();
    }

    private async Task CopySelectedAsync()
    {
        var selected = SelectedLog;
        if (selected is null || RequestCopyText is null)
        {
            return;
        }
        var text = string.Join(
            " ",
            new[]
            {
                selected.TimestampText,
                selected.LevelText,
                selected.KindText,
                selected.Message,
                selected.ContextText,
            }.Where(value => !string.IsNullOrWhiteSpace(value)));
        // 命令 fire-and-forget 调用本方法，剪贴板失败的异常无人观察，需就地转成状态文案。
        try
        {
            await RequestCopyText(text).ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
            return;
        }

        StatusText = _displayNames.Text("ui.run_log.copied");
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
        Message = LocalizeMessage(entry.Message, names);
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
    // U137：WorkflowId 已删。它恒为 "default"（桌面端固定单画布约束），
    // 既不显示也不筛选也不复制——零读者。要诊断具体工作流身份，
    // 后端日志里那一列一直都在，不需要 UI 层再存一份。
    public string? RunId { get; }
    public string? NodeId { get; }
    public string TimestampText { get; }
    public string KindText { get; private set; } = string.Empty;
    public string LevelText { get; private set; } = string.Empty;
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
        var level = Level.ToLowerInvariant() == "warn" ? "warning" : Level.ToLowerInvariant();
        LevelText = names.Text($"ui.level.{level}");
        UnreadText = names.Text("ui.run_log.unread");
        // U137：ID 由「拼进一个字符串」改为**结构化的可点击项**。
        // 此前 ContextText 是纯 TextBlock，用户唯一的筛选路径是：
        // 看到明细里的 a3f9c2e1-… → 肉眼记住 → 手敲进上面的输入框，
        // 而后端是精确等值匹配，差一个字符就是空结果、且不给任何提示。
        // 只保留 run/node 两类：工作流 ID 恒为 "default"（桌面端 2026-07-20 起
        // 固定单画布约束），拿它筛选没有任何区分度。
        ContextChips.Clear();
        AddContextChip(names, "ui.run_log.context.run", RunLogFilterField.RunId, RunId);
        AddContextChip(names, "ui.run_log.context.node", RunLogFilterField.NodeId, NodeId);
        HasContext = ContextChips.Count > 0;
        // 「复制这条日志」要的是纯文本，从 chip 派生而不是另拼一遍——
        // 两份拼装逻辑迟早漂移，届时复制出来的内容与屏幕上看到的不一致。
        ContextText = string.Join(" · ", ContextChips.Select(chip => chip.Label));
        OnPropertyChanged(string.Empty);
    }

    /// <summary>
    /// 明细里可点击回填的 ID 项。
    ///
    /// 每条日志各自持有，因为 chip 的文案（「运行 a3f9c2e1」）与它承载的
    /// 字段+取值绑在一起——共享一份列表会让点击时分不清是哪条日志的 ID。
    /// </summary>
    public ObservableCollection<RunLogContextChipViewModel> ContextChips { get; } = new();

    /// <summary>View 注入：点击 chip 时把 (字段, 取值) 交给页面去筛选。</summary>
    public Action<RunLogFilterField, string>? ApplyFilterRequest { get; set; }

    private void AddContextChip(
        DisplayNameService names,
        string key,
        RunLogFilterField field,
        string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return;
        }

        var label = names.Format(key, new Dictionary<string, string> { ["id"] = value });
        ContextChips.Add(new RunLogContextChipViewModel(
            label,
            field,
            value,
            () => ApplyFilterRequest?.Invoke(field, value)));
    }

    /// <summary>
    /// 运行生命周期日志的 message 存的是**稳定 key**（`workflow.run.succeeded` 等），
    /// 这里翻译成本地化文案。
    ///
    /// U158：后端记运行终态时刻意存 key 而非中文句子——core 不该硬编码展示文案
    /// （与 `deliver_workflow_run_completion` 同一分层理由）。
    ///
    /// **两条性质合起来保证「非 key 的 message 不会被误翻」**，缺一不可：
    /// 1. 查表用**带命名空间的前缀** `ui.run_log.message.{message}`。
    ///    日志里绝大多数条目（worker 错误、Git 诊断）存的是**真实文本**，
    ///    拼上这个前缀后不可能撞上任何真实存在的 key。
    /// 2. 查不到时**回落到原文**而非显示 `[key]`。
    ///
    /// ⚠️ 这里曾多一道 `message.StartsWith("workflow.run.")` 白名单判定。
    /// 变异测试证明它**没有行为效果**：摘掉后全部用例仍绿——因为上面两条已经
    /// 覆盖了它要防的情况（前缀让碰撞不可能，回落让查不到无害）。
    /// 按「变异后仍绿 ⇒ 那段代码本来就没有行为效果，该删并记录原因」删掉了，
    /// 不留成一道看起来在防守、实际什么也没防的假防御。
    ///
    /// ⚠️ 但若将来把查表改成**无前缀**（直接 `names.Text(message)`），白名单就必须补回来——
    /// 那时真实错误文本可能恰好等于某个已存在的 key（本仓库有 1200+ 个 key），
    /// 会把可读的错误信息静默换成一句无关文案。
    /// </summary>
    private static string LocalizeMessage(string message, DisplayNameService names)
    {
        var localized = names.Text($"ui.run_log.message.{message}");
        // 缺 key 时 Text 返回 `[key]`——那种情况下显示原始 message 比显示带方括号的
        // 长串更可读，也能让人一眼看出「这里缺文案」。
        return localized.StartsWith('[') && localized.EndsWith(']')
            ? message
            : localized;
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

/// <summary>
/// 可点击的 ID 字段。
///
/// U137：用枚举而不是字符串 key，因为回填的目标是三个语义不同的筛选槽，
/// 拼错字符串只会静默不生效——正是这条缺陷「手敲即错、错了只是空结果」的翻版。
/// </summary>
public enum RunLogFilterField
{
    RunId,
    NodeId,
}

/// <summary>
/// 日志明细里可点击回填的 ID chip，以及筛选区里可移除的生效项。
///
/// 同一个类兼两用：明细里点它=加筛选，筛选区里点它=去筛选。
/// 差别只在 <see cref="Invoke"/> 绑的是哪个动作，
/// 因此外观与文案能天然保持一致——用户看到的是「同一个 chip 被搬上去了」。
/// </summary>
public sealed class RunLogContextChipViewModel
{
    public RunLogContextChipViewModel(
        string label,
        RunLogFilterField field,
        string value,
        Action action)
    {
        Label = label;
        Field = field;
        Value = value;
        Command = new RelayCommand(action);
    }

    public string Label { get; }
    public RunLogFilterField Field { get; }
    public string Value { get; }
    public RelayCommand Command { get; }
}
