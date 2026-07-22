using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// Product path: single project canvas (LoadProjectCanvasAsync), no multi-workflow selector.
/// </summary>
public sealed class WorkspaceWorkflowNavigationTests
{
    [Fact]
    public async Task PreparedCanvasSave_KeepsEditMadeDuringCommitDirty()
    {
        var backend = CanvasBackend.Create();
        var saveStarted = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var saveRelease = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        backend.SaveHandler = async graph =>
        {
            backend.SavedGraph = graph;
            saveStarted.TrySetResult(true);
            await saveRelease.Task;
            return graph with { ContentRevision = "saved-revision" };
        };
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();
        vm.AddNodeAt("llm", 100, 100);

        Assert.True(await vm.PrepareUnsavedChangesAsync());
        var commit = vm.CommitPreparedUnsavedChangesAsync();
        await saveStarted.Task;
        vm.AddNodeAt("llm", 300, 100);
        saveRelease.TrySetResult(true);

        Assert.False(await commit);
        Assert.Single(backend.SavedGraph!.Nodes);
        Assert.Equal(2, vm.Nodes.Count);
        Assert.True(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task PreparedCanvasSave_RejectsEditBeforeCommitWithoutWriting()
    {
        var backend = CanvasBackend.Create();
        var saveCalls = 0;
        backend.SaveHandler = graph =>
        {
            saveCalls++;
            return Task.FromResult(graph);
        };
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();
        vm.AddNodeAt("llm", 100, 100);

        Assert.True(await vm.PrepareUnsavedChangesAsync());
        vm.AddNodeAt("llm", 300, 100);

        Assert.False(await vm.CommitPreparedUnsavedChangesAsync());
        Assert.Equal(0, saveCalls);
        Assert.True(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task Reload_LoadsSingleProjectCanvas_AndKeepsForeignConfirmationVisible()
    {
        var backend = CanvasBackend.Create();
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await vm.ReloadProjectDataAsync();

        Assert.Equal("default", vm.LoadedWorkflowIdForTests);
        Assert.Equal(1, backend.ProjectCanvasLoadCount);
        Assert.NotNull(vm.SelectedConfirmation);
        Assert.Equal("review", vm.SelectedConfirmation!.WorkflowId);
        Assert.Contains("review", vm.SelectedConfirmation.SourceText, StringComparison.Ordinal);
        // Single project canvas: confirmation from another workflow id does not require a selector switch.
        Assert.Empty(vm.CurrentRunId);
    }

    [Fact]
    public async Task ReloadProjectCanvas_UsesBackendProjectCanvas_NotListWorkflowGraphs()
    {
        var backend = CanvasBackend.Create();
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await vm.ReloadProjectDataAsync();
        await vm.ReloadProjectDataAsync();

        Assert.Equal(2, backend.ProjectCanvasLoadCount);
        Assert.Equal(0, backend.ListWorkflowGraphsCount);
        Assert.Equal("default", vm.LoadedWorkflowIdForTests);
    }

    [Fact]
    public void WorkspaceView_UsesSingleCanvasWithoutWorkflowSelector()
    {
        var xaml = File.ReadAllText(ResolveDesktopSource("Views", "WorkspacePageView.axaml"));
        var vm = new WorkspacePageViewModel(
            DisplayNameService.LoadDefault(),
            CanvasBackend.Create().Client);

        Assert.DoesNotContain("ItemsSource=\"{Binding WorkflowSummaries}\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("SelectedValue=\"{Binding SelectedWorkflowId, Mode=TwoWay}\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("x:Name=\"WorkflowSelectorHost\"", xaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"CanvasToolbarActions\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding SelectedConfirmation.SourceText}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding ReloadProjectCanvasCommand}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Ariadne.Icon.Refresh", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("{Binding ImportText}", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("Ariadne.Icon.Import", xaml, StringComparison.Ordinal);
        Assert.False(string.IsNullOrWhiteSpace(vm.ReloadProjectCanvasText));
        Assert.Equal("default", vm.LoadedWorkflowIdForTests);
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

    private class CanvasBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public int ProjectCanvasLoadCount { get; private set; }
        public int ListWorkflowGraphsCount { get; private set; }
        public WorkflowGraphData? SavedGraph { get; set; }
        public Func<WorkflowGraphData, Task<WorkflowGraphData>> SaveHandler { get; set; } =
            graph => Task.FromResult(graph);

        public static CanvasBackend Create()
        {
            var client = Create<IAriadneBackendClient, CanvasBackend>();
            var backend = (CanvasBackend)(object)client;
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

            if (targetMethod.Name == nameof(IAriadneBackendClient.SaveProjectCanvasAsync))
            {
                return SaveHandler((WorkflowGraphData)args![0]!);
            }

            object? value = targetMethod.Name switch
            {
                nameof(IAriadneBackendClient.LoadProjectCanvasAsync) => LoadCanvas(),
                nameof(IAriadneBackendClient.ListWorkflowGraphsAsync) => CountList(),
                nameof(IAriadneBackendClient.SaveWorkflowGraphAsync) => args![0]!,
                nameof(IAriadneBackendClient.ValidateWorkflowGraphAsync) => null,
                nameof(IAriadneBackendClient.ListConfirmationsAsync) => new[] { ForeignConfirmation() },
                nameof(IAriadneBackendClient.GetProviderConfigAsync) => EmptyProviderConfig(),
                nameof(IAriadneBackendClient.GetWorksTreeAsync) => EmptyWorksTree(),
                _ => UnsupportedDefault(targetMethod),
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

        private WorkflowGraphData LoadCanvas()
        {
            ProjectCanvasLoadCount++;
            return new WorkflowGraphData(
                "default",
                "Project Canvas",
                Array.Empty<CanvasNode>(),
                Array.Empty<CanvasEdge>(),
                new Dictionary<string, object?>(),
                ContentRevision: "canvas-revision");
        }

        private IReadOnlyList<WorkflowSummary> CountList()
        {
            ListWorkflowGraphsCount++;
            return Array.Empty<WorkflowSummary>();
        }

        private static object? UnsupportedDefault(MethodInfo method)
        {
            if (method.ReturnType == typeof(Task) || method.ReturnType.IsGenericType)
            {
                // Prefer explicit fail for unexpected product IPC so tests catch API drift.
                throw new NotSupportedException(method.Name);
            }

            return method.ReturnType.IsValueType ? Activator.CreateInstance(method.ReturnType) : null;
        }

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
