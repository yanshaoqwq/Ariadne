using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U210 回归：Git 页三句「做成了」的文案，在成功路径上从来没被作者读到过。
///
/// # 原缺陷不是「没实现」，而是「实现了、跑过了、被自己抹掉了」
///
/// `RefreshCoreAsync` 末尾的 `SelectAfterRefresh` **无条件**把 `StatusText`
/// 改写成「共 N 个存档点」，而每条成功路径的顺序都是：
///
/// 1. 动作成功
/// 2. `StatusText = 「已存档：X」`   ← 写对了
/// 3. `await RefreshCoreAsync(...)`
/// 4. `SelectAfterRefresh` ⇒ `StatusText = 「共 12 个存档点」` ← 覆盖掉第 2 步
///
/// 作者点「创建存档」，成功，他读到的是一句与他刚做的事无关的存档总数。
/// 被覆盖的窗口小到肉眼捕捉不到，所以这一族缺陷**只有守卫测试拦得住**。
///
/// # 判据为什么必须是「刷新之后仍然是那句成功文案」
///
/// ⚠️ **不能**断言「`StatusText` 曾被赋值成过『已存档：X』」——
/// **缺陷版本里它确实被赋值过**，那种断言在修复前就是绿的，是一条空测。
/// 所以本文件每条用例都在**命令整个跑完之后**（`!IsBusy`，即 `finally`
/// 里的 `EndOperation` 已执行，`RefreshCoreAsync` 与 `_reloadProjectData`
/// 都已 await 完）才读 `StatusText`，读到的就是作者眼睛看到的那一帧。
///
/// # 为什么同时钉住「静息态描述没有消失」
///
/// 只让成功文案活下来有一种廉价的假修法：把 `SelectAfterRefresh` 里那行删掉了事。
/// 那会让「共 N 个存档点」这个信息**整个消失**——修好一个可见性问题、
/// 弄坏另一个。所以每条用例都同时断言 `HistorySummaryText` 仍是那句计数，
/// 并另有一条钉住它在界面上**真的有渲染位**（否则又是「有属性、界面零消费」）。
/// </summary>
// 回档那条用例要替换全局 DialogService 单例 ⇒ 必须与其他替换它的测试互斥。
[Collection("GlobalDialogService")]
public sealed class GitSuccessMessageSurvivesRefreshTests
{
    /// <summary>
    /// 「已存档：X」必须在刷新之后**仍然在屏上**。
    ///
    /// 判据落在 `!IsBusy` 之后读到的 `StatusText`——那一刻
    /// `RefreshCoreAsync` 已 await 完、`EndOperation` 已执行，
    /// 读到的就是作者眼睛看到的那一帧。断言「被赋值过」在缺陷版本里也是绿的。
    /// </summary>
    [Fact]
    public async Task CreateCheckpoint_SuccessMessageIsStillOnScreenAfterTheRefresh()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = GitBackend.Create();
        backend.Graph = new[] { Node("first", "第一章草稿"), Node("second", "第二章草稿") };
        var viewModel = NewViewModel(backend);
        await viewModel.ReloadProjectDataAsync();
        viewModel.CheckpointMessage = "写完第三章";

        Assert.True(viewModel.CreateCheckpointCommand.TryExecute());
        await WaitUntilAsync(() => backend.CheckpointCalls >= 1 && !viewModel.IsBusy);

