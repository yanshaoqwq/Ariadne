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
    /// <summary>
    /// U182-M：无项目时空态里那颗「打开项目」按钮的动作。
    ///
    /// 宿主（MainWindowViewModel）注入的是**它自己那条打开项目链路**，
    /// 不是另写一套后端调用——离开守卫、目录预检、EnterProject 都在那条链上，
    /// 绕过去等于少一半流程。为 null 时按钮**不显示**（见
    /// <see cref="ShowOpenProjectAction"/>）：宁可没有按钮，也不要一颗点了没反应的，
    /// 那正是 U182-M 报的缺陷形态。
    /// </summary>
    private readonly Func<Task>? _requestOpenProject;

    /// <summary>U197-H：宿主查「别的页面有没有未保存改动」。每次现问，不缓存。</summary>
    private readonly Func<bool> _hasOtherUnsavedChanges;
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

    private string _otherPagesUnsavedText = string.Empty;

    private bool _hasOtherPagesUnsaved;
    private string _repositoryReasonText = string.Empty;
    private string _diffSummaryText = string.Empty;
    private string _diffPreviewText = string.Empty;
    private GitHistoryItemViewModel? _selectedCommit;
    private GitOperationState _operationState;
    private PageLoadState _loadState = PageLoadState.Loading;
    private string _errorText = string.Empty;
    private long _loadGeneration;

    public GitPageViewModel(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        Func<Task<bool>>? confirmProjectReload = null,
        Func<Task>? reloadProjectData = null,
        Func<string, bool, Task>? persistPanelState = null,
        Func<Task>? requestOpenProject = null,
        // U197-H：宿主提供「别的页面有没有未保存改动」。
        //
        // 为什么要注入而不是自己查：这是**内存态**，只有各页 VM 自己知道，
        // 而它们的持有者是 MainWindowViewModel（`HasCachedUnsavedChanges`）。
        // Git 页自己无从得知，而不告诉作者的后果见 `RefreshOtherPagesUnsavedHint`。
        //
        // 默认返回 false：单测直接 new 这个 VM 时不该凭空出现该提示。
        Func<bool>? hasOtherUnsavedChanges = null)
    {
        _displayNames = displayNames;
        _backend = backend;
        _confirmProjectReload = confirmProjectReload ?? (() => Task.FromResult(true));
        _reloadProjectData = reloadProjectData ?? (() => Task.CompletedTask);
        _persistPanelState = persistPanelState;
        _requestOpenProject = requestOpenProject;
        _hasOtherUnsavedChanges = hasOtherUnsavedChanges ?? (() => false);
        Commits = new ObservableCollection<GitHistoryItemViewModel>();
        ToggleRightPanelCommand = new RelayCommand(() => _ = ToggleRightPanelAsync());
        RefreshCommand = new RelayCommand(() => _ = RefreshAsync(), CanStartOperation);
        CreateCheckpointCommand = new RelayCommand(() => _ = CreateCheckpointAsync(), CanStartOperation);
        ViewDetailsCommand = new RelayCommand(() => ViewDetails(SelectedCommit), () => HasSelection);
        RestoreCommand = new RelayCommand(
            () => _ = RestoreSelectedAsync(),
            () => HasSelection && CanStartOperation());
        CopyIdCommand = new RelayCommand(() => _ = CopyCommitIdAsync(SelectedCommit), () => HasSelection);
        // U182-M：错误态的重试不能复用 RefreshCommand——后者的 CanExecute 里有
        // `_backend.HasProjectRoot`，而「无项目」正是这颗按钮要走的那条分支。
        // 这里是「打开项目」，与重试是两件事，各自一个命令。
        OpenProjectCommand = new RelayCommand(() => _ = RequestOpenProjectAsync(), () => _requestOpenProject is not null);
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
    public RelayCommand OpenProjectCommand { get; }
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
        set
        {
            if (SetProperty(ref _statusText, value))
            {
                OnPropertyChanged(nameof(HasStatusText));
            }
        }
    }

    public bool HasStatusText => !string.IsNullOrWhiteSpace(StatusText);

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

    /// <summary>
    /// U197-H：「另有页面存在未保存改动」这句提示。
    ///
    /// # 这条要解决的误读
    ///
    /// `DirtyStateText` 说的是**磁盘上**有没有未提交改动，而作品页的
    /// `HasUnsavedChanges` 说的是**内存里**有没有未保存改动 ——
    /// 两个 dirty 都正确，但并列呈现时误导：
    /// 作者在作品页写了 3000 字没按 Ctrl+S，切到 Git 页看到「无未提交变更」，
    /// 会理解成「干净 = 我的东西都在版本控制里了」。而那 3000 字连磁盘都没到。
    ///
    /// 与 U183（正文在 Ctrl+S 之前只存在于内存）叠加时后果最重。
    /// </summary>
    public string OtherPagesUnsavedText
    {
        get => _otherPagesUnsavedText;
        private set => SetProperty(ref _otherPagesUnsavedText, value);
    }

    /// <summary>
    /// 提示是否可见。
    ///
    /// ⚠️ **只在「磁盘干净 + 内存脏」时出现**，这是本条的关键：
    /// 磁盘也脏时 `DirtyStateText` 已经在说「有未提交变更」，
    /// 再补一句「另有未保存改动」只是噪音 —— 而作者此时**不会**误以为东西已入库。
    /// 误读只发生在屏上写着「干净」的那一刻，提示也就只该出现在那一刻。
    /// </summary>
    public bool HasOtherPagesUnsaved => _hasOtherPagesUnsaved;

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

    /// <summary>
    /// 是否已拿到仓库信息。未打开项目时这些字段全为空，
    /// 右栏若照常渲染「仓库状态/当前分支/当前 HEAD…」标签列就会变成
    /// 一张没填完的表单，故用此标志整块隐藏。
    /// </summary>
    public bool HasRepositoryInfo => !string.IsNullOrWhiteSpace(RepositoryStatusText);

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

    /// <summary>
    /// U182-K：页面加载态。**照搬 <see cref="RunLogPageViewModel"/> 的
    /// <see cref="PageLoadState"/>，不新造一套**——本项目已有这套机制（日志页、作品页），
    /// Git 页此前从未接入。
    /// </summary>
    public PageLoadState LoadState
    {
        get => _loadState;
        private set
        {
            if (SetProperty(ref _loadState, value))
            {
                NotifyLoadStateDerived();
            }
        }
    }

    public string ErrorText
    {
        get => _errorText;
        private set => SetProperty(ref _errorText, value);
    }

    /// <summary>
    /// U182-K：空态判据由 <c>Commits.Count == 0</c> 改成**看状态**。
    ///
    /// 原缺陷：`Commits.Count == 0` 是唯一判据 ⇒ 后端报错时列表为空，
    /// 于是有 200 个存档点的项目被告知「你没有存档」；刷新期间也是同一画面。
    /// 「空」是一个**结论**（问过后端、确实没有），不是「手上暂时没数据」。
    /// </summary>
    public bool IsCommitListEmpty => _loadState is PageLoadState.Empty or PageLoadState.IdleNeedProject;
    public bool IsLoading => _loadState == PageLoadState.Loading;
    public bool IsError => _loadState is PageLoadState.Error or PageLoadState.ContentError;

    /// <summary>整页错误页：手上一条存档都没有，只能给「出错了 + 重试」。</summary>
    public bool IsStandaloneError => _loadState == PageLoadState.Error;

    /// <summary>
    /// 内容内错误条：**已有存档时出错不清内容**，列表留着 + 顶部一条错误提示。
    /// 照抄 RunLog 的 `Logs.Count > 0 ? ContentError : Error` 区分——
    /// 把 200 行历史换成一张整页错误页，是拿用户手上唯一的诊断材料去换一句道歉。
    /// </summary>
    public bool IsContentError => _loadState == PageLoadState.ContentError;
    public bool ShowEmpty => IsCommitListEmpty && !IsLoading && !IsError;
    public bool ShowContent => HasCommits && _loadState is PageLoadState.Content or PageLoadState.ContentError;

    /// <summary>
    /// U182-M：只有「没打开项目」这一种空态给可点动作。
    /// 「项目里还没有存档」时该点的是右栏的「创建存档」，不是打开项目。
    /// </summary>
    public bool ShowOpenProjectAction =>
        _loadState == PageLoadState.IdleNeedProject && _requestOpenProject is not null;

    public string EmptyTitle => _loadState == PageLoadState.IdleNeedProject
        ? _displayNames.Text("ui.empty.need_project.title")
        : _displayNames.Text("ui.empty.git.title");
    public string EmptyHint => _loadState == PageLoadState.IdleNeedProject
        ? _displayNames.Text("ui.empty.need_project.hint")
        : _displayNames.Text("ui.empty.git.hint");
    public string ErrorTitle => _displayNames.Text("ui.git.error.title");
    public string LoadingText => _displayNames.Text("ui.git.loading");
    public string OpenProjectText => _displayNames.Text("ui.layout.open_project");
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
        // U182-K：无项目这条路必须**走在 TryBeginOperation 之前**。
        // `CanStartOperation()` 自带 `_backend.HasProjectRoot` 判定，无项目时它返回 false ⇒
        // 整个方法早返回、ClearProjectState 永远不执行，页面会一直停在初始的 Loading 态。
        // （原实现靠 `Commits.Count == 0` 兜住，所以看不出这一点；改成看状态后必须显式处理。）
        if (!_backend.HasProjectRoot)
        {
            ClearProjectState();
            return;
        }
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

        // U182-K：进入加载态**在发起后端调用之前**。放到 await 之后就等于没有加载态——
        // 加载态要覆盖的正是「请求在路上」这段时间。
        LoadState = PageLoadState.Loading;
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
        ErrorText = string.Empty;
        RepositoryStatusText = string.Empty;
        CurrentBranchText = _displayNames.Text("ui.common.none");
        HeadText = _displayNames.Text("ui.common.none");
        DirtyStateText = _displayNames.Text("ui.common.none");
        RepositoryReasonText = string.Empty;
        DiffSummaryText = _displayNames.Text("ui.common.none");
        DiffPreviewText = string.Empty;
        // U197-H：没打开项目时**不显示**「另有页面未保存」——
        // 那句话的前提是「本项目磁盘干净」，而此刻压根没有本项目。
        ClearOtherPagesUnsavedHint();
        // U182-K/M：没打开项目不是「空」也不是「错」，是第三种状态——
        // 它的下一步是「打开项目」，而 Empty 的下一步是「创建存档」。
        LoadState = PageLoadState.IdleNeedProject;
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
            // U197-H：读仓库状态失败时不显示这句 ——
            // 「磁盘干净」这个前提没拿到，凭空说「另有未保存」会让作者
            // 以为提示指的是刚才那个错误。
            ClearOtherPagesUnsavedHint();
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
            // U182-K 的原缺陷正在这里：此前只写 StatusText、**不标记状态**，
            // 于是 Commits 仍为空 ⇒ 空态照常渲染，用户读到的是「你没有存档」。
            // 现在按「手上还有没有内容」分两种呈现，与 RunLog 同一区分。
            ErrorText = UserFacingError.Format(ex, _displayNames);
            StatusText = ErrorText;
            LoadState = Commits.Count > 0 ? PageLoadState.ContentError : PageLoadState.Error;
            // 刻意不 Commits.Clear()：上一次成功的快照是用户手上唯一的诊断材料。
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
        // U182-K：加载成功了才有资格说「空」——此时确实问过后端且它说没有。
        ErrorText = string.Empty;
        LoadState = Commits.Count == 0 ? PageLoadState.Empty : PageLoadState.Content;
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
        // U197-H：磁盘 dirty 判完才能决定要不要补「另有页面未保存」那句
        // （只在磁盘干净时补，理由见 HasOtherPagesUnsaved 的注释）。
        RefreshOtherPagesUnsavedHint(status.Dirty);
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
        OnPropertyChanged(nameof(HasRepositoryInfo));
    }

    /// <summary>
    /// U197-H：按「磁盘是否脏」决定要不要显示「另有页面存在未保存改动」。
    ///
    /// # 为什么每次现问宿主，而不订阅变化
    ///
    /// 内存 dirty 会随作者每一次敲键改变，订阅它等于把 Git 页挂到作品页的
    /// 每次按键上。而这句提示只在**刷新仓库状态时**才需要正确——
    /// 那是作者切到 Git 页、或点刷新的时刻，正是他会读这行字的时刻。
    ///
    /// ⚠️ 代价说清：作者停在 Git 页不动、同时别的页面变脏（例如自动化流程
    /// 在后台改了正文），这句提示不会自己冒出来。取舍理由是本项目
    /// **全应用无 DispatcherTimer**（U194-C 已复核），
    /// 为一句提示引入轮询与它的价值不相称；而 U197-A 那条事件驱动的刷新
    /// 落地后，决议/运行结束等时机会顺带带上这里。
    /// </summary>
    private void RefreshOtherPagesUnsavedHint(bool diskDirty)
    {
        // 磁盘脏时不显示：DirtyStateText 已经在说「有未提交变更」，
        // 此刻作者不会误以为东西已入库 ⇒ 再补一句只是噪音。
        var show = !diskDirty && _hasOtherUnsavedChanges();
        if (!show)
        {
            ClearOtherPagesUnsavedHint();
            return;
        }

        OtherPagesUnsavedText = _displayNames.Text("ui.git.other_pages_unsaved");
        SetOtherPagesUnsavedVisible(true);
    }

    private void ClearOtherPagesUnsavedHint()
    {
        OtherPagesUnsavedText = string.Empty;
        SetOtherPagesUnsavedVisible(false);
    }

    private void SetOtherPagesUnsavedVisible(bool value)
    {
        if (_hasOtherPagesUnsaved == value)
        {
            return;
        }
        _hasOtherPagesUnsaved = value;
        OnPropertyChanged(nameof(HasOtherPagesUnsaved));
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
        OnPropertyChanged(nameof(ShowEmpty));
        OnPropertyChanged(nameof(ShowContent));
        OnPropertyChanged(nameof(EmptyTitle));
        OnPropertyChanged(nameof(EmptyHint));
        NotifySelectionCommands();
    }

    /// <summary>
    /// U182-K：状态跃迁要通知的全部派生属性。
    ///
    /// 单独收成一个方法而不是在 setter 里逐条列：这些属性有 9 个，
    /// 漏通知一条的表现是「状态已经对了但界面还停在上一屏」——
    /// 与「状态没跃迁」在屏幕上完全同形，极难归因。
    /// </summary>
    private void NotifyLoadStateDerived()
    {
        OnPropertyChanged(nameof(IsCommitListEmpty));
        OnPropertyChanged(nameof(IsLoading));
        OnPropertyChanged(nameof(IsError));
        OnPropertyChanged(nameof(IsStandaloneError));
        OnPropertyChanged(nameof(IsContentError));
        OnPropertyChanged(nameof(ShowEmpty));
        OnPropertyChanged(nameof(ShowContent));
        OnPropertyChanged(nameof(ShowOpenProjectAction));
        OnPropertyChanged(nameof(EmptyTitle));
        OnPropertyChanged(nameof(EmptyHint));
    }

    /// <summary>
    /// U182-M：把「点左上角项目名」这句死文案换成一颗真能点的按钮。
    ///
    /// 走宿主注入的委托（MainWindowViewModel 的 OpenProjectCommand 同一条链路），
    /// 而不是自己 `_backend.OpenProjectAsync`：那条链上还有离开守卫、目录预检、
    /// EnterProject 与最近项目登记，跳过任意一项都会留下半开的项目状态。
    /// </summary>
    private async Task RequestOpenProjectAsync()
    {
        if (_requestOpenProject is null)
        {
            return;
        }
        try
        {
            await _requestOpenProject().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
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
