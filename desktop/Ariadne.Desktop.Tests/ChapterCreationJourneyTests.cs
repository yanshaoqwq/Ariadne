using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U174：**新项目里建不出东西来。** 用户报告「一些东西还不能创建」。
///
/// 本文件走**真实 sidecar 进程**，判据取「作品树里能否看到」而不是
/// 「写盘命令是否返回 ok」。理由是这簇缺陷的形态恰恰是**写盘成功但看不见**：
/// `save_document_content` 会老老实实把文件落到磁盘并返回 ok，
/// 而作品树读的是另一个数据源（`.config` 下的章节索引），两者不通。
/// 进程内 mock 与「断言命令没报错」都会照过。
///
/// 已在真实 IPC 上实测确认（见各用例注释）：
///   A. 全新项目的作品树只有一个 planning 根节点，没有任何章节容器
///   B. `save_document_content` 到新路径 → ok=true、文件落盘、
///      但作品树**完全不变**，章节不存在
///   C. 全后端**没有任何**「新建章节」命令，唯一入口是 `import_chapter`
///      （必须先有一个外部源文件）
/// </summary>
public sealed class ChapterCreationJourneyTests
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
    /// U174-A：**保存正文到新路径后，作品树里看不到这一章。**
    ///
    /// 实测（真实 sidecar）：
    ///   save_document_content(documents/chapters/ch01.md) → ok=true（文件真的写进去了）
    ///   随后 get_works_tree → 与保存前**逐字相同**，children 仍为空
    ///
    /// 根因是**两个数据源不通**（两处都已读代码确认）：
    ///   - `get_works_tree`（`commands.rs:2181-2189`）从
    ///     `load_chapter_index` 读 `.config` 下的章节索引来建树
    ///   - `save_chapter_index` 全仓**只有一个调用点**：`import_chapter`
    ///     （`commands.rs:2294`）。`save_document_content` 不碰索引。
    ///
    /// ⇒ 任何不经导入而产生的正文文件，对作品树都是隐形的。
    /// 这正是「东西创建不出来」的机制：**创建动作本身成功了，只是没人认它。**
    ///
    /// **判据取「树里出现该章节」**，不取「保存返回 ok」——
    /// 后者现在就是绿的，正是缺陷能长期存在的原因。
    /// </summary>
    [Fact]
    public async Task SavingANewChapterFile_MustMakeItVisibleInWorksTree()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(SavingANewChapterFile_MustMakeItVisibleInWorksTree)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-newchapter-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "New Chapter");

            var before = CountNodes(await client.GetWorksTreeAsync());

            // 用户视角的「新建一章」：往章节目录写一份正文。
            // document_id 必须是绝对路径（沙箱拒相对路径，CLAUDE.md 已记）。
            var chapterPath = Path.Combine(projectRoot, "documents", "chapters", "ch01.md");
            var write = await client.SaveDocumentContentAsync(
                chapterPath,
                "# 第一章\n\n正文第一句。\n");

            Assert.NotNull(write);

            // 先确认文件真的落盘了——否则失败原因是「没写成」而非「看不见」，
            // 两者的修法完全不同，必须区分开。
            Assert.True(
                File.Exists(chapterPath),
                "前提不成立：保存命令返回了，但文件并没有落盘，"
                + "此时下面的断言测的不是「树里看不见」这件事");

            var after = await client.GetWorksTreeAsync();

            Assert.True(
                CountNodes(after) > before,
                $"正文已落盘（{chapterPath}）且保存命令成功，"
                + $"但作品树节点数没有变化（{before} → {CountNodes(after)}）。"
                + "作品树读的是 .config 下的章节索引，而 save_document_content 不写索引——"
                + "于是「新建的章节」对用户完全隐形（U174-A）。");
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// U174-B：**导入是产生章节的唯一途径，因此「没有源文件就建不出章节」。**
    ///
    /// 这条用例的作用是把 A 的结论钉死在**对照**上：同一个项目里，
    /// 走 `import_chapter` 就能让章节出现在树里，走 `save_document_content` 不能。
    /// 有了对照才能证明 A 不是「树本身坏了」，而是「写入路径没接索引」。
    ///
    /// 顺带说明产品缺口：作者要新建一章，当前必须先在项目外
    /// **手工造一个文件**再导入。`display_name.json` 里也没有任何
    /// 「新建章节」文案（`ui.works.*` 下无 new/create/add 键），
    /// 后端命令表里同样没有——这不是接线漏了，是功能没有。
    /// </summary>
    [Fact]
    public async Task ImportingAChapter_IsCurrentlyTheOnlyWayToMakeOneAppear()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(ImportingAChapter_IsCurrentlyTheOnlyWayToMakeOneAppear)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-importchapter-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Import Chapter");

            var before = CountNodes(await client.GetWorksTreeAsync());

            // 作者必须先在别处有一份稿子——这一步就是产品缺口本身。
            var source = Path.Combine(temp.FullName, "外部稿子.md");
            await File.WriteAllTextAsync(source, "# 第一章\n\n从别处拿来的正文。\n");

            await client.ImportChapterAsync(new ChapterImportRequest(
                ChapterId: "ch01",
                Title: "第一章",
                Order: 1,
                SourcePath: source,
                TargetPath: Path.Combine(projectRoot, "documents", "chapters", "ch01.md"),
                Overwrite: false));

            var after = await client.GetWorksTreeAsync();

            // 导入这条路必须真的能让章节出现，否则 A 的对照不成立。
            Assert.True(
                CountNodes(after) > before,
                "导入也没能让章节出现在作品树里——"
                + "此时 U174-A 的结论需要重新判断：问题可能在树的构建而不在索引接线");

            Assert.Contains(Flatten(after), node => node.ChapterId == "ch01");
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    private static int CountNodes(WorksTreeNode root) => Flatten(root).Count();

    private static IEnumerable<WorksTreeNode> Flatten(WorksTreeNode node)
    {
        yield return node;
        foreach (var child in node.Children ?? Array.Empty<WorksTreeNode>())
        {
            foreach (var nested in Flatten(child))
            {
                yield return nested;
            }
        }
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
