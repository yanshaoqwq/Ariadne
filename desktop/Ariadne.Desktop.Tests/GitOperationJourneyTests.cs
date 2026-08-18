using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U170：Git 页全流程——改稿 → 看到脏 → 存检查点 → 历史 → 分支图 → 回档到新分支。
///
/// 走**真实 sidecar + 真实 git 仓库**，判据取 `git` 自己的可观测状态
/// （当前分支、历史条目、工作树是否干净），不取命令回执。
/// 理由：这一簇的失效形态是「回执说成功了，但仓库里没发生」——
/// 既有 `FrontendUserActionJourneyTests.UserAction_RestoreToNewBranch_...`
/// 已用真实 `git` 二进制验过回档那一步，本份把**其余动作串成完整链路**，
/// 并补上它没覆盖的：脏状态识别、检查点与历史的对应、分支图、.gitignore 生效。
/// </summary>
public sealed class GitOperationJourneyTests
{
    private static string? ResolveSidecar()
    {
        SidecarAppStateIsolation.RequireIsolatedAppState();
        var fromEnv = Environment.GetEnvironmentVariable("ARIADNE_BACKEND_IPC");
        if (!string.IsNullOrWhiteSpace(fromEnv) && File.Exists(fromEnv))
        {
            return fromEnv;
        }
        return JsonLineBackendClient.DiscoverBackendCommand(
            AppContext.BaseDirectory,
            Environment.CurrentDirectory);
    }

    /// <summary>
    /// 主线：改稿 → 状态变脏 → 存检查点 → 历史里出现 → 状态转干净。
    ///
    /// 「存完检查点后工作树必须干净」是这条的核心：若检查点只 commit 了
    /// 一部分被跟踪的文件，状态会仍然是脏——用户会以为没存上，
    /// 反复点检查点，产生一串空提交。
    ///
    /// ⚠️ **必须先把脚手架存成基线检查点再改稿**（变异测试查出来的）：
    /// `create_project` 铺完目录后工作树**本来就是脏的**（那些脚手架文件还没提交）。
    /// 第一版直接「建项目 → 写正文 → 断言脏」，于是把写正文那一步整段摘掉、
    /// 什么都不改，断言**照样通过**——它测到的是脚手架的脏，不是改稿的脏。
    /// 先存基线并**断言基线干净**，「脏」才唯一归因于用户这次改稿。
    /// </summary>
    [Fact]
    public async Task EditThenCheckpoint_ShowsUpInHistoryAndLeavesTreeClean()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(EditThenCheckpoint_ShowsUpInHistoryAndLeavesTreeClean)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-git-journey-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Git Journey");

            // ── 0. 先把脚手架收进基线，并确认基线真的干净。
            // 这一步是后面「脏」这个判据成立的前提，理由见方法注释。
            await client.CreateCheckpointAsync("基线：项目脚手架");
            var baseline = await client.GetGitRepositoryStatusAsync();
            Assert.False(
                baseline.Dirty,
                "基线检查点存完仍报脏——后面「改稿导致变脏」这个判据将无法归因，"
                + "任何改动都会被脚手架的脏掩盖");

            // ── 1. 写一章正文（被 git 跟踪的内容）
            var chapter = Path.Combine(projectRoot, "documents", "chapters", "ch01.md");
            await client.SaveDocumentContentAsync(chapter, "# 第一章\n\n初稿正文。\n");

            // ── 2. 状态必须能看出「有未提交改动」（此刻唯一的改动就是上面这一章）
            var dirty = await client.GetGitRepositoryStatusAsync();
            Assert.True(
                dirty.Dirty,
                "刚写了正文，Git 状态却报干净——用户无从判断有没有需要保存的改动");

            // ── 3. 存检查点
            const string message = "旅程检查点：第一章初稿";
            var checkpoint = await client.CreateCheckpointAsync(message);
            Assert.False(string.IsNullOrWhiteSpace(checkpoint.CommitId));

            // ── 4. 历史里必须出现这条（判据取历史列表，不取检查点回执）
            var history = await client.GetGitHistoryAsync();
            Assert.Contains(
                history,
                commit => commit.CommitId == checkpoint.CommitId);
            Assert.Contains(history, commit => commit.Summary.Contains("第一章初稿", StringComparison.Ordinal));

