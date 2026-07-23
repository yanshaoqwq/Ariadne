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
public sealed class SettingsViewLifecycleTests
{
    [Fact]
    public async Task SettingsView_VisualTreeLifecycleKeepsOneBindingAndCommitsOneScroll()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
        var first = NewViewModel();
        var second = NewViewModel();
        var view = new SettingsPageView { DataContext = first };
        var window = new Window
        {
            Width = 1200,
            Height = 800,
            Content = view,
        };

        window.Show();
        await DrainDispatcherAsync();
        Assert.Equal(1, first.SectionNavigationSubscriberCountForTests);
        Assert.True(first.HasFolderPickerForTests);

        view.DataContext = second;
        await DrainDispatcherAsync();
        Assert.Equal(0, first.SectionNavigationSubscriberCountForTests);
        Assert.Equal(1, second.SectionNavigationSubscriberCountForTests);
        Assert.True(second.HasFolderPickerForTests);

        await second.SelectSectionForTestsAsync("directories");
        await DrainDispatcherAsync();
        Assert.Equal(1, view.SectionOffsetCommitCountForTests);

        window.Content = null;
        await DrainDispatcherAsync();
        Assert.Equal(0, second.SectionNavigationSubscriberCountForTests);
        Assert.False(second.HasFolderPickerForTests);

        window.Content = view;
        await DrainDispatcherAsync();
        Assert.Equal(1, second.SectionNavigationSubscriberCountForTests);
        Assert.True(second.HasFolderPickerForTests);

        window.Content = null;
        window.Close();
        await DrainDispatcherAsync();
        return true;
        }, CancellationToken.None);
    }

    [Fact]
    public async Task SettingsView_DetachedInstanceDoesNotStayAliveThroughViewModelEvents()
    {
        WeakReference weakView;
        using (var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest))
        {
            weakView = await session.Dispatch(CreateDetachedViewAsync, CancellationToken.None);
        }
        for (var attempt = 0; attempt < 8 && weakView.IsAlive; attempt++)
        {
            GC.Collect();
            GC.WaitForPendingFinalizers();
            GC.Collect();
        }

        Assert.False(weakView.IsAlive);
    }

    private static async Task<WeakReference> CreateDetachedViewAsync()
    {
        var window = new Window
        {
            Width = 1200,
            Height = 800,
        };
        var view = new SettingsPageView { DataContext = NewViewModel() };
        window.Content = view;
        window.Show();
        await DrainDispatcherAsync();
        window.Content = null;
        window.Close();
        await DrainDispatcherAsync();
        return new WeakReference(view);
    }

    private static async Task DrainDispatcherAsync()
    {
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.SystemIdle);
    }

    private static SettingsPageViewModel NewViewModel() =>
        new(DisplayNameService.LoadDefault(), NoopBackend.Create());

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args) =>
            throw new NotSupportedException(targetMethod?.Name);
    }
}

[CollectionDefinition("AvaloniaHeadless", DisableParallelization = true)]
public sealed class AvaloniaHeadlessCollection
{
}
