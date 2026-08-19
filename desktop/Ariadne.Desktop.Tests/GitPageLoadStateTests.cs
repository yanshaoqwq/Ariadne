using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U182-K / U182-M：Git 页的 Loading / Error / ContentError / Empty / IdleNeedProject 五态。
///
/// 原缺陷：`IsCommitListEmpty => Commits.Count == 0` 是**唯一判据**，且分支图与
/// fallback 双双失败时 `catch` 只写 StatusText、不标记状态 ⇒ 有 200 个存档点的项目
/// 在后端报错时被告知「你没有存档」，刷新期间也是同一画面。
///
/// 判据一律落在**用户可见的呈现属性**上（IsError / IsCommitListEmpty / ShowEmpty /
/// ShowContent），不断言 enum 本身——「LoadState 变了」但界面属性没跟着变的话，
/// 屏幕上与没修一模一样。
/// </summary>
public sealed class GitPageLoadStateTests
{
    [Fact]
    public async Task BackendFailureWithNoHistory_ShowsErrorAndNeverMasqueradesAsEmpty()
    {
        var backend = GitBackend.Create();
        backend.GraphHandler = _ => throw new IOException("branch graph exploded");
        backend.HistoryHandler = _ => throw new IOException("history exploded");
        var viewModel = NewViewModel(backend);

        await viewModel.ReloadProjectDataAsync();

        // 核心断言：报错不得伪装成空。
        Assert.True(viewModel.IsError);
        Assert.False(viewModel.IsCommitListEmpty);
        Assert.False(viewModel.ShowEmpty);
        // 手上一条存档都没有 ⇒ 整页错误页 + 重试。
        Assert.True(viewModel.IsStandaloneError);
        Assert.False(viewModel.IsContentError);
        Assert.False(viewModel.ShowContent);
        Assert.False(viewModel.IsLoading);
        // 错误必须配文字，且不能是原始异常串。
        Assert.NotEmpty(viewModel.ErrorText);
        Assert.DoesNotContain("[ui.git.error.title", viewModel.ErrorTitle, StringComparison.Ordinal);
        Assert.Equal(
            DisplayNameService.LoadDefault().Text("ui.git.error.title"),
            viewModel.ErrorTitle);
    }

    [Fact]
    public async Task BackendFailureAfterGoodLoad_KeepsCommitsAndShowsInlineErrorNotFullPage()
    {
        var backend = GitBackend.Create();
        backend.Graph = new[] { Node("first", "First"), Node("second", "Second") };
        var viewModel = NewViewModel(backend);
        await viewModel.ReloadProjectDataAsync();
        Assert.True(viewModel.ShowContent);
        Assert.Equal(2, viewModel.Commits.Count);

        // 第二轮刷新两条路都炸：内容必须留着。
        backend.GraphHandler = _ => throw new IOException("branch graph exploded");
        backend.HistoryHandler = _ => throw new IOException("history exploded");
        await viewModel.ReloadProjectDataAsync();

        Assert.True(viewModel.IsError);
        Assert.True(viewModel.IsContentError);
        // 已有内容时**不该**换成整页错误页——那等于拿用户唯一的诊断材料换一句道歉。
        Assert.False(viewModel.IsStandaloneError);
        Assert.True(viewModel.ShowContent);
        Assert.Equal(2, viewModel.Commits.Count);
        Assert.False(viewModel.IsCommitListEmpty);
        Assert.False(viewModel.ShowEmpty);
    }

