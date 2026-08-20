using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U207-G：版本页空态里那颗「创建存档」按钮。
///
/// # 原缺陷
///
/// 空态文案是「还没有存档 / 写完一章或想留个保险时，**在这里存一下**，以后能回看。」
/// 而唯一那颗「创建存档」按钮在**右栏**里，右栏又可以是折叠的
/// （偏好项 <c>git.right_panel</c> 记住上次状态，页面最右一个 `‹` pill 才能展开）。
/// ⇒ 空态指了路，路的尽头是折叠面板，作者第一次来必然找不到。
///
/// 与 U182-L（13 个空态里只有 3 个给出下一步）同源，但本条更隐蔽：
/// **给了下一步，而下一步不可达**。屏上看起来"已经引导过了"，复查时容易一眼放过。
///
/// # 判据落在哪里
///
/// 一律落在「**那颗按钮存在、可点、且点了真的落盘**」上，不落在文案上：
/// 文案一个字都不用改本条就能修好，反过来只改文案则一点问题都没解决。
/// 尤其要断言它走的是**同一条落盘链路**（<c>CreateCheckpointAsync</c>）——
/// 「空态有个按钮但点了没用」比没按钮更糟。
/// </summary>
public sealed class GitEmptyStateCheckpointActionTests
{
    [Fact]
    public async Task EmptyProject_OffersACreateCheckpointActionThatActuallyArchives()
    {
        var backend = GitBackend.Create();
        backend.Graph = Array.Empty<BranchGraphNode>();
        var viewModel = NewViewModel(backend);

        await viewModel.ReloadProjectDataAsync();

        // 前置：确实处在「项目里还没有存档」这一种空态。
        Assert.True(viewModel.ShowEmpty);
        Assert.True(viewModel.IsCommitListEmpty);
        Assert.False(viewModel.IsError);

        // ① 按钮存在。
        Assert.True(
            viewModel.ShowCreateCheckpointAction,
            "空态没有给出「创建存档」这个下一步");
        // ② 按钮可点（不是一颗灰的/点了没反应的）。
        Assert.True(viewModel.CreateFirstCheckpointCommand.CanExecute(null));
        // ③ 点了真的落盘 —— 这一条才是本用例的核心。
        Assert.Equal(0, backend.CheckpointCalls);
        Assert.True(viewModel.CreateFirstCheckpointCommand.TryExecute());
        await WaitUntilAsync(() => backend.CheckpointCalls >= 1);
        Assert.Equal(1, backend.CheckpointCalls);
        // 文案不能是缺 key 的 `[...]` 兜底形态。
        Assert.DoesNotContain(
            "[ui.git.create_checkpoint",
            viewModel.CreateCheckpointText,
            StringComparison.Ordinal);
    }

    /// <summary>
    /// 空态那颗与右栏那颗必须走**同一条落盘链路**。
    ///
    /// 「空态有个按钮但点了没用」比没按钮更糟：作者会以为自己存过了。
    /// 所以判据不是「两个命令都存在」，而是「两者产生的后端调用不可区分」——
    /// 若空态那颗被接到别的实现上（哪怕只是漏了 RefreshCoreAsync），这条就红。
    /// </summary>
    [Fact]
    public async Task EmptyStateButtonAndRightPanelButton_ShareTheSameArchivePath()
    {
        var backend = GitBackend.Create();
        backend.Graph = Array.Empty<BranchGraphNode>();
        var viewModel = NewViewModel(backend);
        await viewModel.ReloadProjectDataAsync();

        // 右栏那颗：填了说明。
        viewModel.CheckpointMessage = "右栏存的";
        Assert.True(viewModel.CreateCheckpointCommand.TryExecute());
        await WaitUntilAsync(() => backend.CheckpointCalls >= 1);
        Assert.Equal("右栏存的", backend.LastCheckpointMessage);
        // 落盘后说明清空（两颗都该有这个行为）。
        Assert.Equal(string.Empty, viewModel.CheckpointMessage);

        // 空态那颗：不填说明，走同一条链路 ⇒ 后端照样收到一次调用。
        viewModel.CheckpointMessage = "空态存的";
        Assert.True(viewModel.CreateFirstCheckpointCommand.TryExecute());
        await WaitUntilAsync(() => backend.CheckpointCalls >= 2);
        Assert.Equal("空态存的", backend.LastCheckpointMessage);
        Assert.Equal(string.Empty, viewModel.CheckpointMessage);
    }

