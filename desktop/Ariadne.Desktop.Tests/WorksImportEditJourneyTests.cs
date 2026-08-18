using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U173：作品页「导入 → 编辑 → 导出」全流程，走**真实 sidecar 进程**。
///
/// 与既有 `FrontendUserActionJourneyTests` 的分工：那份测单个动作的往返
/// （保存/读回/版本冲突）。本份测**跨动作的连贯性**——导入进来的章节
/// 能不能接着编辑、编辑完能不能导出、导出物里是不是编辑后的内容。
/// 这些缺陷只在动作**串起来**时才暴露：每一步单独看都成功，
/// 但下一步读到的是上一步之前的状态。
///
/// **判据一律取磁盘产物或后端再次读回的结果**，不取「命令返回 ok」。
/// 理由是这一簇的失效形态都是「写成功了但另一条读路径看不见」——
/// U174 已经实测出一个（save_document_content 写盘成功、作品树看不见）。
/// </summary>
public sealed class WorksImportEditJourneyTests
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
    /// 全流程主线：导入外部稿 → 在树里找到它 → 读正文 → 改正文 → 再读回 → 导出。
    ///
    /// 每一步的判据都取**下一步能否看到上一步的结果**，
    /// 这样任何一环「只在内存里生效」都会当场断链。
    /// </summary>
    [Fact]
    public async Task ImportThenEditThenExport_EachStepSeesThePreviousResult()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(ImportThenEditThenExport_EachStepSeesThePreviousResult)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-works-journey-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Works Journey");

            // ── 1. 导入：源稿在**项目外**（U163-B 记录过这是正常用例，后端明确支持）
            var source = Path.Combine(temp.FullName, "下载", "第一章.md");
            Directory.CreateDirectory(Path.GetDirectoryName(source)!);
            const string imported = "# 第一章\n\n她推开门，风雪灌进来。\n";
            await File.WriteAllTextAsync(source, imported);

            var targetPath = Path.Combine(projectRoot, "documents", "chapters", "ch01.md");
            await client.ImportChapterAsync(new ChapterImportRequest(
                ChapterId: "ch01",
                Title: "第一章",
                Order: 1,
                SourcePath: source,
                TargetPath: targetPath,
                Overwrite: false));

            // ── 2. 树里必须能找到（判据取树，不取导入回执）
            var tree = await client.GetWorksTreeAsync();
            var node = Flatten(tree).FirstOrDefault(candidate => candidate.ChapterId == "ch01");
            Assert.True(node is not null, "导入成功但章节没出现在作品页的树里，用户无从打开它");

            // ── 3. 读正文：内容必须是导入进来的那份，而不是空或占位
            var afterImport = await client.GetDocumentContentDetailsAsync(targetPath);
            Assert.Contains("风雪灌进来", afterImport.Content, StringComparison.Ordinal);

            // ── 4. 编辑：带上刚读到的版本号（这是作品页真实的保存路径）
            const string edited = "# 第一章\n\n她推开门，风雪灌进来，灯灭了。\n";
            await client.SaveDocumentContentAsync(targetPath, edited, afterImport.Metadata?.Version);

            // ── 5. 再读回：必须是编辑后的内容（判据取后端再读，不取保存回执）
            var afterEdit = await client.GetDocumentContentDetailsAsync(targetPath);
            Assert.Contains("灯灭了", afterEdit.Content, StringComparison.Ordinal);

            // 磁盘也要一致——后端缓存住旧值也算缺陷。
            Assert.Contains("灯灭了", await File.ReadAllTextAsync(targetPath), StringComparison.Ordinal);

            // ── 6. 导出：产出物里必须是**编辑后**的正文，不是导入时的原稿
            var export = await client.ExportChaptersAsync(new[] { "ch01" });
            Assert.NotNull(export);

            var exportedText = await ReadExportedTextAsync(export, projectRoot);
            Assert.Contains("灯灭了", exportedText, StringComparison.Ordinal);
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// 不带 overwrite 重复导入同一个 chapter_id 必须被拒，且**不能破坏已有正文**。
    ///
    /// 这条测的是「拒绝要干净」：后端 `commands.rs:2271-2279` 会在冲突时返回
    /// conflict，但那个检查发生在**写入之前还是之后**决定了正文会不会被截断。
    /// 判据取「被拒之后原正文逐字未变」——只断言「抛了异常」测不出半写。
    /// </summary>
    [Fact]
    public async Task ReimportingSameChapterWithoutOverwrite_IsRejectedAndLeavesProseIntact()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(ReimportingSameChapterWithoutOverwrite_IsRejectedAndLeavesProseIntact)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-works-reimport-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Reimport");

            var source = Path.Combine(temp.FullName, "源稿.md");
            await File.WriteAllTextAsync(source, "# 第一章\n\n初版正文。\n");
            var targetPath = Path.Combine(projectRoot, "documents", "chapters", "ch01.md");

            await client.ImportChapterAsync(new ChapterImportRequest(
                "ch01", "第一章", 1, source, targetPath, Overwrite: false));

            // 作者已经在应用里改过一轮——这是「重复导入会不会毁稿」的关键前提。
            var details = await client.GetDocumentContentDetailsAsync(targetPath);
            const string authored = "# 第一章\n\n作者亲手改过的正文，不能被覆盖。\n";
            await client.SaveDocumentContentAsync(targetPath, authored, details.Metadata?.Version);

            // 换一份不同的源稿，再导入同一个 chapter_id。
            await File.WriteAllTextAsync(source, "# 第一章\n\n完全不同的第二版。\n");

            await Assert.ThrowsAsync<BackendException>(() => client.ImportChapterAsync(
                new ChapterImportRequest("ch01", "第一章", 1, source, targetPath, Overwrite: false)));

            // 关键判据：被拒之后，作者的正文必须**逐字未变**。
            Assert.Equal(authored, await File.ReadAllTextAsync(targetPath));
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// 导出产物必须落在**作者能找到**的地方（exports/ 下），而不是 .runtime 内部目录。
    ///
    /// U134 的注释记着这个坑：artifactId 传 null 时由后端命名，
    /// 前端自己拼会让文件静默落到 `.runtime/artifacts` 且互相覆盖。
    /// 这条把「产物可被作者找到」钉成断言——判据取**真实文件路径的位置**。
    /// </summary>
    [Fact]
    public async Task ExportedArtifact_LandsWhereTheAuthorCanFindIt()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(ExportedArtifact_LandsWhereTheAuthorCanFindIt)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-works-export-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Export Place");

            var source = Path.Combine(temp.FullName, "稿.md");
            await File.WriteAllTextAsync(source, "# 第一章\n\n可导出的正文。\n");
            await client.ImportChapterAsync(new ChapterImportRequest(
                "ch01", "第一章", 1, source,
                Path.Combine(projectRoot, "documents", "chapters", "ch01.md"),
                Overwrite: false));

            var export = await client.ExportChaptersAsync(new[] { "ch01" });

            var resolved = ResolveExportPath(export, projectRoot);
            Assert.True(
                resolved is not null && File.Exists(resolved),
                $"导出回执没有指向一个真实存在的文件（storage_uri={export?.StorageUri}）——"
                + "作者点了导出却找不到产物");

            Assert.DoesNotContain(
                ".runtime",
                resolved!,
                StringComparison.Ordinal);
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    // ════════════════════════════════════════════════
    // 工具
    // ════════════════════════════════════════════════

    /// <summary>
    /// 从导出回执的 `storage_uri` 解析出真实文件路径。
    ///
    /// 找不到时返回 null 让调用方断言失败，**不抛异常**：
    /// 抛出来会把「产物不存在」伪装成「测试代码坏了」，两者的修法完全不同。
    /// </summary>
    private static string? ResolveExportPath(CombinedExportReport? report, string projectRoot)
    {
        var raw = report?.StorageUri;
        if (string.IsNullOrWhiteSpace(raw))
        {
            return null;
        }

        // storage_uri 可能是 file:// 形式、绝对路径，或项目相对路径。
        var candidate = raw.StartsWith("file://", StringComparison.Ordinal)
            ? new Uri(raw).LocalPath
            : raw;

        if (Path.IsPathRooted(candidate))
        {
            return File.Exists(candidate) ? candidate : null;
        }

        var joined = Path.Combine(projectRoot, candidate);
        return File.Exists(joined) ? joined : null;
    }

    private static async Task<string> ReadExportedTextAsync(
        CombinedExportReport? report,
        string projectRoot)
    {
        var path = ResolveExportPath(report, projectRoot);
        Assert.True(
            path is not null,
            $"导出产物找不到，无法验证内容（storage_uri={report?.StorageUri}）");
        return await File.ReadAllTextAsync(path!);
    }

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
