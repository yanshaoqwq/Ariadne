using System.Collections.ObjectModel;
using System.Text.Json;
using System.Text.Json.Nodes;
using Avalonia.Controls;
using Avalonia.Media;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class WorkspacePageViewModel : ViewModelBase, IUnsavedChangesGuard, IProjectDataReloadable
{
    private const string DefaultWorkflowId = "default";
    private const double MinRightPanelWidth = 260;
    private const double MaxRightPanelWidth = 560;
    // 收起后列宽为 0：展开只靠画布右缘 pill，避免窄条 + float 双箭头
    private const double CollapsedRightPanelWidth = 0;
    private static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web);

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private bool _isRightPanelOpen = true;
    // 默认右栏略窄，多留给画布
    private GridLength _rightPanelColumnWidth = new(280);
    private bool _isLibraryOpen = true;
    private bool _isExecutionPanel;
    private bool _isProjectAiTab = true;
    private double _canvasZoom = 1.0;
    private string _statusText = string.Empty;
    private bool _hasUnsavedChanges;
    private string _savedSnapshot = string.Empty;
    private bool _suppressSnapshotChecks;
    private bool _deferDirtyRefresh;
    private int _nextNodeNumber = 1;
    private string _projectAiMessage = string.Empty;
    private string _projectAiAnswer;
    private bool _isConfirmationPanelExpanded = true;
    private string _currentRunId = string.Empty;
    /// <summary>W8：最近一次已知 run 生命周期状态（running/paused/…）。</summary>
    private string _currentRunStatus = string.Empty;
    private string _selectedWorkflowId = DefaultWorkflowId;
    private string _currentWorkflowName = "Default";
    private bool _suppressWorkflowSelectionChange;
    private long _workflowEventCursor;
    private CancellationTokenSource? _workflowEventPollingCts;
    private string _confirmationReason = string.Empty;
    private WorkflowOperation? _selectedInDoubtOperation;
    private string _inDoubtResponseJson = string.Empty;
    private string _inDoubtStopReason = string.Empty;
    private string _annotationTitle = string.Empty;
    private IReadOnlyList<CanvasEdge> _edges = Array.Empty<CanvasEdge>();
    private readonly List<string> _undoSnapshots = new();
    private readonly List<string> _redoSnapshots = new();
    private readonly List<ProjectAiChatMessage> _projectAiHistory = new();
    private CanvasNode? _clipboardNode;
    private WorkflowNodeViewModel? _selectedNode;
    private ConfirmationItemViewModel? _selectedConfirmation;
    private WorkflowEdgeViewModel? _selectedEdge;
    private WorkflowLoadState _workflowLoadState = WorkflowLoadState.NoProject;
    private string? _workflowContentRevision;
    private string _defaultLlmProviderId = string.Empty;

    public WorkspacePageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        ToggleRightPanelCommand = new RelayCommand(() => IsRightPanelOpen = !IsRightPanelOpen);
        ToggleLibraryCommand = new RelayCommand(() => IsLibraryOpen = !IsLibraryOpen);
        ZoomInCommand = new RelayCommand(() => AdjustCanvasZoom(0.1));
        ZoomOutCommand = new RelayCommand(() => AdjustCanvasZoom(-0.1));
        ResetZoomCommand = new RelayCommand(() => CanvasZoom = 1.0);
        ShowNodeLibraryCommand = new RelayCommand(() => IsExecutionPanel = false);
        ShowExecutionCommand = new RelayCommand(() => IsExecutionPanel = true);
        ShowProjectAiCommand = new RelayCommand(() => IsProjectAiTab = true);
        ShowNodeDetailsCommand = new RelayCommand(() => IsProjectAiTab = false);
        ImportCommand = new RelayCommand(() => _ = LoadWorkflowWithUnsavedCheckAsync());
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
        RunSelectedNodeCommand = new RelayCommand(() => _ = RunSelectedNodeAsync(), () => IsSelectedStartNode);
        // W8：按 run 生命周期门禁，而非仅 HasCurrentRun。
        PauseWorkflowCommand = new RelayCommand(() => _ = PauseWorkflowAsync(), CanPauseWorkflow);
        StopWorkflowCommand = new RelayCommand(() => _ = StopWorkflowAsync(), CanStopWorkflow);
        ResumeWorkflowCommand = new RelayCommand(() => _ = ResumeWorkflowAsync(), CanResumeWorkflow);
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
            IsConfirmationPanelExpanded = !IsConfirmationPanelExpanded);
        ApproveConfirmationCommand = new RelayCommand(() => _ = ResolveSelectedConfirmationAsync("approve"), CanResolveConfirmation);
        RejectConfirmationCommand = new RelayCommand(() => _ = ResolveSelectedConfirmationAsync("reject"), CanResolveConfirmation);
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
        _projectAiAnswer = displayNames.Text("ui.workspace.project_ai.empty");

        Nodes = new ObservableCollection<WorkflowNodeViewModel>();
        StartNodes = new ObservableCollection<WorkflowNodeViewModel>();
        WorkflowSummaries = new ObservableCollection<WorkflowSummary>();
        Confirmations = new ObservableCollection<ConfirmationItemViewModel>();
        InDoubtOperations = new ObservableCollection<WorkflowOperation>();
        Edges = new ObservableCollection<WorkflowEdgeViewModel>();
        RelatedEdges = new ObservableCollection<WorkflowEdgeViewModel>();
        ProjectAiBubbles = new ObservableCollection<ChatBubbleViewModel>();
        EntryNodes = new ObservableCollection<NodeLibraryItemViewModel>
        {
            new("start", displayNames.Text("ui.workspace.start_node.title"), () => AddNode("start")),
        };
        WritingAgents = new ObservableCollection<NodeLibraryItemViewModel>
        {
            new("outliner", displayNames.Text("agent.outliner"), () => AddNode("outliner")),
            new("designer", displayNames.Text("agent.designer"), () => AddNode("designer")),
            new("planner", displayNames.Text("agent.planner"), () => AddNode("planner")),
            new("detail", displayNames.Text("agent.detail"), () => AddNode("detail")),
            new("writer", displayNames.Text("agent.writer"), () => AddNode("writer")),
            new("critic", displayNames.Text("agent.critic"), () => AddNode("critic")),
            new("prudent", displayNames.Text("agent.prudent"), () => AddNode("prudent")),
            new("polisher", displayNames.Text("agent.polisher"), () => AddNode("polisher")),
            new("summarizer", displayNames.Text("agent.summarizer"), () => AddNode("summarizer")),
        };
        UtilityNodes = new ObservableCollection<NodeLibraryItemViewModel>
        {
            new("llm", displayNames.Text("ui.node.llm"), () => AddNode("llm")),
            new("document_read", displayNames.Text("ui.node.document"), () => AddNode("document_read")),
            new("search", displayNames.Text("ui.node.search"), () => AddNode("search")),
            new("condition", displayNames.Text("ui.node.condition"), () => AddNode("condition")),
            new("loop", displayNames.Text("ui.node.loop"), () => AddNode("loop")),
            new("approval", displayNames.Text("ui.node.approval"), () => AddNode("approval")),
            new("export", displayNames.Text("ui.node.export"), () => AddNode("export")),
        };

        AvailableProviderIds = new ObservableCollection<string>();
        AvailableModelIds = new ObservableCollection<string>();
        CaptureSnapshot();
        _ = InitializeWorkflowAsync();
        _ = LoadConfirmationsAsync();
        _ = LoadAvailableModelsAsync();
    }

    public ObservableCollection<string> AvailableProviderIds { get; }
    public bool HasAvailableProviderChoices => AvailableProviderIds.Count > 0;
    public ObservableCollection<string> AvailableModelIds { get; }
    public bool HasAvailableModelChoices => AvailableModelIds.Count > 0;

    public string Title => _displayNames.Text("ui.nav.workspace");
    public string SaveText => _displayNames.Text("ui.workspace.save");
    public string ImportText => _displayNames.Text("ui.workspace.import");
    public string ExportText => _displayNames.Text("ui.workspace.export");
    public string UndoText => _displayNames.Text("ui.action.undo");
    public string RedoText => _displayNames.Text("ui.action.redo");
    public string RunText => _displayNames.Text("ui.workspace.run");
    public string WorkflowSelectorText => _displayNames.Text("ui.workspace.workflow_selector");
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
    public string ConfirmationReasonText => _displayNames.Text("ui.workspace.confirmation.reason");
    public string ConfirmationReasonPlaceholder => _displayNames.Text("ui.workspace.confirmation.reason.placeholder");
    public string ApproveConfirmationText => _displayNames.Text("ui.workspace.confirmation.approve");
    public string RejectConfirmationText => _displayNames.Text("ui.workspace.confirmation.reject");
    public string PromptTemplateText => _displayNames.Text("ui.workspace.prompt_template");
    public string ModelIdText => _displayNames.Text("ui.workspace.model_id");
    public string NodeBudgetText => _displayNames.Text("ui.workspace.node_budget");
    public string NodeTimeoutText => _displayNames.Text("ui.workspace.node_timeout_seconds");
    public string OptionalPlaceholder => _displayNames.Text("ui.common.optional");
    public string SecondsUnitText => _displayNames.Text("ui.common.unit.seconds");
    public string BreakpointText => _displayNames.Text("ui.workspace.breakpoint");
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
    public string PortControlInTip => _displayNames.Text("ui.workspace.port.control_in");
    public string PortControlOutTip => _displayNames.Text("ui.workspace.port.control_out");
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
    public string ExportFormatLabel => _displayNames.Text("ui.workspace.export_node.format");
    public string ExportTitleLabel => _displayNames.Text("ui.workspace.export_node.title_field");
    public string SummarizerNodeTitle => _displayNames.Text("ui.workspace.summarizer.title");
    public string SummarizerNodeHint => _displayNames.Text("ui.workspace.summarizer.hint");
    public string SummarizerProviderIdLabel => _displayNames.Text("ui.workspace.summarizer.provider_id");
    public string SummarizerProviderIdPlaceholder => _displayNames.Text("ui.workspace.summarizer.provider_id_placeholder");
    public string SummarizerChapterIdLabel => _displayNames.Text("ui.workspace.summarizer.chapter_id");
    public string SummarizerChapterIdPlaceholder => _displayNames.Text("ui.workspace.summarizer.chapter_id_placeholder");
    public string SummarizerChapterDocumentIdLabel => _displayNames.Text("ui.workspace.summarizer.chapter_document_id");
    public string SummarizerChapterDocumentIdPlaceholder => _displayNames.Text("ui.workspace.summarizer.chapter_document_id_placeholder");
    public string SummarizerChapterTextAliasLabel => _displayNames.Text("ui.workspace.summarizer.chapter_text_alias");
    public string SummarizerChapterTextAliasHint => _displayNames.Text("ui.workspace.summarizer.chapter_text_alias_hint");
    public string SummarizerAutoModeText => _displayNames.Text("ui.workspace.summarizer.auto_mode");
    public string DataInPinsLabel => _displayNames.Text("ui.workspace.port.data_in");
    public string AddDataInPinText => _displayNames.Text("ui.workspace.pin.add_data_in");
    public string RemoveDataInPinText => _displayNames.Text("ui.workspace.pin.remove_data_in");
    public string ZoomInText => _displayNames.Text("ui.workspace.zoom_in");
    public string ZoomOutText => _displayNames.Text("ui.workspace.zoom_out");
    public string ResetZoomText => _displayNames.Text("ui.workspace.zoom_reset");
    public string ZoomInGlyphText => _displayNames.Text("ui.workspace.zoom_in_glyph");
    public string ZoomOutGlyphText => _displayNames.Text("ui.workspace.zoom_out_glyph");
    public string MinimapText => _displayNames.Text("ui.workspace.minimap");
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
                OnPropertyChanged(nameof(RightPanelSplitterWidth));
                OnPropertyChanged(nameof(RightPanelColumnWidth));
            }
        }
    }
    public RelayCommand ToggleRightPanelCommand { get; }
    public GridLength RightPanelSplitterWidth => IsRightPanelOpen ? new GridLength(4) : new GridLength(0);
    public GridLength RightPanelColumnWidth
    {
        get => IsRightPanelOpen ? _rightPanelColumnWidth : new GridLength(CollapsedRightPanelWidth);
        set
        {
            if (!IsRightPanelOpen)
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
    public bool IsLibraryOpen
    {
        get => _isLibraryOpen;
        set
        {
            if (SetProperty(ref _isLibraryOpen, value))
            {
                OnPropertyChanged(nameof(BottomPanelShowsCollapseGlyph));
            }
        }
    }
    public RelayCommand ToggleLibraryCommand { get; }
    public double CanvasZoom
    {
        get => _canvasZoom;
        private set => SetCanvasZoom(value);
    }

    /// <summary>W2：产品路径设置缩放（FitView / 滚轮 / 工具栏共用）。</summary>
    public void SetCanvasZoom(double value)
    {
        var clamped = Math.Clamp(
            Math.Round(value, 2),
            CanvasViewportHelpers.MinZoom,
            CanvasViewportHelpers.MaxZoom);
        if (SetProperty(ref _canvasZoom, clamped, nameof(CanvasZoom)))
        {
            OnPropertyChanged(nameof(CanvasZoomText));
        }
    }

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
        get => _isProjectAiTab;
        set
        {
            if (SetProperty(ref _isProjectAiTab, value))
            {
                OnPropertyChanged(nameof(IsNodeDetailsTab));
            }
        }
    }

    public bool IsNodeDetailsTab => !_isProjectAiTab;
    public RelayCommand ShowProjectAiCommand { get; }
    public RelayCommand ShowNodeDetailsCommand { get; }
    public RelayCommand ImportCommand { get; }
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
    public Action? RequestFitView { get; set; }

    public string StatusText { get => _statusText; set => SetProperty(ref _statusText, value); }
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
    public string CurrentRunId
    {
        get => _currentRunId;
        set
        {
            if (SetProperty(ref _currentRunId, value))
            {
                OnPropertyChanged(nameof(CurrentRunValueText));
                NotifyRunCommandStates();
                _ = LoadInDoubtOperationsAsync();
            }
        }
    }
    public string SelectedWorkflowId
    {
        get => _selectedWorkflowId;
        set
        {
            var next = string.IsNullOrWhiteSpace(value) ? DefaultWorkflowId : value;
            if (_suppressWorkflowSelectionChange)
            {
                SetSelectedWorkflowId(next);
                return;
            }
            if (!string.Equals(next, _selectedWorkflowId, StringComparison.Ordinal))
            {
                _ = SwitchWorkflowAsync(next);
            }
        }
    }
    public string CurrentWorkflowName
    {
        get => _currentWorkflowName;
        private set => SetProperty(ref _currentWorkflowName, value);
    }
    public string ConfirmationReason { get => _confirmationReason; set => SetProperty(ref _confirmationReason, value); }
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
            : SaveText;

    /// <summary>W16：底部面板开合标签随当前模式（节点库 / 执行）变化。</summary>
    public string BottomPanelToggleText =>
        IsExecutionPanel
            ? _displayNames.Text("ui.workspace.execution")
            : _displayNames.Text("ui.workspace.node_library");

    /// <summary>W16：开=收起箭头(上)，关=展开箭头(下) — 与动作语义一致。</summary>
    public bool BottomPanelShowsCollapseGlyph => IsLibraryOpen;

    public ObservableCollection<WorkflowNodeViewModel> Nodes { get; }
    public ObservableCollection<WorkflowNodeViewModel> StartNodes { get; }
    public ObservableCollection<WorkflowSummary> WorkflowSummaries { get; }
    public ObservableCollection<NodeLibraryItemViewModel> EntryNodes { get; }
    public ObservableCollection<NodeLibraryItemViewModel> WritingAgents { get; }
    public ObservableCollection<NodeLibraryItemViewModel> UtilityNodes { get; }
    public ObservableCollection<ConfirmationItemViewModel> Confirmations { get; }
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
                NotifyNodeCommandStates();
            }
        }
    }

    public bool HasSelectedNode => SelectedNode is not null;
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
                OnPropertyChanged(nameof(HasSelectedConfirmation));
                OnPropertyChanged(nameof(HasPendingConfirmations));
                NotifyConfirmationCommandStates();
            }
        }
    }

    public bool HasSelectedConfirmation => SelectedConfirmation is not null;
    public bool HasPendingConfirmations => Confirmations.Count > 0;
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
                OnPropertyChanged(nameof(ShowConfirmationFullPanel));
                OnPropertyChanged(nameof(ShowConfirmationBanner));
            }
        }
    }
    public bool ShowConfirmationFullPanel => HasPendingConfirmations && IsConfirmationPanelExpanded;
    public bool ShowConfirmationBanner => HasPendingConfirmations && !IsConfirmationPanelExpanded;

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
        HasCurrentRun() && CanvasRunControlHelpers.CanPause(_currentRunStatus);

    public bool CanResumeWorkflow() =>
        HasCurrentRun() && CanvasRunControlHelpers.CanResume(_currentRunStatus);

    public bool CanStopWorkflow() =>
        HasCurrentRun() && CanvasRunControlHelpers.CanStop(_currentRunStatus);

    public string CurrentRunStatus
    {
        get => _currentRunStatus;
        private set
        {
            if (SetProperty(ref _currentRunStatus, value))
            {
                NotifyRunCommandStates();
            }
        }
    }
    private bool HasProjectAiMessage() => !string.IsNullOrWhiteSpace(ProjectAiMessage);
    private bool CanResolveConfirmation() =>
        SelectedConfirmation is not null
        && (!string.IsNullOrWhiteSpace(SelectedConfirmation.RunId)
            || !string.IsNullOrWhiteSpace(CurrentRunId));
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
        foreach (var node in Nodes)
        {
            node.SetPortDragHighlight(
                controlIn: CanConnectPorts(sourceNodeId, sourceKind, sourceDirection, node.Id, NodePortKind.Control, NodePortDirection.In),
                controlOut: CanConnectPorts(sourceNodeId, sourceKind, sourceDirection, node.Id, NodePortKind.Control, NodePortDirection.Out),
                dataIn: CanConnectPorts(sourceNodeId, sourceKind, sourceDirection, node.Id, NodePortKind.Data, NodePortDirection.In),
                dataOut: CanConnectPorts(sourceNodeId, sourceKind, sourceDirection, node.Id, NodePortKind.Data, NodePortDirection.Out),
                communication: CanConnectPorts(sourceNodeId, sourceKind, sourceDirection, node.Id, NodePortKind.Communication, NodePortDirection.Both));
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
        var aliasBase = !string.IsNullOrWhiteSpace(summarizerAlias)
            ? summarizerAlias
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
    }

    private void NotifyRunCommandStates()
    {
        PauseWorkflowCommand.NotifyCanExecuteChanged();
        StopWorkflowCommand.NotifyCanExecuteChanged();
        ResumeWorkflowCommand.NotifyCanExecuteChanged();
        NotifyConfirmationCommandStates();
    }

    private void NotifyConfirmationCommandStates()
    {
        ApproveConfirmationCommand.NotifyCanExecuteChanged();
        RejectConfirmationCommand.NotifyCanExecuteChanged();
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
            defaultWorkDir: nodeType == "start" ? _displayNames.Text("ui.workspace.start_node.default_work_dir") : string.Empty,
            x: Math.Max(0, x),
            y: Math.Max(0, y),
            _backend,
            () => CurrentWorkflowId,
            () => SelectNode(node: null),
            RefreshDirtyState);
        // agent 才填提示词；通用节点用角色配置默认值
        if (node.ShowPromptEditor)
        {
            node.PromptTemplate = Localization.PromptCatalog.ResolveNodePrompt(nodeType);
        }

        SeedUtilityDefaults(node);
        AttachNodeCommands(node);
        Nodes.Add(node);
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
            if (string.IsNullOrWhiteSpace(node.SummarizerProviderId))
            {
                node.SummarizerProviderId = _defaultLlmProviderId;
            }

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
        var node = new WorkflowNodeViewModel(
            graphNode.Id,
            graphNode.Type,
            graphNode.Label ?? NodeLabel(graphNode.Type),
            ReadString(data, "work_dir"),
            graphNode.Position?.X ?? 120 + ((Nodes.Count % 4) * 230),
            graphNode.Position?.Y ?? 80 + ((Nodes.Count / 4) * 170),
            _backend,
            () => CurrentWorkflowId,
            () => SelectNode(node: null),
            RefreshDirtyState)
        {
            Name = ReadString(data, "name", graphNode.Label ?? NodeLabel(graphNode.Type)),
            UserNote = ReadString(data, "user_note"),
            ExposedAsTool = ReadBool(data, "expose_as_tool", graphNode.Type == "start"),
            PromptTemplate = ReadString(data, "prompt_template"),
            ModelId = ReadString(data, "model_id"),
            BudgetUsd = ReadString(data, "budget_usd"),
            TimeoutMs = ReadString(data, "timeout_ms"),
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
            SummarizerProviderId = ReadString(data, "provider_id"),
            SummarizerChapterId = ReadString(data, "chapter_id"),
            SummarizerChapterDocumentId = ReadString(data, "chapter_document_id"),
            SummarizerChapterTextAlias = ReadString(data, "chapter_text_alias", "chapter_text"),
            SummarizerAutoMode = ReadBool(data, "auto_mode", false),
        };
        LoadStopCondition(node, data);
        // 画布已有节点若未存提示词，用 prompt_list 默认补全（不覆盖用户已写内容）；通用/导入节点不填 agent 模板
        if (string.IsNullOrWhiteSpace(node.PromptTemplate) && node.ShowPromptEditor)
        {
            node.PromptTemplate = Localization.PromptCatalog.ResolveNodePrompt(graphNode.Type);
        }
        node.RestoreDataInPins(ReadStringList(data, "data_in_handles"));
        // 必须保留 tool_enabled / input_aliases 等非 UI 键，否则 SaveWorkflowGraph 会整表冲掉
        node.RetainOpaqueData(data);
        AttachNodeCommands(node);
        return node;
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

            if (dict.TryGetValue("expected", out var e) && e is not null)
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

            if (el.TryGetProperty("expected", out var e))
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
        node.RunCommand = new RelayCommand(() => _ = RunNodeAsync(node));
        node.DataInPinRemoved = handle => OnDataInPinRemoved(node, handle);
        node.SummarizerChapterTextAliasChanged = (previous, current) =>
            OnSummarizerChapterTextAliasChanged(node, previous, current);
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
                n.X, n.Y, NodePortSpec.NodeWidth, NodePortSpec.NodeBodyHeight,
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
        CanvasZoom += delta;
        StatusText = CanvasZoomText;
    }

    private static GridLength NormalizeRightPanelWidth(GridLength value)
    {
        if (value.IsStar)
        {
            return new GridLength(360);
        }
        var width = value.IsAuto ? 360 : value.Value;
        return new GridLength(Math.Clamp(width, MinRightPanelWidth, MaxRightPanelWidth));
    }

    private async Task InitializeWorkflowAsync()
    {
        // 无打开项目：保持空画布，不打项目 IPC（避免 cwd 误当项目 / 英文技术报错）
        if (!_backend.HasProjectRoot)
        {
            SetWorkflowLoadState(WorkflowLoadState.NoProject);
            Nodes.Clear();
            StartNodes.Clear();
            Edges.Clear();
            WorkflowSummaries.Clear();
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
            await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
            await LoadWorkflowAsync(SelectedWorkflowId).ConfigureAwait(true);
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

    private async Task LoadAvailableModelsAsync()
    {
        try
        {
            var config = await _backend.GetProviderConfigAsync().ConfigureAwait(true);
            _defaultLlmProviderId = config.DefaultLlmProviderId?.Trim() ?? string.Empty;
            AvailableProviderIds.Clear();
            foreach (var providerId in config.Providers
                         .Where(provider => provider.Enabled)
                         .Select(provider => provider.Provider)
                         .Where(id => !string.IsNullOrWhiteSpace(id))
                         .Distinct(StringComparer.Ordinal)
                         .OrderBy(id => id, StringComparer.Ordinal))
            {
                AvailableProviderIds.Add(providerId);
            }
            AvailableModelIds.Clear();
            foreach (var modelId in config.Providers
                         .SelectMany(provider => provider.Models)
                         .Select(model => model.ModelId)
                         .Where(id => !string.IsNullOrWhiteSpace(id))
                         .Distinct(StringComparer.Ordinal)
                         .OrderBy(id => id, StringComparer.Ordinal))
            {
                AvailableModelIds.Add(modelId);
            }
            OnPropertyChanged(nameof(HasAvailableProviderChoices));
            OnPropertyChanged(nameof(HasAvailableModelChoices));
        }
        catch
        {
            // 无模型列表时保持可手填。
        }
    }

    private async Task RefreshWorkflowSummariesAsync()
    {
        if (!_backend.HasProjectRoot)
        {
            WorkflowSummaries.Clear();
            return;
        }

        try
        {
            var selected = SelectedWorkflowId;
            var summaries = await _backend.ListWorkflowGraphsAsync().ConfigureAwait(true);
            WorkflowSummaries.Clear();
            foreach (var summary in summaries)
            {
                WorkflowSummaries.Add(summary);
            }
            if (WorkflowSummaries.All(summary => summary.WorkflowId != selected))
            {
                selected = WorkflowSummaries.FirstOrDefault()?.WorkflowId ?? DefaultWorkflowId;
            }
            _suppressWorkflowSelectionChange = true;
            try
            {
                SetSelectedWorkflowId(selected);
            }
            finally
            {
                _suppressWorkflowSelectionChange = false;
            }
        }
        catch
        {
            WorkflowSummaries.Clear();
        }
    }

    private async Task SwitchWorkflowAsync(string workflowId)
    {
        if (!await ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
        {
            OnPropertyChanged(nameof(SelectedWorkflowId));
            return;
        }
        SetSelectedWorkflowId(workflowId);
        CurrentRunId = string.Empty;
        CurrentRunStatus = string.Empty;
        _workflowEventPollingCts?.Cancel();
        await LoadWorkflowAsync(workflowId).ConfigureAwait(true);
    }

    private void SetSelectedWorkflowId(string workflowId)
    {
        if (SetProperty(ref _selectedWorkflowId, string.IsNullOrWhiteSpace(workflowId) ? DefaultWorkflowId : workflowId, nameof(SelectedWorkflowId)))
        {
            var summary = WorkflowSummaries.FirstOrDefault(item => item.WorkflowId == _selectedWorkflowId);
            CurrentWorkflowName = summary?.Name ?? _selectedWorkflowId;
        }
    }

    private async Task LoadWorkflowAsync(string? workflowId = null)
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

        SetWorkflowLoadState(WorkflowLoadState.Loading);
        StatusText = _displayNames.Text("ui.common.loading");
        try
        {
            var graph = await _backend.LoadWorkflowGraphAsync(string.IsNullOrWhiteSpace(workflowId) ? SelectedWorkflowId : workflowId).ConfigureAwait(true);
            CurrentWorkflowName = graph.Name;
            ApplyGraph(graph);
            CaptureSnapshot();
            SetWorkflowLoadState(WorkflowLoadState.Loaded);
            StatusText = _displayNames.Text("ui.common.open");
        }
        catch
        {
            SetWorkflowLoadState(WorkflowLoadState.LoadFailed);
            StatusText = _displayNames.Text("ui.workspace.load_failed");
            OnPropertyChanged(nameof(HasStartNodes));
            OnPropertyChanged(nameof(HasNodes));
        }
    }

    /// <summary>
    /// 导入：重新从项目加载默认画布图到当前画布（一项目一画布，无工作流切换）。
    /// </summary>
    private async Task LoadWorkflowWithUnsavedCheckAsync()
    {
        if (!await ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
        {
            return;
        }

        // 始终加载默认工作流图到当前画布，不切换「多工作流」
        SetSelectedWorkflowId(DefaultWorkflowId);
        await LoadWorkflowAsync(DefaultWorkflowId).ConfigureAwait(true);
        ScheduleCanvasHintAfterImport();
    }

    private void ScheduleCanvasHintAfterImport()
    {
        StatusText = _displayNames.Text("ui.workspace.import_to_canvas");
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
            RememberWorkflowRevision(await _backend.SaveWorkflowGraphAsync(graph).ConfigureAwait(true));
            CaptureSnapshot();
            await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
            StatusText = _displayNames.Text("ui.common.save");
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
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
            StatusText = UserFacingError.Format(ex, _displayNames);
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
            StatusText = UserFacingError.Format(ex, _displayNames);
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
            RememberWorkflowRevision(await _backend.SaveWorkflowGraphAsync(graph).ConfigureAwait(true));
            CaptureSnapshot();
            await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
            await _backend.ExportWorkflowSelectionAsync(CurrentWorkflowId, selected).ConfigureAwait(true);
            var exported = _displayNames.Format("ui.workspace.exported_selection", new Dictionary<string, string>
            {
                ["count"] = selected.Length.ToString(),
            });
            // 导出前静默落盘易让作者以为「没保存」；明确提示
            StatusText = wasDirty
                ? exported + " " + _displayNames.Text("ui.workspace.export_autosaved")
                : exported;
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
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
        try
        {
            var startNodeId = node.NodeType == "start" ? node.Id : null;
            var graph = BuildGraph();
            await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
            RememberWorkflowRevision(await _backend.SaveWorkflowGraphAsync(graph).ConfigureAwait(true));
            CaptureSnapshot();
            await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
            var run = await _backend.RunWorkflowAsync(CurrentWorkflowId, startNodeId).ConfigureAwait(true);
            CurrentRunId = run.RunId;
            CurrentRunStatus = run.Status ?? "running";
            _workflowEventCursor = 0;
            node.StatusText = UserFacingError.RuntimeStatus(run.Status, _displayNames);
            StatusText = UserFacingError.RuntimeStatus(run.Status, _displayNames);
            StartWorkflowEventPolling(run.RunId);
        }
        catch (Exception ex)
        {
            node.StatusText = UserFacingError.Format(ex, _displayNames);
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private async Task PauseWorkflowAsync()
    {
        await RunControlAsync((workflowId, runId) => _backend.PauseWorkflowAsync(workflowId, runId, StatusText));
    }

    private async Task StopWorkflowAsync()
    {
        if (string.IsNullOrWhiteSpace(CurrentRunId))
        {
            StatusText = _displayNames.Text("ui.common.none");
            return;
        }
        if (!await ConfirmDangerAsync(
                "ui.dialog.workspace.stop_run.title",
                "ui.dialog.workspace.stop_run.message",
                "ui.dialog.workspace.stop_run.confirm").ConfigureAwait(true))
        {
            return;
        }
        await RunControlAsync((workflowId, runId) => _backend.StopWorkflowAsync(workflowId, runId, StatusText));
    }

    private async Task ResumeWorkflowAsync()
    {
        await RunControlAsync((workflowId, runId) => _backend.ResumeWorkflowAsync(workflowId, runId));
    }

    private async Task RunControlAsync(Func<string, string, Task<WorkflowActionResult>> action)
    {
        if (string.IsNullOrWhiteSpace(CurrentRunId))
        {
            StatusText = _displayNames.Text("ui.common.none");
            return;
        }
        try
        {
            var result = await action(CurrentWorkflowId, CurrentRunId).ConfigureAwait(true);
            CurrentRunId = result.RunId;
            CurrentRunStatus = result.Status ?? CurrentRunStatus;
            StatusText = UserFacingError.RuntimeStatus(result.Status, _displayNames);
            StartWorkflowEventPolling(result.RunId);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private void StartWorkflowEventPolling(string runId)
    {
        _workflowEventPollingCts?.Cancel();
        _workflowEventPollingCts?.Dispose();
        _workflowEventPollingCts = new CancellationTokenSource();
        var token = _workflowEventPollingCts.Token;
        _ = PollWorkflowEventsAsync(runId, token);
    }

    private async Task PollWorkflowEventsAsync(string runId, CancellationToken cancellationToken)
    {
        while (!cancellationToken.IsCancellationRequested)
        {
            try
            {
                var result = await _backend
                    .GetWorkflowEventsAsync(CurrentWorkflowId, runId, _workflowEventCursor, 100, cancellationToken)
                    .ConfigureAwait(true);
                _workflowEventCursor = result.NextSequence;
                ApplyWorkflowEvents(result);
                if (WorkflowRunIsTerminal(result.Status))
                {
                    CurrentRunStatus = result.Status ?? CurrentRunStatus;
                    return;
                }
            }
            catch (OperationCanceledException)
            {
                return;
            }
            catch (Exception ex)
            {
                StatusText = UserFacingError.Format(ex, _displayNames);
                return;
            }

            try
            {
                await Task.Delay(750, cancellationToken).ConfigureAwait(true);
            }
            catch (OperationCanceledException)
            {
                return;
            }
        }
    }

    private void ApplyWorkflowEvents(WorkflowEventsResult result)
    {
        CurrentRunStatus = result.Status ?? CurrentRunStatus;
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

    private static bool WorkflowRunIsTerminal(string status)
    {
        return status is "stopped" or "succeeded" or "failed";
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
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private async Task ResolveSelectedInDoubtOperationAsync(string decision)
    {
        var operation = SelectedInDoubtOperation;
        if (operation is null)
        {
            return;
        }
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
            var result = await _backend.ResolveInDoubtOperationAsync(
                operation.OperationId,
                decision,
                response,
                string.IsNullOrWhiteSpace(InDoubtStopReason) ? null : InDoubtStopReason).ConfigureAwait(true);
            CurrentRunId = result.Workflow.RunId;
            StatusText = _displayNames.Text("ui.workspace.in_doubt.resolved");
            InDoubtResponseJson = string.Empty;
            InDoubtStopReason = string.Empty;
            await LoadInDoubtOperationsAsync().ConfigureAwait(true);
            if (!WorkflowRunIsTerminal(result.Workflow.Status))
            {
                StartWorkflowEventPolling(result.Workflow.RunId);
            }
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
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
            if (HasUnsavedChanges)
            {
                var graph = BuildGraph();
                await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
                RememberWorkflowRevision(await _backend.SaveWorkflowGraphAsync(graph).ConfigureAwait(true));
                CaptureSnapshot();
            }
            // 起点由后端 list_start_nodes 工具 + AI 自行抉择；前端不提供优先起点。
            var result = await _backend.ProjectAiChatAsync(
                ProjectAiMessage,
                _projectAiHistory,
                workflowIdToRun: null).ConfigureAwait(true);
            ProjectAiAnswer = result.Answer;
            _projectAiHistory.Clear();
            ProjectAiBubbles.Clear();
            foreach (var historyMessage in result.ChatHistory)
            {
                _projectAiHistory.Add(historyMessage);
                ProjectAiBubbles.Add(new ChatBubbleViewModel(historyMessage.Role, historyMessage.Content));
            }
            if (ProjectAiBubbles.Count == 0 && !string.IsNullOrWhiteSpace(result.Answer))
            {
                ProjectAiBubbles.Add(new ChatBubbleViewModel("assistant", result.Answer));
            }
            OnPropertyChanged(nameof(HasProjectAiBubbles));
            ProjectAiMessage = string.Empty;
            StatusText = result.WorkflowRun is not null
                ? UserFacingError.RuntimeStatus(result.WorkflowRun.Status, _displayNames)
                : _displayNames.Text("ui.common.configured");
            if (result.WorkflowRun is not null)
            {
                CurrentRunId = result.WorkflowRun.RunId;
                _workflowEventCursor = 0;
                StartWorkflowEventPolling(result.WorkflowRun.RunId);
            }
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
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
            RememberWorkflowRevision(await _backend.SaveWorkflowGraphAsync(graph).ConfigureAwait(true));

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
            await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
            StatusText = _displayNames.Text("ui.common.save");
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
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
            StatusText = UserFacingError.Format(ex, _displayNames);
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
            StatusText = UserFacingError.Format(ex, _displayNames);
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
        try
        {
            var title = _displayNames.Format("ui.workspace.subworkflow_title", new Dictionary<string, string>
            {
                ["count"] = selected.Length.ToString(),
            });
            var report = await _backend.PackWorkflowSelectionAsync(CurrentWorkflowId, selected, null, title).ConfigureAwait(true);
            ApplyGraph(report.Workflow);
            CaptureSnapshot();
            await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
            StatusText = _displayNames.Format("ui.workspace.packed_selection", new Dictionary<string, string>
            {
                ["count"] = selected.Length.ToString(),
            });
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private async Task LoadConfirmationsAsync()
    {
        if (!_backend.HasProjectRoot)
        {
            Confirmations.Clear();
            SelectedConfirmation = null;
            OnPropertyChanged(nameof(HasPendingConfirmations));
            OnPropertyChanged(nameof(ConfirmationCountText));
            OnPropertyChanged(nameof(ConfirmationsBannerText));
            OnPropertyChanged(nameof(ShowConfirmationFullPanel));
            OnPropertyChanged(nameof(ShowConfirmationBanner));
            OnPropertyChanged(nameof(EmptyStartTitle));
            OnPropertyChanged(nameof(EmptyStartHint));
            NotifyConfirmationCommandStates();
            return;
        }

        try
        {
            var entries = await _backend.ListConfirmationsAsync().ConfigureAwait(true);
            Confirmations.Clear();
            SelectedConfirmation = null;
            // 后端已只返回 pending；前端再保险过滤，避免历史态挡住画布。
            foreach (var entry in entries.Where(IsPendingConfirmation))
            {
                Confirmations.Add(new ConfirmationItemViewModel(entry, SelectConfirmation));
            }
            if (Confirmations.Count > 0 && SelectedConfirmation is null)
            {
                SelectConfirmation(Confirmations[0]);
            }
            OnPropertyChanged(nameof(HasPendingConfirmations));
            OnPropertyChanged(nameof(ConfirmationCountText));
            OnPropertyChanged(nameof(ConfirmationsBannerText));
            OnPropertyChanged(nameof(ShowConfirmationFullPanel));
            OnPropertyChanged(nameof(ShowConfirmationBanner));
            if (Confirmations.Count > 0)
            {
                IsConfirmationPanelExpanded = true;
            }
            if (Confirmations.Count == 0)
            {
                StatusText = ConfirmationsEmptyText;
            }
            NotifyConfirmationCommandStates();
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
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
        // 选中待审项时同步会话 run，便于暂停/恢复与事件轮询。
        if (!string.IsNullOrWhiteSpace(item.RunId))
        {
            CurrentRunId = item.RunId;
        }
        if (!string.IsNullOrWhiteSpace(item.WorkflowId))
        {
            SelectedWorkflowId = item.WorkflowId;
        }
        NotifyConfirmationCommandStates();
    }

    private async Task ResolveSelectedConfirmationAsync(string decision)
    {
        if (SelectedConfirmation is null)
        {
            StatusText = ConfirmationsEmptyText;
            return;
        }
        var workflowId = !string.IsNullOrWhiteSpace(SelectedConfirmation.WorkflowId)
            ? SelectedConfirmation.WorkflowId
            : CurrentWorkflowId;
        var runId = !string.IsNullOrWhiteSpace(SelectedConfirmation.RunId)
            ? SelectedConfirmation.RunId
            : CurrentRunId;
        if (string.IsNullOrWhiteSpace(runId) || string.IsNullOrWhiteSpace(workflowId))
        {
            StatusText = _displayNames.Text("ui.workspace.confirmation.missing_run");
            return;
        }
        if (string.Equals(decision, "reject", StringComparison.Ordinal)
            && !await ConfirmDangerAsync(
                    "ui.dialog.workspace.reject_confirmation.title",
                    "ui.dialog.workspace.reject_confirmation.message",
                    "ui.dialog.workspace.reject_confirmation.confirm").ConfigureAwait(true))
        {
            return;
        }
        try
        {
            var result = await _backend.ResolveConfirmationAsync(
                workflowId,
                runId,
                SelectedConfirmation.ConfirmationId,
                decision,
                string.IsNullOrWhiteSpace(ConfirmationReason) ? null : ConfirmationReason).ConfigureAwait(true);
            CurrentRunId = result.Workflow.RunId;
            StatusText = UserFacingError.RuntimeStatus(result.Workflow.Status, _displayNames);
            StartWorkflowEventPolling(result.Workflow.RunId);
            await LoadConfirmationsAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
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

    private bool _leavePrepared;
    private WorkflowGraphData? _preparedLeaveGraph;

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
            _leavePrepared = true;
            return true;
        }
        catch
        {
            _leavePrepared = false;
            _preparedLeaveGraph = null;
            return false;
        }
    }

    public async Task<bool> CommitPreparedUnsavedChangesAsync()
    {
        if (!_leavePrepared)
        {
            return false;
        }

        if (!HasUnsavedChanges || _preparedLeaveGraph is null)
        {
            _leavePrepared = false;
            return true;
        }

        try
        {
            RememberWorkflowRevision(await _backend.SaveWorkflowGraphAsync(_preparedLeaveGraph).ConfigureAwait(true));
            CaptureSnapshot();
            await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
            _leavePrepared = false;
            _preparedLeaveGraph = null;
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

    public async Task ReloadProjectDataAsync()
    {
        CurrentRunId = string.Empty;
        CurrentRunStatus = string.Empty;
        _workflowEventPollingCts?.Cancel();
        await InitializeWorkflowAsync().ConfigureAwait(true);
        await LoadConfirmationsAsync().ConfigureAwait(true);
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
    }

    private WorkflowGraphData BuildGraph()
    {
        return new WorkflowGraphData(
            CurrentWorkflowId,
            CurrentWorkflowName,
            Nodes.Select(node => new CanvasNode(
                node.Id,
                node.NodeType,
                // 作者改的是 Name；Label 是构造时类型默认文案，不能原样写回
                string.IsNullOrWhiteSpace(node.Name) ? node.Label : node.Name,
                node.ToData(),
                new CanvasPosition(node.X, node.Y))).ToArray(),
            Edges.Select(edge => edge.ToCanvasEdge()).ToArray(),
            new Dictionary<string, object?>(),
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
            _suppressSnapshotChecks = false;
        }
    }

    private void CaptureSnapshot()
    {
        _savedSnapshot = CurrentSnapshot();
        _undoSnapshots.Clear();
        _redoSnapshots.Clear();
        NotifyHistoryCommands();
        HasUnsavedChanges = false;
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
                HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot;
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

    private string CurrentWorkflowId => string.IsNullOrWhiteSpace(SelectedWorkflowId) ? DefaultWorkflowId : SelectedWorkflowId;

    private string NodeLabel(string nodeType)
    {
        return nodeType switch
        {
            "start" => _displayNames.Text("ui.workspace.start_node.title"),
            "llm" => _displayNames.Text("ui.node.llm"),
            "document_read" or "document" => _displayNames.Text("ui.node.document"),
            "search" => _displayNames.Text("ui.node.search"),
            "condition" => _displayNames.Text("ui.node.condition"),
            "loop" => _displayNames.Text("ui.node.loop"),
            "approval" => _displayNames.Text("ui.node.approval"),
            "export" => _displayNames.Text("ui.node.export"),
            "outliner" => _displayNames.Text("agent.outliner"),
            "designer" => _displayNames.Text("agent.designer"),
            "planner" => _displayNames.Text("agent.planner"),
            "detail" => _displayNames.Text("agent.detail"),
            "writer" => _displayNames.Text("agent.writer"),
            "critic" => _displayNames.Text("agent.critic"),
            "prudent" => _displayNames.Text("agent.prudent"),
            "polisher" => _displayNames.Text("agent.polisher"),
            "summarizer" => _displayNames.Text("agent.summarizer"),
            _ => nodeType,
        };
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



    private void SelectEdge(WorkflowEdgeViewModel edge)
    {
        foreach (var item in Edges)
        {
            item.IsSelected = item == edge;
        }
        SelectedEdge = edge;
        IsProjectAiTab = false;

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
            StatusText = UserFacingError.Format(ex, _displayNames);
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
        RefreshPortConnectionStates();
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

public sealed class NodeLibraryItemViewModel
{
    public NodeLibraryItemViewModel(string nodeType, string title, Action add)
    {
        NodeType = nodeType;
        Title = title;
        AddCommand = new RelayCommand(add);
    }

    public string NodeType { get; }
    public string Title { get; }
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
        set
        {
            if (SetProperty(ref _isConnected, value))
            {
                OnPropertyChanged(nameof(FillBrush));
            }
        }
    }

    public IBrush FillBrush => IsConnected
        ? new SolidColorBrush(Color.Parse("#2E726B"))
        : new SolidColorBrush(Colors.Transparent);
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
    /// <summary>节点外框宽（单卡片，引脚在内侧）。与 WorkspacePageView 节点模板一致。</summary>
    public const double NodeWidth = 200;
    /// <summary>框选命中用节点高度（通信行 + 标题 + 内容栏近似）。</summary>
    public const double NodeBodyHeight = 96;
    /// <summary>内侧引脚中心到左右边的内缩（padding 6 + 半宽 7 ≈ 13）。</summary>
    public const double PinInsetX = 13;
    /// <summary>通信口中心 Y：顶行 10px 内、半出卡片上沿。</summary>
    public const double CommPortY = 7;
    /// <summary>卡片上沿（通信行高度）。</summary>
    public const double CardTopOffset = 10;
    /// <summary>标题栏高度（含 padding 6,7 + 内容 ~16）。</summary>
    public const double TitleBarHeight = 34;
    /// <summary>内容栏上 padding。</summary>
    public const double ContentBarPaddingY = 8;
    /// <summary>数据引脚外框边长。</summary>
    public const double DataPinBox = 14;
    /// <summary>多数据入垂直间距 = pin 高 14 + StackPanel Spacing 8。</summary>
    public const double DataPortGap = 8;
    public const double DataPortSpacing = DataPinBox + DataPortGap; // 22
    /// <summary>执行口中心 Y（标题行垂直中线）。</summary>
    public const double ExecPortY = CardTopOffset + 7 + 8; // pad-top + half pin
    /// <summary>
    /// 首个数据口中心 Y：卡片顶 + 标题栏 + 内容 pad-top + 半 pin。
    /// 布局：内容栏 VerticalAlignment=Top，ItemsControl Spacing=DataPortGap。
    /// </summary>
    public const double DataPortY = CardTopOffset + TitleBarHeight + ContentBarPaddingY + (DataPinBox / 2.0);
    public const double HitRadius = 16;
    /// <summary>小地图相对逻辑画布的缩放（与 MiniMapX/Y 一致）。</summary>
    public const double MiniMapScale = 0.1;
    public const double MiniMapContentWidth = 140;
    public const double MiniMapContentHeight = 84;

    public static string HandleName(NodePortKind kind, NodePortDirection direction) => kind switch
    {
        NodePortKind.Control => direction == NodePortDirection.In ? "exec_in" : "exec_out",
        NodePortKind.Communication => "communication",
        _ => direction == NodePortDirection.In ? "input" : "output",
    };

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
        NodePortKind.Data when direction == NodePortDirection.In => (PinInsetX, DataPortY),
        _ => (NodeWidth - PinInsetX, DataPortY),
    };

    /// <summary>按 handle 名解析中心（支持 data-in-N 多入）。</summary>
    public static (double X, double Y) LocalCenterForHandle(string? handle)
    {
        if (!TryResolveKind(handle, out var kind, out var direction))
        {
            return LocalCenter(NodePortKind.Data, NodePortDirection.Out);
        }

        if (kind == NodePortKind.Data && direction == NodePortDirection.In)
        {
            var index = ParseDataInIndex(handle);
            return (PinInsetX, DataPortY + (index * DataPortSpacing));
        }

        if (kind == NodePortKind.Data && direction == NodePortDirection.Out)
        {
            return (NodeWidth - PinInsetX, DataPortY);
        }

        return LocalCenter(kind, direction);
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

    /// <summary>小地图坐标 → 逻辑画布坐标。</summary>
    public static (double X, double Y) MiniMapToLogical(double miniX, double miniY) =>
        (miniX / MiniMapScale, miniY / MiniMapScale);

    /// <summary>逻辑画布视口 → 小地图视口框（内容区内）。</summary>
    public static (double X, double Y, double Width, double Height) LogicalViewportToMiniMap(
        double logicalLeft, double logicalTop, double logicalWidth, double logicalHeight)
    {
        var x = Math.Clamp(logicalLeft * MiniMapScale, 0, MiniMapContentWidth);
        var y = Math.Clamp(logicalTop * MiniMapScale, 0, MiniMapContentHeight);
        var rawW = Math.Max(0.0, logicalWidth * MiniMapScale);
        var rawH = Math.Max(0.0, logicalHeight * MiniMapScale);
        // maxW/maxH 可能为 0（视口贴右下角）；min 不得超过 max，否则 Math.Clamp 抛 ArgumentException。
        var maxW = Math.Max(0.0, MiniMapContentWidth - x);
        var maxH = Math.Max(0.0, MiniMapContentHeight - y);
        var minW = Math.Min(8.0, maxW);
        var minH = Math.Min(6.0, maxH);
        var w = Math.Clamp(rawW, minW, maxW);
        var h = Math.Clamp(rawH, minH, maxH);
        return (x, y, w, h);
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
}

public sealed class WorkflowNodeViewModel : ViewModelBase
{
    private static readonly IBrush IdleBrush = new SolidColorBrush(Color.Parse("#8B939D"));
    private static readonly IBrush RunningBrush = new SolidColorBrush(Color.Parse("#2563EB"));
    private static readonly IBrush PendingBrush = new SolidColorBrush(Color.Parse("#6B7280"));
    private static readonly IBrush PausedBrush = new SolidColorBrush(Color.Parse("#D97706"));
    private static readonly IBrush SucceededBrush = new SolidColorBrush(Color.Parse("#0F9D63"));
    private static readonly IBrush FailedBrush = new SolidColorBrush(Color.Parse("#DC2626"));
    private static readonly IBrush SelectedBrush = new SolidColorBrush(Color.Parse("#2E726B"));
    private static readonly IBrush TypeStartBrush = new SolidColorBrush(Color.Parse("#0F9D63"));
    private static readonly IBrush TypeLlmBrush = new SolidColorBrush(Color.Parse("#2563EB"));
    private static readonly IBrush TypeAgentBrush = new SolidColorBrush(Color.Parse("#2E726B"));
    private static readonly IBrush TypeUtilityBrush = new SolidColorBrush(Color.Parse("#7C3AED"));
    private static readonly IBrush TypeControlBrush = new SolidColorBrush(Color.Parse("#D97706"));
    private static readonly IBrush TypeDefaultBrush = new SolidColorBrush(Color.Parse("#8B939D"));

    private readonly IAriadneBackendClient _backend;
    private readonly Func<string> _currentWorkflowId;
    private readonly Action _markDirty;
    /// <summary>加载时保留的非 UI 配置键，保存时经 <see cref="NodeConfigData.MergeUiFields"/> 合并回去。</summary>
    private Dictionary<string, object?> _extraData = new(StringComparer.Ordinal);
    private string _name;
    private string _workDir;
    private string _userNote = string.Empty;
    private bool _exposedAsTool;
    private bool _portControlInConnected;
    private bool _portControlOutConnected;
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
    private string _summarizerProviderId = string.Empty;
    private string _summarizerChapterId = string.Empty;
    private string _summarizerChapterDocumentId = string.Empty;
    private string _summarizerChapterTextAlias = "chapter_text";
    private bool _summarizerAutoMode;
    private int _nextDataInIndex = 1;

    public WorkflowNodeViewModel(
        string id,
        string nodeType,
        string label,
        string defaultWorkDir,
        double x,
        double y,
        IAriadneBackendClient backend,
        Func<string> currentWorkflowId,
        Action clearSelection,
        Action markDirty)
    {
        Id = id;
        NodeType = nodeType;
        Label = label;
        _name = label;
        _workDir = defaultWorkDir;
        _exposedAsTool = nodeType == "start";
        _x = x;
        _y = y;
        _backend = backend;
        _currentWorkflowId = currentWorkflowId;
        _markDirty = markDirty;
        SelectCommand = new RelayCommand(() => clearSelection());
        RunCommand = new RelayCommand(() => _ = RunAsync());
        DataInPins = new ObservableCollection<NodeDataInPinViewModel>();
        AddDataInPinCommand = new RelayCommand(AddDataInPin);
        RemoveDataInPinCommand = new RelayCommand(RemoveLastDataInPin, () => DataInPins.Count > 1);
        RestoreDataInPins(new[] { NodePortSpec.DataInHandleName(0) });
    }

    public string Id { get; }
    public string NodeType { get; }
    public string Label { get; }
    public RelayCommand SelectCommand { get; set; }
    public RelayCommand RunCommand { get; set; }
    public RelayCommand AddDataInPinCommand { get; }
    public RelayCommand RemoveDataInPinCommand { get; }
    public bool IsStartNode => NodeType == "start";
    /// <summary>文档读/导入：选路径 → 输出 document/content（后端 path 字段）。</summary>
    public bool IsDocumentNode => NodeType is "document_read" or "document";
    /// <summary>兼容旧绑定名；仅文档读，不含 export。</summary>
    public bool IsImportNode => IsDocumentNode;
    public bool IsSearchNode => NodeType is "search";
    public bool IsConditionNode => NodeType is "condition" or "eval";
    public bool IsLoopNode => NodeType is "loop";
    public bool IsApprovalNode => NodeType is "approval";
    public bool IsExportNode => NodeType is "export";
    public bool IsSummarizerNode => NodeType is "summarizer";
    public bool IsUtilityNode => IsDocumentNode || IsSearchNode || IsConditionNode
        || IsLoopNode || IsApprovalNode || IsExportNode;
    public bool IsAgentNode => NodeType is "outliner" or "designer" or "planner" or "detail"
        or "writer" or "critic" or "prudent" or "polisher" or "summarizer" or "llm";
    public bool ShowPromptEditor => IsAgentNode;
    public bool ShowDataInPinEditor => !IsStartNode;
    public ObservableCollection<NodeDataInPinViewModel> DataInPins { get; }

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
    public string QueryAlias { get => _queryAlias; set { if (SetProperty(ref _queryAlias, value ?? "query")) _markDirty(); } }
    public string SearchLimit { get => _searchLimit; set { if (SetProperty(ref _searchLimit, value ?? "10")) _markDirty(); } }
    public string ConditionInputAlias { get => _conditionInputAlias; set { if (SetProperty(ref _conditionInputAlias, value ?? "input")) _markDirty(); } }
    public string ConditionOperator { get => _conditionOperator; set { if (SetProperty(ref _conditionOperator, value ?? "truthy")) _markDirty(); } }
    public string ConditionExpected { get => _conditionExpected; set { if (SetProperty(ref _conditionExpected, value ?? string.Empty)) _markDirty(); } }
    public string MaxIterations { get => _maxIterations; set { if (SetProperty(ref _maxIterations, value ?? "5")) _markDirty(); } }
    public string StopInputAlias { get => _stopInputAlias; set { if (SetProperty(ref _stopInputAlias, value ?? "done")) _markDirty(); } }
    public string StopExpected { get => _stopExpected; set { if (SetProperty(ref _stopExpected, value ?? "true")) _markDirty(); } }
    public string ApprovalId { get => _approvalId; set { if (SetProperty(ref _approvalId, value ?? string.Empty)) _markDirty(); } }
    public bool AutoApprove { get => _autoApprove; set { if (SetProperty(ref _autoApprove, value)) _markDirty(); } }
    public string ExportArtifactId { get => _exportArtifactId; set { if (SetProperty(ref _exportArtifactId, value ?? string.Empty)) _markDirty(); } }
    public string ExportFormat { get => _exportFormat; set { if (SetProperty(ref _exportFormat, value ?? "markdown")) _markDirty(); } }
    public string ExportTitle { get => _exportTitle; set { if (SetProperty(ref _exportTitle, value ?? string.Empty)) _markDirty(); } }
    public string SummarizerProviderId { get => _summarizerProviderId; set { if (SetProperty(ref _summarizerProviderId, value ?? string.Empty)) _markDirty(); } }
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
        _markDirty();
        DataInPinRemoved?.Invoke(handle);
    }

    /// <summary>删除数据入后由宿主拆掉占用该 handle 的边。</summary>
    public Action<string>? DataInPinRemoved { get; set; }

    /// <summary>Summarizer 正文 alias 改动后由宿主同步首个数据入边。</summary>
    public Action<string, string>? SummarizerChapterTextAliasChanged { get; set; }

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
                OnPropertyChanged(nameof(StatusIndicatorBrush));
                OnPropertyChanged(nameof(NodeBorderBrush));
            }
        }
    }
    public double X
    {
        get => _x;
        set
        {
            if (SetProperty(ref _x, value))
            {
                OnPropertyChanged(nameof(MiniMapX));
            }
        }
    }
    public double Y
    {
        get => _y;
        set
        {
            if (SetProperty(ref _y, value))
            {
                OnPropertyChanged(nameof(MiniMapY));
            }
        }
    }
    // 点宽 10 / 高 6，夹在 140×84 内容区内
    public double MiniMapX => Math.Clamp(X * NodePortSpec.MiniMapScale, 0, NodePortSpec.MiniMapContentWidth - 10);
    public double MiniMapY => Math.Clamp(Y * NodePortSpec.MiniMapScale, 0, NodePortSpec.MiniMapContentHeight - 6);
    public bool IsSelected
    {
        get => _isSelected;
        set
        {
            if (SetProperty(ref _isSelected, value))
            {
                OnPropertyChanged(nameof(NodeBorderBrush));
            }
        }
    }
    public IBrush NodeBorderBrush => IsSelected ? SelectedBrush : StatusIndicatorBrush;
    public IBrush StatusIndicatorBrush => BrushForStatus(StatusText);

    /// <summary>节点类型色条：入口/代理/控制/工具分色，便于扫读结构。</summary>
    public IBrush TypeAccentBrush => NodeType.ToLowerInvariant() switch
    {
        "start" => TypeStartBrush,
        "llm" => TypeLlmBrush,
        "condition" or "loop" or "approval" => TypeControlBrush,
        "document_read" or "search" or "export" => TypeUtilityBrush,
        "outliner" or "designer" or "planner" or "detail" or "writer"
            or "critic" or "prudent" or "polisher" or "summarizer" => TypeAgentBrush,
        _ => TypeDefaultBrush,
    };

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

    // 未连接=空心（透明填充），已连接=实心
    public IBrush PortControlInFill => _portControlInConnected
        ? new SolidColorBrush(Color.Parse("#2E726B"))
        : Brushes.Transparent;
    public IBrush PortControlOutFill => _portControlOutConnected
        ? new SolidColorBrush(Color.Parse("#2E726B"))
        : Brushes.Transparent;
    public IBrush PortDataInFill => _portDataInConnected
        ? new SolidColorBrush(Color.Parse("#2E726B"))
        : Brushes.Transparent;
    public IBrush PortDataOutFill => _portDataOutConnected
        ? new SolidColorBrush(Color.Parse("#2E726B"))
        : Brushes.Transparent;
    public IBrush PortCommunicationFill => _portCommunicationConnected
        ? new SolidColorBrush(Color.Parse("#7C3AED"))
        : Brushes.Transparent;

    public void SetPortConnected(
        bool controlIn, bool controlOut, bool dataIn, bool dataOut, bool communication)
    {
        if (_portControlInConnected != controlIn)
        {
            _portControlInConnected = controlIn;
            OnPropertyChanged(nameof(PortControlInFill));
        }
        if (_portControlOutConnected != controlOut)
        {
            _portControlOutConnected = controlOut;
            OnPropertyChanged(nameof(PortControlOutFill));
        }
        if (_portDataInConnected != dataIn)
        {
            _portDataInConnected = dataIn;
            OnPropertyChanged(nameof(PortDataInFill));
        }
        if (_portDataOutConnected != dataOut)
        {
            _portDataOutConnected = dataOut;
            OnPropertyChanged(nameof(PortDataOutFill));
        }
        if (_portCommunicationConnected != communication)
        {
            _portCommunicationConnected = communication;
            OnPropertyChanged(nameof(PortCommunicationFill));
        }
    }

    public void SetPortDragHighlight(
        bool controlIn, bool controlOut, bool dataIn, bool dataOut, bool communication)
    {
        // 可连：满不透明 + 兼容标记；不可连：淡出。
        PortControlInCompatible = controlIn;
        PortControlOutCompatible = controlOut;
        PortDataInCompatible = dataIn;
        PortDataOutCompatible = dataOut;
        PortCommunicationCompatible = communication;
        PortControlInOpacity = controlIn ? 1.0 : 0.22;
        PortControlOutOpacity = controlOut ? 1.0 : 0.22;
        PortDataInOpacity = dataIn ? 1.0 : 0.22;
        PortDataOutOpacity = dataOut ? 1.0 : 0.22;
        PortCommunicationOpacity = communication ? 1.0 : 0.22;
    }

    public void ClearPortDragHighlight()
    {
        PortControlInCompatible = false;
        PortControlOutCompatible = false;
        PortDataInCompatible = false;
        PortDataOutCompatible = false;
        PortCommunicationCompatible = false;
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
            BuildUtilityFields());
    }

    /// <summary>按节点类型写出后端期望的配置键（与 workflow nodes / integration 对齐）。</summary>
    public Dictionary<string, object?> BuildUtilityFields()
    {
        var fields = new Dictionary<string, object?>(StringComparer.Ordinal);
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

            // stop_condition: { input_alias, expected }
            fields["stop_condition"] = new Dictionary<string, object?>
            {
                ["input_alias"] = string.IsNullOrWhiteSpace(StopInputAlias) ? "done" : StopInputAlias.Trim(),
                ["expected"] = ParseLooseJsonOrString(string.IsNullOrWhiteSpace(StopExpected) ? "true" : StopExpected),
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

    private async Task RunAsync()
    {
        try
        {
            var run = await _backend.RunWorkflowAsync(_currentWorkflowId(), IsStartNode ? Id : null).ConfigureAwait(true);
            StatusText = UserFacingError.RuntimeStatus(run.Status, Localization.DisplayNameService.Current);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, Localization.DisplayNameService.Current);
        }
    }

    protected override void OnPropertyChanged(string? propertyName = null)
    {
        base.OnPropertyChanged(propertyName);
        if (propertyName is nameof(Name) or nameof(WorkDir) or nameof(UserNote) or nameof(ExposedAsTool)
            or nameof(PromptTemplate) or nameof(ModelId) or nameof(BudgetUsd) or nameof(TimeoutMs) or nameof(TimeoutSecondsText)
            or nameof(BreakpointEnabled) or nameof(X) or nameof(Y))
        {
            _markDirty();
        }
    }

    private static IBrush BrushForStatus(string status)
    {
        var normalized = status.Trim().ToLowerInvariant();
        if (string.IsNullOrWhiteSpace(normalized))
        {
            return IdleBrush;
        }
        if (normalized.Contains("running") || normalized.Contains("运行"))
        {
            return RunningBrush;
        }
        if (normalized.Contains("queued") || normalized.Contains("pending") || normalized.Contains("排队"))
        {
            return PendingBrush;
        }
        if (normalized.Contains("paused") || normalized.Contains("暂停"))
        {
            return PausedBrush;
        }
        if (normalized.Contains("succeeded") || normalized.Contains("success") || normalized.Contains("成功"))
        {
            return SucceededBrush;
        }
        if (normalized.Contains("failed")
            || normalized.Contains("error")
            || normalized.Contains("exception")
            || normalized.Contains("失败")
            || normalized.Contains("错误"))
        {
            return FailedBrush;
        }
        if (normalized.Contains("stopped") || normalized.Contains("停止"))
        {
            return IdleBrush;
        }
        return IdleBrush;
    }
}

public sealed class ConfirmationItemViewModel : ViewModelBase
{
    private bool _isSelected;

    public ConfirmationItemViewModel(ConfirmationLogEntry entry, Action<ConfirmationItemViewModel> select)
    {
        ConfirmationId = entry.ConfirmationId;
        Summary = entry.Summary;
        State = entry.State;
        Diff = entry.Diff;
        WorkflowId = entry.WorkflowId ?? string.Empty;
        RunId = entry.RunId ?? string.Empty;
        SelectCommand = new RelayCommand(() => select(this));
    }

    public string ConfirmationId { get; }
    public string Summary { get; }
    public string State { get; }
    public string Diff { get; }
    public string WorkflowId { get; }
    public string RunId { get; }
    public RelayCommand SelectCommand { get; }
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }
}

public sealed class WorkflowEdgeViewModel : ViewModelBase
{
    private static readonly IBrush DataBrush = new SolidColorBrush(Color.Parse("#2E726B"));
    private static readonly IBrush ControlBrush = new SolidColorBrush(Color.Parse("#8B939D"));
    private static readonly IBrush CommunicationBrush = new SolidColorBrush(Color.Parse("#7C3AED"));

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
                OnPropertyChanged(nameof(StrokeBrush));
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
    private static readonly IBrush SelectedStrokeBrush = new SolidColorBrush(Color.Parse("#F59E0B"));

    /// <summary>W14：选中边用强调色，未选中按类型色。</summary>
    public IBrush StrokeBrush =>
        IsSelected
            ? SelectedStrokeBrush
            : Kind.ToLowerInvariant() switch
            {
                "control" => ControlBrush,
                "communication" => CommunicationBrush,
                _ => DataBrush,
            };

    /// <summary>W14：选中边加粗；通信边默认略粗。</summary>
    public double StrokeThickness =>
        IsSelected ? 3.4 : (IsCommunication ? 2.2 : 1.6);

    /// <summary>W14：选中边不透明，未选中略淡。</summary>
    public double StrokeOpacity => IsSelected ? 1.0 : 0.88;
    public Geometry EdgePath { get; private set; } = new PathGeometry();
    public double LabelX { get; private set; }
    public double LabelY { get; private set; }
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

    public void UpdateEdgePath(double sourceX, double sourceY, double targetX, double targetY)
    {
        if (!NodePortSpec.TryResolveKind(SourceHandle, out var sourceKind, out _))
        {
            sourceKind = NodePortKind.Data;
        }
        if (!NodePortSpec.TryResolveKind(TargetHandle, out var targetKind, out _))
        {
            targetKind = NodePortKind.Data;
        }
        if (string.Equals(Kind, "communication", StringComparison.OrdinalIgnoreCase))
        {
            sourceKind = NodePortKind.Communication;
            targetKind = NodePortKind.Communication;
        }

        // 按 handle 中心起止（支持多数据入索引）
        var (sx, sy) = NodePortSpec.LocalCenterForHandle(SourceHandle);
        var (tx, ty) = NodePortSpec.LocalCenterForHandle(TargetHandle);

        var startX = sourceX + sx;
        var startY = sourceY + sy;
        var endX = targetX + tx;
        var endY = targetY + ty;
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
        // 名称贴在目标入脚旁，不画在线中
        if (isComm)
        {
            var mid = spec.Midpoint;
            LabelX = mid.X - 28;
            LabelY = mid.Y - 14;
        }
        else
        {
            var (lx, ly) = NodePortSpec.LabelBesideDataIn(endX, endY);
            LabelX = lx;
            LabelY = ly;
        }

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
