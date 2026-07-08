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
    private const double MinRightPanelWidth = 300;
    private const double MaxRightPanelWidth = 560;
    private const double CollapsedRightPanelWidth = 24;
    private static readonly JsonSerializerOptions JsonOptions = new(JsonSerializerDefaults.Web);

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private bool _isRightPanelOpen = true;
    private GridLength _rightPanelColumnWidth = new(360);
    private bool _isLibraryOpen = true;
    private bool _isExecutionPanel;
    private bool _isProjectAiTab = true;
    private double _canvasZoom = 1.0;
    private string _statusText = string.Empty;
    private bool _hasUnsavedChanges;
    private string _savedSnapshot = string.Empty;
    private bool _suppressSnapshotChecks;
    private int _nextNodeNumber = 1;
    private string _projectAiMessage = string.Empty;
    private string _projectAiAnswer;
    private string _currentRunId = string.Empty;
    private string _selectedWorkflowId = DefaultWorkflowId;
    private string _currentWorkflowName = "Default";
    private bool _suppressWorkflowSelectionChange;
    private long _workflowEventCursor;
    private CancellationTokenSource? _workflowEventPollingCts;
    private string _confirmationReason = string.Empty;
    private string _annotationTitle = string.Empty;
    private IReadOnlyList<CanvasEdge> _edges = Array.Empty<CanvasEdge>();
    private readonly List<string> _undoSnapshots = new();
    private readonly List<string> _redoSnapshots = new();
    private readonly List<ProjectAiChatMessage> _projectAiHistory = new();
    private CanvasNode? _clipboardNode;
    private WorkflowNodeViewModel? _selectedNode;
    private ConfirmationItemViewModel? _selectedConfirmation;
    private WorkflowEdgeViewModel? _selectedEdge;

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
        ExportCommand = new RelayCommand(() => _ = ExportWorkflowAsync());
        SaveCommand = new RelayCommand(() => _ = SaveWorkflowAsync());
        UndoCommand = new RelayCommand(UndoCanvasChange, () => _undoSnapshots.Count > 0);
        RedoCommand = new RelayCommand(RedoCanvasChange, () => _redoSnapshots.Count > 0);
        AddContextNodeCommand = new RelayCommand(() => AddNode("llm"));
        AddStartNodeCommand = new RelayCommand(() => AddNode("start"));
        DeleteSelectedNodeCommand = new RelayCommand(() => _ = DeleteSelectedNodeAsync(), () => HasSelectedNode);
        RunSelectedNodeCommand = new RelayCommand(() => _ = RunSelectedNodeAsync(), () => IsSelectedStartNode);
        PauseWorkflowCommand = new RelayCommand(() => _ = PauseWorkflowAsync(), HasCurrentRun);
        StopWorkflowCommand = new RelayCommand(() => _ = StopWorkflowAsync(), HasCurrentRun);
        ResumeWorkflowCommand = new RelayCommand(() => _ = ResumeWorkflowAsync(), HasCurrentRun);
        SendProjectAiCommand = new RelayCommand(() => _ = SendProjectAiAsync(), HasProjectAiMessage);
        ApplyNodeConfigCommand = new RelayCommand(() => _ = ApplyNodeConfigAsync(), () => HasSelectedNode);
        ToggleBreakpointCommand = new RelayCommand(() => _ = ToggleBreakpointAsync(), () => HasSelectedNode);
        AddAnnotationCommand = new RelayCommand(() => _ = AddAnnotationAsync());
        ExportSelectionCommand = new RelayCommand(() => _ = ExportWorkflowAsync());
        PackSelectionCommand = new RelayCommand(() => _ = PackSelectionAsync());
        RefreshConfirmationsCommand = new RelayCommand(() => _ = LoadConfirmationsAsync());
        ApproveConfirmationCommand = new RelayCommand(() => _ = ResolveSelectedConfirmationAsync("approve"), CanResolveConfirmation);
        RejectConfirmationCommand = new RelayCommand(() => _ = ResolveSelectedConfirmationAsync("reject"), CanResolveConfirmation);
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
        Edges = new ObservableCollection<WorkflowEdgeViewModel>();
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

        CaptureSnapshot();
        _ = InitializeWorkflowAsync();
        _ = LoadConfirmationsAsync();
    }

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
    public string NoStartNodesText => _displayNames.Text("ui.workspace.no_start_nodes");
    public string SelectStartNodeText => _displayNames.Text("ui.workspace.select_start_node");
    public string NodeLibraryText => _displayNames.Text("ui.workspace.node_library");
    public string ExecutionText => _displayNames.Text("ui.workspace.execution");
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
    public string ExposeToolLabel => _displayNames.Text("ui.workspace.start_node.expose_tool");
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
    public string ConfirmationDiffText => _displayNames.Text("ui.workspace.confirmation.diff");
    public string ConfirmationReasonText => _displayNames.Text("ui.workspace.confirmation.reason");
    public string ConfirmationReasonPlaceholder => _displayNames.Text("ui.workspace.confirmation.reason.placeholder");
    public string ApproveConfirmationText => _displayNames.Text("ui.workspace.confirmation.approve");
    public string RejectConfirmationText => _displayNames.Text("ui.workspace.confirmation.reject");
    public string PromptTemplateText => _displayNames.Text("ui.workspace.prompt_template");
    public string ModelIdText => _displayNames.Text("ui.workspace.model_id");
    public string NodeBudgetText => _displayNames.Text("ui.workspace.node_budget");
    public string NodeTimeoutText => _displayNames.Text("ui.workspace.node_timeout");
    public string BreakpointText => _displayNames.Text("ui.workspace.breakpoint");
    public string ApplyNodeConfigText => _displayNames.Text("ui.workspace.apply_node_config");
    public string ExportSelectionText => _displayNames.Text("ui.workspace.export_selection");
    public string AddAnnotationText => _displayNames.Text("ui.workspace.add_annotation");
    public string AnnotationTitleText => _displayNames.Text("ui.workspace.annotation_title");
    public string AnnotationTitlePlaceholder => _displayNames.Text("ui.workspace.annotation_title.placeholder");
    public string SubworkflowText => _displayNames.Text("ui.workspace.subworkflow");
    public string EdgeDetailsText => _displayNames.Text("ui.workspace.edge_details");
    public string EdgeCountText => $"{Edges.Count}";
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
    public bool IsLibraryOpen { get => _isLibraryOpen; set => SetProperty(ref _isLibraryOpen, value); }
    public RelayCommand ToggleLibraryCommand { get; }
    public double CanvasZoom
    {
        get => _canvasZoom;
        private set
        {
            var clamped = Math.Clamp(Math.Round(value, 2), 0.4, 1.8);
            if (SetProperty(ref _canvasZoom, clamped))
            {
                OnPropertyChanged(nameof(CanvasZoomText));
            }
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
    public RelayCommand AddAnnotationCommand { get; }
    public RelayCommand ExportSelectionCommand { get; }
    public RelayCommand PackSelectionCommand { get; }
    public RelayCommand RefreshConfirmationsCommand { get; }
    public RelayCommand ApproveConfirmationCommand { get; }
    public RelayCommand RejectConfirmationCommand { get; }
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
    public string CurrentRunId
    {
        get => _currentRunId;
        set
        {
            if (SetProperty(ref _currentRunId, value))
            {
                OnPropertyChanged(nameof(CurrentRunValueText));
                NotifyRunCommandStates();
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
    public string AnnotationTitle { get => _annotationTitle; set => SetProperty(ref _annotationTitle, value); }

    public bool HasUnsavedChanges
    {
        get => _hasUnsavedChanges;
        private set => SetProperty(ref _hasUnsavedChanges, value);
    }

    public ObservableCollection<WorkflowNodeViewModel> Nodes { get; }
    public ObservableCollection<WorkflowNodeViewModel> StartNodes { get; }
    public ObservableCollection<WorkflowSummary> WorkflowSummaries { get; }
    public ObservableCollection<NodeLibraryItemViewModel> EntryNodes { get; }
    public ObservableCollection<NodeLibraryItemViewModel> WritingAgents { get; }
    public ObservableCollection<NodeLibraryItemViewModel> UtilityNodes { get; }
    public ObservableCollection<ConfirmationItemViewModel> Confirmations { get; }
    public ObservableCollection<WorkflowEdgeViewModel> Edges { get; }

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

    public WorkflowEdgeViewModel? SelectedEdge
    {
        get => _selectedEdge;
        private set
        {
            if (SetProperty(ref _selectedEdge, value))
            {
                OnPropertyChanged(nameof(HasSelectedEdge));
                SaveEdgeConfigCommand.NotifyCanExecuteChanged();
                InsertForwardTemplateVariableCommand.NotifyCanExecuteChanged();
                InsertReverseTemplateVariableCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public bool HasSelectedEdge => SelectedEdge is not null;
    private bool HasCurrentRun() => !string.IsNullOrWhiteSpace(CurrentRunId);
    private bool HasProjectAiMessage() => !string.IsNullOrWhiteSpace(ProjectAiMessage);
    private bool CanResolveConfirmation() => SelectedConfirmation is not null && HasCurrentRun();

    public void CreateDataEdge(string sourceNodeId, string targetNodeId)
    {
        if (string.Equals(sourceNodeId, targetNodeId, StringComparison.Ordinal))
        {
            return;
        }
        if (Edges.Any(edge => edge.Source == sourceNodeId
                              && edge.Target == targetNodeId
                              && string.Equals(edge.Kind, "data", StringComparison.OrdinalIgnoreCase)))
        {
            StatusText = EdgeDetailsText;
            return;
        }
        CaptureUndoSnapshot();
        var edge = new CanvasEdge(
            $"edge-{Guid.NewGuid():N}",
            sourceNodeId,
            targetNodeId,
            "output",
            "input",
            "data",
            "input",
            new Dictionary<string, object?>());
        var viewModel = new WorkflowEdgeViewModel(edge, _displayNames, SelectEdge, RefreshDirtyState);
        Edges.Add(viewModel);
        _edges = Edges.Select(item => item.ToCanvasEdge()).ToArray();
        SelectEdge(viewModel);
        RefreshDirtyState();
        OnPropertyChanged(nameof(EdgeCountText));
        StatusText = EdgeDetailsText;
    }

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
        AttachNodeCommands(node);
        Nodes.Add(node);
        RefreshStartNodes();
        SelectNode(node);
        if (capture)
        {
            RefreshDirtyState();
        }
    }

    private WorkflowNodeViewModel CreateNodeFromCanvas(CanvasNode graphNode)
    {
        var node = new WorkflowNodeViewModel(
            graphNode.Id,
            graphNode.Type,
            graphNode.Label ?? NodeLabel(graphNode.Type),
            ReadString(graphNode.Data, "work_dir"),
            graphNode.Position?.X ?? 120 + ((Nodes.Count % 4) * 230),
            graphNode.Position?.Y ?? 80 + ((Nodes.Count / 4) * 170),
            _backend,
            () => CurrentWorkflowId,
            () => SelectNode(node: null),
            RefreshDirtyState)
        {
            Name = ReadString(graphNode.Data, "name", graphNode.Label ?? NodeLabel(graphNode.Type)),
            ExposedAsTool = ReadBool(graphNode.Data, "expose_as_tool", graphNode.Type == "start"),
            PromptTemplate = ReadString(graphNode.Data, "prompt_template"),
            ModelId = ReadString(graphNode.Data, "model_id"),
            BudgetUsd = ReadString(graphNode.Data, "budget_usd"),
            TimeoutMs = ReadString(graphNode.Data, "timeout_ms"),
            BreakpointEnabled = ReadBool(graphNode.Data, "breakpoint", false),
        };
        AttachNodeCommands(node);
        return node;
    }

    private void AttachNodeCommands(WorkflowNodeViewModel node)
    {
        node.SelectCommand = new RelayCommand(() => SelectNode(node));
        node.RunCommand = new RelayCommand(() => _ = RunNodeAsync(node));
    }

    private void SelectNode(WorkflowNodeViewModel? node)
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
    }

    private async Task DeleteSelectedNodeAsync()
    {
        var node = SelectedNode;
        if (node is null)
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
        DeleteNode(node);
        StatusText = _displayNames.Format("ui.workspace.deleted_selection", new Dictionary<string, string>
        {
            ["count"] = "1",
        });
    }

    private void DeleteNode(WorkflowNodeViewModel node)
    {
        CaptureUndoSnapshot();
        Nodes.Remove(node);
        _edges = _edges
            .Where(edge => edge.Source != node.Id && edge.Target != node.Id)
            .ToArray();
        Edges.Clear();
        foreach (var edge in _edges)
        {
            Edges.Add(new WorkflowEdgeViewModel(edge, _displayNames, SelectEdge, RefreshDirtyState));
        }
        SelectedNode = null;
        SelectedEdge = null;
        OnPropertyChanged(nameof(EdgeCountText));
        RefreshStartNodes();
        RefreshDirtyState();
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
        await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
        await LoadWorkflowAsync(SelectedWorkflowId).ConfigureAwait(true);
    }

    private async Task RefreshWorkflowSummariesAsync()
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

    private async Task SwitchWorkflowAsync(string workflowId)
    {
        if (!await ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
        {
            OnPropertyChanged(nameof(SelectedWorkflowId));
            return;
        }
        SetSelectedWorkflowId(workflowId);
        CurrentRunId = string.Empty;
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
        try
        {
            var graph = await _backend.LoadWorkflowGraphAsync(string.IsNullOrWhiteSpace(workflowId) ? SelectedWorkflowId : workflowId).ConfigureAwait(true);
            CurrentWorkflowName = graph.Name;
            ApplyGraph(graph);
            CaptureSnapshot();
            StatusText = _displayNames.Text("ui.common.open");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task LoadWorkflowWithUnsavedCheckAsync()
    {
        if (!await ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
        {
            return;
        }

        await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
        await LoadWorkflowAsync(SelectedWorkflowId).ConfigureAwait(true);
    }

    private async Task SaveWorkflowAsync()
    {
        try
        {
            var graph = BuildGraph();
            await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
            await _backend.SaveWorkflowGraphAsync(graph).ConfigureAwait(true);
            CaptureSnapshot();
            await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
            StatusText = _displayNames.Text("ui.common.save");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task ExportWorkflowAsync()
    {
        try
        {
            var selected = SelectedNode is null ? Nodes.Select(node => node.Id).ToArray() : new[] { SelectedNode.Id };
            var graph = BuildGraph();
            await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
            await _backend.SaveWorkflowGraphAsync(graph).ConfigureAwait(true);
            CaptureSnapshot();
            await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
            await _backend.ExportWorkflowSelectionAsync(CurrentWorkflowId, selected).ConfigureAwait(true);
            StatusText = _displayNames.Format("ui.workspace.exported_selection", new Dictionary<string, string>
            {
                ["count"] = selected.Length.ToString(),
            });
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
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
        try
        {
            var startNodeId = node.NodeType == "start" ? node.Id : null;
            var graph = BuildGraph();
            await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
            await _backend.SaveWorkflowGraphAsync(graph).ConfigureAwait(true);
            CaptureSnapshot();
            await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
            var run = await _backend.RunWorkflowAsync(CurrentWorkflowId, startNodeId).ConfigureAwait(true);
            CurrentRunId = run.RunId;
            _workflowEventCursor = 0;
            node.StatusText = run.Status;
            StatusText = run.Status;
            StartWorkflowEventPolling(run.RunId);
        }
        catch (Exception ex)
        {
            node.StatusText = ex.Message;
            StatusText = ex.Message;
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
            StatusText = result.Status;
            StartWorkflowEventPolling(result.RunId);
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
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
                    return;
                }
            }
            catch (OperationCanceledException)
            {
                return;
            }
            catch (Exception ex)
            {
                StatusText = ex.Message;
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
        StatusText = result.Status;
        foreach (var runtimeEvent in result.Events)
        {
            if (!string.IsNullOrWhiteSpace(runtimeEvent.NodeId))
            {
                var node = Nodes.FirstOrDefault(item => item.Id == runtimeEvent.NodeId);
                if (node is not null)
                {
                    node.StatusText = NodeStatusFromEvent(runtimeEvent.EventType, runtimeEvent.Message);
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

    private async Task SendProjectAiAsync()
    {
        try
        {
            if (string.IsNullOrWhiteSpace(ProjectAiMessage))
            {
                StatusText = ProjectAiPlaceholder;
                return;
            }
            if (HasUnsavedChanges)
            {
                var graph = BuildGraph();
                await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
                await _backend.SaveWorkflowGraphAsync(graph).ConfigureAwait(true);
                CaptureSnapshot();
            }
            var result = await _backend.ProjectAiChatAsync(
                ProjectAiMessage,
                _projectAiHistory,
                ProjectAiMessage.Contains("/run", StringComparison.OrdinalIgnoreCase) ? CurrentWorkflowId : null).ConfigureAwait(true);
            ProjectAiAnswer = result.Answer;
            _projectAiHistory.Clear();
            foreach (var message in result.ChatHistory)
            {
                _projectAiHistory.Add(message);
            }
            ProjectAiMessage = string.Empty;
            StatusText = result.WorkflowRun?.Status ?? _displayNames.Text("ui.common.configured");
            if (result.WorkflowRun is not null)
            {
                CurrentRunId = result.WorkflowRun.RunId;
                _workflowEventCursor = 0;
                StartWorkflowEventPolling(result.WorkflowRun.RunId);
            }
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task ApplyNodeConfigAsync()
    {
        if (SelectedNode is null)
        {
            StatusText = NoNodeSelectedText;
            return;
        }
        try
        {
            await _backend.ApplyNodeDetailPatchAsync(CurrentWorkflowId, new NodeDetailPatch(
                SelectedNode.Id,
                SelectedNode.PromptTemplate,
                new Dictionary<string, string>(),
                new Dictionary<string, bool>(),
                new Dictionary<string, string>(),
                string.IsNullOrWhiteSpace(SelectedNode.ModelId) ? null : SelectedNode.ModelId,
                ParseNullableDouble(SelectedNode.BudgetUsd),
                ParseNullableLong(SelectedNode.TimeoutMs))).ConfigureAwait(true);
            await LoadWorkflowAsync().ConfigureAwait(true);
            StatusText = _displayNames.Text("ui.common.save");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task ToggleBreakpointAsync()
    {
        if (SelectedNode is null)
        {
            StatusText = NoNodeSelectedText;
            return;
        }
        try
        {
            await _backend.SetNodeBreakpointAsync(CurrentWorkflowId, SelectedNode.Id, SelectedNode.BreakpointEnabled).ConfigureAwait(true);
            StatusText = BreakpointText;
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task AddAnnotationAsync()
    {
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
            StatusText = ex.Message;
        }
    }

    private async Task PackSelectionAsync()
    {
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
            var graph = await _backend.PackWorkflowSelectionAsync(CurrentWorkflowId, selected, null, title).ConfigureAwait(true);
            ApplyGraph(graph);
            CaptureSnapshot();
            await RefreshWorkflowSummariesAsync().ConfigureAwait(true);
            StatusText = _displayNames.Format("ui.workspace.packed_selection", new Dictionary<string, string>
            {
                ["count"] = selected.Length.ToString(),
            });
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task LoadConfirmationsAsync()
    {
        try
        {
            var entries = await _backend.ListConfirmationsAsync().ConfigureAwait(true);
            Confirmations.Clear();
            SelectedConfirmation = null;
            foreach (var entry in entries)
            {
                Confirmations.Add(new ConfirmationItemViewModel(entry, SelectConfirmation));
            }
            if (Confirmations.Count > 0 && SelectedConfirmation is null)
            {
                SelectConfirmation(Confirmations[0]);
            }
            OnPropertyChanged(nameof(HasPendingConfirmations));
            OnPropertyChanged(nameof(ConfirmationCountText));
            StatusText = Confirmations.Count == 0 ? ConfirmationsEmptyText : $"{Confirmations.Count}";
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private void SelectConfirmation(ConfirmationItemViewModel item)
    {
        foreach (var confirmation in Confirmations)
        {
            confirmation.IsSelected = confirmation == item;
        }
        SelectedConfirmation = item;
    }

    private async Task ResolveSelectedConfirmationAsync(string decision)
    {
        if (SelectedConfirmation is null)
        {
            StatusText = ConfirmationsEmptyText;
            return;
        }
        if (string.IsNullOrWhiteSpace(CurrentRunId))
        {
            StatusText = _displayNames.Text("ui.common.none");
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
                CurrentWorkflowId,
                CurrentRunId,
                SelectedConfirmation.ConfirmationId,
                decision,
                string.IsNullOrWhiteSpace(ConfirmationReason) ? null : ConfirmationReason).ConfigureAwait(true);
            CurrentRunId = result.Workflow.RunId;
            StatusText = result.Workflow.Status;
            StartWorkflowEventPolling(result.Workflow.RunId);
            await LoadConfirmationsAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
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
            CancelResultIndex = 1,
        };
        return await DialogService.Current.ConfirmAsync(dialog).ConfigureAwait(true) == 0;
    }

    public async Task<bool> ConfirmLeaveIfNeededAsync()
    {
        if (!HasUnsavedChanges)
        {
            return true;
        }

        var choice = await DialogService.Current.ConfirmUnsavedLeaveAsync().ConfigureAwait(true);
        switch (choice)
        {
            case UnsavedLeaveChoice.Save:
                await SaveWorkflowAsync().ConfigureAwait(true);
                return !HasUnsavedChanges;
            case UnsavedLeaveChoice.Discard:
                RestoreSnapshot();
                return true;
            default:
                return false;
        }
    }

    public async Task ReloadProjectDataAsync()
    {
        CurrentRunId = string.Empty;
        _workflowEventPollingCts?.Cancel();
        await InitializeWorkflowAsync().ConfigureAwait(true);
        await LoadConfirmationsAsync().ConfigureAwait(true);
    }

    private WorkflowGraphData BuildGraph()
    {
        return new WorkflowGraphData(
            CurrentWorkflowId,
            CurrentWorkflowName,
            Nodes.Select(node => new CanvasNode(
                node.Id,
                node.NodeType,
                node.Label,
                node.ToData(),
                new CanvasPosition(node.X, node.Y))).ToArray(),
            Edges.Select(edge => edge.ToCanvasEdge()).ToArray(),
            new Dictionary<string, object?>());
    }

    private void ApplyGraph(WorkflowGraphData graph)
    {
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
            RefreshStartNodes();
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

    private static double? ParseNullableDouble(string text)
    {
        return double.TryParse(text, out var value) ? value : null;
    }

    private static long? ParseNullableLong(string text)
    {
        return long.TryParse(text, out var value) ? value : null;
    }

    private void SelectEdge(WorkflowEdgeViewModel edge)
    {
        foreach (var item in Edges)
        {
            item.IsSelected = item == edge;
        }
        SelectedEdge = edge;
        IsProjectAiTab = false;
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
            StatusText = ex.Message;
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

public sealed class WorkflowNodeViewModel : ViewModelBase
{
    private static readonly IBrush IdleBrush = new SolidColorBrush(Color.Parse("#8B939D"));
    private static readonly IBrush RunningBrush = new SolidColorBrush(Color.Parse("#2563EB"));
    private static readonly IBrush PendingBrush = new SolidColorBrush(Color.Parse("#6B7280"));
    private static readonly IBrush PausedBrush = new SolidColorBrush(Color.Parse("#D97706"));
    private static readonly IBrush SucceededBrush = new SolidColorBrush(Color.Parse("#0F9D63"));
    private static readonly IBrush FailedBrush = new SolidColorBrush(Color.Parse("#DC2626"));
    private static readonly IBrush SelectedBrush = new SolidColorBrush(Color.Parse("#2E726B"));

    private readonly IAriadneBackendClient _backend;
    private readonly Func<string> _currentWorkflowId;
    private readonly Action _markDirty;
    private string _name;
    private string _workDir;
    private bool _exposedAsTool;
    private bool _breakpointEnabled;
    private string _promptTemplate = string.Empty;
    private string _modelId = string.Empty;
    private string _budgetUsd = string.Empty;
    private string _timeoutMs = string.Empty;
    private string _statusText = string.Empty;
    private double _x;
    private double _y;
    private bool _isSelected;

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
    }

    public string Id { get; }
    public string NodeType { get; }
    public string Label { get; }
    public RelayCommand SelectCommand { get; set; }
    public RelayCommand RunCommand { get; set; }
    public bool IsStartNode => NodeType == "start";

    public string Name { get => _name; set => SetProperty(ref _name, value); }
    public string WorkDir { get => _workDir; set => SetProperty(ref _workDir, value); }
    public bool ExposedAsTool { get => _exposedAsTool; set => SetProperty(ref _exposedAsTool, value); }
    public bool BreakpointEnabled { get => _breakpointEnabled; set => SetProperty(ref _breakpointEnabled, value); }
    public string PromptTemplate { get => _promptTemplate; set => SetProperty(ref _promptTemplate, value); }
    public string ModelId { get => _modelId; set => SetProperty(ref _modelId, value); }
    public string BudgetUsd { get => _budgetUsd; set => SetProperty(ref _budgetUsd, value); }
    public string TimeoutMs { get => _timeoutMs; set => SetProperty(ref _timeoutMs, value); }
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
    public double MiniMapX => Math.Clamp(X * 0.1, 2, 142);
    public double MiniMapY => Math.Clamp(Y * 0.1, 2, 86);
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

    public Dictionary<string, object?> ToData()
    {
        var data = new Dictionary<string, object?>
        {
            ["name"] = Name,
        };
        if (!string.IsNullOrWhiteSpace(WorkDir))
        {
            data["work_dir"] = WorkDir;
        }
        if (IsStartNode)
        {
            data["expose_as_tool"] = ExposedAsTool;
        }
        if (!string.IsNullOrWhiteSpace(PromptTemplate))
        {
            data["prompt_template"] = PromptTemplate;
        }
        if (!string.IsNullOrWhiteSpace(ModelId))
        {
            data["model_id"] = ModelId;
        }
        if (!string.IsNullOrWhiteSpace(BudgetUsd))
        {
            data["budget_usd"] = BudgetUsd;
        }
        if (!string.IsNullOrWhiteSpace(TimeoutMs))
        {
            data["timeout_ms"] = TimeoutMs;
        }
        if (BreakpointEnabled)
        {
            data["breakpoint"] = true;
        }
        return data;
    }

    public CanvasNode ToCanvasNode()
    {
        return new CanvasNode(
            Id,
            NodeType,
            Label,
            ToData(),
            new CanvasPosition(X, Y));
    }

    private async Task RunAsync()
    {
        try
        {
            var run = await _backend.RunWorkflowAsync(_currentWorkflowId(), IsStartNode ? Id : null).ConfigureAwait(true);
            StatusText = run.Status;
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    protected override void OnPropertyChanged(string? propertyName = null)
    {
        base.OnPropertyChanged(propertyName);
        if (propertyName is nameof(Name) or nameof(WorkDir) or nameof(ExposedAsTool)
            or nameof(PromptTemplate) or nameof(ModelId) or nameof(BudgetUsd) or nameof(TimeoutMs)
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
        SelectCommand = new RelayCommand(() => select(this));
    }

    public string ConfirmationId { get; }
    public string Summary { get; }
    public string State { get; }
    public string Diff { get; }
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
        SelectCommand = new RelayCommand(() => select(this));
    }

    public string Id { get; }
    public string Source { get; }
    public string Target { get; }
    public string Kind { get; }
    public string Title => $"{Source} -> {Target}";
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
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }
    public bool IsCommunication => string.Equals(Kind, "communication", StringComparison.OrdinalIgnoreCase);
    public IBrush StrokeBrush => Kind.ToLowerInvariant() switch
    {
        "control" => ControlBrush,
        "communication" => CommunicationBrush,
        _ => DataBrush,
    };
    public Geometry EdgePath { get; private set; } = new PathGeometry();

    public void UpdateEdgePath(double sourceX, double sourceY, double targetX, double targetY)
    {
        const double nodeWidth = 202.0;
        const double portOffsetY = 38.0;
        var startX = sourceX + nodeWidth;
        var startY = sourceY + portOffsetY;
        var endX = targetX;
        var endY = targetY + portOffsetY;
        var controlOffset = Math.Max(44.0, Math.Abs(endX - startX) * 0.5);
        var geometry = new PathGeometry();
        var figure = new PathFigure { StartPoint = new Avalonia.Point(startX, startY) };
        figure.Segments ??= new PathSegments();
        figure.Segments.Add(new BezierSegment
        {
            Point1 = new Avalonia.Point(startX + controlOffset, startY),
            Point2 = new Avalonia.Point(endX - controlOffset, endY),
            Point3 = new Avalonia.Point(endX, endY),
        });
        geometry.Figures ??= new PathFigures();
        geometry.Figures.Add(figure);
        EdgePath = geometry;
        OnPropertyChanged(nameof(EdgePath));
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