        var expected = names.Format(
            "ui.git.checkpoint_created",
            new Dictionary<string, string> { ["summary"] = "写完第三章" });
        Assert.Equal(expected, viewModel.StatusText);
        // 反向钉住：读到的**不能**是那句静息态描述。
        // 这一条是本文件的本体——缺陷版本恰好在这里失败。
        Assert.DoesNotContain("存档点", viewModel.StatusText, StringComparison.Ordinal);
    }

    /// <summary>
    /// 另一半：静息态描述**没有因此消失**。
    ///
    /// 缺了这条，「把 SelectAfterRefresh 里那行删掉」这种假修法能让上面全绿，
    /// 而代价是「共 N 个存档点」这个信息整个不见了——修好一处可见性、
    /// 弄坏另一处，正是本仓已记的「做一半的功能会掩盖没做的一半」。
    /// </summary>
    [Fact]
    public async Task CreateCheckpoint_RestingDescriptionSurvivesToo()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = GitBackend.Create();
        backend.Graph = new[] { Node("first", "第一章草稿"), Node("second", "第二章草稿") };
        var viewModel = NewViewModel(backend);
        await viewModel.ReloadProjectDataAsync();

        Assert.True(viewModel.CreateCheckpointCommand.TryExecute());
        await WaitUntilAsync(() => backend.CheckpointCalls >= 1 && !viewModel.IsBusy);

        // 两句话同时在屏上，各自有渲染位——这正是「分成两个属性」的验收点。
        Assert.Equal(
            names.Format("ui.git.count", new Dictionary<string, string> { ["count"] = "2" }),
            viewModel.HistorySummaryText);
        Assert.Equal(names.Text("ui.git.checkpoint_created_plain"), viewModel.StatusText);
    }

    /// <summary>
    /// 「已切换到回档副本『X』。{followup}」——这条比存档更要紧。
    ///
    /// 回档成功后作者的处境**变了**（他现在在一个新分支上，索引可能还在重建），
    /// followup 那半句正是告诉他这件事的唯一位置。读不到它，
    /// 作者以为什么都没发生，而他的写入已经落在另一个分支上了。
    ///
    /// 这条路径上有**两次**覆盖来源（`RefreshCoreAsync` 直接调用 +
    /// `_reloadProjectData()` 回头刷本页），所以本用例把宿主重载链也接上了。
    /// </summary>
    [Fact]
    public async Task Restore_SuccessMessageSurvivesBothTheRefreshAndTheHostReload()
    {
        var names = DisplayNameService.LoadDefault();
        DialogService.Initialize(names);
        var backend = GitBackend.Create();
        backend.Graph = new[] { Node("first", "第一章草稿"), Node("second", "第二章草稿") };
        backend.RestoreReport = new RestoreReport("restore-abc123", "abc123", true, false);
        var viewModel = NewViewModel(backend);
        // 宿主重载会让本页再刷一遍自己（生产里 Git 页也在 _pageCache 中）。
        backend.HostReload = () => viewModel.ReloadProjectDataAsync();
        await viewModel.ReloadProjectDataAsync();
        viewModel.SelectedCommit = viewModel.Commits[0];

        // 回档要过一个 Danger 确认框（`ConfirmRestoreAsync` 走全局 DialogService）。
        // 后台盯着它、按下确认——这是这一族用例里唯一的真实交互。
        var answering = AnswerConfirmDialogAsync();

        Assert.True(viewModel.RestoreCommand.TryExecute());
        await WaitUntilAsync(() => backend.RestoreCalls >= 1 && !viewModel.IsBusy);
        await answering;

        // 前提自检：宿主重载这条第二次覆盖来源**真的被触发过**。
        // 少了它本用例就只覆盖了一半的覆盖来源。
        Assert.True(backend.HostReloadCalls >= 1, "宿主重载没被调到 ⇒ 只测了一半");
        var expected = names.Format("ui.git.restore_done", new Dictionary<string, string>
        {
            ["branch"] = "restore-abc123",
            ["followup"] = names.Text("ui.git.restore_followup.index"),
        });
        Assert.Equal(expected, viewModel.StatusText);
        Assert.DoesNotContain("存档点", viewModel.StatusText, StringComparison.Ordinal);
    }

    /// <summary>
    /// 结果文案不能变成**永久贴纸**。
    ///
    /// # 为什么这条和上面几条一样重要
    ///
    /// 上面几条要求「成功文案活过刷新」。只满足那个的最省事做法是
    /// 让 `StatusText` 从此没人清 —— 那样作者点完存档、去做别的事、
    /// 回来看到的还是半小时前那句「已存档：写完第三章」。
    /// 一句永远为真的话等于没有话，作者会学会不看这一行。
    ///
    /// 修复取的口径是「结果文案的寿命 = 直到下一次动作开始」，
    /// 清除点收在 `TryBeginOperation`（所有动作的唯一闸门）。
    /// 本用例钉的就是这个：新动作一开始，上一次的结果就不该还在屏上。
    ///
    /// ⚠️ 判据取「新动作**进行中**那一刻」而不是「动作结束后」——
    /// 结束后新的结果文案已经写进去了，那时读不出是「被清过」还是「被覆盖」。
    /// 所以这里用一个**能卡住**的后端：请求发出后停在半路，
    /// 让用例读到进行中那一帧。
    /// </summary>
    [Fact]
    public async Task StartingANewAction_ClearsThePreviousResultMessage()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = GitBackend.Create();
        backend.Graph = new[] { Node("first", "第一章草稿") };
        var viewModel = NewViewModel(backend);
        await viewModel.ReloadProjectDataAsync();
        viewModel.CheckpointMessage = "写完第三章";

        // 第一次：正常跑完，屏上留下成功文案。
        Assert.True(viewModel.CreateCheckpointCommand.TryExecute());
        await WaitUntilAsync(() => backend.CheckpointCalls >= 1 && !viewModel.IsBusy);
        var first = names.Format(
            "ui.git.checkpoint_created",
            new Dictionary<string, string> { ["summary"] = "写完第三章" });
        Assert.Equal(first, viewModel.StatusText);

        // 第二次：让后端在半路卡住，读「动作进行中」那一帧。
        var gate = new TaskCompletionSource();
        backend.CheckpointGate = gate.Task;
        Assert.True(viewModel.CreateCheckpointCommand.TryExecute());
        await WaitUntilAsync(() => viewModel.IsBusy);

        Assert.True(
            string.IsNullOrEmpty(viewModel.StatusText),
            "新动作已经开始，上一次的结果文案还挂在屏上 ⇒ 作者读到的是一句陈旧的「已存档」，"
            + "实际值：" + viewModel.StatusText);

        gate.SetResult();
        await WaitUntilAsync(() => !viewModel.IsBusy);
    }

    /// <summary>
    /// 静息态描述必须**真的有渲染位**，不能只是一个属性。
    ///
    /// 少了这条，「把计数搬进一个新属性」就能让上面几条全绿，
    /// 而界面上「共 N 个存档点」实际消失了 —— 那是本仓反复出现的
    /// 「有实现、有维护、界面零消费」形态（U198-B 的 RecoveryText 原状）。
    /// </summary>
    [Fact]
    public void RestingDescription_HasARealRenderingSlotInTheView()
    {
        // ⚠️ **必须先剥 XAML 注释**。本条对源码原文做 `Contains`，
        // 而 `<!-- … -->` 里若出现同样的字符串就会假命中。
        // 这不是假想：本条施工时的变异（把绑定换成状态文案那个属性）**全绿**，
        // 因为变异标记里复述了被断言的那串绑定表达式，`Assert.Contains`
        // 命中了那行注释本身。
        // ⇒ 两条各自成立的规矩：① 对源码文本做断言的用例要剥注释；
        // ② 变异标记里不要复述被断言的字符串。只做 ② 会留下一个
        // **下一个人写注释时随手引用一下就假绿**的用例，所以这里把 ① 也做上。
        // 📌 本段注释本身刻意**不写出**那串绑定表达式 —— 否则它就是自己在描述的那个雷。
        var view = StripXamlComments(
            File.ReadAllText(ResolveDesktopSource("Views", "GitPageView.axaml")));

        Assert.Contains(
            "Text=\"{Binding HistorySummaryText}\"",
            view,
            StringComparison.Ordinal);
        // 状态 strip 那一行仍然绑 StatusText（动作结果的位置没被搬走）。
        Assert.Contains("Text=\"{Binding StatusText}\"", view, StringComparison.Ordinal);
    }

    /// <summary>剥掉 <c>&lt;!-- --&gt;</c> 注释，只留真实标记。</summary>
    private static string StripXamlComments(string markup)
        => System.Text.RegularExpressions.Regex.Replace(
            markup, "<!--.*?-->", string.Empty,
            System.Text.RegularExpressions.RegexOptions.Singleline);

    /// <summary>
    /// 反向守卫：`SelectAfterRefresh` 不许再写 `StatusText`。
    ///
    /// # 为什么要一条源码断言（通常是坏味道）
    ///
    /// 因为这个缺陷的复发方式是「往刷新收尾里顺手加一行状态文案」，
    /// 而那一行加回去时**上面所有行为用例仍可能是绿的**：
    /// 只要它写在结果文案之前、或只在某个分支上生效，行为测试就抓不到。
    /// 这条断言钉的是「刷新路径不再拥有 StatusText 的写权」这个**结构性质**，
    /// 它才是修复的本体。
    /// </summary>
    [Fact]
    public void RefreshPath_NoLongerOwnsTheStatusText()
    {
        var source = File.ReadAllText(ResolveDesktopSource("ViewModels", "GitPageViewModel.cs"));
        var start = source.IndexOf("private void SelectAfterRefresh(", StringComparison.Ordinal);
        Assert.True(start > 0, "SelectAfterRefresh 不见了 ⇒ 本断言的前提没了，请重新定判据");
        var end = source.IndexOf("\n    }", start, StringComparison.Ordinal);
        Assert.True(end > start, "找不到方法结尾");
        var body = source[start..end];

        // 钉的是**写权**（赋值），不是「方法体里不许出现这个词」——
        // 后者会让「将来在方法体内加一句提到 StatusText 的注释」变成假红。
        Assert.DoesNotContain("StatusText =", body, StringComparison.Ordinal);
        Assert.DoesNotContain("InitializeStatusText", body, StringComparison.Ordinal);
    }

    /// <summary>
    /// 后台盯住全局弹窗并按下 <c>ConfirmResultIndex</c> 那颗按钮。
    ///
    /// ⚠️ 不能用 `RequestConfirmActive()`：危险语义的弹窗（`Severity = Danger`）
    /// 会拒绝键盘 Enter 路由（`AllowEnterConfirm` 为 false），
    /// 那样等下去只会超时，而症状与「回档压根没发起」同形。
    /// 所以这里直接执行确认按钮的 Command，与鼠标点击同一条路。
    /// </summary>
    private static async Task AnswerConfirmDialogAsync()
    {
        for (var attempt = 0; attempt < 400; attempt++)
        {
            var dialog = DialogService.Current.ActiveDialog;
            if (dialog is not null)
            {
                var confirm = dialog.Buttons
                    .FirstOrDefault(button => button.ResultIndex == dialog.ConfirmResultIndex);
                Assert.NotNull(confirm);
                confirm!.Command?.Execute(null);
                return;
            }
            await Task.Delay(5);
        }
        Assert.Fail("回档确认框没有弹出 ⇒ 本用例根本没走到回档");
    }

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

    private static GitPageViewModel NewViewModel(GitBackend backend) =>
        new(DisplayNameService.LoadDefault(),
            backend.Client,
            // 两个确认框一律放行：本组测的是**成功之后**那一帧。
            confirmProjectReload: () => Task.FromResult(true),
            // ⚠️ 这个委托是缺陷的第二次覆盖来源：真实宿主的
            // `ReloadCachedProjectPagesExceptAsync(null)` 遍历整个页缓存，
            // **Git 页自己也在里面** ⇒ 它会让本页再刷一遍。
            // 测试里必须把这条链接上，否则测的是一个比生产宽松的场景。
            reloadProjectData: () => backend.SimulateHostReloadAsync());

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

    private static BranchGraphNode Node(string id, string summary) => new(
        id,
        Array.Empty<string>(),
        Array.Empty<string>(),
        summary,
        0,
        "author",
        "manual",
        false);

    // DispatchProxy 的宿主类不能 sealed——它要在运行时派生该类型。
    private class GitBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public bool HasProjectRoot { get; set; } = true;
        public IReadOnlyList<BranchGraphNode> Graph { get; set; } = Array.Empty<BranchGraphNode>();
        public int CheckpointCalls { get; private set; }
        public int RestoreCalls { get; private set; }

        /// <summary>回档报告。followup 分支由这两个 bool 决定，用例会挑一个断言。</summary>
        public RestoreReport RestoreReport { get; set; } = new(
            "restore-abc123",
            "abc123",
            false,
            false);

        public string? LastCheckpointMessage { get; private set; }

        /// <summary>
        /// 设上之后 `CreateCheckpointAsync` 会等这个 Task 才返回，
        /// 用来把用例停在「动作进行中」那一帧上。
        /// </summary>
        public Task? CheckpointGate { get; set; }

        /// <summary>
        /// 宿主重载回调注入点：`GitPageViewModel` 自己不知道宿主会把它也刷一遍，
        /// 所以由本 fake 在构造后被赋成「调 VM 的 ReloadProjectDataAsync」。
        /// </summary>
        public Func<Task>? HostReload { get; set; }

        public int HostReloadCalls { get; private set; }

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

        /// <summary>可被 <see cref="CheckpointGate"/> 拦在半路的落盘。</summary>
        private async Task<ArchivePoint> CompleteCheckpointAsync(string message)
        {
            if (CheckpointGate is not null)
            {
                await CheckpointGate.ConfigureAwait(false);
            }
            return new ArchivePoint("manual-checkpoint", "new-commit-id", message, "manual");
        }

        public async Task SimulateHostReloadAsync()        {
            HostReloadCalls++;
            if (HostReload is not null)
            {
                await HostReload().ConfigureAwait(false);
            }
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
                case nameof(IAriadneBackendClient.GetProjectMaintenanceAsync):
                    return Task.FromResult<ProjectMaintenanceState?>(null);
                case nameof(IAriadneBackendClient.CreateCheckpointAsync):
                    CheckpointCalls++;
                    LastCheckpointMessage = args is { Length: > 0 } ? args[0] as string : null;
                    return CompleteCheckpointAsync(LastCheckpointMessage ?? string.Empty);
                case nameof(IAriadneBackendClient.RestoreToNewBranchAsync):
                    RestoreCalls++;
                    return Task.FromResult(RestoreReport);
                default:
                    throw new NotSupportedException(targetMethod?.Name);
            }
        }
    }
}
