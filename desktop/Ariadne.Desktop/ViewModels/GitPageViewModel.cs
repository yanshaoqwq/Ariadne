using System.Collections.ObjectModel;
using Avalonia.Media;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public enum GitOperationState
{
    Idle,
    Refreshing,
    Checkpointing,
    Restoring,
}

public sealed class GitPageViewModel : ViewModelBase, IProjectDataReloadable, IUiPreferencesAware, ILocalizedUiAware
{
    private const string RightPanelPreferenceKey = "git.right_panel";
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private readonly Func<Task<bool>> _confirmProjectReload;
    private readonly Func<Task> _reloadProjectData;
    private readonly Func<string, bool, Task>? _persistPanelState;
    private string _gitAutoColor = "#8a8f98";
    private string _gitManualColor = "#f59e0b";
    private bool _isRightPanelOpen = true;
    private string _checkpointMessage = string.Empty;
    private string _restoreBranchName = string.Empty;
    private string _statusText = string.Empty;
    private string _repositoryStatusText = string.Empty;
    private string _currentBranchText = string.Empty;
    private string _headText = string.Empty;
    private string _dirtyStateText = string.Empty;
    private string _repositoryReasonText = string.Empty;
    private string _diffSummaryText = string.Empty;
    private string _diffPreviewText = string.Empty;
    private GitHistoryItemViewModel? _selectedCommit;
    private GitOperationState _operationState;
    private long _loadGeneration;

    public GitPageViewModel(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        Func<Task<bool>>? confirmProjectReload = null,
        Func<Task>? reloadProjectData = null,
        Func<string, bool, Task>? persistPanelState = null)
    {
        _displayNames = displayNames;
        _backend = backend;
        _confirmProjectReload = confirmProjectReload ?? (() => Task.FromResult(true));
        _reloadProjectData = reloadProjectData ?? (() => Task.CompletedTask);
        _persistPanelState = persistPanelState;
        Commits = new ObservableCollection<GitHistoryItemViewModel>();
        ToggleRightPanelCommand = new RelayCommand(() => _ = ToggleRightPanelAsync());
        RefreshCommand = new RelayCommand(() => _ = RefreshAsync(), CanStartOperation);
        CreateCheckpointCommand = new RelayCommand(() => _ = CreateCheckpointAsync(), CanStartOperation);
        ViewDetailsCommand = new RelayCommand(() => ViewDetails(SelectedCommit), () => HasSelection);
        RestoreCommand = new RelayCommand(
            () => _ = RestoreSelectedAsync(),
            () => HasSelection && CanStartOperation());
        CopyIdCommand = new RelayCommand(() => _ = CopyCommitIdAsync(SelectedCommit), () => HasSelection);
    }

    public string ToggleRightPanelText => _displayNames.Text("ui.action.toggle_right_panel");
    public bool IsRightPanelOpen
    {
        get => _isRightPanelOpen;
        set => SetProperty(ref _isRightPanelOpen, value);
    }

    public RelayCommand ToggleRightPanelCommand { get; }
    public RelayCommand RefreshCommand { get; }
    public RelayCommand CreateCheckpointCommand { get; }
    public RelayCommand ViewDetailsCommand { get; }
    public RelayCommand RestoreCommand { get; }
    public RelayCommand CopyIdCommand { get; }
    public ObservableCollection<GitHistoryItemViewModel> Commits { get; }
    public Func<string, Task>? RequestCopyText { get; set; }

    public void ApplyUiPreferences(UiPreferences preferences)
    {
        _gitAutoColor = preferences.GitAutoColor;
        _gitManualColor = preferences.GitManualColor;
        if (preferences.PanelStates?.TryGetValue(RightPanelPreferenceKey, out var isOpen) == true)
        {
            IsRightPanelOpen = isOpen;
        }
        foreach (var commit in Commits)
        {
            commit.ApplyMarkerColors(_gitAutoColor, _gitManualColor);
        }
    }

    public void RefreshLocalizedUi()
    {
        foreach (var commit in Commits)
        {
            commit.RefreshLocalizedUi(
                _displayNames,
                commit.IsAutoCheckpoint ? AutoKindText : commit.IsManualCheckpoint ? ManualKindText : string.Empty,
                HeadBadgeText,
                MergeBadgeText,
                CtxViewDetailsText,
                CtxRestoreText,
                CtxCopyIdText);
        }
        OnPropertyChanged(string.Empty);
    }

