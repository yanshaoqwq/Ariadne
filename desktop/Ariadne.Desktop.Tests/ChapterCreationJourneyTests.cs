using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U174：**新项目里建不出东西来。** 用户报告「一些东西还不能创建」。
///
/// 根因（比原始症状描述准确得多）：**「新建章节」这个动作此前在全应用不存在**——
/// 后端无命令、桌面端无入口、语言包无文案。三层都缺，不是接线漏了。
/// 于是作者拿到一个空项目后，要写第一章必须**先在项目外手工造一个 .md 再走导入**。
/// 原始报告里那句「保存正文后作品树看不见」只是它的一个侧面：
/// `save_document_content` 确实会落盘并返回 ok，但它不写章节索引，
/// 而作品树读的正是索引 ⇒ 文件在磁盘上、命令成功、用户什么也看不到。
///
/// 本文件走**真实 sidecar 进程**，判据取「作品树里能否看到」而不是
/// 「写盘命令是否返回 ok」——后者在缺陷存在时就是绿的，正是它能长期潜伏的原因。
///
/// 【2026-08-18 验证结论】U174 是**真 P1**，已修：后端新增 `create_chapter`
/// （文档 + 索引在同一个项目互斥守卫内一次落地）、`IAriadneBackendClient.CreateChapterAsync`、
/// `WorksPageViewModel` 的新建模式、作品页工具栏与空态的两个入口、三份语言包 7 个 key。
/// </summary>
[Collection("RealSidecar")]
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
    /// U174-A：**新建一章 → 作品树里看得见 → 能写正文 → 重新加载后还在。**
    ///
    /// 这条是用户报告的直接判据。四步缺一不可，各自挡住一类失效：
    ///   1. 建 ——「创建」动作存在（缺陷形态：全应用没有这个动作）
    ///   2. 树里可见 —— 索引真的登记了（缺陷形态：写盘成功但树读的是另一个数据源）
    ///   3. 能写正文并读回 —— 新建出来的不是个打不开的空壳
    ///   4. 重新读索引后还在 —— 登记真的落盘了，不只活在内存里
    ///
    /// **刻意不取「命令返回 ok」**：`save_document_content` 在缺陷存在时也返回 ok，
    /// 那正是用户无从判断「是我操作错了还是程序坏了」的原因。
    /// </summary>
    [Fact]
    public async Task CreatingAChapter_MakesItVisibleAndWritableAndItSurvivesReload()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(CreatingAChapter_MakesItVisibleAndWritableAndItSurvivesReload)))
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

            // ── 1. 建：走产品真正的「新建章节」路径。
            var chapterPath = Path.Combine(projectRoot, "documents", "chapters", "ch01.md");
            await client.CreateChapterAsync(new ChapterCreateRequest(
                ChapterId: "ch01",
                Title: "第一章",
                Order: 1,
                TargetPath: chapterPath,
                InitialContent: string.Empty));

            // ── 2. 树里可见：判据取树，不取上一步的回执。
            var after = await client.GetWorksTreeAsync();
            Assert.True(
                CountNodes(after) > before,
                $"新建章节成功返回，但作品树节点数没有变化（{before} → {CountNodes(after)}）。"
                + "作品树读的是 .config 下的章节索引——若新建没有登记索引，"
                + "章节对用户完全隐形（U174-A 的原始形态）。");

            var node = Flatten(after).FirstOrDefault(candidate => candidate.ChapterId == "ch01");
            Assert.True(
                node is not null,
                "作品树里找不到刚建的 ch01，用户无从打开它");

            // ── 3. 能写正文并读回：新建出来的必须是能真正开始写的章节，不是打不开的空壳。
            const string firstSentence = "# 第一章\n\n她推开门，风雪灌进来。\n";
            await client.SaveDocumentContentAsync(chapterPath, firstSentence);
            var readBack = await client.GetDocumentContentAsync(chapterPath);
            Assert.Equal(firstSentence, readBack);

            // ── 4. 重新加载后还在：索引必须真的落盘，不能只活在 sidecar 内存里。
            //     判据取「重新读一次树」而不是复用上面那份 payload——
            //     后者在「登记只写进内存」时照样是绿的。
            var reloaded = await client.GetWorksTreeAsync();
            Assert.Contains(Flatten(reloaded), item => item.ChapterId == "ch01");
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// U174-B：**导入仍然可用**（保留原对照）。
    ///
    /// 这条是 A 的对照：同一个项目里两条产生章节的路径都必须让章节出现在树里。
    /// 若哪天两条同时红，说明问题在**树的构建**而不在某一条写入路径的索引接线——
    /// 两者的修法完全不同。只留 A 会丢掉这个区分能力，所以**不能删**。
    /// </summary>
    [Fact]
    public async Task ImportingAChapter_AlsoMakesOneAppear()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(ImportingAChapter_AlsoMakesOneAppear)))
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

    /// <summary>
    /// U174-C：**桌面客户端的「新建章节」入口真的存在，且发出的是 `create_chapter`。**
    ///
    /// 为什么必须单独有这一条：A 只证明**后端**可用。而 U174 的三层缺口里，
    /// 「桌面端无入口」是用户实际撞上的那一层——后端命令再齐备，
    /// 作品页上没有按钮、ViewModel 里没有命令，作者依然建不出章节。
    ///
    /// 判据取**真实出站请求**（AGENTS.md 的强判据表：不是「命令能否执行」，
    /// 而是「点击后发出的请求真的带上那个 id」）：
    ///   - 发出的必须是 `CreateChapterAsync`
    ///   - **绝不能**是 `SaveDocumentContentAsync`——那条路只写文件不写索引，
    ///     是 U174 的原始缺陷形态；若哪天有人「顺手简化」成保存，
    ///     后端会返回 ok 而章节重新隐形，本断言当场转红。
    ///
    /// 用 mock 后端而不是真实 sidecar：这一条要钉的是**前端发了什么请求**，
    /// 真实进程只会把它变慢，且拿不到「发的是哪个方法」这个判据。
    /// A 那条已经覆盖了真实跨进程行为，两者分工。
    /// </summary>
    [Fact]
    public void WorksPage_HasACreateChapterEntryThatCallsCreateChapter()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, RecordingBackendProxy>();
        var recorder = (RecordingBackendProxy)(object)backend;
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);

        // 入口存在，且点开后面板真的切到新建模式（而不是复用导入表单）。
        Assert.True(vm.OpenCreateChapterPanelCommand.TryExecute());
        Assert.True(vm.IsCreateChapterMode);
        Assert.True(vm.IsImportPanelOpen);

        // 目标路径与章节 ID 必须被预填成可直接提交的值：
        // AGENTS.md「用户已知的值不要让用户手打」——后端对目标路径是精确校验，
        // 让作者凭空猜一条 documents/ 下的路径，猜错只得到一句校验错误。
        Assert.False(string.IsNullOrWhiteSpace(vm.ImportTargetPath));
        Assert.False(string.IsNullOrWhiteSpace(vm.ImportChapterId));

        vm.ImportChapterTitle = "第一章";
        Assert.True(
            vm.CreateChapterCommand.CanExecute(null),
            "填好标题后「创建」仍不可点，作者会以为程序坏了。"
            + $"（id={vm.ImportChapterId} target={vm.ImportTargetPath}）");
        Assert.True(vm.CreateChapterCommand.TryExecute());

        Assert.NotNull(recorder.LastCreateRequest);
        Assert.Equal("第一章", recorder.LastCreateRequest!.Title);
        Assert.Equal(vm.ImportChapterId, recorder.LastCreateRequest.ChapterId);

        // 关键：**不能**走 save_document_content。那条路写盘成功但不登记索引，
        // 章节对作品树隐形——正是 U174 原始缺陷。
        Assert.Empty(recorder.SavedDocumentIds);
    }

    /// <summary>
    /// 记录出站调用的 mock 后端。
    ///
    /// `DispatchProxy` 的宿主类**不能 sealed**（运行时要派生它），
    /// 见 AGENTS.md「C# 测试的三个坑」。
    /// </summary>
    private class RecordingBackendProxy : DispatchProxy
    {
        internal ChapterCreateRequest? LastCreateRequest { get; private set; }

        internal List<string> SavedDocumentIds { get; } = new();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            switch (targetMethod?.Name)
            {
                case nameof(IAriadneBackendClient.CreateChapterAsync):
                    LastCreateRequest = args?[0] as ChapterCreateRequest;
                    return Task.FromResult(new ChapterDocumentIndexResult(
                        "1", Array.Empty<ChapterIndexEntry>()));
                case nameof(IAriadneBackendClient.SaveDocumentContentAsync):
                    SavedDocumentIds.Add(args?[0] as string ?? string.Empty);
                    break;
                case nameof(IAriadneBackendClient.GetCurrentProjectAsync):
                    // 返回 null 表示「尚未打开项目」：本用例不依赖项目根，
                    // 路径校验对相对路径不需要它。
                    return Task.FromResult<CurrentProjectStatus?>(null);
                case "get_HasProjectRoot":
                    // U208-B 之后必须返回 true。
                    //
                    // 本用例演的是「作者在作品页里新建章节」——那个场景**隐含项目已打开**
                    // （作品页的章节树就是从项目里读出来的）。原先返回 false 只是当初图省事：
                    // 那时 `CanCreateChapter` 还没有项目闸，返回什么都不影响结论。
                    // `034011f`（U208-B）给它加了闸之后，这个替身就在描述一个
                    // **本用例并不想演的场景**，于是「填好标题后创建可点」永远为假。
                    //
                    // ⚠️ 修法刻意是**改替身**而不是放宽 `CanCreateChapter`：
                    // 那个闸正是 U208-B 的修复本体（未开项目时按钮可点可填可提交，
                    // 走到后端才被拒，而拒绝话术把作者引向「检查自己刚填的字」）。
                    // 为了让用例变绿去削弱产品，等于把缺陷改回来 ——
                    // 本仓已记：**「改了产品没同批改用例」的正解是改用例，
                    // 前提是先确认用例演的场景到底是什么。**
                    return true;
            }

            // 其余方法返回该方法返回类型的已完成 Task，避免 VM 里 await null 崩掉。
            return CompletedFor(targetMethod);
        }

        private static object? CompletedFor(MethodInfo? method)
        {
            var returnType = method?.ReturnType;
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
            return returnType is { IsValueType: true } ? Activator.CreateInstance(returnType) : null;
        }
    }

    /// <summary>
    /// U174-D：**「新建」撞名时必须拒绝，且绝不能碰已有正文。**
    ///
    /// 为什么这条不可省：`create_chapter` 刻意**不提供** overwrite——
    /// 「覆盖已有章节」的语义是替换正文，那属于保存或导入。
    /// 若新建能覆盖，作者手滑重建一次「第一章」就会静默毁掉已写的三万字，
    /// 而且没有任何提示（文件已被替换、命令返回 ok）。
    ///
    /// **判据取「原稿逐字未变」，而不只是「抛了异常」**——这是前任在 U173 踩出来的教训：
    /// 护栏有两道且互相独立（`commands.rs` 的索引冲突检查 + `documents/service.rs`
    /// 的 `create_only && try_exists()`），两道同时摘会红在 `Assert.ThrowsAsync` 上，
    /// 对「正文有没有被写坏」这个真正要保的性质**一无所知**。
    /// 所以真正测得准的变异是「保留检查、把它挪到写入之后」的半写形态，
    /// 而那种形态只有本条最后一行断言拦得住。
    /// </summary>
    [Fact]
    public async Task CreatingAChapterTwice_IsRejectedAndLeavesTheExistingDraftByteIdentical()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(CreatingAChapterTwice_IsRejectedAndLeavesTheExistingDraftByteIdentical)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-dupchapter-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Dup Chapter");

            var chapterPath = Path.Combine(projectRoot, "documents", "chapters", "ch01.md");
            await client.CreateChapterAsync(new ChapterCreateRequest(
                ChapterId: "ch01",
                Title: "第一章",
                Order: 1,
                TargetPath: chapterPath,
                InitialContent: string.Empty));

            // 作者已经在这一章里写了正文——这就是「不能被静默毁掉」的那份稿子。
            const string draft = "# 第一章\n\n她推开门，风雪灌进来。这一句必须活下来。\n";
            await client.SaveDocumentContentAsync(chapterPath, draft);

            // 重复新建同一章：必须被拒。
            await Assert.ThrowsAnyAsync<Exception>(() => client.CreateChapterAsync(
                new ChapterCreateRequest(
                    ChapterId: "ch01",
                    Title: "第一章（手滑重建）",
                    Order: 1,
                    TargetPath: chapterPath,
                    InitialContent: string.Empty)));

            // 关键判据：原稿**逐字未变**。
            // 只断言「抛了异常」挡不住「先写后检」的半写形态——
            // 那种形态同样会抛，但正文已经被空内容盖掉了。
            var surviving = await client.GetDocumentContentAsync(chapterPath);
            Assert.Equal(draft, surviving);
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
