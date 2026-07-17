using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class WorksNavigationTreeTests
{
    [Fact]
    public async Task BackendHierarchy_IsPreservedWithTypedExpandableNodes()
    {
        var backend = NavigationBackend.Create();
        var vm = NewViewModel(backend);

        await vm.ReloadProjectDataAsync();

        var root = Assert.Single(vm.WorksTreeRoots);
        Assert.True(root.IsGlobalOutline);
        Assert.True(root.IsExpanded);
        Assert.Equal(2, root.Children.Count);
        Assert.All(root.Children, stage => Assert.True(stage.IsStageOutline));
        Assert.All(root.Children, stage => Assert.True(stage.IsExpanded));
        Assert.Equal(new[] { "第一章", "第二章", "第三章" },
            root.Children.SelectMany(stage => stage.Children).Select(chapter => chapter.Title));
        Assert.All(root.Children.SelectMany(stage => stage.Children), chapter => Assert.True(chapter.IsChapter));
    }

    [Fact]
    public async Task TitleSearch_KeepsAncestorsAndRestoresExpansionState()
    {
        var backend = NavigationBackend.Create();
        var vm = NewViewModel(backend);
        await vm.ReloadProjectDataAsync();
        var root = Assert.Single(vm.WorksTreeRoots);
        var firstStage = root.Children[0];
        firstStage.IsExpanded = false;

        vm.WorksTreeSearchText = "第二章";

        var visibleRoot = Assert.Single(vm.VisibleWorksTreeRoots);
        var visibleStage = Assert.Single(visibleRoot.VisibleChildren);
        Assert.Same(firstStage, visibleStage);
        Assert.Equal("第二章", Assert.Single(visibleStage.VisibleChildren).Title);
        Assert.True(root.IsExpanded);
        Assert.True(firstStage.IsExpanded);

        vm.WorksTreeSearchText = string.Empty;

        Assert.False(firstStage.IsExpanded);
        Assert.Equal(2, root.VisibleChildren.Count);
        Assert.Equal(2, firstStage.VisibleChildren.Count);
    }

    [Fact]
    public async Task Reload_PreservesExpansionSelectionAndCurrentDocumentByStableIdentity()
    {
        var backend = NavigationBackend.Create();
        var vm = NewViewModel(backend);
        await vm.ReloadProjectDataAsync();
        var firstStage = vm.WorksTreeRoots[0].Children[0];
        firstStage.IsExpanded = false;
        var chapter = firstStage.Children[1];

        vm.SelectedWorksTreeNode = chapter;
        await WaitUntilAsync(() => vm.HasCurrentDocument);
        Assert.True(chapter.IsCurrentDocument);

        await vm.ReloadProjectDataAsync();

        var restoredStage = vm.WorksTreeRoots[0].Children[0];
        var restoredChapter = restoredStage.Children[1];
        Assert.False(restoredStage.IsExpanded);
        Assert.Same(restoredChapter, vm.SelectedWorksTreeNode);
        Assert.True(restoredChapter.IsCurrentDocument);
        Assert.Equal("documents/chapter-2.md", restoredChapter.DisplayPath);
    }

    [Fact]
    public void View_UsesTrueTreeSelectionSearchAndExpansionBindings()
    {
        var root = ResolveRepoRoot();
        var viewPath = Path.Combine(root, "desktop", "Ariadne.Desktop", "Views", "WorksPageView.axaml");
        var viewModelPath = Path.Combine(root, "desktop", "Ariadne.Desktop", "ViewModels", "WorksPageViewModel.cs");
        var view = File.ReadAllText(viewPath);
        var viewModel = File.ReadAllText(viewModelPath);

        Assert.Contains("<TreeView", view, StringComparison.Ordinal);
        Assert.Contains("<TreeDataTemplate", view, StringComparison.Ordinal);
        Assert.Contains("VisibleWorksTreeRoots", view, StringComparison.Ordinal);
        Assert.Contains("SelectedWorksTreeNode", view, StringComparison.Ordinal);
        Assert.Contains("IsExpanded, Mode=TwoWay", view, StringComparison.Ordinal);
        Assert.Contains("WorksTreeSearchText", view, StringComparison.Ordinal);
        Assert.DoesNotContain("ItemsSource=\"{Binding WorksTreeNodes}\"", view, StringComparison.Ordinal);
        Assert.DoesNotContain("FlattenTree", viewModel, StringComparison.Ordinal);
    }

    private static WorksPageViewModel NewViewModel(NavigationBackend backend) =>
        new(DisplayNameService.LoadDefault(), backend.Client);

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

        Assert.True(predicate(), "Timed out waiting for navigation state to settle.");
    }

    private static string ResolveRepoRoot()
    {
        var path = Path.GetDirectoryName(typeof(WorksNavigationTreeTests).Assembly.Location)!;
        while (!string.IsNullOrEmpty(path) && !File.Exists(Path.Combine(path, "desktop", "Ariadne.slnx")))
        {
            path = Directory.GetParent(path)?.FullName ?? string.Empty;
        }
        return path;
    }

    private static WorksTreeNode Tree() => new(
        "outline:global",
        "global_outline",
        "全局总纲",
        "planning/global-outline.md",
        new[]
        {
            new WorksTreeNode(
                "stage:first",
                "stage_outline",
                "第一阶段",
                "planning/stages/first.md",
                new[]
                {
                    new WorksTreeNode("chapter:1", "chapter", "第一章", "documents/chapter-1.md", Array.Empty<WorksTreeNode>()),
                    new WorksTreeNode("chapter:2", "chapter", "第二章", "documents/chapter-2.md", Array.Empty<WorksTreeNode>()),
                },
                StageId: "first"),
            new WorksTreeNode(
                "stage:second",
                "stage_outline",
                "第二阶段",
                "planning/stages/second.md",
                new[]
                {
                    new WorksTreeNode("chapter:3", "chapter", "第三章", "documents/chapter-3.md", Array.Empty<WorksTreeNode>()),
                },
                StageId: "second"),
        });

    private class NavigationBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        public static NavigationBackend Create()
        {
            var client = Create<IAriadneBackendClient, NavigationBackend>();
            var backend = (NavigationBackend)(object)client;
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
            if (targetMethod.Name == nameof(IAriadneBackendClient.GetWorksTreeAsync))
            {
                return Task.FromResult(Tree());
            }
            if (targetMethod.Name is nameof(IAriadneBackendClient.GetDocumentContentDetailsByPathAsync)
                or nameof(IAriadneBackendClient.GetDocumentContentDetailsAsync))
            {
                var path = (string)(args ?? Array.Empty<object?>())[0]!;
                return Task.FromResult(new DocumentContentResult(
                    new DocumentMetadata(path, path, "markdown", "text/markdown", 4, "v1"),
                    "正文"));
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