    /// <summary>
    /// 空态那颗点完要把右栏展开。
    ///
    /// # 为什么这也是本条的一部分
    ///
    /// 存完第一档空态就消失了，而以后每一次存档、以及写说明的输入框，
    /// 都只在右栏里。不展开的话作者第二次想存档时会回到**完全相同**的困境：
    /// 知道功能存在，但找不到入口。展开右栏等于把「以后去这里」演示一遍。
    /// </summary>
    [Fact]
    public async Task ClickingTheEmptyStateAction_RevealsTheRightPanelForNextTime()
    {
        var backend = GitBackend.Create();
        backend.Graph = Array.Empty<BranchGraphNode>();
        var viewModel = NewViewModel(backend);
        await viewModel.ReloadProjectDataAsync();

        // 作者上次把右栏折叠了（偏好被记住）——这正是缺陷现场。
        viewModel.IsRightPanelOpen = false;
        Assert.False(viewModel.IsRightPanelOpen);

        Assert.True(viewModel.CreateFirstCheckpointCommand.TryExecute());
        await WaitUntilAsync(() => backend.CheckpointCalls >= 1);

        Assert.True(
            viewModel.IsRightPanelOpen,
            "存完第一档后右栏仍折叠 ⇒ 作者第二次存档还是找不到入口");
    }

    /// <summary>
    /// 没打开项目时**不给**这颗按钮。
    ///
    /// <c>ShowEmpty</c> 覆盖 Empty 与 IdleNeedProject 两种空态；
    /// 无项目时点「创建存档」只能得到一句错误 —— 那正是 U182-M 修掉的形态
    /// （宁可没有按钮，也不要一颗点了没反应的）。
    /// 所以判据必须是 <c>PageLoadState.Empty</c> 本身，不能图省事复用 <c>ShowEmpty</c>。
    /// </summary>
    [Fact]
    public async Task NoProjectOpen_HidesTheCreateCheckpointActionRatherThanOfferingADeadButton()
    {
        var backend = GitBackend.Create();
        backend.HasProjectRoot = false;
        var viewModel = NewViewModel(backend);

        await viewModel.ReloadProjectDataAsync();

        // 仍然是空态，但是**另一种**空态。
        Assert.True(viewModel.ShowEmpty);
        Assert.False(
            viewModel.ShowCreateCheckpointAction,
            "无项目时不该出现「创建存档」——点了只能得到一句错误");
        Assert.False(viewModel.CreateFirstCheckpointCommand.CanExecute(null));
        Assert.Equal(0, backend.CheckpointCalls);
    }

    /// <summary>
    /// 已有存档时不再出现这颗按钮（它是**空态**的下一步，不是常驻控件）。
    /// </summary>
    [Fact]
    public async Task WithExistingArchives_TheEmptyStateActionIsGone()
    {
        var backend = GitBackend.Create();
        backend.Graph = new[] { Node("first", "First") };
        var viewModel = NewViewModel(backend);

        await viewModel.ReloadProjectDataAsync();

        Assert.True(viewModel.ShowContent);
        Assert.False(viewModel.ShowEmpty);
        Assert.False(viewModel.ShowCreateCheckpointAction);
    }

