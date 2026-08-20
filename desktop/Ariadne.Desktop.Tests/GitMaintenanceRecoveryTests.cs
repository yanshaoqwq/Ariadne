using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U196-B / U196-C 回归：回档失败之后，Git 页上有没有可点的东西。
///
/// # 两条缺陷的性质不同，判据也不同
///
/// - **C 是「该拦没拦」**：同一屏上写着「存在未提交变更」，而「回档到副本」照样可点，
///   作者走完两个确认框（其中一个是危险确认）之后才被后端拒绝。
///   判据落在 <c>RestoreCommand.CanExecute</c> **与禁用理由文案**上
///   —— 只灰掉按钮不给理由，作者会以为功能坏了。
/// - **B 是「失败后没有出路」**：项目落进 `failed` 维护态后所有写操作被拒，
///   而界面给的唯一动作是「再点一次回档」，那件事刚刚失败过。
///   判据落在**恢复命令在维护态下是否可点**上。
///
/// # 为什么恢复命令的 CanExecute 是本文件最重要的一条断言
///
/// 「兜底按钮的 CanExecute 不能包含导致失败的那个前提」是本仓已记的教训
/// （U182-M：错误态的重试不能复用 `RefreshCommand`，因为后者要求 `HasProjectRoot`，
/// 而「无项目」正是那颗按钮要走的分支）。这类缺陷极难被测试发现：
/// 按钮存在、命令存在、只是**永远是灰的**。
/// </summary>
public sealed class GitMaintenanceRecoveryTests
{
    [Fact]
    public async Task DirtyWorktree_DisablesRestoreAndSaysWhy()
    {
        var names = DisplayNameService.LoadDefault();
        var viewModel = new GitPageViewModel(names, GitBackend.Create(dirty: true).Client);

        await viewModel.ReloadProjectDataAsync();

        Assert.True(viewModel.HasSelection, "没有选中存档点 ⇒ 下面测的是「没选」而不是「脏」");
        Assert.True(viewModel.IsRestoreBlocked);
        Assert.False(
            viewModel.RestoreCommand.CanExecute(null),
            "工作区脏时回档必失败（后端 ensure_clean_worktree），按钮却可点 ⇒ "
            + "作者读完两个确认框、点了两次「确定」，然后被告知他的输入不合要求");

        // 只灰掉不够：必须说出为什么，且要指向能解决它的动作（创建存档）。
        Assert.False(string.IsNullOrWhiteSpace(viewModel.RestoreBlockedText));
        Assert.DoesNotContain("[ui.git.", viewModel.RestoreBlockedText, StringComparison.Ordinal);
        Assert.Equal(names.Text("ui.git.restore_blocked_dirty"), viewModel.RestoreBlockedText);
    }

    /// <summary>
    /// 另一半。缺了它「无条件禁用回档」也能让上面那条全绿 ——
    /// 而那是把一个「该拦没拦」换成「拦了不该拦的」，对作者更糟。
    /// </summary>
    [Fact]
    public async Task CleanWorktree_KeepsRestoreClickableAndQuiet()
    {
        var viewModel = new GitPageViewModel(
            DisplayNameService.LoadDefault(),
            GitBackend.Create(dirty: false).Client);

        await viewModel.ReloadProjectDataAsync();

        Assert.True(viewModel.HasSelection);
        Assert.False(viewModel.IsRestoreBlocked);
        Assert.True(viewModel.RestoreCommand.CanExecute(null));
        Assert.Equal(string.Empty, viewModel.RestoreBlockedText);
    }

    /// <summary>
    /// 回档有**两个**入口：右栏那颗按钮、存档行的右键菜单。
    ///
    /// 原缺陷里两处共用同一个宽松判据（<c>CanStartOperation</c>），
    /// 所以修的时候也必须两处一起 —— 只改右栏那颗的话，右键菜单仍然把作者
    /// 送进那条必失败的路。这正是「做一半的功能会掩盖没做的一半」。
    /// </summary>
    [Fact]
    public async Task DirtyWorktree_AlsoDisablesTheContextMenuRestore()
    {
        var viewModel = new GitPageViewModel(
            DisplayNameService.LoadDefault(),
            GitBackend.Create(dirty: true).Client);

        await viewModel.ReloadProjectDataAsync();

        var row = Assert.Single(viewModel.Commits);
        Assert.False(
            row.RestoreCommand.CanExecute(null),
            "右键菜单里的「回档到此存档」绕过了脏工作区判定");
    }

