using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U196-D 回归：工作流某个节点失败后，作者在界面上有「从失败处重跑」可点。
///
/// # 原缺陷
///
/// 第 7 个节点失败时前 6 个的产出都在，而作者只能整条重跑（重烧前 6 步的钱）。
/// 后端此前没有这个能力，前端自然也没有入口。
///
/// # 判据落在「发出了哪个 IPC 方法、带了哪个 node_id」
///
/// **不能**只测「命令能不能执行」：把这颗按钮接到 `resume_workflow` 上，
/// 按钮点得动、请求发得出、回包也不报错，而运行状态一动不动
/// —— 后端 `store::claim_resume` 只接受 `Paused | Queued | Running`，
/// 失败的运行拿到 NotResumable。那种缺陷在屏幕上与「重跑正在进行」同形。
/// 所以本文件的主判据是 <see cref="RetryFailedNode_SendsRetryFailedNodeWithTheFailedNodeId"/>：
/// 断言真实出站请求的**方法名**与 **node_id**。
///
/// # 反向也要钉住
///
/// 「一律显示这颗按钮」同样能让主判据全绿，而那会在成功/运行中的画布上
/// 挂一个语义错误的入口。所以有 <see cref="RetryEntry_StaysHiddenWhenTheRunDidNotFail"/>。
/// </summary>
public sealed class FailedNodeRetryEntryTests
{
    private const string FailedNode = "writer";

    /// <summary>
    /// **主判据：点下去真的发出了 `retry_failed_node`，且带着失败那个节点的 id。**
    ///
    /// 判据不是「命令能不能执行」，而是**出站请求的方法名与参数**：
    /// 接到 `resume_workflow` 上的版本同样「命令可执行、请求发出、回包正常」，
    /// 而后端会以 NotResumable 拒绝，运行状态一动不动。
    /// </summary>
    [Fact]
    public async Task RetryFailedNode_SendsRetryFailedNodeWithTheFailedNodeId()
    {
        var (vm, backend) = await FailedCanvasAsync();

        // 前置：没有这两条，用例在「页面根本没跑到 failed」时也会因为
        // 后面什么都没发生而"看起来对"。
        Assert.Equal("failed", vm.CurrentRunStatus);
        Assert.Equal(FailedNode, vm.FailedNodeId);

        Assert.True(
            vm.RetryFailedNodeCommand.CanExecute(null),
            "运行失败了却没有「从失败处重跑」可点 ⇒ 作者只能整条重跑，重烧前面几步的钱");
        vm.RetryFailedNodeCommand.Execute(null);
        await WaitForControlCallAsync(backend);

        var call = Assert.Single(backend.ControlCalls);
        Assert.Equal(nameof(IAriadneBackendClient.RetryFailedNodeAsync), call.Method);
        Assert.Equal(
            FailedNode,
            call.NodeId,
            ignoreCase: false);
    }

    /// <summary>
    /// **反向：不该出现的时候不能出现。**
    ///
    /// 缺了这条，「无条件显示 + 无条件可点」也能让主判据全绿 —— 而那是在
    /// 成功/运行中的画布上挂一个语义错误的入口：运行中点下去会与在跑的 worker
    /// 抢同一条运行，成功后点下去会把已付费的产出清掉重跑。
    /// </summary>
    [Fact]
    public async Task RetryEntry_StaysHiddenWhenTheRunDidNotFail()
    {
        var backend = RetryProbeBackend.Create();
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();

        // 从未跑过：没有 run，入口不该在。
        Assert.False(vm.CanRetryFromFailedNode);
        Assert.False(vm.RetryFailedNodeCommand.CanExecute(null));

        var session = typeof(WorkspacePageViewModel)
            .GetField("_runSession", BindingFlags.Instance | BindingFlags.NonPublic)!
            .GetValue(vm)!;
        var attach = session.GetType().GetMethod("Attach", BindingFlags.Instance | BindingFlags.Public)!;

        foreach (var status in new[] { "running", "paused", "succeeded", "stopped" })
        {
            attach.Invoke(session, new object?[] { "default", "run-1", status, false, false });
            Assert.False(
                vm.CanRetryFromFailedNode,
                $"状态 {status} 下出现了「从失败处重跑」入口");
            Assert.False(
                vm.RetryFailedNodeCommand.CanExecute(null),
                $"状态 {status} 下「从失败处重跑」可点");
        }

        // 全程一个控制类请求都不该发出。
        Assert.Empty(backend.ControlCalls);
    }