            // ── 5. 存完必须干净——否则用户会反复点检查点、堆出一串空提交
            var afterCheckpoint = await client.GetGitRepositoryStatusAsync();
            Assert.False(
                afterCheckpoint.Dirty,
                "存完检查点后工作树仍报脏：说明有被跟踪的文件没被提交进去，"
                + "用户会以为没存上而反复点存档");

            // ── 6. 真实 git 也要认这条提交（后端自己的视图可能有缓存）
            Assert.Contains(message, RunGit(projectRoot, "log", "-1", "--pretty=%s"));
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// 回档到新分支：真实 git 的当前分支必须切过去，且历史基点正确。
    ///
    /// 判据取 `git rev-parse --abbrev-ref HEAD` 与 `git log`——
    /// 后端回执里的 `new_branch` 只是它**打算**做的事。
    /// </summary>
    [Fact]
    public async Task RestoreToNewBranch_ActuallyMovesRealGitHeadAndKeepsOldBranch()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(RestoreToNewBranch_ActuallyMovesRealGitHeadAndKeepsOldBranch)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-git-restore-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Git Restore");
            var chapter = Path.Combine(projectRoot, "documents", "chapters", "ch01.md");

            // 造两个检查点，回档到第一个。
            await client.SaveDocumentContentAsync(chapter, "# 第一章\n\n第一版。\n");
            var first = await client.CreateCheckpointAsync("第一版");

            var details = await client.GetDocumentContentDetailsAsync(chapter);
            await client.SaveDocumentContentAsync(chapter, "# 第一章\n\n第二版。\n", details.Metadata?.Version);
            await client.CreateCheckpointAsync("第二版");

            var branchBefore = RunGit(projectRoot, "rev-parse", "--abbrev-ref", "HEAD").Trim();

            const string restored = "restore-journey";
            var report = await client.RestoreToNewBranchAsync(first.CommitId, restored);
            Assert.Equal(restored, report.NewBranch);

            // 真实 git 的 HEAD 必须在新分支上。
            Assert.Equal(
                restored,
                RunGit(projectRoot, "rev-parse", "--abbrev-ref", "HEAD").Trim());

            // 新分支的内容必须是回档目标那一版。
            Assert.Contains("第一版", await File.ReadAllTextAsync(chapter), StringComparison.Ordinal);

            // 原分支必须还在——回档是「开新分支」而不是「丢历史」。
            Assert.Contains(branchBefore, RunGit(projectRoot, "branch", "--list"));
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// 分支图必须与真实 git 的提交数一致，且标出 HEAD。
    ///
    /// 判据取「图里的 commit 集合 ⊇ git log 的集合」+「恰有一个 is_head」。
    /// 只断言「图非空」测不出漏提交——那是分支图最常见的缺陷形态
    /// （限流 limit 把最近的提交截掉，用户看不到自己刚存的检查点）。
    /// </summary>
    [Fact]
    public async Task BranchGraph_CoversEveryRealCommitAndMarksExactlyOneHead()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(BranchGraph_CoversEveryRealCommitAndMarksExactlyOneHead)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-git-graph-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Git Graph");
            var chapter = Path.Combine(projectRoot, "documents", "chapters", "ch01.md");

            string? version = null;
            for (var i = 1; i <= 3; i++)
            {
                await client.SaveDocumentContentAsync(chapter, $"# 第一章\n\n第 {i} 版。\n", version);
                await client.CreateCheckpointAsync($"第 {i} 版");
                version = (await client.GetDocumentContentDetailsAsync(chapter)).Metadata?.Version;
            }

            var graph = await client.GetGitBranchGraphAsync();
            var realCommits = RunGit(projectRoot, "log", "--pretty=%H")
                .Split('\n', StringSplitOptions.RemoveEmptyEntries)
                .Select(line => line.Trim())
                .ToHashSet(StringComparer.Ordinal);

            var graphCommits = graph.Select(node => node.CommitId).ToHashSet(StringComparer.Ordinal);
            var missing = realCommits.Except(graphCommits).ToList();

