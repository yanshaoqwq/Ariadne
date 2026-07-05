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
    private WorkflowNodeViewModel? _selectedNode;

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
        SendProjectAiCommand = new RelayCommand(() => _ = SendProjectAiAsync());
        _projectAiAnswer = displayNames.Text("ui.workspace.project_ai.empty");

        Nodes = new ObservableCollection<WorkflowNodeViewModel>();
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
    public RelayCommand SendProjectAiCommand { get; }

    public string StatusText { get => _statusText; set => SetProperty(ref _statusText, value); }
    public string ProjectAiMessage { get => _projectAiMessage; set => SetProperty(ref _projectAiMessage, value); }
    public string ProjectAiAnswer { get => _projectAiAnswer; set => SetProperty(ref _projectAiAnswer, value); }

    public bool HasUnsavedChanges
    {
        get => _hasUnsavedChanges;
        private set => SetProperty(ref _hasUnsavedChanges, value);
    }

    public ObservableCollection<WorkflowNodeViewModel> Nodes { get; }
    public ObservableCollection<NodeLibraryItemViewModel> EntryNodes { get; }
    public ObservableCollection<NodeLibraryItemViewModel> WritingAgents { get; }
    public ObservableCollection<NodeLibraryItemViewModel> UtilityNodes { get; }

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
            await _backend.SaveWorkflowGraphAsync(BuildGraph()).ConfigureAwait(true);
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
            await _backend.SaveWorkflowGraphAsync(BuildGraph()).ConfigureAwait(true);
            CaptureSnapshot();
            await _backend.ExportWorkflowSelectionAsync(DefaultWorkflowId, selected).ConfigureAwait(true);
            StatusText = _displayNames.Text("ui.common.export");
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
            await _backend.SaveWorkflowGraphAsync(BuildGraph()).ConfigureAwait(true);
            CaptureSnapshot();
            var run = await _backend.RunWorkflowAsync(DefaultWorkflowId, startNodeId).ConfigureAwait(true);
            node.StatusText = run.Status;
            StatusText = run.Status;
        }
        catch (Exception ex)
        {
            node.StatusText = ex.Message;
            StatusText = ex.Message;
        }
    }

    private async Task SendProjectAiAsync()
    {
        try
        {
            if (HasUnsavedChanges)
            {
                await _backend.SaveWorkflowGraphAsync(BuildGraph()).ConfigureAwait(true);
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
            return element.ValueKind == JsonValueKind.String ? element.GetString() ?? fallback : fallback;
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
        if (propertyName is nameof(Name) or nameof(WorkDir) or nameof(ExposedAsTool) or nameof(X) or nameof(Y))
        {
            _markDirty();
        }
    }
}
