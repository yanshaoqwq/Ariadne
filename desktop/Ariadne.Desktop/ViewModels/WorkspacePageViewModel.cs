using System.Collections.ObjectModel;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// 工作空间（智能体编排）页 VM。
/// 本轮为视觉骨架：画布 + 下侧节点库/执行页 + 右侧项目 AI/节点细节标签。
/// 真实图数据与运行控制在交互阶段对接 load_workflow_graph / run_workflow 等 command。
public sealed class WorkspacePageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private bool _isRightPanelOpen = true;
    private bool _isLibraryOpen = true;
    private bool _isProjectAiTab = true;

    public WorkspacePageViewModel(DisplayNameService displayNames)
    {
        _displayNames = displayNames;
        ToggleRightPanelCommand = new RelayCommand(() => IsRightPanelOpen = !IsRightPanelOpen);
        ToggleLibraryCommand = new RelayCommand(() => IsLibraryOpen = !IsLibraryOpen);
        ShowProjectAiCommand = new RelayCommand(() => IsProjectAiTab = true);
        ShowNodeDetailsCommand = new RelayCommand(() => IsProjectAiTab = false);

        // 起始节点是节点库里的入口类型节点，从库拖到画布使用（此处放置一个作为示例）。
        // 左上=执行按钮，右上=单个执行输出口；体内含名称/工作目录/是否注册为 AI 工具。
        StartNode = new StartNodeViewModel(displayNames, displayNames.Text("ui.workspace.start_node.default_name"), "novels/正篇");

        // 入口节点分组（节点库）
        EntryNodes = new ObservableCollection<string>
        {
            displayNames.Text("ui.workspace.start_node.title"),
        };

        WritingAgents = new ObservableCollection<string>
        {
            displayNames.Text("agent.outliner"),
            displayNames.Text("agent.designer"),
            displayNames.Text("agent.planner"),
            displayNames.Text("agent.detail"),
            displayNames.Text("agent.writer"),
            displayNames.Text("agent.critic"),
            displayNames.Text("agent.prudent"),
            displayNames.Text("agent.polisher"),
            displayNames.Text("agent.summarizer"),
        };

        UtilityNodes = new ObservableCollection<string>
        {
            displayNames.Text("ui.node.llm"),
            displayNames.Text("ui.node.document"),
            displayNames.Text("ui.node.search"),
            displayNames.Text("ui.node.condition"),
            displayNames.Text("ui.node.loop"),
            displayNames.Text("ui.node.approval"),
            displayNames.Text("ui.node.export"),
        };
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

    public string ProjectAiEmptyText => _displayNames.Text("ui.workspace.project_ai.empty");

    public string ProjectAiPlaceholder => _displayNames.Text("ui.workspace.project_ai.placeholder");

    public string NoNodeSelectedText => _displayNames.Text("ui.workspace.no_node_selected");

    public string CanvasHintText => _displayNames.Text("ui.workspace.logs_hint");

    public string ToggleRightPanelText => _displayNames.Text("ui.action.toggle_right_panel");

    public string EntryNodesText => _displayNames.Text("ui.workspace.entry_nodes");

    /// 右侧栏开合状态；收起后由悬浮左向箭头重新展开。
    public bool IsRightPanelOpen
    {
        get => _isRightPanelOpen;
        set => SetProperty(ref _isRightPanelOpen, value);
    }

    public RelayCommand ToggleRightPanelCommand { get; }

    /// 下栏节点库开合状态。
    public bool IsLibraryOpen
    {
        get => _isLibraryOpen;
        set => SetProperty(ref _isLibraryOpen, value);
    }

    public RelayCommand ToggleLibraryCommand { get; }

    /// 右栏标签：true=项目 AI，false=节点细节。
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

    /// 画布上已放置的起始节点示例。
    public StartNodeViewModel StartNode { get; }

    /// 入口节点分组（节点库）。
    public ObservableCollection<string> EntryNodes { get; }

    public ObservableCollection<string> WritingAgents { get; }

    public ObservableCollection<string> UtilityNodes { get; }
}

/// 画布上的单个起始节点：工作流入口。
/// - 名称可编辑；注册为项目空间 AI 工具时即以此名暴露。
/// - 工作目录决定其下游节点的读写根目录，实现同系列多作品的文件隔离。
///   （后端契约见《后端需做.md》：run_workflow 按 start_node 的 work_dir 定位下游文档作用域。）
public sealed class StartNodeViewModel : ViewModelBase
{
    private string _name;
    private string _workDir;
    private bool _exposedAsTool = true;

    public StartNodeViewModel(DisplayNameService displayNames, string name, string workDir)
    {
        _name = name;
        _workDir = workDir;
        TitleText = displayNames.Text("ui.workspace.start_node.title");
        NameLabel = displayNames.Text("ui.workspace.start_node.name_label");
        WorkDirLabel = displayNames.Text("ui.workspace.start_node.work_dir_label");
        WorkDirPlaceholder = displayNames.Text("ui.workspace.start_node.work_dir_placeholder");
        ExposeToolLabel = displayNames.Text("ui.workspace.start_node.expose_tool");
        RunText = displayNames.Text("ui.workspace.run");
        // 执行按钮：从此起点启动工作流（骨架阶段占位，交互阶段接 run_workflow）。
        RunCommand = new RelayCommand(() => { });
    }

    public string TitleText { get; }

    public string NameLabel { get; }

    public string WorkDirLabel { get; }

    public string WorkDirPlaceholder { get; }

    public string ExposeToolLabel { get; }

    public string RunText { get; }

    /// 起始节点名称，可编辑。
    public string Name
    {
        get => _name;
        set => SetProperty(ref _name, value);
    }

    /// 下游节点的工作目录（项目内相对路径）；不同起点用不同目录以隔离作品。
    public string WorkDir
    {
        get => _workDir;
        set => SetProperty(ref _workDir, value);
    }

    /// 是否把该起点对应的工作流注册为项目空间 AI 的可调用工具。
    public bool ExposedAsTool
    {
        get => _exposedAsTool;
        set => SetProperty(ref _exposedAsTool, value);
    }

    /// 左上角执行按钮命令。
    public RelayCommand RunCommand { get; }
}