    /// <summary>
    /// U196-B 的核心断言：**恢复动作必须在维护失败态下可点**。
    ///
    /// 这是本文件最重要的一条。缺陷形态不是「没有按钮」，而是
    /// 「按钮存在、命令存在、但在唯一需要它的那个状态下永远是灰的」——
    /// 按钮和命令都在，普通用例照样全绿。
    /// </summary>
    [Fact]
    public async Task MaintenanceFailed_OffersAClickableRecoveryAction()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = GitBackend.Create(
            dirty: false,
            maintenance: new ProjectMaintenanceState(
                Kind: "git_restore",
                Status: "failed",
                Phase: "restore_incomplete",
                Error: "disk full while checking out"));
        var viewModel = new GitPageViewModel(names, backend.Client);

        await viewModel.ReloadProjectDataAsync();

        Assert.True(viewModel.IsMaintenanceFailed, "维护失败态在 Git 页上不可见 ⇒ 作者只看到一句错误");
        Assert.True(
            viewModel.RecoverMaintenanceCommand.CanExecute(null),
            "恢复命令在维护失败态下不可点 ⇒ 又造了一个死胡同。"
            + "兜底按钮的 CanExecute 不能包含导致失败的那个前提（U182-M 同一教训）");

        // 说明文字必须解析出来：缺 key 时 DisplayNameService 返回 `[key]`，
        // 那时面板可见但内容是占位串，等于没有面板。
        foreach (var text in new[]
                 {
                     viewModel.MaintenanceFailedTitle,
                     viewModel.MaintenanceFailedHint,
                     viewModel.RecoverMaintenanceText,
                 })
        {
            Assert.False(string.IsNullOrWhiteSpace(text));
            Assert.DoesNotContain("[ui.git.", text, StringComparison.Ordinal);
        }

