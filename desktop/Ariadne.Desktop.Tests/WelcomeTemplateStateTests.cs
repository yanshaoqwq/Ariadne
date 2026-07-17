using System.Collections.Concurrent;
using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class WelcomeTemplateStateTests
{
    [Fact]
    public async Task WelcomeRecentProjects_ExposesLoadingState_AndSeparatesErrorFromEmpty()
    {
        var backend = StateBackend.Create();
        backend.BlockRecentProjects = true;
        var vm = new WelcomeViewModel(Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(), backend.Client);

        var load = vm.LoadAsync();
        await backend.RecentProjectsStarted.Task;

        Assert.True(vm.IsLoading);
        Assert.True(vm.IsRecentLoading);
        Assert.False(vm.IsRecentEmpty);
        Assert.False(vm.CreateProjectCommand.CanExecute(null));

        backend.ReleaseRecentProjects();
        await load;
        Assert.True(vm.IsRecentEmpty);

        backend.ThrowRecentProjects = true;
        await vm.LoadAsync();
        Assert.True(vm.IsRecentError);
        Assert.False(vm.IsRecentEmpty);
        Assert.False(string.IsNullOrWhiteSpace(vm.RecentErrorText));
    }

    [Fact]
    public async Task TemplateSearch_UsesLatestQueryGeneration_WhenOlderRequestReturnsLate()
    {
        var backend = StateBackend.Create();
        var vm = new TemplateMarketPageViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            backend.Client);

        vm.SearchQuery = "A";
        var slow = vm.SearchForTestsAsync();
        await backend.SlowSearchStarted.Task;

        vm.SearchQuery = "B";
        await vm.SearchForTestsAsync();
        backend.ReleaseSlowSearch();
        await slow;

        var result = Assert.Single(vm.Templates);
        Assert.Equal("b", result.Id);
        Assert.False(vm.IsError);
    }

    [Fact]
    public async Task TemplateSearch_LoadMoreCommitsPageOnlyAfterSuccess_AndStopsAtEnd()
    {
        var backend = StateBackend.Create();
        var vm = new TemplateMarketPageViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            backend.Client);

        vm.SearchQuery = "paged";
        await vm.SearchForTestsAsync();
        Assert.True(vm.IsLoadMoreVisible);

        await vm.LoadMoreForTestsAsync();
        Assert.True(vm.IsError);
        Assert.True(vm.CanLoadMore);

        await vm.LoadMoreForTestsAsync();
        Assert.Equal(21, vm.Templates.Count);
        Assert.True(vm.IsEndOfList);
        Assert.False(vm.IsLoadMoreVisible);
        Assert.Equal(new[] { 0, 1, 1 }, backend.SearchCalls
            .Where(call => call.Query == "paged")
            .Select(call => call.Page)
            .ToArray());
    }

    [Fact]
    public async Task TemplateSearch_EmptyAndErrorStatesDoNotRenderAsTheSameState()
    {
        var backend = StateBackend.Create();
        var vm = new TemplateMarketPageViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            backend.Client);

        vm.SearchQuery = "empty";
        await vm.SearchForTestsAsync();
        Assert.True(vm.IsEmpty);
        Assert.False(vm.IsError);
        Assert.False(vm.IsLoadMoreVisible);

        backend.ThrowSearch = true;
        await vm.SearchForTestsAsync();
        Assert.True(vm.IsError);
        Assert.False(vm.IsEmpty);
    }

    [Fact]
    public async Task TemplateMarket_LoadsOfficialCatalogOnceAndLocalizesBundledMetadata()
    {
        var backend = StateBackend.Create();
        var vm = new TemplateMarketPageViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            backend.Client);

        await vm.EnsureInitialCatalogLoadedAsync();
        await vm.EnsureInitialCatalogLoadedAsync();

        var call = Assert.Single(backend.SearchCalls);
        Assert.Equal(string.Empty, call.Query);
        var template = Assert.Single(vm.Templates);
        Assert.Equal("长篇小说起步", template.Name);
        Assert.Contains("写小说", template.TagsText, StringComparison.Ordinal);
        Assert.Contains("大纲生成", template.TagsText, StringComparison.Ordinal);

        var root = ResolveRepoRoot();
        var view = File.ReadAllText(Path.Combine(
            root,
            "desktop",
            "Ariadne.Desktop",
            "Views",
            "TemplateMarketPageView.axaml"));
        Assert.Contains("Loaded=\"OnLoaded\"", view, StringComparison.Ordinal);
    }

    [Fact]
    public void WelcomeRecentProjects_UsesContentSizedScrollableCard_WithoutDuplicateFooter()
    {
        var root = ResolveRepoRoot();
        var view = File.ReadAllText(Path.Combine(
            root,
            "desktop",
            "Ariadne.Desktop",
            "Views",
            "WelcomeView.axaml"));

        Assert.Contains("x:Name=\"RecentProjectsCard\"", view, StringComparison.Ordinal);
        Assert.Contains("MaxHeight=\"520\"", view, StringComparison.Ordinal);
        Assert.Contains("VerticalAlignment=\"Center\"", view, StringComparison.Ordinal);
        Assert.Contains("VerticalScrollBarVisibility=\"Auto\"", view, StringComparison.Ordinal);
        Assert.DoesNotContain("MinHeight=\"440\"", view, StringComparison.Ordinal);
        Assert.DoesNotContain("Grid RowDefinitions=\"*,Auto\"", view, StringComparison.Ordinal);
        Assert.Equal(1, view.Split("<Ellipse ", StringSplitOptions.None).Length - 1);
    }

    private class StateBackend : DispatchProxy
    {
        private readonly ConcurrentDictionary<string, int> _calls = new(StringComparer.Ordinal);
        private readonly TaskCompletionSource<bool> _slowSearchRelease =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly TaskCompletionSource<bool> _recentProjectsRelease =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        private bool _failedPagedPage;

        public IAriadneBackendClient Client { get; private set; } = null!;
        public bool BlockRecentProjects { get; set; }
        public bool ThrowRecentProjects { get; set; }
        public bool ThrowSearch { get; set; }
        public TaskCompletionSource<bool> RecentProjectsStarted { get; } =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        public TaskCompletionSource<bool> SlowSearchStarted { get; } =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        public ConcurrentQueue<(string Query, int Page)> SearchCalls { get; } = new();

        public static StateBackend Create()
        {
            var client = Create<IAriadneBackendClient, StateBackend>();
            var backend = (StateBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        public void ReleaseRecentProjects() => _recentProjectsRelease.TrySetResult(true);

        public void ReleaseSlowSearch() => _slowSearchRelease.TrySetResult(true);

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.ListRecentProjectsAsync))
            {
                if (ThrowRecentProjects)
                {
                    return Task.FromException<IReadOnlyList<RecentProjectEntry>>(
                        new InvalidOperationException("recent projects unavailable"));
                }
                if (BlockRecentProjects)
                {
                    RecentProjectsStarted.TrySetResult(true);
                    return WaitRecentProjectsAsync();
                }
                return Task.FromResult<IReadOnlyList<RecentProjectEntry>>(Array.Empty<RecentProjectEntry>());
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.GetTemplateRepositorySettingsAsync))
            {
                return Task.FromResult(new TemplateRepositorySettings("https://templates.example.test"));
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.SearchTemplatesAsync))
            {
                var query = (string)args![1]!;
                var page = (int)args[2]!;
                SearchCalls.Enqueue((query, page));
                if (ThrowSearch)
                {
                    return Task.FromException<IReadOnlyList<TemplateSummary>>(
                        new InvalidOperationException("template search unavailable"));
                }
                if (query == "A")
                {
                    SlowSearchStarted.TrySetResult(true);
                    return WaitSlowSearchAsync();
                }
                if (query == "B")
                {
                    return Task.FromResult<IReadOnlyList<TemplateSummary>>(new[]
                    {
                        new TemplateSummary("b", "B", Array.Empty<string>(), false),
                    });
                }
                if (query == "empty")
                {
                    return Task.FromResult<IReadOnlyList<TemplateSummary>>(Array.Empty<TemplateSummary>());
                }
                if (query.Length == 0)
                {
                    return Task.FromResult<IReadOnlyList<TemplateSummary>>(new[]
                    {
                        new TemplateSummary(
                            "official-novel-starter",
                            "ui.template.builtin.novel_starter.name",
                            new[] { "ui.template.tag.novel", "ui.template.tag.outline" },
                            false),
                    });
                }
                if (query == "paged" && page == 0)
                {
                    return Task.FromResult<IReadOnlyList<TemplateSummary>>(
                        Enumerable.Range(0, 20)
                            .Select(index => new TemplateSummary($"p{index}", $"P{index}", Array.Empty<string>(), false))
                            .ToArray());
                }
                if (query == "paged" && page == 1 && !_failedPagedPage)
                {
                    _failedPagedPage = true;
                    return Task.FromException<IReadOnlyList<TemplateSummary>>(
                        new InvalidOperationException("page temporarily unavailable"));
                }
                if (query == "paged" && page == 1)
                {
                    return Task.FromResult<IReadOnlyList<TemplateSummary>>(new[]
                    {
                        new TemplateSummary("p20", "P20", Array.Empty<string>(), false),
                    });
                }
            }

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                var value = resultType.IsValueType ? Activator.CreateInstance(resultType) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }
            return targetMethod.ReturnType == typeof(Task) ? Task.CompletedTask : null;
        }

        private async Task<IReadOnlyList<RecentProjectEntry>> WaitRecentProjectsAsync()
        {
            await _recentProjectsRelease.Task.ConfigureAwait(false);
            return Array.Empty<RecentProjectEntry>();
        }

        private async Task<IReadOnlyList<TemplateSummary>> WaitSlowSearchAsync()
        {
            await _slowSearchRelease.Task.ConfigureAwait(false);
            return new[] { new TemplateSummary("a", "A", Array.Empty<string>(), false) };
        }
    }

    private static string ResolveRepoRoot()
    {
        var path = Path.GetDirectoryName(typeof(WelcomeTemplateStateTests).Assembly.Location)!;
        while (!string.IsNullOrEmpty(path) && !File.Exists(Path.Combine(path, "desktop", "Ariadne.slnx")))
        {
            path = Directory.GetParent(path)?.FullName ?? string.Empty;
        }
        return path;
    }
}
