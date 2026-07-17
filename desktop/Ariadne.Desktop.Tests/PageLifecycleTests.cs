using System.Collections.Concurrent;
using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class PageLifecycleTests
{
    [Fact]
    public async Task ProjectPageSession_ConstructsWithoutIo_LoadsEachPreloadedPageOnce_AndKeepsIdentity()
    {
        var backend = CountingBackend.Create();
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);

        Assert.Equal(0, backend.Count(nameof(IAriadneBackendClient.ListWorkflowGraphsAsync)));
        Assert.Equal(0, backend.Count(nameof(IAriadneBackendClient.GetWorksTreeAsync)));
        Assert.Equal(0, backend.Count(nameof(IAriadneBackendClient.GetGitBranchGraphAsync)));

        await window.PreloadProjectPagesForTestsAsync();
        await window.PreloadProjectPagesForTestsAsync();

        Assert.Equal(1, backend.Count(nameof(IAriadneBackendClient.ListWorkflowGraphsAsync)));
        Assert.Equal(1, backend.Count(nameof(IAriadneBackendClient.LoadWorkflowGraphAsync)));
        Assert.Equal(2, backend.Count(nameof(IAriadneBackendClient.GetWorksTreeAsync)));
        Assert.Equal(1, backend.Count(nameof(IAriadneBackendClient.GetGitRepositoryStatusAsync)));
        Assert.Equal(1, backend.Count(nameof(IAriadneBackendClient.GetGitBranchGraphAsync)));

        var workspace = window.GetPageForTests("workspace");
        var works = window.GetPageForTests("works");
        var git = window.GetPageForTests("git");
        await window.OpenNavigationItemByIdAsync("workspace");
        await window.OpenNavigationItemByIdAsync("works");
        await window.OpenNavigationItemByIdAsync("git");

        Assert.Same(workspace, window.GetPageForTests("workspace"));
        Assert.Same(works, window.GetPageForTests("works"));
        Assert.Same(git, window.GetPageForTests("git"));
        Assert.Equal(1, backend.Count(nameof(IAriadneBackendClient.LoadWorkflowGraphAsync)));
        Assert.Equal(2, backend.Count(nameof(IAriadneBackendClient.GetWorksTreeAsync)));
    }

    [Fact]
    public async Task ProjectPageSession_ResetInvalidatesOldIdentity_AndStartsOneNewLoadPerPage()
    {
        var backend = CountingBackend.Create();
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await window.PreloadProjectPagesForTestsAsync();
        var oldWorkspace = window.GetPageForTests("workspace");

        window.ResetProjectPageSessionForTests();
        await window.PreloadProjectPagesForTestsAsync();

        Assert.NotSame(oldWorkspace, window.GetPageForTests("workspace"));
        Assert.Equal(2, backend.Count(nameof(IAriadneBackendClient.ListWorkflowGraphsAsync)));
        Assert.Equal(2, backend.Count(nameof(IAriadneBackendClient.LoadWorkflowGraphAsync)));
        Assert.Equal(4, backend.Count(nameof(IAriadneBackendClient.GetWorksTreeAsync)));
        Assert.Equal(2, backend.Count(nameof(IAriadneBackendClient.GetGitBranchGraphAsync)));
    }

    [Fact]
    public async Task ProjectPageSession_OldLoadCompletionCannotMarkNewSessionLoaded()
    {
        var backend = CountingBackend.Create();
        backend.BlockWorkflowList = true;
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);
        var oldPreload = window.PreloadProjectPagesForTestsAsync();
        await backend.WorkflowListStarted.Task;

        window.ResetProjectPageSessionForTests();
        backend.BlockWorkflowList = false;
        backend.ReleaseWorkflowList();
        await oldPreload;
        await window.PreloadProjectPagesForTestsAsync();

        Assert.Equal(2, backend.Count(nameof(IAriadneBackendClient.ListWorkflowGraphsAsync)));
    }

    private class CountingBackend : DispatchProxy
    {
        private readonly ConcurrentDictionary<string, int> _calls = new(StringComparer.Ordinal);

        public IAriadneBackendClient Client { get; private set; } = null!;
        public bool BlockWorkflowList { get; set; }
        public TaskCompletionSource<bool> WorkflowListStarted { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private TaskCompletionSource<bool> WorkflowListRelease { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

        public static CountingBackend Create()
        {
            var client = Create<IAriadneBackendClient, CountingBackend>();
            var backend = (CountingBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        public int Count(string method) => _calls.TryGetValue(method, out var count) ? count : 0;

        public void ReleaseWorkflowList() => WorkflowListRelease.TrySetResult(true);

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

            _calls.AddOrUpdate(targetMethod.Name, 1, (_, count) => count + 1);
            if (targetMethod.Name == nameof(IAriadneBackendClient.ListWorkflowGraphsAsync)
                && BlockWorkflowList)
            {
                WorkflowListStarted.TrySetResult(true);
                return WaitForWorkflowListAsync();
            }
            object? value = targetMethod.Name switch
            {
                nameof(IAriadneBackendClient.ListWorkflowGraphsAsync) => Array.Empty<WorkflowSummary>(),
                nameof(IAriadneBackendClient.LoadWorkflowGraphAsync) => EmptyWorkflow(),
                nameof(IAriadneBackendClient.ListConfirmationsAsync) => Array.Empty<ConfirmationLogEntry>(),
                nameof(IAriadneBackendClient.GetProviderConfigAsync) => EmptyProviderConfig(),
                nameof(IAriadneBackendClient.GetWorksTreeAsync) => EmptyWorksTree(),
                nameof(IAriadneBackendClient.GetGitRepositoryStatusAsync) => EmptyGitStatus(),
                nameof(IAriadneBackendClient.GetGitBranchGraphAsync) => Array.Empty<BranchGraphNode>(),
                nameof(IAriadneBackendClient.GetSidebarBadgesAsync) => new SidebarBadgeCounts(0, 0, 0),
                _ => null,
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
                    .Invoke(null, new object?[] { value });
            }

            return value;
        }

        private async Task<IReadOnlyList<WorkflowSummary>> WaitForWorkflowListAsync()
        {
            await WorkflowListRelease.Task.ConfigureAwait(false);
            return Array.Empty<WorkflowSummary>();
        }

        private static WorkflowGraphData EmptyWorkflow() => new(
            "default",
            "Default",
            Array.Empty<CanvasNode>(),
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>());

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

        private static GitRepositoryStatus EmptyGitStatus() => new(
            "clean",
            null,
            null,
            false,
            null,
            0,
            string.Empty);
    }
}
