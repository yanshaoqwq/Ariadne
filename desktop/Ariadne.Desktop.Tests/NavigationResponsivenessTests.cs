using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U178-A：点侧栏之后界面必须**立刻**有反应。
///
/// 原缺陷：`SelectNavigationItemAsync` 先 `await EnsurePageLoadedAsync`、后 `CommitNavigation`，
/// 于是点「配置」到整页出现之间屏幕完全静止（配置页 `LoadAsync` 一次并发发 14+ 次 IPC），
/// 连被按下的那个导航项都不高亮——`IsSelected` 也是在 `CommitNavigation` 里刷的。
/// 用户读到的信号是「没点上」。
///
/// 判据取「**页面加载仍在途时**，CurrentPage / IsSelected 是否已经是目标页」，
/// 而不是「导航结束后是否停在目标页」——后者在缺陷版本里同样全绿。
/// </summary>
public sealed class NavigationResponsivenessTests
{
    [Fact]
    public async Task Navigation_CommitsTargetPageBeforePageDataFinishesLoading()
    {
        var workspace = ControlledPage.Blocked();
        var window = CreateWindow(_ => workspace, out _);

        var pending = window.OpenNavigationItemByIdAsync("workspace");
        await workspace.Started.Task;

        // 加载确实还在途：这保证下面两条断言测的是「等待期间的界面」，不是「等完之后」。
        Assert.False(workspace.LoadCompleted);
        Assert.Same(workspace, window.CurrentPage);
        Assert.Equal("workspace", window.SelectedNavigationIdForTests);

        workspace.Release();
        await pending;
        Assert.Same(workspace, window.CurrentPage);
    }

    /// <summary>
    /// 连点三页：最终必须停在**最后点的那一页**。
    ///
    /// 这条钉的是「提前 commit 之后代际校验仍在」。提前 commit 让每一次点击都立刻
    /// 把自己的页挂上去，于是三条加载在途；`IsCurrent` 是唯一阻止 A/B 的迟到完成
    /// 继续往下写可见状态的闸。摘掉它就会出现「点了 B 又点 C，最后停在 B」。
    ///
    /// 释放顺序刻意是 A → B（都晚于 C 完成），复现真实场景里「慢的页最后回来」。
    /// </summary>
    [Fact]
    public async Task RapidNavigation_AcrossThreePages_SettlesOnTheLastRequestedPage()
    {
        var pageA = ControlledPage.Blocked();
        var pageB = ControlledPage.Blocked();
        var pageC = ControlledPage.Completed();
        var window = CreateWindow(
            id => id switch
            {
                "workspace" => pageA,
                "works" => pageB,
                _ => pageC,
            },
            out var readSavedNavigationId);

        var navA = window.OpenNavigationItemByIdAsync("workspace");
        await pageA.Started.Task;
        var navB = window.OpenNavigationItemByIdAsync("works");
        await pageB.Started.Task;
        await window.OpenNavigationItemByIdAsync("git");

        Assert.Same(pageC, window.CurrentPage);
        Assert.Equal("git", window.SelectedNavigationIdForTests);

        // 过期请求现在才回来：它们既不能改页，也不能改选中项与落盘的导航 id。
        pageA.Release();
        pageB.Release();
        await navA;
        await navB;

        Assert.Same(pageC, window.CurrentPage);
        Assert.Equal("git", window.SelectedNavigationIdForTests);
        Assert.Equal("git", window.LastNavigationIdForTests);
        Assert.Equal("git", readSavedNavigationId());
    }

    /// <summary>
    /// 加载失败的页**不能**被记成「上次访问的页」。
    ///
    /// 提前 commit 把 `CommitNavigation` 拆成了两段：可见状态立刻提交、落盘的导航 id
    /// 仍等加载成功。若把失败的页也落盘，下次启动会去恢复一个打不开的页、
    /// 再弹回开始页——一个坏页会变成粘性状态。
    /// </summary>
    [Fact]
    public async Task FailedPageLoad_DoesNotPersistNavigationId()
    {
        var workspace = ControlledPage.Blocked();
        var window = CreateWindow(_ => workspace, out var readSavedNavigationId);

        var pending = window.OpenNavigationItemByIdAsync("workspace");
        await workspace.Started.Task;
        workspace.Fail(new IOException("page load failed"));
        await pending;

        Assert.Same(window.Welcome, window.CurrentPage);
        Assert.Null(readSavedNavigationId());
    }

    /// <summary>
    /// 等待期间导航项要有「在读」指示，且**任何出口**都要清掉它。
    ///
    /// 判据取「加载在途时 IsPending 为真、加载结束后为假」。
    /// 只断言「置真」不够：清理漏在某条出口上时，用户看到的是一个永远在读的侧栏项，
    /// 而那条用例照样全绿。
    /// </summary>
    [Fact]
    public async Task PendingIndicator_IsOnWhileLoading_AndClearedOnEveryExit()
    {
        var workspace = ControlledPage.Blocked();
        var window = CreateWindow(_ => workspace, out _);
        var navItem = window.PrimaryNavigationItems.Single(item => item.Id == "workspace");

        Assert.False(navItem.IsPending);
        var pending = window.OpenNavigationItemByIdAsync("workspace");
        await workspace.Started.Task;

        Assert.True(navItem.IsPending);

        workspace.Release();
        await pending;

        Assert.False(navItem.IsPending);
    }

    /// <summary>失败出口同样要清 pending，否则一次加载失败留下永久转圈的侧栏项。</summary>
    [Fact]
    public async Task PendingIndicator_IsClearedWhenPageLoadFails()
    {
        var workspace = ControlledPage.Blocked();
        var window = CreateWindow(_ => workspace, out _);
        var navItem = window.PrimaryNavigationItems.Single(item => item.Id == "workspace");

        var pending = window.OpenNavigationItemByIdAsync("workspace");
        await workspace.Started.Task;
        Assert.True(navItem.IsPending);

        workspace.Fail(new IOException("page load failed"));
        await pending;

        Assert.False(navItem.IsPending);
    }

