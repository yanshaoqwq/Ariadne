using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U197-H 回归：Git 页的「干净」不该让作者以为正文已进版本控制。
///
/// # 缺陷形态：两个 dirty 都正确，并列呈现时误导
///
/// `DirtyStateText` 说的是**磁盘上**有没有未提交改动；
/// 作品页的 `HasUnsavedChanges` 说的是**内存里**有没有未保存改动。
/// 作者在作品页写了 3000 字没按 Ctrl+S，切到 Git 页看到「无未提交变更」
/// ⇒ 理解成「干净 = 我的东西都在版本控制里了」。而那 3000 字连磁盘都没到。
///
/// 与 U183（正文在 Ctrl+S 之前只存在于内存）叠加时后果最重。
///
/// # 判据落在呈现状态上，不落在「宿主委托被调用了」
///
/// 缺陷版本里那个委托压根不存在（`HasCachedUnsavedChanges` 此前**只被关窗守卫**
/// `MainWindow.axaml.cs:275` 消费、零可视化消费点），所以「委托被调用」
/// 这种判据只能证明我接了根线，证明不了屏幕上多出那句话。
/// </summary>
public sealed class GitDirtyHintTests
{
    [Fact]
    public async Task CleanRepositoryWithUnsavedEditsElsewhere_ShowsTheHint()
    {
        var backend = GitBackend.Create(dirty: false);
        var viewModel = new GitPageViewModel(
            DisplayNameService.LoadDefault(),
            backend.Client,
            hasOtherUnsavedChanges: () => true);

        await viewModel.ReloadProjectDataAsync();

        var names = DisplayNameService.LoadDefault();
        Assert.True(
            viewModel.HasOtherPagesUnsaved,
            "磁盘干净 + 别页内存脏 ⇒ 必须提示，否则作者以为 3000 字已进版本控制");
        Assert.Equal(names.Text("ui.git.other_pages_unsaved"), viewModel.OtherPagesUnsavedText);
        // 文案必须真的解析出来：缺 key 时 DisplayNameService 返回 `[key]`，
        // 那时提示可见但内容是个占位串，等于没提示。
        Assert.DoesNotContain("[ui.git.", viewModel.OtherPagesUnsavedText, StringComparison.Ordinal);
    }

    /// <summary>
    /// ⚠️ 这条是本文件的另一半，缺了它「无条件恒显示」也能让上面那条全绿。
    ///
    /// 磁盘也脏时 `DirtyStateText` 已经在说「存在未提交变更」——
    /// 此刻作者**不会**误以为东西已入库，再补一句只是噪音。
    /// 误读只发生在屏上写着「干净」那一刻，提示也就只该出现在那一刻。
    /// </summary>
    [Fact]
    public async Task DirtyRepository_DoesNotStackASecondUnsavedNotice()
    {
        var backend = GitBackend.Create(dirty: true);
        var viewModel = new GitPageViewModel(
            DisplayNameService.LoadDefault(),
            backend.Client,
            hasOtherUnsavedChanges: () => true);

        await viewModel.ReloadProjectDataAsync();

        Assert.False(
            viewModel.HasOtherPagesUnsaved,
            "磁盘已脏时上一行已经在说有未提交变更，再叠一句是噪音");
        Assert.Equal(string.Empty, viewModel.OtherPagesUnsavedText);
    }

    [Fact]
    public async Task CleanRepositoryWithNothingUnsaved_StaysQuiet()
    {
        var backend = GitBackend.Create(dirty: false);
        var viewModel = new GitPageViewModel(
            DisplayNameService.LoadDefault(),
            backend.Client,
            hasOtherUnsavedChanges: () => false);

        await viewModel.ReloadProjectDataAsync();

        Assert.False(viewModel.HasOtherPagesUnsaved);
    }

    // DispatchProxy 的宿主类不能 sealed——它要在运行时派生该类型。
    private class GitBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        public GitRepositoryStatus RepositoryStatus { get; set; } = new(
            "healthy",
            "main",
            "head-commit",
            false,
            null,
            0,
            string.Empty);

        public static GitBackend Create(bool dirty)
        {
            var client = Create<IAriadneBackendClient, GitBackend>();
            var backend = (GitBackend)(object)client;
            backend.Client = client;
            backend.RepositoryStatus = new GitRepositoryStatus(
                "healthy",
                "main",
                "head-commit",
                dirty,
                null,
                0,
                string.Empty);
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
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetGitBranchGraphAsync))
            {
                return Task.FromResult<IReadOnlyList<BranchGraphNode>>(Array.Empty<BranchGraphNode>());
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetGitHistoryAsync))
            {
                return Task.FromResult<IReadOnlyList<GitCommitSummary>>(Array.Empty<GitCommitSummary>());
            }

            throw new NotSupportedException(targetMethod?.Name);
        }
    }
}
