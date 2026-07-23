using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Threading;
using Ariadne.Desktop.Controls;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

[Collection("AvaloniaHeadless")]
public sealed class ControlResourceLifecycleTests
{
    [Fact]
    public async Task ImageControls_DoNotRecreateBitmapsWhileDetached()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var logo = new BrandLogo();
            var art = new EmptyStateArt { Kind = "Workspace" };
            var host = new StackPanel { Children = { logo, art } };
            var window = new Window { Width = 720, Height = 480, Content = host };

            window.Show();
            await DrainDispatcherAsync();
            Assert.True(logo.IsAttachedForTests);
            Assert.True(logo.HasRenderedImageForTests);
            Assert.True(art.IsAttachedForTests);
            Assert.True(art.HasRenderedImageForTests);

            window.Content = null;
            await DrainDispatcherAsync();
            Assert.False(logo.IsAttachedForTests);
            Assert.False(logo.HasRenderedImageForTests);
            Assert.False(art.IsAttachedForTests);
            Assert.False(art.HasRenderedImageForTests);

            logo.OnAccent = true;
            art.Kind = "GitRag";
            AppIconPainter.NotifyIconColorsChanged();
            await DrainDispatcherAsync();
            Assert.False(logo.HasRenderedImageForTests);
            Assert.False(art.HasRenderedImageForTests);

            window.Content = host;
            await DrainDispatcherAsync();
            Assert.True(logo.HasRenderedImageForTests);
            Assert.True(art.HasRenderedImageForTests);

            window.Content = null;
            window.Close();
            await DrainDispatcherAsync();
            return true;
        }, CancellationToken.None);
    }

    [Fact]
    public async Task ConfirmDialog_RecomputesConstraintsWhenHostResizes()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var dialog = new ConfirmDialogView
            {
                DataContext = new ConfirmDialogViewModel(
                    "title",
                    "message",
                    new[] { new DialogButton("ok", DialogButtonVariant.Primary, 0) }),
            };
            var window = new Window { Width = 800, Height = 700, Content = dialog };

            window.Show();
            await DrainDispatcherAsync();
            var initialMaxHeight = dialog.DialogMaxHeightForTests;
            Assert.Equal(560, dialog.DialogMaxWidthForTests);

            window.Width = 320;
            window.Height = 360;
            await DrainDispatcherAsync();
            Assert.Equal(280, dialog.DialogMaxWidthForTests);
            Assert.Equal(312, dialog.DialogMaxHeightForTests);
            Assert.True(dialog.DialogMaxHeightForTests < initialMaxHeight);

            window.Content = null;
            window.Close();
            await DrainDispatcherAsync();
            return true;
        }, CancellationToken.None);
    }

    private static async Task DrainDispatcherAsync()
    {
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.SystemIdle);
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }
}
