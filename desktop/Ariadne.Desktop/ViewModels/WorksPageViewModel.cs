using System.Collections.ObjectModel;
using System.Text.Json;
using Avalonia.Controls;
using AvaloniaEdit.Document;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class WorksPageViewModel : ViewModelBase, IUnsavedChangesGuard, IProjectDataReloadable, IUiPreferencesAware, ILocalizedUiAware
{
    private const string ProjectAiConversationId = "works";
    private const string RightPanelPreferenceKey = "works.right_panel";
    private const double MinRightPanelWidth = 280;
    private const double MaxRightPanelWidth = 520;
    private const int TargetDocumentBlockSize = 4_000;
    private const int HardDocumentBlockSize = 6_000;

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private readonly Func<string, bool, Task>? _persistPanelState;
    private readonly ProjectAutomationState _projectAutomation;
    private readonly ContinuousDocumentBuffer _editorBuffer = new();
    private bool _isRightPanelOpen = true;
    private bool _isProjectPanelVisible = true;
    private GridLength _rightPanelColumnWidth = new(320);
    private bool _isNavTreeTab = true;
    private bool _isImportPanelOpen;
    private string _statusText = string.Empty;
    private string _projectAiMessage = string.Empty;
    private string _projectAiAnswer;
    private readonly List<ProjectAiChatMessage> _projectAiHistory = new();
    private long? _projectAiConversationRevision;
    private string _quickEditInstruction = string.Empty;
    private string _quickEditDiff = string.Empty;
    private string _exportFormat = "markdown";
    private string _currentDocumentId = string.Empty;
    private string _currentDocumentPath = string.Empty;
    private string? _currentDocumentVersion;
    private string _documentTitle;
    private string _importChapterId = string.Empty;
    private string _importChapterTitle = string.Empty;
    private decimal? _importOrder = 0m;
    private string _importSourcePath = string.Empty;
    private string _importTargetPath = string.Empty;
    private string _importProjectRoot = string.Empty;
    private bool _allowImportOverwrite;
    private string _savedSnapshot = string.Empty;
    private bool _hasUnsavedChanges;
    private bool _suppressDirtyTracking;
    private bool _documentDirty;
    private int _documentCharacterCount;
    private bool _isEditMode;
    private QuickEditSession? _pendingQuickEdit;
    private QuickEditUndoState? _quickEditUndo;
    private CancellationTokenSource? _quickEditGenerationCts;
    private long _quickEditGeneration;
    private bool _isQuickEditGenerating;
    private CancellationTokenSource? _summaryLoadCts;
    private long _summaryLoadGeneration;
    private bool _isSummaryLoading;
    private string _summaryErrorText = string.Empty;
    private string _currentSummaryChapterId = string.Empty;
    private string? _chapterSummaryText;
    private string? _summaryStageId;
    private string? _stageSummaryText;
    private string _activeSummarySegmentText = string.Empty;
    private CancellationTokenSource? _worksTreeLoadCts;
    private long _worksTreeLoadGeneration;
    private WorksTreeLoadState _worksTreeState = WorksTreeLoadState.Empty;
    private string _worksTreeErrorText = string.Empty;
    private readonly HashSet<string> _expandedWorksTreeNodeIds = new(StringComparer.Ordinal);
    private bool _worksTreeExpansionInitialized;
    private string _worksTreeSearchText = string.Empty;
    private WorksTreeItemViewModel? _selectedWorksTreeNode;
    private WorksTreeItemViewModel? _currentWorksTreeNode;
    private bool _suppressWorksTreeSelectionNavigation;
    private CancellationTokenSource? _documentLoadCts;
    private long _documentLoadGeneration;
    private long _documentEditRevision;
    private bool _isDocumentSaving;
    private bool _isDocumentLoading;
    private string _documentLoadingTarget = string.Empty;

    private enum WorksTreeLoadState
    {
        Loading,
        Content,
        Empty,
        Error,
    }

    public WorksPageViewModel(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        Func<string, bool, Task>? persistPanelState = null,
        ProjectAutomationState? projectAutomation = null)
    {
        _displayNames = displayNames;
        _backend = backend;
        _persistPanelState = persistPanelState;
        _projectAutomation = projectAutomation ?? new ProjectAutomationState(displayNames, backend);
        _editorBuffer.TextChanged += OnEditorDocumentTextChanged;
        _projectAiAnswer = displayNames.Text("ui.works.project_ai.empty");
        _documentTitle = displayNames.Text("ui.works.no_document_selected");
        WorksTreeRoots = new ObservableCollection<WorksTreeItemViewModel>();
        VisibleWorksTreeRoots = new ObservableCollection<WorksTreeItemViewModel>();
        DocumentBlocks = new ObservableCollection<DocumentBlockViewModel>();
        ProjectAiBubbles = new ObservableCollection<ChatBubbleViewModel>();
        SummarySegments = new ObservableCollection<WorksSummarySegmentItemViewModel>();
        SummaryEvents = new ObservableCollection<WorksSummaryDetailItemViewModel>();
        SummaryChanges = new ObservableCollection<WorksSummaryDetailItemViewModel>();
        SummaryForeshadowing = new ObservableCollection<WorksSummaryDetailItemViewModel>();
        SummaryConfirmations = new ObservableCollection<WorksSummaryDetailItemViewModel>();
        ToggleRightPanelCommand = new RelayCommand(() => _ = ToggleRightPanelAsync(), () => IsRightPanelToggleVisible);
        ShowNavTreeCommand = new RelayCommand(() => IsNavTreeTab = true);
        ShowProjectAiCommand = new RelayCommand(() => IsNavTreeTab = false);
        OpenImportPanelCommand = new RelayCommand(OpenImportPanel);
        ToggleImportPanelCommand = new RelayCommand(ToggleImportPanel);
        BrowseImportSourceCommand = new RelayCommand(() => _ = BrowseImportSourceAsync());
        ImportCommand = new RelayCommand(() => _ = ImportChapterAsync(), CanImportChapter);
        ExportCommand = new RelayCommand(() => _ = ExportAsync(), () => WorksTreeRoots.Count > 0);
        SaveCommand = new RelayCommand(() => _ = SaveAsync(), () => HasCurrentDocument && !IsDocumentSaving);
        RetryWorksTreeCommand = new RelayCommand(() => _ = LoadWorksTreeAsync(), () => IsWorksTreeError && !IsWorksTreeLoading);
        ReadModeCommand = new RelayCommand(() => IsEditMode = false);
        EditModeCommand = new RelayCommand(() => IsEditMode = true);
        CopyCommand = new RelayCommand(() => RequestEditorCopy?.Invoke());
        SelectAllCommand = new RelayCommand(() => RequestEditorSelectAll?.Invoke());
        OpenQuickEditCommand = new RelayCommand(OpenQuickEdit, CanOpenQuickEdit);
        QuickAiCommand = new RelayCommand(() =>
        {
            IsEditMode = true;
            _ = QuickEditAsync();
        }, CanGenerateQuickEdit);
        InsertOutlineCommand = new RelayCommand(InsertOutlineReference, () => HasCurrentDocument);
        ToggleEditCommand = new RelayCommand(() => IsEditMode = !IsEditMode);
        SendProjectAiCommand = new RelayCommand(() => _ = SendProjectAiAsync(), CanSendProjectAi);
        ApplyQuickEditCommand = new RelayCommand(ApplyQuickEdit, CanApplyQuickEdit);
        UndoQuickEditCommand = new RelayCommand(UndoQuickEdit, CanUndoQuickEdit);
        ExportFormats = new ObservableCollection<ExportFormatOption>
        {
            new("markdown", displayNames.Text("ui.works.export_format.markdown")),
            new("epub", displayNames.Text("ui.works.export_format.epub")),
            new("pdf", displayNames.Text("ui.works.export_format.pdf")),
        };
        CaptureSnapshot();
    }

    public string ToggleRightPanelText => _displayNames.Text("ui.action.toggle_right_panel");

    public void RefreshLocalizedUi()
    {
        ExportFormats[0] = new ExportFormatOption("markdown", _displayNames.Text("ui.works.export_format.markdown"));
        ExportFormats[1] = new ExportFormatOption("epub", _displayNames.Text("ui.works.export_format.epub"));
        ExportFormats[2] = new ExportFormatOption("pdf", _displayNames.Text("ui.works.export_format.pdf"));
        OnPropertyChanged(string.Empty);
    }
    public ProjectAutomationState ProjectAutomation => _projectAutomation;

    /// 右侧栏开合状态；开合入口由三页共用的边缘控制器承载。
    public bool IsRightPanelOpen
    {
        get => _isRightPanelOpen;
        set
        {
            if (SetProperty(ref _isRightPanelOpen, value))
            {
                OnPropertyChanged(nameof(RightPanelSplitterWidth));
                OnPropertyChanged(nameof(RightPanelColumnWidth));
                OnPropertyChanged(nameof(IsRightPanelVisible));
            }
        }
    }

    public bool IsProjectPanelVisible
    {
        get => _isProjectPanelVisible;
        private set
        {
            if (SetProperty(ref _isProjectPanelVisible, value))
            {
                OnPropertyChanged(nameof(IsRightPanelVisible));
                OnPropertyChanged(nameof(IsRightPanelToggleVisible));
                OnPropertyChanged(nameof(RightPanelSplitterWidth));
                OnPropertyChanged(nameof(RightPanelColumnWidth));
                ToggleRightPanelCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public bool IsRightPanelToggleVisible => IsProjectPanelVisible || IsImportPanelOpen;

    public bool IsRightPanelVisible => IsRightPanelToggleVisible && IsRightPanelOpen;

    public RelayCommand ToggleRightPanelCommand { get; }

    public GridLength RightPanelSplitterWidth => IsRightPanelVisible ? new GridLength(4) : new GridLength(0);

    public GridLength RightPanelColumnWidth
    {
        get => IsRightPanelVisible ? _rightPanelColumnWidth : new GridLength(0);
        set
        {
            if (!IsRightPanelVisible)
            {
                return;
            }
            var normalized = NormalizeRightPanelWidth(value);
            if (!_rightPanelColumnWidth.Equals(normalized))
            {
                _rightPanelColumnWidth = normalized;
                OnPropertyChanged();
            }
        }
    }

    public void ApplyUiPreferences(UiPreferences preferences)
    {
        IsProjectPanelVisible = preferences.ProjectPanelVisible;
        var isOpen = preferences.PanelStates?.TryGetValue(RightPanelPreferenceKey, out var savedOpen) == true
            ? savedOpen
            : preferences.ProjectPanelVisible;
        IsRightPanelOpen = IsImportPanelOpen || isOpen;
    }

    private async Task ToggleRightPanelAsync()
    {
        if (!IsRightPanelToggleVisible)
        {
            return;
        }
        IsRightPanelOpen = !IsRightPanelOpen;
        if (!IsProjectPanelVisible || _persistPanelState is null)
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

    /// 右栏标签：true=导航树（含章节树/大纲），false=项目 AI。
    public bool IsNavTreeTab
    {
        get => _isNavTreeTab;
        set
        {
            if (SetProperty(ref _isNavTreeTab, value))
            {
                OnPropertyChanged(nameof(IsProjectAiTab));
            }
        }
    }

    public bool IsProjectAiTab => !_isNavTreeTab;

    public bool IsImportPanelOpen
    {
        get => _isImportPanelOpen;
        set
        {
            if (SetProperty(ref _isImportPanelOpen, value))
            {
                OnPropertyChanged(nameof(IsRightPanelToggleVisible));
                OnPropertyChanged(nameof(IsRightPanelVisible));
                OnPropertyChanged(nameof(RightPanelSplitterWidth));
                OnPropertyChanged(nameof(RightPanelColumnWidth));
                ToggleRightPanelCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public RelayCommand ShowNavTreeCommand { get; }

    public RelayCommand ShowProjectAiCommand { get; }

    public RelayCommand OpenImportPanelCommand { get; }

    public RelayCommand ToggleImportPanelCommand { get; }

    public RelayCommand BrowseImportSourceCommand { get; }

    /// <summary>View 注入：挑选导入源文件路径。</summary>
    public Func<Task<string?>>? PickImportSourceFile { get; set; }

    /// <summary>View 注入：在文件管理器中打开目录。</summary>
    public Func<string, Task>? OpenFolderInShell { get; set; }

    public RelayCommand ImportCommand { get; }

    public RelayCommand ExportCommand { get; }

    public RelayCommand SaveCommand { get; }

    public RelayCommand RetryWorksTreeCommand { get; }

    public RelayCommand ReadModeCommand { get; }

    public RelayCommand EditModeCommand { get; }

    public RelayCommand CopyCommand { get; }

    public RelayCommand SelectAllCommand { get; }

    public RelayCommand OpenQuickEditCommand { get; }

    public RelayCommand QuickAiCommand { get; }

    public RelayCommand InsertOutlineCommand { get; }

    public RelayCommand ToggleEditCommand { get; }

    public RelayCommand SendProjectAiCommand { get; }

    public RelayCommand ApplyQuickEditCommand { get; }
    public RelayCommand UndoQuickEditCommand { get; }
    public Action? RequestEditorCopy { get; set; }
    public Action? RequestEditorSelectAll { get; set; }
    public Func<EditorTextSelection>? RequestEditorSelection { get; set; }

    /// <summary>View 注入：把全局 UTF-16 正文范围滚动并选中到连续编辑器。</summary>
    public Action<int, int>? RequestRevealEditorRange { get; set; }

    /// <summary>View 注入：快捷改写面板出现后把焦点交给说明输入框。</summary>
    public Action? RequestFocusQuickEditInstruction { get; set; }

    /// <summary>View 注册：文档切换/打开时清空粘性选区，避免旧索引打到新正文。</summary>
    public Action? ClearStickyEditorSelection { get; set; }

    /// <summary>后端作品树的单一层级身份源；显示筛选只复用这些节点实例。</summary>
    public ObservableCollection<WorksTreeItemViewModel> WorksTreeRoots { get; }

    /// <summary>按标题搜索后的根投影；子层投影由每个节点的 VisibleChildren 维护。</summary>
    public ObservableCollection<WorksTreeItemViewModel> VisibleWorksTreeRoots { get; }

    public WorksTreeItemViewModel? SelectedWorksTreeNode
    {
        get => _selectedWorksTreeNode;
        set
        {
            if (!SetProperty(ref _selectedWorksTreeNode, value)
                || _suppressWorksTreeSelectionNavigation
                || value is null
                || !value.CanOpen)
            {
                return;
            }

            _ = LoadDocumentAsync(value);
        }
    }

    public string WorksTreeSearchText
    {
        get => _worksTreeSearchText;
        set
        {
            if (SetProperty(ref _worksTreeSearchText, value ?? string.Empty))
            {
                ApplyWorksTreeSearch();
            }
        }
    }

    public bool IsWorksTreeSearchActive => !string.IsNullOrWhiteSpace(WorksTreeSearchText);
    public bool ShowWorksTreeSearchEmpty => _worksTreeState == WorksTreeLoadState.Content
                                            && IsWorksTreeSearchActive
                                            && VisibleWorksTreeRoots.Count == 0;

    public ObservableCollection<DocumentBlockViewModel> DocumentBlocks { get; }
    public bool HasDocumentBlocks => DocumentBlocks.Count > 0;
    public TextDocument EditorDocument => _editorBuffer.Document;
    public ObservableCollection<ChatBubbleViewModel> ProjectAiBubbles { get; }
    public bool HasProjectAiBubbles => ProjectAiBubbles.Count > 0;
    public ObservableCollection<WorksSummarySegmentItemViewModel> SummarySegments { get; }
    public ObservableCollection<WorksSummaryDetailItemViewModel> SummaryEvents { get; }
    public ObservableCollection<WorksSummaryDetailItemViewModel> SummaryChanges { get; }
    public ObservableCollection<WorksSummaryDetailItemViewModel> SummaryForeshadowing { get; }
    public ObservableCollection<WorksSummaryDetailItemViewModel> SummaryConfirmations { get; }

    public ObservableCollection<ExportFormatOption> ExportFormats { get; }

    public bool IsWorksTreeLoading => _worksTreeState == WorksTreeLoadState.Loading;

    public bool IsWorksTreeError => _worksTreeState == WorksTreeLoadState.Error;

    public bool IsWorksTreeEmpty => _worksTreeState == WorksTreeLoadState.Empty;

    public bool IsWorksTreeContent => _worksTreeState == WorksTreeLoadState.Content;

    public string WorksTreeLoadingText => _displayNames.Text("ui.works.loading_tree");

    public string WorksTreeErrorText => _worksTreeErrorText;

    public string RetryWorksTreeText => _displayNames.Text("ui.works.retry_tree");

    public string WorksTreeSearchPlaceholder => _displayNames.Text("ui.works.tree_search_placeholder");
    public string WorksTreeSearchName => _displayNames.Text("ui.works.tree_search_name");
    public string WorksTreeSearchEmptyText => _displayNames.Text("ui.works.tree_search_empty");

    /// <summary>有作品树但未选文档：只显示一处空态（U72）。</summary>
    public bool ShowNoDocumentEmpty => _worksTreeState == WorksTreeLoadState.Content && !HasCurrentDocument;

    /// <summary>已选文档时才渲染文档头与正文面。</summary>
    public bool ShowDocumentChrome => _worksTreeState == WorksTreeLoadState.Content && HasCurrentDocument;

    public bool IsSummaryLoading
    {
        get => _isSummaryLoading;
        private set
        {
            if (SetProperty(ref _isSummaryLoading, value))
            {
                NotifySummaryStateChanged();
            }
        }
    }

    public string SummaryErrorText
    {
        get => _summaryErrorText;
        private set
        {
            if (SetProperty(ref _summaryErrorText, value))
            {
                NotifySummaryStateChanged();
            }
        }
    }

    public string CurrentSummaryChapterId => _currentSummaryChapterId;

    public string? ChapterSummaryText
    {
        get => _chapterSummaryText;
        private set
        {
            if (SetProperty(ref _chapterSummaryText, value))
            {
                NotifySummaryStateChanged();
            }
        }
    }

    public string? SummaryStageId
    {
        get => _summaryStageId;
        private set
        {
            if (SetProperty(ref _summaryStageId, value))
            {
                OnPropertyChanged(nameof(SummaryStageHeading));
                NotifySummaryStateChanged();
            }
        }
    }

    public string? StageSummaryText
    {
        get => _stageSummaryText;
        private set
        {
            if (SetProperty(ref _stageSummaryText, value))
            {
                NotifySummaryStateChanged();
            }
        }
    }

    public bool HasSummaryContext => !string.IsNullOrWhiteSpace(_currentSummaryChapterId);
    public bool HasSummaryError => !string.IsNullOrWhiteSpace(SummaryErrorText);
    public bool HasChapterSummary => !string.IsNullOrWhiteSpace(ChapterSummaryText);
    public bool HasStageSummary => !string.IsNullOrWhiteSpace(StageSummaryText);
    public bool HasSummarySegments => SummarySegments.Count > 0;
    public bool HasSummaryEvents => SummaryEvents.Count > 0;
    public bool HasSummaryChanges => SummaryChanges.Count > 0;
    public bool HasSummaryForeshadowing => SummaryForeshadowing.Count > 0;
    public bool HasSummaryConfirmations => SummaryConfirmations.Count > 0;
    public bool HasActiveSummarySegment => !string.IsNullOrWhiteSpace(ActiveSummarySegmentText);
    public string ActiveSummarySegmentText
    {
        get => _activeSummarySegmentText;
        private set
        {
            if (SetProperty(ref _activeSummarySegmentText, value))
            {
                OnPropertyChanged(nameof(HasActiveSummarySegment));
            }
        }
    }
    public bool HasSummaryData => HasChapterSummary
                                  || HasStageSummary
                                  || HasSummarySegments
                                  || HasSummaryEvents
                                  || HasSummaryChanges
                                  || HasSummaryForeshadowing
                                  || HasSummaryConfirmations;
    public bool ShowSummaryContent => HasSummaryContext
                                      && !IsSummaryLoading
                                      && !HasSummaryError
                                      && HasSummaryData;
    public bool ShowSummaryEmpty => HasSummaryContext
                                    && !IsSummaryLoading
                                    && !HasSummaryError
                                    && !HasSummaryData;

    public bool IsEditMode
    {
        get => _isEditMode;
        set
        {
            if (!SetProperty(ref _isEditMode, value))
            {
                return;
            }

            if (!value)
            {
                RebuildDocumentBlocks(_editorBuffer.Text);
            }
            OnPropertyChanged(nameof(ShowReadModeEmptyDocument));
        }
    }

    public string DocumentContent
    {
        get => _editorBuffer.Text;
        set => ReplaceDocumentContent(value ?? string.Empty);
    }

    public bool ShowReadModeEmptyDocument => !IsEditMode && _documentCharacterCount == 0;

    public bool HasUnsavedChanges
    {
        get => _hasUnsavedChanges;
        private set
        {
            if (SetProperty(ref _hasUnsavedChanges, value))
            {
                OnPropertyChanged(nameof(DocumentInfoText));
                OnPropertyChanged(nameof(DocumentSaveStateText));
            }
        }
    }

    public bool HasCurrentDocument => !string.IsNullOrWhiteSpace(_currentDocumentId);

    public bool IsDocumentSaving
    {
        get => _isDocumentSaving;
        private set
        {
            if (SetProperty(ref _isDocumentSaving, value))
            {
                SaveCommand.NotifyCanExecuteChanged();
                OnPropertyChanged(nameof(DocumentInfoText));
                OnPropertyChanged(nameof(DocumentSaveStateText));
            }
        }
    }

    public string DocumentLoadingText => _displayNames.Text("ui.works.loading_document");

    public bool IsDocumentLoading
    {
        get => _isDocumentLoading;
        private set => SetProperty(ref _isDocumentLoading, value);
    }

    public string DocumentLoadingTargetText => string.IsNullOrWhiteSpace(_documentLoadingTarget)
        ? DocumentLoadingText
        : _displayNames.Format("ui.works.loading_document_target", new Dictionary<string, string>
        {
            ["title"] = _documentLoadingTarget,
        });

    public string SavingText => _displayNames.Text("ui.works.saving");

    public string DocumentSaveStateText => !HasCurrentDocument
        ? string.Empty
        : IsDocumentSaving
            ? SavingText
            : HasUnsavedChanges
                ? _displayNames.Text("ui.works.save_state.unsaved")
                : _displayNames.Text("ui.works.save_state.saved");

    public string StatusText
    {
        get => _statusText;
        set => SetProperty(ref _statusText, value);
    }

    public string ProjectAiMessage
    {
        get => _projectAiMessage;
        set
        {
            if (SetProperty(ref _projectAiMessage, value))
            {
                SendProjectAiCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public string ProjectAiAnswer
    {
        get => _projectAiAnswer;
        set => SetProperty(ref _projectAiAnswer, value);
    }

    public string QuickEditInstruction
    {
        get => _quickEditInstruction;
        set
        {
            if (SetProperty(ref _quickEditInstruction, value))
            {
                InvalidateQuickEditGeneration();
                QuickAiCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public string QuickEditDiff
    {
        get => _quickEditDiff;
        set => SetProperty(ref _quickEditDiff, value);
    }

    public bool IsQuickEditGenerating
    {
        get => _isQuickEditGenerating;
        private set
        {
            if (SetProperty(ref _isQuickEditGenerating, value))
            {
                OnPropertyChanged(nameof(QuickEditGenerateText));
                QuickAiCommand.NotifyCanExecuteChanged();
                OpenQuickEditCommand.NotifyCanExecuteChanged();
                ApplyQuickEditCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public string ExportFormat
    {
        get => _exportFormat;
        set => SetProperty(ref _exportFormat, value);
    }

    public string DocumentTitle
    {
        get => _documentTitle;
        set
        {
            if (SetProperty(ref _documentTitle, value))
            {
                OnPropertyChanged(nameof(CurrentDocumentText));
            }
        }
    }

    public string ImportChapterId
    {
        get => _importChapterId;
        set
        {
            if (SetProperty(ref _importChapterId, value))
            {
                AllowImportOverwrite = false;
                NotifyImportFormStateChanged();
            }
        }
    }

    public string ImportChapterTitle
    {
        get => _importChapterTitle;
        set
        {
            if (SetProperty(ref _importChapterTitle, value))
            {
                NotifyImportFormStateChanged();
            }
        }
    }

    public decimal? ImportOrder
    {
        get => _importOrder;
        set
        {
            if (SetProperty(ref _importOrder, value))
            {
                NotifyImportFormStateChanged();
            }
        }
    }

    public string ImportSourcePath
    {
        get => _importSourcePath;
        set
        {
            if (SetProperty(ref _importSourcePath, value))
            {
                NotifyImportFormStateChanged();
            }
        }
    }

    public string ImportTargetPath
    {
        get => _importTargetPath;
        set
        {
            if (SetProperty(ref _importTargetPath, value))
            {
                AllowImportOverwrite = false;
                NotifyImportFormStateChanged();
            }
        }
    }

    public bool AllowImportOverwrite
    {
        get => _allowImportOverwrite;
        set
        {
            if (SetProperty(ref _allowImportOverwrite, value))
            {
                ImportCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public string SidebarTitle => _displayNames.Text("ui.works.sidebar.title");

    public string ImportText => _displayNames.Text("ui.works.import_manuscript");

    public string ExportText => _displayNames.Text("ui.works.export_combined");

    public string ReadModeText => _displayNames.Text("ui.works.read_mode");

    public string EditModeText => _displayNames.Text("ui.works.edit_mode");

    public string SaveText => _displayNames.Text("ui.common.save");

    public string OutlineText => _displayNames.Text("ui.works.outline");

    public string NavTreeText => _displayNames.Text("ui.works.nav_tree");

    public string ProjectAiText => _displayNames.Text("ui.works.project_ai");

    public string SummaryTitle => _displayNames.Text("ui.works.summary.title");
    public string SummaryLoadingText => _displayNames.Text("ui.works.summary.loading");
    public string SummaryEmptyText => _displayNames.Text("ui.works.summary.empty");
    public string ChapterSummaryLabel => _displayNames.Text("ui.works.summary.chapter");
    public string StageSummaryLabel => _displayNames.Text("ui.works.summary.stage");
    public string SummarySegmentsLabel => _displayNames.Text("ui.works.summary.segments");
    public string SummaryEventsLabel => _displayNames.Text("ui.works.summary.events");
    public string SummaryChangesLabel => _displayNames.Text("ui.works.summary.changes");
    public string SummaryForeshadowingLabel => _displayNames.Text("ui.works.summary.foreshadowing");
    public string SummaryConfirmationsLabel => _displayNames.Text("ui.works.summary.confirmations");
    public string SummaryChapterHeading => _displayNames.Format(
        "ui.works.summary.chapter_heading",
        new Dictionary<string, string> { ["chapter"] = ShortValue(_currentSummaryChapterId) });
    public string SummaryStageHeading => _displayNames.Format(
        "ui.works.summary.stage_heading",
        new Dictionary<string, string>
        {
            ["stage"] = string.IsNullOrWhiteSpace(SummaryStageId)
                ? _displayNames.Text("ui.common.none")
                : SummaryStageId,
        });

    public string NoDocumentText => _displayNames.Text("ui.works.no_document_selected");

    public string CurrentDocumentText => DocumentTitle;

    public string DocumentInfoText => string.IsNullOrWhiteSpace(_currentDocumentId)
        ? NoDocumentText
        : _displayNames.Format("ui.works.document_info", new Dictionary<string, string>
        {
            ["path"] = string.IsNullOrWhiteSpace(_currentDocumentPath) ? _currentDocumentId : _currentDocumentPath,
            ["version"] = ShortValue(_currentDocumentVersion),
            ["state"] = HasUnsavedChanges
                ? _displayNames.Text("ui.works.save_state.unsaved")
                : _displayNames.Text("ui.works.save_state.saved"),
        });

    public string DocumentBodyText => string.IsNullOrWhiteSpace(_currentDocumentId)
        ? NoDocumentText
        : _displayNames.Text("ui.works.empty_document");

    public string CharacterCountText => _displayNames.Format("ui.works.characters_count", new Dictionary<string, string>
    {
        ["count"] = _documentCharacterCount.ToString(),
    });

    public string EmptyIndexText => _displayNames.Text("ui.works.empty_index");
    public string EmptyIndexTitle => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.works.index.title")
        : _displayNames.Text("ui.empty.need_project.title");
    public string EmptyIndexHint => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.works.index.hint")
        : _displayNames.Text("ui.empty.need_project.hint");

    public string QuickAiHint => _displayNames.Text("ui.works.quick_ai_hint");

    public string ProjectAiPlaceholder => _displayNames.Text("ui.works.project_ai.placeholder");

    public string ExportFormatText => _displayNames.Text("ui.works.export_format");
    public string ImportTitle => _displayNames.Text("ui.works.import.title");
    public string ImportChapterIdText => _displayNames.Text("ui.works.import.chapter_id");
    public string ImportChapterTitleText => _displayNames.Text("ui.works.import.chapter_title");
    public string ImportOrderText => _displayNames.Text("ui.works.import.order");
    public string ImportSourcePathText => _displayNames.Text("ui.works.import.source_path");
    public string ImportTargetPathText => _displayNames.Text("ui.works.import.target_path");
    public string ImportSourcePlaceholder => _displayNames.Text("ui.works.import.source_placeholder");
    public string ImportTargetPlaceholder => _displayNames.Text("ui.works.import.target_placeholder");
    public string BrowseImportSourceText => _displayNames.Text("ui.works.import.browse_source");
    public string ImportSourceGroupText => _displayNames.Text("ui.works.import.source_group");
    public string ImportTargetGroupText => _displayNames.Text("ui.works.import.target_group");
    public string ImportOverwriteText => _displayNames.Text("ui.works.import.overwrite_confirm");
    public string ImportChapterIdErrorText => HasImportChapterIdError
        ? _displayNames.Text("ui.works.import.error.chapter_id_required")
        : string.Empty;
    public string ImportChapterTitleErrorText => HasImportChapterTitleError
        ? _displayNames.Text("ui.works.import.error.chapter_title_required")
        : string.Empty;
    public string ImportOrderErrorText => HasImportOrderError
        ? _displayNames.Text("ui.works.import.error.order_invalid")
        : string.Empty;
    public string ImportSourceErrorText => ImportPathErrorText(ImportSourceValidation.Error);
    public string ImportTargetErrorText => ImportPathErrorText(ImportTargetValidation.Error);
    public string ImportConflictText => HasImportConflict
        ? _displayNames.Text("ui.works.import.conflict")
        : string.Empty;
    public string ImportTargetPreviewText => ImportTargetValidation.IsValid
        ? _displayNames.Format(
            "ui.works.import.target_preview",
            new Dictionary<string, string> { ["path"] = ImportTargetValidation.NormalizedPath })
        : string.Empty;
    public string ImportConfirmationText => HasImportConfirmation
        ? _displayNames.Format(
            "ui.works.import.confirmation",
            new Dictionary<string, string>
            {
                ["title"] = ImportChapterTitle.Trim(),
                ["path"] = ImportTargetValidation.NormalizedPath,
            })
        : string.Empty;
    public bool HasImportChapterIdError => string.IsNullOrWhiteSpace(ImportChapterId);
    public bool HasImportChapterTitleError => string.IsNullOrWhiteSpace(ImportChapterTitle);
    public bool HasImportOrderError => ImportOrder is null
                                           or < 0
                                           or > long.MaxValue
                                       || decimal.Truncate(ImportOrder.Value) != ImportOrder.Value;
    public bool HasImportSourceError => !ImportSourceValidation.IsValid;
    public bool HasImportTargetError => !ImportTargetValidation.IsValid;
    public bool HasImportTargetPreview => ImportTargetValidation.IsValid;
    public bool HasImportConfirmation => !string.IsNullOrWhiteSpace(ImportChapterTitle)
                                         && ImportTargetValidation.IsValid;
    public bool HasImportConflict => HasImportChapterConflict() || HasImportDocumentConflict();
    public string QuickEditTitle => _displayNames.Text("ui.works.quick_edit.title");
    public string QuickEditPlaceholder => _displayNames.Text("ui.works.quick_edit.placeholder");
    public string QuickEditGenerateText => _displayNames.Text(IsQuickEditGenerating
        ? "ui.works.quick_edit.generating"
        : "ui.works.quick_edit.generate");
    public string QuickEditDiffText => _displayNames.Text("ui.works.quick_edit.diff");
    public string QuickEditApplyText => _displayNames.Text("ui.works.quick_edit.apply");
    public string QuickEditUndoText => _displayNames.Text("ui.works.quick_edit.undo");

    // 右键菜单文案（阅读/修改器）
    public string CtxCopyText => _displayNames.Text("ui.works.context.copy");
    public string CtxSelectAllText => _displayNames.Text("ui.works.context.select_all");
    public string CtxQuickAiText => _displayNames.Text("ui.works.context.quick_ai");
    public string CtxInsertOutlineText => _displayNames.Text("ui.works.context.insert_outline");
    public string CtxToggleEditText => _displayNames.Text("ui.works.context.toggle_edit");

    private bool CanImportChapter()
    {
        return !HasImportChapterIdError
               && !HasImportChapterTitleError
               && !HasImportOrderError
               && ImportSourceValidation.IsValid
               && ImportTargetValidation.IsValid
               && (!HasImportConflict || AllowImportOverwrite);
    }

    private ImportPathValidation ImportSourceValidation => WorksImportHelper.ValidateProjectPath(
        ImportSourcePath,
        _importProjectRoot,
        requireDocumentsDirectory: false);

    private ImportPathValidation ImportTargetValidation => WorksImportHelper.ValidateProjectPath(
        ImportTargetPath,
        _importProjectRoot,
        requireDocumentsDirectory: true);

    private bool HasImportChapterConflict()
    {
        var chapterId = ImportChapterId.Trim();
        return chapterId.Length > 0
               && EnumerateWorksTreeNodes().Any(item => string.Equals(
                   item.ChapterId,
                   chapterId,
                   StringComparison.Ordinal));
    }

    private bool HasImportDocumentConflict()
    {
        var target = ImportTargetValidation;
        if (!target.IsValid)
        {
            return false;
        }

        return EnumerateWorksTreeNodes().Any(item =>
        {
            var existing = WorksImportHelper.ValidateProjectPath(
                item.Path,
                _importProjectRoot,
                requireDocumentsDirectory: false);
            return existing.IsValid
                   && string.Equals(
                       existing.NormalizedPath,
                       target.NormalizedPath,
                       StringComparison.OrdinalIgnoreCase);
        });
    }

    private string ImportPathErrorText(ImportPathError error)
    {
        var key = error switch
        {
            ImportPathError.None => string.Empty,
            ImportPathError.Required => "ui.works.import.error.path_required",
            ImportPathError.OutsideProject => "ui.works.import.error.path_outside_project",
            ImportPathError.ParentTraversal => "ui.works.import.error.path_parent_traversal",
            ImportPathError.TargetOutsideDocuments => "ui.works.import.error.target_outside_documents",
            _ => "ui.works.import.error.path_invalid",
        };
        return key.Length == 0 ? string.Empty : _displayNames.Text(key);
    }

    private void NotifyImportFormStateChanged()
    {
        OnPropertyChanged(nameof(ImportChapterIdErrorText));
        OnPropertyChanged(nameof(ImportChapterTitleErrorText));
        OnPropertyChanged(nameof(ImportOrderErrorText));
        OnPropertyChanged(nameof(ImportSourceErrorText));
        OnPropertyChanged(nameof(ImportTargetErrorText));
        OnPropertyChanged(nameof(ImportConflictText));
        OnPropertyChanged(nameof(ImportTargetPreviewText));
        OnPropertyChanged(nameof(ImportConfirmationText));
        OnPropertyChanged(nameof(HasImportChapterIdError));
        OnPropertyChanged(nameof(HasImportChapterTitleError));
        OnPropertyChanged(nameof(HasImportOrderError));
        OnPropertyChanged(nameof(HasImportSourceError));
        OnPropertyChanged(nameof(HasImportTargetError));
        OnPropertyChanged(nameof(HasImportTargetPreview));
        OnPropertyChanged(nameof(HasImportConfirmation));
        OnPropertyChanged(nameof(HasImportConflict));
        ImportCommand.NotifyCanExecuteChanged();
    }

    private bool CanGenerateQuickEdit()
    {
        return HasCurrentDocument
               && _documentCharacterCount > 0
               && !IsQuickEditGenerating
               && !string.IsNullOrWhiteSpace(QuickEditInstruction);
    }

    private bool CanOpenQuickEdit()
    {
        return HasCurrentDocument && !IsQuickEditGenerating;
    }

    private void OpenQuickEdit()
    {
        IsEditMode = true;
        RequestFocusQuickEditInstruction?.Invoke();
    }

    private bool CanApplyQuickEdit()
    {
        return !IsQuickEditGenerating
               && _pendingQuickEdit is not null
               && _pendingQuickEdit.MatchesCurrent(
                   _currentDocumentId,
                   _currentDocumentVersion,
                   AssembleDocumentContent());
    }

    private bool CanUndoQuickEdit()
    {
        return _quickEditUndo is not null
               && _quickEditUndo.TryUndo(
                   _currentDocumentId,
                   AssembleDocumentContent(),
                   out _);
    }

    private bool CanSendProjectAi()
    {
        return !string.IsNullOrWhiteSpace(ProjectAiMessage);
    }

    private void OnCurrentDocumentChanged()
    {
        RefreshCurrentWorksTreeNode();
        OnPropertyChanged(nameof(HasCurrentDocument));
        OnPropertyChanged(nameof(ShowNoDocumentEmpty));
        OnPropertyChanged(nameof(ShowDocumentChrome));
        OnPropertyChanged(nameof(DocumentInfoText));
        OnPropertyChanged(nameof(DocumentSaveStateText));
        SaveCommand.NotifyCanExecuteChanged();
        OpenQuickEditCommand.NotifyCanExecuteChanged();
        InsertOutlineCommand.NotifyCanExecuteChanged();
        QuickAiCommand.NotifyCanExecuteChanged();
    }

    private void SetWorksTreeState(WorksTreeLoadState state)
    {
        if (_worksTreeState == state && (state != WorksTreeLoadState.Error || !string.IsNullOrWhiteSpace(_worksTreeErrorText)))
        {
            RetryWorksTreeCommand.NotifyCanExecuteChanged();
            ExportCommand.NotifyCanExecuteChanged();
            return;
        }

        _worksTreeState = state;
        if (state != WorksTreeLoadState.Error)
        {
            _worksTreeErrorText = string.Empty;
            OnPropertyChanged(nameof(WorksTreeErrorText));
        }
        OnPropertyChanged(nameof(IsWorksTreeLoading));
        OnPropertyChanged(nameof(IsWorksTreeError));
        OnPropertyChanged(nameof(IsWorksTreeEmpty));
        OnPropertyChanged(nameof(IsWorksTreeContent));
        OnPropertyChanged(nameof(ShowWorksTreeSearchEmpty));
        OnPropertyChanged(nameof(ShowNoDocumentEmpty));
        OnPropertyChanged(nameof(ShowDocumentChrome));
        OnPropertyChanged(nameof(EmptyIndexTitle));
        OnPropertyChanged(nameof(EmptyIndexHint));
        RetryWorksTreeCommand.NotifyCanExecuteChanged();
        ExportCommand.NotifyCanExecuteChanged();
    }

    private void NotifySummaryStateChanged()
    {
        OnPropertyChanged(nameof(HasSummaryContext));
        OnPropertyChanged(nameof(HasSummaryError));
        OnPropertyChanged(nameof(HasChapterSummary));
        OnPropertyChanged(nameof(HasStageSummary));
        OnPropertyChanged(nameof(HasSummarySegments));
        OnPropertyChanged(nameof(HasSummaryEvents));
        OnPropertyChanged(nameof(HasSummaryChanges));
        OnPropertyChanged(nameof(HasSummaryForeshadowing));
        OnPropertyChanged(nameof(HasSummaryConfirmations));
        OnPropertyChanged(nameof(HasSummaryData));
        OnPropertyChanged(nameof(ShowSummaryContent));
        OnPropertyChanged(nameof(ShowSummaryEmpty));
        OnPropertyChanged(nameof(SummaryChapterHeading));
    }

    private void ClearSummaryProjection()
    {
        ActiveSummarySegmentText = string.Empty;
        ChapterSummaryText = null;
        SummaryStageId = null;
        StageSummaryText = null;
        SummarySegments.Clear();
        SummaryEvents.Clear();
        SummaryChanges.Clear();
        SummaryForeshadowing.Clear();
        SummaryConfirmations.Clear();
        NotifySummaryStateChanged();
    }

    private void ClearSummaryState()
    {
        Interlocked.Increment(ref _summaryLoadGeneration);
        _summaryLoadCts?.Cancel();
        _summaryLoadCts?.Dispose();
        _summaryLoadCts = null;
        _currentSummaryChapterId = string.Empty;
        OnPropertyChanged(nameof(CurrentSummaryChapterId));
        IsSummaryLoading = false;
        SummaryErrorText = string.Empty;
        ClearSummaryProjection();
    }

    private async Task LoadChapterSummaryAsync(string chapterId)
    {
        if (string.IsNullOrWhiteSpace(chapterId))
        {
            ClearSummaryState();
            return;
        }

        var generation = Interlocked.Increment(ref _summaryLoadGeneration);
        _summaryLoadCts?.Cancel();
        _summaryLoadCts?.Dispose();
        var cts = new CancellationTokenSource();
        _summaryLoadCts = cts;
        _currentSummaryChapterId = chapterId;
        OnPropertyChanged(nameof(CurrentSummaryChapterId));
        SummaryErrorText = string.Empty;
        ClearSummaryProjection();
        IsSummaryLoading = true;

        try
        {
            var summary = await _backend
                .GetChapterSummaryViewAsync(chapterId, cts.Token)
                .ConfigureAwait(true);
            if (generation != Interlocked.Read(ref _summaryLoadGeneration)
                || cts.IsCancellationRequested
                || !string.Equals(chapterId, _currentSummaryChapterId, StringComparison.Ordinal))
            {
                return;
            }
            if (!string.Equals(summary.ChapterId, chapterId, StringComparison.Ordinal))
            {
                throw new InvalidDataException("chapter summary response does not match the requested chapter");
            }

            ApplySummaryProjection(summary);
        }
        catch (OperationCanceledException) when (cts.IsCancellationRequested)
        {
            // 文档切换会主动取消旧请求；旧结果不得覆盖当前章节。
        }
        catch (Exception ex)
        {
            if (generation == Interlocked.Read(ref _summaryLoadGeneration)
                && string.Equals(chapterId, _currentSummaryChapterId, StringComparison.Ordinal))
            {
                ClearSummaryProjection();
                SummaryErrorText = UserFacingError.Format(ex, _displayNames);
            }
        }
        finally
        {
            if (generation == Interlocked.Read(ref _summaryLoadGeneration))
            {
                IsSummaryLoading = false;
                if (ReferenceEquals(_summaryLoadCts, cts))
                {
                    _summaryLoadCts = null;
                    cts.Dispose();
                }
            }
        }
    }

    private void ApplySummaryProjection(ChapterSummaryView summary)
    {
        ChapterSummaryText = summary.ChapterSummary;
        SummaryStageId = summary.Stage?.StageId;
        StageSummaryText = summary.Stage?.Summary;

        foreach (var segment in summary.Segments)
        {
            var sourceText = _displayNames.Format(
                "ui.works.summary.source",
                new Dictionary<string, string>
                {
                    ["document"] = segment.Source.DocumentId,
                    ["start"] = segment.Source.Range.Start.ToString(),
                    ["end"] = segment.Source.Range.End.ToString(),
                    ["version"] = ShortValue(segment.Source.Version),
                });
            SummarySegments.Add(new WorksSummarySegmentItemViewModel(
                segment,
                _displayNames.Format(
                    "ui.works.summary.segment_item",
                    new Dictionary<string, string>
                    {
                        ["number"] = segment.Number,
                        ["id"] = segment.SegmentId,
                    }),
                segment.Summary,
                sourceText,
                _displayNames.Text("ui.works.summary.reveal_source"),
                () => RevealSummarySource(segment)));
        }

        foreach (var storyEvent in summary.Events)
        {
            SummaryEvents.Add(new WorksSummaryDetailItemViewModel(
                _displayNames.Format(
                    "ui.works.summary.event_item",
                    new Dictionary<string, string> { ["id"] = storyEvent.EventId }),
                storyEvent.Summary,
                LocalizeSummaryStatus(storyEvent.Status),
                storyEvent.SegmentIds));
        }

        foreach (var change in summary.RealizedChanges)
        {
            SummaryChanges.Add(new WorksSummaryDetailItemViewModel(
                LocalizeChangeFunction(change.Function),
                FormatRegisteredChangeContent(change.Content),
                LocalizeSummaryStatus(change.Status)));
        }

        foreach (var record in summary.Foreshadowing)
        {
            SummaryForeshadowing.Add(new WorksSummaryDetailItemViewModel(
                record.Title,
                record.Description,
                LocalizeSummaryStatus(record.Status)));
        }

        foreach (var confirmation in summary.Confirmations)
        {
            SummaryConfirmations.Add(new WorksSummaryDetailItemViewModel(
                LocalizeConfirmationKind(confirmation.Kind),
                _displayNames.Format(
                    "ui.works.summary.confirmation_detail",
                    new Dictionary<string, string>
                    {
                        ["id"] = confirmation.ConfirmationId,
                        ["revision"] = ShortValue(confirmation.RevisionId),
                    }),
                LocalizeSummaryStatus(confirmation.State)));
        }

        RefreshSummarySourceFreshness();
        NotifySummaryStateChanged();
    }

    private string LocalizeSummaryStatus(string status)
    {
        var key = status switch
        {
            "ongoing" => "ui.status.ongoing",
            "paused" => "ui.status.paused",
            "completed" => "ui.status.completed",
            "planned" => "ui.status.planned",
            "realized" => "ui.status.realized",
            "deleted" => "ui.status.deleted",
            "planted" => "ui.status.planted",
            "recovered" => "ui.status.recovered",
            "abandoned" => "ui.status.abandoned",
            "pending" => "ui.status.pending",
            "skipped" => "ui.status.skipped",
            "auto_audited" => "ui.status.auto_audited",
            "approved" => "ui.status.approved",
            "rejected" => "ui.status.rejected",
            _ => "ui.common.unknown",
        };
        return _displayNames.Text(key);
    }

    private string LocalizeChangeFunction(string function)
    {
        var key = function switch
        {
            "character_profile" => "ui.works.summary.change.character_profile",
            "character_plan" => "ui.works.summary.change.character_plan",
            "character_trait" => "ui.works.summary.change.character_trait",
            "relationship" => "ui.works.summary.change.relationship",
            "foreshadowing" => "ui.works.summary.change.foreshadowing",
            "theme_anchor" => "ui.works.summary.change.theme_anchor",
            _ => "ui.common.unknown",
        };
        return _displayNames.Text(key);
    }

    private string LocalizeConfirmationKind(string kind)
    {
        var key = kind switch
        {
            "segment_summary" => "confirmation.summarizer.segment",
            "event_summary" => "confirmation.summarizer.event",
            "chapter_summary" => "confirmation.summarizer.chapter",
            "stage_summary" => "confirmation.summarizer.stage",
            _ => "ui.common.unknown",
        };
        return _displayNames.Text(key);
    }

    private static string FormatRegisteredChangeContent(JsonElement content)
    {
        if (content.ValueKind != JsonValueKind.Object
            || !content.TryGetProperty("content", out var payload))
        {
            return content.ToString();
        }

        var values = new List<string>();
        CollectDisplayValues(payload, values);
        return values.Count == 0 ? payload.ToString() : string.Join(" · ", values.Distinct());
    }

    private static void CollectDisplayValues(JsonElement value, ICollection<string> values)
    {
        switch (value.ValueKind)
        {
            case JsonValueKind.Object:
                foreach (var property in value.EnumerateObject())
                {
                    CollectDisplayValues(property.Value, values);
                }
                break;
            case JsonValueKind.Array:
                foreach (var item in value.EnumerateArray())
                {
                    CollectDisplayValues(item, values);
                }
                break;
            case JsonValueKind.String:
                if (!string.IsNullOrWhiteSpace(value.GetString()))
                {
                    values.Add(value.GetString()!);
                }
                break;
            case JsonValueKind.Number:
            case JsonValueKind.True:
            case JsonValueKind.False:
                values.Add(value.ToString());
                break;
        }
    }

    private void RefreshSummarySourceFreshness()
    {
        foreach (var segment in SummarySegments)
        {
            var source = segment.Source;
            var matchesDocument = SummarySourceMatchesCurrentDocument(source.DocumentId);
            var versionMatches = !string.IsNullOrWhiteSpace(source.Version)
                                 && !string.IsNullOrWhiteSpace(_currentDocumentVersion)
                                 && string.Equals(source.Version, _currentDocumentVersion, StringComparison.Ordinal);
            var isFresh = !_documentDirty && !HasUnsavedChanges && matchesDocument && versionMatches;
            var stateText = isFresh
                ? _displayNames.Text("ui.works.summary.source_fresh")
                : _displayNames.Text("ui.works.summary.source_stale");
            segment.UpdateSourceState(isFresh, stateText);
        }
        if (SummarySegments.Any(segment => segment.IsSelected && !segment.IsSourceFresh))
        {
            SelectSummarySegment(null);
        }
    }

    /// <summary>
    /// 正文 → 总结的反向定位。编辑器传入全局 UTF-16 光标/选区，先转换为
    /// UTF-8 byte offset，再按同一文档与版本命中唯一故事段。
    /// </summary>
    public void UpdateSummarySelectionFromEditor(EditorTextSelection selection)
    {
        RefreshSummarySourceFreshness();
        if (!HasSummaryContext || _documentDirty || HasUnsavedChanges)
        {
            SelectSummarySegment(null);
            return;
        }

        var utf16Offset = Math.Min(selection.Start, selection.End);
        var content = AssembleDocumentContent();
        if (!WorksSummarySourceMapper.TryMapUtf16OffsetToUtf8(
                content,
                utf16Offset,
                out var byteOffset))
        {
            SelectSummarySegment(null);
            return;
        }
        if (!WorksSummarySourceMapper.TryMapUtf16OffsetToUtf8(
                content,
                content.Length,
                out var documentByteLength))
        {
            SelectSummarySegment(null);
            return;
        }

        var selected = SummarySegments.FirstOrDefault(item =>
            item.IsSourceFresh
            && SummarySourceMatchesCurrentDocument(item.Source.DocumentId)
            && item.Source.Range.Start <= byteOffset
            && (byteOffset < item.Source.Range.End
                || (byteOffset == documentByteLength
                    && byteOffset == item.Source.Range.End)));
        SelectSummarySegment(selected);
    }

    private void SelectSummarySegment(WorksSummarySegmentItemViewModel? selected)
    {
        foreach (var item in SummarySegments)
        {
            item.UpdateSelected(ReferenceEquals(item, selected));
        }
        if (selected is null)
        {
            ActiveSummarySegmentText = string.Empty;
            return;
        }

        var eventTitles = SummaryEvents
            .Where(item => item.RelatedSegmentIds?.Contains(selected.Segment.SegmentId) == true)
            .Select(item => item.Title)
            .ToArray();
        ActiveSummarySegmentText = _displayNames.Format(
            "ui.works.summary.active_source",
            new Dictionary<string, string>
            {
                ["segment"] = selected.Title,
                ["events"] = eventTitles.Length == 0
                    ? _displayNames.Text("ui.common.none")
                    : string.Join("、", eventTitles),
            });
    }

    private bool SummarySourceMatchesCurrentDocument(string sourceDocumentId)
    {
        var source = ProjectRelativePath(sourceDocumentId);
        return !string.IsNullOrWhiteSpace(source)
               && (string.Equals(source, ProjectRelativePath(_currentDocumentId), StringComparison.OrdinalIgnoreCase)
                   || string.Equals(source, ProjectRelativePath(_currentDocumentPath), StringComparison.OrdinalIgnoreCase));
    }

    private void RevealSummarySource(StorySegmentView segment)
    {
        RefreshSummarySourceFreshness();
        if (_documentDirty || HasUnsavedChanges)
        {
            StatusText = _displayNames.Text("ui.works.summary.source_unsaved");
            return;
        }
        if (!SummarySourceMatchesCurrentDocument(segment.Source.DocumentId))
        {
            StatusText = _displayNames.Text("ui.works.summary.source_document_mismatch");
            return;
        }
        if (string.IsNullOrWhiteSpace(segment.Source.Version)
            || string.IsNullOrWhiteSpace(_currentDocumentVersion)
            || !string.Equals(segment.Source.Version, _currentDocumentVersion, StringComparison.Ordinal))
        {
            StatusText = _displayNames.Text("ui.works.summary.source_version_mismatch");
            return;
        }
        if (!WorksSummarySourceMapper.TryMapUtf8Range(
                AssembleDocumentContent(),
                segment.Source.Range.Start,
                segment.Source.Range.End,
                out var start,
                out var end))
        {
            StatusText = _displayNames.Text("ui.works.summary.source_invalid");
            return;
        }
        if (RequestRevealEditorRange is null)
        {
            StatusText = _displayNames.Text("ui.works.summary.source_unavailable");
            return;
        }

        SelectSummarySegment(SummarySegments.FirstOrDefault(item =>
            string.Equals(item.Segment.SegmentId, segment.SegmentId, StringComparison.Ordinal)));
        IsEditMode = true;
        RequestRevealEditorRange(start, end);
        StatusText = _displayNames.Text("ui.works.summary.source_revealed");
    }

    private void ClearPendingQuickEdit()
    {
        if (_pendingQuickEdit is null
            && string.IsNullOrEmpty(QuickEditDiff))
        {
            return;
        }

        _pendingQuickEdit = null;
        QuickEditDiff = string.Empty;
        ApplyQuickEditCommand.NotifyCanExecuteChanged();
    }

    private void InvalidateQuickEditGeneration()
    {
        Interlocked.Increment(ref _quickEditGeneration);
        _quickEditGenerationCts?.Cancel();
        _quickEditGenerationCts?.Dispose();
        _quickEditGenerationCts = null;
        IsQuickEditGenerating = false;
        ClearPendingQuickEdit();
    }

    private void ClearQuickEditUndo()
    {
        if (_quickEditUndo is null)
        {
            return;
        }
        _quickEditUndo = null;
        UndoQuickEditCommand.NotifyCanExecuteChanged();
    }

    private void ReplaceDocumentContent(string content)
    {
        var resetUndoHistory = _suppressDirtyTracking;
        var changed = _editorBuffer.Replace(content, resetUndoHistory);
        if (!changed && (resetUndoHistory || !IsEditMode))
        {
            RebuildDocumentBlocks(content);
        }
    }

    private void OnEditorDocumentTextChanged(object? sender, EventArgs e)
    {
        _documentCharacterCount = _editorBuffer.Length;
        OnPropertyChanged(nameof(DocumentContent));
        OnPropertyChanged(nameof(DocumentBodyText));
        OnPropertyChanged(nameof(CharacterCountText));
        OnPropertyChanged(nameof(ShowReadModeEmptyDocument));
        OnPropertyChanged(nameof(DocumentInfoText));
        QuickAiCommand.NotifyCanExecuteChanged();

        if (_suppressDirtyTracking || !IsEditMode)
        {
            RebuildDocumentBlocks(_editorBuffer.Text);
        }

        if (_suppressDirtyTracking)
        {
            return;
        }

        Interlocked.Increment(ref _documentEditRevision);
        InvalidateQuickEditGeneration();
        ClearQuickEditUndo();
    }

    private void MarkDocumentDirty()
    {
        _documentDirty = true;
        HasUnsavedChanges = true;
        RefreshSummarySourceFreshness();
    }

    private string AssembleDocumentContent() => _editorBuffer.Text;

    private void RebuildDocumentBlocks(string content)
    {
        DocumentBlocks.Clear();
        var index = 0;
        foreach (var block in SplitDocumentBlocks(content))
        {
            DocumentBlocks.Add(new DocumentBlockViewModel(
                $"read-block-{index}",
                index++,
                block));
        }
        OnPropertyChanged(nameof(HasDocumentBlocks));
        OnPropertyChanged(nameof(ShowReadModeEmptyDocument));
        OnPropertyChanged(nameof(DocumentInfoText));
    }

    private static IEnumerable<string> SplitDocumentBlocks(string content)
    {
        if (string.IsNullOrEmpty(content))
        {
            yield break;
        }

        var start = 0;
        while (start < content.Length)
        {
            var remaining = content.Length - start;
            if (remaining <= HardDocumentBlockSize)
            {
                yield return content[start..];
                yield break;
            }

            var limit = Math.Min(content.Length, start + HardDocumentBlockSize);
            var preferredStart = Math.Min(content.Length, start + TargetDocumentBlockSize);
            var split = content.LastIndexOf("\n\n", limit - 1, limit - start, StringComparison.Ordinal);
            if (split < preferredStart)
            {
                split = content.LastIndexOf('\n', limit - 1, limit - start);
            }
            if (split < preferredStart)
            {
                split = start + TargetDocumentBlockSize;
            }
            else
            {
                split += content[split] == '\n' ? 1 : 2;
            }

            split = Math.Clamp(split, start + 1, content.Length);
            yield return content[start..split];
            start = split;
        }
    }

    private async Task InitializeAsync(CancellationToken cancellationToken = default)
    {
        await LoadWorksTreeAsync(cancellationToken).ConfigureAwait(true);
    }

    private async Task LoadWorksTreeAsync(CancellationToken cancellationToken = default)
    {
        var generation = Interlocked.Increment(ref _worksTreeLoadGeneration);
        _worksTreeLoadCts?.Cancel();
        _worksTreeLoadCts?.Dispose();
        using var loadCts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        _worksTreeLoadCts = loadCts;
        SetWorksTreeState(WorksTreeLoadState.Loading);

        if (!_backend.HasProjectRoot)
        {
            ReplaceWorksTree(Array.Empty<WorksTreeItemViewModel>(), new Dictionary<string, WorksTreeItemViewModel>(StringComparer.Ordinal));
            SetCurrentWorksTreeNode(null);
            SetSelectedWorksTreeNode(null, navigate: false);
            ClearSummaryState();
            NotifyImportFormStateChanged();
            StatusText = string.Empty;
            if (generation == _worksTreeLoadGeneration)
            {
                SetWorksTreeState(WorksTreeLoadState.Empty);
            }
            return;
        }

        try
        {
            var tree = await _backend.GetWorksTreeAsync(loadCts.Token).ConfigureAwait(true);
            loadCts.Token.ThrowIfCancellationRequested();
            if (generation != _worksTreeLoadGeneration)
            {
                return;
            }

            var nodesById = new Dictionary<string, WorksTreeItemViewModel>(StringComparer.Ordinal);
            var root = BuildWorksTree(tree, parent: null, nodesById);
            ReplaceWorksTree(new[] { root }, nodesById);
            _worksTreeExpansionInitialized = true;
            RestoreWorksTreeSelectionAndCurrentDocument();
            NotifyImportFormStateChanged();
            StatusText = string.Empty;
            SetWorksTreeState(WorksTreeRoots.Count == 0
                ? WorksTreeLoadState.Empty
                : WorksTreeLoadState.Content);
        }
        catch (OperationCanceledException) when (loadCts.IsCancellationRequested)
        {
        }
        catch (Exception ex)
        {
            if (generation == _worksTreeLoadGeneration)
            {
                _worksTreeErrorText = UserFacingError.Format(ex, _displayNames);
                OnPropertyChanged(nameof(WorksTreeErrorText));
                StatusText = _worksTreeErrorText;
                SetWorksTreeState(WorksTreeLoadState.Error);
            }
        }
        finally
        {
            if (generation == _worksTreeLoadGeneration)
            {
                _worksTreeLoadCts = null;
                RetryWorksTreeCommand.NotifyCanExecuteChanged();
            }
        }
    }

    private async Task LoadDocumentAsync(WorksTreeItemViewModel item)
    {
        var nextDocumentId = ProjectRelativePath(item.Path);
        long generation = 0;
        CancellationTokenSource? loadCts = null;
        try
        {
            if (string.Equals(nextDocumentId, _currentDocumentId, StringComparison.Ordinal)
                && !string.IsNullOrWhiteSpace(_currentDocumentId))
            {
                if (!string.IsNullOrWhiteSpace(item.ChapterId)
                    && !string.Equals(item.ChapterId, _currentSummaryChapterId, StringComparison.Ordinal))
                {
                    await LoadChapterSummaryAsync(item.ChapterId).ConfigureAwait(true);
                }
                return;
            }
            if (!await ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
            {
                return;
            }

            InvalidateQuickEditGeneration();
            ClearQuickEditUndo();
            ClearStickyEditorSelection?.Invoke();

            generation = Interlocked.Increment(ref _documentLoadGeneration);
            _documentLoadCts?.Cancel();
            _documentLoadCts?.Dispose();
            loadCts = new CancellationTokenSource();
            _documentLoadCts = loadCts;
            _documentLoadingTarget = item.Title;
            OnPropertyChanged(nameof(DocumentLoadingTargetText));
            IsDocumentLoading = true;
            StatusText = DocumentLoadingText;

            _suppressDirtyTracking = true;
            try
            {
                var document = await _backend.GetDocumentContentDetailsByPathAsync(item.Path, loadCts.Token).ConfigureAwait(true);
                loadCts.Token.ThrowIfCancellationRequested();
                if (generation != _documentLoadGeneration)
                {
                    return;
                }
                DocumentContent = document.Content;
                _currentDocumentId = nextDocumentId;
                _currentDocumentPath = document.Metadata.Path;
                _currentDocumentVersion = document.Metadata.Version;
                OnCurrentDocumentChanged();
                DocumentTitle = item.Title;
                OnPropertyChanged(nameof(DocumentBodyText));
            }
            finally
            {
                _suppressDirtyTracking = false;
            }
            if (generation != _documentLoadGeneration)
            {
                return;
            }
            CaptureSnapshot();
            if (!string.IsNullOrWhiteSpace(item.ChapterId))
            {
                await LoadChapterSummaryAsync(item.ChapterId).ConfigureAwait(true);
            }
            else
            {
                ClearSummaryState();
            }
            StatusText = _displayNames.Text("ui.common.open");
        }
        catch (OperationCanceledException) when (loadCts?.IsCancellationRequested == true
                                                 || generation != 0 && generation != _documentLoadGeneration)
        {
        }
        catch (Exception ex)
        {
            if (generation == _documentLoadGeneration
                && (string.Equals(nextDocumentId, _currentDocumentId, StringComparison.Ordinal)
                    || string.IsNullOrWhiteSpace(_currentDocumentId)))
            {
                StatusText = UserFacingError.Format(ex, _displayNames);
            }
        }
        finally
        {
            if (loadCts is not null && ReferenceEquals(_documentLoadCts, loadCts))
            {
                _documentLoadCts = null;
                IsDocumentLoading = false;
                _documentLoadingTarget = string.Empty;
                OnPropertyChanged(nameof(DocumentLoadingTargetText));
            }
            loadCts?.Dispose();
        }
    }

    private async Task BrowseImportSourceAsync()
    {
        if (PickImportSourceFile is null)
        {
            StatusText = _displayNames.Text("ui.settings.browse_unavailable");
            return;
        }

        try
        {
            await EnsureImportProjectRootAsync().ConfigureAwait(true);
            var path = await PickImportSourceFile().ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(path))
            {
                return;
            }

            ImportSourcePath = path;
            // 从文件名推导 id/标题/目标/排序；已填字段不覆盖
            var suggestion = WorksImportHelper.SuggestFromSourcePath(path, CountWorksTreeChapters());
            var chapterId = ImportChapterId;
            var chapterTitle = ImportChapterTitle;
            var targetPath = ImportTargetPath;
            var order = ImportOrder;
            WorksImportHelper.ApplySuggestionIfEmpty(
                suggestion,
                ref chapterId,
                ref chapterTitle,
                ref targetPath,
                ref order);
            ImportChapterId = chapterId;
            ImportChapterTitle = chapterTitle;
            ImportTargetPath = targetPath;
            ImportOrder = order;
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private async Task ImportChapterAsync()
    {
        try
        {
            if (!CanImportChapter())
            {
                return;
            }

            var source = ImportSourceValidation.NormalizedPath;
            var target = ImportTargetValidation.NormalizedPath;
            await _backend.ImportChapterAsync(new ChapterImportRequest(
                ImportChapterId.Trim(),
                ImportChapterTitle.Trim(),
                decimal.ToInt64(ImportOrder!.Value),
                source,
                target,
                AllowImportOverwrite)).ConfigureAwait(true);
            StatusText = _displayNames.Text("ui.common.import");
            IsImportPanelOpen = false;
            AllowImportOverwrite = false;
            await LoadWorksTreeAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private async Task SaveAsync()
    {
        if (IsDocumentSaving)
        {
            return;
        }

        var saveDocumentId = _currentDocumentId;
        var saveDocumentPath = _currentDocumentPath;
        var saveVersion = _currentDocumentVersion;
        var saveRevision = _documentEditRevision;
        var saveContent = AssembleDocumentContent();
        if (string.IsNullOrWhiteSpace(saveDocumentId))
        {
            StatusText = NoDocumentText;
            return;
        }

        IsDocumentSaving = true;
        StatusText = SavingText;
        try
        {
            var report = await _backend.SaveDocumentContentAsync(
                saveDocumentId,
                saveContent,
                saveVersion).ConfigureAwait(true);
            var sameDocument = string.Equals(_currentDocumentId, saveDocumentId, StringComparison.Ordinal)
                               && (string.IsNullOrWhiteSpace(saveDocumentPath)
                                   || string.Equals(_currentDocumentPath, saveDocumentPath, StringComparison.Ordinal));
            if (!sameDocument)
            {
                return;
            }

            _currentDocumentPath = report.Metadata.Path;
            _currentDocumentVersion = report.Metadata.Version;
            OnPropertyChanged(nameof(DocumentInfoText));
            var unchangedSinceSave = saveRevision == _documentEditRevision
                                     && string.Equals(AssembleDocumentContent(), saveContent, StringComparison.Ordinal);
            if (unchangedSinceSave)
            {
                CaptureSnapshot();
                StatusText = _displayNames.Text("ui.common.save");
            }
            else
            {
                StatusText = _displayNames.Text("ui.works.edited_during_save");
                RefreshDirtyState();
            }
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
        finally
        {
            IsDocumentSaving = false;
        }
    }

    private async Task ExportAsync()
    {
        try
        {
            var report = await _backend.ExportChaptersAsync(Array.Empty<string>(), $"combined-{ExportFormat}", ExportFormat).ConfigureAwait(true);
            var path = string.IsNullOrWhiteSpace(report.StorageUri) ? report.ArtifactId : report.StorageUri;
            StatusText = _displayNames.Format("ui.works.export_done", new Dictionary<string, string>
            {
                ["format"] = report.Format,
                ["path"] = path,
            });
            // 成功后弹窗：关闭 + 打开所在文件夹（延后项：reveal 导出路径）
            var revealDir = ProjectPathHelper.ResolveRevealDirectory(path);
            var canReveal = !string.IsNullOrWhiteSpace(revealDir) && OpenFolderInShell is not null;
            var choice = await DialogService.Current.ConfirmAsync(new ConfirmDialogViewModel(
                _displayNames.Text("ui.works.export_done_title"),
                _displayNames.Format("ui.works.export_done_message", new Dictionary<string, string>
                {
                    ["format"] = report.Format,
                    ["path"] = path,
                }),
                canReveal
                    ? new[]
                    {
                        new DialogButton(_displayNames.Text("ui.works.export_open_folder"), DialogButtonVariant.Primary, 0),
                        new DialogButton(_displayNames.Text("ui.common.close"), DialogButtonVariant.Subtle, 1),
                    }
                    : new[]
                    {
                        new DialogButton(_displayNames.Text("ui.common.close"), DialogButtonVariant.Primary, 0),
                    })
            {
                CancelResultIndex = canReveal ? 1 : 0,
            }).ConfigureAwait(true);

            if (canReveal && choice == 0 && OpenFolderInShell is not null && !string.IsNullOrWhiteSpace(revealDir))
            {
                try
                {
                    await OpenFolderInShell(revealDir).ConfigureAwait(true);
                }
                catch (Exception openEx)
                {
                    StatusText = UserFacingError.Format(openEx, _displayNames);
                }
            }
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    /// <summary>测试与 UI 共用：发送项目 AI（可选注入选区，绕过 Avalonia 焦点）。</summary>
    internal Task SendProjectAiAsync() => SendProjectAiCoreAsync(null);

    /// <summary>
    /// 驱动真实发送路径：message 默认取 <see cref="ProjectAiMessage"/>；
    /// selectionOverride 非空时用于集成测试模拟编辑器选区。
    /// </summary>
    internal async Task SendProjectAiCoreAsync(EditorTextSelection? selectionOverride)
    {
        try
        {
            if (string.IsNullOrWhiteSpace(ProjectAiMessage))
            {
                StatusText = ProjectAiPlaceholder;
                return;
            }

            // 选区改写必须在编辑模式（阅读模式是 SelectableTextBlock，不会进入 RequestEditorSelection）。
            if (!IsEditMode)
            {
                IsEditMode = true;
            }

            var instruction = ProjectAiMessage.Trim();
            var documentContent = AssembleDocumentContent();
            var selection = selectionOverride ?? RequestEditorSelection?.Invoke();
            // 有编辑器选区时：选区改写走 quick_edit 结构化结果 + QuickEditSession 范围应用（作品页目标路径）。
            if (WorksEditorSelectionEdit.TryResolve(
                    documentContent,
                    selection,
                    out var selectionStart,
                    out var selectionEnd,
                    out var selectedText))
            {
                await SendProjectAiSelectionEditAsync(
                    instruction,
                    documentContent,
                    selectionStart,
                    selectionEnd,
                    selectedText).ConfigureAwait(true);
                return;
            }

            // 无选区：只问答，不改正文。最终 StatusText 必须保留选区提示，勿被「已配置」覆盖。
            var noSelectionHint = HasCurrentDocument
                ? _displayNames.Text("ui.works.project_ai.no_selection_hint")
                : null;

            var result = await _backend.ProjectAiChatAsync(
                instruction,
                workflowIdToRun: null,
                conversationId: ProjectAiConversationId,
                conversationRevision: _projectAiConversationRevision).ConfigureAwait(true);
            ProjectAiAnswer = result.Answer;
            _projectAiConversationRevision = ProjectAiConversationUi.Apply(
                result,
                _projectAiHistory,
                ProjectAiBubbles,
                _projectAiConversationRevision);
            OnPropertyChanged(nameof(HasProjectAiBubbles));
            ProjectAiMessage = string.Empty;
            StatusText = ProjectAiConversationUi.ContextWasCompacted(result)
                ? _displayNames.Text("ui.project_ai.context_compacted")
                : noSelectionHint ?? _displayNames.Text("ui.common.configured");
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    /// <summary>
    /// 选中一段正文后，项目 AI 指令经 quick_edit 生成替换稿，并仅写回该选区（带文档 id/版本守卫）。
    /// </summary>
    private async Task SendProjectAiSelectionEditAsync(
        string instruction,
        string documentContent,
        int selectionStart,
        int selectionEnd,
        string selectedText)
    {
        if (string.IsNullOrWhiteSpace(_currentDocumentId))
        {
            StatusText = NoDocumentText;
            return;
        }

        var documentId = _currentDocumentId!;
        var baseVersion = _currentDocumentVersion;
        var userBubble = WorksEditorSelectionEdit.FormatSelectionUserBubble(
            instruction,
            selectedText,
            _displayNames.Text("ui.works.project_ai.selection_context"));
        ProjectAiBubbles.Add(new ChatBubbleViewModel("user", userBubble));
        OnPropertyChanged(nameof(HasProjectAiBubbles));

        try
        {
            var result = await _backend.QuickEditAsync(new QuickEditRequest(
                selectedText,
                instruction,
                documentId)).ConfigureAwait(true);

            var liveContent = AssembleDocumentContent();
            var session = new QuickEditSession(
                documentId,
                baseVersion,
                documentContent,
                selectionStart,
                selectionEnd,
                result);

            if (!session.MatchesCurrent(_currentDocumentId, _currentDocumentVersion, liveContent)
                || !session.TryApply(
                    _currentDocumentId!,
                    _currentDocumentVersion,
                    liveContent,
                    out var updatedContent))
            {
                ProjectAiBubbles.Add(new ChatBubbleViewModel(
                    "assistant",
                    _displayNames.Text("ui.works.project_ai.selection_outdated")));
                ProjectAiAnswer = result.Suggested;
                StatusText = _displayNames.Text("ui.works.quick_edit.outdated");
                ProjectAiMessage = string.Empty;
                return;
            }

            // 再校验：选区外文本未动
            var prefixOk = selectionStart <= updatedContent.Length
                && string.Equals(liveContent[..selectionStart], updatedContent[..selectionStart], StringComparison.Ordinal);
            var suffixLen = liveContent.Length - selectionEnd;
            var suffixOk = suffixLen >= 0
                && updatedContent.Length >= suffixLen
                && string.Equals(liveContent[^suffixLen..], updatedContent[^suffixLen..], StringComparison.Ordinal);
            if (!prefixOk || !suffixOk)
            {
                ProjectAiBubbles.Add(new ChatBubbleViewModel(
                    "assistant",
                    _displayNames.Text("ui.works.project_ai.selection_outdated")));
                StatusText = _displayNames.Text("ui.works.quick_edit.outdated");
                ProjectAiMessage = string.Empty;
                return;
            }

            DocumentContent = updatedContent;
            MarkDocumentDirty();
            _quickEditUndo = new QuickEditUndoState(documentId, updatedContent, liveContent);
            UndoQuickEditCommand.NotifyCanExecuteChanged();
            ClearPendingQuickEdit();
            IsEditMode = true;
            IsNavTreeTab = false; // 留在项目 AI 页看到结果

            var assistantText = _displayNames.Format(
                "ui.works.project_ai.selection_applied_detail",
                new Dictionary<string, string>
                {
                    ["suggested"] = result.Suggested.Length > 400
                        ? result.Suggested[..397] + "…"
                        : result.Suggested,
                });
            ProjectAiBubbles.Add(new ChatBubbleViewModel("assistant", assistantText));
            ProjectAiAnswer = result.Suggested;
            _projectAiHistory.Add(new ProjectAiChatMessage("user", userBubble));
            _projectAiHistory.Add(new ProjectAiChatMessage("assistant", assistantText));
            OnPropertyChanged(nameof(HasProjectAiBubbles));
            ProjectAiMessage = string.Empty;
            StatusText = _displayNames.Text("ui.works.project_ai.selection_applied");
        }
        catch (Exception ex)
        {
            ProjectAiBubbles.Add(new ChatBubbleViewModel(
                "assistant",
                UserFacingError.Format(ex, _displayNames)));
            OnPropertyChanged(nameof(HasProjectAiBubbles));
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    /// <summary>测试用：打开一篇文档到可编辑状态，不经过后端树加载。</summary>
    internal void SeedOpenDocumentForTests(string documentId, string? version, string content)
    {
        ClearStickyEditorSelection?.Invoke();
        ClearSummaryState();
        _currentDocumentId = documentId;
        _currentDocumentPath = documentId;
        _currentDocumentVersion = version;
        DocumentTitle = documentId;
        IsEditMode = true;
        _suppressDirtyTracking = true;
        try
        {
            DocumentContent = content ?? string.Empty;
            CaptureSnapshot();
            HasUnsavedChanges = false;
            _documentDirty = false;
        }
        finally
        {
            _suppressDirtyTracking = false;
        }

        OnPropertyChanged(nameof(HasCurrentDocument));
        OnPropertyChanged(nameof(ShowDocumentChrome));
        OnPropertyChanged(nameof(ShowNoDocumentEmpty));
        OnPropertyChanged(nameof(DocumentSaveStateText));
        SaveCommand.NotifyCanExecuteChanged();
        OpenQuickEditCommand.NotifyCanExecuteChanged();
        InsertOutlineCommand.NotifyCanExecuteChanged();
        QuickAiCommand.NotifyCanExecuteChanged();
    }

    /// <summary>测试用：直接读取指定章节的正式总结投影。</summary>
    internal Task LoadChapterSummaryForTests(string chapterId) => LoadChapterSummaryAsync(chapterId);

    /// <summary>测试与调试：当前会话历史条数（用户+助手成对累积）。</summary>
    internal int ProjectAiHistoryCount => _projectAiHistory.Count;

    private void InsertOutlineReference()
    {
        if (string.IsNullOrWhiteSpace(_currentDocumentId))
        {
            StatusText = NoDocumentText;
            return;
        }
        IsEditMode = true;
        DocumentContent = AssembleDocumentContent() + Environment.NewLine + "@planning/outline.md";
        StatusText = OutlineText;
    }

    private void OpenImportPanel()
    {
        IsRightPanelOpen = true;
        IsNavTreeTab = true;
        IsImportPanelOpen = true;
        _ = EnsureImportProjectRootAsync();
        // 打开时若排序仍是默认 0，用章节数量作下一序号。
        if (ImportOrder is null or 0)
        {
            ImportOrder = Math.Max(0, CountWorksTreeChapters());
        }
    }

    private void ToggleImportPanel()
    {
        if (IsImportPanelOpen)
        {
            IsImportPanelOpen = false;
            return;
        }

        OpenImportPanel();
    }

    private async Task EnsureImportProjectRootAsync()
    {
        if (!string.IsNullOrWhiteSpace(_importProjectRoot) || !_backend.HasProjectRoot)
        {
            return;
        }

        try
        {
            var project = await _backend.GetCurrentProjectAsync().ConfigureAwait(true);
            if (project is not null
                && !string.Equals(_importProjectRoot, project.ProjectRoot, StringComparison.Ordinal))
            {
                _importProjectRoot = project.ProjectRoot;
                NotifyImportFormStateChanged();
            }
        }
        catch
        {
            // 相对路径仍可由后端安全处理；绝对路径保持字段错误，不吞掉为可提交状态。
        }
    }

    private static GridLength NormalizeRightPanelWidth(GridLength value)
    {
        if (value.IsStar)
        {
            return new GridLength(320);
        }
        var width = value.IsAuto ? 320 : value.Value;
        return new GridLength(Math.Clamp(width, MinRightPanelWidth, MaxRightPanelWidth));
    }

    private async Task QuickEditAsync()
    {
        var documentId = _currentDocumentId;
        var baseVersion = _currentDocumentVersion;
        var documentContent = AssembleDocumentContent();
        var instruction = QuickEditInstruction;
        var selection = RequestEditorSelection?.Invoke();
        var hasSelection = selection is { } currentSelection
                           && currentSelection.End > currentSelection.Start
                           && !string.IsNullOrWhiteSpace(currentSelection.Text);
        var selectionStart = hasSelection && selection is not null
            ? Math.Clamp(Math.Min(selection.Start, selection.End), 0, documentContent.Length)
            : 0;
        var selectionEnd = hasSelection && selection is not null
            ? Math.Clamp(Math.Max(selection.Start, selection.End), 0, documentContent.Length)
            : documentContent.Length;
        var selectedText = documentContent[selectionStart..selectionEnd];
        if (string.IsNullOrWhiteSpace(documentId)
            || string.IsNullOrWhiteSpace(selectedText)
            || string.IsNullOrWhiteSpace(instruction))
        {
            StatusText = QuickAiHint;
            return;
        }

        InvalidateQuickEditGeneration();
        var generation = Interlocked.Increment(ref _quickEditGeneration);
        var cancellation = new CancellationTokenSource();
        _quickEditGenerationCts = cancellation;
        IsQuickEditGenerating = true;
        try
        {
            var result = await _backend.QuickEditAsync(new QuickEditRequest(
                selectedText,
                instruction,
                documentId), cancellation.Token).ConfigureAwait(true);
            if (generation != Volatile.Read(ref _quickEditGeneration)
                || cancellation.IsCancellationRequested)
            {
                return;
            }

            var session = new QuickEditSession(
                documentId,
                baseVersion,
                documentContent,
                selectionStart,
                selectionEnd,
                result);
            if (!session.MatchesCurrent(
                    _currentDocumentId,
                    _currentDocumentVersion,
                    AssembleDocumentContent()))
            {
                StatusText = _displayNames.Text("ui.works.quick_edit.outdated");
                return;
            }

            _pendingQuickEdit = session;
            ApplyQuickEditCommand.NotifyCanExecuteChanged();
            var preview = QuickEditPreviewBuilder.Build(result.Diff);
            QuickEditDiff = preview.Text;
            StatusText = _displayNames.Text(preview.IsTruncated
                ? "ui.works.quick_edit.preview_truncated"
                : "ui.works.quick_edit.ready");
        }
        catch (OperationCanceledException) when (cancellation.IsCancellationRequested)
        {
            // 文档切换、继续编辑或新一轮生成已使本次结果失效。
        }
        catch (Exception ex)
        {
            if (generation == Volatile.Read(ref _quickEditGeneration))
            {
                StatusText = UserFacingError.Format(ex, _displayNames);
            }
        }
        finally
        {
            if (generation == Volatile.Read(ref _quickEditGeneration))
            {
                _quickEditGenerationCts?.Dispose();
                _quickEditGenerationCts = null;
                IsQuickEditGenerating = false;
            }
        }
    }

    private void ApplyQuickEdit()
    {
        if (_pendingQuickEdit is null)
        {
            StatusText = QuickAiHint;
            return;
        }
        if (string.IsNullOrWhiteSpace(_currentDocumentId))
        {
            StatusText = NoDocumentText;
            return;
        }
        var documentContent = AssembleDocumentContent();
        if (!_pendingQuickEdit.TryApply(
                _currentDocumentId,
                _currentDocumentVersion,
                documentContent,
                out var updatedContent))
        {
            InvalidateQuickEditGeneration();
            StatusText = _displayNames.Text("ui.works.quick_edit.outdated");
            return;
        }

        DocumentContent = updatedContent;
        MarkDocumentDirty();
        _quickEditUndo = new QuickEditUndoState(
            _currentDocumentId,
            updatedContent,
            documentContent);
        UndoQuickEditCommand.NotifyCanExecuteChanged();
        StatusText = _displayNames.Text("ui.works.quick_edit.applied_locally");
    }

    private void UndoQuickEdit()
    {
        if (_quickEditUndo is null
            || !_quickEditUndo.TryUndo(
                _currentDocumentId,
                AssembleDocumentContent(),
                out var restoredContent))
        {
            ClearQuickEditUndo();
            StatusText = _displayNames.Text("ui.works.quick_edit.undo_unavailable");
            return;
        }

        DocumentContent = restoredContent;
        ClearQuickEditUndo();
        StatusText = _displayNames.Text("ui.works.quick_edit.undone");
    }

    public string UnsavedChangesPageTitle => _displayNames.Text("ui.nav.works");
    public string UnsavedChangesPageId => "works";
    public string? PreparedUnsavedChangesPayloadIdentity => _preparedContent is null
        ? null
        : BatchLeaveSaveCoordinator.CreatePayloadIdentity(JsonSerializer.Serialize(new
        {
            DocumentId = _preparedDocumentId,
            Version = _preparedVersion,
            Content = _preparedContent,
        }));

    public async Task<bool> ConfirmLeaveIfNeededAsync()
    {
        if (!HasUnsavedChanges)
        {
            return true;
        }

        var choice = await DialogService.Current.ConfirmUnsavedLeaveAsync(UnsavedChangesPageTitle).ConfigureAwait(true);
        switch (choice)
        {
            case UnsavedLeaveChoice.Save:
                return await SaveUnsavedChangesAsync().ConfigureAwait(true);
            case UnsavedLeaveChoice.Discard:
                await DiscardUnsavedChangesAsync().ConfigureAwait(true);
                return true;
            default:
                return false;
        }
    }

    private bool _leavePrepared;
    private string? _preparedDocumentId;
    private string? _preparedContent;
    private string? _preparedVersion;

    public Task<bool> PrepareUnsavedChangesAsync()
    {
        ClearPreparedLeave();
        if (!HasUnsavedChanges)
        {
            _leavePrepared = true;
            return Task.FromResult(true);
        }

        if (!HasUnsavedDocumentChanges)
        {
            _leavePrepared = true;
            return Task.FromResult(true);
        }

        if (string.IsNullOrWhiteSpace(_currentDocumentId))
        {
            return Task.FromResult(false);
        }

        _preparedDocumentId = _currentDocumentId;
        _preparedContent = AssembleDocumentContent();
        _preparedVersion = _currentDocumentVersion;
        _leavePrepared = true;
        return Task.FromResult(true);
    }

    public async Task<bool> CommitPreparedUnsavedChangesAsync()
    {
        if (!_leavePrepared)
        {
            return false;
        }

        if (!HasUnsavedChanges || string.IsNullOrWhiteSpace(_preparedDocumentId))
        {
            ClearPreparedLeave();
            return true;
        }

        var preparedDocumentId = _preparedDocumentId;
        var preparedContent = _preparedContent ?? string.Empty;
        var preparedVersion = _preparedVersion;
        if (!string.Equals(_currentDocumentId, preparedDocumentId, StringComparison.Ordinal)
            || !string.Equals(_currentDocumentVersion, preparedVersion, StringComparison.Ordinal)
            || !string.Equals(AssembleDocumentContent(), preparedContent, StringComparison.Ordinal))
        {
            ClearPreparedLeave();
            return false;
        }

        try
        {
            // Commit prepared payload only (not whatever the editor currently holds).
            var report = await _backend.SaveDocumentContentAsync(
                preparedDocumentId,
                preparedContent,
                preparedVersion).ConfigureAwait(true);
            if (!string.Equals(_currentDocumentId, preparedDocumentId, StringComparison.Ordinal)
                || !string.Equals(_currentDocumentVersion, preparedVersion, StringComparison.Ordinal))
            {
                ClearPreparedLeave();
                return false;
            }
            _currentDocumentPath = report.Metadata.Path;
            _currentDocumentVersion = report.Metadata.Version;
            OnPropertyChanged(nameof(DocumentInfoText));
            AcceptSavedDocumentSnapshot(preparedContent);
            ClearPreparedLeave();
            return !HasUnsavedChanges;
        }
        catch
        {
            return false;
        }
    }

    public Task AbortPreparedUnsavedChangesAsync()
    {
        ClearPreparedLeave();
        return Task.CompletedTask;
    }

    public async Task<bool> SaveUnsavedChangesAsync()
    {
        if (!await PrepareUnsavedChangesAsync().ConfigureAwait(true))
        {
            return false;
        }

        return await CommitPreparedUnsavedChangesAsync().ConfigureAwait(true);
    }

    public async Task DiscardUnsavedChangesAsync()
    {
        await AbortPreparedUnsavedChangesAsync().ConfigureAwait(true);
        if (HasUnsavedChanges)
        {
            RestoreSnapshot();
        }
    }

    public async Task ReloadProjectDataAsync(CancellationToken cancellationToken = default)
    {
        await _projectAutomation.EnsureLoadedAsync(cancellationToken).ConfigureAwait(true);
        cancellationToken.ThrowIfCancellationRequested();
        InvalidateQuickEditGeneration();
        ClearQuickEditUndo();
        Interlocked.Increment(ref _documentLoadGeneration);
        _documentLoadCts?.Cancel();
        _documentLoadCts?.Dispose();
        _documentLoadCts = null;
        IsDocumentLoading = false;
        _documentLoadingTarget = string.Empty;
        OnPropertyChanged(nameof(DocumentLoadingTargetText));
        var documentGeneration = _documentLoadGeneration;
        var summaryChapterId = _currentSummaryChapterId;
        Interlocked.Increment(ref _summaryLoadGeneration);
        _summaryLoadCts?.Cancel();
        _summaryLoadCts?.Dispose();
        _summaryLoadCts = null;
        IsSummaryLoading = false;
        await LoadWorksTreeAsync(cancellationToken).ConfigureAwait(true);
        cancellationToken.ThrowIfCancellationRequested();
        if (string.IsNullOrWhiteSpace(_currentDocumentId))
        {
            ClearSummaryState();
            return;
        }

        try
        {
            _suppressDirtyTracking = true;
            var document = await _backend.GetDocumentContentDetailsAsync(_currentDocumentId, cancellationToken).ConfigureAwait(true);
            cancellationToken.ThrowIfCancellationRequested();
            if (documentGeneration != _documentLoadGeneration)
            {
                return;
            }
            DocumentContent = document.Content;
            _currentDocumentPath = document.Metadata.Path;
            _currentDocumentVersion = document.Metadata.Version;
            DocumentTitle = Path.GetFileNameWithoutExtension(document.Metadata.Path);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            throw;
        }
        catch (Exception ex)
        {
            if (documentGeneration == _documentLoadGeneration)
            {
                ClearStickyEditorSelection?.Invoke();
                _currentDocumentId = string.Empty;
                _currentDocumentPath = string.Empty;
                _currentDocumentVersion = null;
                DocumentContent = string.Empty;
                DocumentTitle = NoDocumentText;
                ClearSummaryState();
                StatusText = UserFacingError.Format(ex, _displayNames);
            }
        }
        finally
        {
            _suppressDirtyTracking = false;
        }
        OnCurrentDocumentChanged();
        CaptureSnapshot();
        if (_backend.HasProjectRoot
            && !string.IsNullOrWhiteSpace(summaryChapterId)
            && HasCurrentDocument)
        {
            cancellationToken.ThrowIfCancellationRequested();
            await LoadChapterSummaryAsync(summaryChapterId).ConfigureAwait(true);
        }
    }

    public void DeactivateProjectData()
    {
        InvalidateQuickEditGeneration();
        Interlocked.Increment(ref _worksTreeLoadGeneration);
        _worksTreeLoadCts?.Cancel();
        _worksTreeLoadCts?.Dispose();
        _worksTreeLoadCts = null;
        Interlocked.Increment(ref _documentLoadGeneration);
        _documentLoadCts?.Cancel();
        _documentLoadCts?.Dispose();
        _documentLoadCts = null;
        IsDocumentLoading = false;
        _documentLoadingTarget = string.Empty;
        OnPropertyChanged(nameof(DocumentLoadingTargetText));
        _summaryLoadCts?.Cancel();
        _summaryLoadCts?.Dispose();
        _summaryLoadCts = null;
        Interlocked.Increment(ref _summaryLoadGeneration);
        _expandedWorksTreeNodeIds.Clear();
        _worksTreeExpansionInitialized = false;
        WorksTreeSearchText = string.Empty;
        SetSelectedWorksTreeNode(null, navigate: false);
        SetCurrentWorksTreeNode(null);
        _importProjectRoot = string.Empty;
        AllowImportOverwrite = false;
        NotifyImportFormStateChanged();
    }

    private void CaptureSnapshot()
    {
        _savedSnapshot = AssembleDocumentContent();
        _documentDirty = false;
        HasUnsavedChanges = false;
        RefreshSummarySourceFreshness();
    }

    private void AcceptSavedDocumentSnapshot(string submittedContent)
    {
        _savedSnapshot = submittedContent;
        _documentDirty = !string.Equals(AssembleDocumentContent(), submittedContent, StringComparison.Ordinal);
        HasUnsavedChanges = _documentDirty;
        RefreshSummarySourceFreshness();
    }

    private void ClearPreparedLeave()
    {
        _leavePrepared = false;
        _preparedDocumentId = null;
        _preparedContent = null;
        _preparedVersion = null;
    }

    private void RestoreSnapshot()
    {
        _suppressDirtyTracking = true;
        try
        {
            DocumentContent = _savedSnapshot;
            InvalidateQuickEditGeneration();
            ClearQuickEditUndo();
            _documentDirty = false;
            RefreshDirtyState();
        }
        finally
        {
            _suppressDirtyTracking = false;
        }
    }

    private void RefreshDirtyState()
    {
        HasUnsavedChanges = HasUnsavedDocumentChanges;
        RefreshSummarySourceFreshness();
    }

    private bool HasUnsavedDocumentChanges => _documentDirty || AssembleDocumentContent() != _savedSnapshot;

    private static string ShortValue(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return "-";
        }
        return value.Length <= 12 ? value : value[..12];
    }

    protected override void OnPropertyChanged(string? propertyName = null)
    {
        base.OnPropertyChanged(propertyName);
        if (!_suppressDirtyTracking
            && propertyName is nameof(DocumentContent))
        {
            MarkDocumentDirty();
        }
    }

    private WorksTreeItemViewModel BuildWorksTree(
        WorksTreeNode node,
        WorksTreeItemViewModel? parent,
        Dictionary<string, WorksTreeItemViewModel> nodesById)
    {
        if (string.IsNullOrWhiteSpace(node.NodeId))
        {
            throw new InvalidDataException("works tree node id must not be empty");
        }

        var title = node.Title.StartsWith("ui.", StringComparison.Ordinal)
            ? _displayNames.Text(node.Title)
            : node.Title;
        if (string.Equals(node.Kind, "stage_outline", StringComparison.Ordinal)
            && !string.IsNullOrWhiteSpace(node.StageId))
        {
            title = _displayNames.Format(
                "ui.works.stage_title",
                new Dictionary<string, string> { ["title"] = title });
        }

        var kindLabel = WorksTreeKindLabel(node.Kind);
        var accessibleName = _displayNames.Format(
            "ui.works.tree_node_accessible",
            new Dictionary<string, string>
            {
                ["kind"] = kindLabel,
                ["title"] = title,
            });
        var isExpanded = node.Children.Count > 0
                         && (!_worksTreeExpansionInitialized
                             || _expandedWorksTreeNodeIds.Contains(node.NodeId));
        WorksTreeItemViewModel? item = null;
        item = new WorksTreeItemViewModel(
            node.NodeId,
            title,
            node.Path,
            () => ActivateWorksTreeNode(item!),
            node.Kind,
            node.ChapterId,
            node.StageId,
            !string.IsNullOrWhiteSpace(node.Path),
            parent,
            kindLabel,
            accessibleName,
            isExpanded,
            OnWorksTreeExpansionChanged);
        if (!nodesById.TryAdd(item.NodeId, item))
        {
            throw new InvalidDataException($"duplicate works tree node id: {item.NodeId}");
        }
        if (isExpanded)
        {
            _expandedWorksTreeNodeIds.Add(item.NodeId);
        }

        foreach (var child in node.Children)
        {
            item.Children.Add(BuildWorksTree(child, item, nodesById));
        }
        item.ResetVisibleChildren();
        return item;
    }

    private string WorksTreeKindLabel(string kind) => kind switch
    {
        "global_outline" or "root" => _displayNames.Text("ui.works.tree_kind.global_outline"),
        "stage_outline" => _displayNames.Text("ui.works.tree_kind.stage_outline"),
        "chapter" or "document" => _displayNames.Text("ui.works.tree_kind.chapter"),
        _ => _displayNames.Text("ui.common.unknown"),
    };

    private void ReplaceWorksTree(
        IReadOnlyList<WorksTreeItemViewModel> roots,
        Dictionary<string, WorksTreeItemViewModel> nodesById)
    {
        var selectedNodeId = SelectedWorksTreeNode?.NodeId;
        WorksTreeRoots.Clear();
        foreach (var root in roots)
        {
            WorksTreeRoots.Add(root);
        }
        ApplyWorksTreeSearch();
        SetSelectedWorksTreeNode(
            selectedNodeId is not null && nodesById.TryGetValue(selectedNodeId, out var selected)
                ? selected
                : null,
            navigate: false);
        ExportCommand.NotifyCanExecuteChanged();
    }

    private void ApplyWorksTreeSearch()
    {
        var query = WorksTreeSearchText.Trim();
        VisibleWorksTreeRoots.Clear();
        foreach (var root in WorksTreeRoots)
        {
            if (root.ApplyTitleFilter(query))
            {
                VisibleWorksTreeRoots.Add(root);
            }
        }
        OnPropertyChanged(nameof(IsWorksTreeSearchActive));
        OnPropertyChanged(nameof(ShowWorksTreeSearchEmpty));
    }

    private IEnumerable<WorksTreeItemViewModel> EnumerateWorksTreeNodes()
    {
        foreach (var root in WorksTreeRoots)
        {
            foreach (var node in root.EnumerateSubtree())
            {
                yield return node;
            }
        }
    }

    private int CountWorksTreeChapters() => EnumerateWorksTreeNodes().Count(node => node.IsChapter);

    private void OnWorksTreeExpansionChanged(WorksTreeItemViewModel item, bool isExpanded)
    {
        if (isExpanded)
        {
            _expandedWorksTreeNodeIds.Add(item.NodeId);
        }
        else
        {
            _expandedWorksTreeNodeIds.Remove(item.NodeId);
        }
    }

    private void ActivateWorksTreeNode(WorksTreeItemViewModel item)
    {
        SetSelectedWorksTreeNode(item, navigate: false);
        if (item.CanOpen)
        {
            _ = LoadDocumentAsync(item);
        }
    }

    private void SetSelectedWorksTreeNode(WorksTreeItemViewModel? item, bool navigate)
    {
        _suppressWorksTreeSelectionNavigation = !navigate;
        try
        {
            SelectedWorksTreeNode = item;
        }
        finally
        {
            _suppressWorksTreeSelectionNavigation = false;
        }
    }

    private void SetCurrentWorksTreeNode(WorksTreeItemViewModel? item)
    {
        if (ReferenceEquals(_currentWorksTreeNode, item))
        {
            return;
        }
        if (_currentWorksTreeNode is not null)
        {
            _currentWorksTreeNode.IsCurrentDocument = false;
        }
        _currentWorksTreeNode = item;
        if (_currentWorksTreeNode is not null)
        {
            _currentWorksTreeNode.IsCurrentDocument = true;
        }
    }

    private void RefreshCurrentWorksTreeNode()
    {
        if (string.IsNullOrWhiteSpace(_currentDocumentId))
        {
            SetCurrentWorksTreeNode(null);
            return;
        }

        var current = EnumerateWorksTreeNodes().FirstOrDefault(item =>
            item.CanOpen
            && string.Equals(
                ProjectRelativePath(item.Path),
                _currentDocumentId,
                StringComparison.Ordinal));
        SetCurrentWorksTreeNode(current);
        if (SelectedWorksTreeNode is null && current is not null)
        {
            SetSelectedWorksTreeNode(current, navigate: false);
        }
    }

    private void RestoreWorksTreeSelectionAndCurrentDocument()
    {
        RefreshCurrentWorksTreeNode();
    }

    private static string ProjectRelativePath(string path) =>
        ProjectPathHelper.ToProjectRelativePath(path);

}

public sealed record ExportFormatOption(string Value, string Label);

public sealed record EditorTextSelection(int Start, int End, string Text);

/// <summary>只读模式的虚拟化投影；不再承担编辑、选区或光标状态。</summary>
public sealed class DocumentBlockViewModel
{
    public DocumentBlockViewModel(
        string id,
        int index,
        string text)
    {
        Id = id;
        Index = index;
        Text = text;
    }

    public string Id { get; }
    public int Index { get; }
    public string Text { get; }
}

public sealed class WorksTreeItemViewModel : ViewModelBase
{
    private readonly Action<WorksTreeItemViewModel, bool>? _expansionChanged;
    private bool _isExpanded;
    private bool? _expandedBeforeSearch;
    private bool _isCurrentDocument;

    public WorksTreeItemViewModel(
        string nodeId,
        string title,
        string path,
        Action open,
        string kind = "",
        string? chapterId = null,
        string? stageId = null,
        bool canOpen = true,
        WorksTreeItemViewModel? parent = null,
        string kindLabel = "",
        string accessibleName = "",
        bool isExpanded = false,
        Action<WorksTreeItemViewModel, bool>? expansionChanged = null)
    {
        NodeId = nodeId;
        Title = title;
        Path = path;
        Kind = kind;
        ChapterId = chapterId;
        StageId = stageId;
        CanOpen = canOpen;
        Parent = parent;
        KindLabel = kindLabel;
        AccessibleName = accessibleName;
        _isExpanded = isExpanded;
        _expansionChanged = expansionChanged;
        Children = new ObservableCollection<WorksTreeItemViewModel>();
        VisibleChildren = new ObservableCollection<WorksTreeItemViewModel>();
        OpenCommand = new RelayCommand(open, () => CanOpen);
    }

    public string NodeId { get; }
    public string Title { get; }
    public string Path { get; }
    public string Kind { get; }
    public string? ChapterId { get; }
    public string? StageId { get; }
    public bool CanOpen { get; }
    public WorksTreeItemViewModel? Parent { get; }
    public string KindLabel { get; }
    public string AccessibleName { get; }
    public ObservableCollection<WorksTreeItemViewModel> Children { get; }
    public ObservableCollection<WorksTreeItemViewModel> VisibleChildren { get; }
    public bool HasChildren => Children.Count > 0;
    public bool IsGlobalOutline => Kind is "global_outline" or "root";
    public bool IsStageOutline => Kind == "stage_outline";
    public bool IsChapter => Kind is "chapter" or "document";
    public bool HasPath => !string.IsNullOrWhiteSpace(Path);
    public string DisplayPath => Path.Replace('\\', '/');
    public RelayCommand OpenCommand { get; }

    public bool IsExpanded
    {
        get => _isExpanded;
        set
        {
            if (SetProperty(ref _isExpanded, value) && _expandedBeforeSearch is null)
            {
                _expansionChanged?.Invoke(this, value);
            }
        }
    }

    public bool IsCurrentDocument
    {
        get => _isCurrentDocument;
        internal set => SetProperty(ref _isCurrentDocument, value);
    }

    public IEnumerable<WorksTreeItemViewModel> EnumerateSubtree()
    {
        yield return this;
        foreach (var child in Children)
        {
            foreach (var descendant in child.EnumerateSubtree())
            {
                yield return descendant;
            }
        }
    }

    public void ResetVisibleChildren()
    {
        ReplaceVisibleChildren(Children);
        foreach (var child in Children)
        {
            child.ResetVisibleChildren();
        }
    }

    public bool ApplyTitleFilter(string query)
    {
        if (string.IsNullOrWhiteSpace(query))
        {
            RestoreExpansionAfterSearch();
            ResetVisibleChildren();
            return true;
        }

        BeginSearch();
        if (Title.Contains(query, StringComparison.CurrentCultureIgnoreCase))
        {
            foreach (var child in Children)
            {
                child.ShowFullSubtreeForSearch();
            }
            ReplaceVisibleChildren(Children);
            SetExpandedForSearch(Children.Count > 0 || IsExpanded);
            return true;
        }

        var matchingChildren = new List<WorksTreeItemViewModel>();
        foreach (var child in Children)
        {
            if (child.ApplyTitleFilter(query))
            {
                matchingChildren.Add(child);
            }
        }
        ReplaceVisibleChildren(matchingChildren);
        if (matchingChildren.Count > 0)
        {
            SetExpandedForSearch(true);
            return true;
        }
        return false;
    }

    private void ShowFullSubtreeForSearch()
    {
        BeginSearch();
        ReplaceVisibleChildren(Children);
        foreach (var child in Children)
        {
            child.ShowFullSubtreeForSearch();
        }
    }

    private void BeginSearch()
    {
        _expandedBeforeSearch ??= _isExpanded;
    }

    private void RestoreExpansionAfterSearch()
    {
        if (_expandedBeforeSearch is { } expanded)
        {
            _expandedBeforeSearch = null;
            SetProperty(ref _isExpanded, expanded, nameof(IsExpanded));
        }
        foreach (var child in Children)
        {
            child.RestoreExpansionAfterSearch();
        }
    }

    private void SetExpandedForSearch(bool value)
    {
        SetProperty(ref _isExpanded, value, nameof(IsExpanded));
    }

    private void ReplaceVisibleChildren(IEnumerable<WorksTreeItemViewModel> children)
    {
        VisibleChildren.Clear();
        foreach (var child in children)
        {
            VisibleChildren.Add(child);
        }
    }
}

public sealed class WorksSummarySegmentItemViewModel : ViewModelBase
{
    private bool _isSourceFresh;
    private bool _isSelected;
    private string _sourceStateText = string.Empty;

    public WorksSummarySegmentItemViewModel(
        StorySegmentView segment,
        string title,
        string summary,
        string sourceText,
        string revealText,
        Action reveal)
    {
        Segment = segment;
        Title = title;
        Summary = summary;
        SourceText = sourceText;
        RevealText = revealText;
        RevealCommand = new RelayCommand(reveal);
    }

    public StorySegmentView Segment { get; }
    public WritingSourceSpan Source => Segment.Source;
    public string Title { get; }
    public string Summary { get; }
    public string SourceText { get; }
    public string RevealText { get; }
    public RelayCommand RevealCommand { get; }

    public bool IsSourceFresh
    {
        get => _isSourceFresh;
        private set => SetProperty(ref _isSourceFresh, value);
    }

    public string SourceStateText
    {
        get => _sourceStateText;
        private set => SetProperty(ref _sourceStateText, value);
    }

    public bool IsSelected
    {
        get => _isSelected;
        private set => SetProperty(ref _isSelected, value);
    }

    public void UpdateSourceState(bool isFresh, string stateText)
    {
        IsSourceFresh = isFresh;
        SourceStateText = stateText;
    }

    public void UpdateSelected(bool isSelected)
    {
        IsSelected = isSelected;
    }
}

public sealed record WorksSummaryDetailItemViewModel(
    string Title,
    string Content,
    string StatusText,
    IReadOnlyList<string>? RelatedSegmentIds = null);
