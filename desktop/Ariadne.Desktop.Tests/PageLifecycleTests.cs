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
    public async Task StartupProjectBindingFailure_DoesNotCommitProjectPageIdentity()
    {
        var backend = StartupFailureBackend.Create();
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await window.InitializeAsync();

        Assert.False(window.HasOpenProject);
        Assert.Same(window.Welcome, window.CurrentPage);
        Assert.True(window.HasDiagnostic);
        Assert.Equal(1, backend.SetProjectRootCalls);
    }

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

        Assert.Equal(0, backend.Count(nameof(IAriadneBackendClient.ListWorkflowGraphsAsync)));
        Assert.Equal(1, backend.Count(nameof(IAriadneBackendClient.LoadProjectCanvasAsync)));
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
        Assert.Equal(1, backend.Count(nameof(IAriadneBackendClient.LoadProjectCanvasAsync)));
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
        Assert.Equal(0, backend.Count(nameof(IAriadneBackendClient.ListWorkflowGraphsAsync)));
        Assert.Equal(2, backend.Count(nameof(IAriadneBackendClient.LoadProjectCanvasAsync)));
        Assert.Equal(4, backend.Count(nameof(IAriadneBackendClient.GetWorksTreeAsync)));
        Assert.Equal(2, backend.Count(nameof(IAriadneBackendClient.GetGitBranchGraphAsync)));
    }

    [Fact]
    public void GlobalSettingsPage_IsRetainedAcrossProjectSessionReset()
    {
        var backend = CountingBackend.Create();
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);
        var settings = window.GetPageForTests("settings");

        window.ResetProjectPageSessionForTests();

        Assert.Same(settings, window.GetPageForTests("settings"));
    }

    [Fact]
    public async Task ProjectPageSession_OldLoadCompletionCannotMarkNewSessionLoaded()
    {
        var backend = CountingBackend.Create();
        backend.BlockProjectCanvasLoad = true;
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);
        var oldPreload = window.PreloadProjectPagesForTestsAsync();
        await backend.ProjectCanvasLoadStarted.Task;

        window.ResetProjectPageSessionForTests();
        backend.BlockProjectCanvasLoad = false;
        backend.ReleaseProjectCanvasLoad();
        await oldPreload;
        await window.PreloadProjectPagesForTestsAsync();

        Assert.Equal(2, backend.Count(nameof(IAriadneBackendClient.LoadProjectCanvasAsync)));
    }

    [Fact]
    public async Task NavigationSession_SlowOldPageCannotOverwriteFastNewPageOrPersistedSelection()
    {
        var backend = CountingBackend.Create();
        var workspace = ControlledPage.Blocked();
        var works = ControlledPage.Completed();
        string? savedNavigationId = null;
        var window = new MainWindowViewModel(
            DisplayNameService.LoadDefault(),
            backend.Client,
            id => id == "workspace" ? workspace : works,
            id => savedNavigationId = id);

        var slow = window.OpenNavigationItemByIdAsync("workspace");
        await workspace.Started.Task;
        await window.OpenNavigationItemByIdAsync("works");

        Assert.Same(works, window.CurrentPage);
        Assert.Equal("works", window.SelectedNavigationIdForTests);
        Assert.Equal("works", window.LastNavigationIdForTests);
        Assert.Equal("works", savedNavigationId);

        workspace.Release();
        await slow;

        Assert.Same(works, window.CurrentPage);
        Assert.Equal("works", window.SelectedNavigationIdForTests);
        Assert.Equal("works", window.LastNavigationIdForTests);
        Assert.Equal("works", savedNavigationId);
    }

    [Fact]
    public async Task NavigationSession_LateOldFailureCannotClearSuccessfulNewPage()
    {
        var backend = CountingBackend.Create();
        var workspace = ControlledPage.Blocked();
        var works = ControlledPage.Completed();
        var window = new MainWindowViewModel(
            DisplayNameService.LoadDefault(),
            backend.Client,
            id => id == "workspace" ? workspace : works,
            _ => { });

        var slow = window.OpenNavigationItemByIdAsync("workspace");
        await workspace.Started.Task;
        await window.OpenNavigationItemByIdAsync("works");
        workspace.Fail(new InvalidOperationException("stale page failure"));
        await slow;

        Assert.Same(works, window.CurrentPage);
        Assert.Equal("works", window.SelectedNavigationIdForTests);
    }

    [Fact]
    public async Task NavigationSession_CurrentPageFailureReturnsToWelcomeWithLocalizedError()
    {
        var backend = CountingBackend.Create();
        var workspace = ControlledPage.Blocked();
        var names = DisplayNameService.LoadDefault();
        var window = new MainWindowViewModel(
            names,
            backend.Client,
            _ => workspace,
            _ => { });

        var navigation = window.OpenNavigationItemByIdAsync("workspace");
        await workspace.Started.Task;
        workspace.Fail(new IOException("raw page load diagnostic"));
        await navigation;

        Assert.Same(window.Welcome, window.CurrentPage);
        Assert.Equal(names.Text("ui.error.io"), window.NotificationText);
        Assert.DoesNotContain("raw page load diagnostic", window.NotificationText, StringComparison.Ordinal);
        Assert.Null(window.SelectedNavigationIdForTests);
    }

    [Fact]
    public async Task NavigationSession_ProjectResetInvalidatesPendingNavigationCommit()
    {
        var backend = CountingBackend.Create();
        var workspace = ControlledPage.Blocked();
        var works = ControlledPage.Completed();
        var window = new MainWindowViewModel(
            DisplayNameService.LoadDefault(),
            backend.Client,
            id => id == "workspace" ? workspace : works,
            _ => { });

        var oldNavigation = window.OpenNavigationItemByIdAsync("workspace");
        await workspace.Started.Task;
        window.ResetProjectPageSessionForTests();
        await window.OpenNavigationItemByIdAsync("works");
        workspace.Release();
        await oldNavigation;

        Assert.Same(works, window.CurrentPage);
        Assert.Equal("works", window.SelectedNavigationIdForTests);
    }

    private sealed class ControlledPage : IProjectDataReloadable
    {
        private readonly TaskCompletionSource<bool> _completion =
            new(TaskCreationOptions.RunContinuationsAsynchronously);

        private ControlledPage(bool completed)
        {
            if (completed)
            {
                _completion.TrySetResult(true);
            }
        }

        public TaskCompletionSource<bool> Started { get; } =
            new(TaskCreationOptions.RunContinuationsAsynchronously);

        public static ControlledPage Blocked() => new(false);

        public static ControlledPage Completed() => new(true);

        public async Task ReloadProjectDataAsync(CancellationToken cancellationToken = default)
        {
            Started.TrySetResult(true);
            // 故意忽略取消，证明旧 I/O 即使晚完成也不能提交导航状态。
            await _completion.Task.ConfigureAwait(false);
        }

        public void DeactivateProjectData()
        {
        }

        public void Release() => _completion.TrySetResult(true);

        public void Fail(Exception error) => _completion.TrySetException(error);
    }

    private class StartupFailureBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public int SetProjectRootCalls { get; private set; }

        public static StartupFailureBackend Create()
        {
            var client = Create<IAriadneBackendClient, StartupFailureBackend>();
            var backend = (StartupFailureBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }
            return targetMethod.Name switch
            {
                "get_HasProjectRoot" => false,
                nameof(IAriadneBackendClient.GetAppStatusAsync) => Task.FromResult(new AppStatus(
                    new CurrentProjectStatus("/project-b", "Project B"),
                    new SidebarBadgeCounts(0, 0, 0),
                    DefaultPreferences())),
                nameof(IAriadneBackendClient.ListRecentProjectsAsync) =>
                    Task.FromResult<IReadOnlyList<RecentProjectEntry>>(Array.Empty<RecentProjectEntry>()),
                nameof(IAriadneBackendClient.SetProjectRootAsync) => FailProjectBinding(),
                _ => UnsupportedTask(targetMethod),
            };
        }

        private Task FailProjectBinding()
        {
            SetProjectRootCalls++;
            return Task.FromException(BackendException.FromIpcPayload(
                "conflict",
                "project activation rolled back"));
        }

        private static UiPreferences DefaultPreferences() => new(
            "system",
            "#8a8f98",
            "#f59e0b",
            true,
            null,
            new Dictionary<string, bool>(),
            false,
            Locale: "zh");

        private static object? UnsupportedTask(MethodInfo method)
        {
            if (method.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (method.ReturnType.IsGenericType
                && method.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = method.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object?[] { resultType.IsValueType ? Activator.CreateInstance(resultType) : null });
            }
            return method.ReturnType.IsValueType ? Activator.CreateInstance(method.ReturnType) : null;
        }
    }

    private class CountingBackend : DispatchProxy
    {
        private readonly ConcurrentDictionary<string, int> _calls = new(StringComparer.Ordinal);

        public IAriadneBackendClient Client { get; private set; } = null!;
        public bool BlockProjectCanvasLoad { get; set; }
        public TaskCompletionSource<bool> ProjectCanvasLoadStarted { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private TaskCompletionSource<bool> ProjectCanvasLoadRelease { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);

        public static CountingBackend Create()
        {
            var client = Create<IAriadneBackendClient, CountingBackend>();
            var backend = (CountingBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        public int Count(string method) => _calls.TryGetValue(method, out var count) ? count : 0;

        public void ReleaseProjectCanvasLoad() => ProjectCanvasLoadRelease.TrySetResult(true);

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
            if (targetMethod.Name == nameof(IAriadneBackendClient.LoadProjectCanvasAsync)
                && BlockProjectCanvasLoad)
            {
                ProjectCanvasLoadStarted.TrySetResult(true);
                return WaitForProjectCanvasLoadAsync();
            }
            object? value = targetMethod.Name switch
            {
                nameof(IAriadneBackendClient.ListWorkflowGraphsAsync) => Array.Empty<WorkflowSummary>(),
                nameof(IAriadneBackendClient.LoadProjectCanvasAsync) => EmptyWorkflow(),
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

        private async Task<WorkflowGraphData> WaitForProjectCanvasLoadAsync()
        {
            await ProjectCanvasLoadRelease.Task.ConfigureAwait(false);
            return EmptyWorkflow();
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