    [Fact]
    public async Task WhileRequestInFlight_ShowsLoadingAndHidesEmptyState()
    {
        var backend = GitBackend.Create();
        backend.Graph = Array.Empty<BranchGraphNode>();
        var viewModel = NewViewModel(backend);

        // ⚠️ 必须先完成一轮加载，把状态推离初始的 Loading。
        // 否则本用例是空测：`_loadState` 初值就是 Loading，摘掉 RefreshCoreAsync 里那句
        // `LoadState = Loading` 之后首轮照样「加载中」，断言什么也没验证。
        await viewModel.ReloadProjectDataAsync();
        Assert.True(viewModel.ShowEmpty);
        Assert.False(viewModel.IsLoading);

        var graphStarted = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        var graphRelease = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        backend.GraphHandler = async _ =>
        {
            graphStarted.TrySetResult();
            await graphRelease.Task;
            return (IReadOnlyList<BranchGraphNode>)new[] { Node("first", "First") };
        };

        var reload = viewModel.ReloadProjectDataAsync();
        await graphStarted.Task;

        // 加载中：既不是空态也不是错误态，而且加载指示为真。
        Assert.True(viewModel.IsLoading);
        Assert.False(viewModel.ShowEmpty);
        Assert.False(viewModel.IsCommitListEmpty);
        Assert.False(viewModel.IsError);
        Assert.DoesNotContain("[ui.git.loading", viewModel.LoadingText, StringComparison.Ordinal);

        graphRelease.TrySetResult();
        await reload;

        Assert.False(viewModel.IsLoading);
        Assert.True(viewModel.ShowContent);
    }

    [Fact]
    public async Task SuccessfulEmptyLoad_IsTheOnlyPathToTheGitEmptyCopy()
    {
        var backend = GitBackend.Create();
        backend.Graph = Array.Empty<BranchGraphNode>();
        var viewModel = NewViewModel(backend);

        await viewModel.ReloadProjectDataAsync();

        var names = DisplayNameService.LoadDefault();
        Assert.True(viewModel.ShowEmpty);
        Assert.True(viewModel.IsCommitListEmpty);
        Assert.False(viewModel.IsError);
        Assert.False(viewModel.IsLoading);
        Assert.Equal(names.Text("ui.empty.git.title"), viewModel.EmptyTitle);
        // 项目里没存档时该点的是右栏「创建存档」，不是打开项目。
        Assert.False(viewModel.ShowOpenProjectAction);
    }

    [Fact]
    public async Task NoProjectOpen_ShowsNeedProjectCopyWithAClickableOpenProjectAction()
    {
        var backend = GitBackend.Create();
        backend.HasProjectRoot = false;
        var opened = 0;
        var viewModel = new GitPageViewModel(
            DisplayNameService.LoadDefault(),
            backend.Client,
            requestOpenProject: () =>
            {
                opened++;
                return Task.CompletedTask;
            });

        await viewModel.ReloadProjectDataAsync();

        var names = DisplayNameService.LoadDefault();
        Assert.True(viewModel.ShowEmpty);
        Assert.Equal(names.Text("ui.empty.need_project.title"), viewModel.EmptyTitle);
        Assert.Equal(names.Text("ui.empty.need_project.hint"), viewModel.EmptyHint);

        // U182-M：这颗按钮点了必须真的发生事情——原缺陷正是「点了什么也不会发生」。
        Assert.True(viewModel.ShowOpenProjectAction);
        Assert.True(viewModel.OpenProjectCommand.CanExecute(null));
        Assert.True(viewModel.OpenProjectCommand.TryExecute());
        Assert.Equal(1, opened);
        Assert.DoesNotContain("[ui.layout.open_project", viewModel.OpenProjectText, StringComparison.Ordinal);
    }

    [Fact]
    public async Task WithoutAnInjectedOpenProjectPath_TheActionIsHiddenRatherThanDead()
    {
        var backend = GitBackend.Create();
        backend.HasProjectRoot = false;
        // 宿主没注入打开链路（例如测试宿主、独立预览）：宁可不显示按钮，
        // 也不要一颗点了没反应的——那正是 U182-M 报的缺陷形态。
        var viewModel = NewViewModel(backend);

        await viewModel.ReloadProjectDataAsync();

        Assert.True(viewModel.ShowEmpty);
        Assert.False(viewModel.ShowOpenProjectAction);
        Assert.False(viewModel.OpenProjectCommand.CanExecute(null));
    }

