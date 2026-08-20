using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U207-F 的**前端那一半**：右栏「未提交变更」与「变更摘要」两行。
///
/// # 后端修好了，为什么前端还要一条
///
/// F 条的根因在后端（裸 `git diff` 只比工作区↔索引，看不见未跟踪文件），
/// 已由 `core/tests/git_diff_status_parity_contracts.rs` 的 7 条守住。
/// 但那 7 条守的是**后端算得对**，守不到「算对了却没送到屏幕上」：
/// 两行都是绑定属性，口径对齐了而 PropertyChanged 没广播的话，
/// 界面照旧显示上一轮那对互相打脸的数据 —— 与完全没修在屏幕上同形。
///
/// 所以这里的判据分两层：
/// ① 两行的**值**不互相矛盾（说脏就得有行数）；
/// ② 两行都**广播**了 PropertyChanged（其余用例直接读属性，测不到这个盲区）。
/// </summary>
public sealed class GitDiffSummaryRowParityTests
{
    [Fact]
    public async Task DirtyRowAndDiffSummaryRow_BothBroadcastPropertyChanged()
    {
        var backend = GitBackend.Create();
        var viewModel = NewViewModel(backend);
        // 先走一轮，让状态离开初值——否则「值没变所以 SetProperty 不广播」
        // 会被误读成「广播坏了」。
        await viewModel.ReloadProjectDataAsync();

        var seen = new List<string>();
        viewModel.PropertyChanged += (_, args) =>
        {
            if (args.PropertyName is not null)
            {
                seen.Add(args.PropertyName);
            }
        };

        // 第二轮：磁盘变脏，两行的值都该变、都该广播。
        backend.RepositoryStatus = new GitRepositoryStatus(
            "healthy", "main", "head-commit", true, null, 120, "diff --git a/ch001.md ...");
        await viewModel.ReloadProjectDataAsync();

        Assert.Contains(nameof(GitPageViewModel.DirtyStateText), seen);
        Assert.Contains(nameof(GitPageViewModel.DiffSummaryText), seen);
    }

    /// <summary>
    /// 两行的**值**不许自相矛盾 —— 这是 F 条的用户可见形态。
    ///
    /// 后端对齐口径之后，「脏」与「行数 &gt; 0」应当同真同假。
    /// 这里两个方向各测一次：只测「脏 + 有行数」的话，
    /// 一个恒说「存在未提交变更」的实现照样全绿。
    /// </summary>
    [Theory]
    [InlineData(true, 120)]
    [InlineData(false, 0)]
    public async Task DirtyRowAndDiffSummaryRow_TellTheSameStory(bool dirty, int lineCount)
    {
        var names = DisplayNameService.LoadDefault();
        var backend = GitBackend.Create();
        backend.RepositoryStatus = new GitRepositoryStatus(
            "healthy", "main", "head-commit", dirty, null, lineCount, string.Empty);
        var viewModel = NewViewModel(backend);

        await viewModel.ReloadProjectDataAsync();

        // 「未提交变更」这一行。
        Assert.Equal(
            names.Text(dirty ? "ui.git.dirty" : "ui.git.clean"),
            viewModel.DirtyStateText);
        // 「变更摘要」这一行。
        Assert.Contains(lineCount.ToString(), viewModel.DiffSummaryText, StringComparison.Ordinal);
        // 核心：两行讲的是同一件事。原缺陷是「存在未提交变更」配「0 行 diff」，
        // 正是这个等式不成立的那一刻。
        var summarySaysSomethingChanged = lineCount > 0;
        var dirtySaysSomethingChanged =
            viewModel.DirtyStateText == names.Text("ui.git.dirty");
        Assert.Equal(dirtySaysSomethingChanged, summarySaysSomethingChanged);
        // 文案不能是缺 key 的兜底形态。
        Assert.DoesNotContain("[ui.git.", viewModel.DirtyStateText, StringComparison.Ordinal);
        Assert.DoesNotContain("[ui.git.", viewModel.DiffSummaryText, StringComparison.Ordinal);
    }

    private static GitPageViewModel NewViewModel(GitBackend backend) =>
        new(DisplayNameService.LoadDefault(), backend.Client);

    // DispatchProxy 的宿主类不能 sealed——它要在运行时派生该类型。
    private class GitBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public bool HasProjectRoot { get; set; } = true;

        public GitRepositoryStatus RepositoryStatus { get; set; } = new(
            "healthy", "main", "head-commit", false, null, 0, string.Empty);

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
                    return Task.FromResult<IReadOnlyList<BranchGraphNode>>(
                        Array.Empty<BranchGraphNode>());
                case nameof(IAriadneBackendClient.GetGitHistoryAsync):
                    return Task.FromResult<IReadOnlyList<GitCommitSummary>>(
                        Array.Empty<GitCommitSummary>());
                default:
                    throw new NotSupportedException(targetMethod?.Name);
            }
        }
    }
}
