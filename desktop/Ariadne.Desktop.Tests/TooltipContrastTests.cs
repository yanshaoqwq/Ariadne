using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Presenters;
using Avalonia.Headless;
using Avalonia.Media;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop;
using Xunit;

namespace Ariadne.Desktop.Tests;

[Collection("AvaloniaHeadless")]
public sealed class TooltipContrastTests
{
    // 回归：全局 TextBlock 默认色（TextPrimary 深色）曾盖掉 ToolTip 继承下来的反白，
    // 深底胶囊上变成黑字看不清。现在 ToolTip 内文字必须走 TooltipText（与 ToolTip.Foreground 一致）。
    [Fact]
    public async Task TooltipInnerTextUsesTooltipForeground_NotGlobalTextPrimary()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(Builder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var host = new Button { Content = "hover me", Width = 120, Height = 40 };
            ToolTip.SetTip(host, "示例悬浮提示文本");
            var window = new Window { Width = 400, Height = 300, Content = host };
            window.Show();
            await DrainAsync();

            ToolTip.SetIsOpen(host, true);
            await DrainAsync();

            var tooltip = window.GetVisualDescendants().OfType<ToolTip>().FirstOrDefault();
            Assert.NotNull(tooltip);

            var text = tooltip.GetVisualDescendants().OfType<TextBlock>().FirstOrDefault();
            Assert.NotNull(text);

            var expected = (tooltip.Foreground as ISolidColorBrush)?.Color;
            var actual = (text.Foreground as ISolidColorBrush)?.Color;
            Assert.NotNull(expected);
            Assert.Equal(expected, actual);
            return true;
        }, CancellationToken.None);
    }

    private static async Task DrainAsync()
    {
        for (var i = 0; i < 12; i++)
        {
            Dispatcher.UIThread.RunJobs(DispatcherPriority.Render);
            await Task.Delay(5);
        }
    }

    private static class Builder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }
}