    /// <summary>
    /// U178-A：`IsLoading` 必须在**界面上**有消费点。
    ///
    /// 缺陷形态是 U111「为不存在的执行路径装开关」的镜像：状态位在算但显示端不接。
    /// 它测不出来——`IsLoading` 的单元测试全绿，因为它确实被正确地设了又清。
    /// 唯一能拦住的判据就是「.axaml 里有没有引用」，所以这条用例读源码文本。
    ///
    /// 断言的是 `ShowLoadingSkeleton`（真正的绑定目标）而非 `IsLoading`：
    /// 骨架只在首次加载铺，「取消改动」那条路径不该抽走已显示的内容。
    /// </summary>
    [Fact]
    public void SettingsPage_LoadingStateHasAViewConsumptionPoint()
    {
        var xaml = File.ReadAllText(Path.Combine(
            ResolveRepoRoot(), "desktop", "Ariadne.Desktop", "Views", "SettingsPageView.axaml"));

        Assert.Contains("Binding ShowLoadingSkeleton", xaml, StringComparison.Ordinal);
        Assert.Contains("Binding LoadingText", xaml, StringComparison.Ordinal);
    }

    /// <summary>
    /// 切页过渡必须挂在 `TransitioningContentControl` 上，且时长与既有尺度一致。
    ///
    /// 判据取「过渡真的挂上了」而不是「XAML 里有 CrossFade 字样」：
    /// PageTransition 由 code-behind 按 ReduceMotion 挂/摘，写死在 XAML 里
    /// 会让「减少动效」在切页这条最显眼的路径上失效。
    /// 150ms 落在全仓 74 处过渡的 120–180ms 区间内——同一产品里过渡应是同一种节奏。
    /// </summary>
    [Fact]
    public void MainWindow_PageHostUsesTransitioningContentControlWithProjectScaleDuration()
    {
        var root = ResolveRepoRoot();
        var xaml = File.ReadAllText(Path.Combine(
            root, "desktop", "Ariadne.Desktop", "Views", "MainWindow.axaml"));
        var codeBehind = File.ReadAllText(Path.Combine(
            root, "desktop", "Ariadne.Desktop", "Views", "MainWindow.axaml.cs"));

        Assert.Contains("<TransitioningContentControl x:Name=\"PageHost\"", xaml, StringComparison.Ordinal);
        Assert.Contains("PageHost.PageTransition", codeBehind, StringComparison.Ordinal);
        // ReduceMotion 下必须置 null（不是 0 时长）：null 走同步换内容的快路径。
        Assert.Contains("MotionPreferences.ReduceMotion", codeBehind, StringComparison.Ordinal);

        var duration = System.Text.RegularExpressions.Regex.Match(
            codeBehind,
            @"new CrossFade\(TimeSpan\.FromMilliseconds\((\d+)\)\)");
        Assert.True(duration.Success, "PageHost 的 CrossFade 时长应显式以毫秒给出");
        var ms = int.Parse(duration.Groups[1].Value);
        Assert.InRange(ms, 120, 180);
    }

    private static string ResolveRepoRoot()
    {
        var path = Path.GetDirectoryName(typeof(NavigationResponsivenessTests).Assembly.Location)!;
        while (!string.IsNullOrEmpty(path)
               && !File.Exists(Path.Combine(path, "desktop", "Ariadne.slnx")))
        {
            path = Directory.GetParent(path)?.FullName ?? string.Empty;
        }
        return path;
    }

    private static MainWindowViewModel CreateWindow(
        Func<string, object?> pageFactory,
        out Func<string?> readSavedNavigationId)
    {
        string? saved = null;
        readSavedNavigationId = () => saved;
        return new MainWindowViewModel(
            DisplayNameService.LoadDefault(),
            NoopBackend.Create(),
            pageFactory,
            id => saved = id);
    }

    /// <summary>可控页面：加载在 <see cref="Release"/> / <see cref="Fail"/> 之前一直挂着。</summary>
    private sealed class ControlledPage : IProjectDataReloadable
    {
        private readonly TaskCompletionSource<bool> _completion =
            new(TaskCreationOptions.RunContinuationsAsynchronously);

        private ControlledPage(bool completed)
        {
            if (completed)
            {
                _completion.TrySetResult(true);
            }
        }

        public TaskCompletionSource<bool> Started { get; } =
            new(TaskCreationOptions.RunContinuationsAsynchronously);

        public bool LoadCompleted { get; private set; }

        public static ControlledPage Blocked() => new(false);

        public static ControlledPage Completed() => new(true);

        public async Task ReloadProjectDataAsync(CancellationToken cancellationToken = default)
        {
            Started.TrySetResult(true);
            // 故意忽略取消：要证明旧 I/O 即使晚完成也不能改写已提交的可见状态。
            await _completion.Task.ConfigureAwait(false);
            LoadCompleted = true;
        }

        public void DeactivateProjectData()
        {
        }

        public void Release() => _completion.TrySetResult(true);

        public void Fail(Exception error) => _completion.TrySetException(error);
    }

    /// <summary>后端在本组用例里只需「不抛异常」；导航不依赖它的返回值。</summary>
    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            if (targetMethod.Name == "get_HasProjectRoot")
            {
                return false;
            }

            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                var value = resultType.IsValueType ? Activator.CreateInstance(resultType) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object?[] { value });
            }

            return targetMethod.ReturnType.IsValueType
                ? Activator.CreateInstance(targetMethod.ReturnType)
                : null;
        }
    }
}
