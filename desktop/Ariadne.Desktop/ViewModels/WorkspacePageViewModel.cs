using System.Collections.ObjectModel;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class WorkspacePageViewModel : ViewModelBase, IUnsavedChangesGuard
{
    private const string DefaultWorkflowId = "default";

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private bool _isRightPanelOpen = true;
    private bool _isLibraryOpen = true;
    private bool _isProjectAiTab = true;
    private string _statusText = string.Empty;
    private bool _hasUnsavedChanges;
    private string _savedSnapshot = string.Empty;
    private bool _suppressSnapshotChecks;
    private int _nextNodeNumber = 1;
    private string _projectAiMessage = string.Empty;
    private string _projectAiAnswer;
    private string _currentRunId = string.Empty;
    private string _confirmationReason = string.Empty;
    private string _annotationTitle = string.Empty;
    private WorkflowNodeViewModel? _selectedNode;
    private ConfirmationItemViewModel? _selectedConfirmation;

    public WorkspacePageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        ToggleRightPanelCommand = new RelayCommand(() => IsRightPanelOpen = !IsRightPanelOpen);
        ToggleLibraryCommand = new RelayCommand(() => IsLibraryOpen = !IsLibraryOpen);
        ShowProjectAiCommand = new RelayCommand(() => IsProjectAiTab = true);
        ShowNodeDetailsCommand = new RelayCommand(() => IsProjectAiTab = false);
        ImportCommand = new RelayCommand(() => _ = LoadWorkflowAsync());
        ExportCommand = new RelayCommand(() => _ = ExportWorkflowAsync());
        SaveCommand = new RelayCommand(() => _ = SaveWorkflowAsync());
        AddContextNodeCommand = new RelayCommand(() => AddNode("llm"));
        AddStartNodeCommand = new RelayCommand(() => AddNode("start"));
        DeleteSelectedNodeCommand = new RelayCommand(DeleteSelectedNode);
        RunSelectedNodeCommand = new RelayCommand(() => _ = RunSelectedNodeAsync());
        PauseWorkflowCommand = new RelayCommand(() => _ = PauseWorkflowAsync());
        StopWorkflowCommand = new RelayCommand(() => _ = StopWorkflowAsync());
        ResumeWorkflowCommand = new RelayCommand(() => _ = ResumeWorkflowAsync());
        SendProjectAiCommand = new RelayCommand(() => _ = SendProjectAiAsync());
        ApplyNodeConfigCommand = new RelayCommand(() => _ = ApplyNodeConfigAsync());
        ToggleBreakpointCommand = new RelayCommand(() => _ = ToggleBreakpointAsync());
        AddAnnotationCommand = new RelayCommand(() => _ = AddAnnotationAsync());
        ExportSelectionCommand = new RelayCommand(() => _ = ExportWorkflowAsync());
        PackSelectionCommand = new RelayCommand(() => _ = PackSelectionAsync());
        RefreshConfirmationsCommand = new RelayCommand(() => _ = LoadConfirmationsAsync());
        ApproveConfirmationCommand = new RelayCommand(() => _ = ResolveSelectedConfirmationAsync("approve"));
        RejectConfirmationCommand = new RelayCommand(() => _ = ResolveSelectedConfirmationAsync("reject"));
        _projectAiAnswer = displayNames.Text("ui.workspace.project_ai.empty");

        Nodes = new ObservableCollection<WorkflowNodeViewModel>();
        Confirmations = new ObservableCollection<ConfirmationItemViewModel>();
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

        AddNode("start", capture: false);
        CaptureSnapshot();
        _ = LoadWorkflowAsync();
        _ = LoadConfirmationsAsync();
    }

    public string Title => _displayNames.Text("ui.nav.workspace");
    public string SaveText => _displayNames.Text("ui.workspace.save");
    public string ImportText => _displayNames.Text("ui.workspace.import");
    public string ExportText => _displayNames.Text("ui.workspace.export");
    public string RunText => _displayNames.Text("ui.workspace.run");
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

    public bool IsRightPanelOpen { get => _isRightPanelOpen; set => SetProperty(ref _isRightPanelOpen, value); }
    public RelayCommand ToggleRightPanelCommand { get; }
    public bool IsLibraryOpen { get => _isLibraryOpen; set => SetProperty(ref _isLibraryOpen, value); }
    public RelayCommand ToggleLibraryCommand { get; }

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

    public string StatusText { get => _statusText; set => SetProperty(ref _statusText, value); }
    public string ProjectAiMessage { get => _projectAiMessage; set => SetProperty(ref _projectAiMessage, value); }
    public string ProjectAiAnswer { get => _projectAiAnswer; set => SetProperty(ref _projectAiAnswer, value); }
    public string CurrentRunId { get => _currentRunId; set => SetProperty(ref _currentRunId, value); }
    public string ConfirmationReason { get => _confirmationReason; set => SetProperty(ref _confirmationReason, value); }
    public string AnnotationTitle { get => _annotationTitle; set => SetProperty(ref _annotationTitle, value); }

    public bool HasUnsavedChanges
    {
        get => _hasUnsavedChanges;
        private set => SetProperty(ref _hasUnsavedChanges, value);
    }

    public ObservableCollection<WorkflowNodeViewModel> Nodes { get; }
    public ObservableCollection<NodeLibraryItemViewModel> EntryNodes { get; }
    public ObservableCollection<NodeLibraryItemViewModel> WritingAgents { get; }
    public ObservableCollection<NodeLibraryItemViewModel> UtilityNodes { get; }
    public ObservableCollection<ConfirmationItemViewModel> Confirmations { get; }

    public WorkflowNodeViewModel? SelectedNode
    {
        get => _selectedNode;
        private set
        {
            if (SetProperty(ref _selectedNode, value))
            {
                OnPropertyChanged(nameof(HasSelectedNode));
                OnPropertyChanged(nameof(SelectedNodeTitle));
            }
        }
    }

    public bool HasSelectedNode => SelectedNode is not null;

    public ConfirmationItemViewModel? SelectedConfirmation
    {
        get => _selectedConfirmation;
        private set => SetProperty(ref _selectedConfirmation, value);
    }

    public string CtxAddNodeText => _displayNames.Text("ui.workspace.context.add_node");
    public string CtxAddStartText => _displayNames.Text("ui.workspace.context.add_start");
    public string CtxPasteText => _displayNames.Text("ui.workspace.context.paste");
    public string CtxSelectAllText => _displayNames.Text("ui.workspace.context.select_all");
    public string CtxFitViewText => _displayNames.Text("ui.workspace.context.fit_view");
    public string CtxCopyText => _displayNames.Text("ui.workspace.context.copy");
    public string CtxCutText => _displayNames.Text("ui.workspace.context.cut");
    public string CtxDeleteText => _displayNames.Text("ui.workspace.context.delete");

    private void AddNode(string nodeType, bool capture = true)
    {
        var label = NodeLabel(nodeType);
        var node = new WorkflowNodeViewModel(
            id: $"{nodeType}-{_nextNodeNumber++}",
            nodeType,
            label,
            defaultWorkDir: nodeType == "start" ? "novels/正篇" : string.Empty,
            x: 120 + ((Nodes.Count % 4) * 230),
            y: 80 + ((Nodes.Count / 4) * 170),
            _backend,
            () => SelectNode(node: null),
            RefreshDirtyState);
        node.SelectCommand = new RelayCommand(() => SelectNode(node));
        node.RunCommand = new RelayCommand(() => _ = RunNodeAsync(node));
        Nodes.Add(node);
        SelectNode(node);
        if (capture)
        {
            RefreshDirtyState();
        }
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

    private void DeleteSelectedNode()
    {
        if (SelectedNode is null)
        {
            return;
        }
        Nodes.Remove(SelectedNode);
        SelectedNode = null;
        RefreshDirtyState();
    }

    private async Task LoadWorkflowAsync()
    {
        try
        {
            var graph = await _backend.LoadWorkflowGraphAsync(DefaultWorkflowId).ConfigureAwait(true);
            ApplyGraph(graph);
            CaptureSnapshot();
            StatusText = _displayNames.Text("ui.common.open");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task SaveWorkflowAsync()
    {
        try
        {
            var graph = BuildGraph();
            await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
            await _backend.SaveWorkflowGraphAsync(graph).ConfigureAwait(true);
            CaptureSnapshot();
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
            await _backend.ExportWorkflowSelectionAsync(DefaultWorkflowId, selected).ConfigureAwait(true);
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
            var run = await _backend.RunWorkflowAsync(DefaultWorkflowId, startNodeId).ConfigureAwait(true);
            CurrentRunId = run.RunId;
            node.StatusText = run.Status;
            StatusText = run.Status;
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
        await RunControlAsync((workflowId, runId) => _backend.StopWorkflowAsync(workflowId, runId, StatusText));
    }

    private async Task ResumeWorkflowAsync()
    {
        await RunControlAsync((workflowId, runId) => _backend.ResumeWorkflowAsync(workflowId, runId));
    }

    private async Task RunControlAsync(Func<string, string, Task<WorkflowRunStarted>> action)
    {
        if (string.IsNullOrWhiteSpace(CurrentRunId))
        {
            StatusText = _displayNames.Text("ui.common.none");
            return;
        }
        try
        {
            var result = await action(DefaultWorkflowId, CurrentRunId).ConfigureAwait(true);
            StatusText = result.Status;
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task SendProjectAiAsync()
    {
        try
        {
            if (HasUnsavedChanges)
            {
                var graph = BuildGraph();
                await _backend.ValidateWorkflowGraphAsync(graph).ConfigureAwait(true);
                await _backend.SaveWorkflowGraphAsync(graph).ConfigureAwait(true);
                CaptureSnapshot();
            }
            var result = await _backend.ProjectAiChatAsync(
                ProjectAiMessage,
                ProjectAiMessage.Contains("/run", StringComparison.OrdinalIgnoreCase) ? DefaultWorkflowId : null).ConfigureAwait(true);
            ProjectAiAnswer = result.Answer;
            StatusText = result.WorkflowRun?.Status ?? _displayNames.Text("ui.common.configured");
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
            await _backend.ApplyNodeDetailPatchAsync(DefaultWorkflowId, new NodeDetailPatch(
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
            await _backend.SetNodeBreakpointAsync(DefaultWorkflowId, SelectedNode.Id, SelectedNode.BreakpointEnabled).ConfigureAwait(true);
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
        try
        {
            await _backend.UpsertCanvasAnnotationAsync(DefaultWorkflowId, new CanvasAnnotation(
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
        try
        {
            var title = _displayNames.Format("ui.workspace.subworkflow_title", new Dictionary<string, string>
            {
                ["count"] = selected.Length.ToString(),
            });
            var graph = await _backend.PackWorkflowSelectionAsync(DefaultWorkflowId, selected, null, title).ConfigureAwait(true);
            ApplyGraph(graph);
            CaptureSnapshot();
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
            foreach (var entry in entries)
            {
                Confirmations.Add(new ConfirmationItemViewModel(entry, SelectConfirmation));
            }
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
        try
        {
            var result = await _backend.ResolveConfirmationAsync(
                DefaultWorkflowId,
                CurrentRunId,
                SelectedConfirmation.ConfirmationId,
                decision,
                string.IsNullOrWhiteSpace(ConfirmationReason) ? null : ConfirmationReason).ConfigureAwait(true);
            StatusText = result.State;
            await LoadConfirmationsAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
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

    private WorkflowGraphData BuildGraph()
    {
        return new WorkflowGraphData(
            DefaultWorkflowId,
            "Default",
            Nodes.Select(node => new CanvasNode(
                node.Id,
                node.NodeType,
                node.Label,
                node.ToData(),
                new CanvasPosition(node.X, node.Y))).ToArray(),
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>());
    }

    private void ApplyGraph(WorkflowGraphData graph)
    {
        _suppressSnapshotChecks = true;
        try
        {
            Nodes.Clear();
            SelectedNode = null;
            foreach (var graphNode in graph.Nodes)
            {
                var node = new WorkflowNodeViewModel(
                    graphNode.Id,
                    graphNode.Type,
                    graphNode.Label ?? NodeLabel(graphNode.Type),
                    ReadString(graphNode.Data, "work_dir"),
                    graphNode.Position?.X ?? 120 + ((Nodes.Count % 4) * 230),
                    graphNode.Position?.Y ?? 80 + ((Nodes.Count / 4) * 170),
                    _backend,
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
                node.SelectCommand = new RelayCommand(() => SelectNode(node));
                node.RunCommand = new RelayCommand(() => _ = RunNodeAsync(node));
                Nodes.Add(node);
            }
            if (Nodes.Count == 0)
            {
                AddNode("start", capture: false);
            }
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
        HasUnsavedChanges = false;
    }

    private void RestoreSnapshot()
    {
        try
        {
            var graph = JsonSerializer.Deserialize<WorkflowGraphData>(_savedSnapshot, new JsonSerializerOptions(JsonSerializerDefaults.Web));
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
            HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot;
        }
    }

    private string CurrentSnapshot()
    {
        return JsonSerializer.Serialize(BuildGraph(), new JsonSerializerOptions(JsonSerializerDefaults.Web));
    }

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
    private readonly IAriadneBackendClient _backend;
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
    public string StatusText { get => _statusText; set => SetProperty(ref _statusText, value); }
    public double X { get => _x; set => SetProperty(ref _x, value); }
    public double Y { get => _y; set => SetProperty(ref _y, value); }
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }

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

    private async Task RunAsync()
    {
        try
        {
            var run = await _backend.RunWorkflowAsync("default", IsStartNode ? Id : null).ConfigureAwait(true);
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