    /// <summary>
    /// 守卫：错误态与空态在 XAML 里**不能绑同一个属性**——那就是原缺陷的形态。
    ///
    /// 源码文本断言之所以必要：本机 Avalonia headless 对控件子类有盲区，
    /// 而这条性质是「两个 IsVisible 表达式互不相同」，属于可以纯文本判定的结构性质。
    /// </summary>
    [Fact]
    public void GitPageView_BindsErrorEmptyAndLoadingVisibilityToDistinctProperties()
    {
        var xaml = File.ReadAllText(ResolveDesktopSource("Views", "GitPageView.axaml"));

        Assert.Contains("IsVisible=\"{Binding ShowEmpty}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("IsVisible=\"{Binding IsStandaloneError}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("IsVisible=\"{Binding IsContentError}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("IsVisible=\"{Binding IsLoading}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("IsVisible=\"{Binding ShowContent}\"", xaml, StringComparison.Ordinal);
        // 原缺陷判据必须彻底离开视图：Commits.Count==0 不再决定任何呈现。
        Assert.DoesNotContain("IsVisible=\"{Binding IsCommitListEmpty}\"", xaml, StringComparison.Ordinal);
        Assert.DoesNotContain("IsVisible=\"{Binding HasCommits}\"", xaml, StringComparison.Ordinal);

        // 三个态各绑一个**互不相同**的属性名，而不是共用一个判据。
        var bound = new[] { "ShowEmpty", "IsStandaloneError", "IsLoading" };
        Assert.Equal(bound.Length, bound.Distinct(StringComparer.Ordinal).Count());

        // 错误态与无项目空态都必须给出路（重试 / 打开项目），不能只是死文案。
        Assert.Contains("Command=\"{Binding RefreshCommand}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding OpenProjectCommand}\"", xaml, StringComparison.Ordinal);
        Assert.Contains("IsVisible=\"{Binding ShowOpenProjectAction}\"", xaml, StringComparison.Ordinal);
        // 加载指示走项目现成的矢量基元，不是图标字体也不是自造转圈。
        Assert.Contains("<ctl:BusyDots IsActive=\"{Binding IsLoading}\"", xaml, StringComparison.Ordinal);
    }

    private static GitPageViewModel NewViewModel(GitBackend backend) =>
        new(DisplayNameService.LoadDefault(), backend.Client);

    private static BranchGraphNode Node(string id, string summary) => new(
        id,
        Array.Empty<string>(),
        Array.Empty<string>(),
        summary,
        DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
        "Ariadne Test");

    private static string ResolveDesktopSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var attempt = 0; attempt < 12 && walk is not null; attempt++)
        {
            var candidate = Path.Combine(new[] { walk.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            walk = walk.Parent;
        }
        throw new FileNotFoundException(string.Join('/', parts));
    }

    // DispatchProxy 的宿主类不能 sealed——它要在运行时派生该类型。
    private class GitBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public bool HasProjectRoot { get; set; } = true;
        public IReadOnlyList<BranchGraphNode> Graph { get; set; } = Array.Empty<BranchGraphNode>();
        public GitRepositoryStatus RepositoryStatus { get; set; } = new(
            "healthy",
            "main",
            "head-commit",
            false,
            null,
            0,
            string.Empty);

        /// <summary>分支图取数钩子；默认回 <see cref="Graph"/>，测错误路径时改成抛。</summary>
        public Func<CancellationToken, Task<IReadOnlyList<BranchGraphNode>>>? GraphHandler { get; set; }

        /// <summary>fallback 历史取数钩子。Git 页在分支图失败后会退到它，两条都要炸才算「加载失败」。</summary>
        public Func<CancellationToken, Task<IReadOnlyList<GitCommitSummary>>>? HistoryHandler { get; set; }

        public static GitBackend Create()
        {
            var client = Create<IAriadneBackendClient, GitBackend>();
            var backend = (GitBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return HasProjectRoot;
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetGitRepositoryStatusAsync))
            {
                return Task.FromResult(RepositoryStatus);
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetGitBranchGraphAsync))
            {
                var token = args is { Length: > 1 } ? (CancellationToken)args[1]! : CancellationToken.None;
                return GraphHandler is null ? Task.FromResult(Graph) : GraphHandler(token);
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetGitHistoryAsync))
            {
                var token = args is { Length: > 0 } ? (CancellationToken)args[0]! : CancellationToken.None;
                return HistoryHandler is null
                    ? Task.FromResult<IReadOnlyList<GitCommitSummary>>(Array.Empty<GitCommitSummary>())
                    : HistoryHandler(token);
            }

            throw new NotSupportedException(targetMethod?.Name);
        }
    }
}
