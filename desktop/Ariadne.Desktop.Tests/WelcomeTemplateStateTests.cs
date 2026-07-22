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
        var pickerCalled = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var vm = new WelcomeViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            backend.Client,
            pickProjectFolder: _ =>
            {
                pickerCalled.TrySetResult(true);
                return Task.FromResult<string?>(null);
            });

        var load = vm.LoadAsync();
        await backend.RecentProjectsStarted.Task;

        Assert.True(vm.IsLoading);
        Assert.True(vm.IsRecentLoading);
        Assert.False(vm.IsRecentEmpty);
        // 最近项目列表慢响应不能锁死新建/打开项目；两条状态链彼此独立。
        Assert.True(vm.CreateProjectCommand.CanExecute(null));
        Assert.True(vm.OpenProjectCommand.CanExecute(null));
        vm.OpenProjectCommand.Execute(null);
        await pickerCalled.Task.WaitAsync(TimeSpan.FromSeconds(2));

        backend.ReleaseRecentProjects();
        await load;
        Assert.True(vm.IsRecentEmpty);

        backend.ThrowRecentProjects = true;
        await vm.LoadAsync();
        Assert.True(vm.IsRecentError);
        Assert.False(vm.IsRecentEmpty);
        Assert.False(string.IsNullOrWhiteSpace(vm.RecentErrorText));
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task WelcomeRecentProjects_LateOldRequestCannotOverwriteNewerRefresh(bool oldRequestFails)
    {
        var backend = StateBackend.Create();
        backend.BlockRecentProjects = true;
        backend.BlockedRecentProjectsFail = oldRequestFails;
        backend.BlockedRecentProjectsResult = new[]
        {
            new RecentProjectEntry("Old", "/projects/old", 0),
        };
        var vm = new WelcomeViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            backend.Client);

        var oldLoad = vm.LoadAsync();
        await backend.RecentProjectsStarted.Task;
        backend.BlockRecentProjects = false;
        backend.RecentProjectsResult = new[]
        {
            new RecentProjectEntry("New", "/projects/new", 0),
        };
        await vm.RefreshRecentProjectsForTestsAsync();
        backend.ReleaseRecentProjects();
        await oldLoad;

        var item = Assert.Single(vm.RecentProjects);
        Assert.Equal("New", item.Name);
        Assert.True(vm.HasRecentProjects);
        Assert.False(vm.IsRecentError);
        Assert.False(vm.IsRecentLoading);
        Assert.Equal(string.Empty, vm.RecentErrorText);
    }

    [Fact]
    public async Task CreateProject_DelegatesWholeInitializationToBackend_AndEntersVerifiedReport()
    {
        var backend = StateBackend.Create();
        CurrentProjectStatus? opened = null;
        var vm = new WelcomeViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            backend.Client,
            status =>
            {
                opened = status;
                return Task.CompletedTask;
            });
        using var parent = new TemporaryDirectory();

        var status = await vm.CreateProjectAtAsync(parent.Path, "My Project");

        Assert.NotNull(status);
        Assert.Equal("My Project", status.ProjectName);
        Assert.Equal(status, opened);
        Assert.Equal(1, backend.CreateProjectCalls);
        Assert.Equal(0, backend.GetCurrentProjectCalls);
        Assert.False(backend.DirectoryExistedAtCreateCall);
        Assert.False(string.IsNullOrWhiteSpace(backend.CreatedProjectRoot));
        Assert.False(Directory.Exists(backend.CreatedProjectRoot));
    }

    [Fact]
    public void ProjectFolderPicker_IsOwnedByAttachedMainWindow_NotNavigatedWelcomeView()
    {
        var root = ResolveRepoRoot();
        var mainWindow = File.ReadAllText(Path.Combine(
            root,
            "desktop",
            "Ariadne.Desktop",
            "Views",
            "MainWindow.axaml.cs"));
        var welcomeView = File.ReadAllText(Path.Combine(
            root,
            "desktop",
            "Ariadne.Desktop",
            "Views",
            "WelcomeView.axaml.cs"));

        Assert.Contains("viewModel.Welcome.SetProjectFolderPicker(PickProjectFolderAsync)", mainWindow, StringComparison.Ordinal);
        Assert.Contains("StorageProvider.CanPickFolder", mainWindow, StringComparison.Ordinal);
        Assert.Contains("StorageProvider.OpenFolderPickerAsync", mainWindow, StringComparison.Ordinal);
        Assert.DoesNotContain("SetProjectFolderPicker", welcomeView, StringComparison.Ordinal);
        Assert.DoesNotContain("TopLevel.GetTopLevel(this)", welcomeView, StringComparison.Ordinal);
    }

    [Fact]
    public async Task OpenProjectPicker_CancelRestoresBothProjectCommands()
    {
        var backend = StateBackend.Create();
        var pickerStarted = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var pickerResult = new TaskCompletionSource<string?>(TaskCreationOptions.RunContinuationsAsynchronously);
        var vm = new WelcomeViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            backend.Client,
            pickProjectFolder: _ =>
            {
                pickerStarted.TrySetResult(true);
                return pickerResult.Task;
            });

        var operation = vm.OpenProjectAsync();
        await pickerStarted.Task.WaitAsync(TimeSpan.FromSeconds(2));
        Assert.True(vm.IsLoading);
        Assert.False(vm.OpenProjectCommand.CanExecute(null));
        Assert.False(vm.CreateProjectCommand.CanExecute(null));

        pickerResult.SetResult(null);
        await operation.WaitAsync(TimeSpan.FromSeconds(2));

        Assert.False(vm.IsLoading);
        Assert.True(vm.OpenProjectCommand.CanExecute(null));
        Assert.True(vm.CreateProjectCommand.CanExecute(null));
        Assert.Equal(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault().Text("ui.common.cancel"),
            vm.StatusText);
    }

    [Fact]
    public async Task OpenProjectPicker_ExceptionSurfacesLocalizedErrorAndRestoresCommands()
    {
        var names = Ariadne.Desktop.Localization.DisplayNameService.LoadDefault();
        var vm = new WelcomeViewModel(
            names,
            StateBackend.Create().Client,
            pickProjectFolder: _ => Task.FromException<string?>(new IOException("picker failed")));

        await vm.OpenProjectAsync().WaitAsync(TimeSpan.FromSeconds(2));

        Assert.False(vm.IsLoading);
        Assert.True(vm.OpenProjectCommand.CanExecute(null));
        Assert.True(vm.CreateProjectCommand.CanExecute(null));
        Assert.Equal(names.Text("ui.error.io"), vm.StatusText);
    }

    [Fact]
    public async Task MainWindowProjectCommands_StayDisabledUntilFolderPickerCompletes()
    {
        var pickerStarted = new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously);
        var pickerResult = new TaskCompletionSource<string?>(TaskCreationOptions.RunContinuationsAsynchronously);
        var window = new MainWindowViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            StateBackend.Create().Client);
        window.Welcome.SetProjectFolderPicker(_ =>
        {
            pickerStarted.TrySetResult(true);
            return pickerResult.Task;
        });

        window.OpenProjectCommand.Execute(null);
        await pickerStarted.Task.WaitAsync(TimeSpan.FromSeconds(2));

        Assert.False(window.OpenProjectCommand.CanExecute(null));
        Assert.False(window.CreateProjectCommand.CanExecute(null));
        Assert.False(window.SwitchProjectCommand.CanExecute(null));

        pickerResult.SetResult(null);
        await WaitUntilAsync(
            () => window.OpenProjectCommand.CanExecute(null),
            TimeSpan.FromSeconds(2));

        Assert.True(window.CreateProjectCommand.CanExecute(null));
        Assert.True(window.SwitchProjectCommand.CanExecute(null));
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
    public async Task TemplateSearch_RefreshesGlobalRepositoryForEachNewSearch()
    {
        var backend = StateBackend.Create();
        var vm = new TemplateMarketPageViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            backend.Client);

        vm.SearchQuery = "empty";
        await vm.SearchForTestsAsync();
        backend.RepositoryBaseUrl = "https://templates-2.example.test";
        await vm.SearchForTestsAsync();

        Assert.Equal(2, backend.RepositorySettingsCalls);
        Assert.Equal(
            new[] { "https://templates.example.test", "https://templates-2.example.test" },
            backend.SearchCalls.Select(call => call.BaseUrl).ToArray());
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
    public async Task TemplateInstall_PinsTheProjectSelectedBeforeTheInstallRequest()
    {
        var backend = StateBackend.Create();
        var vm = new TemplateMarketPageViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            backend.Client);
        await vm.EnsureInitialCatalogLoadedAsync();
        var template = Assert.Single(vm.Templates);

        await vm.InstallForTestsAsync(template);

        Assert.Equal("/projects/selected", backend.InstallExpectedProjectRoot);
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
        public bool BlockedRecentProjectsFail { get; set; }
        public bool ThrowRecentProjects { get; set; }
        public bool ThrowSearch { get; set; }
        public IReadOnlyList<RecentProjectEntry> BlockedRecentProjectsResult { get; set; } =
            Array.Empty<RecentProjectEntry>();
        public IReadOnlyList<RecentProjectEntry> RecentProjectsResult { get; set; } =
            Array.Empty<RecentProjectEntry>();
        public string RepositoryBaseUrl { get; set; } = "https://templates.example.test";
        public string? InstallExpectedProjectRoot { get; private set; }
        public string? CreatedProjectRoot { get; private set; }
        public bool DirectoryExistedAtCreateCall { get; private set; }
        public int CreateProjectCalls { get; private set; }
        public int GetCurrentProjectCalls { get; private set; }
        public int RepositorySettingsCalls { get; private set; }
        public TaskCompletionSource<bool> RecentProjectsStarted { get; } =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        public TaskCompletionSource<bool> SlowSearchStarted { get; } =
            new(TaskCreationOptions.RunContinuationsAsynchronously);
        public ConcurrentQueue<(string BaseUrl, string Query, int Page)> SearchCalls { get; } = new();

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
                return Task.FromResult(RecentProjectsResult);
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.GetTemplateRepositorySettingsAsync))
            {
                RepositorySettingsCalls++;
                return Task.FromResult(new TemplateRepositorySettings(RepositoryBaseUrl));
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.GetCurrentProjectAsync))
            {
                GetCurrentProjectCalls++;
                return Task.FromResult<CurrentProjectStatus?>(
                    new CurrentProjectStatus("/projects/selected", "Selected"));
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.CreateProjectAsync))
            {
                CreateProjectCalls++;
                var projectRoot = Assert.IsType<string>(args![0]);
                var projectName = Assert.IsType<string>(args[1]);
                CreatedProjectRoot = projectRoot;
                DirectoryExistedAtCreateCall = Directory.Exists(projectRoot);
                var createdDirs = new[]
                {
                    ".config",
                    ".runtime",
                    "planning",
                    "planning/stages",
                    "planning/chapters",
                    "documents",
                    "workflows",
                    "skills",
                    "exports",
                }.Select(path => Path.Combine(projectRoot, path)).ToArray();
                var configFiles = new[]
                {
                    "app.yaml",
                    "providers.yaml",
                    "permissions.yaml",
                    "rag.yaml",
                    "workflow.yaml",
                    "git.yaml",
                    "auto_mode.yaml",
                }.Select(file => Path.Combine(projectRoot, ".config", file)).ToArray();
                RecentProjectsResult = new[]
                {
                    new RecentProjectEntry(projectName, projectRoot, 1),
                };
                return Task.FromResult(new ProjectInitReport(
                    projectRoot,
                    projectName,
                    createdDirs,
                    configFiles,
                    true,
                    true));
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.InstallTemplateAsync))
            {
                InstallExpectedProjectRoot = Assert.IsType<string>(args![2]);
                return Task.FromResult(new TemplateInstallReport(
                    "installed",
                    "1.0.0",
                    "workflows/installed/workflow.json",
                    false,
                    Array.Empty<string>()));
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.SearchTemplatesAsync))
            {
                var baseUrl = (string)args![0]!;
                var query = (string)args![1]!;
                var page = (int)args[2]!;
                SearchCalls.Enqueue((baseUrl, query, page));
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
            if (BlockedRecentProjectsFail)
            {
                throw new InvalidOperationException("stale recent projects failure");
            }
            return BlockedRecentProjectsResult;
        }

        private async Task<IReadOnlyList<TemplateSummary>> WaitSlowSearchAsync()
        {
            await _slowSearchRelease.Task.ConfigureAwait(false);
            return new[] { new TemplateSummary("a", "A", Array.Empty<string>(), false) };
        }
    }

    private static async Task WaitUntilAsync(Func<bool> predicate, TimeSpan timeout)
    {
        var deadline = DateTime.UtcNow + timeout;
        while (!predicate())
        {
            if (DateTime.UtcNow >= deadline)
            {
                throw new TimeoutException("condition was not reached before the test timeout");
            }
            await Task.Delay(10);
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

    private sealed class TemporaryDirectory : IDisposable
    {
        public TemporaryDirectory()
        {
            Path = System.IO.Path.Combine(
                System.IO.Path.GetTempPath(),
                "ariadne-welcome-test-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(Path);
        }

        public string Path { get; }

        public void Dispose()
        {
            try
            {
                Directory.Delete(Path, recursive: true);
            }
            catch
            {
            }
        }
    }
}