    /// <summary>
    /// **失败了但不知道是哪个节点 ⇒ 入口不出现。**
    ///
    /// 给一颗不知道要重跑什么的「从失败处重跑」，比没有这颗更糟：作者点下去
    /// 得到的是一句后端拒绝，而他刚刚才读过一次失败。
    /// （运行级失败——worker 创建失败之类——的 `stage` 不是节点 id，属这种情形。）
    /// </summary>
    [Fact]
    public async Task RetryEntry_StaysHiddenWhenTheFailedNodeIsUnknown()
    {
        var backend = RetryProbeBackend.Create();
        backend.FailedNodeId = string.Empty;
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();

        var session = typeof(WorkspacePageViewModel)
            .GetField("_runSession", BindingFlags.Instance | BindingFlags.NonPublic)!
            .GetValue(vm)!;
        var attach = session.GetType().GetMethod("Attach", BindingFlags.Instance | BindingFlags.Public)!;
        attach.Invoke(session, new object?[] { "default", "run-1", "running", false, false });
        attach.Invoke(session, new object?[] { "default", "run-1", "failed", false, false });
        // 等那次 get_workflow_run_state 回来（它会把 FailedNodeId 置成空串，
        // 等不到「变化」，所以只能等状态稳定后再断言）。
        await Task.Delay(150);

        Assert.Equal("failed", vm.CurrentRunStatus);
        Assert.Equal(string.Empty, vm.FailedNodeId);
        Assert.False(vm.CanRetryFromFailedNode);
        Assert.False(vm.RetryFailedNodeCommand.CanExecute(null));
        Assert.Empty(backend.ControlCalls);
    }

    private static async Task WaitForControlCallAsync(RetryProbeBackend backend)
    {
        for (var attempt = 0; attempt < 300 && backend.ControlCalls.Count == 0; attempt++)
        {
            await Task.Delay(10);
        }
        Assert.NotEmpty(backend.ControlCalls);
    }

    /// <summary>
    /// **可用性变化必须广播 PropertyChanged。**
    ///
    /// 这条专拦一个盲区：其余用例都直接读属性 ⇒「值算对了但没广播」它们全都测不到。
    /// 而界面是绑定驱动的，没广播时会一直停在初始值 ——
    /// 「后端已经在等作者点重跑，而屏幕上那颗按钮从未出现」，
    /// 与「根本没接这条入口」在屏幕上完全同形。
    /// </summary>
    [Fact]
    public async Task RetryEntryAvailability_RaisesPropertyChanged()
    {
        var backend = RetryProbeBackend.Create();
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();

        var seen = new List<string>();
        vm.PropertyChanged += (_, args) =>
        {
            if (!string.IsNullOrEmpty(args.PropertyName))
            {
                seen.Add(args.PropertyName!);
            }
        };

        await DriveRunToFailedAsync(vm, backend);

        Assert.Contains(nameof(WorkspacePageViewModel.CanRetryFromFailedNode), seen);
        Assert.Contains(nameof(WorkspacePageViewModel.FailedNodeId), seen);
    }

    /// <summary>
    /// **界面真的绑了这些属性，入口真的有渲染位。**
    ///
    /// ViewModel 全对而 XAML 没绑，是本仓反复出现的「做一半」形态：
    /// 所有 VM 用例全绿，而屏幕上一颗按钮都没多出来。
    /// </summary>
    [Fact]
    public void WorkspacePageView_RendersTheRetryEntry()
    {
        var view = File.ReadAllText(ResolveRepoFile(Path.Combine(
            "desktop", "Ariadne.Desktop", "Views", "WorkspacePageView.axaml")));

        foreach (var binding in new[]
                 {
                     "{Binding CanRetryFromFailedNode}",
                     "{Binding RetryFailedNodeCommand}",
                     "{Binding RetryFailedNodeText}",
                     // 禁用/隐藏理由必须配文字：这颗按钮的字面意思说不清
                     // 「前面已完成的步骤不会重复花钱」，而那正是作者最怕的事。
                     "{Binding RetryFailedNodeTooltip}",
                 })
        {
            Assert.Contains(binding, view, StringComparison.Ordinal);
        }

        // 有命名的宿主控件 ⇒ 它是一个真实的渲染位，不是只写在注释里。
        Assert.Contains("x:Name=\"RetryFailedNodeButton\"", view, StringComparison.Ordinal);
    }

