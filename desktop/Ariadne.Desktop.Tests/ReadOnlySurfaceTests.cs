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

/// <summary>
/// U135：只读内容不得由 `TextBox` 承载。
///
/// 缺陷形态：服务 ID 是后端分配的、用户永远改不了的标识，却用
/// `TextBox IsReadOnly="True"` 承载。而主题里**没有任何 `:readonly` 伪类样式**
/// （全仓 grep `readonly` 在 AriadneTheme.axaml 为 0 命中），于是它与可编辑输入框
/// 像素级同款：`Focus()` 返回 true、进 `:focus` 伪类、边框从 Transparent 变成
/// 2px 强调色描边、`IsTabStop` 仍为 true 照样占 Tab 停靠位——
/// 用户点一下会看到它像输入框一样亮起并抢走焦点，但一个字也打不进去。
///
/// ⚠️ **判据刻意落在「只读内容还能不能抢焦点」，而不是「用了什么控件」。**
/// 断言「不是 TextBox」是标记层面的检查：下一个人换成
/// `TextBox IsHitTestVisible=False` 之类的写法照样能过，而用户体验一样坏；
/// 反过来，任何真正解决了「假控件」问题的实现都必然满足「不抢焦点」。
/// 所以这里在**实体化的视觉树**上量 Focusable / IsTabStop / Focus() 的真实返回值。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class ReadOnlySurfaceTests
{
    /// <summary>
    /// **U135 主用例**：服务 ID 展示控件不参与焦点，`Focus()` 必须失败。
    ///
    /// 三条一起断言是因为它们各自可以被单独绕过：
    /// `IsTabStop=False` 只挡键盘 Tab，鼠标点击照样能聚焦；
    /// `Focusable=False` 才是真正的「退出焦点系统」。
    /// 最后用 `Focus()` 的返回值收口——那是运行时的最终裁判，
    /// 而不是我们对两个属性语义的推断。
    /// </summary>
    [Fact]
    public async Task ProviderIdDisplay_DoesNotTakeFocusOrTabStop()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var (window, view) = await OpenSettingsAsync();
            try
            {
                var display = FindProviderIdDisplay(view);
                Assert.NotNull(display);

                Assert.False(
                    display!.Focusable,
                    "只读展示必须退出焦点系统：Focusable=true 时鼠标一点就把焦点抢走，"
                    + "而用户在那里什么也打不进去");
                Assert.False(
                    display.IsTabStop,
                    "只读展示不该占 Tab 停靠位：键盘用户逐个 Tab 过去会停在一个打不了字的格子上");
                Assert.False(
                    display.Focus(),
                    "Focus() 必须失败——这是运行时的最终裁判，"
                    + "两个属性设对了但控件仍能聚焦的情况必须被拦住");
                Assert.False(display.IsFocused);
            }
            finally
            {
                window.Close();
                await DrainAsync();
            }

            return true;
        }, CancellationToken.None);
    }

    /// <summary>
    /// 只读展示仍必须**可取值**——去输入框化不能顺手把「复制出去用」这件事一起砍掉。
    ///
    /// 判据落在「屏幕上真的渲染出了那个 ID」，取值直接读控件的 Text：
    /// 断言 ViewModel 属性等于 ID 在缺陷版本下也是真（那时它也显示着），
    /// 证明不了展示这一环没坏。
    /// </summary>
    [Fact]
    public async Task ProviderIdDisplay_StillRendersTheIdAndSupportsSelection()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var (window, view) = await OpenSettingsAsync("provider-under-test");
            try
            {
                var display = FindProviderIdDisplay(view);
                Assert.NotNull(display);
                Assert.Equal("provider-under-test", display!.Text);

                // SelectableTextBlock：能选中就能 Ctrl+C，只读值的取值诉求不靠假输入框满足。
                Assert.IsAssignableFrom<SelectableTextBlock>(display);
            }
            finally
            {
                window.Close();
                await DrainAsync();
            }

            return true;
        }, CancellationToken.None);
    }

    /// <summary>
    /// 全页普查：**设置页里不允许存在任何能抢焦点的只读 `TextBox`**。
    ///
    /// 单点用例只钉住了服务 ID 这一处。这条是防回归的护栏——
    /// 下一个人再往设置页塞一个 `TextBox IsReadOnly="True"` 时立刻转红，
    /// 而不用等到有人肉眼发现「这一屏又变成一堆假输入框了」。
    /// </summary>
    [Fact]
    public async Task SettingsPage_HasNoFocusableReadOnlyTextBox()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var (window, view) = await OpenSettingsAsync();
            try
            {
                var offenders = view.GetVisualDescendants()
                    .OfType<TextBox>()
                    .Where(box => box.IsReadOnly && (box.Focusable || box.IsTabStop))
                    .Select(box => box.Name ?? box.Text ?? "<未命名>")
                    .ToList();

                Assert.True(
                    offenders.Count == 0,
                    "只读内容不得用可聚焦的 TextBox 承载，违规控件："
                    + string.Join(", ", offenders));
            }
            finally
            {
                window.Close();
                await DrainAsync();
            }

            return true;
        }, CancellationToken.None);
    }

    /// <summary>
    /// 打开设置页并把「高级」折叠面板展开——服务 ID 在 Expander 里，
    /// 不展开就根本不会实体化，测出来的「找不到控件」是假阴性。
    /// </summary>
    private static async Task<(Window Window, SettingsPageView View)> OpenSettingsAsync(
        string providerId = "openai")
    {
        var viewModel = new SettingsPageViewModel(DisplayNameService.LoadDefault(), NoopBackend.Create());
        viewModel.ApplyProviderConfigForTests(new ProviderConfigStatus(
            HasOpenAiKey: true,
            HasAnthropicKey: false,
            HasGeminiKey: false,
            DefaultLlmProviderId: providerId,
            DefaultEmbeddingProviderId: null,
            DefaultRerankerProviderId: null,
            DefaultSearchProviderId: null,
            Providers: new[]
            {
                new ProviderKeyStatus(
                    providerId,
                    "受测服务",
                    "open_ai",
                    Configured: true,
                    Enabled: true,
                    BaseUrl: null,
                    Models: Array.Empty<ModelConfig>(),
                    HasKey: true),
            }));

        // 设置页是分页的：未选中的分页内容在视觉树里压根不实例化。
        // ProviderId 在「模型」分页里，不切过去就一个 SelectableTextBlock 都找不到。
        viewModel.SelectTabForTests("models");

        var view = new SettingsPageView { DataContext = viewModel };
        var window = new Window { Width = 1280, Height = 900, Content = view };
        window.Show();
        await DrainAsync();

        foreach (var expander in view.GetVisualDescendants().OfType<Expander>())
        {
            expander.IsExpanded = true;
        }

        await DrainAsync();
        return (window, view);
    }

    private static SelectableTextBlock? FindProviderIdDisplay(SettingsPageView view) =>
        view.GetVisualDescendants()
            .OfType<SelectableTextBlock>()
            .FirstOrDefault(block => block.Name == "ProviderIdDisplay");

    private static async Task DrainAsync()
    {
        for (var i = 0; i < 12; i++)
        {
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Background);
        }

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
