using System.Collections.ObjectModel;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class GitPageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private readonly Func<Task<bool>> _confirmProjectReload;
    private readonly Func<Task> _reloadProjectData;
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

    public GitPageViewModel(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        Func<Task<bool>>? confirmProjectReload = null,
        Func<Task>? reloadProjectData = null)
    {
        _displayNames = displayNames;
        _backend = backend;
        _confirmProjectReload = confirmProjectReload ?? (() => Task.FromResult(true));
        _reloadProjectData = reloadProjectData ?? (() => Task.CompletedTask);
        Commits = new ObservableCollection<GitHistoryItemViewModel>();
        ToggleRightPanelCommand = new RelayCommand(() => IsRightPanelOpen = !IsRightPanelOpen);
        RefreshCommand = new RelayCommand(() => _ = RefreshAsync());
        CreateCheckpointCommand = new RelayCommand(() => _ = CreateCheckpointAsync());
        ViewDetailsCommand = new RelayCommand(() => ViewDetails(SelectedCommit), () => HasSelection);
        RestoreCommand = new RelayCommand(() => _ = RestoreSelectedAsync(), () => HasSelection);
        CopyIdCommand = new RelayCommand(() => _ = CopyCommitIdAsync(SelectedCommit), () => HasSelection);
        _ = RefreshAsync();
    }

    public string ToggleRightPanelText => _displayNames.Text("ui.action.toggle_right_panel");
    public bool IsRightPanelOpen { get => _isRightPanelOpen; set => SetProperty(ref _isRightPanelOpen, value); }
    public RelayCommand ToggleRightPanelCommand { get; }
    public RelayCommand RefreshCommand { get; }
    public RelayCommand CreateCheckpointCommand { get; }
    public RelayCommand ViewDetailsCommand { get; }
    public RelayCommand RestoreCommand { get; }
    public RelayCommand CopyIdCommand { get; }
    public ObservableCollection<GitHistoryItemViewModel> Commits { get; }
    public Func<string, Task>? RequestCopyText { get; set; }

    public string CheckpointMessage { get => _checkpointMessage; set => SetProperty(ref _checkpointMessage, value); }
    public string RestoreBranchName { get => _restoreBranchName; set => SetProperty(ref _restoreBranchName, value); }
    public string StatusText { get => _statusText; set => SetProperty(ref _statusText, value); }
    public string RepositoryStatusText { get => _repositoryStatusText; private set => SetProperty(ref _repositoryStatusText, value); }
    public string CurrentBranchText { get => _currentBranchText; private set => SetProperty(ref _currentBranchText, value); }
    public string HeadText { get => _headText; private set => SetProperty(ref _headText, value); }
    public string DirtyStateText { get => _dirtyStateText; private set => SetProperty(ref _dirtyStateText, value); }
    public string RepositoryReasonText { get => _repositoryReasonText; private set => SetProperty(ref _repositoryReasonText, value); }
    public string DiffSummaryText { get => _diffSummaryText; private set => SetProperty(ref _diffSummaryText, value); }
    public string DiffPreviewText { get => _diffPreviewText; private set => SetProperty(ref _diffPreviewText, value); }
    public bool HasRepositoryReason => !string.IsNullOrWhiteSpace(RepositoryReasonText);
    public bool HasDiffPreview => !string.IsNullOrWhiteSpace(DiffPreviewText);

    public GitHistoryItemViewModel? SelectedCommit
    {
        get => _selectedCommit;
        private set
        {
            if (SetProperty(ref _selectedCommit, value))
            {
                OnPropertyChanged(nameof(SelectedSummary));
                OnPropertyChanged(nameof(SelectedCommitId));
                OnPropertyChanged(nameof(SelectedKind));
                OnPropertyChanged(nameof(SelectedParents));
                OnPropertyChanged(nameof(HasSelection));
                ViewDetailsCommand.NotifyCanExecuteChanged();
                RestoreCommand.NotifyCanExecuteChanged();
                CopyIdCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public bool HasSelection => SelectedCommit is not null;
    public string SelectedSummary => SelectedCommit?.Summary ?? NoSelectionText;
    public string SelectedCommitId => SelectedCommit?.CommitId ?? _displayNames.Text("ui.common.none");
    public string SelectedKind => SelectedCommit?.KindText ?? _displayNames.Text("ui.common.none");
    public string SelectedParents => SelectedCommit is null || SelectedCommit.Parents.Count == 0
        ? _displayNames.Text("ui.common.none")
        : string.Join(", ", SelectedCommit.Parents);

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
    public string CommitLabel => _displayNames.Text("ui.git.commit_id");
    public string KindLabel => _displayNames.Text("ui.git.kind");
    public string ParentsLabel => _displayNames.Text("ui.git.parents");
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

    private async Task RefreshAsync()
    {
        try
        {
            await RefreshRepositoryStatusAsync().ConfigureAwait(true);
            var graph = await _backend.GetGitBranchGraphAsync().ConfigureAwait(true);
            Commits.Clear();
            foreach (var node in graph)
            {
                Commits.Add(new GitHistoryItemViewModel(
                    node.CommitId,
                    node.Summary,
                    node.Parents,
                    node.Refs,
                    KindText(node.Summary),
                    CtxViewDetailsText,
                    CtxRestoreText,
                    CtxCopyIdText,
                    SelectCommit,
                    ViewDetails,
                    RestoreCommitAsync,
                    CopyCommitIdAsync));
            }
            SelectedCommit = Commits.FirstOrDefault();
            if (SelectedCommit is not null)
            {
                SelectedCommit.IsSelected = true;
            }
            StatusText = Commits.Count == 0 ? EmptyText : $"{Commits.Count}";
        }
        catch
        {
            await RefreshHistoryFallbackAsync().ConfigureAwait(true);
        }
    }

    private async Task RefreshRepositoryStatusAsync()
    {
        try
        {
            var status = await _backend.GetGitRepositoryStatusAsync().ConfigureAwait(true);
            ApplyRepositoryStatus(status);
        }
        catch (Exception ex)
        {
            RepositoryStatusText = ex.Message;
            CurrentBranchText = _displayNames.Text("ui.common.none");
            HeadText = _displayNames.Text("ui.common.none");
            DirtyStateText = _displayNames.Text("ui.common.none");
            RepositoryReasonText = string.Empty;
            DiffSummaryText = _displayNames.Text("ui.common.none");
            DiffPreviewText = string.Empty;
            NotifyRepositoryVisibility();
        }
    }

    private async Task RefreshHistoryFallbackAsync()
    {
        try
        {
            await RefreshRepositoryStatusAsync().ConfigureAwait(true);
            var history = await _backend.GetGitHistoryAsync().ConfigureAwait(true);
            Commits.Clear();
            foreach (var commit in history)
            {
                Commits.Add(new GitHistoryItemViewModel(
                    commit.CommitId,
                    commit.Summary,
                    Array.Empty<string>(),
                    Array.Empty<string>(),
                    commit.CheckpointKind switch
                    {
                        "auto" => AutoKindText,
                        "manual" => ManualKindText,
                        _ => _displayNames.Text("ui.common.none"),
                    },
                    CtxViewDetailsText,
                    CtxRestoreText,
                    CtxCopyIdText,
                    SelectCommit,
                    ViewDetails,
                    RestoreCommitAsync,
                    CopyCommitIdAsync));
            }
            SelectedCommit = Commits.FirstOrDefault();
            if (SelectedCommit is not null)
            {
                SelectedCommit.IsSelected = true;
            }
            StatusText = Commits.Count == 0 ? EmptyText : $"{Commits.Count}";
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task CreateCheckpointAsync()
    {
        try
        {
            var checkpoint = await _backend.CreateCheckpointAsync(CheckpointMessage).ConfigureAwait(true);
            StatusText = checkpoint.Message;
            CheckpointMessage = string.Empty;
            await RefreshAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
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
        try
        {
            var branch = string.IsNullOrWhiteSpace(RestoreBranchName)
                ? $"restore-{commit.ShortCommitId}"
                : RestoreBranchName;
            if (!await ConfirmRestoreAsync(commit, branch).ConfigureAwait(true))
            {
                return;
            }
            if (!await _confirmProjectReload().ConfigureAwait(true))
            {
                return;
            }
            var report = await _backend.RestoreToNewBranchAsync(commit.CommitId, branch).ConfigureAwait(true);
            StatusText = report.NewBranch;
            RestoreBranchName = string.Empty;
            await RefreshAsync().ConfigureAwait(true);
            await _reloadProjectData().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private void ViewDetails(GitHistoryItemViewModel? commit)
    {
        StatusText = commit?.Summary ?? NoSelectionText;
    }

    private void ApplyRepositoryStatus(GitRepositoryStatus status)
    {
        RepositoryStatusText = status.Status switch
        {
            "healthy" => _displayNames.Text("ui.git.status.healthy"),
            "degraded" => _displayNames.Text("ui.git.status.degraded"),
            "not_repository" => _displayNames.Text("ui.git.status.not_repository"),
            "unavailable" => _displayNames.Text("ui.git.status.unavailable"),
            _ => status.Status,
        };
        CurrentBranchText = string.IsNullOrWhiteSpace(status.Branch) ? _displayNames.Text("ui.common.none") : status.Branch;
        HeadText = string.IsNullOrWhiteSpace(status.Head) ? _displayNames.Text("ui.common.none") : ShortHash(status.Head);
        DirtyStateText = status.Dirty ? _displayNames.Text("ui.git.dirty") : _displayNames.Text("ui.git.clean");
        RepositoryReasonText = status.Reason ?? string.Empty;
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

        if (RequestCopyText is not null)
        {
            await RequestCopyText(commit.CommitId).ConfigureAwait(true);
            StatusText = _displayNames.Text("ui.git.copied_commit_id");
            return;
        }

        StatusText = commit.CommitId;
    }

    private void SelectCommit(GitHistoryItemViewModel item)
    {
        foreach (var commit in Commits)
        {
            commit.IsSelected = commit == item;
        }
        SelectedCommit = item;
    }

    private string KindText(string summary)
    {
        if (summary.StartsWith("Checkpoint:", StringComparison.OrdinalIgnoreCase))
        {
            return AutoKindText;
        }
        if (summary.StartsWith("Archive:", StringComparison.OrdinalIgnoreCase))
        {
            return ManualKindText;
        }
        return _displayNames.Text("ui.common.none");
    }

    private async Task<bool> ConfirmRestoreAsync(GitHistoryItemViewModel commit, string branch)
    {
        var message = _displayNames.Format("ui.dialog.git.restore.message", new Dictionary<string, string>
        {
            ["commit"] = commit.ShortCommitId,
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
            CancelResultIndex = 1,
        };
        return await DialogService.Current.ConfirmAsync(dialog).ConfigureAwait(true) == 0;
    }
}

public sealed class GitHistoryItemViewModel : ViewModelBase
{
    private bool _isSelected;

    public GitHistoryItemViewModel(
        string commitId,
        string summary,
        IReadOnlyList<string> parents,
        IReadOnlyList<string> refs,
        string kindText,
        string viewDetailsText,
        string restoreText,
        string copyIdText,
        Action<GitHistoryItemViewModel> select,
        Action<GitHistoryItemViewModel> viewDetails,
        Func<GitHistoryItemViewModel, Task> restore,
        Func<GitHistoryItemViewModel, Task> copyId)
    {
        CommitId = commitId;
        Summary = summary;
        Parents = parents;
        Refs = refs;
        KindText = kindText;
        ViewDetailsText = viewDetailsText;
        RestoreText = restoreText;
        CopyIdText = copyIdText;
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
        });
        CopyIdCommand = new RelayCommand(() =>
        {
            select(this);
            _ = copyId(this);
        });
    }

    public string CommitId { get; }
    public string ShortCommitId => CommitId.Length <= 7 ? CommitId : CommitId[..7];
    public string Summary { get; }
    public IReadOnlyList<string> Parents { get; }
    public IReadOnlyList<string> Refs { get; }
    public string KindText { get; }
    public string RefsText => Refs.Count == 0 ? string.Empty : string.Join(", ", Refs);
    public string ViewDetailsText { get; }
    public string RestoreText { get; }
    public string CopyIdText { get; }
    public RelayCommand SelectCommand { get; }
    public RelayCommand ViewDetailsCommand { get; }
    public RelayCommand RestoreCommand { get; }
    public RelayCommand CopyIdCommand { get; }
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }
}
