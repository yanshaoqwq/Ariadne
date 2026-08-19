using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U184-A：作品页搜索必须能搜到**正文**。
///
/// <para>缺陷形态：全文检索后端（<c>commands.rs::search_project_documents_impl</c>，走
/// Tantivy）与 IPC 分发（<c>ipc.rs</c>）早就就绪，唯独前端零调用——搜索框
/// 只比 <c>Title.Contains</c>。百万字项目靠标题找内容是不可能的，作者会得出
/// 「这软件搜不了正文」的结论然后再也不用这个框。</para>
///
/// <para>⚠️ 判据取「搜一个**只存在于正文、不存在于任何标题**的短语，能命中那一章」。
/// **不能取「搜索命令被调用了」**——标题搜索也会调用，那个判据在缺陷版本里恒真。</para>
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class WorksBodySearchTests
{
    /// 只出现在正文、不出现在任何标题里的探针短语。
    private const string BodyOnlyPhrase = "她把伞留在了车站";

    [Fact]
    public async Task BodyOnlyPhrase_SurfacesTheChapterThatContainsIt()
    {
        var backend = BodySearchBackend.Create();
        // 后端命中的 document_id 是索引时 canonicalize 的**绝对路径**
        // （retrieval/runtime.rs），与作品树里的 path 形态不同——这条用例
        // 顺带钉住那层归一化，否则命中会全部映射不到章节、点不开。
        backend.SearchHandler = (query, _) => Task.FromResult<IReadOnlyList<RetrievalHit>>(
            query.Contains(BodyOnlyPhrase, StringComparison.Ordinal)
                ? new[] { Hit("/home/author/novel/documents/chapter-2.md", $"下车之后，{BodyOnlyPhrase}，再没回去取。") }
                : Array.Empty<RetrievalHit>());
        var vm = await LoadAsync(backend);

        vm.WorksTreeSearchText = BodyOnlyPhrase;
        await WaitUntilAsync(() => !vm.IsBodySearching);

        // 标题那路必然 0 命中：探针短语不在任何标题里（缺陷版本到此为止）。
        Assert.Empty(vm.VisibleWorksTreeRoots);

        var hit = Assert.Single(vm.BodySearchHits);
        Assert.Equal("第二章", hit.Title);
        Assert.Contains(BodyOnlyPhrase, hit.Snippet, StringComparison.Ordinal);
        Assert.True(vm.HasBodySearchHits);
        Assert.True(vm.ShowBodySearchGroup);
        // 有正文命中时不许再说「没有匹配的作品条目」——那句话此时是错的，
        // 还会把作者的注意力从真实存在的命中上引开。
        Assert.False(vm.ShowWorksTreeSearchEmpty);
        Assert.False(vm.ShowBodySearchEmpty);

        // 命中必须真的连到那一章，否则片段只是一句「某处有」。
        //
        // 判据取「点击后选中的是第二章」而不是「正文加载完成」：
        // `OpenCommand.CanExecute` 为真已经证明绝对路径 → 树节点那层归一化成立
        // （映射失败时 open 委托为 null ⇒ CanExecute 为假），而选中项证明点击
        // 路由到了**正确**的那一章。
        //
        // ⚠️ 刻意不断言 `HasCurrentDocument`：正文落地要写 `TextDocument`，
        // 那是**有线程亲和的 UI 对象**，而本用例在 `await Task.Delay` 之后已经
        // 换到线程池线程上 ⇒ 必然抛 "Call from invalid thread."。
        // 那是测试基建约束、不是产品缺陷；正文加载本身另有覆盖
        // （WorksDocumentStateTests / WorksNavigationTreeTests，它们在 await 之前点开）。
        Assert.True(hit.OpenCommand.CanExecute(null), "正文命中必须能连回作品树里的那一章");
        hit.OpenCommand.Execute(null);
        Assert.Equal("第二章", vm.SelectedWorksTreeNode?.Title);
    }

    [Fact]
    public async Task TitleAndBodyMatches_StayInSeparateGroups()
    {
        var backend = BodySearchBackend.Create();
        // 同一个词「第二章」既在标题里、又（假设）在第三章正文里被提到。
        backend.SearchHandler = (_, _) => Task.FromResult<IReadOnlyList<RetrievalHit>>(
            new[] { Hit("/home/author/novel/documents/chapter-3.md", "承接第二章的伏笔。") });
        var vm = await LoadAsync(backend);

        vm.WorksTreeSearchText = "第二章";
        await WaitUntilAsync(() => !vm.IsBodySearching);

        // 标题组：树里只剩第二章。
        var titleHit = Assert.Single(
            vm.VisibleWorksTreeRoots.SelectMany(root => root.VisibleChildren).SelectMany(stage => stage.VisibleChildren));
        Assert.Equal("第二章", titleHit.Title);
        // 正文组：第三章。两组各自独立，正文命中**没有**混进树里——
        // 混排后作者无法判断某章为什么出现在结果里。
        // ⚠️ 这里必须走 VisibleChildren（过滤后的可见树）而不是 EnumerateSubtree()
        // （后者走 Children，即完整树，永远包含第三章 ⇒ 断言恒假、量不到东西）。
        Assert.Equal("第三章", Assert.Single(vm.BodySearchHits).Title);
        Assert.DoesNotContain("第三章", VisibleTitles(vm));
        // 两组都必须自带标题，否则分组等于没做。
        Assert.True(vm.ShowWorksTreeTitleGroup);
        Assert.True(vm.ShowBodySearchGroup);
        Assert.NotEqual(vm.WorksTreeTitleGroupText, vm.BodySearchGroupText);
    }

    [Fact]
    public async Task IndexingNotReady_IsShownAsRetryableWaitNotAnError()
    {
        var backend = BodySearchBackend.Create();
        var attempts = 0;
        backend.SearchHandler = (_, _) =>
        {
            attempts++;
            // 第一次撞上索引门禁（作者刚保存完就搜），第二次索引追上了。
            return attempts == 1
                ? Task.FromException<IReadOnlyList<RetrievalHit>>(IndexingNotReady())
                : Task.FromResult<IReadOnlyList<RetrievalHit>>(
                    new[] { Hit("/home/author/novel/documents/chapter-2.md", $"{BodyOnlyPhrase}。") });
        };
        var vm = await LoadAsync(backend);

        vm.WorksTreeSearchText = BodyOnlyPhrase;
        await WaitUntilAsync(() => vm.ShowBodySearchIndexing || vm.ShowBodySearchError);

        // 关键：这是**等待态**，不是错误态。渲染成红色报错会让作者判定
        // 「搜索功能坏了」，与缺陷版本的「永远 0 结果」一样糟。
        Assert.True(vm.ShowBodySearchIndexing);
        Assert.False(vm.ShowBodySearchError);
        // 也不能说「正文里没有匹配」——那是个尚未成立的结论。
        Assert.False(vm.ShowBodySearchEmpty);
        Assert.False(vm.ShowWorksTreeSearchEmpty);
        Assert.NotEmpty(vm.BodySearchErrorText);
        Assert.DoesNotContain("[ui.", vm.BodySearchErrorText, StringComparison.Ordinal);

        // 必须给一个「现在再试」的动作，否则只能重打一遍关键词。
        Assert.True(vm.RetryBodySearchCommand.CanExecute(null));
        vm.RetryBodySearchCommand.Execute(null);
        await WaitUntilAsync(() => vm.HasBodySearchHits);
        Assert.False(vm.ShowBodySearchIndexing);
        Assert.Equal("第二章", Assert.Single(vm.BodySearchHits).Title);
    }

    [Fact]
    public async Task ClearingTheQuery_DropsBodyHitsAndLeavesNoStaleState()
    {
        var backend = BodySearchBackend.Create();
        backend.SearchHandler = (_, _) => Task.FromResult<IReadOnlyList<RetrievalHit>>(
            new[] { Hit("/home/author/novel/documents/chapter-2.md", $"{BodyOnlyPhrase}。") });
        var vm = await LoadAsync(backend);
        vm.WorksTreeSearchText = BodyOnlyPhrase;
        await WaitUntilAsync(() => vm.HasBodySearchHits);

        vm.WorksTreeSearchText = string.Empty;

        // 清空即回到完整目录：残留的正文命中会让作者以为还在过滤态。
        Assert.Empty(vm.BodySearchHits);
        Assert.False(vm.ShowBodySearchGroup);
        Assert.False(vm.ShowWorksTreeTitleGroup);
        Assert.False(vm.IsBodySearching);
        Assert.NotEmpty(vm.VisibleWorksTreeRoots);
    }

    /// <summary>
    /// 正文命中组必须**真的画出来**，且片段与章节名都在视觉树里。
    ///
    /// <para>为什么单独一条：前四条都在量 ViewModel。Avalonia 的
    /// <c>Classes</c>/资源键**缺失时静默失效**——绑错属性名、少一个
    /// <c>IsVisible</c> 取反、DataTemplate 的 <c>DataType</c> 写错，
    /// 全都不报错，只是「什么都不显示」。ViewModel 全绿 + 界面空白
    /// 恰好是 U184-A 这一族缺陷的形态（后端就绪 + 前端零调用）。</para>
    ///
    /// <para>判据取「视觉树里能找到含片段文字的 TextBlock」，
    /// 而不是「XAML 源码里含某个字符串」——后者只证明有人打了那些字符。</para>
    /// </summary>
    [Fact]
    public async Task BodySearchGroup_ActuallyRendersTitleAndSnippet()
    {
        const string snippet = "下车之后，她把伞留在了车站，再没回去取。";
        await RunHeadlessAsync(async () =>
        {
            var backend = BodySearchBackend.Create();
            backend.SearchHandler = (_, _) => Task.FromResult<IReadOnlyList<RetrievalHit>>(
                new[] { Hit("/home/author/novel/documents/chapter-2.md", snippet) });
            var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
            await vm.ReloadProjectDataAsync();
            SetWorksTreeStateToContent(vm);

            var view = new WorksPageView { DataContext = vm };
            var window = new Window { Width = 1400, Height = 900, Content = view };
            window.Show();
            await DrainAsync();

            vm.WorksTreeSearchText = BodyOnlyPhrase;
            await WaitUntilAsync(() => vm.HasBodySearchHits);
            window.UpdateLayout();
            await DrainAsync();

            var texts = view.GetVisualDescendants()
                .OfType<TextBlock>()
                .Select(block => block.Text ?? string.Empty)
                .ToList();

            // 前置：视觉树真的被遍历到了，否则下面三条在比空集合。
            Assert.NotEmpty(texts);
            Assert.Contains(vm.BodySearchGroupText, texts);
            Assert.Contains("第二章", texts);
            Assert.Contains(texts, text => text.Contains(BodyOnlyPhrase, StringComparison.Ordinal));

            window.Content = null;
            window.Close();
            await DrainAsync();
            return true;
        });
    }

    /// <summary>
    /// 后端把索引门禁标成 <c>ui.error.indexing_not_ready</c>
    /// （<c>commands.rs::tag_indexing_not_ready</c>）。测试里如实复现那个载荷，
    /// 而不是自己造一个 <c>InvalidOperationException</c>——判据必须落在
    /// 真实的 IPC 错误形状上，否则生产里 key 变了用例照样全绿。
    /// </summary>
    private static BackendException IndexingNotReady() => BackendException.FromIpcPayload(
        "validation",
        "indexing_not_ready: project search blocked while index invalidation is pending or processing",
        "ui.error.indexing_not_ready");

    private static RetrievalHit Hit(string documentId, string snippet) =>
        new($"chunk:{documentId}", documentId, snippet, 0.87, "full_text");

    /// <summary>
    /// 过滤后**可见**树里所有节点的标题。
    ///
    /// 刻意不用 <c>EnumerateSubtree()</c>：它走 <c>Children</c>（完整树），
    /// 拿它做「某章不在结果里」的断言会永远为假——量的是过滤前的东西。
    /// </summary>
    private static IEnumerable<string> VisibleTitles(WorksPageViewModel vm)
    {
        var pending = new Stack<WorksTreeItemViewModel>(vm.VisibleWorksTreeRoots);
        while (pending.Count > 0)
        {
            var node = pending.Pop();
            yield return node.Title;
            foreach (var child in node.VisibleChildren)
            {
                pending.Push(child);
            }
        }
    }

    private static async Task<WorksPageViewModel> LoadAsync(BodySearchBackend backend)
    {
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();
        Assert.NotEmpty(vm.WorksTreeRoots);
        return vm;
    }

    private static async Task WaitUntilAsync(Func<bool> predicate)
    {
        for (var attempt = 0; attempt < 400; attempt++)
        {
            if (predicate())
            {
                return;
            }
            await Task.Delay(10);
        }

        Assert.True(predicate(), "Timed out waiting for the body-search state to settle.");
    }

    /// <summary>
    /// ⚠️ `session.Dispatch` 必须用**有返回值**的重载：`Func&lt;Task&gt;` 那个
    /// 无返回值重载会**静默吞掉断言失败**，插 `Assert.Fail` 都仍绿 = 彻底空测。
    /// 照 `ReadingEditingParityTests.RunHeadlessAsync` 抄。
    /// </summary>
    private static async Task RunHeadlessAsync(Func<Task<bool>> body)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(body, CancellationToken.None);
    }

    /// 作品页只在 `Content` 态渲染树栏与搜索结果；替身没有真实项目，直接置态。
    private static void SetWorksTreeStateToContent(WorksPageViewModel viewModel)
    {
        var type = typeof(WorksPageViewModel);
        var stateType = type.GetNestedType("WorksTreeLoadState", BindingFlags.NonPublic)
                        ?? type.Assembly.GetTypes().First(candidate => candidate.Name == "WorksTreeLoadState");
        type.GetField("_worksTreeState", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(viewModel, Enum.Parse(stateType, "Content"));
    }

    private static async Task DrainAsync()
    {
        for (var i = 0; i < 16; i++)
        {
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Render);
        }
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    private static WorksTreeNode Tree() => new(
        "outline:global",
        "global_outline",
        "全局总纲",
        "planning/global.md",
        new[]
        {
            new WorksTreeNode(
                "stage:first",
                "stage_outline",
                "第一阶段",
                "planning/stages/first.md",
                new[]
                {
                    // ⚠️ 刻意**不填 ChapterId**：填了打开章节就会连带去拉章节总结，
                    // 而本用例的替身对 GetChapterSummaryViewAsync 返回 null ⇒ NRE，
                    // 失败会伪装成「正文搜索坏了」。章节总结不在 U184-A 的范围内，
                    // 用最小夹具把它排除掉，而不是在这里再造一份总结假数据。
                    new WorksTreeNode("chapter:1", "chapter", "第一章", "documents/chapter-1.md", Array.Empty<WorksTreeNode>()),
                    new WorksTreeNode("chapter:2", "chapter", "第二章", "documents/chapter-2.md", Array.Empty<WorksTreeNode>()),
                    new WorksTreeNode("chapter:3", "chapter", "第三章", "documents/chapter-3.md", Array.Empty<WorksTreeNode>()),
                },
                StageId: "first"),
        });

    private class BodySearchBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        public Func<string, int, Task<IReadOnlyList<RetrievalHit>>> SearchHandler { get; set; } =
            (_, _) => Task.FromResult<IReadOnlyList<RetrievalHit>>(Array.Empty<RetrievalHit>());

        public static BodySearchBackend Create()
        {
            var client = Create<IAriadneBackendClient, BodySearchBackend>();
            var backend = (BodySearchBackend)(object)client;
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
            if (targetMethod.Name == nameof(IAriadneBackendClient.GetWorksTreeAsync))
            {
                return Task.FromResult(Tree());
            }
            if (targetMethod.Name == nameof(IAriadneBackendClient.SearchProjectDocumentsAsync))
            {
                return SearchHandler((string)parameters[0]!, (int)parameters[1]!);
            }
            if (targetMethod.Name is nameof(IAriadneBackendClient.GetDocumentContentDetailsByPathAsync)
                or nameof(IAriadneBackendClient.GetDocumentContentDetailsAsync))
            {
                var path = (string)parameters[0]!;
                return Task.FromResult(new DocumentContentResult(
                    new DocumentMetadata(path, path, "markdown", "text/markdown", 8, "v1"),
                    $"正文：{BodyOnlyPhrase}"));
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