            Assert.True(
                missing.Count == 0,
                $"分支图漏了 {missing.Count} 个真实提交（共 {realCommits.Count} 个）："
                + string.Join(", ", missing.Select(id => id[..7]))
                + "。用户在时间线上看不到自己存过的检查点。");

            // 恰好一个 HEAD：零个 = 用户不知道自己在哪；多个 = 图的语义坏了。
            Assert.Equal(1, graph.Count(node => node.IsHead));
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// `ignored_paths` 必须真的让文件**进不了提交**——判据取 `git show --name-only`。
    ///
    /// 既有 `FrontendSettingsAndPerfJourneyTests.Settings_GitConfig_RoundTrips...`
    /// 只验了这个列表**能存能读**。存得下不等于生效，而「设置往返成功但从未影响
    /// 提交内容」会让用户以为某些目录被排除了、实际全被提交上去。
    ///
    /// ⚠️ **判据不能用 `git status`**（我第一版就是这么写错的）：
    /// `ignored_paths` 不写 `.gitignore`，它是**暂存期的 exclude pathspec**
    /// （`git/service.rs:699-707` 拼 `:(exclude,top,literal)…`）。
    /// 所以被排除的文件在 `git status` 里**照样显示为未跟踪**——那是正确行为，
    /// 不是缺陷。真正的契约是「不会被 commit 进去」，只有查提交内容才测得到。
    /// </summary>
    [Fact]
    public async Task IgnoredPathsSetting_ActuallyKeepsFilesOutOfCommits()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(IgnoredPathsSetting_ActuallyKeepsFilesOutOfCommits)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-git-ignore-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Git Ignore");

            var current = await client.GetGitSettingsAsync();
            await client.SaveGitSettingsAsync(new GitSettings(new GitConfig(
                current.Git.SchemaVersion,
                current.Git.TrackDocuments,
                current.Git.TrackWorkflows,
                current.Git.TrackSkills,
                current.Git.TrackNonSensitiveConfig,
                new[] { "scratch" })));

            // 一个应当被排除的文件，和一个应当被提交的文件——**必须同时有**：
            // 只放被排除的那个，检查点可能因为「无改动」而根本没提交，
            // 那时断言会因为错误的原因通过。
            var scratch = Path.Combine(projectRoot, "scratch");
            Directory.CreateDirectory(scratch);
            await File.WriteAllTextAsync(Path.Combine(scratch, "草稿.txt"), "临时内容");
            await client.SaveDocumentContentAsync(
                Path.Combine(projectRoot, "documents", "chapters", "ch01.md"),
                "# 第一章\n\n应当被提交的正文。\n");

            var checkpoint = await client.CreateCheckpointAsync("排除验证");
            var committed = RunGit(projectRoot, "show", "--name-only", "--pretty=format:", checkpoint.CommitId);

            // 对照项：正常内容确实进了提交，证明这次提交不是空的。
            Assert.Contains("ch01.md", committed, StringComparison.Ordinal);

            // 主判据：被排除的路径没进提交。
            Assert.DoesNotContain("scratch", committed, StringComparison.Ordinal);
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    // ════════════════════════════════════════════════════════
    // 真实 git
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 直接调 `git` 二进制取仓库真相。
    ///
    /// **不用任何 git 库**：本文件的全部价值就在于「后端说做了」与
    /// 「仓库里真的发生了」之间的对照。用后端同一套代码去验证等于自证。
    /// </summary>
    private static string RunGit(string projectRoot, params string[] args)
    {
        var startInfo = new System.Diagnostics.ProcessStartInfo("git")
        {
            WorkingDirectory = projectRoot,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
        };
        foreach (var arg in args)
        {
            startInfo.ArgumentList.Add(arg);
        }

        using var process = System.Diagnostics.Process.Start(startInfo);
        Assert.NotNull(process);
        var stdout = process!.StandardOutput.ReadToEnd();
        var stderr = process.StandardError.ReadToEnd();
        process.WaitForExit(30_000);

        Assert.True(
            process.ExitCode == 0,
            $"git {string.Join(' ', args)} 失败（exit {process.ExitCode}）：{stderr}");

        return stdout;
    }

    private static void TryCleanup(DirectoryInfo temp)
    {
        try
        {
            temp.Delete(recursive: true);
        }
        catch
        {
            // 清理失败不影响断言结论。
        }
    }
}
