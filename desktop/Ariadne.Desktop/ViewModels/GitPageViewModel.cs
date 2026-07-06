using System.Collections.ObjectModel;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class GitPageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private bool _isRightPanelOpen = true;
    private string _checkpointMessage = string.Empty;
    private string _restoreBranchName = string.Empty;
    private string _statusText = string.Empty;
    private GitHistoryItemViewModel? _selectedCommit;

    public GitPageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        Commits = new ObservableCollection<GitHistoryItemViewModel>();
        ToggleRightPanelCommand = new RelayCommand(() => IsRightPanelOpen = !IsRightPanelOpen);
        RefreshCommand = new RelayCommand(() => _ = RefreshAsync());
        CreateCheckpointCommand = new RelayCommand(() => _ = CreateCheckpointAsync());
        ViewDetailsCommand = new RelayCommand(() => ViewDetails(SelectedCommit));
        RestoreCommand = new RelayCommand(() => _ = RestoreSelectedAsync());
        CopyIdCommand = new RelayCommand(() => _ = CopyCommitIdAsync(SelectedCommit));
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
    public string CtxCreateCheckpointText => _displayNames.Text("ui.git.context.create_checkpoint");
    public string CtxViewDetailsText => _displayNames.Text("ui.git.context.view_details");
    public string CtxRestoreText => _displayNames.Text("ui.git.context.restore");
    public string CtxCopyIdText => _displayNames.Text("ui.git.context.copy_id");

    private async Task RefreshAsync()
    {
        try
        {
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
                    CtxCreateCheckpointText,
                    CtxRestoreText,
                    CtxCopyIdText,
                    SelectCommit,
                    ViewDetails,
                    CreateCheckpointFromItemAsync,
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

    private async Task RefreshHistoryFallbackAsync()
    {
        try
        {
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
                    CtxCreateCheckpointText,
                    CtxRestoreText,
                    CtxCopyIdText,
                    SelectCommit,
                    ViewDetails,
                    CreateCheckpointFromItemAsync,
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
            var report = await _backend.RestoreToNewBranchAsync(commit.CommitId, branch).ConfigureAwait(true);
            StatusText = report.NewBranch;
            RestoreBranchName = string.Empty;
            await RefreshAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task CreateCheckpointFromItemAsync(GitHistoryItemViewModel _)
    {
        await CreateCheckpointAsync().ConfigureAwait(true);
    }

    private void ViewDetails(GitHistoryItemViewModel? commit)
    {
        StatusText = commit?.Summary ?? NoSelectionText;
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
        string createCheckpointText,
        string restoreText,
        string copyIdText,
        Action<GitHistoryItemViewModel> select,
        Action<GitHistoryItemViewModel> viewDetails,
        Func<GitHistoryItemViewModel, Task> createCheckpoint,
        Func<GitHistoryItemViewModel, Task> restore,
        Func<GitHistoryItemViewModel, Task> copyId)
    {
        CommitId = commitId;
        Summary = summary;
        Parents = parents;
        Refs = refs;
        KindText = kindText;
        ViewDetailsText = viewDetailsText;
        CreateCheckpointText = createCheckpointText;
        RestoreText = restoreText;
        CopyIdText = copyIdText;
        SelectCommand = new RelayCommand(() => select(this));
        ViewDetailsCommand = new RelayCommand(() =>
        {
            select(this);
            viewDetails(this);
        });
        CreateCheckpointCommand = new RelayCommand(() =>
        {
            select(this);
            _ = createCheckpoint(this);
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
    public string CreateCheckpointText { get; }
    public string RestoreText { get; }
    public string CopyIdText { get; }
    public RelayCommand SelectCommand { get; }
    public RelayCommand ViewDetailsCommand { get; }
    public RelayCommand CreateCheckpointCommand { get; }
    public RelayCommand RestoreCommand { get; }
    public RelayCommand CopyIdCommand { get; }
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }
}
