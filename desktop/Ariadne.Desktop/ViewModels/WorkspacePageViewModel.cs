using System.Collections.ObjectModel;
using System.Text.Json;
using System.Text.Json.Nodes;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class WorkspacePageViewModel : PageViewModelBase, IUnsavedChangesGuard, IProjectDataReloadable, IUiPreferencesAware, ILocalizedUiAware
{
    private const string RightPanelPreferenceKey = "workspace.right_panel";
    private const string ProjectAiConversationId = "workspace";
    private const string DefaultWorkflowId = "default";
    // 收起后列宽为 0：展开只靠画布右缘 pill，避免窄条 + float 双箭头
    private const double CollapsedRightPanelWidth = 0;
    private static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web);

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private readonly Func<string, bool, Task>? _persistPanelState;
    private readonly ProjectAutomationState _projectAutomation;
    private readonly WorkspaceRunSessionCoordinator _runSession;
    // U4：首屏只展开底部节点库；右侧详情按需打开，避免三栏同时挤压画布。
    private bool _isRightPanelOpen;
    // 默认右栏略窄，多留给画布
    private GridLength _rightPanelColumnWidth = new(280);
    private double _availableWorkspaceWidth = double.PositiveInfinity;
    private bool _isLibraryOpen = true;
    private bool _isCanvasFocusMode;
    private bool _focusRestoreLibraryOpen;
    private bool _focusRestoreRightPanelOpen;
    private bool _isExecutionPanel;
    private WorkspaceRightPanelTab _rightPanelTab = WorkspaceRightPanelTab.ProjectAi;
    private readonly CanvasViewportSession _canvasViewport = new();
    private bool _hasUnsavedChanges;
    private string _savedSnapshot = string.Empty;
    private string _savedContentSnapshot = string.Empty;
    private bool _suppressSnapshotChecks;
    private bool _deferDirtyRefresh;
    private int _nextNodeNumber = 1;
    private string _projectAiMessage = string.Empty;
    // U196-D：失败的那个节点 id（后端 WorkflowRunFailure.Stage）。
    private string _failedNodeId = string.Empty;
    private string _projectAiAnswer;
    private bool _isConfirmationPanelExpanded = true;
    // 审批历史默认收起：进入审阅态第一眼该落在待审那一项上，历史是「想查才查」。
    private bool _isConfirmationHistoryExpanded;
    // 「没有待审项时也要能查历史」的钉开哨兵（U187-A）。
    // 若不要这个哨兵，历史入口就只在「恰好还有待审项」时才够得着——
    // 而作者最需要查历史的时刻恰恰是「昨天全审完了、今天想回看批过什么」。
    private bool _isConfirmationHistoryPinnedOpen;
    private readonly RequestGenerationSession _canvasLoading = new();
    private string _projectCanvasName = "Project Canvas";
    private Dictionary<string, object?> _canvasMetadata = new(StringComparer.Ordinal);
    private string _confirmationReason = string.Empty;
    private bool _isRejectArmed;
    // 进入审阅态时「已自动切过右栏」的哨兵：只切一次，之后尊重用户的手动切换。
    private bool _confirmationReviewSwitchedRightPanel;
    private WorkflowOperation? _selectedInDoubtOperation;
    private string _inDoubtResponseJson = string.Empty;
    private string _inDoubtStopReason = string.Empty;
    private string _annotationTitle = string.Empty;
    private IReadOnlyList<CanvasEdge> _edges = Array.Empty<CanvasEdge>();
    private readonly List<string> _undoSnapshots = new();
    private readonly List<string> _redoSnapshots = new();
    private readonly List<ProjectAiChatMessage> _projectAiHistory = new();
    private long? _projectAiConversationRevision;
    private CanvasNode? _clipboardNode;
    private WorkflowNodeViewModel? _selectedNode;
    private ConfirmationItemViewModel? _selectedConfirmation;
    private WorkflowEdgeViewModel? _selectedEdge;
    private WorkflowLoadState _workflowLoadState = WorkflowLoadState.NoProject;
    private bool _isApplyingGraph;
    private string? _workflowContentRevision;

    public WorkspacePageViewModel(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        Func<string, bool, Task>? persistPanelState = null,
        ProjectAutomationState? projectAutomation = null)
    {
        _displayNames = displayNames;
        _backend = backend;
        _persistPanelState = persistPanelState;
        _projectAutomation = projectAutomation ?? new ProjectAutomationState(displayNames, backend);
        _runSession = new WorkspaceRunSessionCoordinator(backend);
        _runSession.StateChanged += OnRunSessionStateChanged;
        _runSession.EventsReceived += ApplyWorkflowEvents;
        _runSession.PollingFailed += OnRunSessionPollingFailed;
        ToggleRightPanelCommand = new RelayCommand(() => _ = ToggleRightPanelAsync());
        ToggleLibraryCommand = new RelayCommand(() => IsLibraryOpen = !IsLibraryOpen);
        ToggleCanvasFocusModeCommand = new RelayCommand(ToggleCanvasFocusMode);
        ZoomInCommand = new RelayCommand(() => AdjustCanvasZoom(0.1));
        ZoomOutCommand = new RelayCommand(() => AdjustCanvasZoom(-0.1));
        ResetZoomCommand = new RelayCommand(ResetCanvasZoom);
        ShowNodeLibraryCommand = new RelayCommand(() =>
        {
            IsExecutionPanel = false;
            IsLibraryOpen = true;
        });
        ShowExecutionCommand = new RelayCommand(() =>
        {
            IsExecutionPanel = true;
            IsLibraryOpen = true;
        });
        ShowProjectAiCommand = new RelayCommand(() =>
        {
            SetRightPanelTab(WorkspaceRightPanelTab.ProjectAi);
            IsRightPanelOpen = true;
        });
        ShowNodeDetailsCommand = new RelayCommand(() =>
        {
            SetRightPanelTab(WorkspaceRightPanelTab.NodeDetails);
            IsRightPanelOpen = true;
        });
        ShowEdgeDetailsCommand = new RelayCommand(() =>
        {
            SetRightPanelTab(WorkspaceRightPanelTab.EdgeDetails);
            IsRightPanelOpen = true;
        });
        ReloadProjectCanvasCommand = new RelayCommand(() => _ = ReloadProjectCanvasWithUnsavedCheckAsync());
        ExportCommand = new RelayCommand(() => _ = ExportWorkflowAsync(requireSelection: false));
        SaveCommand = new RelayCommand(() => _ = SaveWorkflowAsync(), CanPersistWorkflow);
        UndoCommand = new RelayCommand(UndoCanvasChange, () => _undoSnapshots.Count > 0);
        RedoCommand = new RelayCommand(RedoCanvasChange, () => _redoSnapshots.Count > 0);
        AddContextNodeCommand = new RelayCommand(() => AddNode("llm"));
        AddStartNodeCommand = new RelayCommand(() => AddNode("start"));
        // W1：Delete 优先删选中边（不连带端点节点），否则删节点。
        DeleteSelectedNodeCommand = new RelayCommand(
            () => _ = DeleteSelectionAsync(),
            () => HasSelectedEdge || HasSelectedNode);
        RunSelectedNodeCommand = new RelayCommand(
            () => _ = RunSelectedNodeAsync(),
            () => IsSelectedStartNode && CanPersistWorkflow());
        // W8：按 run 生命周期门禁，而非仅 HasCurrentRun。
        PauseWorkflowCommand = new RelayCommand(() => _ = PauseWorkflowAsync(), CanPauseWorkflow);
        StopWorkflowCommand = new RelayCommand(() => _ = StopWorkflowAsync(), CanStopWorkflow);
        ResumeWorkflowCommand = new RelayCommand(() => _ = ResumeWorkflowAsync(), CanResumeWorkflow);
        // U196-D：从失败的那个节点重跑。它**不能**并进上面三键：那三键的 CanExecute
        // 全部要求非终态（见 CanvasRunControlHelpers），而这颗恰恰只在终态 failed 下有用。
        RetryFailedNodeCommand = new RelayCommand(
            () => _ = RetryFailedNodeAsync(),
            CanRetryFailedNode);
        // U207-C①：下栏标题栏的「开始运行」入口。原先此处只有暂停/继续/停止三个
        // **运行中**控制，空画布上一个开始入口都没有（起始节点卡上的三角要先有节点，
        // 执行面板的主按钮要先切 tab），作者只会把「继续」当播放键去点。
        RunWorkflowCommand = new RelayCommand(() => _ = RunWorkflowFromEntryAsync(), CanRunWorkflowFromEntry);
        SendProjectAiCommand = new RelayCommand(() => _ = SendProjectAiAsync(), HasProjectAiMessage);
        ApplyNodeConfigCommand = new RelayCommand(() => _ = ApplyNodeConfigAsync(), () => HasSelectedNode);
        ToggleBreakpointCommand = new RelayCommand(() => _ = ToggleBreakpointAsync(), () => HasSelectedNode);
        BrowseWorkDirCommand = new RelayCommand(() => _ = BrowseWorkDirAsync(), () => IsSelectedStartNode);
        BrowseImportFileCommand = new RelayCommand(() => _ = BrowseImportFileAsync(), () => SelectedNode?.IsImportNode == true);
        AddAnnotationCommand = new RelayCommand(() => _ = AddAnnotationAsync());
        // 导出所选：必须有选中节点；整图导出走 ExportCommand
        ExportSelectionCommand = new RelayCommand(() => _ = ExportWorkflowAsync(requireSelection: true), () => HasSelectedNode);
        PackSelectionCommand = new RelayCommand(() => _ = PackSelectionAsync());
        RefreshConfirmationsCommand = new RelayCommand(() => _ = LoadConfirmationsAsync());
        ToggleConfirmationPanelCommand = new RelayCommand(() =>
        {
            IsConfirmationPanelExpanded = !IsConfirmationPanelExpanded;
            // 「收起看画布」必须同时解除历史钉开，否则点了收起画布仍被盖住——
            // 按钮说了话没做到，比没这个按钮更糟（U187-A）。
            if (!IsConfirmationPanelExpanded)
            {
                SetConfirmationHistoryPinned(false);
            }
        });
        // 审批历史折叠区（U187-A）：只翻自己的开合，**不碰** IsConfirmationPanelExpanded——
        // 那个是「审阅面板 vs 画布」的开关，两者混在一起会让查历史顺手把画布盖掉。
        ToggleConfirmationHistoryCommand = new RelayCommand(() =>
            IsConfirmationHistoryExpanded = !IsConfirmationHistoryExpanded);
        // 无待审项时进入审批历史的入口（画布工具条）。
        // 钉开面板 + 展开折叠区一次做完：点「审批历史」就该直接看见历史，
        // 而不是打开一个还要再点一次才出内容的面板。
        ShowConfirmationHistoryCommand = new RelayCommand(
            () =>
            {
                SetConfirmationHistoryPinned(true);
                IsConfirmationHistoryExpanded = true;
            },
            () => HasResolvedConfirmations);
        ApproveConfirmationCommand = new RelayCommand(() => _ = ResolveSelectedConfirmationAsync("approve"), CanResolveConfirmation);
        // 第一次点击只展开理由输入线；第二次才真正提交拒绝。
        RejectConfirmationCommand = new RelayCommand(
            () =>
            {
                if (!IsRejectArmed)
                {
                    IsRejectArmed = true;
                    RequestFocusRejectReason?.Invoke();
                    return;
                }

                _ = ResolveSelectedConfirmationAsync("reject");
            },
            CanResolveConfirmation);
        CancelRejectConfirmationCommand = new RelayCommand(DisarmReject);
        // 引用当前确认项问项目 AI（U139④）：能力齐备、入口原本不存在。
        AskAiAboutConfirmationCommand = new RelayCommand(
            () => _ = AskAiAboutConfirmationAsync(),
            () => SelectedConfirmation is not null);
        // 主按钮：武装态下是退出口（取消拒绝），常态才是确认通过。
        ApproveOrCancelCommand = new RelayCommand(
            () =>
            {
                if (IsRejectArmed)
                {
                    DisarmReject();
                    return;
                }

                _ = ResolveSelectedConfirmationAsync("approve");
            },
            CanResolveConfirmation);
        RetryInDoubtOperationCommand = new RelayCommand(() => _ = ResolveSelectedInDoubtOperationAsync("retry"), HasSelectedInDoubtOperation);
        UseInDoubtResponseCommand = new RelayCommand(() => _ = ResolveSelectedInDoubtOperationAsync("use_response"), HasSelectedInDoubtOperation);
        StopInDoubtOperationCommand = new RelayCommand(() => _ = ResolveSelectedInDoubtOperationAsync("stop"), HasSelectedInDoubtOperation);
        SaveEdgeConfigCommand = new RelayCommand(SaveSelectedEdgeConfig, () => HasSelectedEdge);
        InsertForwardTemplateVariableCommand = new RelayCommand(InsertForwardTemplateVariable, () => SelectedEdge?.IsCommunication == true);
        InsertReverseTemplateVariableCommand = new RelayCommand(InsertReverseTemplateVariable, () => SelectedEdge?.IsCommunication == true);
        CopySelectedNodeCommand = new RelayCommand(CopySelectedNode, () => HasSelectedNode);
        CutSelectedNodeCommand = new RelayCommand(() => _ = CutSelectedNodeAsync(), () => HasSelectedNode);
        PasteNodeCommand = new RelayCommand(PasteNode, () => _clipboardNode is not null);
        FitViewCommand = new RelayCommand(FitView);
        // Ctrl+K 的「AI 填变量值」面板：与作品页快捷改写同构，不新增命令通道
        // （生成走既有 project_ai_chat）。报状态用同一个 StatusText，
        // 错误翻译沿用 UserFacingError——面板本身不认识后端异常类型。
        VariableFill = new VariableFillPanelViewModel(
            _displayNames.Text,
            _displayNames.Format,
            message => StatusText = message,
            ex => ReportFailure(ex, _displayNames));
        VariableFill.RequestFill = FillVariablesWithProjectAiAsync;
        OpenVariableFillCommand = new RelayCommand(
            () => VariableFill.Open(ResolveVariableFillTarget()),
            // 画布上没有带变量的起始节点时不可用：Ctrl+K 开一个空面板等于摆死按钮。
            () => ResolveVariableFillTarget() is not null);
        _projectAiAnswer = displayNames.Text("ui.workspace.project_ai.empty");

        // U206-B：跨章知识查询。两条出站通道都在这里注入 ——
        // 查询直接走 resolve_project_reference（本地检索、零 token），
        // 「问 AI」复用 project_ai_chat 的 references 参数（引用式数据流，不内联正文）。
        KnowledgeLookup = new KnowledgeLookupPanelViewModel(
            _displayNames.Text,
            message => StatusText = message,
            ex => ReportFailure(ex, _displayNames));
        KnowledgeLookup.RequestLookup = reference => _backend.ResolveProjectReferenceAsync(reference);
        KnowledgeLookup.RequestAskAi = AskAiAboutKnowledgeAsync;

        Nodes = new ObservableCollection<WorkflowNodeViewModel>();
        StartNodes = new ObservableCollection<WorkflowNodeViewModel>();
        Confirmations = new ObservableCollection<ConfirmationItemViewModel>();
        InDoubtOperations = new ObservableCollection<WorkflowOperation>();
        Edges = new ObservableCollection<WorkflowEdgeViewModel>();
        RelatedEdges = new ObservableCollection<WorkflowEdgeViewModel>();
        // U178-B：新入集合的节点/边立刻继承当前语义缩放态。
        // 挂在集合上而不是逐个改造 new 的调用点：创建路径有好几条
        // （新建、粘贴、按图加载、导入），漏一条就会出现「缩小状态下新建的卡片
        // 引脚全显示」这种局部不一致，且只在特定路径复现、极难发现。
        Nodes.CollectionChanged += (_, args) =>
        {
            foreach (var node in args.NewItems?.OfType<WorkflowNodeViewModel>()
                                 ?? Enumerable.Empty<WorkflowNodeViewModel>())
            {
                ApplySemanticZoomTo(node);
            }
        };
        Edges.CollectionChanged += (_, args) =>
        {
            foreach (var edge in args.NewItems?.OfType<WorkflowEdgeViewModel>()
                                 ?? Enumerable.Empty<WorkflowEdgeViewModel>())
            {
                ApplySemanticZoomTo(edge);
            }
        };
        ProjectAiBubbles = new ObservableCollection<ChatBubbleViewModel>();
        EntryNodes = CreateNodeLibraryGroup("entry");
        WritingAgents = CreateNodeLibraryGroup("writing");
        UtilityNodes = CreateNodeLibraryGroup("utility");

        AvailableModelOptions = new ObservableCollection<WorkflowModelOption>
        {
            WorkflowModelOption.Inherited(displayNames.Text("ui.workspace.model_inherit_global")),
        };
        SummarizerChapterOptions = new ObservableCollection<SummarizerChapterOption>();
        // U145：标识/别名候选。空集合也要建好——XAML 的 AutoCompleteBox 绑定它，
        // 延迟到「选中节点时才 new」会让首次绑定拿到 null 而永久不再刷新。
        NodeAliasCandidates = new ObservableCollection<string>();
        ChapterIdCandidates = new ObservableCollection<string>();
        ChapterDocumentIdCandidates = new ObservableCollection<string>();
        ApprovalIdCandidates = new ObservableCollection<string>();
        ExportArtifactIdCandidates = new ObservableCollection<string>();
        SourceHandleCandidates = new ObservableCollection<string>();
        TargetHandleCandidates = new ObservableCollection<string>();
        CommunicationAliasCandidates = new ObservableCollection<string>();
        CaptureSnapshot();
    }

    public ObservableCollection<WorkflowModelOption> AvailableModelOptions { get; }
    public ProjectAutomationState ProjectAutomation => _projectAutomation;
    public bool HasAvailableModelChoices => AvailableModelOptions.Any(option => !option.IsInherited);
    public ObservableCollection<SummarizerChapterOption> SummarizerChapterOptions { get; }
    public bool HasSummarizerChapterChoices => SummarizerChapterOptions.Count > 0;

    // ==================================================================
    // U145：标识 / 别名候选源。
    //
    // 这几组值此前全靠用户手打，而后端对它们是精确等值匹配——手打即错，
    // 且错了只是静默无结果。候选**产品全都持有**，来源逐条列在下面。
    // 控件用 AutoCompleteBox（可搜索 + 列表外的值仍可提交），
    // 因为「引用尚未创建的节点」这类高级场景必须留活口。
    // ==================================================================

    /// <summary>数据别名候选：来自当前节点已连上的数据边（别名在连边时就已定义过一次）。</summary>
    public ObservableCollection<string> NodeAliasCandidates { get; }
    /// <summary>章节 id 候选：来自作品树（后端 ChapterDocumentIndex）。</summary>
    public ObservableCollection<string> ChapterIdCandidates { get; }
    /// <summary>章节文档 id 候选：同上，与章节 id 一一对应。</summary>
    public ObservableCollection<string> ChapterDocumentIdCandidates { get; }
    /// <summary>审批 id 候选：画布上所有审批节点已用的 id（含本节点的默认值）。</summary>
    public ObservableCollection<string> ApprovalIdCandidates { get; }
    /// <summary>
    /// 导出产物 id 候选：画布上各导出节点已用的 id + 一个带 `exports/` 前缀的建议值。
    ///
    /// ⚠️ **前缀不是装饰**：`documents/service.rs` 的 `artifact_path` 只对
    /// `exports/` 前缀做导出根重定向，否则一律落 `.runtime/artifacts`——
    /// 这正是 U134 那条「用户在设置里配的导出目录完全不生效」。U134 只修了作品页
    /// 合并导出（后端自己生成 id），**工作流导出节点这条仍要求前端给出 artifact_id**
    /// （`workflow/integration.rs` 的 `require_non_empty_node_field`），
    /// 所以这里必须把带前缀的正确形态摆进候选，否则用户照旧只能手打出一个落错目录的值。
    /// </summary>
    public ObservableCollection<string> ExportArtifactIdCandidates { get; }
    /// <summary>源引脚名候选：来自节点类型定义（NodePortSpec），用户无从凭记忆写对。</summary>
    public ObservableCollection<string> SourceHandleCandidates { get; }
    /// <summary>目标引脚名候选：目标节点**实际存在**的数据入引脚 + 执行入。</summary>
    public ObservableCollection<string> TargetHandleCandidates { get; }
    /// <summary>通信边正/反向别名候选：通信边的约定别名 + 画布上其它通信边已用过的。</summary>
    public ObservableCollection<string> CommunicationAliasCandidates { get; }

    /// <summary>
    /// 重算全部标识候选。**必须在选中节点/边变化后调用**——候选依赖「当前选中的是谁」
    /// （目标引脚要看目标节点有几个数据入），选中不变而候选不变时 Sync 不会动集合。
    /// </summary>
    private void RefreshIdentifierCandidates()
    {
        IdentifierCandidates.Sync(NodeAliasCandidates, ComposeNodeAliasCandidates());
        IdentifierCandidates.Sync(
            ChapterIdCandidates,
            IdentifierCandidates.Compose(SummarizerChapterOptions.Select(option => option.ChapterId)));
        IdentifierCandidates.Sync(
            ChapterDocumentIdCandidates,
            IdentifierCandidates.Compose(SummarizerChapterOptions.Select(option => option.DocumentId)));
        IdentifierCandidates.Sync(
            ApprovalIdCandidates,
            IdentifierCandidates.Compose(
                Nodes.Where(node => node.IsApprovalNode).Select(node => node.ApprovalId)));
        IdentifierCandidates.Sync(ExportArtifactIdCandidates, ComposeExportArtifactIdCandidates());
        IdentifierCandidates.Sync(
            SourceHandleCandidates,
            IdentifierCandidates.Compose(NodePortSpec.SourceHandleNames()));
        IdentifierCandidates.Sync(TargetHandleCandidates, ComposeTargetHandleCandidates());
        IdentifierCandidates.Sync(
            CommunicationAliasCandidates,
            IdentifierCandidates.Compose(
                NodePortSpec.CommunicationAliasNames(),
                Edges.Where(edge => edge.IsCommunication)
                    .SelectMany(edge => new[] { edge.ForwardAlias, edge.ReverseAlias })));
    }

    /// <summary>
    /// 数据别名候选：当前节点入边上已定义的别名优先，其后是节点类型的约定默认值。
    ///
    /// 入边别名排在最前是因为它才是**这个字段真正要对上的东西**：
    /// 节点读取输入用的就是边上那个别名，两边不一致就取不到值。
    /// </summary>
    private List<string> ComposeNodeAliasCandidates()
    {
        var nodeId = SelectedNode?.Id;
        var incoming = string.IsNullOrEmpty(nodeId)
            ? Enumerable.Empty<string>()
            : Edges
                .Where(edge => edge.IsData && string.Equals(edge.Target, nodeId, StringComparison.Ordinal))
                .Select(edge => string.IsNullOrWhiteSpace(edge.Label) ? edge.TargetHandle : edge.Label);
        return IdentifierCandidates.Compose(incoming, NodePortSpec.ConventionalAliasNames());
    }

    /// <summary>
    /// 导出产物 id 候选：先给当前节点一个**带 `exports/` 前缀**的建议值，
    /// 再列出画布上其它导出节点已用的 id。
    ///
    /// 建议值排第一是刻意的：不带前缀的产物会落进 `.runtime/artifacts`
    /// 而不是用户配置的导出目录（U134 的病灶）。把正确形态放在第一位，
    /// 用户点一下就对，而不用先知道这条前缀规则的存在。
    /// </summary>
    private List<string> ComposeExportArtifactIdCandidates()
    {
        var suggested = SelectedNode is { IsExportNode: true } node
            ? new[] { $"exports/{node.Id}" }
            : Array.Empty<string>();
        return IdentifierCandidates.Compose(
            suggested,
            Nodes.Where(item => item.IsExportNode).Select(item => item.ExportArtifactId));
    }

    /// <summary>
    /// 目标引脚候选：取**目标节点实际存在的**数据入引脚，而不是笼统列一堆名字。
    ///
    /// 「data-in-3」在只有一个数据入的节点上是错的（连线落不到任何引脚），
    /// 所以候选必须按目标节点的引脚列表算——否则下拉本身就在教用户填错。
    /// </summary>
    private List<string> ComposeTargetHandleCandidates()
    {
        var targetId = SelectedEdge?.Target;
        var target = string.IsNullOrEmpty(targetId)
            ? null
            : Nodes.FirstOrDefault(node => string.Equals(node.Id, targetId, StringComparison.Ordinal));
        var pins = target is null
            ? Enumerable.Empty<string>()
            : target.DataInPins.Select(pin => pin.Handle);
        return IdentifierCandidates.Compose(pins, NodePortSpec.TargetHandleNames());
    }

    private ObservableCollection<NodeLibraryItemViewModel> CreateNodeLibraryGroup(string group)
    {
        return new ObservableCollection<NodeLibraryItemViewModel>(
            WorkflowNodeCatalog.ForGroup(group).Select(entry =>
            {
                var nodeType = entry.NodeType;
                return new NodeLibraryItemViewModel(
                    nodeType,
                    _displayNames.Text(entry.DisplayNameKey),
                    () => AddNode(nodeType));
            }));
    }

    public void RefreshLocalizedUi()
    {
        foreach (var group in new[] { EntryNodes, WritingAgents, UtilityNodes })
        {
            foreach (var item in group)
            {
                var descriptor = WorkflowNodeCatalog.ForGroup(
                        ReferenceEquals(group, EntryNodes) ? "entry"
                        : ReferenceEquals(group, WritingAgents) ? "writing"
                        : "utility")
                    .FirstOrDefault(candidate => candidate.NodeType == item.NodeType);
                if (descriptor is not null)
                {
                    item.Title = _displayNames.Text(descriptor.DisplayNameKey);
                }
            }
        }
        if (AvailableModelOptions.Count > 0 && AvailableModelOptions[0].IsInherited)
        {
            AvailableModelOptions[0] = WorkflowModelOption.Inherited(ModelInheritGlobalText);
        }
        // 知识查询面板是**常驻**在项目 AI 栏里的（不像 Ctrl+K 那个弹出面板可以靠
        // 「关掉重开」换语言），它的文案属性挂在自己身上，页面这句
        // OnPropertyChanged(string.Empty) 到不了 —— 必须显式转发。
        KnowledgeLookup.RefreshLocalizedText();
        OnPropertyChanged(string.Empty);
    }

    public string Title => _displayNames.Text("ui.nav.workspace");
    public string SaveText => _displayNames.Text("ui.workspace.save");
    public string ReloadProjectCanvasText => _displayNames.Text("ui.workspace.reload_default");
    public string ExportText => _displayNames.Text("ui.workspace.export");
    public string UndoText => _displayNames.Text("ui.action.undo");
    public string RedoText => _displayNames.Text("ui.action.redo");
    public string RunText => _displayNames.Text("ui.workspace.run");
    public string RunFromStartText => _displayNames.Text("ui.workspace.run_from_start");
    public string CurrentRunText => _displayNames.Text("ui.workspace.current_run");
    public string CurrentRunValueText => string.IsNullOrWhiteSpace(CurrentRunId) ? _displayNames.Text("ui.common.none") : CurrentRunId;
    public string InDoubtTitleText => _displayNames.Text("ui.workspace.in_doubt.title");
    public string InDoubtHintText => _displayNames.Text("ui.workspace.in_doubt.hint");
    public string InDoubtResponseText => _displayNames.Text("ui.workspace.in_doubt.response");
    public string InDoubtResponsePlaceholder => _displayNames.Text("ui.workspace.in_doubt.response.placeholder");
    public string InDoubtReasonText => _displayNames.Text("ui.workspace.in_doubt.reason");
    public string RetryInDoubtText => _displayNames.Text("ui.workspace.in_doubt.retry");
    public string UseInDoubtResponseText => _displayNames.Text("ui.workspace.in_doubt.use_response");
    public string StopInDoubtText => _displayNames.Text("ui.workspace.in_doubt.stop");
    public string NoStartNodesText => _displayNames.Text("ui.workspace.no_start_nodes");
    public string EmptyStartTitle => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.workspace.start.title")
        : _displayNames.Text("ui.empty.need_project.title");
    public string EmptyStartHint => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.workspace.start.hint")
        : _displayNames.Text("ui.empty.need_project.hint");
    public string EmptyCanvasTitle => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.workspace.start.title")
        : _displayNames.Text("ui.empty.need_project.title");
    public string EmptyCanvasHint => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.workspace.start.hint")
        : _displayNames.Text("ui.empty.need_project.hint");
    public string EmptyProjectAiTitle => _displayNames.Text("ui.empty.workspace.ai.title");
    public string EmptyProjectAiHint => _displayNames.Text("ui.empty.workspace.ai.hint");
    public string SelectStartNodeText => _displayNames.Text("ui.workspace.select_start_node");
    public string NodeLibraryText => _displayNames.Text("ui.workspace.node_library");
    public string ExecutionText => _displayNames.Text("ui.workspace.execution");
    public string LibraryDragHintText => _displayNames.Text("ui.workspace.library.drag_hint");
    public string ExecutionHintText => _displayNames.Text("ui.workspace.execution.hint");
    public string WritingAgentsText => _displayNames.Text("ui.workspace.writing_agents");
    public string UtilityNodesText => _displayNames.Text("ui.workspace.utility_nodes");
    public string ProjectAiText => _displayNames.Text("ui.works.project_ai");
    public string NodeDetailsText => _displayNames.Text("ui.workspace.node_details");
    public string ProjectAiPlaceholder => _displayNames.Text("ui.workspace.project_ai.placeholder");
    public string ProjectAiEmptyText => _displayNames.Text("ui.workspace.project_ai.empty");
    public string CanvasHintText => _displayNames.Text("ui.workspace.logs_hint");
    public string ToggleRightPanelText => _displayNames.Text("ui.action.toggle_right_panel");
    public string EntryNodesText => _displayNames.Text("ui.workspace.entry_nodes");
    public string NodeNameLabel => _displayNames.Text("ui.workspace.start_node.name_label");
    public string WorkDirLabel => _displayNames.Text("ui.workspace.start_node.work_dir_label");
    public string WorkDirPlaceholder => _displayNames.Text("ui.workspace.start_node.work_dir_placeholder");
    public string BrowseWorkDirText => _displayNames.Text("ui.workspace.start_node.browse_work_dir");
    public string ExposeToolLabel => _displayNames.Text("ui.workspace.start_node.expose_tool");
    public string UserNoteLabel => _displayNames.Text("ui.workspace.start_node.user_note");
    public string UserNotePlaceholder => _displayNames.Text("ui.workspace.start_node.user_note_placeholder");
    public string NoNodeSelectedText => _displayNames.Text("ui.workspace.no_node_selected");
    public string SelectedNodeTitle => SelectedNode?.Label ?? NoNodeSelectedText;
    public string DeleteText => _displayNames.Text("ui.workspace.context.delete");
    public string PauseText => _displayNames.Text("ui.workspace.pause");
    public string StopText => _displayNames.Text("ui.workspace.stop");
    public string ResumeText => _displayNames.Text("ui.workspace.resume");
    public string ConfirmationsText => _displayNames.Text("ui.workspace.confirmations");
    public string ConfirmationsEmptyText => _displayNames.Text("ui.workspace.confirmations.empty");
    public string ConfirmationCountText => _displayNames.Format("ui.workspace.confirmations.count", new Dictionary<string, string>
    {
        ["count"] = Confirmations.Count.ToString(),
    });
    public string RefreshConfirmationsText => _displayNames.Text("ui.workspace.confirmations.reload");
    public string ExpandConfirmationsText => _displayNames.Text("ui.workspace.confirmations.expand");
    public string CollapseConfirmationsText => _displayNames.Text("ui.workspace.confirmations.collapse");
    public string ConfirmationsBannerText => _displayNames.Format("ui.workspace.confirmations.banner", new Dictionary<string, string>
    {
        ["count"] = Confirmations.Count.ToString(),
    });
    public string ConfirmationDiffText => _displayNames.Text("ui.workspace.confirmation.diff");
    // ConfirmationReasonText / ConfirmationReasonPlaceholder 随常驻「审阅理由」输入框一起删除（U139③）：
    // 理由现在只在点「驳回」后展开的那条线上出现，提示语由 RejectReasonPromptText 承担。
    // 留着两个零消费者的文案属性会让人以为那个框还在（AGENTS §4「死代码会冒充已完成的工作」）。
    public string ApproveConfirmationText => _displayNames.Text("ui.workspace.confirmation.approve");
    public string RejectConfirmationText => _displayNames.Text("ui.workspace.confirmation.reject");
    /// <summary>审阅面板左栏小标题：说明这一列是「等你拍板的条目」。</summary>
    public string ConfirmationListTitleText => _displayNames.Text("ui.workspace.confirmation.list_title");
    /// <summary>「问问 AI」按钮文案：把当前确认项交给项目 AI 评估。</summary>
    public string AskAiAboutConfirmationText => _displayNames.Text("ui.workspace.confirmation.ask_ai");
    public string AskAiAboutConfirmationHintText => _displayNames.Text("ui.workspace.confirmation.ask_ai.hint");
    /// <summary>确认项不带正文改动时的说明；空白面会让人以为加载失败。</summary>
    public string ConfirmationDiffEmptyText => _displayNames.Text("ui.workspace.confirmation.diff_empty");

    /// <summary>审批历史折叠区标题（U187-A）：这一段回答「我到底批准过什么」。</summary>
    public string ConfirmationHistoryTitleText => _displayNames.Text("ui.workspace.confirmation.history");

    /// <summary>画布工具条上的历史入口文案：无待审项时唯一够得着的路。</summary>
    public string OpenConfirmationHistoryText =>
        _displayNames.Text("ui.workspace.confirmation.history.open");

    /// <summary>折叠区里的一句说明：讲清这里的条目已决议、不需要也不能再处置。</summary>
    public string ConfirmationHistoryHintText => _displayNames.Text("ui.workspace.confirmation.history.hint");

    /// <summary>折叠区开合按钮文案，随状态换词——按钮要说它会做什么，而不是它现在是什么。</summary>
    public string ConfirmationHistoryToggleText => IsConfirmationHistoryExpanded
        ? _displayNames.Text("ui.workspace.confirmation.history.collapse")
        : _displayNames.Text("ui.workspace.confirmation.history.expand");

    /// <summary>历史条数。与待审计数分开两句文案：混在一起读者分不清哪个数字要他动手。</summary>
    public string ConfirmationHistoryCountText => _displayNames.Format(
        "ui.workspace.confirmation.history.count",
        new Dictionary<string, string>
        {
            ["count"] = ResolvedConfirmations.Count.ToString(),
        });
    public string PromptTemplateText => _displayNames.Text("ui.workspace.prompt_template");

    /// <summary>
    /// U150：Ctrl+左键展开引用的**可发现性**提示。
    ///
    /// 手势不写出来等于没做——用户不会去猜「按住 Ctrl 点一下试试」。
    /// 这一行同时说清了另一半语义（「发给模型时一律展开」），
    /// 否则看到折叠摘要的人会合理地担心「模型是不是只收到这一行摘要」。
    /// </summary>
    public string PromptReferenceHintText => _displayNames.Text("ui.node.prompt.reference_hint");

    /// <summary>
    /// U201-B：提示词编辑器 Ctrl+左键预览引用正文时，用它去取那份文档。
    ///
    /// # 为什么在 VM 上而不是让控件自己拿客户端
    ///
    /// 控件是纯呈现层，不该知道 IPC 的存在；而后端客户端只有页面 VM 持有。
    /// 暴露成一个委托属性，XAML 一行绑定即可，控件仍然可以在单测里被注入假实现。
    ///
    /// ⚠️ **这是「真预览」的最后一环**。缺了它，Ctrl+左键会走
    /// `PromptTemplateEditor` 的「预览暂不可用」分支 —— 而所有单测（直接给控件
    /// 赋委托）照样全绿。U150 那一版就是在这一环上停住的：能力做好了、
    /// 没接到用户看得见处。
    ///
    /// 每次调用现问后端、不缓存：正文会被别处改（作者自己编辑、工作流写回），
    /// 缓存一份就会让预览显示旧内容，而「预览」这个动作的全部意义是看当下是什么。
    /// </summary>
    public Func<string, Task<string?>> ReferenceTextProvider => async documentId =>
    {
        try
        {
            return await _backend.GetDocumentContentAsync(documentId).ConfigureAwait(true);
        }
        catch
        {
            // 吞掉异常并返回 null：控件那边把 null 当「取不到」处理，
            // 会显示可读的「预览不可用」而不是让异常冒到 UI 线程上。
            // 具体是文档不存在、越权还是后端没起来，对「我只想看一眼引用」
            // 这个动作没有分辨价值 —— 三者的下一步都是「检查这条引用写对了没」。
            return null;
        }
    };
    public string ModelIdText => _displayNames.Text("ui.workspace.model_id");
    public string ModelSelectorPlaceholder => _displayNames.Text("ui.workspace.model_selector_placeholder");
    public string ModelInheritGlobalText => _displayNames.Text("ui.workspace.model_inherit_global");
    public string NodeBudgetText => _displayNames.Text("ui.workspace.node_budget");
    public string NodeTimeoutText => _displayNames.Text("ui.workspace.node_timeout_seconds");
    public string OptionalPlaceholder => _displayNames.Text("ui.common.optional");
    public string SecondsUnitText => _displayNames.Text("ui.common.unit.seconds");
    public string BreakpointText => _displayNames.Text("ui.workspace.breakpoint");
    public string SaveBreakpointText => _displayNames.Text("ui.workspace.breakpoint.save");
    public string ApplyNodeConfigText => _displayNames.Text("ui.workspace.apply_node_config");
    public string ExportSelectionText => _displayNames.Text("ui.workspace.export_selection");
    public string AddAnnotationText => _displayNames.Text("ui.workspace.add_annotation");
    public string AnnotationTitleText => _displayNames.Text("ui.workspace.annotation_title");
    public string AnnotationTitlePlaceholder => _displayNames.Text("ui.workspace.annotation_title.placeholder");
    public string SubworkflowText => _displayNames.Text("ui.workspace.subworkflow");
    public string EdgeDetailsText => _displayNames.Text("ui.workspace.edge_details");
    /// <summary>与当前选中节点相关的边数量（非整图）。</summary>
    public string EdgeCountText => $"{RelatedEdges.Count}";
    public bool HasRelatedEdges => RelatedEdges.Count > 0;
    public string SourceAliasText => _displayNames.Text("ui.workspace.edge.source_alias");
    public string TargetAliasText => _displayNames.Text("ui.workspace.edge.target_alias");
    public string EdgeLabelText => _displayNames.Text("ui.workspace.edge.label");
    public string EdgeDataText => _displayNames.Text("ui.workspace.edge.data");
    public string ApplyEdgeConfigText => _displayNames.Text("ui.workspace.apply_edge_config");
    public string ForwardAliasText => _displayNames.Text("ui.workspace.edge.forward_alias");
    public string ReverseAliasText => _displayNames.Text("ui.workspace.edge.reverse_alias");
    public string ForwardTemplateText => _displayNames.Text("ui.workspace.edge.forward_template");
    public string ReverseTemplateText => _displayNames.Text("ui.workspace.edge.reverse_template");
    public string MaxCommunicationCountText => _displayNames.Text("ui.workspace.edge.max_communication_count");
    public string InsertForwardVariableText => _displayNames.Text("ui.workspace.edge.insert_forward_variable");
    public string InsertReverseVariableText => _displayNames.Text("ui.workspace.edge.insert_reverse_variable");
    public string TemplatePreviewText => _displayNames.Text("ui.workspace.edge.template_preview");
    // 正/反向预览各自限定标签：同一面板里两处预览若都叫「预览」，无法区分是哪条。
    public string ForwardTemplatePreviewLabel => _displayNames.Text("ui.workspace.edge.forward_template_preview");
    public string ReverseTemplatePreviewLabel => _displayNames.Text("ui.workspace.edge.reverse_template_preview");
    public string PortControlInTip => _displayNames.Text("ui.workspace.port.control_in");
    public string PortControlOutTip => _displayNames.Text("ui.workspace.port.control_out");
    /// <summary>U125：condition「条件成立」分支引脚提示。</summary>
    public string PortControlOutTrueTip => _displayNames.Text("ui.workspace.port.control_out_true");
    /// <summary>U125：condition「条件不成立」分支引脚提示。</summary>
    public string PortControlOutFalseTip => _displayNames.Text("ui.workspace.port.control_out_false");
    public string PortDataInTip => _displayNames.Text("ui.workspace.port.data_in");
    public string PortDataOutTip => _displayNames.Text("ui.workspace.port.data_out");
    public string PortCommunicationTip => _displayNames.Text("ui.workspace.port.communication");
    public string ImportFileLabel => _displayNames.Text("ui.workspace.import.file");
    public string ImportFileHint => _displayNames.Text("ui.workspace.import.path_hint");
    public string ImportNoFileText => _displayNames.Text("ui.workspace.import.no_file");
    public string BrowseImportFileText => _displayNames.Text("ui.workspace.import.browse");
    public string IncludeContentText => _displayNames.Text("ui.workspace.document.include_content");
    public string SearchNodeTitle => _displayNames.Text("ui.workspace.search.title");
    public string SearchNodeHint => _displayNames.Text("ui.workspace.search.hint");
    public string QueryAliasLabel => _displayNames.Text("ui.workspace.search.query_alias");
    public string SearchQueryPlaceholder => _displayNames.Text("ui.workspace.search.query_placeholder");
    public string SearchLimitLabel => _displayNames.Text("ui.workspace.search.limit");
    public string ConditionNodeTitle => _displayNames.Text("ui.workspace.condition.title");
    public string ConditionNodeHint => _displayNames.Text("ui.workspace.condition.hint");
    public string ConditionInputAliasLabel => _displayNames.Text("ui.workspace.condition.input_alias");
    public string ConditionOperatorLabel => _displayNames.Text("ui.workspace.condition.operator");
    public string ConditionExpectedLabel => _displayNames.Text("ui.workspace.condition.expected");
    public string ConditionExpectedPlaceholder => _displayNames.Text("ui.workspace.condition.expected_placeholder");
    public string LoopNodeTitle => _displayNames.Text("ui.workspace.loop.title");
    public string LoopNodeHint => _displayNames.Text("ui.workspace.loop.hint");
    public string MaxIterationsLabel => _displayNames.Text("ui.workspace.loop.max_iterations");
    public string StopInputAliasLabel => _displayNames.Text("ui.workspace.loop.stop_input_alias");
    public string StopExpectedLabel => _displayNames.Text("ui.workspace.loop.stop_expected");
    public string ApprovalNodeTitle => _displayNames.Text("ui.workspace.approval.title");
    public string ApprovalNodeHint => _displayNames.Text("ui.workspace.approval.hint");
    public string ApprovalIdLabel => _displayNames.Text("ui.workspace.approval.id");
    public string AutoApproveText => _displayNames.Text("ui.workspace.approval.auto_approve");
    public string ExportNodeTitle => _displayNames.Text("ui.workspace.export_node.title");
    public string ExportNodeHint => _displayNames.Text("ui.workspace.export_node.hint");
    public string ExportArtifactIdLabel => _displayNames.Text("ui.workspace.export_node.artifact_id");
    /// <summary>U145：把「exports/ 前缀决定落哪个目录」这条隐规则说出来（U134 的病灶）。</summary>
    public string ExportArtifactIdHint => _displayNames.Text("ui.workspace.export_node.artifact_id_hint");
    public string ExportFormatLabel => _displayNames.Text("ui.workspace.export_node.format");
    public string ExportTitleLabel => _displayNames.Text("ui.workspace.export_node.title_field");
    public string SummarizerNodeTitle => _displayNames.Text("ui.workspace.summarizer.title");
    public string SummarizerNodeHint => _displayNames.Text("ui.workspace.summarizer.hint");
    public string SummarizerChapterSelectorLabel => _displayNames.Text("ui.workspace.summarizer.chapter_selector");
    public string SummarizerChapterSelectorPlaceholder => _displayNames.Text("ui.workspace.summarizer.chapter_selector_placeholder");
    public string SummarizerChapterIdLabel => _displayNames.Text("ui.workspace.summarizer.chapter_id");
    public string SummarizerChapterIdPlaceholder => _displayNames.Text("ui.workspace.summarizer.chapter_id_placeholder");
    public string SummarizerChapterDocumentIdLabel => _displayNames.Text("ui.workspace.summarizer.chapter_document_id");
    public string SummarizerChapterDocumentIdPlaceholder => _displayNames.Text("ui.workspace.summarizer.chapter_document_id_placeholder");
    public string SummarizerChapterTextAliasLabel => _displayNames.Text("ui.workspace.summarizer.chapter_text_alias");
    public string SummarizerChapterTextAliasHint => _displayNames.Text("ui.workspace.summarizer.chapter_text_alias_hint");
    public string SummarizerAutoModeText => _displayNames.Text("ui.workspace.summarizer.auto_mode");
    public string DataInPinsLabel => _displayNames.Text("ui.workspace.pin.data_inputs_label");
    public string AddDataInPinText => _displayNames.Text("ui.workspace.pin.add_data_in");
    public string RemoveDataInPinText => _displayNames.Text("ui.workspace.pin.remove_data_in");
    public string ZoomInText => _displayNames.Text("ui.workspace.zoom_in");
    public string ZoomOutText => _displayNames.Text("ui.workspace.zoom_out");
    public string ResetZoomText => _displayNames.Text("ui.workspace.zoom_reset");
    // 缩放按钮的 `+`/`-` 字形属性已删除（连同 ui.workspace.zoom_*_glyph 两个 key）：
    // 那两个按钮改用矢量图标 Ariadne.Icon.Add / .Subtract。字形靠字体基线居中，
    // 连字符在 em 框里偏上、加号居中，两者对不齐——这是它被换掉的原因，别再加回来。
    public string MinimapText => _displayNames.Text("ui.workspace.minimap");
    public string CanvasOverviewFocusText => _displayNames.Text("ui.workspace.canvas_overview_focus");
    public string CanvasFocusText => _displayNames.Text(
        IsCanvasFocusMode
            ? "ui.workspace.canvas_focus_exit"
            : "ui.workspace.canvas_focus");
    public string CanvasZoomText => _displayNames.Format("ui.workspace.zoom_percent", new Dictionary<string, string>
    {
        ["percent"] = Math.Round(CanvasZoom * 100).ToString("0"),
    });

    public bool IsRightPanelOpen
    {
        get => _isRightPanelOpen;
        set
        {
            if (SetProperty(ref _isRightPanelOpen, value))
            {
                ExitCanvasFocusModeForPanelOpen(value);
                OnPropertyChanged(nameof(RightPanelSplitterWidth));
                OnPropertyChanged(nameof(RightPanelColumnWidth));
                OnPropertyChanged(nameof(IsRightPanelDocked));
            }
        }
    }
    public RelayCommand ToggleRightPanelCommand { get; }

    public void ApplyUiPreferences(UiPreferences preferences)
    {
        if (preferences.PanelStates?.TryGetValue(RightPanelPreferenceKey, out var isOpen) == true)
        {
            IsRightPanelOpen = isOpen;
        }
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
            StatusText = ReportFailure(ex, _displayNames);
        }
    }
    public RelayCommand ToggleCanvasFocusModeCommand { get; }
    public bool UseOverlayRightPanel => ResponsiveRightPanelLayout.UseOverlayRightPanel;
    public bool IsRightPanelDocked => IsRightPanelOpen && !UseOverlayRightPanel;
    public double RightPanelMaximumWidth => ResponsiveRightPanelLayout.MaximumDockedRightPanelWidth;
    public double RightPanelOverlayWidth => ResponsiveRightPanelLayout.OverlayRightPanelWidth;
    public GridLength RightPanelSplitterWidth => IsRightPanelDocked
        ? new GridLength(WorkspaceResponsiveLayoutHelpers.RightPanelSplitterWidth)
        : new GridLength(0);
    public GridLength RightPanelColumnWidth
    {
        get => IsRightPanelDocked
            ? new GridLength(ResponsiveRightPanelLayout.DockedRightPanelWidth)
            : new GridLength(CollapsedRightPanelWidth);
        set
        {
            if (!IsRightPanelDocked)
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

    public void SetAvailableWorkspaceWidth(double width)
    {
        if (!double.IsFinite(width)
            || width <= 0
            || Math.Abs(_availableWorkspaceWidth - width) < 0.5)
        {
            return;
        }

        _availableWorkspaceWidth = width;
        OnPropertyChanged(nameof(UseOverlayRightPanel));
        OnPropertyChanged(nameof(IsRightPanelDocked));
        OnPropertyChanged(nameof(RightPanelSplitterWidth));
        OnPropertyChanged(nameof(RightPanelColumnWidth));
        OnPropertyChanged(nameof(RightPanelMaximumWidth));
        OnPropertyChanged(nameof(RightPanelOverlayWidth));
    }

    private WorkspaceResponsiveLayout ResponsiveRightPanelLayout =>
        WorkspaceResponsiveLayoutHelpers.Compute(
            _availableWorkspaceWidth,
            _rightPanelColumnWidth.IsAbsolute
                ? _rightPanelColumnWidth.Value
                : 360,
            IsRightPanelOpen);
    public bool IsLibraryOpen
    {
        get => _isLibraryOpen;
        set
        {
            if (SetProperty(ref _isLibraryOpen, value))
            {
                ExitCanvasFocusModeForPanelOpen(value);
                OnPropertyChanged(nameof(BottomPanelShowsCollapseGlyph));
            }
        }
    }
    public RelayCommand ToggleLibraryCommand { get; }
    public bool IsCanvasFocusMode
    {
        get => _isCanvasFocusMode;
        private set
        {
            if (SetProperty(ref _isCanvasFocusMode, value))
            {
                OnPropertyChanged(nameof(CanvasFocusText));
            }
        }
    }

    private void ToggleCanvasFocusMode()
    {
        if (!IsCanvasFocusMode)
        {
            _focusRestoreLibraryOpen = IsLibraryOpen;
            _focusRestoreRightPanelOpen = IsRightPanelOpen;
            IsLibraryOpen = false;
            IsRightPanelOpen = false;
            IsCanvasFocusMode = true;
            StatusText = _displayNames.Text("ui.workspace.canvas_focus_entered");
            return;
        }

        IsCanvasFocusMode = false;
        IsLibraryOpen = _focusRestoreLibraryOpen;
        IsRightPanelOpen = _focusRestoreRightPanelOpen;
        StatusText = _displayNames.Text("ui.workspace.canvas_focus_exited");
    }

    private void ExitCanvasFocusModeForPanelOpen(bool opening)
    {
        if (opening && IsCanvasFocusMode)
        {
            IsCanvasFocusMode = false;
        }
    }

    public double CanvasZoom
    {
        get => _canvasViewport.Zoom;
        private set => SetCanvasZoom(value);
    }

    /// <summary>A5：视口会话是 zoom、offset 与 pan 生命周期的唯一状态源。</summary>
    internal CanvasViewportSession CanvasViewport => _canvasViewport;

    /// <summary>W2：产品路径设置缩放（FitView / 滚轮 / 工具栏共用）。</summary>
    public void SetCanvasZoom(double value)
    {
        var previousZoom = _canvasViewport.Zoom;
        var state = _canvasViewport.SetZoom(value);
        NotifyCanvasZoomChanged(previousZoom, state.Zoom);
    }

    internal CanvasViewportState SetCanvasZoomAt(double value, double anchorX, double anchorY)
    {
        var previousZoom = _canvasViewport.Zoom;
        var state = _canvasViewport.ZoomAt(value, anchorX, anchorY);
        NotifyCanvasZoomChanged(previousZoom, state.Zoom);
        return state;
    }

    internal CanvasViewportState FitCanvasViewport(
        double minX,
        double minY,
        double maxX,
        double maxY,
        CanvasViewportRect safeViewport)
    {
        var previousZoom = _canvasViewport.Zoom;
        var state = _canvasViewport.Fit(minX, minY, maxX, maxY, safeViewport);
        NotifyCanvasZoomChanged(previousZoom, state.Zoom);
        return state;
    }

    private void NotifyCanvasZoomChanged(double previousZoom, double nextZoom)
    {
        if (Math.Abs(previousZoom - nextZoom) >= 1e-9)
        {
            OnPropertyChanged(nameof(CanvasZoom));
            OnPropertyChanged(nameof(CanvasZoomText));
            OnPropertyChanged(nameof(ShowCanvasDetails));
            OnPropertyChanged(nameof(ShowCanvasPrecisionControls));
            BroadcastSemanticZoomToItems();
        }
    }

    /// <summary>
    /// U178-B：把两个页面级语义缩放开关广播到每个节点/边 VM。
    ///
    /// 这是「脱掉 per-item 祖先绑定」的另一半——模板不再向上取值，
    /// 就必须由页面主动下推，否则缩放后卡片不会跟着变。
    ///
    /// 只在 zoom **跨过阈值**时才真正写值：`SetProperty` 同值不发通知，
    /// 所以连续缩放中绝大多数格数在这里是纯比较、零通知，
    /// 不会把省下的祖先绑定成本换成一堆属性通知。
    /// </summary>
    private void BroadcastSemanticZoomToItems()
    {
        var details = ShowCanvasDetails;
        var precision = ShowCanvasPrecisionControls;
        foreach (var node in Nodes)
        {
            node.ShowCanvasDetails = details;
            node.ShowCanvasPrecisionControls = precision;
        }
        foreach (var edge in Edges)
        {
            edge.ShowCanvasDetails = details;
        }
    }

    /// <summary>
    /// 新加入的节点/边要立刻拿到当前语义缩放态。
    ///
    /// 不做这一步的话，在缩得很小的画布上新建节点会带着「默认 true」的初值出现：
    /// 全部引脚和详情都显示，与周围卡片不一致，直到用户下次缩放才纠正。
    /// </summary>
    internal void ApplySemanticZoomTo(WorkflowNodeViewModel node)
    {
        node.ShowCanvasDetails = ShowCanvasDetails;
        node.ShowCanvasPrecisionControls = ShowCanvasPrecisionControls;
    }

    internal void ApplySemanticZoomTo(WorkflowEdgeViewModel edge)
    {
        edge.ShowCanvasDetails = ShowCanvasDetails;
    }

    /// <summary>W9：总览倍率隐藏正文和边标签，仅保留节点身份、运行态和拓扑骨架。</summary>
    public bool ShowCanvasDetails => CanvasSemanticZoomHelpers.ShowDetails(CanvasZoom);

    /// <summary>W9：端口、运行按钮和拖动只在可编辑倍率出现。</summary>
    public bool ShowCanvasPrecisionControls =>
        CanvasSemanticZoomHelpers.AllowPrecisionControls(CanvasZoom);

    public RelayCommand ZoomInCommand { get; }
    public RelayCommand ZoomOutCommand { get; }
    public RelayCommand ResetZoomCommand { get; }

    public bool IsExecutionPanel
    {
        get => _isExecutionPanel;
        set
        {
            if (SetProperty(ref _isExecutionPanel, value))
            {
                OnPropertyChanged(nameof(IsNodeLibraryPanel));
                OnPropertyChanged(nameof(BottomPanelToggleText));
            }
        }
    }

    public bool IsNodeLibraryPanel => !IsExecutionPanel;
    public RelayCommand ShowNodeLibraryCommand { get; }
    public RelayCommand ShowExecutionCommand { get; }

    public bool IsProjectAiTab
    {
        get => _rightPanelTab == WorkspaceRightPanelTab.ProjectAi;
        set => SetRightPanelTab(value
            ? WorkspaceRightPanelTab.ProjectAi
            : WorkspaceRightPanelTab.NodeDetails);
    }

    public bool IsNodeDetailsTab => _rightPanelTab == WorkspaceRightPanelTab.NodeDetails;
    public bool IsEdgeDetailsTab => _rightPanelTab == WorkspaceRightPanelTab.EdgeDetails;
    public RelayCommand ShowProjectAiCommand { get; }
    public RelayCommand ShowNodeDetailsCommand { get; }
    public RelayCommand ShowEdgeDetailsCommand { get; }
    public RelayCommand ReloadProjectCanvasCommand { get; }
    public RelayCommand ExportCommand { get; }
    public RelayCommand SaveCommand { get; }
    public RelayCommand UndoCommand { get; }
    public RelayCommand RedoCommand { get; }
    public RelayCommand AddContextNodeCommand { get; }
    public RelayCommand AddStartNodeCommand { get; }
    public RelayCommand DeleteSelectedNodeCommand { get; }
    public RelayCommand RunSelectedNodeCommand { get; }
    public RelayCommand PauseWorkflowCommand { get; }
    public RelayCommand StopWorkflowCommand { get; }
    public RelayCommand ResumeWorkflowCommand { get; }

    /// <summary>U196-D：从失败的那个节点重跑，已完成的步骤不重跑。</summary>
    public RelayCommand RetryFailedNodeCommand { get; }
    /// <summary>U207-C①：空闲态下栏标题栏那个「运行」主按钮。</summary>
    public RelayCommand RunWorkflowCommand { get; }
    public RelayCommand SendProjectAiCommand { get; }
    public RelayCommand ApplyNodeConfigCommand { get; }
    public RelayCommand ToggleBreakpointCommand { get; }
    public RelayCommand BrowseWorkDirCommand { get; }
    public RelayCommand BrowseImportFileCommand { get; }
    public RelayCommand AddAnnotationCommand { get; }
    public RelayCommand ExportSelectionCommand { get; }

    /// <summary>View 注入：选文件夹（起始节点 work_dir）。</summary>
    public Func<string?, Task<string?>>? PickFolder { get; set; }
    /// <summary>View 注入：选文件（导入节点）。</summary>
    public Func<string?, Task<string?>>? PickFile { get; set; }
    public RelayCommand PackSelectionCommand { get; }
    public RelayCommand RefreshConfirmationsCommand { get; }
    public RelayCommand ToggleConfirmationPanelCommand { get; }
    public RelayCommand ApproveConfirmationCommand { get; }
    public RelayCommand RejectConfirmationCommand { get; }
    public RelayCommand CancelRejectConfirmationCommand { get; }
    public RelayCommand ApproveOrCancelCommand { get; }

    /// <summary>把当前确认项作为 `@确认项:<id>` 引用发给项目 AI（U139④）。</summary>
    public RelayCommand AskAiAboutConfirmationCommand { get; }

    /// <summary>View 注入：理由输入线展开后把焦点交给它（沿用作品页快捷改写的做法）。</summary>
    public Action? RequestFocusRejectReason { get; set; }

    public RelayCommand RetryInDoubtOperationCommand { get; }
    public RelayCommand UseInDoubtResponseCommand { get; }
    public RelayCommand StopInDoubtOperationCommand { get; }
    public RelayCommand SaveEdgeConfigCommand { get; }
    public RelayCommand InsertForwardTemplateVariableCommand { get; }
    public RelayCommand InsertReverseTemplateVariableCommand { get; }
    public RelayCommand CopySelectedNodeCommand { get; }
    public RelayCommand CutSelectedNodeCommand { get; }
    public RelayCommand PasteNodeCommand { get; }
    public RelayCommand FitViewCommand { get; }

    /// <summary>执行页 Ctrl+K 的「AI 填变量值」面板（13C 第 5 项）。</summary>
    public VariableFillPanelViewModel VariableFill { get; }
    public RelayCommand OpenVariableFillCommand { get; }

    /// <summary>项目 AI 栏里的跨章知识查询面板（U206-B）：`@知识:<词>` 的唯一前端入口。</summary>
    public KnowledgeLookupPanelViewModel KnowledgeLookup { get; }

    public Action? RequestFitView { get; set; }
    public Action<double>? RequestCanvasZoomStep { get; set; }
    public Action? RequestResetCanvasZoom { get; set; }
    public Action<WorkflowNodeViewModel>? RequestEnsureNodeVisible { get; set; }

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
    public string ProjectAiAnswer { get => _projectAiAnswer; set => SetProperty(ref _projectAiAnswer, value); }

    internal int ProjectAiHistoryCount => _projectAiHistory.Count;
    public string CurrentRunId => _runSession.RunId;
    public string ConfirmationReason { get => _confirmationReason; set => SetProperty(ref _confirmationReason, value); }

    /// <summary>
    /// 拒绝已进入「问理由」态：横线已展开，同一按钮变为「确认拒绝」。
    ///
    /// 两步而不是弹危险对话框：拒绝本身是可逆的日常审阅动作，弹窗打断阅读；
    /// 就地展开一条输入线既留住了上下文，又让第二次点击成为那道确认闸口。
    /// </summary>
    public bool IsRejectArmed
    {
        get => _isRejectArmed;
        private set
        {
            if (SetProperty(ref _isRejectArmed, value))
            {
                OnPropertyChanged(nameof(RejectButtonText));
                OnPropertyChanged(nameof(ApproveButtonText));
            }
        }
    }

    /// <summary>同一个按钮的两态文案：未武装为「拒绝」，武装后为「确认拒绝」。</summary>
    public string RejectButtonText => IsRejectArmed
        ? _displayNames.Text("ui.workspace.confirmation.reject.commit")
        : RejectConfirmationText;

    /// <summary>
    /// 主按钮两态：常态是「确认通过」，拒绝武装后让位为「取消」。
    ///
    /// 不另开一个取消键：武装态下「通过」本就不该被点（要通过就不会先点拒绝），
    /// 把它借用成退出口，按钮总数不变，手也不用移到新位置。
    /// </summary>
    public string ApproveButtonText => IsRejectArmed
        ? _displayNames.Text("ui.workspace.confirmation.reject.cancel")
        : ApproveConfirmationText;

    public string RejectReasonPromptText =>
        _displayNames.Text("ui.workspace.confirmation.reject.reason_prompt");

    public string RejectCancelText =>
        _displayNames.Text("ui.workspace.confirmation.reject.cancel");
    public string InDoubtResponseJson { get => _inDoubtResponseJson; set => SetProperty(ref _inDoubtResponseJson, value); }
    public string InDoubtStopReason { get => _inDoubtStopReason; set => SetProperty(ref _inDoubtStopReason, value); }
    public string AnnotationTitle { get => _annotationTitle; set => SetProperty(ref _annotationTitle, value); }

    public bool HasUnsavedChanges
    {
        get => _hasUnsavedChanges;
        private set
        {
            if (SetProperty(ref _hasUnsavedChanges, value))
            {
                OnPropertyChanged(nameof(UnsavedChangesBadgeText));
                OnPropertyChanged(nameof(SaveToolTipText));
            }
        }
    }

    /// <summary>W5：脏状态对作者可见（工具栏 Save 提示 + 角标文案）。</summary>
    public string UnsavedChangesBadgeText =>
        HasUnsavedChanges ? _displayNames.Text("ui.workspace.unsaved_badge") : string.Empty;

    public string SaveToolTipText =>
        HasUnsavedChanges
            ? _displayNames.Text("ui.workspace.save_unsaved_tip")
            : _displayNames.Text("ui.workspace.save_tip");

    /// <summary>W16：底部面板开合标签随当前模式（节点库 / 执行）变化。</summary>
    public string BottomPanelToggleText =>
        IsExecutionPanel
            ? _displayNames.Text("ui.workspace.execution")
            : _displayNames.Text("ui.workspace.node_library");

    /// <summary>W16：开=收起箭头(上)，关=展开箭头(下) — 与动作语义一致。</summary>
    public bool BottomPanelShowsCollapseGlyph => IsLibraryOpen;

    public ObservableCollection<WorkflowNodeViewModel> Nodes { get; }
    public ObservableCollection<WorkflowNodeViewModel> StartNodes { get; }
    public ObservableCollection<NodeLibraryItemViewModel> EntryNodes { get; }
    public ObservableCollection<NodeLibraryItemViewModel> WritingAgents { get; }
    public ObservableCollection<NodeLibraryItemViewModel> UtilityNodes { get; }
    public ObservableCollection<ConfirmationItemViewModel> Confirmations { get; }

    /// <summary>
    /// 审批历史：已决议（approved / rejected / auto_audited）的确认项（U187-A）。
    ///
    /// **刻意与 <see cref="Confirmations"/> 分开，不是重复造集合**：
    /// <see cref="HasPendingConfirmations"/> 只看 <see cref="Confirmations"/>，
    /// 而它驱动审阅面板强制展开并替换整个画布。历史项一旦混进待审集合，
    /// 「有 N 件事等你确认」就恒真，作者再也回不到画布。
    /// 所以这一份只用于呈现审计链，**不参与**面板展开、badge 计数与逐项审批。
    /// </summary>
    public ObservableCollection<ResolvedConfirmationItemViewModel> ResolvedConfirmations { get; } = new();

    /// <summary>
    /// 当前确认项 diff 的分行投影（U139②）。
    ///
    /// 复用作品页快速编辑的 <see cref="QuickEditDiffLineViewModel"/>：两处消费的是**同一份**
    /// 后端 diff 产出（`- ` / `+ ` / 两空格上下文前缀），各写一个解析器迟早会漂移。
    /// 承载控件是 ItemsControl 而非只读 TextBox——后者无法分行着色，且会抢焦点占 Tab 位。
    /// </summary>
    public ObservableCollection<QuickEditDiffLineViewModel> ConfirmationDiffLines { get; } = new();

    /// <summary>有无可渲染的 diff 行；无 diff 的确认项改显一句说明，不留空白面。</summary>
    public bool HasConfirmationDiff => ConfirmationDiffLines.Count > 0;
    public ObservableCollection<WorkflowOperation> InDoubtOperations { get; }
    public ObservableCollection<WorkflowEdgeViewModel> Edges { get; }
    /// <summary>仅当前选中节点相关的边（右栏列表）。</summary>
    public ObservableCollection<WorkflowEdgeViewModel> RelatedEdges { get; }
    public ObservableCollection<ChatBubbleViewModel> ProjectAiBubbles { get; }
    public bool HasProjectAiBubbles => ProjectAiBubbles.Count > 0;

    public WorkflowNodeViewModel? SelectedNode
    {
        get => _selectedNode;
        private set
        {
            if (SetProperty(ref _selectedNode, value))
            {
                OnPropertyChanged(nameof(HasSelectedNode));
                OnPropertyChanged(nameof(IsSelectedStartNode));
                OnPropertyChanged(nameof(SelectedNodeTitle));
                OnPropertyChanged(nameof(SelectedSummarizerChapterOption));
                OnPropertyChanged(nameof(SelectedNodeModelOption));
                NotifyNodeCommandStates();
            }
        }
    }

    public bool HasSelectedNode => SelectedNode is not null;

    /// <summary>节点级 Provider/模型选择。一次选择同时更新两个运行时字段，避免跨 Provider 的同名模型误路由。</summary>
    public WorkflowModelOption? SelectedNodeModelOption
    {
        get
        {
            if (SelectedNode is not { ShowPromptEditor: true } node)
            {
                return null;
            }

            if (string.IsNullOrWhiteSpace(node.ProviderId) && string.IsNullOrWhiteSpace(node.ModelId))
            {
                return AvailableModelOptions.FirstOrDefault(option => option.IsInherited);
            }

            return AvailableModelOptions.FirstOrDefault(option =>
                !option.IsInherited
                && string.Equals(option.ProviderId, node.ProviderId, StringComparison.Ordinal)
                && string.Equals(option.ModelId, node.ModelId, StringComparison.Ordinal));
        }
        set
        {
            if (value is null || SelectedNode is not { ShowPromptEditor: true } node)
            {
                return;
            }

            node.ProviderId = value.IsInherited ? string.Empty : value.ProviderId;
            node.ModelId = value.IsInherited ? string.Empty : value.ModelId;
            OnPropertyChanged();
        }
    }

    public SummarizerChapterOption? SelectedSummarizerChapterOption
    {
        get
        {
            if (SelectedNode is not { IsSummarizerNode: true } node)
            {
                return null;
            }

            return SummarizerChapterOptions.FirstOrDefault(option =>
                string.Equals(option.ChapterId, node.SummarizerChapterId, StringComparison.Ordinal)
                && string.Equals(option.DocumentId, node.SummarizerChapterDocumentId, StringComparison.Ordinal));
        }
        set
        {
            if (value is null || SelectedNode is not { IsSummarizerNode: true } node)
            {
                return;
            }

            node.SummarizerChapterId = value.ChapterId;
            node.SummarizerChapterDocumentId = value.DocumentId;
            OnPropertyChanged();
        }
    }
    public bool IsSelectedStartNode => SelectedNode?.IsStartNode == true;
    public bool HasStartNodes => StartNodes.Count > 0;
    public bool HasNodes => Nodes.Count > 0;

    public ConfirmationItemViewModel? SelectedConfirmation
    {
        get => _selectedConfirmation;
        private set
        {
            if (SetProperty(ref _selectedConfirmation, value))
            {
                // 换审阅对象必须解除武装：否则上一项留下的「确认拒绝」态会让
                // 下一项的第一次点击直接拒绝——那是这个两步设计要防的事。
                DisarmReject();
                RebuildConfirmationDiffLines();
                OnPropertyChanged(nameof(HasSelectedConfirmation));
                OnPropertyChanged(nameof(HasPendingConfirmations));
                NotifyConfirmationCommandStates();
            }
        }
    }

    /// <summary>
    /// 把当前确认项的 diff 文本翻成分行视图（U139②）。
    ///
    /// 只做「前缀 → 类别」的翻译，不重新实现 diff 算法：算法在后端一处，
    /// 前端两个消费点（作品页快速编辑、这里）共用同一份产出。
    /// </summary>
    private void RebuildConfirmationDiffLines()
    {
        ConfirmationDiffLines.Clear();
        var diff = SelectedConfirmation?.Diff;
        if (!string.IsNullOrEmpty(diff))
        {
            // 按字符感知的行切分：正文是中文，按字节找 '\n' 会切在多字节字符中间。
            // 末尾空行不渲染（diff 常以换行结尾），否则底部凭空多出一条空色带。
            foreach (var line in diff.Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n'))
            {
                ConfirmationDiffLines.Add(new QuickEditDiffLineViewModel(line));
            }
            while (ConfirmationDiffLines.Count > 0
                && ConfirmationDiffLines[^1].Text.Length == 0
                && ConfirmationDiffLines[^1].Kind == QuickEditDiffLineKind.Context)
            {
                ConfirmationDiffLines.RemoveAt(ConfirmationDiffLines.Count - 1);
            }
        }
        OnPropertyChanged(nameof(HasConfirmationDiff));
        AskAiAboutConfirmationCommand.NotifyCanExecuteChanged();
    }

    public bool HasSelectedConfirmation => SelectedConfirmation is not null;
    /// <summary>
    /// 「有事等你拍板」——只数 <see cref="Confirmations"/>（U187-A）。
    ///
    /// ⚠️ 不要把 <see cref="ResolvedConfirmations"/> 加进来。这个属性驱动
    /// 审阅面板强制展开、面板/横幅二选一以及右栏自动切页；含进历史项后它恒为真，
    /// 结果是审阅面板永久盖住画布。历史只在面板内的「审批历史」折叠区里出现。
    /// </summary>
    public bool HasPendingConfirmations => Confirmations.Count > 0;

    /// <summary>有无审批历史可查；无历史时整个折叠区不渲染，不留空盒子。</summary>
    public bool HasResolvedConfirmations => ResolvedConfirmations.Count > 0;

    /// <summary>审批历史折叠区的开合。默认收起：进入审阅态第一眼要看的是待审那一项。</summary>
    public bool IsConfirmationHistoryExpanded
    {
        get => _isConfirmationHistoryExpanded;
        set
        {
            if (SetProperty(ref _isConfirmationHistoryExpanded, value))
            {
                // 按钮文案随状态换词，必须跟着通知：否则点了「展开」列表出来了、
                // 按钮还写着「展开」，用户不知道再点一次是收起（U186 同类形态）。
                OnPropertyChanged(nameof(ConfirmationHistoryToggleText));
            }
        }
    }

    public RelayCommand ToggleConfirmationHistoryCommand { get; }

    /// <summary>无待审项时打开审批历史（画布工具条入口，U187-A）。</summary>
    public RelayCommand ShowConfirmationHistoryCommand { get; }

    /// <summary>
    /// 钉开/解除钉开审阅面板以查看历史。
    ///
    /// 单独抽出来是因为它必须**同时**通知面板显隐——`_isConfirmationHistoryPinnedOpen`
    /// 是 `ShowConfirmationFullPanel` 的输入之一，光改字段界面不会动
    /// （Avalonia 缺通知与缺绑定同样静默）。
    /// </summary>
    private void SetConfirmationHistoryPinned(bool pinned)
    {
        if (_isConfirmationHistoryPinnedOpen == pinned)
        {
            return;
        }

        _isConfirmationHistoryPinnedOpen = pinned;
        NotifyConfirmationPanelVisibility();
    }
    public bool HasInDoubtOperations => InDoubtOperations.Count > 0;
    public WorkflowOperation? SelectedInDoubtOperation
    {
        get => _selectedInDoubtOperation;
        set
        {
            if (SetProperty(ref _selectedInDoubtOperation, value))
            {
                OnPropertyChanged(nameof(SelectedInDoubtOperationSummary));
                RetryInDoubtOperationCommand.NotifyCanExecuteChanged();
                UseInDoubtResponseCommand.NotifyCanExecuteChanged();
                StopInDoubtOperationCommand.NotifyCanExecuteChanged();
            }
        }
    }
    public string SelectedInDoubtOperationSummary => SelectedInDoubtOperation is null
        ? string.Empty
        : _displayNames.Format("ui.workspace.in_doubt.operation", new Dictionary<string, string>
        {
            ["operation"] = SelectedInDoubtOperation.OperationId,
            ["node"] = SelectedInDoubtOperation.NodeId,
        });
    public bool IsConfirmationPanelExpanded
    {
        get => _isConfirmationPanelExpanded;
        set
        {
            if (SetProperty(ref _isConfirmationPanelExpanded, value))
            {
                NotifyConfirmationPanelVisibility();
            }
        }
    }
    /// <summary>
    /// 审阅面板是否展开。两个来源：**有待审项**（原语义，一个字没动），
    /// 或作者主动点了「审批历史」把它钉开（U187-A）。
    ///
    /// ⚠️ 这里刻意**不写** `HasResolvedConfirmations`：历史一旦存在就永远存在，
    /// 把它并进条件等于让审阅面板从此永久盖住画布——那正是这轮修复要避免的事。
    /// 钉开只能由 <see cref="ShowConfirmationHistoryCommand"/> 显式发起，
    /// 由「收起看画布」解除。
    /// </summary>
    public bool ShowConfirmationFullPanel =>
        (HasPendingConfirmations && IsConfirmationPanelExpanded)
        || (_isConfirmationHistoryPinnedOpen && HasResolvedConfirmations);

    /// <summary>
    /// 收起态的顶栏横幅。判据取 `!ShowConfirmationFullPanel` 而非 `!IsConfirmationPanelExpanded`：
    /// 历史被钉开时面板已占满画布，此时再叠一条横幅就是两层东西讲同一件事。
    /// </summary>
    public bool ShowConfirmationBanner => HasPendingConfirmations && !ShowConfirmationFullPanel;

    /// <summary>
    /// 审阅面板显隐的统一出口：通知两个可见性属性，并在**进入**审阅态时把右栏切到项目 AI（U139⑤）。
    ///
    /// 为什么自动切：审阅面板此刻替换了整个画布，节点检查器（它讲的是画布上选中的节点）
    /// 在这一刻无事可讲；用户真正需要的是问 AI「这段改得对不对」。
    /// 右栏**不隐藏**——它与面板分列外层 Grid 的两列，正好放对话。
    ///
    /// 只在「进入」时切、且只切一次（`_confirmationReviewSwitchedRightPanel` 作哨兵）：
    /// 否则用户在审阅中手动切回节点检查器会被下一次刷新反复拽回来，等于剥夺了切换能力（U133 的教训）。
    /// </summary>
    private void NotifyConfirmationPanelVisibility()
    {
        OnPropertyChanged(nameof(ShowConfirmationFullPanel));
        OnPropertyChanged(nameof(ShowConfirmationBanner));
        if (ShowConfirmationFullPanel)
        {
            if (!_confirmationReviewSwitchedRightPanel)
            {
                _confirmationReviewSwitchedRightPanel = true;
                SetRightPanelTab(WorkspaceRightPanelTab.ProjectAi);
                IsRightPanelOpen = true;
            }
        }
        else
        {
            // 离开审阅态后哨兵复位，下次再进来仍会自动切一次。
            _confirmationReviewSwitchedRightPanel = false;
        }
    }

    public WorkflowEdgeViewModel? SelectedEdge
    {
        get => _selectedEdge;
        private set
        {
            if (SetProperty(ref _selectedEdge, value))
            {
                OnPropertyChanged(nameof(HasSelectedEdge));
                OnPropertyChanged(nameof(ShowEdgeConfigPanel));
                SaveEdgeConfigCommand.NotifyCanExecuteChanged();
                InsertForwardTemplateVariableCommand.NotifyCanExecuteChanged();
                InsertReverseTemplateVariableCommand.NotifyCanExecuteChanged();
                DeleteSelectedNodeCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public bool HasSelectedEdge => SelectedEdge is not null;

    /// <summary>
    /// 边配置面板：仅当选中边，且该边挂在当前选中节点上（点击相关节点/相关边时）。
    /// </summary>
    public bool ShowEdgeConfigPanel =>
        SelectedEdge is not null
        && HasSelectedNode
        && RelatedEdges.Any(e => ReferenceEquals(e, SelectedEdge));
    private bool HasCurrentRun() => !string.IsNullOrWhiteSpace(CurrentRunId);

    /// <summary>W8 产品路径：可测的 can-execute 矩阵。</summary>
    public bool CanPauseWorkflow() =>
        HasCurrentRun() && CanvasRunControlHelpers.CanPause(CurrentRunStatus);

    public bool CanResumeWorkflow() =>
        HasCurrentRun() && CanvasRunControlHelpers.CanResume(CurrentRunStatus);

    public bool CanStopWorkflow() =>
        HasCurrentRun() && CanvasRunControlHelpers.CanStop(CurrentRunStatus);

    /// <summary>
    /// U207-C①：运行控制三键（暂停/继续/停止）此刻是否值得出现在界面上。
    ///
    /// 判据刻意取「三者**至少一个**真能做事」，而不是「有 run id」：
    /// - 空闲（从未跑过）：三者全 false ⇒ 整组不渲染，同一位置让给「运行」；
    /// - 终态（succeeded/failed/stopped）：run id 还在，但三者仍全 false
    ///   ⇒ 也算空闲，换回「运行」（跑完一轮后下一件事是再跑一轮，
    ///     而不是停止一个已经停了的运行）。
    ///
    /// 这样写的另一个好处是自维护：以后 <see cref="CanvasRunControlHelpers"/>
    /// 的生命周期矩阵怎么改，显隐都跟着对，不会漏掉某个状态。
    /// </summary>
    public bool ShowRunControls =>
        CanPauseWorkflow() || CanResumeWorkflow() || CanStopWorkflow();

    /// <summary>
    /// 「运行」入口与运行控制三键**互斥**：同一块地方在同一时刻只承担一件事。
    ///
    /// 缺陷原形是两者都不在（空画布 + 默认「节点库」tab ⇒ 零个开始入口）
    /// 却常驻三个点不动的控制键，界面上最醒目的反而是此刻最无意义的「停止」。
    /// </summary>
    public bool ShowRunEntry => !ShowRunControls;

    /// <summary>
    /// U196-D：失败的那个节点 id。取自后端 <c>WorkflowRunFailure.Stage</c>
    /// —— 字段叫 stage，但 <c>runtime.rs::record_node_error</c> 往里放的是节点 id
    /// （注释写明「用户在画布上按它定位到具体哪个方块」）。
    ///
    /// 空串表示「不知道是哪个节点失败的」，此时入口不出现：给一颗不知道要重跑什么的
    /// 「从失败处重跑」，比没有这颗更糟。
    /// </summary>
    public string FailedNodeId
    {
        get => _failedNodeId;
        private set
        {
            if (SetProperty(ref _failedNodeId, value))
            {
                OnPropertyChanged(nameof(CanRetryFromFailedNode));
                RetryFailedNodeCommand.NotifyCanExecuteChanged();
            }
        }
    }

    /// <summary>
    /// U196-D：「从失败处重跑」此刻能不能点。
    ///
    /// 三个条件都是必需的，缺一个就会造出一颗有害的按钮：
    /// - 有 run id：没有运行可谈；
    /// - 状态是 failed：其他状态下重跑要么无意义（succeeded）要么会与在跑的 worker 抢；
    /// - 知道是哪个节点失败：见 <see cref="FailedNodeId"/>。
    ///
    /// ⚠️ 刻意**不**复用 <see cref="CanvasRunControlHelpers"/>：那套矩阵的三个判定
    /// 全部把 failed 排除在外（`IsTerminal` 里就有 failed），这颗按钮恰恰只在
    /// failed 下有用。往那套矩阵里塞一个「终态也算可用」的例外，会让暂停/停止
    /// 在终态下一起亮起来。
    /// </summary>
    public bool CanRetryFailedNode() =>
        HasCurrentRun()
        && string.Equals(CurrentRunStatus, "failed", StringComparison.Ordinal)
        && !string.IsNullOrWhiteSpace(FailedNodeId);

    /// <summary>渲染位的显隐；与 <see cref="CanRetryFailedNode"/> 同源，不另设条件。</summary>
    public bool CanRetryFromFailedNode => CanRetryFailedNode();

    public string RetryFailedNodeText => _displayNames.Text("ui.workspace.retry_failed_node");

    /// <summary>
    /// 悬停说明。这颗按钮的字面意思（「从失败处重跑」）说不清最要紧的那件事
    /// ——**前面已完成的步骤不会重复花钱**。作者失败一次之后最怕的正是这个，
    /// 不写明的话他会宁愿什么都不点。
    /// </summary>
    public string RetryFailedNodeTooltip =>
        _displayNames.Text("ui.workspace.retry_failed_node_tooltip");

    /// <summary>
    /// 「运行」按钮能不能点。
    ///
    /// 没有起始节点时禁用——但**禁用理由必须配文字**（与 <see cref="RunNodeAsync"/>
    /// 里对 required 变量的处置同一条规矩），那句话由
    /// <see cref="RunEntryTooltip"/> 承担，且挂在按钮外层的容器上
    /// （Avalonia 禁用控件不参与命中测试，挂在按钮自己身上永远看不到）。
    /// </summary>
    public bool CanRunWorkflowFromEntry() => HasStartNodes && CanPersistWorkflow();

    /// <summary>
    /// 悬停说明：能跑时说「从起始节点运行」，不能跑时说**差什么**。
    ///
    /// 三档顺序按「先决条件的依赖次序」排：没项目 → 画布没加载好 → 没起始节点。
    /// 反过来排会在没打开项目时对着作者念「先拖一个开始节点」，那是句废话。
    /// </summary>
    public string RunEntryTooltip
    {
        get
        {
            if (!_backend.HasProjectRoot)
            {
                return _displayNames.Text("ui.empty.need_project.hint");
            }

            if (!CanPersistWorkflow())
            {
                return _displayNames.Text("ui.workspace.load_required_before_save");
            }

            if (!HasStartNodes)
            {
                return _displayNames.Text("ui.workspace.run.needs_start_node");
            }

            return RunFromStartText;
        }
    }

    /// <summary>W8：最近一次已知 run 生命周期状态（running/paused/…）。</summary>
    public string CurrentRunStatus => _runSession.Status;
    private bool HasProjectAiMessage() => !string.IsNullOrWhiteSpace(ProjectAiMessage);
    private bool CanResolveConfirmation()
    {
        if (SelectedConfirmation is null)
        {
            return false;
        }

        if (!string.IsNullOrWhiteSpace(SelectedConfirmation.RunId))
        {
            return true;
        }

        return (string.IsNullOrWhiteSpace(SelectedConfirmation.WorkflowId)
                || string.Equals(SelectedConfirmation.WorkflowId, CurrentWorkflowId, StringComparison.Ordinal))
            && !string.IsNullOrWhiteSpace(CurrentRunId);
    }
    private bool HasSelectedInDoubtOperation() => SelectedInDoubtOperation is not null;

    /// <summary>
    /// 与 <see cref="TryConnectPorts"/> 相同的类型/方向规则；高亮与落点共用此判定，避免双套矩阵。
    /// </summary>
    public bool CanConnectPorts(
        string sourceNodeId, NodePortKind sourceKind, NodePortDirection sourceDirection,
        string targetNodeId, NodePortKind targetKind, NodePortDirection targetDirection)
    {
        return TryEvaluateConnection(
            sourceNodeId, sourceKind, sourceDirection,
            targetNodeId, targetKind, targetDirection,
            out _, out _, out _, out _, out _, out _);
    }

    /// <summary>拖线开始：按同源规则点亮可落端口。</summary>
    public void BeginPortDragHighlight(
        string sourceNodeId, NodePortKind sourceKind, NodePortDirection sourceDirection)
    {
        var sourceHandle = NodePortSpec.HandleName(sourceKind, sourceDirection);
        foreach (var node in Nodes)
        {
            // U181-E：**起点端口自己不许被淡出。**
            //
            // 原缺陷：这个循环遍历全部 Nodes（含起点节点自己），而
            // TryEvaluateConnection 对同节点必然判 Self 失败 ⇒ 起点节点的
            // **所有**端口都走 SetPortDragHighlight 的 false 分支、Opacity 打到 0.22。
            // 于是作者选完连线起点，那个起点反而比别的端口更淡 —— 键盘连线路径
            // 因此是零起点指示（鼠标路径靠橡皮筋兜底，键盘路径连橡皮筋都没有）。
            //
            // 起点节点的其余端口仍然淡出：它们确实不可连（同节点），
            // 保留淡出才让「哪些能落」这件事继续成立。只把**被选中的那一个**
            // 拎出来给满不透明 + 一个明确的「已选为起点」态。
            var isSourceNode = string.Equals(node.Id, sourceNodeId, StringComparison.Ordinal);
            node.SetPortDragHighlight(
                controlIn: CanConnectPorts(sourceNodeId, sourceKind, sourceDirection, node.Id, NodePortKind.Control, NodePortDirection.In),
                controlOut: CanConnectPorts(sourceNodeId, sourceKind, sourceDirection, node.Id, NodePortKind.Control, NodePortDirection.Out),
                dataIn: CanConnectPorts(sourceNodeId, sourceKind, sourceDirection, node.Id, NodePortKind.Data, NodePortDirection.In),
                dataOut: CanConnectPorts(sourceNodeId, sourceKind, sourceDirection, node.Id, NodePortKind.Data, NodePortDirection.Out),
                communication: CanConnectPorts(sourceNodeId, sourceKind, sourceDirection, node.Id, NodePortKind.Communication, NodePortDirection.Both),
                originHandle: isSourceNode ? sourceHandle : null);
        }
    }

    /// <summary>拖线结束：恢复端口默认外观。</summary>
    public void EndPortDragHighlight()
    {
        foreach (var node in Nodes)
        {
            node.ClearPortDragHighlight();
        }
    }

    /// <summary>
    /// 任意端口拖线：同类可连，异类拒绝。方向可从出到入，也可从入到出（自动纠正）。
    /// </summary>
    public bool TryConnectPorts(string sourceNodeId, NodePortKind sourceKind, NodePortDirection sourceDirection,
        string targetNodeId, NodePortKind targetKind, NodePortDirection targetDirection,
        string? sourceHandle = null, string? targetHandle = null)
    {
        if (!TryEvaluateConnection(
                sourceNodeId, sourceKind, sourceDirection,
                targetNodeId, targetKind, targetDirection,
                out var fromNodeId, out var toNodeId, out var fromHandle, out var toHandle, out var edgeKind,
                out var rejectReason,
                sourceHandle, targetHandle))
        {
            StatusText = rejectReason switch
            {
                ConnectRejectReason.Self => _displayNames.Text("ui.workspace.edge.connect_rejected_self"),
                ConnectRejectReason.Type => _displayNames.Format("ui.workspace.edge.connect_rejected_type", new Dictionary<string, string>
                {
                    ["source"] = PortKindLabel(sourceKind),
                    ["target"] = PortKindLabel(targetKind),
                }),
                ConnectRejectReason.Direction => _displayNames.Text("ui.workspace.edge.connect_rejected_direction"),
                ConnectRejectReason.Duplicate => _displayNames.Text("ui.workspace.edge.connect_rejected_duplicate"),
                ConnectRejectReason.Occupied => _displayNames.Text("ui.workspace.edge.connect_rejected_occupied"),
                _ => _displayNames.Text("ui.workspace.edge.connect_rejected_miss"),
            };
            return false;
        }

        CaptureUndoSnapshot();
        object? edgeData = edgeKind == "communication"
            ? DefaultCommunicationData()
            : new Dictionary<string, object?>();
        var aliasOrLabel = edgeKind == "data"
            ? NextDataAlias(toNodeId, toHandle)
            : null;
        var edge = new CanvasEdge(
            $"edge-{Guid.NewGuid():N}",
            fromNodeId,
            toNodeId,
            fromHandle,
            toHandle,
            edgeKind,
            aliasOrLabel,
            edgeData);
        var viewModel = new WorkflowEdgeViewModel(edge, _displayNames, SelectEdge, RefreshDirtyState);
        Edges.Add(viewModel);
        RefreshRelatedEdges();
        RefreshEdgeLabels();
        RefreshPortConnectionStates();
        _edges = Edges.Select(item => item.ToCanvasEdge()).ToArray();
        SelectEdge(viewModel);
        RefreshDirtyState();
        OnPropertyChanged(nameof(EdgeCountText));
        StatusText = _displayNames.Format("ui.workspace.edge.connect_created", new Dictionary<string, string>
        {
            ["kind"] = PortKindLabel(sourceKind),
        });
        return true;
    }

    private enum ConnectRejectReason
    {
        None,
        Self,
        Type,
        Direction,
        Duplicate,
        Occupied,
    }

    private bool TryEvaluateConnection(
        string sourceNodeId, NodePortKind sourceKind, NodePortDirection sourceDirection,
        string targetNodeId, NodePortKind targetKind, NodePortDirection targetDirection,
        out string fromNodeId, out string toNodeId, out string fromHandle, out string toHandle, out string edgeKind,
        out ConnectRejectReason rejectReason,
        string? sourceHandle = null,
        string? targetHandle = null)
    {
        fromNodeId = string.Empty;
        toNodeId = string.Empty;
        fromHandle = string.Empty;
        toHandle = string.Empty;
        edgeKind = string.Empty;
        rejectReason = ConnectRejectReason.None;

        if (string.Equals(sourceNodeId, targetNodeId, StringComparison.Ordinal))
        {
            rejectReason = ConnectRejectReason.Self;
            return false;
        }

        if (sourceKind != targetKind)
        {
            rejectReason = ConnectRejectReason.Type;
            return false;
        }

        if (!NodePortSpec.TryNormalizeConnection(
                sourceNodeId, sourceKind, sourceDirection,
                targetNodeId, targetKind, targetDirection,
                out fromNodeId, out toNodeId, out fromHandle, out toHandle, out edgeKind))
        {
            rejectReason = ConnectRejectReason.Direction;
            return false;
        }

        // 指定多数据入 handle 时覆盖默认 input
        if (!string.IsNullOrWhiteSpace(sourceHandle) || !string.IsNullOrWhiteSpace(targetHandle))
        {
            var aIsOut = sourceDirection is NodePortDirection.Out or NodePortDirection.Both;
            if (string.Equals(fromNodeId, sourceNodeId, StringComparison.Ordinal))
            {
                if (!string.IsNullOrWhiteSpace(sourceHandle) && aIsOut)
                {
                    fromHandle = sourceHandle!;
                }

                if (!string.IsNullOrWhiteSpace(targetHandle) && !aIsOut)
                {
                    // source was In → from is other node
                }

                if (!string.IsNullOrWhiteSpace(targetHandle)
                    && string.Equals(toNodeId, targetNodeId, StringComparison.Ordinal))
                {
                    toHandle = targetHandle!;
                }

                if (!string.IsNullOrWhiteSpace(sourceHandle)
                    && string.Equals(toNodeId, sourceNodeId, StringComparison.Ordinal))
                {
                    toHandle = sourceHandle!;
                }

                if (!string.IsNullOrWhiteSpace(targetHandle)
                    && string.Equals(fromNodeId, targetNodeId, StringComparison.Ordinal))
                {
                    fromHandle = targetHandle!;
                }
            }
            else
            {
                // 拖线从 B 起、A 为 from
                if (!string.IsNullOrWhiteSpace(targetHandle)
                    && string.Equals(fromNodeId, targetNodeId, StringComparison.Ordinal))
                {
                    fromHandle = targetHandle!;
                }

                if (!string.IsNullOrWhiteSpace(sourceHandle)
                    && string.Equals(toNodeId, sourceNodeId, StringComparison.Ordinal))
                {
                    toHandle = sourceHandle!;
                }

                if (!string.IsNullOrWhiteSpace(sourceHandle)
                    && string.Equals(fromNodeId, sourceNodeId, StringComparison.Ordinal))
                {
                    fromHandle = sourceHandle!;
                }

                if (!string.IsNullOrWhiteSpace(targetHandle)
                    && string.Equals(toNodeId, targetNodeId, StringComparison.Ordinal))
                {
                    toHandle = targetHandle!;
                }
            }
        }

        // 拷贝到局部，避免 lambda 捕获 out 参数（CS1628）。
        var normalizedFrom = fromNodeId;
        var normalizedTo = toNodeId;
        var normalizedKind = edgeKind;
        var normalizedToHandle = toHandle;
        if (Edges.Any(edge =>
                string.Equals(edge.Kind, normalizedKind, StringComparison.OrdinalIgnoreCase)
                && ((edge.Source == normalizedFrom && edge.Target == normalizedTo)
                    || (normalizedKind == "communication"
                        && edge.Source == normalizedTo
                        && edge.Target == normalizedFrom))))
        {
            rejectReason = ConnectRejectReason.Duplicate;
            return false;
        }

        // 一数据入只能一根线
        if (string.Equals(normalizedKind, "data", StringComparison.OrdinalIgnoreCase)
            && CanvasSelectionHelpers.IsDataInOccupied(
                Edges.Select(edge => (edge.Kind, edge.Target, edge.TargetHandle)),
                normalizedTo,
                normalizedToHandle))
        {
            rejectReason = ConnectRejectReason.Occupied;
            return false;
        }

        return true;
    }

    private void RefreshEdgeLabels()
    {
        var names = Nodes.ToDictionary(
            node => node.Id,
            node => string.IsNullOrWhiteSpace(node.Name) ? node.Label : node.Name,
            StringComparer.Ordinal);
        foreach (var edge in Edges)
        {
            names.TryGetValue(edge.Source, out var sourceName);
            names.TryGetValue(edge.Target, out var targetName);
            edge.SetEndpointLabels(sourceName ?? edge.Source, targetName ?? edge.Target);
        }
    }

    /// <summary>兼容旧调用：默认按数据口出→入连接。</summary>
    public void CreateDataEdge(string sourceNodeId, string targetNodeId)
    {
        TryConnectPorts(
            sourceNodeId, NodePortKind.Data, NodePortDirection.Out,
            targetNodeId, NodePortKind.Data, NodePortDirection.In);
    }

    public void NotifyConnectMissed()
    {
        StatusText = _displayNames.Text("ui.workspace.edge.connect_rejected_miss");
    }

    public void NotifyKeyboardConnectStarted()
    {
        StatusText = _displayNames.Text("ui.workspace.keyboard_connect_started");
    }

    public void NotifyKeyboardConnectCancelled()
    {
        StatusText = _displayNames.Text("ui.workspace.keyboard_connect_cancelled");
    }

    private string NextDataAlias(string targetNodeId, string targetHandle)
    {
        var used = Edges
            .Where(edge => edge.Target == targetNodeId
                           && string.Equals(edge.Kind, "data", StringComparison.OrdinalIgnoreCase))
            .Select(edge => string.IsNullOrWhiteSpace(edge.Label) ? edge.TargetHandle : edge.Label)
            .Where(alias => !string.IsNullOrWhiteSpace(alias))
            .ToHashSet(StringComparer.Ordinal);
        var targetNode = Nodes.FirstOrDefault(node => node.Id == targetNodeId);
        var summarizerAlias = targetNode?.IsSummarizerNode == true
            ? targetNode.SummarizerChapterTextAlias.Trim()
            : string.Empty;
        var utilityAlias = targetNode?.IsSearchNode == true
            ? targetNode.QueryAlias.Trim()
            : targetNode?.IsLoopNode == true
                ? targetNode.StopInputAlias.Trim()
                : string.Empty;
        var aliasBase = !string.IsNullOrWhiteSpace(summarizerAlias)
            ? summarizerAlias
            : !string.IsNullOrWhiteSpace(utilityAlias)
                ? utilityAlias
            : string.IsNullOrWhiteSpace(targetHandle) ? "input" : targetHandle.Trim();
        if (!used.Contains(aliasBase))
        {
            return aliasBase;
        }
        for (var i = 2; i < 1000; i++)
        {
            var candidate = $"{aliasBase}_{i}";
            if (!used.Contains(candidate))
            {
                return candidate;
            }
        }
        return $"{aliasBase}_{Guid.NewGuid():N}"[..16];
    }

    /// <summary>
    /// Summarizer 的首个数据入就是章节正文端口；作者改 alias 时同步正式数据边，
    /// 防止节点配置与边标签各自保存成两套契约。
    /// </summary>
    private void OnSummarizerChapterTextAliasChanged(
        WorkflowNodeViewModel node,
        string previousAlias,
        string currentAlias)
    {
        if (!node.IsSummarizerNode || string.IsNullOrWhiteSpace(currentAlias))
        {
            return;
        }

        var primaryHandle = node.DataInPins.FirstOrDefault()?.Handle;
        var edge = Edges.FirstOrDefault(candidate =>
            candidate.IsData
            && candidate.Target == node.Id
            && (string.IsNullOrWhiteSpace(primaryHandle)
                || string.Equals(candidate.TargetHandle, primaryHandle, StringComparison.OrdinalIgnoreCase)));
        if (edge is null)
        {
            return;
        }

        var prior = previousAlias.Trim();
        if (!string.IsNullOrWhiteSpace(edge.Label)
            && !string.IsNullOrWhiteSpace(prior)
            && !string.Equals(edge.Label.Trim(), prior, StringComparison.Ordinal))
        {
            return;
        }

        edge.Label = currentAlias.Trim();
        _edges = Edges.Select(item => item.ToCanvasEdge()).ToArray();
        RefreshDirtyState();
    }

    private Dictionary<string, object?> DefaultCommunicationData()
    {
        return new Dictionary<string, object?>
        {
            ["forward_alias"] = "forward_output",
            ["reverse_alias"] = "reverse_output",
            ["forward_template"] = _displayNames.Text("ui.workspace.edge.default_forward_template"),
            ["reverse_template"] = _displayNames.Text("ui.workspace.edge.default_reverse_template"),
            ["max_communication_count"] = 2u,
        };
    }

    private string PortKindLabel(NodePortKind kind) => kind switch
    {
        NodePortKind.Control => _displayNames.Text("ui.workspace.edge.kind.control"),
        NodePortKind.Communication => _displayNames.Text("ui.workspace.edge.kind.communication"),
        _ => _displayNames.Text("ui.workspace.edge.kind.data"),
    };

    public void AddNodeAt(string nodeType, double x, double y)
    {
        AddNode(nodeType, x, y);
    }

    public string CtxAddNodeText => _displayNames.Text("ui.workspace.context.add_node");
    public string CtxAddStartText => _displayNames.Text("ui.workspace.context.add_start");
    public string CtxPasteText => _displayNames.Text("ui.workspace.context.paste");
    public string CtxSelectAllText => _displayNames.Text("ui.workspace.context.select_all");
    public string CtxFitViewText => _displayNames.Text("ui.workspace.context.fit_view");
    public string CtxCopyText => _displayNames.Text("ui.workspace.context.copy");
    public string CtxCutText => _displayNames.Text("ui.workspace.context.cut");
    public string CtxDeleteText => _displayNames.Text("ui.workspace.context.delete");

    private void NotifyNodeCommandStates()
    {
        DeleteSelectedNodeCommand.NotifyCanExecuteChanged();
        RunSelectedNodeCommand.NotifyCanExecuteChanged();
        ApplyNodeConfigCommand.NotifyCanExecuteChanged();
        ToggleBreakpointCommand.NotifyCanExecuteChanged();
        BrowseWorkDirCommand.NotifyCanExecuteChanged();
        ExportSelectionCommand.NotifyCanExecuteChanged();
        CopySelectedNodeCommand.NotifyCanExecuteChanged();
        CutSelectedNodeCommand.NotifyCanExecuteChanged();
        // 选中节点决定 Ctrl+K 填哪一组变量；选中从无变量节点切到有变量节点时
        // 可用性也随之变化。
        OpenVariableFillCommand.NotifyCanExecuteChanged();
    }

    private void NotifyRunCommandStates()
    {
        PauseWorkflowCommand.NotifyCanExecuteChanged();
        StopWorkflowCommand.NotifyCanExecuteChanged();
        ResumeWorkflowCommand.NotifyCanExecuteChanged();
        // U207-C①：三键的显隐是这三个 can-execute 的函数，所以必须在**同一处**广播。
        // 漏了这两行的话，运行起来后「运行」按钮不会让位，三键也不会出现。
        OnPropertyChanged(nameof(ShowRunControls));
        OnPropertyChanged(nameof(ShowRunEntry));
        // U196-D：「从失败处重跑」的可用性是 CurrentRunStatus 的函数，必须在同一处广播。
        // 漏了这两行，值算对了但界面停在上一屏 —— 与「根本没接这条判定」在屏幕上同形。
        RetryFailedNodeCommand.NotifyCanExecuteChanged();
        OnPropertyChanged(nameof(CanRetryFromFailedNode));
        NotifyConfirmationCommandStates();
    }

    private void OnRunSessionStateChanged(
        WorkspaceRunSessionState previous,
        WorkspaceRunSessionState current)
    {
        var identityChanged = !string.Equals(
                previous.WorkflowId,
                current.WorkflowId,
                StringComparison.Ordinal)
            || !string.Equals(previous.RunId, current.RunId, StringComparison.Ordinal);
        if (!string.Equals(previous.RunId, current.RunId, StringComparison.Ordinal))
        {
            OnPropertyChanged(nameof(CurrentRunId));
            OnPropertyChanged(nameof(CurrentRunValueText));
        }
        if (identityChanged)
        {
            _ = LoadInDoubtOperationsAsync();
        }
        if (!string.Equals(previous.Status, current.Status, StringComparison.Ordinal))
        {
            OnPropertyChanged(nameof(CurrentRunStatus));
        }
        // U198-B：跨进 failed 的那一次跃迁，去后端把「为什么 + 下一步」取回来。
        // 接在**跃迁边沿**（与 RunTerminalStateNotifier 同一取舍）：轮询每 750ms
        // 一轮，接在 ApplyWorkflowEvents 里会重复发请求；而且控制指令直接 Attach
        // 结果的那条路（点停止立刻拿到终态）根本不会再有下一轮回包。
        if (string.Equals(current.Status, "failed", StringComparison.Ordinal)
            && !string.Equals(previous.Status, "failed", StringComparison.Ordinal))
        {
            _ = LoadRunFailureRecoveryAsync(current.WorkflowId, current.RunId);
        }
        NotifyRunCommandStates();
    }

    /// <summary>
    /// 运行失败后把后端的补救建议取回来显示（U198-B / U196-E）。
    ///
    /// ## 为什么要多发一次请求
    ///
    /// 事件流里只有 `status = "failed"`，页面据此显示「已失败」——**为什么失败、
    /// 下一步做什么，一个字都没有**。建议在 `WorkflowRunFailure.recovery_suggestion`
    /// 里（后端 `workflow/runtime.rs` 按错误类别产出成文中文），只有
    /// `get_workflow_run_state` 带得回来。运行失败一次只发一次，成本可以忽略。
    ///
    /// ## 为什么不解析事件的 metadata
    ///
    /// `run_failed` 事件的 metadata 里确实也有 `recovery_suggestion`，但前端
    /// `WorkflowRuntimeEvent.Metadata` 是 `object?`（未定型 JSON）。
    /// 走已定型的 `WorkflowRunFailure` 比在 UI 层手写 JSON 取值可靠得多，
    /// 而且后端改字段时编译期就能发现。
    /// </summary>
    private async Task LoadRunFailureRecoveryAsync(string workflowId, string runId)
    {
        if (string.IsNullOrWhiteSpace(workflowId) || string.IsNullOrWhiteSpace(runId))
        {
            return;
        }
        try
        {
            var state = await _backend
                .GetWorkflowRunStateAsync(workflowId, runId)
                .ConfigureAwait(true);
            // 会话可能已经换了（作者切了工作流 / 又起了一次跑）：迟到的建议不得
            // 覆盖新会话的页面状态，否则作者看到的是上一次失败的「下一步」。
            if (!string.Equals(workflowId, CurrentWorkflowId, StringComparison.Ordinal)
                || !string.Equals(runId, CurrentRunId, StringComparison.Ordinal))
            {
                return;
            }
            SetRecoverySuggestion(state.Failure?.RecoverySuggestion, _displayNames);
            // U196-D：同一次回包里把失败节点 id 也收下 —— 「从失败处重跑」需要它。
            // 与建议同源不是省请求：两者必须来自**同一个** run state 快照，
            // 否则会出现「建议说的是 A 节点、重跑打到 B 节点」。
            FailedNodeId = state.Failure?.Stage ?? string.Empty;
        }
        catch (Exception)
        {
            // 取建议失败不改 StatusText：那里现在写着「已失败」，是这次运行的
            // 真实结论。把它换成「无法连接服务」等于用取建议这条附带请求的失败，
            // 盖掉作者真正要看的那条。静默降级为「没有建议」是正确的取舍。
        }
    }

    private void OnRunSessionPollingFailed(Exception error)
    {
        StatusText = ReportFailure(error, _displayNames);
    }

    private void NotifyConfirmationCommandStates()
    {
        ApproveConfirmationCommand.NotifyCanExecuteChanged();
        RejectConfirmationCommand.NotifyCanExecuteChanged();
        ApproveOrCancelCommand.NotifyCanExecuteChanged();
        AskAiAboutConfirmationCommand.NotifyCanExecuteChanged();
        // 历史入口的可用性随「有没有历史」变，也在这里一并刷新：
        // 它和上面四个一样只在确认项重载后才可能改变。
        ShowConfirmationHistoryCommand.NotifyCanExecuteChanged();
    }

    /// <summary>
    /// 收起理由输入线，按钮回到「拒绝」。
    ///
    /// 不清空已写的理由：作者写了一段又收起来，再展开时内容还在。
    /// 理由本身对通过路径也有效（ConfirmationReason 是共用字段）。
    /// </summary>
    private void DisarmReject()
    {
        IsRejectArmed = false;
    }

    public void CaptureCanvasHistory()
    {
        CaptureUndoSnapshot();
    }

    private void CaptureUndoSnapshot()
    {
        if (_suppressSnapshotChecks)
        {
            return;
        }
        var snapshot = CurrentSnapshot();
        if (_undoSnapshots.Count == 0 || _undoSnapshots[^1] != snapshot)
        {
            _undoSnapshots.Add(snapshot);
            if (_undoSnapshots.Count > 100)
            {
                _undoSnapshots.RemoveAt(0);
            }
        }
        _redoSnapshots.Clear();
        NotifyHistoryCommands();
    }

    private void UndoCanvasChange()
    {
        if (_undoSnapshots.Count == 0)
        {
            return;
        }
        var current = CurrentSnapshot();
        var previous = _undoSnapshots[^1];
        _undoSnapshots.RemoveAt(_undoSnapshots.Count - 1);
        if (_redoSnapshots.Count == 0 || _redoSnapshots[^1] != current)
        {
            _redoSnapshots.Add(current);
        }
        RestoreGraphSnapshot(previous);
        NotifyHistoryCommands();
    }

    private void RedoCanvasChange()
    {
        if (_redoSnapshots.Count == 0)
        {
            return;
        }
        var current = CurrentSnapshot();
        var next = _redoSnapshots[^1];
        _redoSnapshots.RemoveAt(_redoSnapshots.Count - 1);
        if (_undoSnapshots.Count == 0 || _undoSnapshots[^1] != current)
        {
            _undoSnapshots.Add(current);
        }
        RestoreGraphSnapshot(next);
        NotifyHistoryCommands();
    }

    private void RestoreGraphSnapshot(string snapshot)
    {
        var graph = JsonSerializer.Deserialize<WorkflowGraphData>(snapshot, JsonOptions);
        if (graph is null)
        {
            return;
        }
        ApplyGraph(graph);
        RefreshDirtyState();
    }

    private void NotifyHistoryCommands()
    {
        UndoCommand.NotifyCanExecuteChanged();
        RedoCommand.NotifyCanExecuteChanged();
    }

    private void AddNode(string nodeType, bool capture = true)
    {
        var x = 120 + ((Nodes.Count % 4) * 230);
        var y = 80 + ((Nodes.Count / 4) * 170);
        AddNode(nodeType, x, y, capture);
    }

    private void AddNode(string nodeType, double x, double y, bool capture = true)
    {
        if (capture)
        {
            CaptureUndoSnapshot();
        }
        var label = NodeLabel(nodeType);
        var node = new WorkflowNodeViewModel(
            id: NextNodeId(nodeType),
            nodeType,
            label,
            defaultWorkDir: WorkflowNodeCatalog.Resolve(nodeType).ConfigKind == "start"
                ? _displayNames.Text("ui.workspace.start_node.default_work_dir")
                : string.Empty,
            // U141：画布是无限的，坐标系有负半轴。原先这里是 Math.Max(0, x/y)，
            // 把新节点钉在第一象限——用户往左上方平移后落点会被悄悄挪回原点侧，
            // 且这个改写会随工作流存盘。落点合法性属于内容，不由视口或象限裁定。
            x: x,
            y: y,
            runRequested: runNode => _ = RunNodeAsync(runNode),
            () => SelectNode(node: null),
            RefreshDirtyState,
            canRun: CanPersistWorkflow);
        // agent 才填提示词；通用节点用角色配置默认值
        //
        // U201-C：填的是**默认提示词占位符**（一行 `{{outliner 默认提示词}}`），
        // 不是 300~470 字的全文。全文副本存进工作流文件会让官方后续调整默认提示词
        // 对已建节点无效，而且编辑框一进节点就被占满。
        if (node.ShowPromptEditor)
        {
            node.PromptTemplate = Localization.PromptCatalog.ResolveNodePromptPlaceholder(
                nodeType,
                _displayNames.Text);
        }

        SeedUtilityDefaults(node);
        AttachNodeCommands(node);
        Nodes.Add(node);
        RequestEnsureNodeVisible?.Invoke(node);
        RefreshStartNodes();
        SelectNode(node);
        if (capture)
        {
            RefreshDirtyState();
        }
    }

    private void SeedUtilityDefaults(WorkflowNodeViewModel node)
    {
        if (node.IsSummarizerNode)
        {
            if (string.IsNullOrWhiteSpace(node.SummarizerChapterTextAlias))
            {
                node.SummarizerChapterTextAlias = "chapter_text";
            }
        }

        if (node.IsApprovalNode && string.IsNullOrWhiteSpace(node.ApprovalId))
        {
            node.ApprovalId = $"approval-{node.Id}";
        }

        if (node.IsExportNode)
        {
            if (string.IsNullOrWhiteSpace(node.ExportArtifactId))
            {
                node.ExportArtifactId = $"export-{node.Id}";
            }

            if (string.IsNullOrWhiteSpace(node.ExportFormat))
            {
                node.ExportFormat = "markdown";
            }
        }

        if (node.IsSearchNode && string.IsNullOrWhiteSpace(node.QueryAlias))
        {
            node.QueryAlias = "query";
        }

        if (node.IsConditionNode && string.IsNullOrWhiteSpace(node.ConditionInputAlias))
        {
            node.ConditionInputAlias = "input";
        }
    }

    private WorkflowNodeViewModel CreateNodeFromCanvas(CanvasNode graphNode)
    {
        var data = graphNode.Data ?? new Dictionary<string, object?>();
        var descriptor = WorkflowNodeCatalog.Resolve(graphNode.Type);
        var node = new WorkflowNodeViewModel(
            graphNode.Id,
            graphNode.Type,
            graphNode.Label ?? NodeLabel(graphNode.Type),
            ReadString(data, "work_dir"),
            graphNode.Position?.X ?? 120 + ((Nodes.Count % 4) * 230),
            graphNode.Position?.Y ?? 80 + ((Nodes.Count / 4) * 170),
            runRequested: runNode => _ = RunNodeAsync(runNode),
            () => SelectNode(node: null),
            RefreshDirtyState,
            canRun: CanPersistWorkflow)
        {
            Name = ReadString(data, "name", graphNode.Label ?? NodeLabel(graphNode.Type)),
            UserNote = ReadString(data, "user_note"),
            ExposedAsTool = ReadBool(data, "expose_as_tool", descriptor.ConfigKind == "start"),
            PromptTemplate = ReadString(data, "prompt_template"),
            ProviderId = ReadString(data, "provider_id"),
            ModelId = ReadString(data, "model_id"),
            BudgetUsd = ReadString(data, "budget_usd"),
            TimeoutMs = ReadString(
                data,
                "timeout_ms",
                descriptor.ConfigKind == "loop" ? "300000" : string.Empty),
            BreakpointEnabled = ReadBool(data, "breakpoint", false),
            ImportPath = CoalescePath(data),
            IncludeContent = ReadBool(data, "include_content", true),
            QueryAlias = ReadString(data, "query_alias", "query"),
            SearchLimit = ReadString(data, "limit", "10"),
            ConditionInputAlias = ReadString(data, "input_alias", "input"),
            ConditionOperator = ReadString(data, "operator", "truthy"),
            ConditionExpected = ReadValueAsString(data, "expected"),
            MaxIterations = ReadString(data, "max_iterations", "5"),
            ApprovalId = ReadString(data, "approval_id"),
            AutoApprove = ReadBool(data, "auto_approve", false),
            ExportArtifactId = ReadString(data, "artifact_id"),
            ExportFormat = ReadString(data, "format", "markdown"),
            ExportTitle = ReadString(data, "title"),
            SummarizerChapterId = ReadString(data, "chapter_id"),
            SummarizerChapterDocumentId = ReadString(data, "chapter_document_id"),
            SummarizerChapterTextAlias = ReadString(data, "chapter_text_alias", "chapter_text"),
            SummarizerAutoMode = ReadBool(data, "auto_mode", false),
        };
        LoadStopCondition(node, data);
        // 画布已有节点若未存提示词，用默认占位符补全（不覆盖用户已写内容）；通用/导入节点不填 agent 模板
        //
        // U201-C：这里补的是占位符而不是全文，理由同 `AddNode`。
        // ⚠️ **只在为空时补**这条不能松：非空说明作者写过东西（可能是他改的全文、
        // 也可能是存量文件里的全文副本），覆盖它等于删掉作者的稿子。
        // 存量的全文副本因此原样保留，也照样能跑——它就是一段普通 prompt_template。
        if (string.IsNullOrWhiteSpace(node.PromptTemplate) && node.ShowPromptEditor)
        {
            node.PromptTemplate = Localization.PromptCatalog.ResolveNodePromptPlaceholder(
                graphNode.Type,
                _displayNames.Text);
        }
        node.RestoreDataInPins(ReadStringList(data, "data_in_handles"));
        // 起始节点才有变量：变量的生命周期是整个 run，挂在其它节点上无从确定归属层。
        if (node.IsStartNode)
        {
            var group = new WorkflowVariableGroupViewModel(
                _displayNames.Text,
                _displayNames.Format,
                RefreshDirtyState);
            group.Load(ReadVariableDeclarations(data), ReadString(data, "summary_template"));
            group.RequestGenerateSummary = () => GenerateVariableSummaryAsync(group);
            node.Variables = group;
        }
        // 必须保留 tool_enabled / input_aliases 等非 UI 键，否则 SaveWorkflowGraph 会整表冲掉
        node.RetainOpaqueData(data);
        AttachNodeCommands(node);
        return node;
    }

    /// <summary>
    /// 让项目空间 AI 起一句摘要句式，写进句式框（作者可再改）。
    ///
    /// 复用 project_ai_chat 通道，因此**沿用当前对话上下文**——作者刚在对话里
    /// 交代过的设定，起句式时能用上。这是与「快捷改写」同一条要求。
    ///
    /// 刻意不落进对话气泡：这是一次工具性取值，不是一轮对话；
    /// 把它塞进历史会污染后续轮次的语义。因此不更新 conversation revision。
    /// </summary>
    private async Task GenerateVariableSummaryAsync(WorkflowVariableGroupViewModel group)
    {
        try
        {
            var prompt = group.BuildGenerateSummaryPrompt();
            if (string.IsNullOrWhiteSpace(prompt))
            {
                // 提示词资源缺失：明说，而不是发一个空请求。
                StatusText = _displayNames.Text("ui.workspace.variable_summary_prompt_missing");
                return;
            }

            var sessionFence = _runSession.CaptureFence();
            var result = await _backend.ProjectAiChatAsync(
                prompt,
                workflowIdToRun: null,
                referenceWorkflowId: CurrentWorkflowId,
                referenceRunId: string.IsNullOrWhiteSpace(CurrentRunId) ? null : CurrentRunId,
                conversationId: ProjectAiConversationId,
                conversationRevision: _projectAiConversationRevision)
                .ConfigureAwait(true);
            _runSession.ThrowIfStale(sessionFence);

            var sentence = WorkflowVariableRules.CleanGeneratedSummary(result.Answer);
            if (sentence.Length == 0)
            {
                StatusText = _displayNames.Text("ui.workspace.variable_summary_generate_failed");
                return;
            }

            group.SummaryTemplate = sentence;
            StatusText = _displayNames.Text("ui.common.configured");
        }
        catch (OperationCanceledException)
        {
            // 工作流已切换；迟到的生成结果不得覆盖新会话的句式。
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private static string CoalescePath(Dictionary<string, object?> data)
    {
        var path = ReadString(data, "path");
        if (!string.IsNullOrWhiteSpace(path))
        {
            return path;
        }

        return ReadString(data, "import_path");
    }

    private static void LoadStopCondition(WorkflowNodeViewModel node, Dictionary<string, object?> data)
    {
        if (!data.TryGetValue("stop_condition", out var raw) || raw is null)
        {
            return;
        }

        if (raw is Dictionary<string, object?> dict)
        {
            if (dict.TryGetValue("input_alias", out var a) && a is not null)
            {
                node.StopInputAlias = a.ToString() ?? "done";
            }

            if ((dict.TryGetValue("equals", out var e) || dict.TryGetValue("expected", out e)) && e is not null)
            {
                node.StopExpected = e is bool b ? (b ? "true" : "false") : (e.ToString() ?? "true");
            }

            return;
        }

        if (raw is System.Text.Json.JsonElement el && el.ValueKind == System.Text.Json.JsonValueKind.Object)
        {
            if (el.TryGetProperty("input_alias", out var a))
            {
                node.StopInputAlias = a.GetString() ?? "done";
            }

            if (el.TryGetProperty("equals", out var e) || el.TryGetProperty("expected", out e))
            {
                node.StopExpected = e.ValueKind switch
                {
                    System.Text.Json.JsonValueKind.True => "true",
                    System.Text.Json.JsonValueKind.False => "false",
                    _ => e.ToString(),
                };
            }
        }
    }

    private static string ReadValueAsString(Dictionary<string, object?> data, string key)
    {
        if (!data.TryGetValue(key, out var value) || value is null)
        {
            return string.Empty;
        }

        return value switch
        {
            bool b => b ? "true" : "false",
            System.Text.Json.JsonElement el => el.ValueKind switch
            {
                System.Text.Json.JsonValueKind.True => "true",
                System.Text.Json.JsonValueKind.False => "false",
                System.Text.Json.JsonValueKind.String => el.GetString() ?? string.Empty,
                _ => el.ToString(),
            },
            _ => value.ToString() ?? string.Empty,
        };
    }

    private static IReadOnlyList<string> ReadStringList(Dictionary<string, object?> data, string key)
    {
        if (!data.TryGetValue(key, out var value) || value is null)
        {
            return Array.Empty<string>();
        }

        if (value is IEnumerable<object?> objs)
        {
            return objs.Select(o => o?.ToString() ?? string.Empty)
                .Where(s => !string.IsNullOrWhiteSpace(s))
                .ToArray();
        }

        if (value is System.Text.Json.JsonElement el && el.ValueKind == System.Text.Json.JsonValueKind.Array)
        {
            return el.EnumerateArray()
                .Select(e => e.GetString() ?? string.Empty)
                .Where(s => !string.IsNullOrWhiteSpace(s))
                .ToArray();
        }

        return Array.Empty<string>();
    }

    private void AttachNodeCommands(WorkflowNodeViewModel node)
    {
        node.SelectCommand = new RelayCommand(() => SelectNode(node));
        node.DataInPinRemoved = handle => OnDataInPinRemoved(node, handle);
        node.SummarizerChapterTextAliasChanged = (previous, current) =>
            OnSummarizerChapterTextAliasChanged(node, previous, current);
        node.QueryAliasChanged = (previous, current) =>
            OnUtilityInputAliasChanged(node, previous, current);
        node.StopInputAliasChanged = (previous, current) =>
            OnUtilityInputAliasChanged(node, previous, current);
    }

    private void OnUtilityInputAliasChanged(
        WorkflowNodeViewModel node,
        string previousAlias,
        string currentAlias)
    {
        if ((!node.IsSearchNode && !node.IsLoopNode) || string.IsNullOrWhiteSpace(currentAlias))
        {
            return;
        }

        var primaryHandle = node.DataInPins.FirstOrDefault()?.Handle;
        var edge = Edges.FirstOrDefault(candidate =>
            candidate.IsData
            && candidate.Target == node.Id
            && (string.IsNullOrWhiteSpace(primaryHandle)
                || string.Equals(candidate.TargetHandle, primaryHandle, StringComparison.OrdinalIgnoreCase)));
        if (edge is null)
        {
            return;
        }

        var prior = previousAlias.Trim();
        if (!string.IsNullOrWhiteSpace(edge.Label)
            && !string.IsNullOrWhiteSpace(prior)
            && !string.Equals(edge.Label.Trim(), prior, StringComparison.Ordinal))
        {
            return;
        }

        edge.Label = currentAlias.Trim();
        _edges = Edges.Select(item => item.ToCanvasEdge()).ToArray();
        RefreshDirtyState();
    }

    public void SelectNode(WorkflowNodeViewModel? node)
    {
        foreach (var item in Nodes)
        {
            item.IsSelected = item == node;
        }
        SelectedNode = node;
        if (node is not null)
        {
            IsProjectAiTab = false;
            IsRightPanelOpen = true;
        }

        // 换节点时清掉无关边选中；相关边配置仅随当前节点展示
        if (SelectedEdge is not null
            && (node is null
                || !CanvasSelectionHelpers.EdgeTouchesNode(SelectedEdge.Source, SelectedEdge.Target, node.Id)))
        {
            foreach (var edge in Edges)
            {
                edge.IsSelected = false;
            }

            SelectedEdge = null;
        }

        RefreshRelatedEdges();
        NotifySelectionCommands();
    }

    /// <summary>多选（框选 / Shift+点选）：主选为列表最后一项，供右栏细节。</summary>
    public void SelectNodes(IReadOnlyList<WorkflowNodeViewModel> nodes, bool additive = false)
    {
        var set = new HashSet<WorkflowNodeViewModel>(nodes);
        if (additive)
        {
            foreach (var item in Nodes)
            {
                if (set.Contains(item))
                {
                    item.IsSelected = true;
                }
            }
        }
        else
        {
            foreach (var item in Nodes)
            {
                item.IsSelected = set.Contains(item);
            }
        }

        SelectedNode = Nodes.LastOrDefault(n => n.IsSelected) ?? nodes.LastOrDefault();
        if (SelectedNode is not null)
        {
            IsProjectAiTab = false;
        }

        // 多选后只保留挂在已选节点上的边选中
        if (SelectedEdge is not null)
        {
            var ids = GetSelectedNodes().Select(n => n.Id).ToArray();
            if (!CanvasSelectionHelpers.EdgeTouchesAnyNode(SelectedEdge.Source, SelectedEdge.Target, ids))
            {
                foreach (var edge in Edges)
                {
                    edge.IsSelected = false;
                }

                SelectedEdge = null;
            }
        }

        RefreshRelatedEdges();
        NotifySelectionCommands();
    }

    /// <summary>刷新「与选中节点相关」的边列表。</summary>
    public void RefreshRelatedEdges()
    {
        // U145：标识候选依赖「当前选中的是谁」+「画布上有哪些边/节点」，
        // 而这个方法是所有选中变化与增删边的共同收口——挂在这里才不会漏路径。
        // （各调用点各写一遍必然漏，那会表现为「换了个节点但下拉还是上一个节点的别名」。）
        RefreshIdentifierCandidates();

        var selectedIds = GetSelectedNodes().Select(n => n.Id).ToArray();
        if (selectedIds.Length == 0 && SelectedNode is not null)
        {
            selectedIds = new[] { SelectedNode.Id };
        }

        RelatedEdges.Clear();
        if (selectedIds.Length == 0)
        {
            OnPropertyChanged(nameof(EdgeCountText));
            OnPropertyChanged(nameof(HasRelatedEdges));
            OnPropertyChanged(nameof(ShowEdgeConfigPanel));
            return;
        }

        foreach (var edge in Edges)
        {
            if (CanvasSelectionHelpers.EdgeTouchesAnyNode(edge.Source, edge.Target, selectedIds))
            {
                RelatedEdges.Add(edge);
            }
        }

        OnPropertyChanged(nameof(EdgeCountText));
        OnPropertyChanged(nameof(HasRelatedEdges));
        OnPropertyChanged(nameof(ShowEdgeConfigPanel));
    }

    /// <summary>框选矩形（逻辑坐标）命中的节点。</summary>
    public IReadOnlyList<WorkflowNodeViewModel> HitTestNodesInRect(
        double x0, double y0, double x1, double y1)
    {
        var (rx, ry, rw, rh) = CanvasSelectionHelpers.NormalizeRect(x0, y0, x1, y1);
        return Nodes
            .Where(n => CanvasSelectionHelpers.NodeIntersectsRect(
                n.X, n.Y, NodePortSpec.NodeWidth, n.CanvasHeight,
                rx, ry, rw, rh))
            .ToArray();
    }

    public IReadOnlyList<WorkflowNodeViewModel> GetSelectedNodes() =>
        Nodes.Where(n => n.IsSelected).ToArray();

    private void NotifySelectionCommands()
    {
        DeleteSelectedNodeCommand.NotifyCanExecuteChanged();
        RunSelectedNodeCommand.NotifyCanExecuteChanged();
        CopySelectedNodeCommand.NotifyCanExecuteChanged();
        CutSelectedNodeCommand.NotifyCanExecuteChanged();
        ApplyNodeConfigCommand.NotifyCanExecuteChanged();
        ToggleBreakpointCommand.NotifyCanExecuteChanged();
        BrowseImportFileCommand.NotifyCanExecuteChanged();
        BrowseWorkDirCommand.NotifyCanExecuteChanged();
        OnPropertyChanged(nameof(HasSelectedNode));
        OnPropertyChanged(nameof(IsSelectedStartNode));
        OnPropertyChanged(nameof(SelectedNodeTitle));
    }

    /// <summary>W1：优先删边（保留端点节点）；否则删选中节点。</summary>
    private async Task DeleteSelectionAsync()
    {
        if (CanvasSelectionHelpers.PreferDeleteEdgeOverNodes(HasSelectedEdge, HasSelectedNode)
            && SelectedEdge is not null)
        {
            DeleteSelectedEdge();
            return;
        }

        await DeleteSelectedNodeAsync().ConfigureAwait(true);
    }

    /// <summary>W1 产品入口：删除选中边，不删除端点节点；可 undo。</summary>
    public void DeleteSelectedEdge()
    {
        var edge = SelectedEdge;
        if (edge is null)
        {
            StatusText = _displayNames.Text("ui.common.none");
            return;
        }

        CaptureUndoSnapshot();
        Edges.Remove(edge);
        _edges = Edges.Select(item => item.ToCanvasEdge()).ToArray();
        SelectedEdge = null;
        foreach (var item in Edges)
        {
            item.IsSelected = false;
        }

        RefreshRelatedEdges();
        RefreshEdgeLabels();
        RefreshPortConnectionStates();
        OnPropertyChanged(nameof(EdgeCountText));
        OnPropertyChanged(nameof(HasSelectedEdge));
        OnPropertyChanged(nameof(ShowEdgeConfigPanel));
        RefreshDirtyState();
        NotifySelectionCommands();
        StatusText = _displayNames.Text("ui.workspace.edge.deleted");
    }

    private async Task DeleteSelectedNodeAsync()
    {
        var selected = GetSelectedNodes();
        if (selected.Count == 0 && SelectedNode is not null)
        {
            selected = new[] { SelectedNode };
        }

        if (selected.Count == 0)
        {
            StatusText = NoNodeSelectedText;
            return;
        }

        if (!await ConfirmDangerAsync(
                "ui.dialog.workspace.delete_node.title",
                "ui.dialog.workspace.delete_node.message",
                "ui.dialog.workspace.delete_node.confirm").ConfigureAwait(true))
        {
            return;
        }

        DeleteNodes(selected);
        StatusText = _displayNames.Format("ui.workspace.deleted_selection", new Dictionary<string, string>
        {
            ["count"] = selected.Count.ToString(),
        });
    }

    private void DeleteNode(WorkflowNodeViewModel node) => DeleteNodes(new[] { node });

    private void DeleteNodes(IReadOnlyList<WorkflowNodeViewModel> nodes)
    {
        if (nodes.Count == 0)
        {
            return;
        }

        CaptureUndoSnapshot();
        var ids = nodes.Select(n => n.Id).ToHashSet(StringComparer.Ordinal);
        foreach (var node in nodes.ToArray())
        {
            Nodes.Remove(node);
        }

        _edges = _edges
            .Where(edge => !ids.Contains(edge.Source) && !ids.Contains(edge.Target))
            .ToArray();
        Edges.Clear();
        foreach (var edge in _edges)
        {
            Edges.Add(new WorkflowEdgeViewModel(edge, _displayNames, SelectEdge, RefreshDirtyState));
        }
        SelectedNode = null;
        SelectedEdge = null;
        RefreshRelatedEdges();
        OnPropertyChanged(nameof(EdgeCountText));
        RefreshEdgeLabels();
        RefreshStartNodes();
        RefreshPortConnectionStates();
        RefreshDirtyState();
        NotifySelectionCommands();
    }

    private void CopySelectedNode()
    {
        var node = SelectedNode;
        if (node is null)
        {
            StatusText = NoNodeSelectedText;
            return;
        }

        _clipboardNode = node.ToCanvasNode();
        PasteNodeCommand.NotifyCanExecuteChanged();
        StatusText = _displayNames.Format("ui.workspace.copied_selection", new Dictionary<string, string>
        {
            ["count"] = "1",
        });
    }

    private async Task CutSelectedNodeAsync()
    {
        var node = SelectedNode;
        if (node is null)
        {
            StatusText = NoNodeSelectedText;
            return;
        }
        if (!await ConfirmDangerAsync(
                "ui.dialog.workspace.cut_node.title",
                "ui.dialog.workspace.cut_node.message",
                "ui.dialog.workspace.cut_node.confirm").ConfigureAwait(true))
        {
            return;
        }

        _clipboardNode = node.ToCanvasNode();
        PasteNodeCommand.NotifyCanExecuteChanged();
        CaptureUndoSnapshot();
        DeleteNode(node);
        StatusText = _displayNames.Format("ui.workspace.cut_selection", new Dictionary<string, string>
        {
            ["count"] = "1",
        });
    }

    private void PasteNode()
    {
        if (_clipboardNode is null)
        {
            StatusText = _displayNames.Text("ui.common.none");
            return;
        }

        var data = JsonSerializer.Deserialize<Dictionary<string, object?>>(
            JsonSerializer.Serialize(_clipboardNode.Data, JsonOptions),
            JsonOptions) ?? new Dictionary<string, object?>();
        var position = new CanvasPosition(
            (_clipboardNode.Position?.X ?? 120) + 36,
            (_clipboardNode.Position?.Y ?? 80) + 36);
        var pasted = _clipboardNode with
        {
            Id = NextNodeId(_clipboardNode.Type),
            Data = data,
            Position = position,
        };
        var node = CreateNodeFromCanvas(pasted);
        CaptureUndoSnapshot();
        Nodes.Add(node);
        RequestEnsureNodeVisible?.Invoke(node);
        RefreshStartNodes();
        SelectNode(node);
        RefreshDirtyState();
        StatusText = _displayNames.Format("ui.workspace.pasted_selection", new Dictionary<string, string>
        {
            ["count"] = "1",
        });
    }

    private void FitView()
    {
        if (Nodes.Count == 0)
        {
            StatusText = _displayNames.Text("ui.common.none");
            return;
        }

        RequestFitView?.Invoke();
        StatusText = CtxFitViewText;
    }

    private void AdjustCanvasZoom(double delta)
    {
        if (RequestCanvasZoomStep is not null)
        {
            RequestCanvasZoomStep(delta);
        }
        else
        {
            CanvasZoom += delta;
        }
        StatusText = CanvasZoomText;
    }

    private void ResetCanvasZoom()
    {
        if (RequestResetCanvasZoom is not null)
        {
            RequestResetCanvasZoom();
        }
        else
        {
            CanvasZoom = 1.0;
        }
        StatusText = CanvasZoomText;
    }

    private static GridLength NormalizeRightPanelWidth(GridLength value)
    {
        if (value.IsStar)
        {
            return new GridLength(360);
        }
        var width = value.IsAuto ? 360 : value.Value;
        return new GridLength(WorkspaceResponsiveLayoutHelpers.NormalizeRequestedRightPanelWidth(width));
    }

    private async Task InitializeWorkflowAsync(CancellationToken cancellationToken = default)
    {
        // 无打开项目：保持空画布，不打项目 IPC（避免 cwd 误当项目 / 英文技术报错）
        if (!_backend.HasProjectRoot)
        {
            SetWorkflowLoadState(WorkflowLoadState.NoProject);
            Nodes.Clear();
            StartNodes.Clear();
            Edges.Clear();
            StatusText = string.Empty;
            CaptureSnapshot();
            OnPropertyChanged(nameof(HasStartNodes));
            OnPropertyChanged(nameof(HasNodes));
            OnPropertyChanged(nameof(EmptyStartTitle));
            OnPropertyChanged(nameof(EmptyStartHint));
            OnPropertyChanged(nameof(EmptyCanvasTitle));
            OnPropertyChanged(nameof(EmptyCanvasHint));
            return;
        }

        try
        {
            await LoadProjectCanvasAsync(cancellationToken: cancellationToken).ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch
        {
            SetWorkflowLoadState(WorkflowLoadState.LoadFailed);
            StatusText = _displayNames.Text("ui.workspace.load_failed");
            OnPropertyChanged(nameof(HasStartNodes));
            OnPropertyChanged(nameof(HasNodes));
            OnPropertyChanged(nameof(EmptyCanvasTitle));
            OnPropertyChanged(nameof(EmptyCanvasHint));
        }
    }

    private async Task LoadAvailableModelsAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            var config = await _backend.GetProviderConfigAsync(cancellationToken).ConfigureAwait(true);
            cancellationToken.ThrowIfCancellationRequested();
            AvailableModelOptions.Clear();
            AvailableModelOptions.Add(WorkflowModelOption.Inherited(ModelInheritGlobalText));
            foreach (var provider in config.Providers
                         .Where(provider => provider.Enabled)
                         .OrderBy(provider => provider.DisplayName, StringComparer.Ordinal)
                         .ThenBy(provider => provider.Provider, StringComparer.Ordinal))
            {
                foreach (var model in provider.Models
                             .Where(model => IsLlmModelCapability(model.Capability))
                             .Where(model => !string.IsNullOrWhiteSpace(model.ModelId))
                             .OrderBy(model => model.ModelId, StringComparer.Ordinal))
                {
                    AvailableModelOptions.Add(new WorkflowModelOption(
                        provider.Provider,
                        model.ModelId,
                        string.IsNullOrWhiteSpace(provider.DisplayName)
                            ? provider.Provider
                            : provider.DisplayName));
                }
            }

            OnPropertyChanged(nameof(HasAvailableModelChoices));
            OnPropertyChanged(nameof(SelectedNodeModelOption));
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch
        {
            // 获取失败时保留节点已有 provider_id/model_id，不伪造候选项。
        }
    }

    private static bool IsLlmModelCapability(string? capability) =>
        string.Equals(capability, "llm", StringComparison.OrdinalIgnoreCase)
        || string.Equals(capability, "tool_use", StringComparison.OrdinalIgnoreCase);

    private async Task LoadSummarizerChapterOptionsAsync(CancellationToken cancellationToken = default)
    {
        SummarizerChapterOptions.Clear();
        if (!_backend.HasProjectRoot)
        {
            OnPropertyChanged(nameof(HasSummarizerChapterChoices));
            OnPropertyChanged(nameof(SelectedSummarizerChapterOption));
            return;
        }

        try
        {
            var tree = await _backend.GetWorksTreeAsync(cancellationToken).ConfigureAwait(true);
            cancellationToken.ThrowIfCancellationRequested();
            foreach (var option in FlattenSummarizerChapterOptions(tree)
                         .GroupBy(item => (item.ChapterId, item.DocumentId))
                         .Select(group => group.First())
                         .OrderBy(item => item.DisplayTitle, StringComparer.CurrentCulture))
            {
                SummarizerChapterOptions.Add(option);
            }
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch (Exception error)
        {
            ReportFailure(error, _displayNames);
        }
        finally
        {
            OnPropertyChanged(nameof(HasSummarizerChapterChoices));
            OnPropertyChanged(nameof(SelectedSummarizerChapterOption));
            // U145：章节 id / 文档 id 候选就来自这份列表，装载完必须同步一次；
            // 否则「先选中 summarizer 节点、再等作品树到货」这条顺序下候选恒为空。
            RefreshIdentifierCandidates();
        }
    }

    private static IEnumerable<SummarizerChapterOption> FlattenSummarizerChapterOptions(
        WorksTreeNode node)
    {
        if (string.Equals(node.Kind, "chapter", StringComparison.Ordinal)
            && !string.IsNullOrWhiteSpace(node.ChapterId)
            && !string.IsNullOrWhiteSpace(node.DocumentId))
        {
            yield return new SummarizerChapterOption(
                node.ChapterId,
                node.DocumentId,
                node.Title,
                node.Path);
        }

        foreach (var child in node.Children)
        {
            foreach (var option in FlattenSummarizerChapterOptions(child))
            {
                yield return option;
            }
        }
    }

    private async Task LoadProjectCanvasAsync(
        bool confirmLeave = false,
        CancellationToken cancellationToken = default)
    {
        if (!_backend.HasProjectRoot)
        {
            SetWorkflowLoadState(WorkflowLoadState.NoProject);
            Nodes.Clear();
            StartNodes.Clear();
            Edges.Clear();
            StatusText = string.Empty;
            CaptureSnapshot();
            OnPropertyChanged(nameof(HasStartNodes));
            OnPropertyChanged(nameof(HasNodes));
            OnPropertyChanged(nameof(EmptyCanvasTitle));
            OnPropertyChanged(nameof(EmptyCanvasHint));
            return;
        }

        var request = _canvasLoading.Begin();
        if (confirmLeave && !await ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
        {
            return;
        }
        var hadLoadedWorkflow = _workflowLoadState == WorkflowLoadState.Loaded;
        SetWorkflowLoadState(WorkflowLoadState.Loading);
        StatusText = _displayNames.Text("ui.common.loading");
        using var linkedCancellation = CancellationTokenSource.CreateLinkedTokenSource(
            request.CancellationToken,
            cancellationToken);
        try
        {
            var graph = await _backend.LoadProjectCanvasAsync(linkedCancellation.Token).ConfigureAwait(true);
            linkedCancellation.Token.ThrowIfCancellationRequested();
            if (!_canvasLoading.IsCurrent(request))
            {
                return;
            }
            if (!string.Equals(graph.WorkflowId, DefaultWorkflowId, StringComparison.Ordinal))
            {
                throw new InvalidOperationException(
                    $"project canvas identity mismatch: expected={DefaultWorkflowId}, actual={graph.WorkflowId}");
            }

            _runSession.Reset();
            ApplyGraph(graph);
            CaptureSnapshot();
            SetWorkflowLoadState(WorkflowLoadState.Loaded);
            StatusText = _displayNames.Text("ui.common.open");
        }
        catch (OperationCanceledException) when (linkedCancellation.IsCancellationRequested)
        {
        }
        catch
        {
            if (_canvasLoading.IsCurrent(request))
            {
                if (!hadLoadedWorkflow)
                {
                    SetWorkflowLoadState(WorkflowLoadState.LoadFailed);
                }
                StatusText = _displayNames.Text("ui.workspace.load_failed");
                OnPropertyChanged(nameof(HasStartNodes));
                OnPropertyChanged(nameof(HasNodes));
            }
        }
    }

    /// <summary>
    /// 显式重新加载项目规范画布，避免把该动作误标为“导入”。
    /// </summary>
    private async Task ReloadProjectCanvasWithUnsavedCheckAsync()
    {
        await LoadProjectCanvasAsync(confirmLeave: true).ConfigureAwait(true);
        if (_workflowLoadState == WorkflowLoadState.Loaded)
        {
            StatusText = _displayNames.Text("ui.workspace.reload_default_done");
        }
    }

    private async Task SaveWorkflowAsync()
    {
        if (!EnsureWorkflowLoadedForPersistence())
        {
            return;
        }
        try
        {
            var graph = BuildGraph();
            await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
            var saved = await _backend.SaveProjectCanvasAsync(graph).ConfigureAwait(true);
            RememberWorkflowRevision(saved);
            AcceptSavedGraph(graph);
            StatusText = _displayNames.Text("ui.common.save");
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private async Task BrowseImportFileAsync()
    {
        if (SelectedNode is not { IsImportNode: true })
        {
            StatusText = NoNodeSelectedText;
            return;
        }

        if (PickFile is null)
        {
            StatusText = _displayNames.Text("ui.settings.browse_unavailable");
            return;
        }

        try
        {
            var picked = await PickFile(_displayNames.Text("ui.workspace.import.browse")).ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(picked))
            {
                return;
            }

            SelectedNode.ImportPath = picked;
            SelectedNode.StatusText = System.IO.Path.GetFileName(picked);
            RefreshDirtyState();
            StatusText = _displayNames.Text("ui.workspace.import.file");
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private async Task BrowseWorkDirAsync()
    {
        if (SelectedNode is null || !SelectedNode.IsStartNode)
        {
            StatusText = NoNodeSelectedText;
            return;
        }

        if (PickFolder is null)
        {
            StatusText = _displayNames.Text("ui.settings.browse_unavailable");
            return;
        }

        try
        {
            var picked = await PickFolder(
                _displayNames.Text("ui.workspace.start_node.browse_work_dir_title")).ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(picked))
            {
                return;
            }

            var project = await _backend.GetCurrentProjectAsync().ConfigureAwait(true);
            var root = project?.ProjectRoot;
            if (string.IsNullOrWhiteSpace(root))
            {
                StatusText = _displayNames.Text("ui.workspace.start_node.work_dir_no_project");
                return;
            }

            if (!ProjectPathHelper.TryMakeRelativeToProjectRoot(picked, root, out var relative))
            {
                StatusText = _displayNames.Format(
                    "ui.workspace.start_node.work_dir_outside_project",
                    new Dictionary<string, string>
                    {
                        ["path"] = picked,
                        ["root"] = root,
                    });
                return;
            }

            SelectedNode.WorkDir = relative;
            StatusText = _displayNames.Format(
                "ui.workspace.start_node.work_dir_set",
                new Dictionary<string, string> { ["path"] = relative });
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private async Task ExportWorkflowAsync(bool requireSelection)
    {
        if (!EnsureWorkflowLoadedForPersistence())
        {
            return;
        }
        try
        {
            if (requireSelection && SelectedNode is null)
            {
                StatusText = NoNodeSelectedText;
                return;
            }

            // requireSelection:true → 仅选中节点；false（工具栏「导出图」）→ 始终全部节点，不被选中缩窄
            var allIds = Nodes.Select(node => node.Id).ToArray();
            var selected = WorkflowExportSelection.ResolveNodeIds(
                requireSelection,
                SelectedNode?.Id,
                allIds);
            if (selected.Length == 0)
            {
                StatusText = _displayNames.Text("ui.workspace.export_selection_empty");
                return;
            }

            var wasDirty = HasUnsavedChanges;
            var graph = BuildGraph();
            await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
            RememberWorkflowRevision(await _backend.SaveProjectCanvasAsync(graph).ConfigureAwait(true));
            CaptureSnapshot();
            var export = await _backend
                .ExportWorkflowSelectionAsync(CurrentWorkflowId, selected)
                .ConfigureAwait(true);

            // 返回值**不能再丢**。此前这里是 `await ...(...)` 后直接写「已导出 N 个节点」，
            // 而整条链路没有任何写盘 ⇒ 用户看到成功提示、磁盘上找不到文件，
            // 与 U156「点运行什么都不会发生」同型。现在以 storage_uri 是否存在为准：
            // 没有落盘位置就明确说没产生文件，不许把空动作报成成功。
            if (string.IsNullOrWhiteSpace(export.StorageUri))
            {
                StatusText = _displayNames.Text("ui.workspace.export_no_file");
                return;
            }

            // 报出**路径**而不只是节点数：U134 的教训是「导出成功却不知道文件在哪」，
            // 那条弹窗甚至把用户指向一个不生效的设置项。
            var exported = _displayNames.Format("ui.workspace.exported_selection_to", new Dictionary<string, string>
            {
                ["count"] = selected.Length.ToString(),
                ["path"] = export.StorageUri!,
            });
            // 导出前静默落盘易让作者以为「没保存」；明确提示
            StatusText = wasDirty
                ? exported + " " + _displayNames.Text("ui.workspace.export_autosaved")
                : exported;
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private async Task RunSelectedNodeAsync()
    {
        if (SelectedNode is not null)
        {
            if (!SelectedNode.IsStartNode)
            {
                StatusText = SelectStartNodeText;
                return;
            }
            await RunNodeAsync(SelectedNode).ConfigureAwait(true);
        }
    }

    private async Task RunNodeAsync(WorkflowNodeViewModel node)
    {
        if (!EnsureWorkflowLoadedForPersistence())
        {
            return;
        }
        var workflowId = CurrentWorkflowId;
        var sessionFence = _runSession.CaptureFence();
        try
        {
            // required 变量留空 / 类型填错时不发请求：本地拦住并写明是哪个变量，
            // 比让后端拒绝再翻译错误更直接。禁用理由必须配文字，不能只灰掉按钮。
            var blocking = node.Variables?.BlockingReason();
            if (blocking is not null)
            {
                node.StatusText = blocking;
                StatusText = blocking;
                return;
            }
            var startNodeId = node.IsStartNode ? node.Id : null;
            var graph = BuildGraph();
            await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
            RememberWorkflowRevision(await _backend.SaveProjectCanvasAsync(graph).ConfigureAwait(true));
            CaptureSnapshot();
            _runSession.ThrowIfStale(sessionFence);
            var run = await _runSession
                .StartAsync(workflowId, startNodeId, node.Variables?.BuildStartVariables())
                .ConfigureAwait(true);
            node.StatusText = UserFacingError.RuntimeStatus(run.Status, _displayNames);
            StatusText = UserFacingError.RuntimeStatus(run.Status, _displayNames);
        }
        catch (OperationCanceledException)
        {
            // 工作流已切换；迟到的启动结果不得覆盖新会话的页面状态。
        }
        catch (Exception ex)
        {
            node.StatusText = ReportFailure(ex, _displayNames);
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    /// <summary>
    /// U207-C①：下栏「运行」按钮 —— 从起始节点开跑。
    ///
    /// 挑哪个起始节点：优先当前选中的那个（作者的注意力就在它上面），
    /// 否则取第一个。与 Ctrl+K 的 <see cref="ResolveVariableFillTarget"/> 同一套取舍，
    /// 区别是这里不要求节点带变量。
    ///
    /// 落到 <see cref="RunNodeAsync"/> 而不是另开一条启动路径：那条路上
    /// 已经串好了 required 变量本地拦截 → 图校验 → 落盘 → 快照 → run session，
    /// 绕开它就等于新造一个少了四道工序的运行入口
    /// （`WorkspaceCanvas08Tests.N9` 守的正是「全部运行入口走同一个 coordinator」）。
    /// </summary>
    private async Task RunWorkflowFromEntryAsync()
    {
        var target = ResolveRunEntryTarget();
        if (target is null)
        {
            // 理论上按钮此时是禁用的；真到了这里也要给句话，不能静默什么都不做——
            // 「点了没反应」正是本编号被误报成「运行键失灵」的起因。
            StatusText = _displayNames.Text("ui.workspace.run.needs_start_node");
            return;
        }

        await RunNodeAsync(target).ConfigureAwait(true);
    }

    /// <summary>「运行」这一下要从哪个起始节点开跑。</summary>
    private WorkflowNodeViewModel? ResolveRunEntryTarget()
    {
        if (SelectedNode is { IsStartNode: true } selected)
        {
            return selected;
        }

        return StartNodes.FirstOrDefault();
    }

    private async Task PauseWorkflowAsync()
    {
        await RunControlAsync(() => _runSession.PauseAsync(StatusText));
    }

    private async Task StopWorkflowAsync()
    {
        if (string.IsNullOrWhiteSpace(CurrentRunId))
        {
            StatusText = _displayNames.Text("ui.common.none");
            return;
        }
        var sessionFence = _runSession.CaptureFence();
        if (!await ConfirmDangerAsync(
                "ui.dialog.workspace.stop_run.title",
                "ui.dialog.workspace.stop_run.message",
                "ui.dialog.workspace.stop_run.confirm").ConfigureAwait(true))
        {
            return;
        }
        await RunControlAsync(() =>
        {
            _runSession.ThrowIfStale(sessionFence);
            return _runSession.StopAsync(StatusText);
        });
    }

    private async Task ResumeWorkflowAsync()
    {
        await RunControlAsync(() => _runSession.ResumeAsync());
    }

    /// <summary>
    /// U196-D：从失败的那个节点重跑。
    ///
    /// 发出的是 `retry_failed_node` 而**不是** `resume_workflow`：后者在后端走
    /// `store::claim_resume`，那里只接受 `Paused | Queued | Running`，
    /// 失败的运行会拿到 NotResumable —— 换成它的话按钮点得动、请求发得出、
    /// 回包也不报错，而运行状态一动不动。所以本条的判据必须落在
    /// **真实出站请求的方法名与 node_id 上**，不能只测「命令能不能执行」。
    ///
    /// 重跑前清掉上一次的失败建议：那句话属于刚被撤销的那次失败，
    /// 留在屏幕上会让作者以为重跑又立刻失败了。
    /// </summary>
    private async Task RetryFailedNodeAsync()
    {
        var nodeId = FailedNodeId;
        if (string.IsNullOrWhiteSpace(nodeId))
        {
            return;
        }
        RecoveryText = string.Empty;
        await RunControlAsync(() => _runSession.RetryFailedNodeAsync(nodeId));
    }

    private async Task RunControlAsync(Func<Task<WorkflowActionResult>> action)
    {
        if (string.IsNullOrWhiteSpace(CurrentRunId))
        {
            StatusText = _displayNames.Text("ui.common.none");
            return;
        }
        try
        {
            var result = await action().ConfigureAwait(true);
            StatusText = UserFacingError.RuntimeStatus(result.Status, _displayNames);
        }
        catch (OperationCanceledException)
        {
            // 控制请求返回前已切换会话；协调器已拒绝旧结果。
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private void ApplyWorkflowEvents(WorkflowEventsResult result)
    {
        StatusText = UserFacingError.RuntimeStatus(result.Status, _displayNames);
        foreach (var runtimeEvent in result.Events)
        {
            if (!string.IsNullOrWhiteSpace(runtimeEvent.NodeId))
            {
                var node = Nodes.FirstOrDefault(item => item.Id == runtimeEvent.NodeId);
                if (node is not null)
                {
                    // Never use runtimeEvent.Message (engineer text) as primary node status.
                    var code = NodeStatusFromEvent(runtimeEvent.EventType, CurrentRunStatus);
                    node.StatusText = UserFacingError.RuntimeStatus(code, _displayNames);
                }
            }
            if (runtimeEvent.EventType is "confirmation_updated")
            {
                _ = LoadConfirmationsAsync();
            }
        }
        if (result.Events.Any(item => item.EventType is "run_paused" or "confirmation_updated"))
        {
            _ = LoadConfirmationsAsync();
        }
        if (result.Status == "paused")
        {
            _ = LoadInDoubtOperationsAsync();
        }
    }

    private static string NodeStatusFromEvent(string eventType, string fallback)
    {
        return eventType switch
        {
            "node_started" => "running",
            "node_succeeded" => "succeeded",
            "node_paused" => "paused",
            "node_failed" => "failed",
            "node_skipped" => "skipped",
            "node_retry_scheduled" => "retry_scheduled",
            _ => fallback,
        };
    }

    private async Task LoadInDoubtOperationsAsync()
    {
        InDoubtOperations.Clear();
        SelectedInDoubtOperation = null;
        OnPropertyChanged(nameof(HasInDoubtOperations));
        if (!_backend.HasProjectRoot || string.IsNullOrWhiteSpace(CurrentRunId))
        {
            return;
        }
        try
        {
            var operations = await _backend
                .ListInDoubtOperationsAsync(CurrentWorkflowId, CurrentRunId)
                .ConfigureAwait(true);
            foreach (var operation in operations)
            {
                InDoubtOperations.Add(operation);
            }
            SelectedInDoubtOperation = InDoubtOperations.FirstOrDefault();
            OnPropertyChanged(nameof(HasInDoubtOperations));
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private async Task ResolveSelectedInDoubtOperationAsync(string decision)
    {
        var operation = SelectedInDoubtOperation;
        if (operation is null)
        {
            return;
        }
        var sessionFence = _runSession.CaptureFence();
        object? response = null;
        if (decision == "use_response")
        {
            try
            {
                response = JsonNode.Parse(InDoubtResponseJson)
                    ?? throw new JsonException("empty JSON response");
            }
            catch (JsonException)
            {
                StatusText = _displayNames.Text("ui.workspace.in_doubt.invalid_response");
                return;
            }
        }
        if (decision == "stop"
            && !await ConfirmDangerAsync(
                    "ui.dialog.workspace.stop_run.title",
                    "ui.dialog.workspace.stop_run.message",
                    "ui.dialog.workspace.stop_run.confirm").ConfigureAwait(true))
        {
            return;
        }
        try
        {
            _runSession.ThrowIfStale(sessionFence);
            var result = await _backend.ResolveInDoubtOperationAsync(
                operation.OperationId,
                decision,
                response,
                string.IsNullOrWhiteSpace(InDoubtStopReason) ? null : InDoubtStopReason).ConfigureAwait(true);
            _runSession.ThrowIfStale(sessionFence);
            _runSession.Attach(
                result.Workflow.WorkflowId,
                result.Workflow.RunId,
                result.Workflow.Status,
                startPolling: !WorkspaceRunSessionCoordinator.IsTerminal(result.Workflow.Status));
            StatusText = _displayNames.Text("ui.workspace.in_doubt.resolved");
            InDoubtResponseJson = string.Empty;
            InDoubtStopReason = string.Empty;
            await LoadInDoubtOperationsAsync().ConfigureAwait(true);
        }
        catch (OperationCanceledException)
        {
            // 解析请求属于旧会话；不将其结果挂接到当前工作流。
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private async Task SendProjectAiAsync()
    {
        try
        {
            if (string.IsNullOrWhiteSpace(ProjectAiMessage))
            {
                StatusText = ProjectAiPlaceholder;
                return;
            }
            if (!EnsureWorkflowLoadedForPersistence())
            {
                return;
            }
            var workflowId = CurrentWorkflowId;
            var referenceRunId = string.IsNullOrWhiteSpace(CurrentRunId) ? null : CurrentRunId;
            var message = ProjectAiMessage;
            var sessionFence = _runSession.CaptureFence();
            if (HasUnsavedChanges)
            {
                var graph = BuildGraph();
                await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
                RememberWorkflowRevision(await _backend.SaveProjectCanvasAsync(graph).ConfigureAwait(true));
                CaptureSnapshot();
            }
            _runSession.ThrowIfStale(sessionFence);
            // 起点由后端 list_start_nodes 工具 + AI 自行抉择；前端不提供优先起点。
            var result = await _backend.ProjectAiChatAsync(
                message,
                workflowIdToRun: null,
                referenceWorkflowId: workflowId,
                referenceRunId: referenceRunId,
                conversationId: ProjectAiConversationId,
                conversationRevision: _projectAiConversationRevision)
                .ConfigureAwait(true);
            _runSession.ThrowIfStale(sessionFence);
            ProjectAiAnswer = result.Answer;
            _projectAiConversationRevision = ProjectAiConversationUi.Apply(
                result,
                _projectAiHistory,
                ProjectAiBubbles,
                _projectAiConversationRevision);
            OnPropertyChanged(nameof(HasProjectAiBubbles));
            ProjectAiMessage = string.Empty;
            StatusText = result.WorkflowRun is not null
                ? UserFacingError.RuntimeStatus(result.WorkflowRun.Status, _displayNames)
                : ProjectAiConversationUi.ContextWasCompacted(result)
                    ? _displayNames.Text("ui.project_ai.context_compacted")
                    : _displayNames.Text("ui.common.configured");
            if (result.WorkflowRun is not null)
            {
                _runSession.Attach(
                    workflowId,
                    result.WorkflowRun.RunId,
                    result.WorkflowRun.Status,
                    resetCursor: true);
            }
        }
        catch (OperationCanceledException)
        {
            // Project AI 响应属于已切走的工作流；丢弃其页面投影。
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    /// <summary>
    /// 带着当前确认项去问项目 AI「这段改得对不对」（U139④）。
    ///
    /// 后端 `@确认项:<id>` 引用链路一直是通的（`resolve_reference` → `resolve_confirmation`
    /// 返回 summary + state + diff），断的只是前端：`references` 被写死成空数组、
    /// 审阅面板也没有任何入口。这里两头都接上。
    ///
    /// **引用必须带 `@确认项:` 前缀**：顶层 `parse_project_reference` 要求含 `:` 或 `/`，
    /// 裸 id 只有内层 store 容忍，走项目 AI 这条路会被判非法引用。
    ///
    /// 走 references 而不是把 diff 拼进 message：diff 可能是整段章节正文，
    /// 内联传大段文本违反引用式数据流；而且后端展开引用时会自己做截断与预算记账。
    /// </summary>
    private async Task AskAiAboutConfirmationAsync()
    {
        var confirmation = SelectedConfirmation;
        if (confirmation is null)
        {
            StatusText = ConfirmationsEmptyText;
            return;
        }
        // 审阅态下右栏应当已经是项目 AI 页；用户手动切走后再点「问 AI」，
        // 把它切回来——否则回答落在一个看不见的页面上。
        SetRightPanelTab(WorkspaceRightPanelTab.ProjectAi);
        IsRightPanelOpen = true;
        var question = _displayNames.Text("ui.workspace.confirmation.ask_ai.question");
        var reference = $"@确认项:{confirmation.ConfirmationId}";
        var workflowId = string.IsNullOrWhiteSpace(confirmation.WorkflowId)
            ? CurrentWorkflowId
            : confirmation.WorkflowId;
        var referenceRunId = !string.IsNullOrWhiteSpace(confirmation.RunId)
            ? confirmation.RunId
            : string.IsNullOrWhiteSpace(CurrentRunId) ? null : CurrentRunId;
        var sessionFence = _runSession.CaptureFence();
        try
        {
            // 提问先进气泡：审阅期间等回答可能要几秒，不回显的话点了像没反应。
            ProjectAiBubbles.Add(new ChatBubbleViewModel("user", question));
            OnPropertyChanged(nameof(HasProjectAiBubbles));
            StatusText = _displayNames.Text("ui.workspace.confirmation.ask_ai.sent");
            var result = await _backend.ProjectAiChatAsync(
                question,
                workflowIdToRun: null,
                referenceWorkflowId: string.IsNullOrWhiteSpace(workflowId) ? null : workflowId,
                referenceRunId: referenceRunId,
                conversationId: ProjectAiConversationId,
                conversationRevision: _projectAiConversationRevision,
                references: new[] { reference })
                .ConfigureAwait(true);
            _runSession.ThrowIfStale(sessionFence);
            ProjectAiAnswer = result.Answer;
            _projectAiConversationRevision = ProjectAiConversationUi.Apply(
                result,
                _projectAiHistory,
                ProjectAiBubbles,
                _projectAiConversationRevision);
            OnPropertyChanged(nameof(HasProjectAiBubbles));
            StatusText = ProjectAiConversationUi.ContextWasCompacted(result)
                ? _displayNames.Text("ui.project_ai.context_compacted")
                : _displayNames.Text("ui.common.configured");
        }
        catch (OperationCanceledException)
        {
            // 回答属于已切走的运行会话；丢弃其页面投影。
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    /// <summary>
    /// 带着刚查到的知识条目去问项目 AI（U206-B）。
    ///
    /// 与 U139④ 走同一条出站通道，差别只有引用前缀（`@知识:` vs `@确认项:`）。
    /// **不抽公共方法**：那两条路的上下文取值规则不同——确认项要把它自己的
    /// workflow_id / run_id 顶掉当前值（审阅的可能是别的运行留下的确认项），
    /// 知识查询与运行无关，传当前值即可。抽出来只能把这段差异变成一串开关参数。
    /// </summary>
    private async Task AskAiAboutKnowledgeAsync(string reference)
    {
        // 回答落在项目 AI 栏；用户手动切走后再点，把它切回来，否则答案落在看不见的页面上。
        SetRightPanelTab(WorkspaceRightPanelTab.ProjectAi);
        IsRightPanelOpen = true;
        var question = _displayNames.Format(
            "ui.workspace.knowledge_lookup.ask_ai.question",
            new Dictionary<string, string> { ["reference"] = reference });
        var sessionFence = _runSession.CaptureFence();
        try
        {
            // 提问先进气泡：等回答可能要几秒，不回显的话点了像没反应。
            ProjectAiBubbles.Add(new ChatBubbleViewModel("user", question));
            OnPropertyChanged(nameof(HasProjectAiBubbles));
            StatusText = _displayNames.Text("ui.workspace.knowledge_lookup.ask_ai.sent");
            var result = await _backend.ProjectAiChatAsync(
                question,
                workflowIdToRun: null,
                referenceWorkflowId: string.IsNullOrWhiteSpace(CurrentWorkflowId) ? null : CurrentWorkflowId,
                referenceRunId: string.IsNullOrWhiteSpace(CurrentRunId) ? null : CurrentRunId,
                conversationId: ProjectAiConversationId,
                conversationRevision: _projectAiConversationRevision,
                references: new[] { reference })
                .ConfigureAwait(true);
            _runSession.ThrowIfStale(sessionFence);
            ProjectAiAnswer = result.Answer;
            _projectAiConversationRevision = ProjectAiConversationUi.Apply(
                result,
                _projectAiHistory,
                ProjectAiBubbles,
                _projectAiConversationRevision);
            OnPropertyChanged(nameof(HasProjectAiBubbles));
            StatusText = ProjectAiConversationUi.ContextWasCompacted(result)
                ? _displayNames.Text("ui.project_ai.context_compacted")
                : _displayNames.Text("ui.common.configured");
        }
        catch (OperationCanceledException)
        {
            // 回答属于已切走的运行会话；丢弃其页面投影。
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private async Task ApplyNodeConfigAsync()
    {
        if (SelectedNode is null)
        {
            StatusText = NoNodeSelectedText;
            return;
        }
        if (!EnsureWorkflowLoadedForPersistence())
        {
            return;
        }
        try
        {
            // 先整图落盘：节点名 / work_dir / 暴露工具 / 边 等细节 patch 覆盖不到。
            // 旧逻辑只 patch 再 LoadWorkflow，会冲掉未保存画布改动（新边、拖动、名称等）。
            var graph = BuildGraph();
            await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
            RememberWorkflowRevision(await _backend.SaveProjectCanvasAsync(graph).ConfigureAwait(true));

            await _backend.ApplyNodeDetailPatchAsync(CurrentWorkflowId, new NodeDetailPatch(
                SelectedNode.Id,
                SelectedNode.PromptTemplate,
                new Dictionary<string, string>(),
                new Dictionary<string, bool>(),
                new Dictionary<string, string>(),
                string.IsNullOrWhiteSpace(SelectedNode.ModelId) ? null : SelectedNode.ModelId,
                NodeTimeoutHelper.ParseNullableDouble(SelectedNode.BudgetUsd),
                NodeTimeoutHelper.ParseNullableLongMs(SelectedNode.TimeoutMs))).ConfigureAwait(true);

            CaptureSnapshot();
            StatusText = _displayNames.Text("ui.common.save");
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private async Task ToggleBreakpointAsync()
    {
        if (SelectedNode is null)
        {
            StatusText = NoNodeSelectedText;
            return;
        }
        if (!EnsureWorkflowLoadedForPersistence())
        {
            return;
        }
        try
        {
            await _backend.SetNodeBreakpointAsync(CurrentWorkflowId, SelectedNode.Id, SelectedNode.BreakpointEnabled).ConfigureAwait(true);
            StatusText = BreakpointText;
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private async Task AddAnnotationAsync()
    {
        if (!EnsureWorkflowLoadedForPersistence())
        {
            return;
        }
        var selected = SelectedNode is null ? Nodes.Select(node => node.Id).ToArray() : new[] { SelectedNode.Id };
        if (SelectedNode is null && selected.Length > 1
            && !await ConfirmAllNodesAsync("ui.dialog.workspace.annotate_all.message").ConfigureAwait(true))
        {
            return;
        }
        try
        {
            await _backend.UpsertCanvasAnnotationAsync(CurrentWorkflowId, new CanvasAnnotation(
                $"annotation-{DateTimeOffset.UtcNow.ToUnixTimeMilliseconds()}",
                string.IsNullOrWhiteSpace(AnnotationTitle) ? _displayNames.Text("ui.workspace.default_annotation_title") : AnnotationTitle,
                selected,
                new Dictionary<string, object?>())).ConfigureAwait(true);
            StatusText = _displayNames.Text("ui.workspace.annotation_saved");
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private async Task PackSelectionAsync()
    {
        if (!EnsureWorkflowLoadedForPersistence())
        {
            return;
        }
        var selected = SelectedNode is null ? Nodes.Select(node => node.Id).ToArray() : new[] { SelectedNode.Id };
        if (SelectedNode is null && selected.Length > 1
            && !await ConfirmDangerAsync(
                    "ui.dialog.workspace.pack_all.title",
                    "ui.dialog.workspace.pack_all.message",
                    "ui.dialog.workspace.pack_all.confirm").ConfigureAwait(true))
        {
            return;
        }
        var operationId = $"desktop-pack-{Guid.NewGuid():N}";
        var title = _displayNames.Format("ui.workspace.subworkflow_title", new Dictionary<string, string>
        {
            ["count"] = selected.Length.ToString(),
        });
        try
        {
            var report = await PackSelectionWithRecoveryAsync(
                CurrentWorkflowId,
                selected,
                null,
                title,
                _workflowContentRevision,
                operationId,
                _backend).ConfigureAwait(true);
            ApplyPackReport(report, selected.Length);
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    internal static async Task<WorkflowPackReport> PackSelectionWithRecoveryAsync(
        string workflowId,
        IReadOnlyList<string> selectedNodeIds,
        string? subworkflowNodeId,
        string? title,
        string? expectedRevision,
        string operationId,
        IAriadneBackendClient backend,
        CancellationToken cancellationToken = default)
    {
        try
        {
            return await backend.PackWorkflowSelectionAsync(
                workflowId,
                selectedNodeIds,
                subworkflowNodeId,
                title,
                expectedRevision,
                operationId,
                cancellationToken).ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            throw;
        }
        catch
        {
            try
            {
                return await backend.GetPackOperationAsync(operationId, cancellationToken).ConfigureAwait(true);
            }
            catch when (!cancellationToken.IsCancellationRequested)
            {
                return await backend.PackWorkflowSelectionAsync(
                    workflowId,
                    selectedNodeIds,
                    subworkflowNodeId,
                    title,
                    expectedRevision,
                    operationId,
                    cancellationToken).ConfigureAwait(true);
            }
        }
    }

    private void ApplyPackReport(WorkflowPackReport report, int selectedCount)
    {
        ApplyGraph(report.Workflow);
        CaptureSnapshot();
        StatusText = _displayNames.Format("ui.workspace.packed_selection", new Dictionary<string, string>
        {
            ["count"] = selectedCount.ToString(),
        });
    }

    private async Task LoadConfirmationsAsync(CancellationToken cancellationToken = default)
    {
        if (!_backend.HasProjectRoot)
        {
            Confirmations.Clear();
            ResolvedConfirmations.Clear();
            SelectedConfirmation = null;
            // 换项目/无项目时解除钉开：上一个项目的历史面板不该盖着新项目的画布。
            _isConfirmationHistoryPinnedOpen = false;
            OnPropertyChanged(nameof(HasPendingConfirmations));
            OnPropertyChanged(nameof(HasResolvedConfirmations));
            OnPropertyChanged(nameof(ConfirmationCountText));
            OnPropertyChanged(nameof(ConfirmationHistoryCountText));
            OnPropertyChanged(nameof(ConfirmationsBannerText));
            NotifyConfirmationPanelVisibility();
            OnPropertyChanged(nameof(EmptyStartTitle));
            OnPropertyChanged(nameof(EmptyStartHint));
            NotifyConfirmationCommandStates();
            return;
        }

        try
        {
            var entries = await _backend.ListConfirmationsAsync(cancellationToken).ConfigureAwait(true);
            cancellationToken.ThrowIfCancellationRequested();
            Confirmations.Clear();
            ResolvedConfirmations.Clear();
            SelectedConfirmation = null;
            // U187-A：后端 `list_confirmations` 从 7378ddc 起就合并了运行态 pending
            // **与 `confirmation_logs` 里的已决议历史**——这里必须按状态**分流**，
            // 不能过滤掉已决议项（此前那句「后端已只返回 pending」的注释在 7378ddc
            // 之后就过期了，而它正是缺陷的掩护：照它读，过滤看起来是安全的）。
            //
            // 为什么必须分两个集合、而不是「删掉过滤了事」：
            // `HasPendingConfirmations => Confirmations.Count > 0` 驱动审阅面板
            // **强制展开并替换整个画布**（见下方 IsConfirmationPanelExpanded 与
            // NotifyConfirmationPanelVisibility）。把 30 条历史混进同一个集合，
            // 那个条件就恒真，作者每次打开项目都被一个盖住画布的面板拦住、再也回不到画布
            // ——比「查不到历史」严重得多。
            // ⇒ Confirmations 只放 pending（面板展开 / badge 计数 / 逐项审批全部沿用它，
            //    语义一个字没变），已决议项进 ResolvedConfirmations，只作审计呈现。
            foreach (var entry in entries)
            {
                if (IsPendingConfirmation(entry))
                {
                    Confirmations.Add(new ConfirmationItemViewModel(entry, _displayNames, SelectConfirmation));
                }
                else
                {
                    ResolvedConfirmations.Add(new ResolvedConfirmationItemViewModel(entry, _displayNames));
                }
            }
            if (Confirmations.Count > 0 && SelectedConfirmation is null)
            {
                SelectConfirmation(Confirmations[0]);
            }
            OnPropertyChanged(nameof(HasPendingConfirmations));
            OnPropertyChanged(nameof(HasResolvedConfirmations));
            OnPropertyChanged(nameof(ConfirmationCountText));
            OnPropertyChanged(nameof(ConfirmationHistoryCountText));
            OnPropertyChanged(nameof(ConfirmationsBannerText));
            if (Confirmations.Count > 0)
            {
                IsConfirmationPanelExpanded = true;
            }
            // 必须在 IsConfirmationPanelExpanded 之后再通知一次：该属性**已是 true 时不会触发**
            // setter 里的通知（SetProperty 同值短路），而「有待审项」这件事本身刚刚才变真——
            // 少了这一次，首屏进入审阅态就不会自动切右栏（U139⑤）。
            NotifyConfirmationPanelVisibility();
            if (Confirmations.Count == 0)
            {
                StatusText = ConfirmationsEmptyText;
            }
            NotifyConfirmationCommandStates();
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private static bool IsPendingConfirmation(ConfirmationLogEntry entry)
    {
        return string.Equals(entry.State, "pending", StringComparison.OrdinalIgnoreCase);
    }

    private void SelectConfirmation(ConfirmationItemViewModel item)
    {
        foreach (var confirmation in Confirmations)
        {
            confirmation.IsSelected = confirmation == item;
        }
        SelectedConfirmation = item;
        NotifyConfirmationCommandStates();
    }

    private async Task ResolveSelectedConfirmationAsync(string decision)
    {
        if (SelectedConfirmation is null)
        {
            StatusText = ConfirmationsEmptyText;
            return;
        }
        var confirmation = SelectedConfirmation;
        var targetsCurrentWorkflow = string.IsNullOrWhiteSpace(confirmation.WorkflowId)
            || string.Equals(confirmation.WorkflowId, CurrentWorkflowId, StringComparison.Ordinal);
        var workflowId = !string.IsNullOrWhiteSpace(confirmation.WorkflowId)
            ? confirmation.WorkflowId
            : CurrentWorkflowId;
        var runId = !string.IsNullOrWhiteSpace(confirmation.RunId)
            ? confirmation.RunId
            : targetsCurrentWorkflow ? CurrentRunId : string.Empty;
        if (string.IsNullOrWhiteSpace(runId) || string.IsNullOrWhiteSpace(workflowId))
        {
            StatusText = _displayNames.Text("ui.workspace.confirmation.missing_run");
            return;
        }
        var sessionFence = _runSession.CaptureFence();
        // 拒绝不再弹危险对话框：确认闸口已由「展开理由线 → 第二次点击」承担。
        // 两道确认叠加只会让人闭眼连点，反而削弱那道闸口的意义。
        try
        {
            _runSession.ThrowIfStale(sessionFence);
            var result = await _backend.ResolveConfirmationAsync(
                workflowId,
                runId,
                confirmation.ConfirmationId,
                decision,
                string.IsNullOrWhiteSpace(ConfirmationReason) ? null : ConfirmationReason).ConfigureAwait(true);
            _runSession.ThrowIfStale(sessionFence);
            StatusText = UserFacingError.RuntimeStatus(result.Workflow.Status, _displayNames);
            // 已提交：收起理由线并清空理由，下一项从干净状态开始。
            DisarmReject();
            ConfirmationReason = string.Empty;
            if (string.Equals(workflowId, CurrentWorkflowId, StringComparison.Ordinal))
            {
                _runSession.Attach(
                    result.Workflow.WorkflowId,
                    result.Workflow.RunId,
                    result.Workflow.Status);
            }
            await LoadConfirmationsAsync().ConfigureAwait(true);
        }
        catch (OperationCanceledException)
        {
            // 确认请求属于旧会话；后端结果仍持久化，但不得覆盖当前页面会话。
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private Task<bool> ConfirmAllNodesAsync(string messageKey)
    {
        return ConfirmDialogAsync(
            "ui.dialog.workspace.all_nodes.title",
            messageKey,
            "ui.dialog.workspace.all_nodes.confirm",
            DialogButtonVariant.Primary);
    }

    private Task<bool> ConfirmDangerAsync(string titleKey, string messageKey, string confirmKey)
    {
        return ConfirmDialogAsync(titleKey, messageKey, confirmKey, DialogButtonVariant.Danger);
    }

    private async Task<bool> ConfirmDialogAsync(
        string titleKey,
        string messageKey,
        string confirmKey,
        DialogButtonVariant confirmVariant)
    {
        var dialog = new ConfirmDialogViewModel(
            _displayNames.Text(titleKey),
            _displayNames.Text(messageKey),
            new[]
            {
                new DialogButton(_displayNames.Text(confirmKey), confirmVariant, 0),
                new DialogButton(_displayNames.Text("ui.common.cancel"), DialogButtonVariant.Subtle, 1),
            })
        {
            Severity = confirmVariant == DialogButtonVariant.Danger
                ? DialogSeverity.Danger
                : DialogSeverity.Warning,
            CancelResultIndex = 1,
            ConfirmResultIndex = 0,
        };
        return await DialogService.Current.ConfirmAsync(dialog).ConfigureAwait(true) == 0;
    }

    public string UnsavedChangesPageTitle => Title;
    public string UnsavedChangesPageId => "workspace";
    public string? PreparedUnsavedChangesPayloadIdentity => _preparedLeaveSnapshot is null
        ? null
        : BatchLeaveSaveCoordinator.CreatePayloadIdentity(_preparedLeaveSnapshot);

    private bool _leavePrepared;
    private WorkflowGraphData? _preparedLeaveGraph;
    private string? _preparedLeaveSnapshot;

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

    public async Task<bool> PrepareUnsavedChangesAsync()
    {
        if (!HasUnsavedChanges)
        {
            _leavePrepared = true;
            _preparedLeaveGraph = null;
            _preparedLeaveSnapshot = null;
            return true;
        }

        if (!EnsureWorkflowLoadedForPersistence())
        {
            _leavePrepared = false;
            return false;
        }

        try
        {
            var graph = BuildGraph();
            await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
            _preparedLeaveGraph = graph;
            _preparedLeaveSnapshot = SnapshotGraph(graph);
            _leavePrepared = true;
            return true;
        }
        catch
        {
            _leavePrepared = false;
            _preparedLeaveGraph = null;
            _preparedLeaveSnapshot = null;
            return false;
        }
    }

    public async Task<bool> CommitPreparedUnsavedChangesAsync()
    {
        if (!_leavePrepared)
        {
            return false;
        }

        if (!HasUnsavedChanges || _preparedLeaveGraph is null || _preparedLeaveSnapshot is null)
        {
            _leavePrepared = false;
            _preparedLeaveGraph = null;
            _preparedLeaveSnapshot = null;
            return true;
        }

        var preparedGraph = _preparedLeaveGraph;
        var preparedSnapshot = _preparedLeaveSnapshot;
        if (!string.Equals(CurrentContentSnapshot(), preparedSnapshot, StringComparison.Ordinal))
        {
            _leavePrepared = false;
            _preparedLeaveGraph = null;
            _preparedLeaveSnapshot = null;
            return false;
        }

        try
        {
            var saved = await _backend.SaveProjectCanvasAsync(preparedGraph).ConfigureAwait(true);
            RememberWorkflowRevision(saved);
            AcceptSavedGraph(preparedGraph);
            _leavePrepared = false;
            _preparedLeaveGraph = null;
            _preparedLeaveSnapshot = null;
            return !HasUnsavedChanges;
        }
        catch
        {
            return false;
        }
    }

    public Task AbortPreparedUnsavedChangesAsync()
    {
        _leavePrepared = false;
        _preparedLeaveGraph = null;
        _preparedLeaveSnapshot = null;
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
        _runSession.Reset();
        await _projectAutomation.EnsureLoadedAsync(cancellationToken).ConfigureAwait(true);
        cancellationToken.ThrowIfCancellationRequested();
        await InitializeWorkflowAsync(cancellationToken).ConfigureAwait(true);
        cancellationToken.ThrowIfCancellationRequested();
        await LoadConfirmationsAsync(cancellationToken).ConfigureAwait(true);
        cancellationToken.ThrowIfCancellationRequested();
        await LoadAvailableModelsAsync(cancellationToken).ConfigureAwait(true);
        cancellationToken.ThrowIfCancellationRequested();
        await LoadSummarizerChapterOptionsAsync(cancellationToken).ConfigureAwait(true);
    }

    public void DeactivateProjectData()
    {
        _canvasLoading.Invalidate();
        // U194-C：原先这里是 `_runSession.CancelPolling()` ⇒ **一离开画布页轮询就停**，
        // 作者启动一个几十分钟的工作流再切去写作，跑完/失败/停下都毫无提示。
        // 改成降级为「只查终态」的后台监视：重的事件轮询（750ms 拉 100 条事件）
        // 在别的页面上确实没有消费者，该停；但「跑完了吗」这一个比特必须继续问。
        // 已在终态或压根没在跑时它会自行退化成 CancelPolling，不留空转任务。
        _runSession.WatchTerminalStateInBackground();
    }

    private bool CanPersistWorkflow()
    {
        return WorkflowLoadGuard.CanPersist(_backend.HasProjectRoot, _workflowLoadState);
    }

    private bool EnsureWorkflowLoadedForPersistence()
    {
        if (CanPersistWorkflow())
        {
            return true;
        }

        StatusText = _displayNames.Text("ui.workspace.load_required_before_save");
        return false;
    }

    private void SetWorkflowLoadState(WorkflowLoadState state)
    {
        if (_workflowLoadState == state)
        {
            return;
        }

        _workflowLoadState = state;
        SaveCommand.NotifyCanExecuteChanged();
        RunSelectedNodeCommand.NotifyCanExecuteChanged();
        // U207-C①：「运行」与起始节点卡上的三角同源同门禁，加载态变了要一起刷新。
        RunWorkflowCommand.NotifyCanExecuteChanged();
        OnPropertyChanged(nameof(RunEntryTooltip));
        foreach (var node in Nodes)
        {
            node.NotifyRunCommandState();
        }
    }

    private WorkflowGraphData BuildGraph()
    {
        return new WorkflowGraphData(
            CurrentWorkflowId,
            _projectCanvasName,
            Nodes.Select(node => new CanvasNode(
                node.Id,
                node.NodeType,
                // 作者改的是 Name；Label 是构造时类型默认文案，不能原样写回
                string.IsNullOrWhiteSpace(node.Name) ? node.Label : node.Name,
                node.ToData(),
                new CanvasPosition(node.X, node.Y))).ToArray(),
            Edges.Select(edge => edge.ToCanvasEdge()).ToArray(),
            new Dictionary<string, object?>(_canvasMetadata, StringComparer.Ordinal),
            ContentRevision: null,
            ExpectedRevision: _workflowContentRevision);
    }

    private void RememberWorkflowRevision(WorkflowGraphData? graph)
    {
        _workflowContentRevision = graph?.ContentRevision;
    }

    private void ApplyGraph(WorkflowGraphData graph)
    {
        RememberWorkflowRevision(graph);
        _projectCanvasName = string.IsNullOrWhiteSpace(graph.Name) ? "Project Canvas" : graph.Name;
        _canvasMetadata = new Dictionary<string, object?>(graph.Metadata, StringComparer.Ordinal);
        _isApplyingGraph = true;
        _suppressSnapshotChecks = true;
        try
        {
            Nodes.Clear();
            SelectedNode = null;
            _edges = graph.Edges.ToArray();
            Edges.Clear();
            foreach (var edge in _edges)
            {
                Edges.Add(new WorkflowEdgeViewModel(edge, _displayNames, SelectEdge, RefreshDirtyState));
            }
            OnPropertyChanged(nameof(EdgeCountText));
            foreach (var graphNode in graph.Nodes)
            {
                var node = CreateNodeFromCanvas(graphNode);
                Nodes.Add(node);
            }
            RefreshEdgeLabels();
            RefreshStartNodes();
            RefreshPortConnectionStates();
            RefreshRelatedEdges();
            _nextNodeNumber = Math.Max(_nextNodeNumber, Nodes.Count + 1);
        }
        finally
        {
            _isApplyingGraph = false;
            _suppressSnapshotChecks = false;
            GraphRevision++;
            OnPropertyChanged(nameof(GraphRevision));
        }
    }

    internal bool IsApplyingGraph => _isApplyingGraph;
    internal int GraphRevision { get; private set; }

    /// <summary>供发布性能探针注入合成图；仍复用正式画布图 DTO 和节点构造路径。</summary>
    internal void LoadReleaseProbeGraph(WorkflowGraphData graph)
    {
        ApplyGraph(graph);
        CaptureSnapshot();
        SetWorkflowLoadState(WorkflowLoadState.Loaded);
    }

    private void CaptureSnapshot()
    {
        _savedSnapshot = CurrentSnapshot();
        _savedContentSnapshot = CurrentContentSnapshot();
        _undoSnapshots.Clear();
        _redoSnapshots.Clear();
        NotifyHistoryCommands();
        HasUnsavedChanges = false;
    }

    private void AcceptSavedGraph(WorkflowGraphData submitted)
    {
        var baseline = submitted with
        {
            ContentRevision = _workflowContentRevision,
            ExpectedRevision = _workflowContentRevision,
        };
        _savedSnapshot = JsonSerializer.Serialize(baseline, JsonOptions);
        _savedContentSnapshot = SnapshotGraph(baseline);
        var unchanged = string.Equals(CurrentContentSnapshot(), _savedContentSnapshot, StringComparison.Ordinal);
        if (unchanged)
        {
            _undoSnapshots.Clear();
            _redoSnapshots.Clear();
            NotifyHistoryCommands();
        }
        HasUnsavedChanges = !unchanged;
    }

    private void RestoreSnapshot()
    {
        try
        {
            var graph = JsonSerializer.Deserialize<WorkflowGraphData>(_savedSnapshot, JsonOptions);
            if (graph is not null)
            {
                ApplyGraph(graph);
            }
            HasUnsavedChanges = false;
        }
        catch
        {
            HasUnsavedChanges = false;
        }
    }

    private void RefreshDirtyState()
    {
        // C5-b：连续拖动期间 defer 昂贵 snapshot；松手 EndContinuousCanvasEdit 再算。
        if (_deferDirtyRefresh)
        {
            return;
        }
        if (!_suppressSnapshotChecks)
        {
            try
            {
                HasUnsavedChanges = CurrentContentSnapshot() != _savedContentSnapshot;
            }
            catch
            {
                HasUnsavedChanges = true;
            }
        }
    }

    public void BeginContinuousCanvasEdit()
    {
        // C5-b：拖动期间 defer snapshot 对比；不在 Begin 时清掉已有脏标记。
        _deferDirtyRefresh = true;
    }

    public void EndContinuousCanvasEdit()
    {
        _deferDirtyRefresh = false;
        // C5-b：拖动只改 X/Y 时 setter 不再 RefreshDirty；松手必须按最终坐标重算 dirty。
        // 零位移时 CurrentSnapshot==_savedSnapshot → HasUnsavedChanges 仍为 false。
        if (CanvasDragFrameHelpers.MustRefreshDirtyAfterContinuousEditEnd)
        {
            RefreshDirtyState();
        }
    }

    private string CurrentSnapshot()
    {
        return JsonSerializer.Serialize(BuildGraph(), JsonOptions);
    }

    private string CurrentContentSnapshot() => SnapshotGraph(BuildGraph());

    private static string SnapshotGraph(WorkflowGraphData graph)
    {
        return JsonSerializer.Serialize(
            graph with { ContentRevision = null, ExpectedRevision = null },
            JsonOptions);
    }

    private const string CurrentWorkflowId = DefaultWorkflowId;

    internal string LoadedWorkflowIdForTests => CurrentWorkflowId;

    internal string? WorkflowContentRevisionForTests => _workflowContentRevision;

    /// <summary>
    /// 测试用：把页面推到「有一个运行在跑」的状态，不真起一次工作流。
    ///
    /// 存在理由是 U194-C 的接线判据：`DeactivateProjectData()` 必须把轮询**降级**成
    /// 后台终态监视而不是掐掉，而验证那一下需要先有个非终态的运行挂着。
    /// 走真实运行会把整条工作流加载/后端回包都拉进来，失败面与本条要守的性质无关。
    /// </summary>
    internal void AttachRunForTests(string workflowId, string runId, string status) =>
        _runSession.Attach(workflowId, runId, status, resetCursor: true);

    private string NodeLabel(string nodeType)
    {
        var entry = WorkflowNodeCatalog.FindKnown(nodeType);
        return entry is null ? nodeType : _displayNames.Text(entry.DisplayNameKey);
    }

    private string NextNodeId(string nodeType)
    {
        string id;
        do
        {
            id = $"{nodeType}-{_nextNodeNumber++}";
        }
        while (Nodes.Any(node => node.Id == id));
        return id;
    }

    private static string ReadString(Dictionary<string, object?> data, string key, string fallback = "")
    {
        if (!data.TryGetValue(key, out var value) || value is null)
        {
            return fallback;
        }
        if (value is JsonElement element)
        {
            return element.ValueKind switch
            {
                JsonValueKind.String => element.GetString() ?? fallback,
                JsonValueKind.Number => element.ToString(),
                JsonValueKind.True => "true",
                JsonValueKind.False => "false",
                _ => fallback,
            };
        }
        return value.ToString() ?? fallback;
    }

    private static bool ReadBool(Dictionary<string, object?> data, string key, bool fallback)
    {
        if (!data.TryGetValue(key, out var value) || value is null)
        {
            return fallback;
        }
        if (value is JsonElement element)
        {
            return element.ValueKind == JsonValueKind.True || (element.ValueKind == JsonValueKind.False ? false : fallback);
        }
        return value is bool boolean ? boolean : fallback;
    }



    /// <summary>
    /// 解析 start 节点上的变量声明。
    ///
    /// 走 JsonElement 而非强类型反序列化：节点 data 是 opaque 字典，
    /// 未知键要原样保留（见 NodeConfigData.CaptureExtra），这里只读不写。
    /// 解析不出来的条目直接跳过——执行页少一行，比拿半个声明去启动安全。
    /// </summary>
    private static IReadOnlyList<WorkflowVariableDeclaration> ReadVariableDeclarations(
        Dictionary<string, object?> data)
    {
        if (!data.TryGetValue("variables", out var raw) || raw is null)
        {
            return Array.Empty<WorkflowVariableDeclaration>();
        }

        if (raw is not JsonElement element || element.ValueKind != JsonValueKind.Array)
        {
            return Array.Empty<WorkflowVariableDeclaration>();
        }

        var declarations = new List<WorkflowVariableDeclaration>();
        foreach (var item in element.EnumerateArray())
        {
            if (item.ValueKind != JsonValueKind.Object)
            {
                continue;
            }

            var name = item.TryGetProperty("name", out var nameElement)
                ? nameElement.GetString() ?? string.Empty
                : string.Empty;
            if (string.IsNullOrWhiteSpace(name))
            {
                continue;
            }

            var kind = item.TryGetProperty("kind", out var kindElement)
                ? kindElement.GetString() ?? WorkflowVariableRules.KindString
                : WorkflowVariableRules.KindString;

            object? defaultValue = null;
            if (item.TryGetProperty("default", out var defaultElement))
            {
                defaultValue = defaultElement.ValueKind switch
                {
                    JsonValueKind.String => defaultElement.GetString(),
                    JsonValueKind.Number => defaultElement.GetDouble(),
                    JsonValueKind.True => true,
                    JsonValueKind.False => false,
                    _ => null,
                };
            }

            var required = item.TryGetProperty("required", out var requiredElement)
                && requiredElement.ValueKind == JsonValueKind.True;
            var hidden = item.TryGetProperty("hidden", out var hiddenElement)
                && hiddenElement.ValueKind == JsonValueKind.True;

            declarations.Add(new WorkflowVariableDeclaration(name, kind, defaultValue, required, hidden));
        }

        return declarations;
    }

    private void SelectEdge(WorkflowEdgeViewModel edge)
    {
        foreach (var item in Edges)
        {
            item.IsSelected = item == edge;
        }
        SelectedEdge = edge;
        SetRightPanelTab(WorkspaceRightPanelTab.EdgeDetails);
        IsRightPanelOpen = true;

        // 点边时同步选中一个端点节点，右栏才显示「相关边」配置（符合直觉）
        var prefer = SelectedNode is not null
                     && CanvasSelectionHelpers.EdgeTouchesNode(edge.Source, edge.Target, SelectedNode.Id)
            ? SelectedNode
            : Nodes.FirstOrDefault(n => n.Id == edge.Source)
              ?? Nodes.FirstOrDefault(n => n.Id == edge.Target);

        if (prefer is not null)
        {
            foreach (var item in Nodes)
            {
                item.IsSelected = item == prefer;
            }

            SelectedNode = prefer;
        }

        RefreshRelatedEdges();
        NotifySelectionCommands();
    }

    private void SetRightPanelTab(WorkspaceRightPanelTab tab)
    {
        if (_rightPanelTab == tab)
        {
            return;
        }

        _rightPanelTab = tab;
        OnPropertyChanged(nameof(IsProjectAiTab));
        OnPropertyChanged(nameof(IsNodeDetailsTab));
        OnPropertyChanged(nameof(IsEdgeDetailsTab));
    }

    private enum WorkspaceRightPanelTab
    {
        ProjectAi,
        NodeDetails,
        EdgeDetails,
    }

    private void SaveSelectedEdgeConfig()
    {
        if (SelectedEdge is null)
        {
            StatusText = _displayNames.Text("ui.common.none");
            return;
        }
        try
        {
            CaptureUndoSnapshot();
            _edges = Edges.Select(edge => edge.ToCanvasEdge()).ToArray();
            RefreshDirtyState();
            StatusText = EdgeDetailsText;
        }
        catch (Exception ex)
        {
            StatusText = ReportFailure(ex, _displayNames);
        }
    }

    private void InsertForwardTemplateVariable()
    {
        if (SelectedEdge?.IsCommunication != true)
        {
            return;
        }
        SelectedEdge.ForwardTemplate = AppendTemplateVariable(SelectedEdge.ForwardTemplate, "{{input.forward_output}}");
    }

    private void InsertReverseTemplateVariable()
    {
        if (SelectedEdge?.IsCommunication != true)
        {
            return;
        }
        SelectedEdge.ReverseTemplate = AppendTemplateVariable(SelectedEdge.ReverseTemplate, "{{input.reverse_output}}");
    }

    private static string AppendTemplateVariable(string template, string variable)
    {
        if (template.Contains(variable, StringComparison.Ordinal))
        {
            return template;
        }
        return string.IsNullOrWhiteSpace(template) ? variable : $"{template.TrimEnd()}\n{variable}";
    }

    private void RefreshStartNodes()
    {
        StartNodes.Clear();
        foreach (var node in Nodes.Where(node => node.IsStartNode))
        {
            StartNodes.Add(node);
        }
        OnPropertyChanged(nameof(HasStartNodes));
        OnPropertyChanged(nameof(HasNodes));
        OnPropertyChanged(nameof(EmptyCanvasTitle));
        OnPropertyChanged(nameof(EmptyCanvasHint));
        // U207-C①：画上第一个起始节点后「运行」要立刻可点，删光后要立刻变回禁用，
        // 悬停说明也要跟着从「先拖一个开始节点」换成「从起始节点运行」。
        RunWorkflowCommand.NotifyCanExecuteChanged();
        OnPropertyChanged(nameof(RunEntryTooltip));
        // 起始节点集合变了，Ctrl+K 有无可填目标随之变化——不通知的话
        // 新画上第一个起始节点后按 Ctrl+K 仍然「没反应」。
        OpenVariableFillCommand.NotifyCanExecuteChanged();
        RefreshPortConnectionStates();
    }

    /// <summary>
    /// Ctrl+K 这次要填哪个起始节点的变量。
    ///
    /// 优先当前选中的起始节点（作者的注意力就在它上面）；没选中时取第一个
    /// 带变量的起始节点——单起点画布是常态，让他为了填值先去点一下节点是多余步骤。
    /// 一个带变量的都没有则返回 null，命令随之禁用。
    /// </summary>
    private WorkflowVariableGroupViewModel? ResolveVariableFillTarget()
    {
        if (SelectedNode is { IsStartNode: true, Variables: { HasVariables: true } selected })
        {
            return selected;
        }

        return StartNodes
            .Select(node => node.Variables)
            .FirstOrDefault(group => group is { HasVariables: true });
    }

    /// <summary>
    /// 把「填变量值」的请求发给项目空间 AI。
    ///
    /// 走既有 `project_ai_chat`，因此**沿用当前对话上下文**——作者刚在对话里交代过
    /// 「这一章写雪灾」，填值时它能用上。这是 13C 对本项的明确要求。
    ///
    /// 刻意不落进对话气泡、也不推进 conversation revision：这是一次工具性取值，
    /// 不是一轮对话，塞进历史会污染后续轮次的语义（与句式生成同一取舍）。
    /// </summary>
    private async Task<string> FillVariablesWithProjectAiAsync(string message)
    {
        var sessionFence = _runSession.CaptureFence();
        var result = await _backend.ProjectAiChatAsync(
            message,
            workflowIdToRun: null,
            referenceWorkflowId: CurrentWorkflowId,
            referenceRunId: string.IsNullOrWhiteSpace(CurrentRunId) ? null : CurrentRunId,
            conversationId: ProjectAiConversationId,
            conversationRevision: _projectAiConversationRevision)
            .ConfigureAwait(true);
        _runSession.ThrowIfStale(sessionFence);
        return result.Answer;
    }

    /// <summary>按边集合刷新各节点引脚「已连接=实心 / 未连接=空心」。</summary>
    private void RefreshPortConnectionStates()
    {
        foreach (var node in Nodes)
        {
            var controlIn = false;
            var controlOut = false;
            var dataIn = false;
            var dataOut = false;
            var communication = false;
            foreach (var edge in Edges)
            {
                if (edge.Source == node.Id)
                {
                    if (NodePortSpec.TryResolveKind(edge.SourceHandle, out var kind, out _))
                    {
                        switch (kind)
                        {
                            case NodePortKind.Control: controlOut = true; break;
                            case NodePortKind.Data: dataOut = true; break;
                            case NodePortKind.Communication: communication = true; break;
                        }
                    }
                    if (string.Equals(edge.Kind, "communication", StringComparison.OrdinalIgnoreCase))
                    {
                        communication = true;
                    }
                }
                if (edge.Target == node.Id)
                {
                    if (NodePortSpec.TryResolveKind(edge.TargetHandle, out var kind, out _))
                    {
                        switch (kind)
                        {
                            case NodePortKind.Control: controlIn = true; break;
                            case NodePortKind.Data: dataIn = true; break;
                            case NodePortKind.Communication: communication = true; break;
                        }
                    }
                    if (string.Equals(edge.Kind, "communication", StringComparison.OrdinalIgnoreCase))
                    {
                        communication = true;
                    }
                }
            }
            node.SetPortConnected(controlIn, controlOut, dataIn, dataOut, communication);
            // U125：两个分支引脚各自判定，不能共用上面的 controlOut——
            // 只连了真分支时，假分支不应被画成已连接。
            node.SetBranchPortConnected(
                trueBranch: Edges.Any(edge => edge.Source == node.Id
                    && string.Equals(edge.SourceHandle, NodePortSpec.ExecOutTrueHandle, StringComparison.OrdinalIgnoreCase)),
                falseBranch: Edges.Any(edge => edge.Source == node.Id
                    && string.Equals(edge.SourceHandle, NodePortSpec.ExecOutFalseHandle, StringComparison.OrdinalIgnoreCase)));
            // 各数据入是否已占用
            foreach (var pin in node.DataInPins)
            {
                pin.IsConnected = Edges.Any(e =>
                    string.Equals(e.Kind, "data", StringComparison.OrdinalIgnoreCase)
                    && e.Target == node.Id
                    && string.Equals(e.TargetHandle, pin.Handle, StringComparison.OrdinalIgnoreCase));
            }
        }
    }

    /// <summary>删除节点上的数据入时，同步拆掉占用该 handle 的边。</summary>
    public void OnDataInPinRemoved(WorkflowNodeViewModel node, string handle)
    {
        var doomed = Edges
            .Where(e => e.Target == node.Id
                        && string.Equals(e.TargetHandle, handle, StringComparison.OrdinalIgnoreCase))
            .ToArray();
        if (doomed.Length == 0)
        {
            RefreshPortConnectionStates();
            return;
        }

        CaptureUndoSnapshot();
        foreach (var edge in doomed)
        {
            Edges.Remove(edge);
        }

        _edges = Edges.Select(e => e.ToCanvasEdge()).ToArray();
        if (SelectedEdge is not null && doomed.Any(d => d.Id == SelectedEdge.Id))
        {
            SelectedEdge = null;
        }

        RefreshRelatedEdges();
        RefreshPortConnectionStates();
        RefreshDirtyState();
        OnPropertyChanged(nameof(EdgeCountText));
    }
}

public sealed record WorkflowModelOption(
    string ProviderId,
    string ModelId,
    string ProviderDisplayName,
    string? OverrideDisplayName = null,
    string? AliasId = null)
{
    public string DisplayName => OverrideDisplayName ?? $"{ProviderDisplayName} · {ModelId}";

    public bool IsAlias => !string.IsNullOrWhiteSpace(AliasId);

    public bool IsInherited => !IsAlias
                               && string.IsNullOrWhiteSpace(ProviderId)
                               && string.IsNullOrWhiteSpace(ModelId);

    public static WorkflowModelOption Inherited(string displayName) =>
        new(string.Empty, string.Empty, string.Empty, displayName);

    public static WorkflowModelOption Unconfigured(string displayName) =>
        new(string.Empty, string.Empty, string.Empty, displayName);

    public static WorkflowModelOption Alias(string aliasId, string displayName) =>
        new(string.Empty, string.Empty, string.Empty, displayName, aliasId);
}

public sealed record SummarizerChapterOption(
    string ChapterId,
    string DocumentId,
    string Title,
    string Path)
{
    public string DisplayTitle => string.IsNullOrWhiteSpace(Path)
        ? Title
        : $"{Title} · {Path.Replace('\\', '/')}";
}

public sealed class NodeLibraryItemViewModel : ViewModelBase
{
    private string _title;

    public NodeLibraryItemViewModel(string nodeType, string title, Action add)
    {
        NodeType = nodeType;
        _title = title;
        AddCommand = new RelayCommand(add);
    }

    public string NodeType { get; }
    public string Title { get => _title; set => SetProperty(ref _title, value); }
    public RelayCommand AddCommand { get; }
}

/// <summary>节点上的动态数据入口引脚。</summary>
public sealed class NodeDataInPinViewModel : ViewModelBase
{
    private bool _isConnected;

    public NodeDataInPinViewModel(string handle, string shortLabel, Action remove)
    {
        Handle = handle;
        ShortLabel = shortLabel;
        // Tag: data|in|handle 供拖线解析
        PortTag = $"data|in|{handle}";
        RemoveCommand = new RelayCommand(remove);
    }

    public string Handle { get; }
    public string ShortLabel { get; }
    public string PortTag { get; }
    public RelayCommand RemoveCommand { get; }

    public bool IsConnected
    {
        get => _isConnected;
        set => SetProperty(ref _isConnected, value);
    }
}

/// <summary>画布端口语义类型；拖线时仅同类可连。</summary>
public enum NodePortKind
{
    Data,
    Control,
    Communication,
}

/// <summary>端口方向；通信口为 Both，支持双向拖线。</summary>
public enum NodePortDirection
{
    In,
    Out,
    Both,
}

/// <summary>节点上的可视化端口定义，与后端 exec/data/communication 引脚对齐。</summary>
public static class NodePortSpec
{
    /// <summary>节点外框宽（单卡片，引脚在内侧）。与 WorkspacePageView 节点模板一致。
    /// 232 而非 200：200 宽时内容栏「数据入引脚列 + 加号 + 正文 + 数据出引脚」挤不开，
    /// 加号会被裁掉一半。</summary>
    public const double NodeWidth = 232;
    /// <summary>
    /// 节点最小高度 = 单个数据入引脚所需高度（首个引脚中心 + 底部安全间距）。
    /// 原先写死 96，但几何修正后单引脚实际需要 98，写死值会比真实需要还矮，
    /// 使「最小高」这个概念自相矛盾。改为由同一套几何派生，与
    /// NodeHeightForDataInputCount(1) 恒等。
    /// </summary>
    public const double MinimumNodeHeight = DataPortY + DataPortBottomInset;

    // ==================================================================
    // 以下常量必须与 WorkspacePageView.axaml 的节点模板逐项对应。
    // 一旦模板改了 padding / 引脚尺寸而这里没跟着改，连线端点就会与引脚
    // 错开若干像素（表现为「节点边偏移」）—— 这正是加宽到 232 那次留下的
    // 回归：模板把标题栏 padding 由 6,7 改成 8,8、执行引脚由 14 改成 16，
    // 而这里仍按旧值算。因此改为由「有名字的组成部分」推导，不再写死结果，
    // 并由 NodeEdgeGeometryTests 守住模板与常量的一致性。
    // ==================================================================

    /// <summary>卡片边框粗细（node-card BorderThickness）。</summary>
    public const double CardBorderThickness = 1;
    /// <summary>卡片顶缘 1px 受光亮线（DockPanel.Dock=Top 的玻璃高光）。</summary>
    public const double CardTopLightLine = 1;
    /// <summary>标题栏内边距（Border Padding="8,8"）。</summary>
    public const double TitleBarPadding = 8;
    /// <summary>执行引脚外框边长（标题栏两端的 pin-glass-soft，16x16）。</summary>
    public const double ExecPinBox = 16;
    /// <summary>内容栏左右内边距（Border Padding="6,8" 的水平分量）。</summary>
    public const double ContentBarPaddingX = 6;

    /// <summary>
    /// 内侧引脚中心到节点左右边的内缩。
    /// 执行引脚位于标题栏内：卡片边框 1 + 标题栏 padding 8 + 半个引脚 8 = 17。
    /// </summary>
    public const double PinInsetX = CardBorderThickness + TitleBarPadding + (ExecPinBox / 2.0);
    /// <summary>通信口中心 Y：顶行 10px 内、半出卡片上沿。</summary>
    public const double CommPortY = 7;
    /// <summary>卡片上沿（通信行高度）。</summary>
    public const double CardTopOffset = 10;
    /// <summary>
    /// 标题行内容高度。模板给标题行设了固定高度，使起始节点（含「运行」按钮，
    /// 原本 20px 行高）与普通节点（16px）保持一致——否则两类节点的标题栏差 4px，
    /// 数据口 Y 就无法用单个常量表达。
    /// </summary>
    public const double TitleRowHeight = 20;
    /// <summary>标题栏总高 = 上下 padding + 标题行高。</summary>
    public const double TitleBarHeight = (TitleBarPadding * 2) + TitleRowHeight;
    /// <summary>内容栏上 padding。</summary>
    public const double ContentBarPaddingY = 8;
    /// <summary>数据引脚外框边长。</summary>
    public const double DataPinBox = 14;
    /// <summary>多数据入垂直间距 = pin 高 14 + StackPanel Spacing 8。</summary>
    public const double DataPortGap = 8;
    public const double DataPortSpacing = DataPinBox + DataPortGap; // 22
    /// <summary>最后一个数据入中心到节点底边的安全间距，包含列表下方无框添加按钮。</summary>
    public const double DataPortBottomInset = 35;
    /// <summary>
    /// 执行口中心 Y = 通信行 + 卡片边框 + 顶缘光边 + 标题栏 pad-top + 半个引脚。
    /// </summary>
    public const double ExecPortY =
        CardTopOffset + CardBorderThickness + CardTopLightLine + TitleBarPadding + (ExecPinBox / 2.0);
    /// <summary>
    /// 首个数据口中心 Y：卡片顶 + 边框 + 顶缘光边 + 标题栏 + 内容 pad-top + 半 pin。
    /// 布局：内容栏 VerticalAlignment=Top，ItemsControl Spacing=DataPortGap。
    /// </summary>
    public const double DataPortY = CardTopOffset + CardBorderThickness + CardTopLightLine
        + TitleBarHeight + ContentBarPaddingY + (DataPinBox / 2.0);
    /// <summary>
    /// 数据引脚中心到节点左右边的内缩：卡片边框 1 + 内容栏 padding 6 + 半个 pin 7 = 14。
    /// 与执行引脚（在标题栏内，padding 8 + 半宽 8）不同，不能共用 PinInsetX。
    /// </summary>
    public const double DataPinInsetX = CardBorderThickness + ContentBarPaddingX + (DataPinBox / 2.0);
    public const double HitRadius = 16;
    public const double MiniMapContentWidth = CanvasMiniMapHelpers.ContentWidth;
    public const double MiniMapContentHeight = CanvasMiniMapHelpers.ContentHeight;

    public static string HandleName(NodePortKind kind, NodePortDirection direction) => kind switch
    {
        NodePortKind.Control => direction == NodePortDirection.In ? "exec_in" : "exec_out",
        NodePortKind.Communication => "communication",
        _ => direction == NodePortDirection.In ? "input" : "output",
    };

    /// <summary>U125：condition 的「条件成立」执行出引脚。与后端常量同名。</summary>
    public const string ExecOutTrueHandle = "exec_out_true";
    /// <summary>U125：condition 的「条件不成立」执行出引脚。</summary>
    public const string ExecOutFalseHandle = "exec_out_false";

    /// <summary>
    /// U145：源引脚名的完整取值集合。
    ///
    /// 这些名字**写死在节点类型定义里**（后端 `contracts/workflow.rs` 的
    /// `EXECUTION_OUTPUT_PORT` / `_TRUE` / `_FALSE` / `COMMUNICATION_PORT`），
    /// 用户无从知道该填什么；此前边检查器却给了个自由文本框。
    /// 顺序按「最常用在前」：普通数据出 → 执行出 → 分支出 → 通信。
    /// </summary>
    public static IReadOnlyList<string> SourceHandleNames() => new[]
    {
        HandleName(NodePortKind.Data, NodePortDirection.Out),
        HandleName(NodePortKind.Control, NodePortDirection.Out),
        ExecOutTrueHandle,
        ExecOutFalseHandle,
        HandleName(NodePortKind.Communication, NodePortDirection.Out),
    };

    /// <summary>
    /// U145：目标引脚名的类型级取值集合（首个数据入 + 执行入 + 通信）。
    ///
    /// 多数据入的 `data-in-N` **不在这里**：它取决于目标节点当前有几个引脚，
    /// 由调用方按目标节点的 `DataInPins` 补进候选前段。
    /// </summary>
    public static IReadOnlyList<string> TargetHandleNames() => new[]
    {
        HandleName(NodePortKind.Data, NodePortDirection.In),
        HandleName(NodePortKind.Control, NodePortDirection.In),
        HandleName(NodePortKind.Communication, NodePortDirection.In),
    };

    /// <summary>U145：通信边的约定别名（与 DefaultCommunicationData 同源）。</summary>
    public static IReadOnlyList<string> CommunicationAliasNames() => new[]
    {
        "forward_output",
        "reverse_output",
    };

    /// <summary>
    /// U145：各类节点对数据别名的约定默认值。
    ///
    /// 与 `SeedUtilityDefaults` 是同一组字面量：search 用 `query`、condition 用
    /// `input`、loop 用 `done`、summarizer 用 `chapter_text`。它们是「留空时后端会
    /// 采用的值」，因此也正是用户最可能想填的值。
    /// </summary>
    public static IReadOnlyList<string> ConventionalAliasNames() => new[]
    {
        "input",
        "query",
        "done",
        "chapter_text",
        "output",
    };

    /// <summary>
    /// U125：两个分支引脚相对标题行中线的垂直错开量（真在上、假在下，等距对称）。
    ///
    /// 取 (标题栏总高 - 引脚) / 2 = 10：这是**不越界的最大间距**，两脚中心相距 20、
    /// 16px 引脚之间留 4px 净空，包络 36 正好等于标题栏总高。
    /// 原来取 +1（间距 18、净空 2px）时两脚几乎贴成一坨；再往外拉到 +3 则包络 38
    /// 溢出标题栏、会撞上顶部通信口——BranchPins_StayWithinTitleBar 守的就是这条。
    /// </summary>
    public const double BranchPinOffsetY = (TitleBarHeight - ExecPinBox) / 2.0;

    /// <summary>U125：分支引脚名 → 相对通用执行出口中心的 Y 偏移（真在上、假在下）。</summary>
    public static double BranchPinOffsetFor(string? handle) => (handle ?? string.Empty).Trim() switch
    {
        ExecOutTrueHandle => -BranchPinOffsetY,
        ExecOutFalseHandle => BranchPinOffsetY,
        _ => 0,
    };

    /// <summary>U125：是否为 condition 的分支执行出引脚。</summary>
    public static bool IsBranchExecOutHandle(string? handle)
    {
        var name = (handle ?? string.Empty).Trim();
        return string.Equals(name, ExecOutTrueHandle, StringComparison.OrdinalIgnoreCase)
               || string.Equals(name, ExecOutFalseHandle, StringComparison.OrdinalIgnoreCase);
    }

    public static string EdgeKindName(NodePortKind kind) => kind switch
    {
        NodePortKind.Control => "control",
        NodePortKind.Communication => "communication",
        _ => "data",
    };

    /// <summary>
    /// 相对节点左上角的端口中心坐标（与 Workspace 节点模板几何一致，引脚到引脚连线）。
    /// </summary>
    public static (double X, double Y) LocalCenter(NodePortKind kind, NodePortDirection direction) => kind switch
    {
        NodePortKind.Communication => (NodeWidth / 2.0, CommPortY),
        NodePortKind.Control when direction == NodePortDirection.In => (PinInsetX, ExecPortY),
        NodePortKind.Control => (NodeWidth - PinInsetX, ExecPortY),
        NodePortKind.Data when direction == NodePortDirection.In => (DataPinInsetX, DataPortY),
        _ => (NodeWidth - DataPinInsetX, DataPortY),
    };

    /// <summary>按 handle 名解析中心（支持 data-in-N 多入、U125 分支执行出）。</summary>
    public static (double X, double Y) LocalCenterForHandle(string? handle)
    {
        if (!TryResolveKind(handle, out var kind, out var direction))
        {
            return LocalCenter(NodePortKind.Data, NodePortDirection.Out);
        }

        if (kind == NodePortKind.Data && direction == NodePortDirection.In)
        {
            var index = ParseDataInIndex(handle);
            return (DataPinInsetX, DataPortY + (index * DataPortSpacing));
        }

        if (kind == NodePortKind.Data && direction == NodePortDirection.Out)
        {
            return (NodeWidth - DataPinInsetX, DataPortY);
        }

        // U125：分支引脚与通用执行出口同列，只在 Y 上错开。
        if (IsBranchExecOutHandle(handle))
        {
            var (baseX, baseY) = LocalCenter(NodePortKind.Control, NodePortDirection.Out);
            return (baseX, baseY + BranchPinOffsetFor(handle));
        }

        return LocalCenter(kind, direction);
    }

    /// <summary>
    /// 循环节点的执行引脚左右互换（出口在左、入口在右），把「回流」画成真正折返的形状。
    /// 只镜像执行口：数据口与通信口位置不变。
    /// </summary>
    public static (double X, double Y) MirrorExecIfLoop(
        (double X, double Y) center,
        string? handle,
        bool mirrored)
    {
        if (!mirrored
            || !TryResolveKind(handle, out var kind, out _)
            || kind != NodePortKind.Control)
        {
            return center;
        }

        return (NodeWidth - center.X, center.Y);
    }

    /// <summary>input → 0；data-in-1 → 1；data-in-N → N。</summary>
    public static int ParseDataInIndex(string? handle)
    {
        var name = (handle ?? string.Empty).Trim();
        if (string.IsNullOrEmpty(name)
            || string.Equals(name, "input", StringComparison.OrdinalIgnoreCase))
        {
            return 0;
        }

        if (name.StartsWith("data-in-", StringComparison.OrdinalIgnoreCase)
            && int.TryParse(name["data-in-".Length..], out var n)
            && n >= 0)
        {
            return n;
        }

        return 0;
    }

    public static string DataInHandleName(int index) =>
        index <= 0 ? "input" : $"data-in-{index}";

    /// <summary>
    /// W7：XAML 高度、框选、Fit、小地图和节点命中共用的节点高度。
    /// 状态正文不再参与卡片测量；多数据入仍按端口间距确定真实高度。
    /// </summary>
    public static double NodeHeightForDataInputCount(int dataInputCount)
    {
        var count = Math.Max(1, dataInputCount);
        var lastPortCenter = DataPortY + ((count - 1) * DataPortSpacing);
        return Math.Max(MinimumNodeHeight, lastPortCenter + DataPortBottomInset);
    }

    /// <summary>数据入旁标签：贴在目标引脚右侧。</summary>
    public static (double X, double Y) LabelBesideDataIn(double endX, double endY) =>
        (endX + 10, endY - 7);

    /// <summary>
    /// 边路径几何：data/control 为水平 S 形三次贝塞尔；
    /// communication 为「从上方跳过」的开口向下抛物线风格（二次控制点抬高，再转三次）。
    /// </summary>
    public static EdgePathSpec BuildEdgePath(
        double startX, double startY, double endX, double endY, bool isCommunication)
    {
        if (isCommunication)
        {
            return BuildCommunicationJumpPath(startX, startY, endX, endY);
        }

        var controlOffset = Math.Max(48.0, Math.Abs(endX - startX) * 0.45);
        var c1x = startX + controlOffset;
        var c1y = startY;
        var c2x = endX - controlOffset;
        var c2y = endY;
        return new EdgePathSpec(
            Start: new Avalonia.Point(startX, startY),
            Control1: new Avalonia.Point(c1x, c1y),
            Control2: new Avalonia.Point(c2x, c2y),
            End: new Avalonia.Point(endX, endY));
    }

    /// <summary>
    /// 通信跳线：开口向下二次函数感——两端贴通信口向上翘，中点抬高像桥。
    /// 二次控制点 C = (midX, min(y) - lift)，再映射为三次贝塞尔。
    /// </summary>
    public static EdgePathSpec BuildCommunicationJumpPath(
        double startX, double startY, double endX, double endY)
    {
        var dx = endX - startX;
        var span = Math.Abs(dx);
        // 水平跨度小也要明显拱起；跨度大时拱更高，像跳过中间节点
        var lift = Math.Clamp(36.0 + span * 0.28, 48.0, 160.0);
        var peakY = Math.Min(startY, endY) - lift;
        var midX = (startX + endX) * 0.5;
        // 二次 Bezier 控制点（抛物线顶点附近）
        var qControlX = midX;
        var qControlY = peakY;
        // 二次 → 三次：C1 = P0 + 2/3 (Q - P0), C2 = P1 + 2/3 (Q - P1)
        const double twoThirds = 2.0 / 3.0;
        var c1x = startX + twoThirds * (qControlX - startX);
        var c1y = startY + twoThirds * (qControlY - startY);
        var c2x = endX + twoThirds * (qControlX - endX);
        var c2y = endY + twoThirds * (qControlY - endY);
        // 两端再略上提，出脚更有「跳」的起势
        var launch = Math.Min(18.0, lift * 0.22);
        c1y -= launch * 0.35;
        c2y -= launch * 0.35;
        return new EdgePathSpec(
            Start: new Avalonia.Point(startX, startY),
            Control1: new Avalonia.Point(c1x, c1y),
            Control2: new Avalonia.Point(c2x, c2y),
            End: new Avalonia.Point(endX, endY),
            PeakY: peakY);
    }

    public static Avalonia.Point CubicBezierPoint(
        Avalonia.Point p0, Avalonia.Point p1, Avalonia.Point p2, Avalonia.Point p3, double t)
    {
        t = Math.Clamp(t, 0, 1);
        var u = 1.0 - t;
        var x = (u * u * u * p0.X) + (3 * u * u * t * p1.X) + (3 * u * t * t * p2.X) + (t * t * t * p3.X);
        var y = (u * u * u * p0.Y) + (3 * u * u * t * p1.Y) + (3 * u * t * t * p2.Y) + (t * t * t * p3.Y);
        return new Avalonia.Point(x, y);
    }

    /// <summary>
    /// 归一化连接方向：出→入；通信口双向。与 TryConnectPorts / 高亮共用。
    /// </summary>
    public static bool TryNormalizeConnection(
        string aNodeId, NodePortKind aKind, NodePortDirection aDir,
        string bNodeId, NodePortKind bKind, NodePortDirection bDir,
        out string fromNodeId, out string toNodeId, out string fromHandle, out string toHandle, out string edgeKind)
    {
        fromNodeId = string.Empty;
        toNodeId = string.Empty;
        fromHandle = string.Empty;
        toHandle = string.Empty;
        edgeKind = EdgeKindName(aKind);

        if (aKind != bKind)
        {
            return false;
        }

        // 通信口双向：任意顺序，发起端为拖线起点。
        if (aKind == NodePortKind.Communication && bKind == NodePortKind.Communication)
        {
            fromNodeId = aNodeId;
            toNodeId = bNodeId;
            fromHandle = HandleName(NodePortKind.Communication, NodePortDirection.Out);
            toHandle = HandleName(NodePortKind.Communication, NodePortDirection.In);
            return true;
        }

        var aCanOut = aDir is NodePortDirection.Out or NodePortDirection.Both;
        var aCanIn = aDir is NodePortDirection.In or NodePortDirection.Both;
        var bCanOut = bDir is NodePortDirection.Out or NodePortDirection.Both;
        var bCanIn = bDir is NodePortDirection.In or NodePortDirection.Both;

        if (aCanOut && bCanIn)
        {
            fromNodeId = aNodeId;
            toNodeId = bNodeId;
            fromHandle = HandleName(aKind, NodePortDirection.Out);
            toHandle = HandleName(bKind, NodePortDirection.In);
            return true;
        }

        if (aCanIn && bCanOut)
        {
            fromNodeId = bNodeId;
            toNodeId = aNodeId;
            fromHandle = HandleName(bKind, NodePortDirection.Out);
            toHandle = HandleName(aKind, NodePortDirection.In);
            return true;
        }

        return false;
    }

    public static bool TryResolveKind(string? handle, out NodePortKind kind, out NodePortDirection direction)
    {
        var name = (handle ?? string.Empty).Trim();
        if (string.Equals(name, "exec_in", StringComparison.OrdinalIgnoreCase))
        {
            kind = NodePortKind.Control;
            direction = NodePortDirection.In;
            return true;
        }
        if (string.Equals(name, "exec_out", StringComparison.OrdinalIgnoreCase))
        {
            kind = NodePortKind.Control;
            direction = NodePortDirection.Out;
            return true;
        }
        // U125：两个分支执行出引脚同样是控制口。必须在下方 "out*" 兜底分支之前
        // 命中——否则 exec_out_true 会被当成数据出口，连线类型判定与几何全错。
        if (IsBranchExecOutHandle(name))
        {
            kind = NodePortKind.Control;
            direction = NodePortDirection.Out;
            return true;
        }
        if (string.Equals(name, "communication", StringComparison.OrdinalIgnoreCase))
        {
            kind = NodePortKind.Communication;
            direction = NodePortDirection.Both;
            return true;
        }
        if (string.Equals(name, "input", StringComparison.OrdinalIgnoreCase)
            || name.StartsWith("data-in-", StringComparison.OrdinalIgnoreCase)
            || name.StartsWith("in-", StringComparison.OrdinalIgnoreCase)
            || string.Equals(name, "in", StringComparison.OrdinalIgnoreCase))
        {
            kind = NodePortKind.Data;
            direction = NodePortDirection.In;
            return true;
        }
        if (string.Equals(name, "output", StringComparison.OrdinalIgnoreCase)
            || name.StartsWith("data-out", StringComparison.OrdinalIgnoreCase)
            || name.StartsWith("out-", StringComparison.OrdinalIgnoreCase)
            || string.Equals(name, "out", StringComparison.OrdinalIgnoreCase))
        {
            kind = NodePortKind.Data;
            direction = NodePortDirection.Out;
            return true;
        }
        // 兼容旧别名：in* / out*（但 data-in 已在上面处理）
        if (name.StartsWith("in", StringComparison.OrdinalIgnoreCase))
        {
            kind = NodePortKind.Data;
            direction = NodePortDirection.In;
            return true;
        }
        if (name.StartsWith("out", StringComparison.OrdinalIgnoreCase))
        {
            kind = NodePortKind.Data;
            direction = NodePortDirection.Out;
            return true;
        }

        kind = NodePortKind.Data;
        direction = NodePortDirection.Out;
        return false;
    }

}

/// <summary>边路径控制点规格（可单测）。</summary>
public readonly record struct EdgePathSpec(
    Avalonia.Point Start,
    Avalonia.Point Control1,
    Avalonia.Point Control2,
    Avalonia.Point End,
    double? PeakY = null)
{
    public Avalonia.Point Midpoint =>
        NodePortSpec.CubicBezierPoint(Start, Control1, Control2, End, 0.5);

    public Avalonia.Vector MidpointTangent => new(
        (0.75 * (Control1.X - Start.X))
        + (1.5 * (Control2.X - Control1.X))
        + (0.75 * (End.X - Control2.X)),
        (0.75 * (Control1.Y - Start.Y))
        + (1.5 * (Control2.Y - Control1.Y))
        + (0.75 * (End.Y - Control2.Y)));
}

public sealed class WorkflowNodeViewModel : ViewModelBase
{
    private readonly WorkflowNodeCatalogEntry _descriptor;

    private readonly Action _markDirty;
    /// <summary>加载时保留的非 UI 配置键，保存时经 <see cref="NodeConfigData.MergeUiFields"/> 合并回去。</summary>
    private Dictionary<string, object?> _extraData = new(StringComparer.Ordinal);
    private string _name;
    private string _workDir;
    private string _userNote = string.Empty;
    private bool _exposedAsTool;
    private bool _portControlInConnected;
    private bool _portControlOutConnected;
    // U125：两个分支引脚各自独立的连接态（不能与 _portControlOutConnected 合并）。
    private bool _portControlOutTrueConnected;
    private bool _portControlOutFalseConnected;
    private bool _portDataInConnected;
    private bool _portDataOutConnected;
    private bool _portCommunicationConnected;
    private bool _breakpointEnabled;
    private string _promptTemplate = string.Empty;
    private string _modelId = string.Empty;
    private string _budgetUsd = string.Empty;
    private string _timeoutMs = string.Empty;
    private string _statusText = string.Empty;
    private double _x;
    private double _y;
    private bool _isSelected;
    private double _portControlInOpacity = 1.0;
    private double _portControlOutOpacity = 1.0;
    private double _portDataInOpacity = 1.0;
    private double _portDataOutOpacity = 1.0;
    private double _portCommunicationOpacity = 1.0;
    private bool _portControlInCompatible;
    private bool _portControlOutCompatible;
    private bool _portDataInCompatible;
    private bool _portDataOutCompatible;
    private bool _portCommunicationCompatible;
    // U181-E：连线起点标记（每个引脚一个）。
    private bool _portControlInIsOrigin;
    private bool _portControlOutIsOrigin;
    private bool _portDataInIsOrigin;
    private bool _portDataOutIsOrigin;
    private bool _portCommunicationIsOrigin;
    private string _importPath = string.Empty;
    private bool _includeContent = true;
    private string _queryAlias = "query";
    private string _searchLimit = "10";
    private string _conditionInputAlias = "input";
    private string _conditionOperator = "truthy";
    private string _conditionExpected = string.Empty;
    private string _maxIterations = "5";
    private string _stopInputAlias = "done";
    private string _stopExpected = "true";
    private string _approvalId = string.Empty;
    private bool _autoApprove;
    private string _exportArtifactId = string.Empty;
    private string _exportFormat = "markdown";
    private string _exportTitle = string.Empty;
    private string _providerId = string.Empty;
    private string _summarizerChapterId = string.Empty;
    private string _summarizerChapterDocumentId = string.Empty;
    private string _summarizerChapterTextAlias = "chapter_text";
    private bool _summarizerAutoMode;
    private int _nextDataInIndex = 1;
    // U178-B：页面级语义缩放开关的**投影副本**。
    // 这两个值逻辑上属于页面（由 CanvasZoom 决定），但节点卡片模板要按它们
    // 显隐引脚与详情。原先模板里用 `$parent[UserControl].DataContext.X` 直接
    // 向上取，代价是每节点 8 个祖先绑定：各建一个 ControlTracker、订阅
    // attach/detach 两个事件、并跑 10 层 LINQ 祖先遍历，
    // 而订阅的正是 attach/detach ⇒ 成本全落在「切回画布页」的重挂载路径（U159）。
    // 投影到节点自身后模板改绑普通属性，零祖先遍历、零 attach 订阅。
    // ⚠️ 与 C-1 那 18 个静态文案不同：这两个**会变**（用户缩放时切换），
    // 所以必须仍是绑定，只是绑到节点 VM 上；页面在 zoom 跨阈值时广播给全部节点。
    private bool _showCanvasDetails = true;
    private bool _showCanvasPrecisionControls = true;

    public WorkflowNodeViewModel(
        string id,
        string nodeType,
        string label,
        string defaultWorkDir,
        double x,
        double y,
        Action<WorkflowNodeViewModel> runRequested,
        Action clearSelection,
        Action markDirty,
        Func<bool>? canRun = null)
    {
        Id = id;
        NodeType = nodeType;
        _descriptor = WorkflowNodeCatalog.Resolve(nodeType);
        Label = label;
        _name = label;
        _workDir = defaultWorkDir;
        _exposedAsTool = _descriptor.ConfigKind == "start";
        _x = x;
        _y = y;
        _markDirty = markDirty;
        if (_descriptor.ConfigKind == "loop")
        {
            _timeoutMs = "300000";
        }
        SelectCommand = new RelayCommand(() => clearSelection());
        RunCommand = new RelayCommand(() => runRequested(this), canRun);
        DataInPins = new ObservableCollection<NodeDataInPinViewModel>();
        AddDataInPinCommand = new RelayCommand(AddDataInPin);
        RemoveDataInPinCommand = new RelayCommand(RemoveLastDataInPin, () => DataInPins.Count > 1);
        RestoreDataInPins(new[] { NodePortSpec.DataInHandleName(0) });
    }

    public string Id { get; }
    public string NodeType { get; }
    public string Label { get; }
    public RelayCommand SelectCommand { get; set; }
    public RelayCommand RunCommand { get; }

    public void NotifyRunCommandState() => RunCommand.NotifyCanExecuteChanged();
    public RelayCommand AddDataInPinCommand { get; }
    public RelayCommand RemoveDataInPinCommand { get; }
    public bool IsStartNode => _descriptor.ConfigKind == "start";

    /// <summary>
    /// U178-B：页面级「显示详情」开关在本节点上的投影。
    ///
    /// 值由页面 VM 在 zoom 跨阈值时统一写入（<see cref="WorkspacePageViewModel"/>
    /// 的 NotifyCanvasZoomChanged）。节点模板绑这个而不是绑祖先，
    /// 理由见字段处注释——祖先绑定的成本落在切页重挂载路径上。
    /// </summary>
    public bool ShowCanvasDetails
    {
        get => _showCanvasDetails;
        internal set => SetProperty(ref _showCanvasDetails, value);
    }

    /// <summary>U178-B：页面级「显示精度控件」开关在本节点上的投影，同上。</summary>
    public bool ShowCanvasPrecisionControls
    {
        get => _showCanvasPrecisionControls;
        internal set => SetProperty(ref _showCanvasPrecisionControls, value);
    }

    /// <summary>
    /// 起始节点的变量组；非起始节点为 null。
    ///
    /// 由页面 VM 构造并注入（变量行的文案需要 DisplayNameService，
    /// 而节点 VM 本身不持有它——沿用本文件既有的「文案留在页面级」惯例）。
    /// </summary>
    public WorkflowVariableGroupViewModel? Variables { get; set; }

    /// <summary>没有可见变量时整块不渲染，空变量区只是噪点。</summary>
    public bool HasVariables => Variables is { HasVariables: true };
    /// <summary>文档读/导入：选路径 → 输出 document/content（后端 path 字段）。</summary>
    public bool IsDocumentNode => _descriptor.ConfigKind == "document";
    /// <summary>兼容旧绑定名；仅文档读，不含 export。</summary>
    public bool IsImportNode => IsDocumentNode;
    public bool IsSearchNode => _descriptor.ConfigKind == "search";
    public bool IsConditionNode => _descriptor.ConfigKind == "condition";
    /// <summary>
    /// U125：condition/eval 用两个分支执行出引脚，其它节点用单个通用 exec_out。
    /// 两者必须互斥——同时出现会让用户既能画分支边又能画通用「恒放行」边，
    /// 而通用边已被后端保存边界拒绝，用户会撞上一个画得出却存不下的状态。
    /// （外层可见性仍由页面级 ShowCanvasPrecisionControls 统一控制。）
    /// </summary>
    public bool ShowBranchExecOutPins => IsConditionNode;

    /// <summary>非 condition 节点才显示通用执行出引脚。</summary>
    public bool ShowGenericExecOutPin => !IsConditionNode;

    /// <summary>
    /// condition 节点不显示数据出引脚。
    ///
    /// 它的 passed/reason/branch 输出已由两个分支执行引脚在结构上表达；
    /// 再挂一个数据出口只会让人以为该把判定结果当数据往下接。
    /// 数据**入**引脚必须保留：`execute_condition` 要按 input_alias 从 inputs
    /// 取判定对象，没有它节点直接报 condition input alias missing。
    ///
    /// 注意这里只表达「节点类型允不允许」；页面级的
    /// ShowCanvasPrecisionControls 由视图另外与它取交集，两者不能互相顶替。
    /// </summary>
    public bool ShowDataOutPin => !IsConditionNode;
    public bool IsLoopNode => _descriptor.ConfigKind == "loop";
    public bool IsApprovalNode => _descriptor.ConfigKind == "approval";
    public bool IsExportNode => _descriptor.ConfigKind == "export";
    public bool IsSummarizerNode => _descriptor.ConfigKind == "summarizer";
    public bool IsUtilityNode => _descriptor.LibraryGroup == "utility";
    public bool IsAgentNode => _descriptor.HasModelExecution;
    /// <summary>
    /// 执行引脚左右互换（目前仅循环节点）。循环把流程送回上游，左右不换的话
    /// 回流边要绕过整张卡片；换过来后「左出 → 上游、右入 ← 下游」正好顺着回流方向，
    /// 两个箭头一起朝左，一眼能看出这是个往回走的环。
    /// 连线端点走视觉树实测（TryGetPortCanvasCenter），所以模板换列即自动跟随；
    /// 仅首帧未测量时的常量兜底需要同步（见 NodePortSpec.LocalCenter 的 mirrored 重载）。
    /// </summary>
    public bool MirrorExecPorts => IsLoopNode;
    /// <summary>执行入引脚所在列：常规 0（左）、镜像 3（右）。</summary>
    public int ExecInColumn => MirrorExecPorts ? 3 : 0;
    /// <summary>执行出引脚所在列：常规 3（右）、镜像 0（左）。</summary>
    public int ExecOutColumn => MirrorExecPorts ? 0 : 3;
    public bool ShowPromptEditor => IsAgentNode;
    public bool ShowDataInPinEditor => !IsStartNode;
    public ObservableCollection<NodeDataInPinViewModel> DataInPins { get; }
    public double CanvasHeight => NodePortSpec.NodeHeightForDataInputCount(DataInPins.Count);

    public string ImportPath
    {
        get => _importPath;
        set
        {
            if (SetProperty(ref _importPath, value ?? string.Empty))
            {
                OnPropertyChanged(nameof(HasImportPath));
                OnPropertyChanged(nameof(ImportPathDisplay));
                _markDirty();
            }
        }
    }

    public bool IncludeContent { get => _includeContent; set { if (SetProperty(ref _includeContent, value)) _markDirty(); } }
    public string QueryAlias
    {
        get => _queryAlias;
        set
        {
            var previous = _queryAlias;
            if (SetProperty(ref _queryAlias, value ?? "query"))
            {
                _markDirty();
                QueryAliasChanged?.Invoke(previous, _queryAlias);
            }
        }
    }
    public string SearchLimit { get => _searchLimit; set { if (SetProperty(ref _searchLimit, value ?? "10")) _markDirty(); } }
    public string ConditionInputAlias { get => _conditionInputAlias; set { if (SetProperty(ref _conditionInputAlias, value ?? "input")) _markDirty(); } }
    public string ConditionOperator { get => _conditionOperator; set { if (SetProperty(ref _conditionOperator, value ?? "truthy")) _markDirty(); } }
    public string ConditionExpected { get => _conditionExpected; set { if (SetProperty(ref _conditionExpected, value ?? string.Empty)) _markDirty(); } }
    public string MaxIterations { get => _maxIterations; set { if (SetProperty(ref _maxIterations, value ?? "5")) _markDirty(); } }
    public string StopInputAlias
    {
        get => _stopInputAlias;
        set
        {
            var previous = _stopInputAlias;
            if (SetProperty(ref _stopInputAlias, value ?? "done"))
            {
                _markDirty();
                StopInputAliasChanged?.Invoke(previous, _stopInputAlias);
            }
        }
    }
    public string StopExpected { get => _stopExpected; set { if (SetProperty(ref _stopExpected, value ?? "true")) _markDirty(); } }
    public string ApprovalId { get => _approvalId; set { if (SetProperty(ref _approvalId, value ?? string.Empty)) _markDirty(); } }
    public bool AutoApprove { get => _autoApprove; set { if (SetProperty(ref _autoApprove, value)) _markDirty(); } }
    public string ExportArtifactId { get => _exportArtifactId; set { if (SetProperty(ref _exportArtifactId, value ?? string.Empty)) _markDirty(); } }
    public string ExportFormat { get => _exportFormat; set { if (SetProperty(ref _exportFormat, value ?? "markdown")) _markDirty(); } }
    public string ExportTitle { get => _exportTitle; set { if (SetProperty(ref _exportTitle, value ?? string.Empty)) _markDirty(); } }
    public string ProviderId
    {
        get => _providerId;
        set
        {
            if (SetProperty(ref _providerId, value ?? string.Empty))
            {
                OnPropertyChanged(nameof(SummarizerProviderId));
                _markDirty();
            }
        }
    }

    /// <summary>兼容总结节点既有绑定；与所有 LLM/写作节点共用同一个 provider_id。</summary>
    public string SummarizerProviderId
    {
        get => ProviderId;
        set => ProviderId = value;
    }
    public string SummarizerChapterId { get => _summarizerChapterId; set { if (SetProperty(ref _summarizerChapterId, value ?? string.Empty)) _markDirty(); } }
    public string SummarizerChapterDocumentId { get => _summarizerChapterDocumentId; set { if (SetProperty(ref _summarizerChapterDocumentId, value ?? string.Empty)) _markDirty(); } }
    public string SummarizerChapterTextAlias
    {
        get => _summarizerChapterTextAlias;
        set
        {
            var previous = _summarizerChapterTextAlias;
            if (SetProperty(ref _summarizerChapterTextAlias, value ?? string.Empty))
            {
                _markDirty();
                SummarizerChapterTextAliasChanged?.Invoke(previous, _summarizerChapterTextAlias);
            }
        }
    }
    public bool SummarizerAutoMode { get => _summarizerAutoMode; set { if (SetProperty(ref _summarizerAutoMode, value)) _markDirty(); } }

    public bool HasImportPath => !string.IsNullOrWhiteSpace(ImportPath);
    public string ImportPathDisplay => HasImportPath ? ImportPath : string.Empty;

    public static readonly string[] ConditionOperators = { "truthy", "equals", "not_equals" };
    public static readonly string[] ExportFormats = { "markdown", "epub", "pdf", "json" };

    public void AddDataInPin()
    {
        var handle = NodePortSpec.DataInHandleName(_nextDataInIndex++);
        while (DataInPins.Any(p => string.Equals(p.Handle, handle, StringComparison.OrdinalIgnoreCase)))
        {
            handle = NodePortSpec.DataInHandleName(_nextDataInIndex++);
        }

        DataInPins.Add(new NodeDataInPinViewModel(handle, shortLabel: $"in{_nextDataInIndex - 1}", () => RemoveDataInPin(handle)));
        RemoveDataInPinCommand.NotifyCanExecuteChanged();
        OnPropertyChanged(nameof(DataInPins));
        OnPropertyChanged(nameof(CanvasHeight));
        _markDirty();
    }

    public void RemoveLastDataInPin()
    {
        if (DataInPins.Count <= 1)
        {
            return;
        }

        var last = DataInPins[^1];
        RemoveDataInPin(last.Handle);
    }

    public void RemoveDataInPin(string handle)
    {
        if (DataInPins.Count <= 1)
        {
            return;
        }

        var pin = DataInPins.FirstOrDefault(p => string.Equals(p.Handle, handle, StringComparison.OrdinalIgnoreCase));
        if (pin is null)
        {
            return;
        }

        DataInPins.Remove(pin);
        RemoveDataInPinCommand.NotifyCanExecuteChanged();
        OnPropertyChanged(nameof(DataInPins));
        OnPropertyChanged(nameof(CanvasHeight));
        _markDirty();
        DataInPinRemoved?.Invoke(handle);
    }

    /// <summary>删除数据入后由宿主拆掉占用该 handle 的边。</summary>
    public Action<string>? DataInPinRemoved { get; set; }

    /// <summary>Summarizer 正文 alias 改动后由宿主同步首个数据入边。</summary>
    public Action<string, string>? SummarizerChapterTextAliasChanged { get; set; }

    /// <summary>Search/Loop 输入 alias 改动后同步首个数据入边。</summary>
    public Action<string, string>? QueryAliasChanged { get; set; }
    public Action<string, string>? StopInputAliasChanged { get; set; }

    /// <summary>从已存配置恢复多数据入列表。</summary>
    public void RestoreDataInPins(IEnumerable<string>? handles)
    {
        var list = (handles ?? Array.Empty<string>())
            .Where(h => !string.IsNullOrWhiteSpace(h))
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .ToList();
        if (list.Count == 0)
        {
            list.Add(NodePortSpec.DataInHandleName(0));
        }

        DataInPins.Clear();
        var i = 0;
        foreach (var h in list)
        {
            var captured = h;
            DataInPins.Add(new NodeDataInPinViewModel(captured, i == 0 ? "in" : $"in{i}", () => RemoveDataInPin(captured)));
            var idx = NodePortSpec.ParseDataInIndex(h);
            if (idx >= _nextDataInIndex)
            {
                _nextDataInIndex = idx + 1;
            }

            i++;
        }

        RemoveDataInPinCommand.NotifyCanExecuteChanged();
        OnPropertyChanged(nameof(DataInPins));
        OnPropertyChanged(nameof(CanvasHeight));
    }

    public string Name { get => _name; set => SetProperty(ref _name, value); }
    public string WorkDir { get => _workDir; set => SetProperty(ref _workDir, value); }
    /// <summary>用户备注：项目 AI list_start_nodes 会读出给模型抉择。</summary>
    public string UserNote { get => _userNote; set => SetProperty(ref _userNote, value); }
    public bool ExposedAsTool { get => _exposedAsTool; set => SetProperty(ref _exposedAsTool, value); }
    public bool BreakpointEnabled { get => _breakpointEnabled; set => SetProperty(ref _breakpointEnabled, value); }
    public string PromptTemplate { get => _promptTemplate; set => SetProperty(ref _promptTemplate, value); }
    public string ModelId { get => _modelId; set => SetProperty(ref _modelId, value); }
    public string BudgetUsd { get => _budgetUsd; set => SetProperty(ref _budgetUsd, value); }
    public string TimeoutMs
    {
        get => _timeoutMs;
        set
        {
            if (SetProperty(ref _timeoutMs, value))
            {
                OnPropertyChanged(nameof(TimeoutSecondsText));
            }
        }
    }
    /// <summary>作者向秒数展示；内部仍存 ms（见 <see cref="NodeTimeoutHelper"/>）。</summary>
    public string TimeoutSecondsText
    {
        get => NodeTimeoutHelper.FormatSecondsFromMs(TimeoutMs);
        set => TimeoutMs = NodeTimeoutHelper.ParseSecondsToMs(value);
    }
    public string StatusText
    {
        get => _statusText;
        set
        {
            if (SetProperty(ref _statusText, value))
            {
                OnPropertyChanged(nameof(IsRunningStatus));
                OnPropertyChanged(nameof(IsPendingStatus));
                OnPropertyChanged(nameof(IsPausedStatus));
                OnPropertyChanged(nameof(IsSucceededStatus));
                OnPropertyChanged(nameof(IsFailedStatus));
            }
        }
    }
    public double X
    {
        get => _x;
        set => SetProperty(ref _x, value);
    }
    public double Y
    {
        get => _y;
        set => SetProperty(ref _y, value);
    }
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }
    public bool IsRunningStatus => ClassifyStatus(StatusText) == NodeRuntimeStatus.Running;
    public bool IsPendingStatus => ClassifyStatus(StatusText) == NodeRuntimeStatus.Pending;
    public bool IsPausedStatus => ClassifyStatus(StatusText) == NodeRuntimeStatus.Paused;
    public bool IsSucceededStatus => ClassifyStatus(StatusText) == NodeRuntimeStatus.Succeeded;
    public bool IsFailedStatus => ClassifyStatus(StatusText) == NodeRuntimeStatus.Failed;

    // 节点不再按类型着色/配图：卡片质感全部由主题的「毛玻璃→实心」纵向渐变承担，
    // 类别信息靠标题文案与检查器表达，避免画布被一堆彩色芯片打花。

    public double PortControlInOpacity { get => _portControlInOpacity; private set => SetProperty(ref _portControlInOpacity, value); }
    public double PortControlOutOpacity { get => _portControlOutOpacity; private set => SetProperty(ref _portControlOutOpacity, value); }
    public double PortDataInOpacity { get => _portDataInOpacity; private set => SetProperty(ref _portDataInOpacity, value); }
    public double PortDataOutOpacity { get => _portDataOutOpacity; private set => SetProperty(ref _portDataOutOpacity, value); }
    public double PortCommunicationOpacity { get => _portCommunicationOpacity; private set => SetProperty(ref _portCommunicationOpacity, value); }
    public bool PortControlInCompatible { get => _portControlInCompatible; private set => SetProperty(ref _portControlInCompatible, value); }
    public bool PortControlOutCompatible { get => _portControlOutCompatible; private set => SetProperty(ref _portControlOutCompatible, value); }
    public bool PortDataInCompatible { get => _portDataInCompatible; private set => SetProperty(ref _portDataInCompatible, value); }
    public bool PortDataOutCompatible { get => _portDataOutCompatible; private set => SetProperty(ref _portDataOutCompatible, value); }
    public bool PortCommunicationCompatible { get => _portCommunicationCompatible; private set => SetProperty(ref _portCommunicationCompatible, value); }

    // U181-E：「这个引脚就是当前连线的起点」。
    //
    // 与 `*Compatible` 分开而不是复用它，是因为两者语义相反：
    // Compatible = 「线可以落在这」，IsOrigin = 「线是从这出发的」。
    // 视觉上也必须不同 —— 起点用一圈**实边**（与 U178 那批「状态不能只靠颜色」
    // 一致），可落点用满不透明 + 兼容色。合并成一个标记会让作者
    // 在一堆等价高亮里找不出自己刚选的那个。
    public bool PortControlInIsOrigin { get => _portControlInIsOrigin; private set => SetProperty(ref _portControlInIsOrigin, value); }
    public bool PortControlOutIsOrigin { get => _portControlOutIsOrigin; private set => SetProperty(ref _portControlOutIsOrigin, value); }
    public bool PortDataInIsOrigin { get => _portDataInIsOrigin; private set => SetProperty(ref _portDataInIsOrigin, value); }
    public bool PortDataOutIsOrigin { get => _portDataOutIsOrigin; private set => SetProperty(ref _portDataOutIsOrigin, value); }
    public bool PortCommunicationIsOrigin { get => _portCommunicationIsOrigin; private set => SetProperty(ref _portCommunicationIsOrigin, value); }

    public bool PortControlInConnected => _portControlInConnected;
    public bool PortControlOutConnected => _portControlOutConnected;
    /// <summary>U125：condition「真」分支引脚是否已连线（实心/空心）。</summary>
    public bool PortControlOutTrueConnected => _portControlOutTrueConnected;
    /// <summary>U125：condition「假」分支引脚是否已连线。</summary>
    public bool PortControlOutFalseConnected => _portControlOutFalseConnected;
    public bool PortDataInConnected => _portDataInConnected;
    public bool PortDataOutConnected => _portDataOutConnected;
    public bool PortCommunicationConnected => _portCommunicationConnected;

    /// <summary>
    /// U125：分别刷新两个分支引脚的连接态。
    ///
    /// 不能复用 `controlOut` 这一个布尔——两个引脚各自独立连线，
    /// 合成一个值会让「只连了真分支」的 condition 把假分支也画成实心，
    /// 用户以为两支都接上了。
    /// </summary>
    public void SetBranchPortConnected(bool trueBranch, bool falseBranch)
    {
        if (_portControlOutTrueConnected != trueBranch)
        {
            _portControlOutTrueConnected = trueBranch;
            OnPropertyChanged(nameof(PortControlOutTrueConnected));
        }
        if (_portControlOutFalseConnected != falseBranch)
        {
            _portControlOutFalseConnected = falseBranch;
            OnPropertyChanged(nameof(PortControlOutFalseConnected));
        }
    }

    public void SetPortConnected(
        bool controlIn, bool controlOut, bool dataIn, bool dataOut, bool communication)
    {
        if (_portControlInConnected != controlIn)
        {
            _portControlInConnected = controlIn;
            OnPropertyChanged(nameof(PortControlInConnected));
        }
        if (_portControlOutConnected != controlOut)
        {
            _portControlOutConnected = controlOut;
            OnPropertyChanged(nameof(PortControlOutConnected));
        }
        if (_portDataInConnected != dataIn)
        {
            _portDataInConnected = dataIn;
            OnPropertyChanged(nameof(PortDataInConnected));
        }
        if (_portDataOutConnected != dataOut)
        {
            _portDataOutConnected = dataOut;
            OnPropertyChanged(nameof(PortDataOutConnected));
        }
        if (_portCommunicationConnected != communication)
        {
            _portCommunicationConnected = communication;
            OnPropertyChanged(nameof(PortCommunicationConnected));
        }
    }

    /// <summary>
    /// 拖线中的端口外观：可连满不透明、不可连淡出。
    ///
    /// <paramref name="originHandle"/> 是 U181-E：当本节点就是连线**起点**所在节点时，
    /// 传入被选中的那个引脚名。该引脚会强制满不透明并置上「已选为起点」标记，
    /// 不再跟着同节点其余端口一起淡出。
    /// </summary>
    public void SetPortDragHighlight(
        bool controlIn, bool controlOut, bool dataIn, bool dataOut, bool communication,
        string? originHandle = null)
    {
        // 可连：满不透明 + 兼容标记；不可连：淡出。
        PortControlInCompatible = controlIn;
        PortControlOutCompatible = controlOut;
        PortDataInCompatible = dataIn;
        PortDataOutCompatible = dataOut;
        PortCommunicationCompatible = communication;

        // 起点标记先算出来，再参与不透明度：起点必须压过「不可连 ⇒ 淡出」那条，
        // 否则同节点判 Self 失败会把它一起淡掉（原缺陷）。
        PortControlInIsOrigin = originHandle == NodePortSpec.HandleName(NodePortKind.Control, NodePortDirection.In);
        PortControlOutIsOrigin = originHandle == NodePortSpec.HandleName(NodePortKind.Control, NodePortDirection.Out);
        PortDataInIsOrigin = originHandle == NodePortSpec.HandleName(NodePortKind.Data, NodePortDirection.In);
        PortDataOutIsOrigin = originHandle == NodePortSpec.HandleName(NodePortKind.Data, NodePortDirection.Out);
        PortCommunicationIsOrigin = originHandle == NodePortSpec.HandleName(NodePortKind.Communication, NodePortDirection.Both);

        PortControlInOpacity = controlIn || PortControlInIsOrigin ? 1.0 : 0.22;
        PortControlOutOpacity = controlOut || PortControlOutIsOrigin ? 1.0 : 0.22;
        PortDataInOpacity = dataIn || PortDataInIsOrigin ? 1.0 : 0.22;
        PortDataOutOpacity = dataOut || PortDataOutIsOrigin ? 1.0 : 0.22;
        PortCommunicationOpacity = communication || PortCommunicationIsOrigin ? 1.0 : 0.22;
    }

    public void ClearPortDragHighlight()
    {
        PortControlInCompatible = false;
        PortControlOutCompatible = false;
        PortDataInCompatible = false;
        PortDataOutCompatible = false;
        PortCommunicationCompatible = false;
        // U181-E：起点标记必须与兼容标记同批清掉，否则取消连线后那圈「已选为起点」
        // 的实边会留在画布上，作者会以为连线还没结束。
        PortControlInIsOrigin = false;
        PortControlOutIsOrigin = false;
        PortDataInIsOrigin = false;
        PortDataOutIsOrigin = false;
        PortCommunicationIsOrigin = false;
        PortControlInOpacity = 1.0;
        PortControlOutOpacity = 1.0;
        PortDataInOpacity = 1.0;
        PortDataOutOpacity = 1.0;
        PortCommunicationOpacity = 1.0;
    }

    /// <summary>
    /// 从加载/粘贴的 graph node.Data 保留 opaque 键（tool_enabled 等），供后续 ToData 合并。
    /// </summary>
    public void RetainOpaqueData(IReadOnlyDictionary<string, object?>? sourceData)
    {
        _extraData = NodeConfigData.CaptureExtra(sourceData);
    }

    public Dictionary<string, object?> ToData()
    {
        return NodeConfigData.MergeUiFields(
            _extraData,
            Name,
            WorkDir,
            UserNote,
            IsStartNode,
            ExposedAsTool,
            ShowPromptEditor ? PromptTemplate : string.Empty,
            ModelId,
            BudgetUsd,
            TimeoutMs,
            BreakpointEnabled,
            IsDocumentNode ? ImportPath : null,
            DataInPins.Select(p => p.Handle).ToArray(),
            BuildUtilityFields(),
            ProviderId);
    }

    /// <summary>按节点类型写出后端期望的配置键（与 workflow nodes / integration 对齐）。</summary>
    public Dictionary<string, object?> BuildUtilityFields()
    {
        var fields = new Dictionary<string, object?>(StringComparer.Ordinal);
        // 起始节点的摘要句式：作者在执行页改过就要能存回去。
        // 只有它在 Variables 上（非起始节点没有变量组），所以在这里按类型写出。
        // 空句式写 null 而非空串：缺省语义是「没有句式，按声明顺序拼」，
        // 存一个空串会让后端把它当成「句式就是空的」而渲染出一行空白。
        if (IsStartNode && Variables is { } variables)
        {
            var template = variables.SummaryTemplate.Trim();
            fields["summary_template"] = template.Length == 0 ? null : template;
        }
        if (IsSummarizerNode)
        {
            fields["provider_id"] = SummarizerProviderId.Trim();
            fields["chapter_id"] = SummarizerChapterId.Trim();
            fields["chapter_document_id"] = SummarizerChapterDocumentId.Trim();
            fields["chapter_text_alias"] = string.IsNullOrWhiteSpace(SummarizerChapterTextAlias)
                ? "chapter_text"
                : SummarizerChapterTextAlias.Trim();
            fields["auto_mode"] = SummarizerAutoMode;
        }
        else if (IsDocumentNode)
        {
            fields["include_content"] = IncludeContent;
        }
        else if (IsSearchNode)
        {
            fields["query_alias"] = string.IsNullOrWhiteSpace(QueryAlias) ? "query" : QueryAlias.Trim();
            if (int.TryParse(SearchLimit, out var lim) && lim > 0)
            {
                fields["limit"] = lim;
            }
        }
        else if (IsConditionNode)
        {
            fields["input_alias"] = string.IsNullOrWhiteSpace(ConditionInputAlias) ? "input" : ConditionInputAlias.Trim();
            fields["operator"] = string.IsNullOrWhiteSpace(ConditionOperator) ? "truthy" : ConditionOperator.Trim();
            if (!string.IsNullOrWhiteSpace(ConditionExpected))
            {
                fields["expected"] = ParseLooseJsonOrString(ConditionExpected);
            }
        }
        else if (IsLoopNode)
        {
            if (int.TryParse(MaxIterations, out var mi) && mi > 0)
            {
                fields["max_iterations"] = mi;
            }

            if (long.TryParse(TimeoutMs, out var timeoutMs) && timeoutMs > 0)
            {
                fields["timeout_ms"] = timeoutMs;
            }

            // stop_condition: { input_alias, equals }
            fields["stop_condition"] = new Dictionary<string, object?>
            {
                ["input_alias"] = string.IsNullOrWhiteSpace(StopInputAlias) ? "done" : StopInputAlias.Trim(),
                ["equals"] = ParseLooseJsonOrString(string.IsNullOrWhiteSpace(StopExpected) ? "true" : StopExpected),
            };
        }
        else if (IsApprovalNode)
        {
            fields["approval_id"] = string.IsNullOrWhiteSpace(ApprovalId)
                ? $"approval-{Id}"
                : ApprovalId.Trim();
            fields["auto_approve"] = AutoApprove;
        }
        else if (IsExportNode)
        {
            fields["artifact_id"] = string.IsNullOrWhiteSpace(ExportArtifactId)
                ? $"export-{Id}"
                : ExportArtifactId.Trim();
            fields["format"] = string.IsNullOrWhiteSpace(ExportFormat) ? "markdown" : ExportFormat.Trim();
            if (!string.IsNullOrWhiteSpace(ExportTitle))
            {
                fields["title"] = ExportTitle.Trim();
            }
        }

        return fields;
    }

    private static object ParseLooseJsonOrString(string raw)
    {
        var t = raw.Trim();
        if (t is "true" or "True")
        {
            return true;
        }

        if (t is "false" or "False")
        {
            return false;
        }

        if (long.TryParse(t, out var n))
        {
            return n;
        }

        if (double.TryParse(t, System.Globalization.NumberStyles.Float,
                System.Globalization.CultureInfo.InvariantCulture, out var d))
        {
            return d;
        }

        if ((t.StartsWith('{') && t.EndsWith('}')) || (t.StartsWith('[') && t.EndsWith(']')))
        {
            try
            {
                return System.Text.Json.JsonSerializer.Deserialize<object>(t) ?? t;
            }
            catch
            {
                // fall through
            }
        }

        return t;
    }

    public CanvasNode ToCanvasNode()
    {
        return new CanvasNode(
            Id,
            NodeType,
            string.IsNullOrWhiteSpace(Name) ? Label : Name,
            ToData(),
            new CanvasPosition(X, Y));
    }

    protected override void OnPropertyChanged(string? propertyName = null)
    {
        base.OnPropertyChanged(propertyName);
        if (propertyName is nameof(Name) or nameof(WorkDir) or nameof(UserNote) or nameof(ExposedAsTool)
            or nameof(PromptTemplate) or nameof(ProviderId) or nameof(ModelId) or nameof(BudgetUsd) or nameof(TimeoutMs) or nameof(TimeoutSecondsText)
            or nameof(BreakpointEnabled) or nameof(X) or nameof(Y))
        {
            _markDirty();
        }
    }

    private static NodeRuntimeStatus ClassifyStatus(string status)
    {
        var normalized = status.Trim().ToLowerInvariant();
        if (string.IsNullOrWhiteSpace(normalized))
        {
            return NodeRuntimeStatus.Idle;
        }
        if (normalized.Contains("running") || normalized.Contains("运行"))
        {
            return NodeRuntimeStatus.Running;
        }
        if (normalized.Contains("queued") || normalized.Contains("pending") || normalized.Contains("排队"))
        {
            return NodeRuntimeStatus.Pending;
        }
        if (normalized.Contains("paused") || normalized.Contains("暂停"))
        {
            return NodeRuntimeStatus.Paused;
        }
        if (normalized.Contains("succeeded") || normalized.Contains("success") || normalized.Contains("成功"))
        {
            return NodeRuntimeStatus.Succeeded;
        }
        if (normalized.Contains("failed")
            || normalized.Contains("error")
            || normalized.Contains("exception")
            || normalized.Contains("失败")
            || normalized.Contains("错误"))
        {
            return NodeRuntimeStatus.Failed;
        }
        return NodeRuntimeStatus.Idle;
    }

    private enum NodeRuntimeStatus
    {
        Idle,
        Running,
        Pending,
        Paused,
        Succeeded,
        Failed,
    }
}

public sealed class ConfirmationItemViewModel : ViewModelBase
{
    private bool _isSelected;

    public ConfirmationItemViewModel(
        ConfirmationLogEntry entry,
        DisplayNameService displayNames,
        Action<ConfirmationItemViewModel> select)
    {
        ConfirmationId = entry.ConfirmationId;
        Summary = entry.Summary;
        State = entry.State;
        Diff = entry.Diff;
        WorkflowId = entry.WorkflowId ?? string.Empty;
        RunId = entry.RunId ?? string.Empty;
        StateText = displayNames.Format("ui.workspace.confirmation.state", new Dictionary<string, string>
        {
            ["state"] = State,
        });
        SourceText = displayNames.Format("ui.workspace.confirmation.source", new Dictionary<string, string>
        {
            ["workflow"] = string.IsNullOrWhiteSpace(WorkflowId)
                ? displayNames.Text("ui.common.none")
                : WorkflowId,
            ["run"] = string.IsNullOrWhiteSpace(RunId)
                ? displayNames.Text("ui.common.none")
                : RunId,
        });
        SelectCommand = new RelayCommand(() => select(this));
    }

    public string ConfirmationId { get; }
    public string Summary { get; }
    public string State { get; }
    public string StateText { get; }
    public string SourceText { get; }
    public string Diff { get; }
    public string WorkflowId { get; }
    public string RunId { get; }
    public RelayCommand SelectCommand { get; }
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }
}

/// <summary>
/// 审批历史里的一条已决议确认项（U187-A）。
///
/// **刻意不带 SelectCommand / IsSelected / Diff**：它不是可审阅对象。
/// 后端 `resolve_confirmation` 对已决议项不再受理，给历史行配一个「同意/拒绝」入口
/// 只会让人点了没反应；这一份是**审计读物**，回答的是「我到底批准过什么、为什么」。
///
/// 全部字段在构造时定死（无 setter）：条目一旦决议就不再变，
/// 可变属性只会让人以为这里能改。
/// </summary>
public sealed class ResolvedConfirmationItemViewModel : ViewModelBase
{
    public ResolvedConfirmationItemViewModel(ConfirmationLogEntry entry, DisplayNameService displayNames)
    {
        ConfirmationId = entry.ConfirmationId;
        Summary = string.IsNullOrWhiteSpace(entry.Summary) ? entry.ConfirmationId : entry.Summary;
        State = entry.State;
        IsApproved = string.Equals(entry.State, "approved", StringComparison.OrdinalIgnoreCase);
        IsRejected = string.Equals(entry.State, "rejected", StringComparison.OrdinalIgnoreCase);
        // 决议结果走三个独立文案键而不是把后端状态串直接印出来：
        // `auto_audited` 这种 snake_case 内部值对作者没有意义。
        // 未知状态（后端将来加档位）回落到原值——比显示 `[key]` 或空白有用。
        ResultText = entry.State.ToLowerInvariant() switch
        {
            "approved" => displayNames.Text("ui.workspace.confirmation.history.result.approved"),
            "rejected" => displayNames.Text("ui.workspace.confirmation.history.result.rejected"),
            "auto_audited" => displayNames.Text("ui.workspace.confirmation.history.result.auto_audited"),
            _ => entry.State,
        };
        // Kind 是后端一直在发、前端从来没用的字段（U187-D 也记了这条）。
        // 历史里必须显示：不然 summarizer 一次批量产出的四类确认项在列表里长得一模一样。
        KindText = displayNames.Format(
            "ui.workspace.confirmation.history.kind",
            new Dictionary<string, string> { ["kind"] = entry.Kind });
        // `handling_method` 有理由时装的是 review_reason，没理由时装的是状态词本身
        // （见 commands.rs `confirmation_log_entry_from_runtime`）。
        // 因此必须把「等于状态词」的情形判成「没写理由」，否则历史里会出现
        // 「理由：rejected」这种把内部值当人话印出来的行。
        var reason = entry.HandlingMethod?.Trim() ?? string.Empty;
        HasReason = reason.Length > 0
            && !string.Equals(reason, entry.State, StringComparison.OrdinalIgnoreCase)
            && !IsStateSentinel(reason);
        ReasonText = HasReason
            ? displayNames.Format(
                "ui.workspace.confirmation.history.reason",
                new Dictionary<string, string> { ["reason"] = reason })
            : string.Empty;
        DecidedAtText = entry.TimestampMs > 0
            ? displayNames.Format(
                "ui.workspace.confirmation.history.decided_at",
                new Dictionary<string, string> { ["time"] = FormatTimestamp(entry.TimestampMs) })
            : string.Empty;
    }

    public string ConfirmationId { get; }
    public string Summary { get; }
    public string State { get; }
    public string ResultText { get; }
    public string KindText { get; }
    public string ReasonText { get; }
    public bool HasReason { get; }
    public string DecidedAtText { get; }
    /// <summary>结果着色只用语义状态位，色值全部来自主题（视图侧绑 Classes）。</summary>
    public bool IsApproved { get; }
    public bool IsRejected { get; }

    /// <summary>后端在「无理由」时塞进 handling_method 的四个哨兵词。</summary>
    private static bool IsStateSentinel(string value) =>
        value is "pending" or "approved" or "rejected" or "auto_audited";

    private static string FormatTimestamp(long ms)
    {
        try
        {
            // 固定格式走 InvariantCulture：这里的 `yyyy-MM-dd HH:mm` 是**格式串**而非
            // 本地化偏好，跟 CurrentCulture 会在非公历日历下把年份印成别的纪元。
            // 时区仍取本地（LocalDateTime）——作者关心的是「我当时几点批的」。
            return DateTimeOffset.FromUnixTimeMilliseconds(ms).LocalDateTime.ToString(
                "yyyy-MM-dd HH:mm",
                System.Globalization.CultureInfo.InvariantCulture);
        }
        catch (ArgumentOutOfRangeException)
        {
            // 越界时间戳（旧日志或时钟异常）不该让整个历史面板打不开。
            return ms.ToString(System.Globalization.CultureInfo.InvariantCulture);
        }
    }
}

public sealed class WorkflowEdgeViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly Action _markDirty;
    private bool _isSelected;
    private string _sourceHandle;
    private string _targetHandle;
    private string _label;
    private string _dataJson;
    private string _forwardAlias;
    private string _reverseAlias;
    private string _forwardTemplate;
    private string _reverseTemplate;
    private string _maxCommunicationCount;
    private bool _hasLabelLayout;
    private bool _labelLayoutVisible = true;
    // U178-B：页面级「显示详情」在边上的投影（边标签胶囊按它显隐）。
    // 与节点同理：原先是模板内的祖先绑定，成本落在重挂载路径上。
    private bool _showCanvasDetails = true;
    private double _labelOffsetX;
    private double _labelOffsetY;

    public WorkflowEdgeViewModel(
        CanvasEdge edge,
        DisplayNameService displayNames,
        Action<WorkflowEdgeViewModel> select,
        Action markDirty)
    {
        _displayNames = displayNames;
        _markDirty = markDirty;
        Id = edge.Id;
        Source = edge.Source;
        Target = edge.Target;
        Kind = edge.Kind;
        _sourceHandle = edge.SourceHandle;
        _targetHandle = edge.TargetHandle;
        _label = edge.Label ?? string.Empty;
        _dataJson = EdgeDataToJson(edge.Data);
        _forwardAlias = ReadDataString(edge.Data, "forward_alias", "forward_output");
        _reverseAlias = ReadDataString(edge.Data, "reverse_alias", "reverse_output");
        _forwardTemplate = ReadDataString(edge.Data, "forward_template", displayNames.Text("ui.workspace.edge.default_forward_template"));
        _reverseTemplate = ReadDataString(edge.Data, "reverse_template", displayNames.Text("ui.workspace.edge.default_reverse_template"));
        _maxCommunicationCount = ReadDataString(edge.Data, "max_communication_count", "2");
        _sourceLabel = edge.Source;
        _targetLabel = edge.Target;
        SelectCommand = new RelayCommand(() => select(this));
    }

    public string Id { get; }
    public string Source { get; }
    public string Target { get; }
    public string Kind { get; }
    private string _sourceLabel;
    private string _targetLabel;
    public string Title => $"{_sourceLabel} → {_targetLabel}";
    public string KindDisplay
    {
        get
        {
            var key = Kind.ToLowerInvariant() switch
            {
                "control" => "ui.workspace.edge.kind.control",
                "communication" => "ui.workspace.edge.kind.communication",
                _ => "ui.workspace.edge.kind.data",
            };
            return _displayNames.Text(key);
        }
    }

    public void SetEndpointLabels(string sourceLabel, string targetLabel)
    {
        _sourceLabel = string.IsNullOrWhiteSpace(sourceLabel) ? Source : sourceLabel;
        _targetLabel = string.IsNullOrWhiteSpace(targetLabel) ? Target : targetLabel;
        OnPropertyChanged(nameof(Title));
    }
    public string SourceHandle { get => _sourceHandle; set => SetProperty(ref _sourceHandle, value); }
    public string TargetHandle { get => _targetHandle; set => SetProperty(ref _targetHandle, value); }
    public string Label { get => _label; set => SetProperty(ref _label, value); }
    public string DataJson { get => _dataJson; set => SetProperty(ref _dataJson, value); }
    public string ForwardAlias { get => _forwardAlias; set => SetProperty(ref _forwardAlias, value); }
    public string ReverseAlias { get => _reverseAlias; set => SetProperty(ref _reverseAlias, value); }
    public string ForwardTemplate { get => _forwardTemplate; set => SetProperty(ref _forwardTemplate, value); }
    public string ReverseTemplate { get => _reverseTemplate; set => SetProperty(ref _reverseTemplate, value); }
    public string ForwardTemplatePreview => TemplatePreview(ForwardTemplate, "forward_output", _displayNames.Text("ui.workspace.edge.preview_forward_value"));
    public string ReverseTemplatePreview => TemplatePreview(ReverseTemplate, "reverse_output", _displayNames.Text("ui.workspace.edge.preview_reverse_value"));
    public string MaxCommunicationCount { get => _maxCommunicationCount; set => SetProperty(ref _maxCommunicationCount, value); }
    public RelayCommand SelectCommand { get; }
    public bool IsSelected
    {
        get => _isSelected;
        set
        {
            if (SetProperty(ref _isSelected, value))
            {
                OnPropertyChanged(nameof(StrokeThickness));
                OnPropertyChanged(nameof(StrokeOpacity));
            }
        }
    }
    public bool IsCommunication => string.Equals(Kind, "communication", StringComparison.OrdinalIgnoreCase);
    public bool IsControl => string.Equals(Kind, "control", StringComparison.OrdinalIgnoreCase);
    public bool IsData => !IsCommunication && !IsControl;
    /// <summary>数据边可改别名/句柄；执行边一般只改标签；通信边用通信专用字段。</summary>
    public bool ShowHandleFields => IsData;
    public bool ShowLabelField => true;
    public bool ShowCommunicationFields => IsCommunication;
    /// <summary>W14：选中边加粗；通信边默认略粗。点阵背景上略加粗/加不透明以保住可读性。</summary>
    public double StrokeThickness =>
        IsSelected ? 3.4 : (IsCommunication ? 2.6 : 2.0);

    /// <summary>W14：选中边不透明，未选中略淡。</summary>
    public double StrokeOpacity => IsSelected ? 1.0 : 0.95;
    public Geometry EdgePath { get; private set; } = new PathGeometry();
    public double LabelX { get; private set; }
    public double LabelY { get; private set; }
    public double LabelAnchorX { get; private set; }
    public double LabelAnchorY { get; private set; }
    public double LabelTangentX { get; private set; } = 1;
    public double LabelTangentY { get; private set; }
    /// <summary>入脚旁名称：优先 label / alias；不默认甩类型名（避免线上噪点）。</summary>
    public string MidpointLabel
    {
        get
        {
            if (!string.IsNullOrWhiteSpace(Label))
            {
                return Label;
            }
            if (IsCommunication && !string.IsNullOrWhiteSpace(ForwardAlias))
            {
                return ForwardAlias;
            }
            // 数据边：显示目标句柄短名
            if (IsData && !string.IsNullOrWhiteSpace(TargetHandle))
            {
                return TargetHandle;
            }

            return string.Empty;
        }
    }
    public bool HasMidpointLabel => !string.IsNullOrWhiteSpace(MidpointLabel);
    public bool IsCanvasLabelVisible => HasMidpointLabel && _labelLayoutVisible;

    /// <summary>U178-B：页面级「显示详情」开关在本边上的投影（由页面 VM 下推）。</summary>
    public bool ShowCanvasDetails
    {
        get => _showCanvasDetails;
        internal set => SetProperty(ref _showCanvasDetails, value);
    }

    public void SetLabelLayout(double x, double y, bool isVisible)
    {
        _hasLabelLayout = true;
        _labelOffsetX = x - LabelAnchorX;
        _labelOffsetY = y - LabelAnchorY;
        if (Math.Abs(LabelX - x) > 0.01)
        {
            LabelX = x;
            OnPropertyChanged(nameof(LabelX));
        }
        if (Math.Abs(LabelY - y) > 0.01)
        {
            LabelY = y;
            OnPropertyChanged(nameof(LabelY));
        }
        if (_labelLayoutVisible != isVisible)
        {
            _labelLayoutVisible = isVisible;
            OnPropertyChanged(nameof(IsCanvasLabelVisible));
        }
    }

    /// <param name="sourceMirrored">源节点是否镜像执行口（循环节点）。</param>
    /// <param name="targetMirrored">目标节点是否镜像执行口。</param>
    public void UpdateEdgePath(
        double sourceX,
        double sourceY,
        double targetX,
        double targetY,
        bool sourceMirrored = false,
        bool targetMirrored = false)
    {
        var sourceResolved = NodePortSpec.TryResolveKind(SourceHandle, out var sourceKind, out _);
        if (!sourceResolved)
        {
            sourceKind = NodePortKind.Data;
        }
        var targetResolved = NodePortSpec.TryResolveKind(TargetHandle, out var targetKind, out _);
        if (!targetResolved)
        {
            targetKind = NodePortKind.Data;
        }
        if (string.Equals(Kind, "communication", StringComparison.OrdinalIgnoreCase))
        {
            sourceKind = NodePortKind.Communication;
            targetKind = NodePortKind.Communication;
        }

        // 按 handle 中心起止（支持多数据入索引）；循环节点执行口左右镜像。
        var (sx, sy) = sourceResolved
            ? NodePortSpec.LocalCenterForHandle(SourceHandle)
            : NodePortSpec.LocalCenter(sourceKind, NodePortDirection.Out);
        var (tx, ty) = targetResolved
            ? NodePortSpec.LocalCenterForHandle(TargetHandle)
            : NodePortSpec.LocalCenter(targetKind, NodePortDirection.In);
        (sx, sy) = NodePortSpec.MirrorExecIfLoop((sx, sy), SourceHandle, sourceMirrored);
        (tx, ty) = NodePortSpec.MirrorExecIfLoop((tx, ty), TargetHandle, targetMirrored);

        var startX = sourceX + sx;
        var startY = sourceY + sy;
        var endX = targetX + tx;
        var endY = targetY + ty;
        UpdateEdgePathFromAnchors(startX, startY, endX, endY, sourceKind, targetKind);
    }

    /// <summary>按视图测得的真实引脚中心绘制连线。</summary>
    public void UpdateEdgePathFromAnchors(
        double startX,
        double startY,
        double endX,
        double endY,
        NodePortKind sourceKind,
        NodePortKind targetKind)
    {
        var isComm = sourceKind == NodePortKind.Communication
                     || targetKind == NodePortKind.Communication
                     || IsCommunication;
        var spec = NodePortSpec.BuildEdgePath(startX, startY, endX, endY, isComm);
        var geometry = new PathGeometry();
        var figure = new PathFigure
        {
            StartPoint = spec.Start,
            IsClosed = false,
            IsFilled = false,
        };
        figure.Segments ??= new PathSegments();
        figure.Segments.Add(new BezierSegment
        {
            Point1 = spec.Control1,
            Point2 = spec.Control2,
            Point3 = spec.End,
        });
        geometry.Figures ??= new PathFigures();
        geometry.Figures.Add(figure);
        EdgePath = geometry;
        var midpoint = spec.Midpoint;
        var tangent = spec.MidpointTangent;
        LabelAnchorX = midpoint.X;
        LabelAnchorY = midpoint.Y;
        LabelTangentX = tangent.X;
        LabelTangentY = tangent.Y;
        if (!_hasLabelLayout)
        {
            var (width, height) = CanvasEdgeLabelLayoutHelpers.FallbackSize(MidpointLabel);
            var magnitude = Math.Sqrt((tangent.X * tangent.X) + (tangent.Y * tangent.Y));
            var normalX = magnitude < 0.001 ? 0 : -tangent.Y / magnitude;
            var normalY = magnitude < 0.001 ? 1 : tangent.X / magnitude;
            var normalOffset = (height * 0.5) + 9;
            _labelOffsetX = (normalX * normalOffset) - (width * 0.5);
            _labelOffsetY = (normalY * normalOffset) - (height * 0.5);
        }
        LabelX = LabelAnchorX + _labelOffsetX;
        LabelY = LabelAnchorY + _labelOffsetY;

        OnPropertyChanged(nameof(EdgePath));
        OnPropertyChanged(nameof(LabelX));
        OnPropertyChanged(nameof(LabelY));
        OnPropertyChanged(nameof(MidpointLabel));
        OnPropertyChanged(nameof(HasMidpointLabel));
    }

    protected override void OnPropertyChanged(string? propertyName = null)
    {
        base.OnPropertyChanged(propertyName);
        if (propertyName is nameof(SourceHandle) or nameof(TargetHandle) or nameof(Label)
            or nameof(DataJson) or nameof(ForwardAlias) or nameof(ReverseAlias)
            or nameof(ForwardTemplate) or nameof(ReverseTemplate) or nameof(MaxCommunicationCount))
        {
            if (propertyName is nameof(ForwardTemplate))
            {
                OnPropertyChanged(nameof(ForwardTemplatePreview));
            }
            if (propertyName is nameof(ReverseTemplate))
            {
                OnPropertyChanged(nameof(ReverseTemplatePreview));
            }
            if (propertyName is nameof(Label) or nameof(ForwardAlias))
            {
                OnPropertyChanged(nameof(MidpointLabel));
                OnPropertyChanged(nameof(HasMidpointLabel));
                OnPropertyChanged(nameof(IsCanvasLabelVisible));
            }
            _markDirty();
        }
    }

    public CanvasEdge ToCanvasEdge()
    {
        object? data = IsCommunication
            ? CommunicationData()
            : string.IsNullOrWhiteSpace(DataJson)
                ? new Dictionary<string, object?>()
                : JsonNode.Parse(DataJson);
        return new CanvasEdge(
            Id,
            Source,
            Target,
            SourceHandle,
            TargetHandle,
            Kind,
            string.IsNullOrWhiteSpace(Label) ? null : Label,
            data);
    }

    private Dictionary<string, object?> CommunicationData()
    {
        var count = uint.TryParse(MaxCommunicationCount, out var parsed) && parsed > 0 ? parsed : 2;
        return new Dictionary<string, object?>
        {
            ["forward_alias"] = string.IsNullOrWhiteSpace(ForwardAlias) ? "forward_output" : ForwardAlias,
            ["reverse_alias"] = string.IsNullOrWhiteSpace(ReverseAlias) ? "reverse_output" : ReverseAlias,
            ["forward_template"] = string.IsNullOrWhiteSpace(ForwardTemplate)
                ? _displayNames.Text("ui.workspace.edge.default_forward_template")
                : ForwardTemplate,
            ["reverse_template"] = string.IsNullOrWhiteSpace(ReverseTemplate)
                ? _displayNames.Text("ui.workspace.edge.default_reverse_template")
                : ReverseTemplate,
            ["max_communication_count"] = count,
        };
    }

    private static string EdgeDataToJson(object? data)
    {
        if (data is null)
        {
            return "{}";
        }
        if (data is JsonElement element)
        {
            return element.GetRawText();
        }
        return JsonSerializer.Serialize(data, new JsonSerializerOptions(JsonSerializerDefaults.Web));
    }

    private static string ReadDataString(object? data, string key, string fallback)
    {
        if (data is JsonElement element && element.ValueKind == JsonValueKind.Object && element.TryGetProperty(key, out var property))
        {
            return property.ValueKind switch
            {
                JsonValueKind.String => property.GetString() ?? fallback,
                JsonValueKind.Number => property.ToString(),
                JsonValueKind.True => "true",
                JsonValueKind.False => "false",
                _ => fallback,
            };
        }
        if (data is JsonObject jsonObject && jsonObject.TryGetPropertyValue(key, out var node) && node is not null)
        {
            return node.GetValueKind() == JsonValueKind.String ? node.GetValue<string>() : node.ToJsonString();
        }
        if (data is Dictionary<string, object?> dictionary && dictionary.TryGetValue(key, out var value) && value is not null)
        {
            return value.ToString() ?? fallback;
        }
        return fallback;
    }

    private static string TemplatePreview(string template, string alias, string value)
    {
        return (string.IsNullOrWhiteSpace(template) ? "{{input." + alias + "}}" : template)
            .Replace("{{input." + alias + "}}", value, StringComparison.Ordinal);
    }
}