    /// <summary>
    /// 守卫：空态那颗按钮必须**真的挂在 XAML 的空态区里**。
    ///
    /// ViewModel 上的属性和命令全对，而 XAML 里没挂这颗按钮 —— 屏幕上与没修一模一样。
    /// 本机 Avalonia headless 对控件子类有盲区（见 CLAUDE.md 记录），
    /// 而「空态 Border 内部是否存在某个绑定」是可纯文本判定的结构性质，故走源码文本断言。
    /// </summary>
    [Fact]
    public void GitPageView_PutsTheCreateCheckpointButtonInsideTheEmptyStateBlock()
    {
        var xaml = File.ReadAllText(ResolveDesktopSource("Views", "GitPageView.axaml"));

        // 绑定本身存在。
        Assert.Contains(
            "Command=\"{Binding CreateFirstCheckpointCommand}\"",
            xaml,
            StringComparison.Ordinal);
        Assert.Contains(
            "IsVisible=\"{Binding ShowCreateCheckpointAction}\"",
            xaml,
            StringComparison.Ordinal);

        // 位置判据：这颗按钮必须落在绑 ShowEmpty 的那个 Border 之内，
        // 而不是"在文件里某处存在"。挂到右栏里等于本条一点没修。
        var emptyBlockStart = xaml.IndexOf(
            "IsVisible=\"{Binding ShowEmpty}\"",
            StringComparison.Ordinal);
        Assert.True(emptyBlockStart > 0, "找不到绑 ShowEmpty 的空态区");
        var emptyBlockEnd = xaml.IndexOf("</Border>", emptyBlockStart, StringComparison.Ordinal);
        Assert.True(emptyBlockEnd > emptyBlockStart, "空态区没有闭合的 Border");
        var emptyBlock = xaml[emptyBlockStart..emptyBlockEnd];
        Assert.Contains(
            "Command=\"{Binding CreateFirstCheckpointCommand}\"",
            emptyBlock,
            StringComparison.Ordinal);

        // 右栏那颗仍在原处（本条是**新增**入口，不是搬走原入口）。
        Assert.Contains(
            "Command=\"{Binding CreateCheckpointCommand}\"",
            xaml,
            StringComparison.Ordinal);
    }

    private static GitPageViewModel NewViewModel(GitBackend backend) =>
        new(DisplayNameService.LoadDefault(), backend.Client);

    /// <summary>
    /// 等一个 fire-and-forget 命令走完。RelayCommand 的 Execute 是 <c>void</c>
    /// （`() => _ = XxxAsync()`），拿不到 Task，所以只能轮询可观测副作用。
    /// 照抄 ConfirmationReviewPanelTests 的同名助手，行为保持一致。
    /// </summary>
    private static async Task WaitUntilAsync(Func<bool> condition)
    {
        for (var attempt = 0; attempt < 200; attempt++)
        {
            if (condition())
            {
                return;
            }
            await Task.Delay(10);
        }
        Assert.Fail("等待条件超时");
    }

    private static BranchGraphNode Node(string id, string summary) => new(
        id,
        Array.Empty<string>(),
        Array.Empty<string>(),
        summary,
        DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(),
        "Ariadne Test");

    private static string ResolveDesktopSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var attempt = 0; attempt < 12 && walk is not null; attempt++)
        {
            var candidate = Path.Combine(
                new[] { walk.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            walk = walk.Parent;
        }
        throw new FileNotFoundException(string.Join('/', parts));
    }

    // DispatchProxy 的宿主类不能 sealed——它要在运行时派生该类型。
    private class GitBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public bool HasProjectRoot { get; set; } = true;
        public IReadOnlyList<BranchGraphNode> Graph { get; set; } = Array.Empty<BranchGraphNode>();

        /// <summary>落盘次数。本组用例的核心判据就是它从 0 变成 1。</summary>
        public int CheckpointCalls { get; private set; }

        /// <summary>最后一次收到的存档说明；空态那颗不填说明，应当是空串。</summary>
        public string? LastCheckpointMessage { get; private set; }

        public GitRepositoryStatus RepositoryStatus { get; set; } = new(
            "healthy",
            "main",
            "head-commit",
            false,
            null,
            0,
            string.Empty);

        public static GitBackend Create()
        {
            var client = Create<IAriadneBackendClient, GitBackend>();
            var backend = (GitBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            switch (targetMethod?.Name)
            {
                case "get_HasProjectRoot":
                    return HasProjectRoot;
                case nameof(IAriadneBackendClient.GetGitRepositoryStatusAsync):
                    return Task.FromResult(RepositoryStatus);
                case nameof(IAriadneBackendClient.GetGitBranchGraphAsync):
                    return Task.FromResult(Graph);
                case nameof(IAriadneBackendClient.GetGitHistoryAsync):
                    return Task.FromResult<IReadOnlyList<GitCommitSummary>>(
                        Array.Empty<GitCommitSummary>());
                case nameof(IAriadneBackendClient.CreateCheckpointAsync):
                    CheckpointCalls++;
                    LastCheckpointMessage = args is { Length: > 0 } ? args[0] as string : null;
                    return Task.FromResult(new ArchivePoint(
                        "manual-checkpoint",
                        "new-commit-id",
                        LastCheckpointMessage ?? string.Empty,
                        "manual"));
                default:
                    throw new NotSupportedException(targetMethod?.Name);
            }
        }
    }
}
