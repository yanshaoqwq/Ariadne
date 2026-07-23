using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Input;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

[Collection("AvaloniaHeadless")]
public sealed class WorkspaceInteractionTests
{
    [Fact]
    public async Task NodeLibraryChip_DragsThroughHandledButtonRoute_AndDropsOnCanvas()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var viewModel = new WorkspacePageViewModel(
                DisplayNameService.LoadDefault(),
                DispatchProxy.Create<IAriadneBackendClient, SoftBackendProxy>());
            var view = new WorkspacePageView { DataContext = viewModel };
            var window = new Window
            {
                Width = 1200,
                Height = 800,
                Content = view,
            };

            window.Show();
            await DrainDispatcherAsync();

            var libraryButton = view
                .GetVisualDescendants()
                .OfType<Button>()
                .First(control => control.DataContext is NodeLibraryItemViewModel { NodeType: "start" });
            var canvas = view.FindControl<Canvas>("CanvasOverlay");
            Assert.NotNull(canvas);

            var press = libraryButton.TranslatePoint(
                new Point(libraryButton.Bounds.Width / 2, libraryButton.Bounds.Height / 2),
                window);
            var drop = canvas!.TranslatePoint(
                new Point(canvas.Bounds.Width / 2, canvas.Bounds.Height / 2),
                window);
            Assert.NotNull(press);
            Assert.NotNull(drop);
            Assert.True(
                press!.Value.X >= 0 && press.Value.X <= window.ClientSize.Width
                    && press.Value.Y >= 0 && press.Value.Y <= window.ClientSize.Height,
                $"library chip translated outside client: {press.Value}, client={window.ClientSize}, "
                    + $"view={view.Bounds}/{view.DesiredSize}, "
                    + $"workspace={view.FindControl<Grid>("WorkspaceGrid")?.Bounds}/{view.FindControl<Grid>("WorkspaceGrid")?.DesiredSize}, "
                    + $"canvas={canvas.Bounds}/{canvas.DesiredSize}, "
                    + $"library={view.FindControl<Border>("LibraryContent")?.Bounds}/{view.FindControl<Border>("LibraryContent")?.DesiredSize}, "
                    + $"button={libraryButton.Bounds}/{libraryButton.DesiredSize}");
            var hit = window.InputHitTest(press!.Value);
            Assert.NotNull(hit);
            Assert.True(
                (hit as Visual)?.GetSelfAndVisualAncestors().Contains(libraryButton) == true,
                $"library press hit {hit?.GetType().Name ?? "null"} instead of the chip");

            window.MouseDown(press.Value, MouseButton.Left, RawInputModifiers.None);
            window.MouseMove(drop!.Value, RawInputModifiers.LeftMouseButton);
            window.MouseUp(drop.Value, MouseButton.Left, RawInputModifiers.None);
            await DrainDispatcherAsync();

            var node = Assert.Single(viewModel.Nodes);
            Assert.Equal("start", node.NodeType);
            Assert.InRange(node.X, 250, 950);
            Assert.InRange(node.Y, 50, 550);

            window.Content = null;
            window.Close();
            await DrainDispatcherAsync();
            return true;
        }, CancellationToken.None);
    }

    private static async Task DrainDispatcherAsync()
    {
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Render);
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.SystemIdle);
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    private class SoftBackendProxy : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }

            if (targetMethod?.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }

            if (targetMethod?.ReturnType.IsGenericType == true
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                var value = resultType.IsValueType ? Activator.CreateInstance(resultType) : null;
                return typeof(Task)
                    .GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }

            return targetMethod?.ReturnType.IsValueType == true
                ? Activator.CreateInstance(targetMethod.ReturnType)
                : null;
        }
    }
}