    private async Task ToggleRightPanelAsync()
    {
        IsRightPanelOpen = !IsRightPanelOpen;
        if (_persistPanelState is null)
        {
            return;
        }
        try
        {
            await _persistPanelState(RightPanelPreferenceKey, IsRightPanelOpen).ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    public string CheckpointMessage
    {
        get => _checkpointMessage;
        set => SetProperty(ref _checkpointMessage, value);
    }

    public string RestoreBranchName
    {
        get => _restoreBranchName;
        set => SetProperty(ref _restoreBranchName, value);
    }

    public string StatusText
    {
        get => _statusText;
        set => SetProperty(ref _statusText, value);
    }

    public string RepositoryStatusText
    {
        get => _repositoryStatusText;
        private set => SetProperty(ref _repositoryStatusText, value);
    }

    public string CurrentBranchText
    {
        get => _currentBranchText;
        private set => SetProperty(ref _currentBranchText, value);
    }

    public string HeadText
    {
        get => _headText;
        private set => SetProperty(ref _headText, value);
    }

    public string DirtyStateText
    {
        get => _dirtyStateText;
        private set => SetProperty(ref _dirtyStateText, value);
    }

    public string RepositoryReasonText
    {
        get => _repositoryReasonText;
        private set => SetProperty(ref _repositoryReasonText, value);
    }

    public string DiffSummaryText
    {
        get => _diffSummaryText;
        private set => SetProperty(ref _diffSummaryText, value);
    }

    public string DiffPreviewText
    {
        get => _diffPreviewText;
        private set => SetProperty(ref _diffPreviewText, value);
    }

    public bool HasRepositoryReason => !string.IsNullOrWhiteSpace(RepositoryReasonText);
    public bool HasDiffPreview => !string.IsNullOrWhiteSpace(DiffPreviewText);

    public GitHistoryItemViewModel? SelectedCommit
    {
        get => _selectedCommit;
        set
        {
            if (SetProperty(ref _selectedCommit, value))
            {
                OnPropertyChanged(nameof(SelectedSummary));
                OnPropertyChanged(nameof(SelectedCommitId));
                OnPropertyChanged(nameof(SelectedKind));
                OnPropertyChanged(nameof(SelectedParents));
                OnPropertyChanged(nameof(SelectedRefs));
                OnPropertyChanged(nameof(SelectedAuthor));
                OnPropertyChanged(nameof(SelectedTime));
                OnPropertyChanged(nameof(HasSelection));
                NotifySelectionCommands();
            }
        }
    }

    public GitOperationState OperationState
    {
        get => _operationState;
        private set
        {
            if (SetProperty(ref _operationState, value))
            {
                OnPropertyChanged(nameof(IsBusy));
                OnPropertyChanged(nameof(IsRefreshing));
                OnPropertyChanged(nameof(IsCheckpointing));
                OnPropertyChanged(nameof(IsRestoring));
                OnPropertyChanged(nameof(OperationStatusText));
                NotifyOperationCommands();
            }
        }
    }

    public bool IsBusy => OperationState != GitOperationState.Idle;
    public bool IsRefreshing => OperationState == GitOperationState.Refreshing;
    public bool IsCheckpointing => OperationState == GitOperationState.Checkpointing;
    public bool IsRestoring => OperationState == GitOperationState.Restoring;
    public string OperationStatusText => OperationState switch
    {
        GitOperationState.Refreshing => _displayNames.Text("ui.git.operation.refreshing"),
        GitOperationState.Checkpointing => _displayNames.Text("ui.git.operation.checkpointing"),
        GitOperationState.Restoring => _displayNames.Text("ui.git.operation.restoring"),
        _ => string.Empty,
    };

    public bool HasSelection => SelectedCommit is not null;
    public bool HasCommits => Commits.Count > 0;
    public bool IsCommitListEmpty => Commits.Count == 0;
    public string EmptyTitle => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.git.title")
        : _displayNames.Text("ui.empty.need_project.title");
    public string EmptyHint => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.git.hint")
        : _displayNames.Text("ui.empty.need_project.hint");
    public string SelectedSummary => SelectedCommit?.Summary ?? NoSelectionText;
    public string SelectedCommitId => SelectedCommit?.CommitId ?? _displayNames.Text("ui.common.none");
    public string SelectedKind => SelectedCommit?.KindText ?? _displayNames.Text("ui.common.none");
    public string SelectedParents => SelectedCommit is null || SelectedCommit.Parents.Count == 0
        ? _displayNames.Text("ui.common.none")
        : string.Join(", ", SelectedCommit.Parents);
    public string SelectedRefs => SelectedCommit is null || SelectedCommit.Refs.Count == 0
        ? _displayNames.Text("ui.common.none")
        : string.Join(", ", SelectedCommit.Refs);
    public string SelectedAuthor => SelectedCommit?.AuthorText ?? _displayNames.Text("ui.common.none");
    public string SelectedTime => SelectedCommit?.TimestampText ?? _displayNames.Text("ui.common.none");

    public string Title => _displayNames.Text("ui.git.title");
    public string Description => _displayNames.Text("ui.git.desc");
    public string RefreshText => _displayNames.Text("ui.common.refresh");
    public string CheckpointPlaceholder => _displayNames.Text("ui.git.checkpoint.placeholder");
    public string CreateCheckpointText => _displayNames.Text("ui.git.create_checkpoint");
    public string BranchGraphText => _displayNames.Text("ui.git.branch_graph");
    public string DetailsText => _displayNames.Text("ui.git.details");
    public string TechnicalDetailsText => _displayNames.Text("ui.git.technical_details");
    public string NoSelectionText => _displayNames.Text("ui.git.no_selection");
    public string EmptyText => _displayNames.Text("ui.git.empty");
    public string RestoreBranchNameText => _displayNames.Text("ui.git.restore_branch_name");
    public string RestoreNewBranchText => _displayNames.Text("ui.git.restore_new_branch");
    public string SummaryLabel => _displayNames.Text("ui.git.summary");
    public string CommitLabel => _displayNames.Text("ui.git.commit_id");
    public string KindLabel => _displayNames.Text("ui.git.kind");
    public string ParentsLabel => _displayNames.Text("ui.git.parents");
    public string AuthorLabel => _displayNames.Text("ui.git.author");
    public string TimeLabel => _displayNames.Text("ui.git.time");
    public string ManualKindText => _displayNames.Text("ui.git.kind.manual");
    public string AutoKindText => _displayNames.Text("ui.git.kind.auto");
    public string BranchRefsLabel => _displayNames.Text("ui.git.refs");
    public string RepositoryStatusLabel => _displayNames.Text("ui.git.repository_status");
    public string CurrentBranchLabel => _displayNames.Text("ui.git.current_branch");
    public string HeadLabel => _displayNames.Text("ui.git.head");
    public string DirtyStateLabel => _displayNames.Text("ui.git.dirty_state");
    public string RepositoryReasonLabel => _displayNames.Text("ui.git.reason");
    public string DiffSummaryLabel => _displayNames.Text("ui.git.diff_summary");
    public string DiffPreviewLabel => _displayNames.Text("ui.git.diff_preview");
    public string CtxViewDetailsText => _displayNames.Text("ui.git.context.view_details");
    public string CtxRestoreText => _displayNames.Text("ui.git.context.restore");
    public string CtxCopyIdText => _displayNames.Text("ui.git.context.copy_id");
    public string HeadBadgeText => _displayNames.Text("ui.git.head_badge");
    public string MergeBadgeText => _displayNames.Text("ui.git.merge_badge");

    private async Task RefreshAsync(CancellationToken cancellationToken = default)
    {
        if (!TryBeginOperation(GitOperationState.Refreshing))
        {
            return;
        }
        try
        {
            await RefreshCoreAsync(cancellationToken).ConfigureAwait(true);
        }
        finally
        {
            EndOperation();
        }
    }

    private async Task RefreshCoreAsync(CancellationToken cancellationToken)
    {
        var generation = Interlocked.Increment(ref _loadGeneration);
        if (!_backend.HasProjectRoot)
        {
            ClearProjectState();
            return;
        }

        await RefreshRepositoryStatusAsync(cancellationToken).ConfigureAwait(true);
        cancellationToken.ThrowIfCancellationRequested();
        try
        {
            var graph = await _backend.GetGitBranchGraphAsync(cancellationToken: cancellationToken).ConfigureAwait(true);
            cancellationToken.ThrowIfCancellationRequested();
            if (generation != Interlocked.Read(ref _loadGeneration))
            {
                return;
            }
            ApplyBranchGraph(graph);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch
        {
            await RefreshHistoryFallbackAsync(cancellationToken, generation).ConfigureAwait(true);
        }
    }

    public async Task ReloadProjectDataAsync(CancellationToken cancellationToken = default)
    {
        await RefreshAsync(cancellationToken).ConfigureAwait(true);
    }

    public void DeactivateProjectData()
    {
        Interlocked.Increment(ref _loadGeneration);
    }

    private void ClearProjectState()
    {
        Commits.Clear();
        SelectedCommit = null;
        StatusText = string.Empty;
        RepositoryStatusText = string.Empty;
        CurrentBranchText = _displayNames.Text("ui.common.none");
        HeadText = _displayNames.Text("ui.common.none");
        DirtyStateText = _displayNames.Text("ui.common.none");
        RepositoryReasonText = string.Empty;
        DiffSummaryText = _displayNames.Text("ui.common.none");
        DiffPreviewText = string.Empty;
        NotifyHistoryState();
    }

    private async Task RefreshRepositoryStatusAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            var status = await _backend.GetGitRepositoryStatusAsync(cancellationToken).ConfigureAwait(true);
            cancellationToken.ThrowIfCancellationRequested();
            ApplyRepositoryStatus(status);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch (Exception ex)
        {
            RepositoryStatusText = UserFacingError.Format(ex, _displayNames);
            CurrentBranchText = _displayNames.Text("ui.common.none");
            HeadText = _displayNames.Text("ui.common.none");
            DirtyStateText = _displayNames.Text("ui.common.none");
            RepositoryReasonText = string.Empty;
            DiffSummaryText = _displayNames.Text("ui.common.none");
            DiffPreviewText = string.Empty;
            NotifyRepositoryVisibility();
        }
    }

    private async Task RefreshHistoryFallbackAsync(CancellationToken cancellationToken, long generation)
    {
        try
        {
            var history = await _backend.GetGitHistoryAsync(cancellationToken).ConfigureAwait(true);
            cancellationToken.ThrowIfCancellationRequested();
            if (generation != Interlocked.Read(ref _loadGeneration))
            {
                return;
            }

            var previousId = SelectedCommit?.CommitId;
            Commits.Clear();
            foreach (var commit in history)
            {
                Commits.Add(CreateHistoryItem(
                    commit.CommitId,
                    commit.Summary,
                    Array.Empty<string>(),
                    Array.Empty<string>(),
                    commit.TimestampMs,
                    commit.Author,
                    commit.CheckpointKind,
                    isHead: false,
                    laneIndex: 0));
            }
            SelectAfterRefresh(previousId);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
            NotifyHistoryState();
        }
    }

    private void ApplyBranchGraph(IReadOnlyList<BranchGraphNode> graph)
    {
        var previousId = SelectedCommit?.CommitId;
        var lanes = new List<string>();
        Commits.Clear();
        foreach (var node in graph)
        {
            var laneIndex = lanes.FindIndex(id => string.Equals(id, node.CommitId, StringComparison.Ordinal));
            if (laneIndex < 0)
            {
                laneIndex = lanes.Count;
                lanes.Add(node.CommitId);
            }

            if (node.Parents.Count == 0)
            {
                lanes.RemoveAt(laneIndex);
            }
            else
            {
                lanes[laneIndex] = node.Parents[0];
                for (var index = 1; index < node.Parents.Count; index++)
                {
                    if (!lanes.Contains(node.Parents[index], StringComparer.Ordinal))
                    {
                        lanes.Insert(Math.Min(laneIndex + index, lanes.Count), node.Parents[index]);
                    }
                }
            }

            Commits.Add(CreateHistoryItem(
                node.CommitId,
                node.Summary,
                node.Parents,
                node.Refs,
                node.TimestampMs,
                node.Author,
                node.CheckpointKind,
                node.IsHead,
                laneIndex));
        }
        SelectAfterRefresh(previousId);
    }

    private GitHistoryItemViewModel CreateHistoryItem(
        string commitId,
        string summary,
        IReadOnlyList<string> parents,
        IReadOnlyList<string> refs,
        long timestampMs,
        string? author,
        string? checkpointKind,
        bool isHead,
        int laneIndex)
    {
        var resolvedKind = ResolveCheckpointKind(checkpointKind, summary);
        return new GitHistoryItemViewModel(
            commitId,
            summary,
            parents,
            refs,
            timestampMs,
            author,
            KindText(resolvedKind, summary),
            resolvedKind == "auto",
            resolvedKind == "manual",
            _gitAutoColor,
            _gitManualColor,
            isHead || refs.Any(value => value == "HEAD" || value.StartsWith("HEAD -> ", StringComparison.Ordinal)),
            laneIndex,
            HeadBadgeText,
            MergeBadgeText,
            CtxViewDetailsText,
            CtxRestoreText,
            CtxCopyIdText,
            _displayNames,
            SelectCommit,
            ViewDetails,
            RestoreCommitAsync,
            CopyCommitIdAsync,
            CanStartOperation);
    }

    private void SelectAfterRefresh(string? previousId)
    {
        SelectedCommit = previousId is null
            ? Commits.FirstOrDefault()
            : Commits.FirstOrDefault(item => item.CommitId == previousId) ?? Commits.FirstOrDefault();
        StatusText = Commits.Count == 0
            ? EmptyText
            : _displayNames.Format("ui.git.count", new Dictionary<string, string>
            {
                ["count"] = Commits.Count.ToString(),
            });
        NotifyHistoryState();
    }

    private async Task CreateCheckpointAsync()
    {
        if (!TryBeginOperation(GitOperationState.Checkpointing))
        {
            return;
        }
        try
        {
            var checkpoint = await _backend.CreateCheckpointAsync(CheckpointMessage).ConfigureAwait(true);
            var summary = (checkpoint.Message ?? string.Empty).Trim();
            StatusText = summary.Length is > 0 and <= 80
                ? _displayNames.Format("ui.git.checkpoint_created", new Dictionary<string, string> { ["summary"] = summary })
                : _displayNames.Text("ui.git.checkpoint_created_plain");
            CheckpointMessage = string.Empty;
            await RefreshCoreAsync(CancellationToken.None).ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
        finally
        {
            EndOperation();
        }
    }

    private async Task RestoreSelectedAsync()
    {
        if (SelectedCommit is null)
        {
            StatusText = NoSelectionText;
            return;
        }
        await RestoreCommitAsync(SelectedCommit).ConfigureAwait(true);
    }

    private async Task RestoreCommitAsync(GitHistoryItemViewModel commit)
    {
        if (!TryBeginOperation(GitOperationState.Restoring))
        {
            return;
        }
        try
        {
            var branch = string.IsNullOrWhiteSpace(RestoreBranchName)
                ? $"restore-{commit.ShortCommitId}"
                : RestoreBranchName.Trim();
            if (!await ConfirmRestoreAsync(commit, branch).ConfigureAwait(true))
            {
                return;
            }
            if (!await _confirmProjectReload().ConfigureAwait(true))
            {
                return;
            }
            var report = await _backend.RestoreToNewBranchAsync(commit.CommitId, branch).ConfigureAwait(true);
            StatusText = _displayNames.Format("ui.git.restore_done", new Dictionary<string, string>
            {
                ["branch"] = report.NewBranch,
                ["followup"] = RestoreFollowUpText(report),
            });
            RestoreBranchName = string.Empty;
            await RefreshCoreAsync(CancellationToken.None).ConfigureAwait(true);
            await _reloadProjectData().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
        finally
        {
            EndOperation();
        }
    }

    private void ViewDetails(GitHistoryItemViewModel? commit)
    {
        if (commit is not null)
        {
            SelectedCommit = commit;
            IsRightPanelOpen = true;
        }
    }

    private string RestoreFollowUpText(RestoreReport report)
    {
        return (report.IndexRebuildRequired, report.RuntimeRebindRequired) switch
        {
            (true, true) => _displayNames.Text("ui.git.restore_followup.index_runtime"),
            (true, false) => _displayNames.Text("ui.git.restore_followup.index"),
            (false, true) => _displayNames.Text("ui.git.restore_followup.runtime"),
            _ => _displayNames.Text("ui.git.restore_followup.none"),
        };
    }

    private void ApplyRepositoryStatus(GitRepositoryStatus status)
    {
        RepositoryStatusText = status.Status switch
        {
            "healthy" => _displayNames.Text("ui.git.status.healthy"),
            "degraded" => _displayNames.Text("ui.git.status.degraded"),
            "not_repository" => _displayNames.Text("ui.git.status.not_repository"),
            "unavailable" => _displayNames.Text("ui.git.status.unavailable"),
            _ => _displayNames.Text("ui.git.status.unavailable"),
        };
        CurrentBranchText = string.IsNullOrWhiteSpace(status.Branch)
            ? _displayNames.Text("ui.common.none")
            : status.Branch;
        HeadText = string.IsNullOrWhiteSpace(status.Head)
            ? _displayNames.Text("ui.common.none")
            : ShortHash(status.Head);
        DirtyStateText = status.Dirty
            ? _displayNames.Text("ui.git.dirty")
            : _displayNames.Text("ui.git.clean");
        RepositoryReasonText = status.Status switch
        {
            "degraded" => _displayNames.Text("ui.git.reason.no_commits"),
            "not_repository" => _displayNames.Text("ui.git.reason.not_repository"),
            _ => string.Empty,
        };
        DiffSummaryText = _displayNames.Format("ui.git.diff_lines", new Dictionary<string, string>
        {
            ["count"] = status.DiffLineCount.ToString(),
        });
        DiffPreviewText = status.DiffPreview;
        NotifyRepositoryVisibility();
    }

    private void NotifyRepositoryVisibility()
    {
        OnPropertyChanged(nameof(HasRepositoryReason));
        OnPropertyChanged(nameof(HasDiffPreview));
    }

    private static string ShortHash(string value)
    {
        return value.Length <= 12 ? value : value[..12];
    }

    private async Task CopyCommitIdAsync(GitHistoryItemViewModel? commit)
    {
        if (commit is null)
        {
            StatusText = NoSelectionText;
            return;
        }
        try
        {
            if (RequestCopyText is not null)
            {
                await RequestCopyText(commit.CommitId).ConfigureAwait(true);
                StatusText = _displayNames.Text("ui.git.copied_commit_id");
                return;
            }
            StatusText = commit.CommitId;
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private void SelectCommit(GitHistoryItemViewModel item)
    {
        SelectedCommit = item;
    }

    private string KindText(string? checkpointKind, string summary)
    {
        return ResolveCheckpointKind(checkpointKind, summary) switch
        {
            "auto" => AutoKindText,
            "manual" => ManualKindText,
            _ => string.Empty,
        };
    }

    private static string? ResolveCheckpointKind(string? checkpointKind, string summary)
    {
        return checkpointKind switch
        {
            "auto" or "manual" => checkpointKind,
            _ when summary.StartsWith("Checkpoint:", StringComparison.OrdinalIgnoreCase) => "auto",
            _ when summary.StartsWith("Archive:", StringComparison.OrdinalIgnoreCase) => "manual",
            _ => null,
        };
    }

    private bool TryBeginOperation(GitOperationState operation)
    {
        if (!CanStartOperation())
        {
            return false;
        }
        OperationState = operation;
        return true;
    }

    private void EndOperation()
    {
        OperationState = GitOperationState.Idle;
    }

    private bool CanStartOperation()
    {
        return OperationState == GitOperationState.Idle && _backend.HasProjectRoot;
    }

    private void NotifySelectionCommands()
    {
        ViewDetailsCommand.NotifyCanExecuteChanged();
        RestoreCommand.NotifyCanExecuteChanged();
        CopyIdCommand.NotifyCanExecuteChanged();
    }

    private void NotifyOperationCommands()
    {
        RefreshCommand.NotifyCanExecuteChanged();
        CreateCheckpointCommand.NotifyCanExecuteChanged();
        RestoreCommand.NotifyCanExecuteChanged();
        foreach (var commit in Commits)
        {
            commit.NotifyOperationStateChanged();
        }
    }

    private void NotifyHistoryState()
    {
        OnPropertyChanged(nameof(HasCommits));
        OnPropertyChanged(nameof(IsCommitListEmpty));
        OnPropertyChanged(nameof(EmptyTitle));
        OnPropertyChanged(nameof(EmptyHint));
        NotifySelectionCommands();
    }

    private async Task<bool> ConfirmRestoreAsync(GitHistoryItemViewModel commit, string branch)
    {
        var message = _displayNames.Format("ui.dialog.git.restore.message_detailed", new Dictionary<string, string>
        {
            ["summary"] = commit.Summary,
            ["time"] = commit.TimestampText,
            ["refs"] = commit.Refs.Count == 0 ? _displayNames.Text("ui.common.none") : commit.RefsText,
            ["commit"] = commit.CommitId,
            ["branch"] = branch,
        });
        var dialog = new ConfirmDialogViewModel(
            _displayNames.Text("ui.dialog.git.restore.title"),
            message,
            new[]
            {
                new DialogButton(_displayNames.Text("ui.dialog.git.restore.confirm"), DialogButtonVariant.Danger, 0),
                new DialogButton(_displayNames.Text("ui.common.cancel"), DialogButtonVariant.Subtle, 1),
            })
        {
            Severity = DialogSeverity.Danger,
            CancelResultIndex = 1,
            ConfirmResultIndex = 0,
        };
        return await DialogService.Current.ConfirmAsync(dialog).ConfigureAwait(true) == 0;
    }
}

public sealed class GitHistoryItemViewModel : ViewModelBase
{
    private IBrush? _markerBrush;

    public GitHistoryItemViewModel(
        string commitId,
        string summary,
        IReadOnlyList<string> parents,
        IReadOnlyList<string> refs,
        long timestampMs,
        string? author,
        string kindText,
        bool isAutoCheckpoint,
        bool isManualCheckpoint,
        string autoColor,
        string manualColor,
        bool isHead,
        int laneIndex,
        string headBadgeText,
        string mergeBadgeText,
        string viewDetailsText,
        string restoreText,
        string copyIdText,
        DisplayNameService displayNames,
        Action<GitHistoryItemViewModel> select,
        Action<GitHistoryItemViewModel> viewDetails,
        Func<GitHistoryItemViewModel, Task> restore,
        Func<GitHistoryItemViewModel, Task> copyId,
        Func<bool> canStartOperation)
    {
        CommitId = commitId;
        Summary = summary;
        Parents = parents;
        Refs = refs;
        TimestampMs = timestampMs;
        Author = author;
        AuthorText = string.IsNullOrWhiteSpace(author) ? displayNames.Text("ui.common.none") : author;
        KindText = kindText;
        IsAutoCheckpoint = isAutoCheckpoint;
        IsManualCheckpoint = isManualCheckpoint;
        IsHead = isHead;
        LaneIndex = Math.Clamp(laneIndex, 0, 8);
        LaneOffset = LaneIndex * 14d;
        HeadBadgeText = headBadgeText;
        MergeBadgeText = mergeBadgeText;
        ViewDetailsText = viewDetailsText;
        RestoreText = restoreText;
        CopyIdText = copyIdText;
        TimestampText = timestampMs > 0
            ? FormatTimestamp(timestampMs)
            : displayNames.Text("ui.common.none");
        RelativeTimeText = timestampMs > 0
            ? FormatRelativeTime(timestampMs, displayNames)
            : displayNames.Text("ui.common.none");
        SelectCommand = new RelayCommand(() => select(this));
        ViewDetailsCommand = new RelayCommand(() =>
        {
            select(this);
            viewDetails(this);
        });
        RestoreCommand = new RelayCommand(() =>
        {
            select(this);
            _ = restore(this);
        }, canStartOperation);
        CopyIdCommand = new RelayCommand(() =>
        {
            select(this);
            _ = copyId(this);
        });
        ApplyMarkerColors(autoColor, manualColor);
    }

    public string CommitId { get; }
    public string ShortCommitId => CommitId.Length <= 7 ? CommitId : CommitId[..7];
    public string Summary { get; }
    public IReadOnlyList<string> Parents { get; }
    public IReadOnlyList<string> Refs { get; }
    public long TimestampMs { get; }
    public string TimestampText { get; private set; }
    public string RelativeTimeText { get; private set; }
    public string? Author { get; }
    public string AuthorText { get; private set; }
    public string KindText { get; private set; }
    public bool IsAutoCheckpoint { get; }
    public bool IsManualCheckpoint { get; }
    public bool HasCustomMarker => IsAutoCheckpoint || IsManualCheckpoint;
    public IBrush? MarkerBrush
    {
        get => _markerBrush;
        private set => SetProperty(ref _markerBrush, value);
    }
    public string RefsText => Refs.Count == 0 ? string.Empty : string.Join(" · ", Refs);
    public bool HasRefs => Refs.Count > 0;
    public bool HasKind => !string.IsNullOrWhiteSpace(KindText);
    public bool IsHead { get; }
    public bool IsMerge => Parents.Count > 1;
    public bool HasGraphContinuation => Parents.Count > 0;
    public int LaneIndex { get; }
    public double LaneOffset { get; }
    public string HeadBadgeText { get; private set; }
    public string MergeBadgeText { get; private set; }
    public string ViewDetailsText { get; private set; }
    public string RestoreText { get; private set; }
    public string CopyIdText { get; private set; }
    public RelayCommand SelectCommand { get; }
    public RelayCommand ViewDetailsCommand { get; }
    public RelayCommand RestoreCommand { get; }
    public RelayCommand CopyIdCommand { get; }

    public void NotifyOperationStateChanged()
    {
        RestoreCommand.NotifyCanExecuteChanged();
    }

    internal void RefreshLocalizedUi(
        DisplayNameService displayNames,
        string kindText,
        string headBadgeText,
        string mergeBadgeText,
        string viewDetailsText,
        string restoreText,
        string copyIdText)
    {
        AuthorText = string.IsNullOrWhiteSpace(Author) ? displayNames.Text("ui.common.none") : Author!;
        KindText = kindText;
        HeadBadgeText = headBadgeText;
        MergeBadgeText = mergeBadgeText;
        ViewDetailsText = viewDetailsText;
        RestoreText = restoreText;
        CopyIdText = copyIdText;
        TimestampText = TimestampMs > 0
            ? FormatTimestamp(TimestampMs)
            : displayNames.Text("ui.common.none");
        RelativeTimeText = TimestampMs > 0
            ? FormatRelativeTime(TimestampMs, displayNames)
            : displayNames.Text("ui.common.none");
        OnPropertyChanged(string.Empty);
    }

    public void ApplyMarkerColors(string autoColor, string manualColor)
    {
        if (!HasCustomMarker)
        {
            MarkerBrush = null;
            return;
        }
        var value = IsAutoCheckpoint ? autoColor : manualColor;
        try
        {
            MarkerBrush = new SolidColorBrush(Color.Parse(value));
        }
        catch
        {
            MarkerBrush = null;
        }
    }

    private static string FormatTimestamp(long timestampMs)
    {
        try
        {
            return DateTimeOffset.FromUnixTimeMilliseconds(timestampMs)
                .ToLocalTime()
                .ToString("yyyy-MM-dd HH:mm:ss zzz");
        }
        catch
        {
            return timestampMs.ToString();
        }
    }

    private static string FormatRelativeTime(long timestampMs, DisplayNameService names)
    {
        try
        {
            var timestamp = DateTimeOffset.FromUnixTimeMilliseconds(timestampMs);
            var elapsed = DateTimeOffset.Now - timestamp;
            if (elapsed < TimeSpan.FromMinutes(1))
            {
                return names.Text("ui.git.time.just_now");
            }
            if (elapsed < TimeSpan.FromHours(1))
            {
                return names.Format("ui.git.time.minutes_ago", new Dictionary<string, string>
                {
                    ["count"] = Math.Max(1, (int)elapsed.TotalMinutes).ToString(),
                });
            }
            if (elapsed < TimeSpan.FromDays(1))
            {
                return names.Format("ui.git.time.hours_ago", new Dictionary<string, string>
                {
                    ["count"] = Math.Max(1, (int)elapsed.TotalHours).ToString(),
                });
            }
            if (elapsed < TimeSpan.FromDays(7))
            {
                return names.Format("ui.git.time.days_ago", new Dictionary<string, string>
                {
                    ["count"] = Math.Max(1, (int)elapsed.TotalDays).ToString(),
                });
            }
            return timestamp.ToLocalTime().ToString("yyyy-MM-dd");
        }
        catch
        {
            return timestampMs.ToString();
        }
    }
}
