using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Threading;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

[Collection("AvaloniaHeadless")]
public sealed class ProjectPageViewLifecycleTests
{
    [Fact]
    public async Task CachedProjectViews_RestoreHostDelegatesAfterReattach_AndClearThemOnDetach()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var names = DisplayNameService.LoadDefault();
            var backend = NoopBackend.Create();
            var window = new Window { Width = 1200, Height = 800 };
            window.Show();

            var workspace = new WorkspacePageViewModel(names, backend);
            var workspaceView = new WorkspacePageView { DataContext = workspace };
            await AssertWorkspaceLifecycleAsync(window, workspaceView, workspace);

            var works = new WorksPageViewModel(names, backend);
            var worksView = new WorksPageView { DataContext = works };
            await AssertWorksLifecycleAsync(window, worksView, works);

            var git = new GitPageViewModel(names, backend);
            var gitView = new GitPageView { DataContext = git };
            await AssertGitLifecycleAsync(window, gitView, git);

            window.Close();
            return true;
        }, CancellationToken.None);
    }

    private static async Task AssertWorkspaceLifecycleAsync(
        Window window,
        WorkspacePageView view,
        WorkspacePageViewModel viewModel)
    {
        window.Content = view;
        await DrainDispatcherAsync();
        Assert.NotNull(viewModel.PickFolder);
        Assert.NotNull(viewModel.PickFile);
        Assert.NotNull(viewModel.RequestEnsureNodeVisible);

        window.Content = null;
        await DrainDispatcherAsync();
        Assert.Null(viewModel.PickFolder);
        Assert.Null(viewModel.PickFile);
        Assert.Null(viewModel.RequestEnsureNodeVisible);

        window.Content = view;
        await DrainDispatcherAsync();
        Assert.NotNull(viewModel.PickFolder);
        Assert.NotNull(viewModel.PickFile);
        Assert.NotNull(viewModel.RequestEnsureNodeVisible);

        window.Content = null;
        await DrainDispatcherAsync();
    }

    private static async Task AssertWorksLifecycleAsync(
        Window window,
        WorksPageView view,
        WorksPageViewModel viewModel)
    {
        window.Content = view;
        await DrainDispatcherAsync();
        Assert.NotNull(viewModel.RequestEditorCopy);
        Assert.NotNull(viewModel.RequestEditorSelection);
        Assert.NotNull(viewModel.PickImportSourceFile);

        window.Content = null;
        await DrainDispatcherAsync();
        Assert.Null(viewModel.RequestEditorCopy);
        Assert.Null(viewModel.RequestEditorSelection);
        Assert.Null(viewModel.PickImportSourceFile);

        window.Content = view;
        await DrainDispatcherAsync();
        Assert.NotNull(viewModel.RequestEditorCopy);
        Assert.NotNull(viewModel.RequestEditorSelection);
        Assert.NotNull(viewModel.PickImportSourceFile);

        window.Content = null;
        await DrainDispatcherAsync();
    }

    private static async Task AssertGitLifecycleAsync(
        Window window,
        GitPageView view,
        GitPageViewModel viewModel)
    {
        window.Content = view;
        await DrainDispatcherAsync();
        Assert.NotNull(viewModel.RequestCopyText);

        window.Content = null;
        await DrainDispatcherAsync();
        Assert.Null(viewModel.RequestCopyText);

        window.Content = view;
        await DrainDispatcherAsync();
        Assert.NotNull(viewModel.RequestCopyText);

        window.Content = null;
        await DrainDispatcherAsync();
    }

    private static async Task DrainDispatcherAsync()
    {
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == $"get_{nameof(IAriadneBackendClient.HasProjectRoot)}")
            {
                return false;
            }

            throw new NotSupportedException(targetMethod?.Name);
        }
    }
}
