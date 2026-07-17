using System.Collections.Concurrent;
using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class WorkspaceWorkflowNavigationTests
{
    [Fact]
    public async Task Reload_ExposesAllWorkflows_AndConfirmationSelectionDoesNotSwitchContext()
    {
        var backend = WorkflowBackend.Create();
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await vm.ReloadProjectDataAsync();

        Assert.Equal(2, vm.WorkflowSummaries.Count);
        Assert.Equal("default", vm.SelectedWorkflowId);
        Assert.Equal("默认流程", vm.CurrentWorkflowName);
        Assert.Equal(new[] { "default" }, backend.LoadedWorkflowIds);
        Assert.NotNull(vm.SelectedConfirmation);
        Assert.Equal("review", vm.SelectedConfirmation!.WorkflowId);
        Assert.Equal("来源：工作流 review · 运行 run-review", vm.SelectedConfirmation.SourceText);
        Assert.Empty(vm.CurrentRunId);
        Assert.True(vm.CanOpenSelectedConfirmationWorkflow);
        Assert.True(vm.OpenSelectedConfirmationWorkflowCommand.CanExecute(null));
    }

    [Fact]
    public async Task WorkflowSelector_LoadsSelectedWorkflow_AndUpdatesVisibleIdentity()
    {
        var backend = WorkflowBackend.Create();
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();

        vm.SelectedWorkflowId = "review";
        await WaitUntilAsync(() => backend.LoadedWorkflowIds.Contains("review"));

        Assert.Equal("review", vm.SelectedWorkflowId);
        Assert.Equal("审阅流程", vm.CurrentWorkflowName);
        Assert.False(vm.CanOpenSelectedConfirmationWorkflow);
    }

    [Fact]
    public async Task ConfirmationSourceSwitch_IsExplicit_AndForeignResolutionDoesNotPolluteCurrentRun()
    {
        var backend = WorkflowBackend.Create();
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();

        Assert.True(vm.ApproveConfirmationCommand.TryExecute());
        await WaitUntilAsync(() => vm.Confirmations.Count == 0);

        Assert.Equal("review", backend.ResolvedWorkflowId);
        Assert.Equal("run-review", backend.ResolvedRunId);
        Assert.Equal("default", vm.SelectedWorkflowId);
        Assert.Empty(vm.CurrentRunId);

        backend.RestorePendingConfirmation();
        await vm.ReloadProjectDataAsync();
        Assert.True(vm.OpenSelectedConfirmationWorkflowCommand.TryExecute());
        await WaitUntilAsync(() => backend.LoadedWorkflowIds.LastOrDefault() == "review");

        Assert.Equal("review", vm.SelectedWorkflowId);
        Assert.Equal("审阅流程", vm.CurrentWorkflowName);
        Assert.Empty(vm.CurrentRunId);
    }

    [Fact]
    public void WorkspaceView_UsesWorkflowSelector_SourceAction_AndHonestReloadLabel()
    {
        var xaml = File.ReadAllText(ResolveDesktopSource("Views", "WorkspacePageView.axaml"));
        var vm = new WorkspacePageViewModel(
            DisplayNameService.LoadDefault(),
            WorkflowBackend.Create().Client);

        Assert.Contains("ItemsSource=\"{Binding WorkflowSummaries}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("SelectedValue=\"{Binding SelectedWorkflowId, Mode=TwoWay}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding CurrentWorkflowName}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding UnsavedChangesBadgeText}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding SelectedConfirmation.SourceText}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding OpenSelectedConfirmationWorkflowCommand}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding ReloadDefaultWorkflowCommand}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Ariadne.Icon.Refresh", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("{Binding ImportText}", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("Ariadne.Icon.Import", xaml, StringComparison.Ordinal);
        Assert.Equal("重新加载默认工作流", vm.ReloadDefaultWorkflowText);
        Assert.Equal("打开来源工作流", vm.OpenConfirmationWorkflowText);
    }

    private static async Task WaitUntilAsync(Func<bool> predicate)
    {
        for (var attempt = 0; attempt < 100; attempt++)
        {
            if (predicate())
            {
                return;
            }
            await Task.Delay(10);
        }

        Assert.Fail("异步工作流状态未在预期时间内收敛。");
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null)
        {
            var candidate = Path.Combine(
                new[] { directory.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            directory = directory.Parent;
        }

        throw new FileNotFoundException(string.Join('/', parts));
    }

    private class WorkflowBackend : DispatchProxy
    {
        private readonly ConcurrentQueue<string> _loadedWorkflowIds = new();
        private bool _confirmationPending = true;

        public IAriadneBackendClient Client { get; private set; } = null!;
        public IReadOnlyList<string> LoadedWorkflowIds => _loadedWorkflowIds.ToArray();
        public string? ResolvedWorkflowId { get; private set; }
        public string? ResolvedRunId { get; private set; }

        public static WorkflowBackend Create()
        {
            var client = Create<IAriadneBackendClient, WorkflowBackend>();
            var backend = (WorkflowBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        public void RestorePendingConfirmation()
        {
            _confirmationPending = true;
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
                nameof(IAriadneBackendClient.ListWorkflowGraphsAsync) => WorkflowSummaries(),
                nameof(IAriadneBackendClient.LoadWorkflowGraphAsync) => LoadWorkflow((string)args![0]!),
                nameof(IAriadneBackendClient.ListConfirmationsAsync) =>
                    _confirmationPending ? new[] { ForeignConfirmation() } : Array.Empty<ConfirmationLogEntry>(),
                nameof(IAriadneBackendClient.ResolveConfirmationAsync) => ResolveConfirmation(args!),
                nameof(IAriadneBackendClient.GetProviderConfigAsync) => EmptyProviderConfig(),
                nameof(IAriadneBackendClient.GetWorksTreeAsync) => EmptyWorksTree(),
                _ => throw new NotSupportedException(targetMethod.Name),
            };

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }

            throw new NotSupportedException(targetMethod.Name);
        }

        private WorkflowGraphData LoadWorkflow(string workflowId)
        {
            _loadedWorkflowIds.Enqueue(workflowId);
            var name = workflowId == "review" ? "审阅流程" : "默认流程";
            return new WorkflowGraphData(
                workflowId,
                name,
                Array.Empty<CanvasNode>(),
                Array.Empty<CanvasEdge>(),
                new Dictionary<string, object?>());
        }

        private ResolveConfirmationResult ResolveConfirmation(object?[] args)
        {
            ResolvedWorkflowId = args[0] as string;
            ResolvedRunId = args[1] as string;
            _confirmationPending = false;
            return new ResolveConfirmationResult(
                new WorkflowActionResult("review", "run-review", "running"),
                ForeignConfirmation() with { State = "approved" },
                new SidebarBadgeCounts(0, 0, 0));
        }

        private static IReadOnlyList<WorkflowSummary> WorkflowSummaries() =>
            new[]
            {
                new WorkflowSummary("default", "默认流程", "workflows/default.json", 0, 0),
                new WorkflowSummary("review", "审阅流程", "workflows/review.json", 0, 0),
            };

        private static ConfirmationLogEntry ForeignConfirmation() => new(
            "confirmation-review",
            "approval",
            "approval-node",
            1_700_000_000_000,
            "pending",
            "manual",
            "审阅章节发布",
            "diff",
            "review",
            "run-review");

        private static ProviderConfigStatus EmptyProviderConfig() => new(
            false,
            false,
            false,
            null,
            null,
            null,
            null,
            Array.Empty<ProviderKeyStatus>());

        private static WorksTreeNode EmptyWorksTree() => new(
            "root",
            "root",
            "Root",
            string.Empty,
            Array.Empty<WorksTreeNode>());
    }
}