        // 后端诊断作为二级信息带出来——它是作者判断「回档做了多少」的唯一线索。
        Assert.True(viewModel.HasMaintenanceDiagnostic);
        Assert.Contains("disk full", viewModel.MaintenanceDiagnosticText, StringComparison.Ordinal);
    }

    /// <summary>
    /// `active` 不是 `failed`：那是一场**正在跑**的维护，作者要做的是等。
    ///
    /// 提供「解除」会诱导他打断 checkout 临界区 —— 后端也会拒
    /// （`begin_maintenance` 见 `active` 直接返回 Err），但那时作者已经点了。
    /// </summary>
    [Fact]
    public async Task MaintenanceActive_DoesNotOfferRecovery()
    {
        var backend = GitBackend.Create(
            maintenance: new ProjectMaintenanceState("git_restore", "active", "checking_out_branch", null));
        var viewModel = new GitPageViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await viewModel.ReloadProjectDataAsync();

        Assert.False(viewModel.IsMaintenanceFailed);
        Assert.False(viewModel.RecoverMaintenanceCommand.CanExecute(null));
    }

    /// <summary>
    /// 健康项目上不能出现这块面板。
    ///
    /// 缺了这条，「无条件显示恢复面板」也能让上面那条全绿 ——
    /// 而那会让每个作者都在 Git 页顶上常驻一句「项目暂时只读」。
    /// </summary>
    [Fact]
    public async Task NoMaintenance_HidesTheRecoveryPanel()
    {
        var viewModel = new GitPageViewModel(
            DisplayNameService.LoadDefault(),
            GitBackend.Create().Client);

        await viewModel.ReloadProjectDataAsync();

        Assert.False(viewModel.IsMaintenanceFailed);
        Assert.False(viewModel.HasMaintenanceDiagnostic);
        Assert.False(viewModel.RecoverMaintenanceCommand.CanExecute(null));
    }

    /// <summary>
    /// 恢复成功后面板消失，且**索引状态如实呈现**。
    ///
    /// 两种结果对作者的意义不同：worker 起来了 = 索引正在重建；
    /// 没起来 = 重建还在队列里，此刻搜到的可能是回档前的旧内容。
    /// 共用一句「已恢复」会把后者说成前者。
    /// </summary>
    [Theory]
    [InlineData(true, "ui.git.maintenance.recovered_rebuilding")]
    [InlineData(false, "ui.git.maintenance.recovered_queued")]
    public async Task Recovery_ClearsThePanelAndTellsTheTruthAboutTheIndex(
        bool indexRebuildStarted,
        string expectedKey)
    {
        var names = DisplayNameService.LoadDefault();
        var backend = GitBackend.Create(
            maintenance: new ProjectMaintenanceState("git_restore", "failed", "restore_incomplete", "boom"));
        backend.RecoveryReport = new MaintenanceRecoveryReport(
            "git_restore",
            "restore_incomplete",
            "boom",
            indexRebuildStarted);
        var viewModel = new GitPageViewModel(names, backend.Client);
        await viewModel.ReloadProjectDataAsync();
        Assert.True(viewModel.IsMaintenanceFailed, "前提没造出来");

        viewModel.RecoverMaintenanceCommand.Execute(null);
        await WaitUntilAsync(() => backend.RecoveryCalls > 0 && !viewModel.IsBusy);

        Assert.Equal(1, backend.RecoveryCalls);
        Assert.False(
            viewModel.IsMaintenanceFailed,
            "恢复成功了面板还在 ⇒ 作者以为没成功，会再点一次（第二次会撞上「没有可解除的失败态」）");
        Assert.Equal(names.Text(expectedKey), viewModel.StatusText);
        Assert.DoesNotContain("[ui.git.", viewModel.StatusText, StringComparison.Ordinal);
    }

    /// <summary>
    /// 恢复失败时面板必须**留在屏上**。
    ///
    /// 清掉它等于把失败报成成功：作者以为项目解禁了，接着每一次保存
    /// 又被拒绝，而屏上已经没有任何解释和出路。
    /// </summary>
    [Fact]
    public async Task RecoveryFailure_KeepsThePanelOnScreen()
    {
        var backend = GitBackend.Create(
            maintenance: new ProjectMaintenanceState("git_restore", "failed", "restore_incomplete", "boom"));
        backend.RecoveryFailure = new BackendException("conflict", "still failing");
        var viewModel = new GitPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();

        viewModel.RecoverMaintenanceCommand.Execute(null);
        await WaitUntilAsync(() => backend.RecoveryCalls > 0 && !viewModel.IsBusy);

        Assert.True(viewModel.IsMaintenanceFailed, "恢复失败却把面板收了 ⇒ 出路从屏上消失");
        Assert.True(viewModel.RecoverMaintenanceCommand.CanExecute(null), "重试必须仍然可点");
        Assert.False(string.IsNullOrWhiteSpace(viewModel.StatusText));
    }

    /// <summary>
    /// 这两个标志**必须广播 PropertyChanged**。
    ///
    /// # 为什么单独一条
    ///
    /// 上面所有用例都直接读属性 ⇒ 「值算对了但没广播」这个盲区它们**全都测不到**。
    /// 而界面是绑定驱动的：`IsVisible="{Binding IsMaintenanceFailed}"` 在没有
    /// 通知时会一直停在初始值 ——「状态已经对了但界面还停在上一屏」，
    /// 与「根本没接这条判定」在屏幕上完全同形（本仓已把这条教训写在
    /// `NotifyLoadStateDerived` 的注释里）。
    /// </summary>
    [Fact]
    public async Task BlockedAndMaintenanceFlags_RaisePropertyChanged()
    {
        var backend = GitBackend.Create(dirty: false);
        var viewModel = new GitPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();

        var seen = new List<string>();
        viewModel.PropertyChanged += (_, args) =>
        {
            if (!string.IsNullOrEmpty(args.PropertyName))
            {
                seen.Add(args.PropertyName!);
            }
        };

        // 作者在别处写了几个字（磁盘变脏），然后回到 Git 页刷新。
        backend.RepositoryStatus = backend.RepositoryStatus with { Dirty = true };
        backend.Maintenance = new ProjectMaintenanceState("git_restore", "failed", "restore_incomplete", "boom");
        await viewModel.ReloadProjectDataAsync();

        Assert.Contains(nameof(GitPageViewModel.IsRestoreBlocked), seen);
        Assert.Contains(nameof(GitPageViewModel.RestoreBlockedText), seen);
        Assert.Contains(nameof(GitPageViewModel.IsMaintenanceFailed), seen);
        Assert.Contains(nameof(GitPageViewModel.MaintenanceDiagnosticText), seen);
    }

    /// <summary>
    /// 界面**真的绑了**这些属性。
    ///
    /// ViewModel 全对而 XAML 没绑，是本仓反复出现的「做一半」形态：
    /// 所有 VM 用例全绿，屏幕上什么都没变。
    /// </summary>
    [Fact]
    public void GitPageView_BindsTheRecoveryPanelAndTheBlockedReason()
    {
        var view = File.ReadAllText(ResolveRepoFile(Path.Combine(
            "desktop", "Ariadne.Desktop", "Views", "GitPageView.axaml")));

        foreach (var binding in new[]
                 {
                     "{Binding IsMaintenanceFailed}",
                     "{Binding MaintenanceFailedTitle}",
                     "{Binding MaintenanceFailedHint}",
                     "{Binding RecoverMaintenanceCommand}",
                     "{Binding RecoverMaintenanceText}",
                     "{Binding MaintenanceDiagnosticText}",
                     "{Binding HasMaintenanceDiagnostic}",
                     "{Binding RestoreBlockedText}",
                     "{Binding IsRestoreBlocked}",
                 })
        {
            Assert.Contains(binding, view, StringComparison.Ordinal);
        }
    }

    private static async Task WaitUntilAsync(Func<bool> condition)
    {
        for (var attempt = 0; attempt < 400 && !condition(); attempt++)
        {
            await Task.Delay(5);
        }
        Assert.True(condition(), "等待条件未满足（命令没跑完？）");
    }

    private static string ResolveRepoFile(string relative)
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 12 && dir is not null; i++)
        {
            var candidate = Path.Combine(dir.FullName, relative);
            if (File.Exists(candidate))
            {
                return candidate;
            }
            dir = dir.Parent;
        }
        throw new FileNotFoundException($"找不到 {relative}（从 {AppContext.BaseDirectory} 向上找）");
    }

    // DispatchProxy 的宿主类不能 sealed——它要在运行时派生该类型。
    private class GitBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public GitRepositoryStatus RepositoryStatus { get; set; } = null!;
        public ProjectMaintenanceState? Maintenance { get; set; }
        public MaintenanceRecoveryReport RecoveryReport { get; set; } =
            new("git_restore", "restore_incomplete", "disk full", IndexRebuildStarted: true);
        public int RecoveryCalls { get; private set; }

        /// <summary>恢复成功后维护态应当消失——替身照真实后端的行为翻面。</summary>
        public bool ClearMaintenanceOnRecovery { get; set; } = true;

        public Exception? RecoveryFailure { get; set; }

        public static GitBackend Create(
            bool dirty = false,
            ProjectMaintenanceState? maintenance = null)
        {
            var client = Create<IAriadneBackendClient, GitBackend>();
            var backend = (GitBackend)(object)client;
            backend.Client = client;
            backend.RepositoryStatus = new GitRepositoryStatus(
                "healthy",
                "main",
                "head-commit-id",
                dirty,
                null,
                dirty ? 3 : 0,
                dirty ? "+ 新写的一段" : string.Empty);
            backend.Maintenance = maintenance;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetGitRepositoryStatusAsync))
            {
                return Task.FromResult(RepositoryStatus);
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetProjectMaintenanceAsync))
            {
                return Task.FromResult(Maintenance);
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.RecoverProjectMaintenanceAsync))
            {
                RecoveryCalls++;
                if (RecoveryFailure is not null)
                {
                    return Task.FromException<MaintenanceRecoveryReport>(RecoveryFailure);
                }
                if (ClearMaintenanceOnRecovery)
                {
                    Maintenance = null;
                }
                return Task.FromResult(RecoveryReport);
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetGitBranchGraphAsync))
            {
                return Task.FromResult<IReadOnlyList<BranchGraphNode>>(new[]
                {
                    new BranchGraphNode(
                        "head-commit-id",
                        Array.Empty<string>(),
                        new[] { "HEAD -> main" },
                        "Archive: 第三章完成",
                        TimestampMs: 1_760_000_000_000,
                        Author: "作者",
                        CheckpointKind: "manual",
                        IsHead: true),
                });
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetGitHistoryAsync))
            {
                return Task.FromResult<IReadOnlyList<GitCommitSummary>>(Array.Empty<GitCommitSummary>());
            }

            throw new NotSupportedException(targetMethod?.Name);
        }
    }
}
