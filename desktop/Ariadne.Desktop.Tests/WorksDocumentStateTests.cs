using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class WorksDocumentStateTests
{
    [Fact]
    public async Task SaveCompletionAfterLocalEdit_KeepsTheNewEditDirty()
    {
        var backend = WorksBackend.Create();
        var saveStarted = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var saveRelease = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        string? savedContent = null;
        backend.TreeHandler = _ => Task.FromResult(SingleDocumentTree("a.md", "A"));
        backend.DocumentHandler = (_, _) => Task.FromResult(Document("a.md", "A", "v1"));
        backend.SaveHandler = async (_, content, _, _) =>
        {
            savedContent = content;
            saveStarted.TrySetResult(true);
            await saveRelease.Task;
            return WriteReport("a.md", "v2");
        };

        var vm = NewViewModel(backend);
        await vm.ReloadProjectDataAsync();
        TreeNodes(vm).First(item => item.HasPath).OpenCommand.Execute(null);
        await WaitUntilAsync(() => vm.HasCurrentDocument);

        vm.DocumentContent = "A*";
        vm.SaveCommand.Execute(null);
        await saveStarted.Task;
        Assert.False(vm.SaveCommand.TryExecute(), "a second keyboard/programmatic save must not overlap the active save");
        vm.DocumentContent = "A*+";
        saveRelease.TrySetResult(true);
        await WaitUntilAsync(() => !vm.IsDocumentSaving);

        Assert.Equal("A*", savedContent);
        Assert.Equal("A*+", vm.DocumentContent);
        Assert.True(vm.HasUnsavedChanges);
        Assert.Contains("未保存", vm.DocumentInfoText, StringComparison.Ordinal);
    }

    [Fact]
    public async Task WorksTreeFailure_RetainsPreviousNodesAndUsesErrorState()
    {
        var backend = WorksBackend.Create();
        var loadCount = 0;
        backend.TreeHandler = _ =>
        {
            loadCount++;
            return loadCount switch
            {
                1 or 3 => Task.FromResult(SingleDocumentTree("a.md", "A")),
                _ => Task.FromException<WorksTreeNode>(new InvalidOperationException("tree unavailable")),
            };
        };

        var vm = NewViewModel(backend);
        await vm.ReloadProjectDataAsync();
        Assert.True(vm.WorksTreeRoots.Count > 0);

        await vm.ReloadProjectDataAsync();

        Assert.True(vm.IsWorksTreeError);
        Assert.False(vm.IsWorksTreeEmpty);
        Assert.Equal(2, TreeNodes(vm).Count());
        Assert.NotEmpty(vm.WorksTreeErrorText);

        vm.RetryWorksTreeCommand.Execute(null);
        await WaitUntilAsync(() => !vm.IsWorksTreeLoading);
        Assert.False(vm.IsWorksTreeError);
        Assert.False(vm.IsWorksTreeEmpty);
    }

    [Fact]
    public async Task OlderDocumentLoadCannotOverwriteTheNewerSelection()
    {
        var backend = WorksBackend.Create();
        // TextDocument 是 UI 线程对象；用当前测试线程模拟 UI 消息循环，
        // 由释放动作内联恢复两个异步加载，既保留竞态顺序又不伪造跨线程 UI 写入。
        var aStarted = new TaskCompletionSource<bool>();
        var aRelease = new TaskCompletionSource<bool>();
        var bStarted = new TaskCompletionSource<bool>();
        var bRelease = new TaskCompletionSource<bool>();
        backend.TreeHandler = _ => Task.FromResult(TwoDocumentTree());
        backend.DocumentHandler = async (path, _) =>
        {
            if (path.EndsWith("a.md", StringComparison.Ordinal))
            {
                aStarted.TrySetResult(true);
                await aRelease.Task;
                return Document("a.md", "A", "v1");
            }

            bStarted.TrySetResult(true);
            await bRelease.Task;
            return Document("b.md", "B", "v1");
        };

        var vm = NewViewModel(backend);
        await vm.ReloadProjectDataAsync();
        TreeNodes(vm).First(item => item.Path.EndsWith("a.md", StringComparison.Ordinal)).OpenCommand.Execute(null);
        Assert.True(aStarted.Task.IsCompleted);
        TreeNodes(vm).First(item => item.Path.EndsWith("b.md", StringComparison.Ordinal)).OpenCommand.Execute(null);
        Assert.True(bStarted.Task.IsCompleted);
        bRelease.TrySetResult(true);
        Assert.Equal("B", vm.DocumentContent);
        aRelease.TrySetResult(true);

        Assert.Equal("B", vm.DocumentContent);
        Assert.Contains("documents/b.md", vm.DocumentInfoText, StringComparison.Ordinal);
    }

    private static WorksPageViewModel NewViewModel(WorksBackend backend) =>
        new(DisplayNameService.LoadDefault(), backend.Client);

    private static IEnumerable<WorksTreeItemViewModel> TreeNodes(WorksPageViewModel vm) =>
        vm.WorksTreeRoots.SelectMany(root => root.EnumerateSubtree());

    private static async Task WaitUntilAsync(Func<bool> predicate)
    {
        for (var attempt = 0; attempt < 200; attempt++)
        {
            if (predicate())
            {
                return;
            }
            await Task.Delay(10);
        }

        Assert.True(predicate(), "Timed out waiting for the ViewModel state to settle.");
    }

    private static WorksTreeNode SingleDocumentTree(string fileName, string title) => new(
        "root",
        "root",
        "Root",
        string.Empty,
        new[] { new WorksTreeNode(fileName, "document", title, $"documents/{fileName}", Array.Empty<WorksTreeNode>()) });

    private static WorksTreeNode TwoDocumentTree() => new(
        "root",
        "root",
        "Root",
        string.Empty,
        new[]
        {
            new WorksTreeNode("a", "document", "A", "documents/a.md", Array.Empty<WorksTreeNode>()),
            new WorksTreeNode("b", "document", "B", "documents/b.md", Array.Empty<WorksTreeNode>()),
        });

    private static DocumentContentResult Document(string path, string content, string version) => new(
        new DocumentMetadata(path, $"documents/{path}", "markdown", "text/markdown", content.Length, version),
        content);

    private static DocumentWriteReport WriteReport(string path, string version) => new(
        new DocumentMetadata(path, $"documents/{path}", "markdown", "text/markdown", 1, version),
        null);

    private class WorksBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public Func<CancellationToken, Task<WorksTreeNode>> TreeHandler { get; set; } =
            _ => Task.FromResult(SingleDocumentTree("default.md", "Default"));
        public Func<string, CancellationToken, Task<DocumentContentResult>> DocumentHandler { get; set; } =
            (_, _) => Task.FromResult(Document("default.md", string.Empty, "v1"));
        public Func<string, string, string?, CancellationToken, Task<DocumentWriteReport>> SaveHandler { get; set; } =
            (_, _, _, _) => Task.FromResult(WriteReport("default.md", "v2"));

        public static WorksBackend Create()
        {
            var client = Create<IAriadneBackendClient, WorksBackend>();
            var backend = (WorksBackend)(object)client;
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

            var parameters = args ?? Array.Empty<object?>();
            object? result = targetMethod.Name switch
            {
                nameof(IAriadneBackendClient.GetWorksTreeAsync) => TreeHandler((CancellationToken)parameters[0]!),
                nameof(IAriadneBackendClient.GetDocumentContentDetailsByPathAsync) =>
                    DocumentHandler((string)parameters[0]!, (CancellationToken)parameters[1]!),
                nameof(IAriadneBackendClient.SaveDocumentContentAsync) => SaveHandler(
                    (string)parameters[0]!,
                    (string)parameters[1]!,
                    (string?)parameters[2],
                    (CancellationToken)parameters[3]!),
                _ => null,
            };

            if (result is not null)
            {
                return result;
            }

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
                    .Invoke(null, new object?[] { null });
            }

            return null;
        }
    }
}