    /// <summary>
    /// 三份语言包都要有这两个 key。
    ///
    /// en/ja 缺键时 <c>DisplayNameService</c> 静默回落到 zh —— 不报错、
    /// 只是英文/日文界面上突然出现两行中文，唯一的发现途径就是这条断言。
    /// </summary>
    [Fact]
    public void RetryEntryCopy_ExistsInEveryLanguagePack()
    {
        foreach (var pack in new[]
                 {
                     "display_name.json",
                     "display_name.en.json",
                     "display_name.ja.json",
                 })
        {
            var path = ResolveRepoFile(Path.Combine("core", "resources", pack));
            using var document = System.Text.Json.JsonDocument.Parse(File.ReadAllText(path));
            foreach (var key in new[]
                     {
                         "ui.workspace.retry_failed_node",
                         "ui.workspace.retry_failed_node_tooltip",
                     })
            {
                Assert.True(
                    document.RootElement.TryGetProperty(key, out var value),
                    $"{pack} 缺少 {key}");
                Assert.False(
                    string.IsNullOrWhiteSpace(value.GetString()),
                    $"{pack} 的 {key} 是空串");
            }
        }

        // 中文那份必须真的解析得出（不是 `[key]` 那种缺键回落形态）。
        var names = DisplayNameService.LoadDefault();
        Assert.DoesNotContain("[ui.workspace.retry", names.Text("ui.workspace.retry_failed_node"), StringComparison.Ordinal);
        Assert.DoesNotContain("[ui.workspace.retry", names.Text("ui.workspace.retry_failed_node_tooltip"), StringComparison.Ordinal);
    }

