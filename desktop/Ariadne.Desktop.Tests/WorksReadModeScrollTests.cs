using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Primitives;
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
/// U128：作品页阅读模式的正文必须能滚动。
///
/// 缺陷：承载正文的 `ItemsControl` 既无祖先也无后代 `ScrollViewer`，而裸
/// `ItemsControl` 在 Avalonia 里**不自带滚动**——`ItemsPresenter` 不是 `IScrollable`，
/// 内部的 `VirtualizingStackPanel` 也不实现 `ILogicalScrollable`。于是长篇正文
/// 只能看到第一屏，末块永远不可达：**产品的主用途（读自己写的小说）不成立**。
///
/// 这里用真实视觉树实测，而不是只比对标记——「有没有 ScrollViewer」是标记问题，
/// 但「滚动是否真的生效」只有实体化后的坐标能回答。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class WorksReadModeScrollTests
{
    /// <summary>阅读模式必须有一个可寻址的滚动容器，且内容确实超出视口。</summary>
    [Fact]
    public async Task ReadMode_HasScrollViewer_WithContentTallerThanViewport()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var (window, view, _) = await OpenLongDocumentInReadModeAsync();
            try
            {
                var scroll = FindReaderScroll(view);

                Assert.NotNull(scroll);
                Assert.True(
                    scroll!.Extent.Height > scroll.Viewport.Height,
                    $"正文内容高度 {scroll.Extent.Height} 必须超过视口 {scroll.Viewport.Height}，" +
                    "否则这条用例根本没在测滚动");
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
    /// **U128 主用例**：向下滚动后，实体化的正文块必须换成后面的块。
    ///
    /// 判据取「实体化了哪些块」而不是「Offset 变了没有」——`Offset` 是
    /// `ScrollViewer` 自己的属性，就算下面的 `ItemsControl` 完全不响应也能被赋值。
    /// 只有实体化集合真的前移，才证明用户看到的内容变了。
    /// </summary>
    [Fact]
    public async Task ReadMode_ScrollingDown_RealizesLaterBlocks()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var (window, view, viewModel) = await OpenLongDocumentInReadModeAsync();
            try
            {
                var scroll = FindReaderScroll(view);
                Assert.NotNull(scroll);

                var before = RealizedBlockIndexes(view);
                Assert.NotEmpty(before);
                Assert.Contains(0, before);

                // 滚到底：末块必须可达——这正是修复前做不到的事。
                scroll!.Offset = new Vector(0, scroll.Extent.Height - scroll.Viewport.Height);
                await DrainAsync();

                var after = RealizedBlockIndexes(view);
                Assert.NotEmpty(after);
                Assert.True(
                    after.Min() > before.Max(),
                    $"滚到底后实体化块仍与初始重叠（before={string.Join(",", before)} " +
                    $"after={string.Join(",", after)}）——正文没有真的滚动");
                Assert.Contains(viewModel.DocumentBlocks.Count - 1, after);
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
    /// 切到编辑模式后，阅读容器必须让位——两个视图不能同时占位。
    ///
    /// 顺带守住一件事：修复引入的 `ScrollViewer` 挂的是同一个 `!IsEditMode`，
    /// 若漏挂，编辑模式下会有一个不可见但仍占布局的滚动容器压住编辑器。
    ///
    /// 只断言可见性、**不实体化编辑器**：headless 下 AvaloniaEdit 的
    /// `TextEditor` 一旦进入布局就会挂起，那是测试宿主的限制，与本条契约无关。
    /// </summary>
    [Fact]
    public async Task EditMode_HidesReaderScroll()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);
        await session.Dispatch(async () =>
        {
            var (window, view, viewModel) = await OpenLongDocumentInReadModeAsync();
            try
            {
                var scroll = FindReaderScroll(view);
                Assert.NotNull(scroll);
                Assert.True(scroll!.IsVisible);

                viewModel.IsEditMode = true;
                await DrainAsync();

                Assert.False(scroll.IsVisible);
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
    /// U129 的前置数据：每个块必须带正确的起始字符偏移。
    ///
    /// 「切换视图保留阅读位置」需要在块索引与编辑器字符偏移之间换算，
    /// 而块粒度是数千字符——没有偏移就只能做块级对齐，差 8–12 屏。
    /// </summary>
    [Fact]
    public void DocumentBlocks_CarryContiguousStartOffsets()
    {
        var viewModel = NewViewModel();
        var content = BuildLongDocument();
        viewModel.SeedOpenDocumentForTests("documents/long.md", "v1", content);
        viewModel.IsEditMode = false;

        Assert.True(viewModel.DocumentBlocks.Count > 1, "正文必须被切成多块，否则这条用例无意义");
        Assert.Equal(0, viewModel.DocumentBlocks[0].StartOffset);
        for (var i = 0; i < viewModel.DocumentBlocks.Count; i++)
        {
            var block = viewModel.DocumentBlocks[i];
            Assert.Equal(
                block.Text,
                content.Substring(block.StartOffset, block.Text.Length));
            if (i > 0)
            {
                // 半开区间必须首尾相接：有空隙就意味着有正文既不属于前块也不属于后块。
                Assert.Equal(viewModel.DocumentBlocks[i - 1].EndOffset, block.StartOffset);
            }
        }
        Assert.Equal(content.Length, viewModel.DocumentBlocks[^1].EndOffset);
    }

    private static async Task<(Window Window, WorksPageView View, WorksPageViewModel ViewModel)>
        OpenLongDocumentInReadModeAsync()
    {
        // 关键顺序：**先**把 ViewModel 置成「树已加载 + 已打开文档 + 阅读模式」，
        // **再**挂进窗口。这样首次布局的绑定求值就能读到 ShowDocumentChrome=true，
        // 不需要任何 PropertyChanged，也不用触碰会拉起后端异步加载的
        // OnCurrentDocumentChanged / SeedOpenDocumentForTests（后者内部还会
        // 先 `IsEditMode = true` 把 AvaloniaEdit 的 TextEditor 拖进布局）。
        var viewModel = NewViewModel();
        var type = typeof(WorksPageViewModel);
        type.GetField("_currentDocumentId", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(viewModel, "chapters/chapter-01.md");
        SetWorksTreeStateToContent(viewModel);
        viewModel.DocumentContent = BuildLongDocument();
        viewModel.IsEditMode = false;

        var view = new WorksPageView { DataContext = viewModel };
        var window = new Window { Width = 1400, Height = 900, Content = view };
        window.Show();
        await DrainAsync();

        return (window, view, viewModel);
    }

    /// <summary>把作品树状态置为 Content，让文档区真正参与布局。</summary>
    private static void SetWorksTreeStateToContent(WorksPageViewModel viewModel)
    {
        var type = typeof(WorksPageViewModel);
        var stateType = type.GetNestedType("WorksTreeLoadState", BindingFlags.NonPublic)
                        ?? type.Assembly.GetTypes().First(candidate => candidate.Name == "WorksTreeLoadState");
        type.GetField("_worksTreeState", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(viewModel, Enum.Parse(stateType, "Content"));
    }

    /// <summary>
    /// 约 3.6 万字符的正文——按 4000–6000 字符的块粒度切成 7–9 块，
    /// 与审查报告里实测的规模一致（9 块）。
    ///
    /// 刻意不用 48 万字符：headless 下真实文本测量是同步的，
    /// 上百个块会让单条用例跑到分钟级，而「能不能滚」这件事 9 块就足以证明。
    /// </summary>
    private static string BuildLongDocument() =>
        string.Join(
            "\n\n",
            Enumerable.Range(0, 60).Select(i => $"第{i}段：" + new string('文', 600)));

    private static ScrollViewer? FindReaderScroll(WorksPageView view) =>
        view.GetVisualDescendants()
            .OfType<ScrollViewer>()
            .FirstOrDefault(scroll => scroll.Name == "DocumentReaderScroll");

    /// <summary>当前实体化的正文块索引（按块文本的段号前缀反解）。</summary>
    private static List<int> RealizedBlockIndexes(WorksPageView view) =>
        view.GetVisualDescendants()
            .OfType<SelectableTextBlock>()
            .Where(block => block.Classes.Contains("reading"))
            .Select(block => block.DataContext as DocumentBlockViewModel)
            .Where(block => block is not null)
            .Select(block => block!.Index)
            .OrderBy(index => index)
            .ToList();

    private static WorksPageViewModel NewViewModel() =>
        new(DisplayNameService.LoadDefault(), NoopBackend.Create());

    private static async Task DrainAsync()
    {
        for (var i = 0; i < 16; i++)
        {
            await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Background);
        }
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() => Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == $"get_{nameof(IAriadneBackendClient.HasProjectRoot)}")
            {
                return false;
            }

            var returnType = targetMethod?.ReturnType;
            if (returnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (returnType is not null
                && returnType.IsGenericType
                && returnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var inner = returnType.GetGenericArguments()[0];
                var value = inner.IsValueType ? Activator.CreateInstance(inner) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(inner)
                    .Invoke(null, new[] { value });
            }
            return null;
        }
    }
}
