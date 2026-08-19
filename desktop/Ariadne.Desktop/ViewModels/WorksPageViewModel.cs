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
    /// U184-A：正文搜索防抖。标题那路本地即时，正文这路每次请求都要抢项目互斥锁并跑
    /// 知识同步，逐字符发请求会让后端排队；280ms 落在「打完一个词的停顿」量级。
    private const int BodySearchDebounceMs = 280;
    /// 上限取后端 `MAX_PRODUCT_SEARCH_LIMIT`（50）以内的一个稳妥值：
    /// 侧栏列表读不完更多，而后端还有整体字节预算校验（`validate_product_search_result_budget`）。
    private const int BodySearchLimit = 20;
    /// 片段裁剪长度。chunk 默认上千字，整段渲染会把命中列表冲掉。
    private const int BodySearchSnippetChars = 160;

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
    // U131：章节大纲对照栏。原实现把 "@planning/outline.md" 追加进正文，
    // 用户按 Ctrl+S 就把这行垃圾持久化进小说；且那个路径全后端零存在
    // （真实约定是 planning/chapters/{id}.md）。改为只读对照，绝不碰正文。
    private bool _isOutlinePanelOpen;
    private string _chapterOutlineText = string.Empty;
    private bool _isOutlineLoading;
    private CancellationTokenSource? _outlineLoadCts;
    private string _documentTitle;
    private string _importChapterId = string.Empty;
    private string _importChapterTitle = string.Empty;
    private decimal? _importOrder = 0m;
    private string _importSourcePath = string.Empty;
    private string _importTargetPath = string.Empty;
    private string _importProjectRoot = string.Empty;
    private bool _allowImportOverwrite;
    /// U174：面板处于「新建章节」模式还是「导入稿件」模式。
    ///
    /// 两种模式**共用同一套表单字段与校验**（章节 ID / 标题 / 排序 / 目标路径），
    /// 唯一差别是正文来源：导入多一个源文件字段，新建不需要。
    /// 刻意不做成两套表单：路径规则完全相同，复制一套必然漂移
    /// （U174 文档已就 `ui.works.import.error.*` 复用做过同样的判断）。
    private bool _isCreateChapterMode;
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
    private bool _isQuickEditOpen;
    /// <summary>
    /// U129：跨视图切换保留的阅读位置（全篇字符偏移）。
    ///
    /// 阅读侧与编辑器侧的滚动坐标系不可直接互换，字符偏移是唯一的共同量。
    /// 换文档时必须清空（<see cref="ReadingPositionMapper.ClearOnDocumentChange"/>）——
    /// 旧偏移套到新正文上会滚到毫无关系的位置，比「从头开始」更难被理解成 bug。
    /// </summary>
    private int? _readingOffsetAnchor;
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
    // U184-A：正文搜索（后端 Tantivy）与标题搜索并行的那一路状态。
    // 标题那路是本地即时的；这一路走 IPC，所以要有防抖、代际、取消与暂态三件套。
    private CancellationTokenSource? _bodySearchCts;
    private long _bodySearchGeneration;
    private BodySearchState _bodySearchState = BodySearchState.Idle;
    private string _bodySearchQuery = string.Empty;
    private string _bodySearchErrorText = string.Empty;
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

    /// <summary>
    /// U184-A：正文搜索这一路的状态。
    ///
    /// <para><see cref="Indexing"/> **刻意不是 Error 的一个子类**：它是「索引还没追上
    /// 刚才的保存」这个可重试暂态（后端 <c>ensure_search_not_blocked_by_pending_index</c>），
    /// 作者改完一章立刻搜必然撞上。把它渲染成红色报错，作者会得出
    /// 「搜索坏了」的结论，效果与缺陷版本的「永远 0 结果」一样糟。</para>
    /// </summary>
    private enum BodySearchState
    {
        /// 没在搜（查询为空，或已清空）。
        Idle,
        /// 正在等后端。
        Searching,
        /// 搜完了（可能 0 命中）。
        Done,
        /// 索引正在追赶，等几秒可重试。
        Indexing,
        /// 真的失败了。
        Failed,
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
        // U184-A：正文命中单独一组。刻意**不**混进 VisibleWorksTreeRoots——
        // 「标题含关键词」与「正文含关键词」是不同语义，混在一棵树里作者
        // 无法判断某章为什么出现在结果里。
        BodySearchHits = new ObservableCollection<WorksBodySearchHitViewModel>();
        // U145：导入表单的章节 ID 候选。取值集合就是作品树里已有的章节
        // （后端 ChapterDocumentIndex），此前是自由文本框——打错一个字符不会报错，
        // 只是导入进一个谁也不会去读的孤儿章节 id。
        ImportChapterIdCandidates = new ObservableCollection<string>();
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
        // U174：「新建章节」此前在全应用不存在——后端无命令、前端无入口、语言包无文案，
        // 于是作者拿到空项目后必须先在项目外手工造一个 .md 再导入。
        OpenCreateChapterPanelCommand = new RelayCommand(OpenCreateChapterPanel);
        CreateChapterCommand = new RelayCommand(() => _ = CreateChapterAsync(), CanCreateChapter);
        ExportCommand = new RelayCommand(() => _ = ExportAsync(), () => WorksTreeRoots.Count > 0);
        SaveCommand = new RelayCommand(() => _ = SaveAsync(), () => HasCurrentDocument && !IsDocumentSaving);
        RetryWorksTreeCommand = new RelayCommand(() => _ = LoadWorksTreeAsync(), () => IsWorksTreeError && !IsWorksTreeLoading);
        // U184-A：正文搜索重试。撞上索引门禁时作者需要一个「现在再试一次」的动作——
        // 只给一句「稍后再试」而不给按钮，等于让他重新打一遍关键词。
        RetryBodySearchCommand = new RelayCommand(
            () => StartBodySearch(WorksTreeSearchText, immediate: true),
            () => _bodySearchState is BodySearchState.Indexing or BodySearchState.Failed);
        ReadModeCommand = new RelayCommand(() => IsEditMode = false);
        EditModeCommand = new RelayCommand(() => IsEditMode = true);
        CopyCommand = new RelayCommand(() => RequestEditorCopy?.Invoke());
        SelectAllCommand = new RelayCommand(() => RequestEditorSelectAll?.Invoke());
        OpenQuickEditCommand = new RelayCommand(OpenQuickEdit, CanOpenQuickEdit);
        CloseQuickEditCommand = new RelayCommand(CloseQuickEdit, () => !IsQuickEditGenerating);
        QuickAiCommand = new RelayCommand(() =>
        {
            // U130：不再顺手把用户推进修改模式。改写结果落在 diff 预览里，
            // 阅读态照样能看能应用；「应用建议」才需要编辑器承接，那一步再切。
            IsQuickEditOpen = true;
            _ = QuickEditAsync();
        }, CanGenerateQuickEdit);
        ToggleOutlinePanelCommand = new RelayCommand(ToggleOutlinePanel, () => HasCurrentDocument);
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

    /// <summary>
    /// 设置项「作品页右栏默认展开」的当前值。
    ///
    /// U133：它现在**只作默认值**用（<see cref="ApplyUiPreferences"/> 里在没有
    /// 保存过的开合状态时取它），不再参与任何可见性判断。此前它同时决定
    /// 「右栏是否可见」与「药丸是否可见」，后者让用户在页面内彻底失去恢复手段。
    /// 保留为公开属性是为了设置页改动后能立即反映；私有 setter 保证只由偏好驱动。
    /// </summary>
    public bool IsProjectPanelVisible
    {
        get => _isProjectPanelVisible;
        private set => SetProperty(ref _isProjectPanelVisible, value);
    }

    /// <summary>
    /// 收展药丸（页面内唯一的右栏开合入口）是否可见。
    ///
    /// U133：**恒为 true，刻意不再看 <see cref="IsProjectPanelVisible"/>**。
    /// 此前它是 `IsProjectPanelVisible || IsImportPanelOpen`，于是设置里关掉那个
    /// 开关后药丸一起消失、`ToggleRightPanelCommand.CanExecute()` 返回 false——
    /// **在作品页内没有任何办法把导航树叫回来**，只能回设置页重新勾选。
    /// 设置项该决定的是「进页面时默认收着还是展开」，不是「剥夺开合能力」：
    /// 一个个性化偏好不该把功能锁死。
    ///
    /// 也不按「作品树是否为空」判断——右栏有两个标签，项目 AI 在没有任何章节时
    /// 照样可用（正是从零开始那一刻最需要问 AI 的时候）。
    /// 保留这个属性而不是删掉，是因为三页共用的 <c>RightPanelTogglePill</c>
    /// 都绑它，各页的判据未必一致。
    /// </summary>
    public bool IsRightPanelToggleVisible => true;

    /// U131：章节大纲对照栏是否展开。与正文并列显示，只读。
    public bool IsOutlinePanelOpen
    {
        get => _isOutlinePanelOpen;
        set
        {
            if (SetProperty(ref _isOutlinePanelOpen, value))
            {
                OnPropertyChanged(nameof(OutlinePanelWidth));
                OnPropertyChanged(nameof(DocumentSurfaceMaxWidth));
            }
        }
    }

    /// 对照栏宽度；关闭时为 0，配合 IsVisible 彻底不占版面。
    public double OutlinePanelWidth => IsOutlinePanelOpen ? 320d : 0d;

    /// <summary>
    /// 稿纸外框宽度上限。
    ///
    /// U136/U140：基准 720 = 正文测量宽 576（16px × 36 字）+ 左右内边距 144。
    /// 对照栏展开时**必须把纸加宽**而不是让正文让位——否则版心被压到 372px、
    /// 每行只剩 23 个字，读起来比 65 字/行还难受（那正是这次要修的方向的反面）。
    /// 加的量 = 对照栏宽 320 + 它与正文之间的 28 间距 + 20 内缩，与 XAML 里
    /// 那个 Border 的 Margin/Padding 对齐；两处改动必须同步，否则纸宽与实际
    /// 占位对不上，正文会被悄悄挤窄。
    /// </summary>
    public double DocumentSurfaceMaxWidth => IsOutlinePanelOpen ? 720d + 368d : 720d;

    /// 当前章节的大纲正文；找不到时是可诊断文案而非空串。
    public string ChapterOutlineText
    {
        get => _chapterOutlineText;
        private set => SetProperty(ref _chapterOutlineText, value);
    }

    public bool IsOutlineLoading
    {
        get => _isOutlineLoading;
        private set => SetProperty(ref _isOutlineLoading, value);
    }

    /// 对照栏标题。
    public string OutlineCompareText => _displayNames.Text("ui.works.outline_compare");

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
        // U133：**去掉 `!IsProjectPanelVisible` 的早退**。此前设置项关着时页面内的
        // 开合不落盘，于是「在页面里展开 → 切页回来又收起了」——用户会以为
        // 展开失败。设置项是**默认值**，页面内的手动开合是更晚的、更明确的用户意图，
        // 该覆盖它并记住。
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

    /// <summary>
    /// U174：打开「新建章节」表单。与 <see cref="OpenImportPanelCommand"/> 共用同一个面板，
    /// 只是把它切到新建模式（源文件字段隐藏、提交按钮改调 create_chapter）。
    /// </summary>
    public RelayCommand OpenCreateChapterPanelCommand { get; }

    /// <summary>
    /// U174：提交新建。判据必须落在「作品树里能否看到」——
    /// 走 <c>save_document_content</c> 也能让文件落盘并返回 ok，但那条路**不写章节索引**，
    /// 而作品树读的正是索引，于是章节对用户完全隐形（U174-A 的原始形态）。
    /// </summary>
    public RelayCommand CreateChapterCommand { get; }

    /// <summary>
    /// 面板当前是「新建章节」还是「导入稿件」。
    ///
    /// 两模式共用字段是有意的：章节 ID / 标题 / 排序 / 目标路径的校验规则一字不差，
    /// 分成两套表单只会让两处文案与规则各自漂移。
    /// </summary>
    public bool IsCreateChapterMode
    {
        get => _isCreateChapterMode;
        private set
        {
            if (SetProperty(ref _isCreateChapterMode, value))
            {
                OnPropertyChanged(nameof(IsImportChapterMode));
                OnPropertyChanged(nameof(ImportPanelTitle));
                NotifyImportFormStateChanged();
            }
        }
    }

    /// <summary>源文件字段只在导入模式出现：新建没有源稿，摆一个空输入框只会让人以为必填。</summary>
    public bool IsImportChapterMode => !_isCreateChapterMode;

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
    public RelayCommand RetryBodySearchCommand { get; }

    public RelayCommand ReadModeCommand { get; }

    public RelayCommand EditModeCommand { get; }

    public RelayCommand CopyCommand { get; }

    public RelayCommand SelectAllCommand { get; }

    public RelayCommand OpenQuickEditCommand { get; }
    public RelayCommand CloseQuickEditCommand { get; }

    public RelayCommand QuickAiCommand { get; }

    public RelayCommand ToggleOutlinePanelCommand { get; }

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

    /// <summary>
    /// View 注入：读取**当前可见视图**的阅读位置，折算成全篇字符偏移。
    ///
    /// U129：由 View 提供而不在 ViewModel 里算，因为位置只有视觉层知道
    /// （阅读侧要问 ScrollViewer 的 Offset 与块的实际高度，编辑器侧要问
    /// VisualLine 的 VisualTop）。返回 null 表示「此刻没有可用位置」，
    /// 例如正文为空或控件还没完成布局——这种情况下不该覆盖已有锚点。
    /// </summary>
    public Func<int?>? CaptureReadingOffset { get; set; }

    /// <summary>
    /// View 注入：把全篇字符偏移恢复到**切换后**的那个视图。
    ///
    /// 与 <see cref="CaptureReadingOffset"/> 成对。分开两个钩子而不是一个
    /// 「SyncPosition」，是因为捕获必须在切换**前**（旧视图还可见时）执行，
    /// 恢复必须在切换**后**（新视图完成布局时）执行，两个时机隔着一次布局。
    /// </summary>
    public Action<int>? RestoreReadingOffset { get; set; }

    /// <summary>后端作品树的单一层级身份源；显示筛选只复用这些节点实例。</summary>
    public ObservableCollection<WorksTreeItemViewModel> WorksTreeRoots { get; }

    /// <summary>按标题搜索后的根投影；子层投影由每个节点的 VisibleChildren 维护。</summary>
    public ObservableCollection<WorksTreeItemViewModel> VisibleWorksTreeRoots { get; }

    /// <summary>
    /// U145：导入用的章节 ID 候选，来自作品树里已有的章节。
    ///
    /// 仍允许手打列表外的值：**导入新章节**本来就是要给出一个还不存在的 id，
    /// 这个字段不能收成纯下拉。候选的用途是「覆盖既有章节」时对齐已有 id——
    /// 那条路径下打错就是静默导入成一个孤儿章节。
    /// </summary>
    public ObservableCollection<string> ImportChapterIdCandidates { get; }

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
                // U184-A：双路。标题那路在 ApplyWorksTreeSearch 里本地即时算完，
                // 正文这路走 IPC，因此带防抖（见 StartBodySearch）。
                StartBodySearch(_worksTreeSearchText, immediate: false);
            }
        }
    }

    public bool IsWorksTreeSearchActive => !string.IsNullOrWhiteSpace(WorksTreeSearchText);

    /// <summary>
    /// 「标题含关键词」分组标题；只在搜索态且有标题命中时出现。
    ///
    /// 搜索前树是完整目录、不需要标题；一旦进入搜索态就必须点明这一组的语义，
    /// 否则它与下面的正文命中组无法区分。
    /// </summary>
    public string WorksTreeTitleGroupText => _displayNames.Text("ui.works.body_search.title_group_title");
    public bool ShowWorksTreeTitleGroup => IsWorksTreeSearchActive && VisibleWorksTreeRoots.Count > 0;

    public ObservableCollection<WorksBodySearchHitViewModel> BodySearchHits { get; }
    public string BodySearchGroupText => _displayNames.Text("ui.works.body_search.group_title");
    public string BodySearchSearchingText => _displayNames.Text("ui.works.body_search.searching");
    public string BodySearchEmptyText => _displayNames.Text("ui.works.body_search.empty");
    public string BodySearchRetryText => _displayNames.Text("ui.works.body_search.retry");
    public string BodySearchErrorText => _bodySearchErrorText;
    public bool HasBodySearchHits => BodySearchHits.Count > 0;
    public bool IsBodySearching => _bodySearchState == BodySearchState.Searching;

    /// <summary>
    /// 「正文里没有匹配」的空态。要求 <see cref="BodySearchState.Done"/> ——
    /// 搜索**还在路上**时说「没有匹配」是在报告一个尚未成立的结论。
    /// </summary>
    public bool ShowBodySearchEmpty => _bodySearchState == BodySearchState.Done && BodySearchHits.Count == 0;

    /// <summary>
    /// 索引追赶中的提示（可重试，非错误）。见 <see cref="BodySearchState.Indexing"/>。
    /// </summary>
    public bool ShowBodySearchIndexing => _bodySearchState == BodySearchState.Indexing;
    public bool ShowBodySearchError => _bodySearchState == BodySearchState.Failed;
    public bool ShowBodySearchGroup => IsWorksTreeSearchActive
                                       && _bodySearchState != BodySearchState.Idle;

    public bool ShowWorksTreeSearchEmpty => _worksTreeState == WorksTreeLoadState.Content
                                            && IsWorksTreeSearchActive
                                            && VisibleWorksTreeRoots.Count == 0
                                            // U184-A：正文有命中时不再说「没有匹配的作品条目」。
                                            // 那句话在有正文命中的情况下是错的，而且会把作者的注意力
                                            // 从下面真实存在的命中上引开。
                                            && BodySearchHits.Count == 0
                                            && _bodySearchState is BodySearchState.Idle or BodySearchState.Done;

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

    // U184-A：换 key 而不是改旧 key 的值。旧文案「按标题搜索大纲或章节…」在功能
    // 变成双路后是**劝退式的错误说明**——作者读完就不会去搜正文了（放宽约束必须同批改文案）。
    // 旧 key 保持原值不动：别处可能还在引用它。
    public string WorksTreeSearchPlaceholder => _displayNames.Text("ui.works.tree_search_placeholder_body");

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
            if (_isEditMode == value)
            {
                return;
            }

            // U129：**切换前**先把旧视图的阅读位置折算成字符偏移存下来。
            // 顺序不能反——一旦 IsVisible 翻转，旧视图的 ScrollViewer/VisualLine
            // 就问不出有效位置了（未测量的控件返回 0，那等于静默丢失位置）。
            var captured = CaptureReadingOffset?.Invoke();
            if (captured is { } offset)
            {
                _readingOffsetAnchor = offset;
            }

            if (!SetProperty(ref _isEditMode, value))
            {
                return;
            }

            if (!value)
            {
                RebuildDocumentBlocks(_editorBuffer.Text);
            }
            OnPropertyChanged(nameof(ShowReadModeEmptyDocument));
            OnPropertyChanged(nameof(ShowSelectionContextItems));

            // 恢复交给 View：新视图此刻还没完成布局，必须等一轮 Dispatcher。
            if (_readingOffsetAnchor is { } anchor)
            {
                RestoreReadingOffset?.Invoke(anchor);
            }
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
        set
        {
            if (SetProperty(ref _quickEditDiff, value))
            {
                RebuildQuickEditDiffLines();
            }
        }
    }

    /// <summary>
    /// 快速编辑 diff 的行级投影，供统一视图（一行红一行绿）渲染。
    ///
    /// 之所以不直接把整段 diff 文本丢进一个只读 TextBox：那是纯文本，
    /// 增删行没有任何视觉区分，用户要逐字比对才知道改了什么；
    /// 而项目本身已在主题里备好 <c>Ariadne.DiffAddBackground</c> /
    /// <c>Ariadne.DiffRemoveBackground</c>（亮/暗两套都有），此前全仓无人使用。
    /// </summary>
    public ObservableCollection<QuickEditDiffLineViewModel> QuickEditDiffLines { get; } = new();

    /// <summary>diff 为空时整块不渲染，避免留下一个空白框。</summary>
    public bool HasQuickEditDiff => QuickEditDiffLines.Count > 0;

    private void RebuildQuickEditDiffLines()
    {
        QuickEditDiffLines.Clear();
        if (!string.IsNullOrEmpty(_quickEditDiff))
        {
            // 按 \n 切；后端产出统一用 \n（CRLF 在 save_document_with_policy 收口处已规范化）。
            foreach (var line in _quickEditDiff.Split('\n'))
            {
                // 末尾换行会切出一个空串，跳过它，否则视图底部多一条空行。
                if (line.Length == 0)
                {
                    continue;
                }
                QuickEditDiffLines.Add(new QuickEditDiffLineViewModel(line));
            }
        }
        OnPropertyChanged(nameof(HasQuickEditDiff));
    }

    public bool IsQuickEditGenerating
    {
        get => _isQuickEditGenerating;
        private set
        {
            if (SetProperty(ref _isQuickEditGenerating, value))
            {
                OnPropertyChanged(nameof(QuickEditGenerateText));
                OnPropertyChanged(nameof(IsQuickEditCloseEnabled));
                QuickAiCommand.NotifyCanExecuteChanged();
                OpenQuickEditCommand.NotifyCanExecuteChanged();
                ApplyQuickEditCommand.NotifyCanExecuteChanged();
                CloseQuickEditCommand.NotifyCanExecuteChanged();
            }
        }
    }

    /// <summary>
    /// 快速编辑悬浮窗是否打开。
    ///
    /// U130：此前它只绑 <see cref="IsEditMode"/>，即「进修改模式」与「要 AI 改写」
    /// 是同一个动作——面板占掉 289px、正文塌到 209px，而且**关不掉**。
    /// 两件事本来无关：想手写一段不需要 AI 面板，想让 AI 改一段也不必先进编辑器。
    /// 所以拆成独立开关；生成中不允许关闭窗口，否则用户看不到自己等的结果。
    /// </summary>
    public bool IsQuickEditOpen
    {
        get => _isQuickEditOpen;
        set
        {
            if (SetProperty(ref _isQuickEditOpen, value))
            {
                OnPropertyChanged(nameof(IsQuickEditCloseEnabled));
            }
        }
    }

    /// <summary>生成中禁用关闭：请求已经发出去了（钱已经花了），关窗只会让结果无处落地。</summary>
    public bool IsQuickEditCloseEnabled => !IsQuickEditGenerating;

    public string QuickEditCloseText => _displayNames.Text("ui.common.close");

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

    // U136：DocumentInfoText 已删。它把「路径 + 版本哈希 + 保存状态」印在稿纸刊头上，
    // 即在书名下方打印文件系统元数据——真正的「页」上只该有章节名。
    // 保存状态由顶栏 DocumentSaveStateText 承担（此前两处各印一遍），
    // 路径与版本属于属性面板，不属于阅读界面。

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

    // U174：新建章节文案。
    public string CreateChapterText => _displayNames.Text("ui.works.create_chapter");
    public string CreateChapterHint => _displayNames.Text("ui.works.create.hint");
    public string CreateChapterSubmitText => _displayNames.Text("ui.works.create.submit");

    /// <summary>面板标题随模式切换：同一个面板承担两件事时，标题是用户唯一的定位依据。</summary>
    public string ImportPanelTitle => _displayNames.Text(
        IsCreateChapterMode ? "ui.works.create.title" : "ui.works.import.title");

    /// <summary>
    /// 撞名提示。新建**没有** overwrite 复选框，所以文案必须给出可行动作
    /// （换 ID 或换路径），而不是照抄导入那句「确认覆盖后才能导入」——
    /// 那会让用户去找一个不存在的复选框。
    /// </summary>
    public string CreateConflictText => HasImportConflict
        ? _displayNames.Text("ui.works.create.conflict")
        : string.Empty;

    /// <summary>
    /// 撞名提示的两个可见性开关。分开是必要的：两句文案指向的动作不同
    /// （导入 → 勾覆盖；新建 → 换 ID/路径），只用 <see cref="HasImportConflict"/>
    /// 会让新建模式下也显示「确认覆盖后才能导入」，把用户指向一个已隐藏的复选框。
    /// </summary>
    public bool HasImportConflictInImportMode => HasImportConflict && IsImportChapterMode;

    public bool HasImportConflictInCreateMode => HasImportConflict && IsCreateChapterMode;
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
    public string CtxShowOutlineText => _displayNames.Text("ui.works.context.insert_outline");
    public string CtxToggleEditText => _displayNames.Text("ui.works.context.toggle_edit");

    /// <summary>
    /// 右键菜单是否显示「复制」「全选」两项。
    ///
    /// U132 产品决策：**阅读态不显示这两项，只保留 Ctrl+A / Ctrl+C 快捷键**。
    /// 此前它们在阅读态既可见又是错的——「复制」无视用户选中的 10 个字、
    /// 把整章 51088 字符塞进剪贴板；「全选」带 IsEditMode 前置判断，
    /// 点了毫无反应。两个都是**会误导的可见入口**：比没有更糟，
    /// 因为用户会以为自己操作错了，而不是知道这里没这个功能。
    /// </summary>
    public bool ShowSelectionContextItems => IsEditMode;

    private bool CanImportChapter()
    {
        return !HasImportChapterIdError
               && !HasImportChapterTitleError
               && !HasImportOrderError
               && ImportSourceValidation.IsValid
               && ImportTargetValidation.IsValid
               && (!HasImportConflict || AllowImportOverwrite);
    }

    /// <summary>
    /// 新建可否提交。与导入的差别只有两处，且两处都是**刻意**的：
    ///
    /// 1. **不查源文件路径**：新建没有源稿，把 `ImportSourceValidation` 也算进来
    ///    会让「新建」永远不可点（空路径 = Required 错误）。
    /// 2. **撞名一律拒，没有覆盖开关**：新建的语义里不存在「覆盖」。
    ///    想替换已有章节的正文是保存或导入的事；让新建能覆盖，
    ///    手滑一次就会静默毁掉已经写了三万字的那一章。
    ///    后端也是同一判断（`create_chapter` 不接受 overwrite），
    ///    这里挡在前面只是为了给出可读提示而不是等一条 conflict 异常。
    /// </summary>
    private bool CanCreateChapter()
    {
        return !HasImportChapterIdError
               && !HasImportChapterTitleError
               && !HasImportOrderError
               && ImportTargetValidation.IsValid
               && !HasImportConflict;
    }

    /// 导入源允许在项目外：作者从下载目录 / U 盘 / 别的写作软件导出目录挑稿子是常规用法，
    /// 后端 import_source_path_buf 也只禁 `..` 不查项目根（U163-B）。
    private ImportPathValidation ImportSourceValidation => WorksImportHelper.ValidateProjectPath(
        ImportSourcePath,
        _importProjectRoot,
        requireDocumentsDirectory: false,
        requireInsideProject: false);

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
            ImportPathError.UnsupportedPathForm => "ui.works.import.error.path_unsupported_form",
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
        OnPropertyChanged(nameof(CreateConflictText));
        OnPropertyChanged(nameof(HasImportConflictInImportMode));
        OnPropertyChanged(nameof(HasImportConflictInCreateMode));
        ImportCommand.NotifyCanExecuteChanged();
        CreateChapterCommand.NotifyCanExecuteChanged();
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
        // U130：只开面板，不动阅读/修改模式。用户按 Ctrl+K 要的是「跟 AI 说一句」，
        // 不是「把正文变成可编辑」——后者会让他在毫无准备时误碰键盘就改了小说。
        IsQuickEditOpen = true;
        RequestFocusQuickEditInstruction?.Invoke();
    }

    private void CloseQuickEdit()
    {
        if (IsQuickEditGenerating)
        {
            return;
        }
        IsQuickEditOpen = false;
        // 关窗即丢弃未应用的建议：留着它下次开窗会看到一条与当前正文早已不同步的
        // 旧 diff，而 CanApplyQuickEdit 又会拒绝应用，等于摆一个死按钮。
        ClearPendingQuickEdit();
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
        OnPropertyChanged(nameof(DocumentSaveStateText));
        SaveCommand.NotifyCanExecuteChanged();
        OpenQuickEditCommand.NotifyCanExecuteChanged();
        ToggleOutlinePanelCommand.NotifyCanExecuteChanged();
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
        // U129：块的起始字符偏移必须在切分时累加记录。切完再回头 IndexOf 找不可靠——
        // 长篇正文里重复段落很常见，按内容反查会命中错误的位置。
        var offset = 0;
        foreach (var block in SplitDocumentBlocks(content))
        {
            DocumentBlocks.Add(new DocumentBlockViewModel(
                $"read-block-{index}",
                index++,
                block,
                offset));
            offset += block.Length;
        }
        OnPropertyChanged(nameof(HasDocumentBlocks));
        OnPropertyChanged(nameof(ShowReadModeEmptyDocument));
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
        // U163-A：字段绝不能活过 `using` 作用域。loadCts 在方法返回时被 Dispose，
        // 若 _worksTreeLoadCts 仍指着它，下一次 Cancel()/Dispose() 就会抛
        // ObjectDisposedException——它不是取消异常，会一路冒泡到
        // MainWindowViewModel 切页的 catch，把用户从作品页弹回欢迎界面。
        // 原先只有内层 try 的 finally 清字段，而「无项目根」那条早返回绕过了它。
        // 这里用 ReferenceEquals 而不是代次比较：只清自己那一个，
        // 不能把后来者装进去的新 CTS 顺手清掉。
        try
        {
            await LoadWorksTreeCoreAsync(generation, loadCts).ConfigureAwait(true);
        }
        finally
        {
            if (ReferenceEquals(_worksTreeLoadCts, loadCts))
            {
                _worksTreeLoadCts = null;
            }
        }
    }

    private async Task LoadWorksTreeCoreAsync(long generation, CancellationTokenSource loadCts)
    {
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
                // 字段的清理已上移到 LoadWorksTreeAsync 的 finally（覆盖所有返回路径）；
                // 这里只刷新重试按钮的可用性。
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
            // U129：换文档即锚点作废。旧偏移套到新正文上会滚到毫无关系的位置。
            _readingOffsetAnchor = ReadingPositionMapper.ClearOnDocumentChange();

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

    /// <summary>
    /// U174：提交「新建章节」。
    ///
    /// **必须走 `create_chapter` 而不是 `save_document_content`**：后者只写文件、
    /// 不登记章节索引，而作品树读的是索引 ⇒ 文件落盘、命令返回 ok、用户在树里
    /// 什么都看不到。那正是用户报的「一些东西还不能创建」的机制——
    /// 创建动作本身成功了，只是没人认它。
    ///
    /// 收尾必须 `LoadWorksTreeAsync`：本方法的成功判据是「树里出现该章节」，
    /// 不刷新树等于把可见性推给下一次页面切换，而用户此刻正等着看到它。
    /// </summary>
    private async Task CreateChapterAsync()
    {
        try
        {
            if (!CanCreateChapter())
            {
                return;
            }

            var target = ImportTargetValidation.NormalizedPath;
            var title = ImportChapterTitle.Trim();
            await _backend.CreateChapterAsync(new ChapterCreateRequest(
                ImportChapterId.Trim(),
                title,
                decimal.ToInt64(ImportOrder!.Value),
                target,
                // 初始正文给空字符串：新建一章后作者自己写第一句是常态，
                // 塞一行模板标题只会让人先去删它。
                string.Empty)).ConfigureAwait(true);
            StatusText = _displayNames.Format(
                "ui.works.create.created",
                new Dictionary<string, string> { ["title"] = title });
            IsImportPanelOpen = false;
            await LoadWorksTreeAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private async Task SaveAsync()
    {        if (IsDocumentSaving)
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
            // U134：artifactId 传 null，交由后端命名。前端曾传 $"combined-{ExportFormat}"，
            // 缺 exports/ 前缀导致导出目录设置完全不生效、缺扩展名导致双击打不开、
            // 固定字符串导致第二次导出静默覆盖第一次。
            var report = await _backend.ExportChaptersAsync(Array.Empty<string>(), null, ExportFormat).ConfigureAwait(true);
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
        _readingOffsetAnchor = ReadingPositionMapper.ClearOnDocumentChange();
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
        ToggleOutlinePanelCommand.NotifyCanExecuteChanged();
        QuickAiCommand.NotifyCanExecuteChanged();
    }

    /// <summary>测试用：直接读取指定章节的正式总结投影。</summary>
    internal Task LoadChapterSummaryForTests(string chapterId) => LoadChapterSummaryAsync(chapterId);

    /// <summary>
    /// 测试用：只设置当前章节归属，不触发总结加载。
    ///
    /// 与 <see cref="LoadChapterSummaryForTests"/> 的区别是不打后端——
    /// U131 的大纲对照只需要「当前是哪一章」这个事实。
    /// </summary>
    internal void SeedSummaryChapterForTests(string chapterId)
    {
        _currentSummaryChapterId = chapterId;
        OnPropertyChanged(nameof(CurrentSummaryChapterId));
    }

    /// <summary>测试与调试：当前会话历史条数（用户+助手成对累积）。</summary>
    internal int ProjectAiHistoryCount => _projectAiHistory.Count;

    /// <summary>
    /// U131：打开/关闭章节大纲对照栏。
    ///
    /// 原实现（<c>InsertOutlineReference</c>）干三件事：强行进入修改模式、把
    /// <c>"@planning/outline.md"</c> 追加进正文末尾、改状态栏。后果是**用户按 Ctrl+S
    /// 就把这行垃圾持久化进小说**，而那个路径全后端零存在——真实约定是
    /// <c>planning/chapters/{id}.md</c>（见 core/src/rag/tools.rs）。
    ///
    /// 现在改为只读对照：读该章大纲文件显示在侧栏，**绝不修改正文**，
    /// 也不切换编辑模式（设计要求见 ui设计方案.md §2.2/2.3「左正文、右该章大纲」）。
    /// </summary>
    private void ToggleOutlinePanel()
    {
        if (IsOutlinePanelOpen)
        {
            IsOutlinePanelOpen = false;
            return;
        }

        if (string.IsNullOrWhiteSpace(_currentDocumentId))
        {
            StatusText = NoDocumentText;
            return;
        }

        IsOutlinePanelOpen = true;
        _ = LoadChapterOutlineAsync();
    }

    /// <summary>
    /// 读当前章节的大纲文件。找不到时显示可诊断文案而非静默留空——
    /// 「打开了对照栏但一片空白」分不清是没大纲还是加载失败。
    /// </summary>
    private async Task LoadChapterOutlineAsync()
    {
        var chapterId = _currentSummaryChapterId;
        if (string.IsNullOrWhiteSpace(chapterId))
        {
            // 没有章节归属（例如打开的是全局总纲本身），此时没有「该章大纲」可言。
            ChapterOutlineText = _displayNames.Format(
                "ui.works.outline_missing",
                new Dictionary<string, string> { ["chapter"] = "-" });
            return;
        }

        _outlineLoadCts?.Cancel();
        _outlineLoadCts?.Dispose();
        var cts = new CancellationTokenSource();
        _outlineLoadCts = cts;

        IsOutlineLoading = true;
        ChapterOutlineText = string.Empty;
        try
        {
            var path = $"planning/chapters/{chapterId}.md";
            var content = await _backend
                .GetDocumentContentByPathAsync(path, cts.Token)
                .ConfigureAwait(true);
            if (cts.IsCancellationRequested)
            {
                return;
            }
            ChapterOutlineText = string.IsNullOrWhiteSpace(content)
                ? _displayNames.Format(
                    "ui.works.outline_missing",
                    new Dictionary<string, string> { ["chapter"] = chapterId })
                : content;
        }
        catch (OperationCanceledException)
        {
            // 切章导致的取消不是错误，也不该覆盖新一轮的加载结果。
        }
        catch (Exception)
        {
            if (!cts.IsCancellationRequested)
            {
                // 后端对不存在的路径返回错误。这里不透传原始错误文本：
                // 用户要的答案是「这章还没写大纲」，不是 IPC 错误码。
                ChapterOutlineText = _displayNames.Format(
                    "ui.works.outline_missing",
                    new Dictionary<string, string> { ["chapter"] = chapterId });
            }
        }
        finally
        {
            if (!cts.IsCancellationRequested)
            {
                IsOutlineLoading = false;
            }
        }
    }

    private void OpenImportPanel()
    {
        IsCreateChapterMode = false;
        OpenChapterPanel();
    }

    /// <summary>
    /// U174：打开新建表单，并把目标路径预填成 `documents/chapters/{id}.md` 那一档。
    ///
    /// 预填的理由是 AGENTS.md 那条硬约束：**用户已知的值不要让用户手打**。
    /// 后端对目标路径是精确校验（必须在 `documents/` 下），让用户凭空猜一条路径，
    /// 猜错只会得到一句校验错误。这里给出可直接提交的默认值，作者仍可改。
    /// </summary>
    private void OpenCreateChapterPanel()
    {
        IsCreateChapterMode = true;
        OpenChapterPanel();
        // 只在字段还空着时填，不覆盖用户已经打了一半的内容。
        if (string.IsNullOrWhiteSpace(ImportTargetPath))
        {
            var next = Math.Max(1, CountWorksTreeChapters() + 1);
            ImportTargetPath = $"documents/chapters/ch{next:D2}.md";
            if (string.IsNullOrWhiteSpace(ImportChapterId))
            {
                ImportChapterId = $"ch{next:D2}";
            }
        }
    }

    private void OpenChapterPanel()
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
        // U130：**应用**才切修改模式（生成不切）。改写落到内存正文后用户必然要
        // 复核、微调、保存，此刻编辑器是他要的界面；而在此之前把他推进编辑器
        // 只是让 289px 的面板挤掉正文。
        IsEditMode = true;
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
                _readingOffsetAnchor = ReadingPositionMapper.ClearOnDocumentChange();
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
        // U145：章节 ID 候选跟着作品树走。挂在这个装配点而不是各调用处：
        // 树只在这里被整体替换，漏一处就会出现「树已刷新但候选还是上一版」。
        IdentifierCandidates.Sync(
            ImportChapterIdCandidates,
            IdentifierCandidates.Compose(
                EnumerateWorksTreeNodes()
                    .Where(node => node.IsChapter)
                    .Select(node => node.ChapterId)));
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
        OnPropertyChanged(nameof(ShowWorksTreeTitleGroup));
        OnPropertyChanged(nameof(ShowBodySearchGroup));
    }

    /// <summary>
    /// U184-A：正文搜索这一路的入口（防抖 + 代际 + 取消）。
    ///
    /// <para><paramref name="immediate"/> 为 false 时等 <see cref="BodySearchDebounceMs"/>
    /// 再发请求：这个框是**边打边搜**的（标题那路即时），不防抖会让每敲一个字都打一次
    /// IPC，而后端每次搜索要抢项目互斥锁 + 跑知识同步。重试按钮走 immediate=true。</para>
    ///
    /// <para>本方法是同步的、故意不返回 Task：调用点在属性 setter 里，
    /// 让 setter 去 await 一个网络往返会把打字卡住。</para>
    /// </summary>
    private void StartBodySearch(string rawQuery, bool immediate)
    {
        var query = (rawQuery ?? string.Empty).Trim();
        var generation = Interlocked.Increment(ref _bodySearchGeneration);
        _bodySearchCts?.Cancel();
        _bodySearchCts?.Dispose();
        _bodySearchCts = null;

        if (query.Length == 0)
        {
            _bodySearchQuery = string.Empty;
            SetBodySearchState(BodySearchState.Idle, clearHits: true);
            return;
        }

        _bodySearchQuery = query;
        var cts = new CancellationTokenSource();
        _bodySearchCts = cts;
        SetBodySearchState(BodySearchState.Searching, clearHits: false);
        _ = RunBodySearchAsync(query, generation, immediate, cts);
    }

    private async Task RunBodySearchAsync(
        string query,
        long generation,
        bool immediate,
        CancellationTokenSource cts)
    {
        try
        {
            if (!immediate)
            {
                await Task.Delay(BodySearchDebounceMs, cts.Token).ConfigureAwait(true);
            }
            if (generation != _bodySearchGeneration)
            {
                return;
            }
            var hits = await _backend
                .SearchProjectDocumentsAsync(query, BodySearchLimit, cts.Token)
                .ConfigureAwait(true);
            if (generation != _bodySearchGeneration)
            {
                return;
            }
            ReplaceBodySearchHits(hits);
            SetBodySearchState(BodySearchState.Done, clearHits: false);
        }
        catch (OperationCanceledException)
        {
            // 打字太快或换了关键词：不是失败，什么都不要显示。
        }
        catch (Exception ex)
        {
            if (generation != _bodySearchGeneration)
            {
                return;
            }
            // ⚠️ 这道分叉是本条的验收重点之一。索引门禁
            // （`retrieval/lifecycle.rs::ensure_search_not_blocked_by_pending_index`）
            // 在作者「刚保存完就搜」时必然命中，它是暂态而非故障；
            // 渲染成红色报错会让作者判定「搜索功能坏了」，与缺陷版本的
            // 「永远 0 结果」一样糟。识别凭后端贴的稳定 key（U1：不嗅探英文诊断串）。
            if (IsIndexingNotReady(ex))
            {
                _bodySearchErrorText = _displayNames.Text("ui.works.body_search.indexing");
                SetBodySearchState(BodySearchState.Indexing, clearHits: true);
                return;
            }
            _bodySearchErrorText = UserFacingError.Format(ex, _displayNames);
            SetBodySearchState(BodySearchState.Failed, clearHits: true);
        }
        finally
        {
            if (ReferenceEquals(_bodySearchCts, cts))
            {
                _bodySearchCts = null;
            }
            cts.Dispose();
        }
    }

    /// <summary>
    /// 后端把索引门禁标成了 <c>ui.error.indexing_not_ready</c>
    /// （<c>commands.rs::tag_indexing_not_ready</c>）。这里只认那个 key。
    /// </summary>
    private static bool IsIndexingNotReady(Exception ex)
    {
        var backend = ex as BackendException ?? ex.InnerException as BackendException;
        return string.Equals(backend?.MessageKey, "ui.error.indexing_not_ready", StringComparison.Ordinal);
    }

    private void ReplaceBodySearchHits(IReadOnlyList<RetrievalHit> hits)
    {
        BodySearchHits.Clear();
        // 按 document_id 去重：一章正文被切成多个 chunk，命中两个 chunk 时
        // 作者看到同一章出现两次只会困惑。留分数最高的那条（后端已按分排序）。
        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (var hit in hits)
        {
            if (!seen.Add(hit.DocumentId))
            {
                continue;
            }
            var node = ResolveBodySearchNode(hit.DocumentId);
            BodySearchHits.Add(new WorksBodySearchHitViewModel(
                BodySearchHitTitle(hit.DocumentId, node),
                CondenseSnippet(hit.Snippet),
                _displayNames.Format(
                    "ui.works.body_search.hit_accessible",
                    new Dictionary<string, string> { ["title"] = BodySearchHitTitle(hit.DocumentId, node) }),
                node is null ? null : () => ActivateWorksTreeNode(node)));
        }
    }

    /// <summary>
    /// 把命中的 <c>document_id</c> 映射回作品树节点。
    ///
    /// <para>⚠️ 两侧路径形态**不同**，不能直接比字面：索引侧是
    /// <c>path.canonicalize()</c> 的绝对路径（<c>retrieval/runtime.rs</c>），
    /// 树侧是后端 <c>WorksTreeNode.path</c>。统一归一到项目相对路径再比。</para>
    ///
    /// <para>知识库命中（<c>ariadne-knowledge://…</c>）没有对应树节点，返回 null——
    /// 它照样值得显示（片段本身有用），只是点不开。</para>
    /// </summary>
    private WorksTreeItemViewModel? ResolveBodySearchNode(string documentId)
    {
        if (string.IsNullOrWhiteSpace(documentId)
            || documentId.StartsWith("ariadne-knowledge://", StringComparison.Ordinal))
        {
            return null;
        }
        var target = ProjectRelativePath(documentId);
        if (target.Length == 0)
        {
            return null;
        }
        foreach (var node in EnumerateWorksTreeNodes())
        {
            if (!node.HasPath)
            {
                continue;
            }
            if (string.Equals(ProjectRelativePath(node.Path), target, StringComparison.OrdinalIgnoreCase))
            {
                return node;
            }
        }
        return null;
    }

    private string BodySearchHitTitle(string documentId, WorksTreeItemViewModel? node)
    {
        if (node is not null)
        {
            return node.Title;
        }
        if (documentId.StartsWith("ariadne-knowledge://", StringComparison.Ordinal))
        {
            return _displayNames.Text("ui.works.body_search.knowledge_document");
        }
        var relative = ProjectRelativePath(documentId);
        return relative.Length > 0
            ? relative
            : _displayNames.Text("ui.works.body_search.unknown_document");
    }

    /// <summary>
    /// 片段折行压缩。chunk 默认上千字，整段塞进窄侧栏会把命中列表冲掉；
    /// 换行也要压掉，否则一条命中在树栏里能占十几行。
    /// </summary>
    private static string CondenseSnippet(string? snippet)
    {
        if (string.IsNullOrWhiteSpace(snippet))
        {
            return string.Empty;
        }
        var text = snippet.Replace('\r', ' ').Replace('\n', ' ').Trim();
        while (text.Contains("  ", StringComparison.Ordinal))
        {
            text = text.Replace("  ", " ", StringComparison.Ordinal);
        }
        return text.Length <= BodySearchSnippetChars ? text : text[..BodySearchSnippetChars] + "…";
    }

    private void SetBodySearchState(BodySearchState state, bool clearHits)
    {
        _bodySearchState = state;
        if (clearHits)
        {
            BodySearchHits.Clear();
        }
        if (state is not (BodySearchState.Indexing or BodySearchState.Failed))
        {
            _bodySearchErrorText = string.Empty;
        }
        OnPropertyChanged(nameof(IsBodySearching));
        OnPropertyChanged(nameof(HasBodySearchHits));
        OnPropertyChanged(nameof(ShowBodySearchEmpty));
        OnPropertyChanged(nameof(ShowBodySearchIndexing));
        OnPropertyChanged(nameof(ShowBodySearchError));
        OnPropertyChanged(nameof(ShowBodySearchGroup));
        OnPropertyChanged(nameof(BodySearchErrorText));
        OnPropertyChanged(nameof(ShowWorksTreeSearchEmpty));
        RetryBodySearchCommand.NotifyCanExecuteChanged();
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

/// <summary>
/// U184-A：一条正文命中。
///
/// <para>刻意**不是** <see cref="WorksTreeItemViewModel"/>：那个类型承载展开状态、
/// 父子链、当前文档高亮等树语义，正文命中一个都不需要，且复用它就等于把两组结果
/// 混成同一种东西——而「为什么这一章出现在结果里」正是本条要回答的问题。</para>
///
/// <para><see cref="Snippet"/> 直接来自后端 <c>RetrievalResult.snippet</c>（已裁剪压行），
/// 前端不重新去读正文再切——那会为了显示一条命中而读一整章。</para>
///
/// <para><see cref="OpenCommand"/> 在命中落在作品树之外（知识库条目、已移动的文件）时
/// 不可执行；那种命中仍然显示，因为片段本身就有信息量。</para>
/// </summary>
public sealed class WorksBodySearchHitViewModel
{
    public WorksBodySearchHitViewModel(string title, string snippet, string accessibleName, Action? open)
    {
        Title = title;
        Snippet = snippet;
        AccessibleName = accessibleName;
        OpenCommand = new RelayCommand(() => open?.Invoke(), () => open is not null);
    }

    public string Title { get; }
    public string Snippet { get; }
    public string AccessibleName { get; }
    public RelayCommand OpenCommand { get; }
}

/// <summary>
/// 快速编辑 diff 的一行。
///
/// 后端 <c>simple_diff</c> 产出的是带前缀的行：<c>- </c> 删除、<c>+ </c> 新增、
/// 两个空格为上下文（含 <c>  ... (N unchanged lines)</c> 这样的折叠标记）。
/// 这里只做「前缀 → 类别」的翻译，不重新实现 diff 算法——
/// 算法留在后端一处，前端两处（快速编辑、将来的冲突合并）共用同一份产出，
/// 避免两套 diff 结果对不上。
/// </summary>
public sealed class QuickEditDiffLineViewModel
{
    public QuickEditDiffLineViewModel(string rawLine)
    {
        if (rawLine.StartsWith("- ", StringComparison.Ordinal))
        {
            Kind = QuickEditDiffLineKind.Removed;
            Text = rawLine[2..];
        }
        else if (rawLine.StartsWith("+ ", StringComparison.Ordinal))
        {
            Kind = QuickEditDiffLineKind.Added;
            Text = rawLine[2..];
        }
        else
        {
            Kind = QuickEditDiffLineKind.Context;
            // 上下文行带两个空格前缀；行内容本身可能以空格开头，故只剥固定前缀。
            Text = rawLine.StartsWith("  ", StringComparison.Ordinal) ? rawLine[2..] : rawLine;
        }
    }

    public QuickEditDiffLineKind Kind { get; }

    public string Text { get; }

    /// <summary>行首标记。留空而不是省略，是为了让三类行的正文左边缘对齐。</summary>
    public string Marker => Kind switch
    {
        QuickEditDiffLineKind.Removed => "-",
        QuickEditDiffLineKind.Added => "+",
        _ => " ",
    };

    public bool IsRemoved => Kind == QuickEditDiffLineKind.Removed;
    public bool IsAdded => Kind == QuickEditDiffLineKind.Added;
}

public enum QuickEditDiffLineKind
{
    Context,
    Added,
    Removed,
}

public sealed record EditorTextSelection(int Start, int End, string Text);

/// <summary>只读模式的虚拟化投影；不再承担编辑、选区或光标状态。</summary>
public sealed class DocumentBlockViewModel
{
    public DocumentBlockViewModel(
        string id,
        int index,
        string text,
        int startOffset)
    {
        Id = id;
        Index = index;
        Text = text;
        StartOffset = startOffset;
    }

    public string Id { get; }
    public int Index { get; }
    public string Text { get; }
    /// <summary>
    /// U129：该块首字符在**整篇正文**中的偏移。
    ///
    /// 阅读模式与编辑器是两套完全不同的滚动坐标系（块索引 vs 行号/像素偏移），
    /// 唯一能在两者间换算的共同量就是字符偏移。没有它，「切换视图保留位置」
    /// 只能退化成块级对齐——而块粒度是 4000–6000 字符（约 8–12 屏），差得太远。
    /// </summary>
    public int StartOffset { get; }
    /// <summary>该块末字符之后的偏移（半开区间右端）。</summary>
    public int EndOffset => StartOffset + Text.Length;
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