    private static string ResolveRepoFile(string relative)
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 12 && dir is not null; i++)
        {
            var candidate = Path.Combine(dir.FullName, relative);
            if (File.Exists(candidate))
            {
                return candidate;
            }
            dir = dir.Parent;
        }
        throw new FileNotFoundException($"找不到 {relative}（从 {AppContext.BaseDirectory} 向上找）");
    }

    /// <summary>
    /// 把 run 会话按到 failed 上（走真实的 `Attach` 跃迁，不起轮询），
    /// 并等 `LoadRunFailureRecoveryAsync` 那次 fire-and-forget 的
    /// `get_workflow_run_state` 真的回来。
    ///
    /// 等待条件刻意取「FailedNodeId **等于**后端给的那个节点」而不是「非空」：
    /// 等「非空」会在错误的值上提前退出，于是断言读到的是来自别处的数据
    /// （「变异全绿也可能是遗留状态」那条教训的二次变异形态）。
    /// </summary>
    private static async Task DriveRunToFailedAsync(
        WorkspacePageViewModel vm,
        RetryProbeBackend backend)
    {
        var session = typeof(WorkspacePageViewModel)
            .GetField("_runSession", BindingFlags.Instance | BindingFlags.NonPublic)!
            .GetValue(vm)!;
        var attach = session.GetType().GetMethod("Attach", BindingFlags.Instance | BindingFlags.Public)!;
        // 先 running 再 failed：跃迁边沿判据要求 previous != "failed"，
        // 这也是产品里真实的形状。
        attach.Invoke(session, new object?[] { "default", "run-1", "running", false, false });
        attach.Invoke(session, new object?[] { "default", "run-1", "failed", false, false });

        for (var attempt = 0; attempt < 300; attempt++)
        {
            if (string.Equals(vm.FailedNodeId, backend.FailedNodeId, StringComparison.Ordinal))
            {
                return;
            }
            await Task.Delay(10);
        }
    }

    private static async Task<(WorkspacePageViewModel Vm, RetryProbeBackend Backend)> FailedCanvasAsync(
        string? failedNode = FailedNode)
    {
        var backend = RetryProbeBackend.Create();
        backend.FailedNodeId = failedNode ?? string.Empty;
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();
        await DriveRunToFailedAsync(vm, backend);
        // 取证环境自检：替身漏实现某个 IPC 会走 catch → ReportFailure，
        // 于是下面读到的状态来自那条失败路径而不是运行失败链路。
        Assert.Empty(backend.UnsupportedCalls);
        return (vm, backend);
    }

    // DispatchProxy 的 TProxy 不能是 sealed（运行时要为它生成子类）。
    private class RetryProbeBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        /// <summary>后端 `WorkflowRunFailure.Stage` 里放的失败节点 id。</summary>
        public string FailedNodeId { get; set; } = FailedNode;

        /// <summary>每次运行控制类 IPC 的 (方法名, node_id)。主判据读的就是这里。</summary>
        public List<(string Method, string? NodeId)> ControlCalls { get; } = new();

        public List<string> UnsupportedCalls { get; } = new();

        public static RetryProbeBackend Create()
        {
            var client = DispatchProxy.Create<IAriadneBackendClient, RetryProbeBackend>();
            var backend = (RetryProbeBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }
            if (targetMethod.Name == "get_HasProjectRoot")
            {
                return true;
            }

            object? value = targetMethod.Name switch
            {
                nameof(IAriadneBackendClient.LoadProjectCanvasAsync) => EmptyCanvas(),
                nameof(IAriadneBackendClient.ListWorkflowGraphsAsync) => Array.Empty<WorkflowSummary>(),
                nameof(IAriadneBackendClient.ListConfirmationsAsync) => Array.Empty<ConfirmationLogEntry>(),
                nameof(IAriadneBackendClient.ListInDoubtOperationsAsync) => Array.Empty<WorkflowOperation>(),
                nameof(IAriadneBackendClient.GetProviderConfigAsync) => ConfiguredProviders(),
                nameof(IAriadneBackendClient.GetAutomationSettingsAsync) => IdleAutomation(),
                nameof(IAriadneBackendClient.GetWorksTreeAsync) => EmptyWorksTree(),
                nameof(IAriadneBackendClient.GetWorkflowRunStateAsync) => FailedRunState(),
                // 三条控制类 IPC 都记账：主判据要能区分「发的是 retry_failed_node」
                // 与「发的是 resume_workflow」，只记一条就区分不出来。
                nameof(IAriadneBackendClient.RetryFailedNodeAsync) => RecordControl(
                    nameof(IAriadneBackendClient.RetryFailedNodeAsync), args?[2] as string, "queued"),
                nameof(IAriadneBackendClient.ResumeWorkflowAsync) => RecordControl(
                    nameof(IAriadneBackendClient.ResumeWorkflowAsync), null, "queued"),
                nameof(IAriadneBackendClient.RunWorkflowAsync) => RecordRun(),
                _ => Unsupported(targetMethod),
            };

            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }
            return value;
        }

        private WorkflowActionResult RecordControl(string method, string? nodeId, string status)
        {
            ControlCalls.Add((method, nodeId));
            return new WorkflowActionResult("default", "run-1", status);
        }

        private WorkflowRunStarted RecordRun()
        {
            ControlCalls.Add((nameof(IAriadneBackendClient.RunWorkflowAsync), null));
            return new WorkflowRunStarted("run-2", "queued");
        }

        private WorkflowRunState FailedRunState() => new(
            "default",
            "run-1",
            "failed",
            PauseReason: null,
            StopReason: null,
            // stage 放的是**节点 id**（runtime.rs::record_node_error 的注释写明
            // 「用户在画布上按它定位到具体哪个方块」），不是阶段名。
            Failure: new WorkflowRunFailure(
                "external",
                FailedNodeId,
                "provider returned 401",
                "请检查服务商配置后重试"),
            Events: Array.Empty<string>());

        private static WorkflowGraphData EmptyCanvas() => new(
            "default",
            "Project Canvas",
            Array.Empty<CanvasNode>(),
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: "canvas-revision");

        private static ProviderConfigStatus ConfiguredProviders() => new(
            HasOpenAiKey: true,
            HasAnthropicKey: false,
            HasGeminiKey: false,
            DefaultLlmProviderId: "openai",
            DefaultEmbeddingProviderId: null,
            DefaultRerankerProviderId: null,
            DefaultSearchProviderId: null,
            Providers: Array.Empty<ProviderKeyStatus>());

        private static AutomationSettings IdleAutomation() => new(
            new BudgetStatus(0, 0, PreauthorizedUsd: null, AutoModeEnabled: false),
            Array.Empty<ConfirmationPolicySetting>());

        private static WorksTreeNode EmptyWorksTree() => new(
            "root",
            "project",
            "Project",
            "/tmp/ariadne-u196d-probe",
            Array.Empty<WorksTreeNode>());

        /// <summary>替身没实现的 IPC：**先登记再抛**（登记是自检断言唯一的数据来源）。</summary>
        private object? Unsupported(MethodInfo method)
        {
            UnsupportedCalls.Add(method.Name);
            throw new NotSupportedException(method.Name);
        }
    }
}
