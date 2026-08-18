using System.Text.Json;
using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 桌面端**画布编辑动作**的真实 sidecar 全流程：整图保存、图校验、节点细节 patch、
/// 断点、批注、片段导出、子流程打包。
///
/// **立此文件的依据（先查过覆盖再写）**：把 `IAriadneBackendClient` 的 97 个方法
/// 与三个既有 journey 文件比对，画布这一簇有 8 个方法零真实-sidecar 覆盖——
/// ValidateWorkflowGraph / SaveProjectCanvas / LoadProjectCanvas /
/// ApplyNodeDetailPatch / SetNodeBreakpoint / UpsertCanvasAnnotation /
/// ExportWorkflowSelection / PackWorkflowSelection / ListWorkflowGraphs。
/// 它们**有测试**（`WorkspaceCanvas08Tests` 等），但用的是 `DispatchProxy` 假客户端：
/// 不做 serde 反序列化、不落盘、不过进程边界。
///
/// **判据一律取 `workflows/default.json` 的磁盘内容**，不取「命令没报错」。
/// 理由是这一簇全部是「改了画布」类动作，而后端返回值都是新图——
/// 一个只在内存里改、忘记落盘的实现会照样返回一张改过的图。
/// 磁盘骗不过去。
///
/// 与另三个 journey 文件的分工：
/// - <see cref="FrontendProductionJourneyTests"/>：通用 llm 节点跑到终态
/// - <see cref="FrontendWritingChainJourneyTests"/>：写作链（产出→审批→落盘→导出）
/// - <see cref="FrontendUserActionJourneyTests"/>：正文编辑与运行中途干预
/// - 本文件：**画布本身的编辑与持久化**
///
/// sidecar 未编译时按 <see cref="SidecarAppStateIsolation"/> 约定**显式失败**，
/// 不静默跳过（U156 的教训：xUnit 把 `return` 记成绿）。
/// </summary>
[Collection("RealSidecar")]
public sealed class FrontendCanvasEditJourneyTests : IDisposable
{
    private const string ProviderId = "primary";
    private const string ModelId = "canvas-model";

    /// <summary>
    /// 后端把一切画布写操作都归一到这个 id（`PROJECT_CANVAS_WORKFLOW_ID`，
    /// `core/src/workflow/project_canvas.rs:7`），桌面端常量 `DefaultWorkflowId`
    /// 与它同值。测试直接读这个文件，不猜路径。
    /// </summary>
    private const string CanvasWorkflowId = "default";

    private readonly DirectoryInfo _temp =
        Directory.CreateTempSubdirectory("ariadne-canvas-edit-");

    public void Dispose()
    {
        try
        {
            _temp.Delete(recursive: true);
        }
        catch
        {
            // 清理失败不影响断言结论。
        }
    }

    private static string? ResolveSidecar()
    {
        SidecarAppStateIsolation.RequireIsolatedAppState();
        SidecarAppStateIsolation.UseSharedSecretMasterKey();

        var fromEnv = Environment.GetEnvironmentVariable("ARIADNE_BACKEND_IPC");
        if (!string.IsNullOrWhiteSpace(fromEnv) && File.Exists(fromEnv))
        {
            return fromEnv;
        }
        return JsonLineBackendClient.DiscoverBackendCommand(
            AppContext.BaseDirectory,
            Environment.CurrentDirectory);
    }

    // ════════════════════════════════════════════════════════
    // 磁盘判据工具
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 画布落盘路径。`workflows` 目录名可被 `app.yaml` 的 `workflows_dir` 改写，
    /// 但测试项目用默认值（`config/models.rs:1192` 的 `default_workflows_dir`）。
    /// </summary>
    private static string CanvasPath(string projectRoot)
        => Path.Combine(projectRoot, "workflows", CanvasWorkflowId + ".json");

    /// <summary>
    /// 读磁盘上的画布 JSON。**这是本文件全部判据的来源**：
    /// 后端返回值是「它认为的新图」，磁盘是「下次打开会看到的图」，两者可以不一致。
    /// </summary>
    private static JsonDocument ReadCanvasFromDisk(string projectRoot)
    {
        var path = CanvasPath(projectRoot);
        Assert.True(File.Exists(path), $"画布未落盘：{path}");
        return JsonDocument.Parse(File.ReadAllText(path));
    }

    /// <summary>按 id 取磁盘图里的节点 config；找不到节点就让断言报出实际有哪些 id。</summary>
    private static JsonElement NodeConfigFromDisk(string projectRoot, string nodeId)
    {
        using var doc = ReadCanvasFromDisk(projectRoot);
        var nodes = doc.RootElement.GetProperty("nodes");
        foreach (var node in nodes.EnumerateArray())
        {
            if (node.GetProperty("id").GetString() == nodeId)
            {
                // JsonDocument 会随 using 释放，克隆出独立元素再返回。
                return node.GetProperty("config").Clone();
            }
        }
        var ids = string.Join(", ", nodes.EnumerateArray()
            .Select(node => node.GetProperty("id").GetString()));
        throw new InvalidOperationException($"磁盘图里没有节点 {nodeId}；实际节点=[{ids}]");
    }

    private static async Task ConfigureProviderAsync(
        IAriadneBackendClient client, string? baseUrl = null)
    {
        await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            ProviderId, "open_ai_compatible", "我的模型服务", true,
            baseUrl ?? "http://127.0.0.1:1",
            new[] { new ModelConfig(ModelId, "llm", null, null, null) },
            true, false, false, false));
        await client.SaveProviderKeyAsync(ProviderId, "sk-canvas-edit");
    }

    private static CanvasNode LlmNode(string id, string prompt, double x = 0, double y = 0)
        => new(id, "llm", null, new Dictionary<string, object?>
        {
            ["provider_id"] = ProviderId,
            ["model_id"] = ModelId,
            ["prompt_template"] = prompt,
        }, new CanvasPosition(x, y));

    /// <summary>
    /// 建项目并落一张可用的初始画布，返回 (项目根, 保存后的图)。
    ///
    /// 必须先取一次 `LoadProjectCanvasAsync` 拿 `content_revision`：
    /// 后端 `save_workflow_graph_locked`（`commands.rs:5713`）在文件已存在而
    /// `expected_revision` 为 null 时**直接拒绝**，这是刻意的 CAS 保护。
    /// 新建项目已经带了一张默认画布，所以「第一次保存」也需要 revision。
    /// </summary>
    private async Task<(string ProjectRoot, WorkflowGraphData Saved)> SeedCanvasAsync(
        IAriadneBackendClient client,
        string projectName,
        IReadOnlyList<CanvasNode> nodes,
        IReadOnlyList<CanvasEdge>? edges = null)
    {
        var projectRoot = Path.Combine(_temp.FullName, projectName);
        await client.CreateProjectAsync(projectRoot, projectName);
        await ConfigureProviderAsync(client);

        var current = await client.LoadProjectCanvasAsync();
        var saved = await client.SaveProjectCanvasAsync(new WorkflowGraphData(
            CanvasWorkflowId,
            projectName,
            nodes,
            edges ?? Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: null,
            ExpectedRevision: current.ContentRevision));
        return (projectRoot, saved);
    }

    // ════════════════════════════════════════════════════════
    // 动作 1：保存整图（作者按「保存」）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 保存画布 → 磁盘 JSON 里有那些节点 → 重新加载读回同一张图。
    ///
    /// **判据取磁盘 + 二次加载两层**：只断言返回值等于送进去的图，
    /// 一个「原样回显但没写盘」的实现照样通过；只断言磁盘，
    /// 又漏掉「写盘了但读回路径解析到别处」。两层一起才闭合。
    /// </summary>
    [Fact]
    public async Task Canvas_SaveGraph_LandsOnDiskAndReloadsIdentically()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_SaveGraph_LandsOnDiskAndReloadsIdentically)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var (projectRoot, saved) = await SeedCanvasAsync(client, "画布保存", new[]
        {
            LlmNode("node-写", "写一段{章节}的开头。", 40, 60),
            LlmNode("node-改", "润色上一段。", 320, 60),
        });

        // 判据一：磁盘上真的有这两个节点，且位置与提示词都在。
        using (var doc = ReadCanvasFromDisk(projectRoot))
        {
            var ids = doc.RootElement.GetProperty("nodes").EnumerateArray()
                .Select(node => node.GetProperty("id").GetString()).ToArray();
            Assert.Contains("node-写", ids);
            Assert.Contains("node-改", ids);
        }

        var config = NodeConfigFromDisk(projectRoot, "node-写");
        Assert.Equal(
            "写一段{章节}的开头。",
            config.GetProperty("prompt_template").GetString());

        // 判据二：重新加载读回同一张图（排除「写到了另一个路径」）。
        var reloaded = await client.LoadProjectCanvasAsync();
        Assert.Equal(2, reloaded.Nodes.Count);
        Assert.Contains(reloaded.Nodes, node => node.Id == "node-写");

        // 保存必须给出新 revision，否则作者下一次保存无从做 CAS。
        Assert.False(string.IsNullOrWhiteSpace(saved.ContentRevision),
            "保存后必须回传 content_revision，否则后续保存会被 CAS 拒绝且用户无法恢复");
    }

    /// <summary>
    /// **并发保护**：拿过期 revision 保存必须被拒，且磁盘不得被改。
    ///
    /// 真实场景：作者在画布上改了东西，同时运行中的工作流或另一个入口
    /// （运行前自动保存、Project AI 自动保存）也写过这张图。
    ///
    /// 判据取**磁盘内容**而不是「有没有抛异常」：一个「先写盘后比对 revision」
    /// 的实现会照样抛异常，但另一方的改动已经被覆盖——不可逆的数据丢失。
    /// </summary>
    [Fact]
    public async Task Canvas_SaveWithStaleRevision_IsRejectedAndDiskUnchanged()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_SaveWithStaleRevision_IsRejectedAndDiskUnchanged)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var (projectRoot, first) = await SeedCanvasAsync(client, "画布并发", new[]
        {
            LlmNode("node-1", "第一版提示词。"),
        });
        var staleRevision = first.ContentRevision;

        // 另一方（或另一个入口）先改了一轮，revision 前进。
        var second = await client.SaveProjectCanvasAsync(new WorkflowGraphData(
            CanvasWorkflowId, "画布并发",
            new[] { LlmNode("node-1", "别人改过的提示词。") },
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: null,
            ExpectedRevision: staleRevision));
        Assert.NotEqual(staleRevision, second.ContentRevision);

        var diskBefore = await File.ReadAllTextAsync(CanvasPath(projectRoot));

        // 本方拿着**过期**的 revision 提交自己的版本。
        var error = await Assert.ThrowsAnyAsync<Exception>(() =>
            client.SaveProjectCanvasAsync(new WorkflowGraphData(
                CanvasWorkflowId, "画布并发",
                new[] { LlmNode("node-1", "我的改动，不该覆盖别人的。") },
                Array.Empty<CanvasEdge>(),
                new Dictionary<string, object?>(),
                ContentRevision: null,
                ExpectedRevision: staleRevision)));

        // 拒绝理由要能指向「版本冲突」，否则用户只看到一句「保存失败」无从处置。
        Assert.Contains("revision", error.Message, StringComparison.OrdinalIgnoreCase);

        // 判据：磁盘逐字节未变——别人的改动还在。
        Assert.Equal(diskBefore, await File.ReadAllTextAsync(CanvasPath(projectRoot)));
        Assert.Contains("别人改过的提示词", await File.ReadAllTextAsync(CanvasPath(projectRoot)));
    }

    // ════════════════════════════════════════════════════════
    // 动作 2：图校验（保存前的预检）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// **预检必须与保存边界同口径**：`ValidateWorkflowGraph` 放行的图，
    /// `SaveProjectCanvas` 也必须接受；预检拒绝的，保存也必须拒绝。
    ///
    /// 这条是 U113 的形状：预检用默认限制、保存用项目真实限制时，
    /// 用户会看到「校验通过但保存失败」这种自相矛盾的结果，且无从判断该改什么。
    /// 判据取**两个命令对同一张图的结论是否一致**，而不是各自单独是否报错。
    /// </summary>
    [Fact]
    public async Task Canvas_ValidateAndSave_AgreeOnTheSameGraph()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_ValidateAndSave_AgreeOnTheSameGraph)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "画布校验");
        await client.CreateProjectAsync(projectRoot, "画布校验");
        await ConfigureProviderAsync(client);
        var current = await client.LoadProjectCanvasAsync();

        // 一张合法图：预检过 → 保存也过。
        var good = new WorkflowGraphData(
            CanvasWorkflowId, "画布校验",
            new[] { LlmNode("node-1", "合法的提示词。") },
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: null,
            ExpectedRevision: current.ContentRevision);
        await client.ValidateWorkflowGraphAsync(good);
        var saved = await client.SaveProjectCanvasAsync(good);
        Assert.Single(saved.Nodes);

        // 一张非法图：condition 节点的 input_alias 没有对应入边
        // （`integration.rs:1666` 的 require_incoming_data_alias）。
        // 选这个缺陷是因为它**只能靠图的整体结构判定**——单看节点 config 是合法的，
        // 所以能区分「真的做了拓扑校验」与「只做了字段存在性检查」。
        var bad = new WorkflowGraphData(
            CanvasWorkflowId, "画布校验",
            new[]
            {
                new CanvasNode("node-判", "condition", null, new Dictionary<string, object?>
                {
                    ["input_alias"] = "上游结论",
                    ["operator"] = "truthy",
                }, new CanvasPosition(0, 0)),
            },
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: null,
            ExpectedRevision: saved.ContentRevision);

        var validateError = await Assert.ThrowsAnyAsync<Exception>(
            () => client.ValidateWorkflowGraphAsync(bad));
        var saveError = await Assert.ThrowsAnyAsync<Exception>(
            () => client.SaveProjectCanvasAsync(bad));

        // 判据：两处都拒，且都提到那个 alias——否则用户改不动。
        Assert.Contains("上游结论", validateError.Message);
        Assert.Contains("上游结论", saveError.Message);

        // 且非法图不得落盘：磁盘上仍是上一版的合法图。
        using var doc = ReadCanvasFromDisk(projectRoot);
        var ids = doc.RootElement.GetProperty("nodes").EnumerateArray()
            .Select(node => node.GetProperty("id").GetString()).ToArray();
        Assert.Contains("node-1", ids);
        Assert.DoesNotContain("node-判", ids);
    }

    // ════════════════════════════════════════════════════════
    // 动作 3：节点细节 patch（右侧详情面板按「保存」）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 改节点的提示词 / 模型 / 预算 / 超时 → 磁盘 config 里逐项对上。
    ///
    /// **判据取磁盘 config 的每一个键**，而不是「patch 命令成功」。
    /// 理由：`NodeDetailPatch` 有 8 个字段，后端 `apply_node_detail_patch`
    /// （`frontend/service.rs:2036`）对每个字段是**独立的 if 分支**——
    /// 漏掉任何一个分支都不会报错，只是那一项静默不生效。
    /// 「命令成功」对这类缺陷零区分度。
    ///
    /// 预算与超时刻意取非整数/大数值：它们在 UI 层是 string、
    /// 后端期望 f64/u64，中间经过一次解析（`NodeConfigData.cs:143`）。
    /// </summary>
    [Fact]
    public async Task Canvas_NodeDetailPatch_EveryFieldReachesDiskConfig()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_NodeDetailPatch_EveryFieldReachesDiskConfig)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var (projectRoot, _) = await SeedCanvasAsync(client, "节点细节", new[]
        {
            LlmNode("node-1", "旧提示词。"),
        });

        await client.ApplyNodeDetailPatchAsync(CanvasWorkflowId, new NodeDetailPatch(
            "node-1",
            PromptTemplate: "新提示词：你是一位极高水平的小说家。",
            InputAliases: new Dictionary<string, string>(),
            ToolEnabled: new Dictionary<string, bool>(),
            ApprovalPolicy: new Dictionary<string, string>(),
            ModelId: "另一个模型",
            BudgetUsd: 0.35,
            TimeoutMs: 456_000));

        var config = NodeConfigFromDisk(projectRoot, "node-1");

        Assert.Equal(
            "新提示词：你是一位极高水平的小说家。",
            config.GetProperty("prompt_template").GetString());
        Assert.Equal("另一个模型", config.GetProperty("model_id").GetString());
        Assert.Equal(0.35, config.GetProperty("budget_usd").GetDouble(), 6);
        Assert.Equal(456_000, config.GetProperty("timeout_ms").GetInt64());

        // patch 不得冲掉它不负责的键：provider_id 没在 patch 里，必须还在。
        // 这一条钉的是「patch 变成整体替换」——那会让作者一改提示词就丢掉模型服务商，
        // 症状是下次运行报「找不到 provider」，与提示词改动毫无表面关联。
        Assert.Equal(ProviderId, config.GetProperty("provider_id").GetString());
    }

    /// <summary>
    /// **非法预算必须被拒，且不得部分生效**。
    ///
    /// `apply_node_detail_patch` 是「逐字段写入 config」的顺序实现，
    /// 而 budget 校验（`frontend/service.rs:2074`）在**中途**。
    /// 若实现把已改的字段先落了盘再报错，用户会得到一个半改状态：
    /// 提示词换了、预算没换、还看到一条错误——完全无法判断自己现在是什么配置。
    ///
    /// 判据取**磁盘上提示词是否仍是旧值**，这正是「有没有部分生效」的探针。
    /// </summary>
    [Fact]
    public async Task Canvas_NodeDetailPatch_NegativeBudgetRejectedWithoutPartialWrite()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_NodeDetailPatch_NegativeBudgetRejectedWithoutPartialWrite)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var (projectRoot, _) = await SeedCanvasAsync(client, "非法预算", new[]
        {
            LlmNode("node-1", "原始提示词，不该被改。"),
        });

        var error = await Assert.ThrowsAnyAsync<Exception>(() =>
            client.ApplyNodeDetailPatchAsync(CanvasWorkflowId, new NodeDetailPatch(
                "node-1",
                PromptTemplate: "这个改动不该生效。",
                InputAliases: new Dictionary<string, string>(),
                ToolEnabled: new Dictionary<string, bool>(),
                ApprovalPolicy: new Dictionary<string, string>(),
                ModelId: null,
                BudgetUsd: -1.0,
                TimeoutMs: null)));

        Assert.Contains("budget", error.Message, StringComparison.OrdinalIgnoreCase);

        // 判据：磁盘上提示词仍是旧值 —— 整个 patch 原子失败，没有半改状态。
        var config = NodeConfigFromDisk(projectRoot, "node-1");
        Assert.Equal("原始提示词，不该被改。", config.GetProperty("prompt_template").GetString());
    }

    /// <summary>
    /// patch 指向不存在的节点必须报「找不到该节点」，而不是静默无操作。
    ///
    /// 真实触发路径：作者在两个窗口开同一项目，一边删了节点、另一边还选着它按保存。
    /// 静默成功会让作者以为改动生效了。
    /// </summary>
    [Fact]
    public async Task Canvas_NodeDetailPatch_MissingNodeFailsLoudly()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_NodeDetailPatch_MissingNodeFailsLoudly)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        await SeedCanvasAsync(client, "缺节点", new[] { LlmNode("node-1", "只有这一个节点。") });

        var error = await Assert.ThrowsAnyAsync<Exception>(() =>
            client.ApplyNodeDetailPatchAsync(CanvasWorkflowId, new NodeDetailPatch(
                "node-并不存在",
                PromptTemplate: "随便什么。",
                InputAliases: new Dictionary<string, string>(),
                ToolEnabled: new Dictionary<string, bool>(),
                ApprovalPolicy: new Dictionary<string, string>(),
                ModelId: null,
                BudgetUsd: null,
                TimeoutMs: null)));

        // 报错要带上那个 id：只说「节点不存在」用户不知道是哪一个。
        Assert.Contains("node-并不存在", error.Message);
    }

    // ════════════════════════════════════════════════════════
    // 动作 4：断点（右键节点「断点」）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// **断点的判据是「运行真的停在那个节点之前」**，不是「config 里有 breakpoint:true」。
    ///
    /// 这是本文件最重要的一条。`set_node_breakpoint` 只往 config 写一个布尔值
    /// （`frontend/service.rs:2367`），而消费方在运行时另一处
    /// （`workflow/runtime.rs:3670` 的 `should_pause_for_breakpoint`）。
    /// 「写进去了」与「运行会停」是两件事——CLAUDE.md 记着这个仓库反复出现
    /// 「实现完整 + 有测试覆盖 + 生产零调用者」，断点正是这个形状的高危项：
    /// 一个只写 config、运行时压根不读的实现，任何「查 config」的断言都会全绿。
    ///
    /// 所以这里必须真跑一次工作流，判据取 `status == paused` 且
    /// `pause_reason` 指名那个节点。
    ///
    /// provider 指向一个**没人监听的端口**：断点若生效，运行在发出请求之前就停了，
    /// 压根不会连；断点若不生效，会连接失败而进入 failed——两个结局清晰可分。
    /// 这个设计让「断点没生效」不会伪装成「网络问题」。
    /// </summary>
    [Fact]
    public async Task Canvas_Breakpoint_ActuallyPausesTheRunBeforeThatNode()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_Breakpoint_ActuallyPausesTheRunBeforeThatNode)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var (projectRoot, _) = await SeedCanvasAsync(client, "断点", new[]
        {
            LlmNode("node-1", "这个节点不该被执行。"),
        });

        await client.SetNodeBreakpointAsync(CanvasWorkflowId, "node-1", enabled: true);

        // 前置判据：确实落盘了（这一层是必要但远不充分的）。
        var config = NodeConfigFromDisk(projectRoot, "node-1");
        Assert.True(config.GetProperty("breakpoint").GetBoolean(), "断点必须落盘");

        var started = await client.RunWorkflowAsync(CanvasWorkflowId);
        var state = await WaitForTerminalStateAsync(client, CanvasWorkflowId, started.RunId);

        // 核心判据：**pause_reason 必须指名 breakpoint**，而不只是「状态是 paused」。
        //
        // 首版我写的是 `status == paused` + `pause_reason 含 "node-1"`，变异测试证明它是**空测**：
        // 把 `should_pause_for_breakpoint` 摘成恒 false（模拟「只写 config、运行时不消费」，
        // 正是这条要防的缺陷）之后，测试**照样全绿**——因为 provider 连不通，
        // 重试耗尽后运行也 pause，理由里也含 "node-1"。
        // 两条断言都被一个完全无关的原因满足了。
        //
        // `runtime.rs` 里有 15 处 `self.pause(...)`，光看状态区分不出是哪一处。
        // 判据必须落在**只有断点这条路径才会产生**的那句文案上。
        Assert.Equal("paused", state.Status.ToLowerInvariant());
        Assert.NotNull(state.PauseReason);
        Assert.Contains("breakpoint", state.PauseReason!, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("node-1", state.PauseReason!);

        // 且不得是「重试耗尽」那条路径——它同样 paused、同样提到节点名。
        // 显式排除，让将来有人改文案时这条会红而不是静默退化成空测。
        Assert.DoesNotContain("exhausted retry", state.PauseReason!, StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>
    /// 关掉断点后运行不再停——否则作者会被自己设过的断点永久挡住，
    /// 而症状（「运行一点就停，什么也没发生」）与 U156 那种「运行恒定失败」难以区分。
    ///
    /// 判据取「不是 paused」而非「是 failed」：这里 provider 连不通，
    /// 终态是 failed 属预期，但**测试要断言的性质是「没有因断点而停」**。
    /// 把判据写成 `== failed` 会让它在将来 provider 可用时莫名转红。
    /// </summary>
    [Fact]
    public async Task Canvas_BreakpointDisabled_RunNoLongerPausesThere()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_BreakpointDisabled_RunNoLongerPausesThere)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var (projectRoot, _) = await SeedCanvasAsync(client, "断点关闭", new[]
        {
            LlmNode("node-1", "这个节点应当被尝试执行。"),
        });

        await client.SetNodeBreakpointAsync(CanvasWorkflowId, "node-1", enabled: true);
        await client.SetNodeBreakpointAsync(CanvasWorkflowId, "node-1", enabled: false);

        // 关闭必须真的写成 false 或移除键——留着 true 会让下面的运行仍然停住。
        var config = NodeConfigFromDisk(projectRoot, "node-1");
        var stillOn = config.TryGetProperty("breakpoint", out var flag)
            && flag.ValueKind == JsonValueKind.True;
        Assert.False(stillOn, "关掉断点后磁盘上不得仍是 true");

        var started = await client.RunWorkflowAsync(CanvasWorkflowId);
        var state = await WaitForTerminalStateAsync(client, CanvasWorkflowId, started.RunId);

        // 判据是「**没有因断点而停**」，不是「不是 paused」。
        //
        // 首版我写的是 `Assert.NotEqual("paused", status)`，实测转红：状态确实是 paused，
        // 但原因是 `node node-1 exhausted retry attempts: provider request error ...`
        // ——provider 连不通、重试耗尽后运行也会 pause。那次失败**不是缺陷**，
        // 是我把判据选粗了一层：paused 有十几个来源（`runtime.rs` 里 15 处 `self.pause`），
        // 只看状态区分不出「断点停的」与「别的原因停的」。
        //
        // 这正是 AGENTS.md 那张弱/强判据表的形状：判据选错一层，测试就成了装饰
        // ——反过来也会误伤，把正常行为报成缺陷。
        Assert.NotNull(state.PauseReason ?? state.Failure?.Message ?? state.Status);
        var pausedByBreakpoint =
            string.Equals(state.Status, "paused", StringComparison.OrdinalIgnoreCase)
            && (state.PauseReason?.Contains("breakpoint", StringComparison.OrdinalIgnoreCase)
                ?? false);
        Assert.False(
            pausedByBreakpoint,
            $"关掉断点后运行仍因断点暂停：pause_reason={state.PauseReason}");
    }

    // ════════════════════════════════════════════════════════
    // 动作 5：批注框
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 批注要能落盘并在重新加载后回到画布 metadata。
    ///
    /// 判据取**二次加载后的 metadata**：批注存在 workflow metadata 而非节点里
    /// （`frontend/service.rs:2130`），而桌面端保存整图时是把 `_canvasMetadata`
    /// 整体回写的（`WorkspacePageViewModel.cs:3788`）。
    /// 「保存时清空 metadata」这类缺陷只有走完一轮 load→save→load 才暴露——
    /// 单看 upsert 的返回值永远是对的。
    /// </summary>
    [Fact]
    public async Task Canvas_Annotation_SurvivesReloadAndSubsequentGraphSave()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_Annotation_SurvivesReloadAndSubsequentGraphSave)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var (projectRoot, _) = await SeedCanvasAsync(client, "批注", new[]
        {
            LlmNode("node-1", "被批注的节点。"),
        });

        await client.UpsertCanvasAnnotationAsync(CanvasWorkflowId, new CanvasAnnotation(
            "annotation-1",
            "这一段是第三卷的伏笔",
            new[] { "node-1" },
            new Dictionary<string, object?>()));

        // 判据一：磁盘 metadata 里有它。
        using (var doc = ReadCanvasFromDisk(projectRoot))
        {
            var annotations = doc.RootElement
                .GetProperty("metadata")
                .GetProperty("canvas_annotations");
            Assert.Equal(1, annotations.GetArrayLength());
            Assert.Equal(
                "这一段是第三卷的伏笔",
                annotations[0].GetProperty("title").GetString());
        }

        // 判据二：**再存一次整图之后批注还在**。
        // 这一步是本条的关键：作者加完批注几乎一定会继续改图并保存，
        // 若整图保存把 metadata 清掉，批注功能等于不存在，而 upsert 本身完全正常。
        var loaded = await client.LoadProjectCanvasAsync();
        var resaved = await client.SaveProjectCanvasAsync(new WorkflowGraphData(
            CanvasWorkflowId, "批注",
            loaded.Nodes,
            loaded.Edges,
            loaded.Metadata,
            ContentRevision: null,
            ExpectedRevision: loaded.ContentRevision));
        Assert.NotNull(resaved);

        using var after = ReadCanvasFromDisk(projectRoot);
        Assert.True(
            after.RootElement.GetProperty("metadata")
                .TryGetProperty("canvas_annotations", out var kept)
            && kept.GetArrayLength() == 1,
            "整图保存后批注消失——作者每次保存都会丢掉自己写的批注");
    }

    // ════════════════════════════════════════════════════════
    // 动作 6：导出片段 —— 这一簇里唯一「点了什么都不会产生」的动作
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// **导出必须真的产生一个用户能拿到的文件。**
    ///
    /// 这条是本文件立项的直接原因。读代码时发现整条「导出」链路上没有任何写盘：
    /// - 后端 `commands.rs:2437` 的 `export_workflow_selection` 只 load + 组装，返回一个结构体
    /// - 服务层 `frontend/service.rs:2379` 纯函数，无 IO
    /// - 前端 `WorkspacePageViewModel.cs:2851` `await ...ExportWorkflowSelectionAsync(...)`
    ///   **丢掉返回值**，紧接着就把状态栏写成「已导出 N 个节点」
    /// - 工作区页没有任何保存文件对话框（只有 OpenFolder / OpenFile 两个**读**取器）
    ///
    /// 对照：真正会落盘的章节导出走 `export_chapters`（`commands.rs:2311`），
    /// 它有 artifact 写入与 `storage_uri`。工作流导出没有对应物。
    ///
    /// 于是用户点「导出图」，看到「已导出 2 个节点。」，然后在磁盘上找不到任何东西——
    /// 与 U156（点运行什么都不会发生）同型：**报告成功的空动作**。
    ///
    /// **顺带查出第二个独立缺陷**（写这条时用裸 IPC 探针确认的）：
    /// 后端返回的 JSON 顶层键是 `{workflow, boundary_inputs, boundary_outputs}`，
    /// 而前端把它反序列化成 `WorkflowGraphData`（`JsonLineBackendClient.cs:529`）
    /// ——形状根本不匹配，`nodes` 键不存在。所以拿到的对象里 `Nodes` 是 null，
    /// 一碰就 NRE。前端因为丢弃返回值才没炸；**任何想真正使用导出结果的改动
    /// 都会立刻 NRE**。这一层错配单独存在，修「不落盘」时若不一并修类型，
    /// 修完照样用不了。
    ///
    /// 判据取「项目目录里出现了一个新文件」这个最宽的口径——
    /// 不限定格式、不限定路径，就是为了避免把「我猜的落盘位置不对」
    /// 误报成缺陷。宽口径下仍然找不到文件，才是真的什么都没产生。
    ///
    /// 修复后本条应当转绿；在修复前它是**已知红**，记录的是缺陷本身。
    /// </summary>
    [Fact]
    public async Task Canvas_ExportSelection_ProducesAFileTheAuthorCanActuallyFind()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_ExportSelection_ProducesAFileTheAuthorCanActuallyFind)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var (projectRoot, _) = await SeedCanvasAsync(client, "导出片段", new[]
        {
            LlmNode("node-1", "第一个节点。", 0, 0),
            LlmNode("node-2", "第二个节点。", 300, 0),
        });

        // 导出前的文件全集（含所有子目录）。
        var before = Directory
            .GetFiles(projectRoot, "*", SearchOption.AllDirectories)
            .ToHashSet(StringComparer.Ordinal);

        var exported = await client.ExportWorkflowSelectionAsync(
            CanvasWorkflowId, new[] { "node-1", "node-2" });

        // 缺陷二（类型错配）已修：后端给的是 {workflow, boundary_*, storage_uri}，
        // 现在前端有同构的 WorkflowSelectionExportData 接它（此前按 WorkflowGraphData 解，
        // 顶层没有 nodes 键 ⇒ Nodes 恒为 null，只因调用点丢弃返回值才没 NRE）。
        // 判据落在**片段里真的有那两个节点**上：类型对齐了但 workflow 装错内容一样没用。
        Assert.NotNull(exported.Workflow);
        Assert.NotNull(exported.Workflow.Nodes);
        Assert.Equal(2, exported.Workflow.Nodes.Count);

        // 缺陷一（不落盘）已修：storage_uri 是「真的产生了文件」的唯一凭据，
        // 也是 UI 判断该不该说「已导出」的依据。
        Assert.False(
            string.IsNullOrWhiteSpace(exported.StorageUri),
            "导出没有返回 storage_uri ⇒ 没有落盘位置可报给用户，"
            + "UI 只能重新退回「报告成功的空动作」那个形态。");

        var after = Directory
            .GetFiles(projectRoot, "*", SearchOption.AllDirectories)
            .ToHashSet(StringComparer.Ordinal);
        var created = after.Except(before, StringComparer.Ordinal).ToArray();

        Assert.True(
            created.Length > 0,
            "「导出所选片段」没有在项目目录下产生任何文件。修复前 UI 会把状态栏写成"
            + "「已导出 N 个节点」而磁盘上什么都没有，是 U156 同型的**报告成功的空动作**。\n"
            + "对照：章节导出走 export_chapters（commands.rs:2311）有 artifact 写入与 "
            + "storage_uri，工作流片段导出此前是纯函数、无 IO。");

        // 落盘位置必须与返回的 storage_uri 一致——否则 UI 报给用户的路径是编的。
        // 这一条是「宽口径找到文件」之外单独必要的：随便写一个文件到别处也能让
        // created.Length > 0 通过，而用户按状态栏给的路径去找依然找不到。
        Assert.Contains(
            created,
            path => path.Replace('\\', '/').EndsWith(
                exported.StorageUri!.Replace('\\', '/').TrimStart('/'),
                StringComparison.Ordinal)
                || exported.StorageUri!.Replace('\\', '/')
                    .EndsWith(Path.GetFileName(path), StringComparison.Ordinal));
    }

    // ════════════════════════════════════════════════════════
    // 动作 7：打包成子流程
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 打包必须**真的改了磁盘上的图**：选中节点被折叠成一个 subworkflow 节点。
    ///
    /// 与导出的区别正是这条要钉住的：打包是 in-place 变换（改图并落盘），
    /// 导出是产出物（应该给文件）。两者返回值形状相似，都是「一张图」，
    /// 所以「返回值里有东西」对区分它们零帮助——判据必须落在磁盘状态上。
    /// </summary>
    [Fact]
    public async Task Canvas_PackSelection_ReplacesNodesWithSubworkflowOnDisk()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_PackSelection_ReplacesNodesWithSubworkflowOnDisk)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var (projectRoot, saved) = await SeedCanvasAsync(client, "打包", new[]
        {
            LlmNode("node-1", "被打包的第一个。", 0, 0),
            LlmNode("node-2", "被打包的第二个。", 300, 0),
            LlmNode("node-3", "留在外面的。", 600, 0),
        });

        var report = await client.PackWorkflowSelectionAsync(
            CanvasWorkflowId,
            new[] { "node-1", "node-2" },
            subworkflowNodeId: null,
            title: "开篇两步",
            expectedRevision: saved.ContentRevision);

        Assert.False(string.IsNullOrWhiteSpace(report.SubworkflowNodeId));
        Assert.Equal(2, report.EmbeddedWorkflow.Nodes.Count);

        // 判据：磁盘图里那两个节点没了，取而代之是一个 subworkflow 节点；
        // 没选中的 node-3 必须还在原地。
        using var doc = ReadCanvasFromDisk(projectRoot);
        var nodes = doc.RootElement.GetProperty("nodes").EnumerateArray().ToArray();
        var ids = nodes.Select(node => node.GetProperty("id").GetString()).ToArray();

        Assert.DoesNotContain("node-1", ids);
        Assert.DoesNotContain("node-2", ids);
        Assert.Contains("node-3", ids);
        Assert.Contains(report.SubworkflowNodeId, ids);

        // 被折叠的片段要嵌在新节点的 config 里，否则打包等于把两个节点删了。
        var packed = NodeConfigFromDisk(projectRoot, report.SubworkflowNodeId);
        var embeddedText = packed.GetRawText();
        Assert.Contains("被打包的第一个", embeddedText);
        Assert.Contains("被打包的第二个", embeddedText);
    }

    /// <summary>
    /// 打包的**幂等回执**：同一个 `operation_id` 重放不得打包两次。
    ///
    /// 真实场景是 IPC 响应丢失后前端重试（后端为此专门留了 `operation_id`，
    /// 见 `commands.rs:849` 的注释）。重复打包会让画布上多出一个空的
    /// subworkflow 节点，而作者无从知道它是怎么来的。
    ///
    /// 判据取**磁盘节点数不变**，而不是「第二次调用是否报错」——
    /// 幂等的正确行为是「安静地返回同一个回执」，报错反而是错的。
    /// </summary>
    [Fact]
    public async Task Canvas_PackSelection_ReplayWithSameOperationIdDoesNotPackTwice()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Canvas_PackSelection_ReplayWithSameOperationIdDoesNotPackTwice)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var (projectRoot, saved) = await SeedCanvasAsync(client, "打包重放", new[]
        {
            LlmNode("node-1", "第一个。", 0, 0),
            LlmNode("node-2", "第二个。", 300, 0),
        });

        // 只能用 ASCII 字母数字 `- _ :`：后端 `commands.rs:13830` 明确校验。
        // 首版我写成 "pack-op-固定"，被拒——那是**校验在正常工作**，不是缺陷。
        // 本仓库其余测试数据刻意用中文（查 UTF-8 相关缺陷），但这个字段是
        // 幂等键、要进文件名与日志，限制成 ASCII 是合理设计，不该「顺手放宽」。
        const string operationId = "pack-op-fixed";
        var first = await client.PackWorkflowSelectionAsync(
            CanvasWorkflowId,
            new[] { "node-1", "node-2" },
            subworkflowNodeId: null,
            title: "两步",
            expectedRevision: saved.ContentRevision,
            operationId: operationId);

        int NodeCount()
        {
            using var doc = ReadCanvasFromDisk(projectRoot);
            return doc.RootElement.GetProperty("nodes").GetArrayLength();
        }
        var countAfterFirst = NodeCount();

        // 重放：同 operation_id、同 expected_revision（前端重试时手里就是旧的那个）。
        var second = await client.PackWorkflowSelectionAsync(
            CanvasWorkflowId,
            new[] { "node-1", "node-2" },
            subworkflowNodeId: null,
            title: "两步",
            expectedRevision: saved.ContentRevision,
            operationId: operationId);

        Assert.Equal(first.SubworkflowNodeId, second.SubworkflowNodeId);
        Assert.Equal(countAfterFirst, NodeCount());
    }

    // ════════════════════════════════════════════════════════
    // 共用：等终态
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 轮询到终态。`paused` 也算终态——断点那两条正是要区分 paused 与 failed。
    /// 超时信息带上最后状态与失败原因：否则「等超时了」这一句对排查毫无帮助。
    /// </summary>
    private static async Task<WorkflowRunState> WaitForTerminalStateAsync(
        IAriadneBackendClient client, string workflowId, string runId)
    {
        WorkflowRunState? last = null;
        for (var i = 0; i < 900; i++)
        {
            last = await client.GetWorkflowRunStateAsync(workflowId, runId);
            var status = last.Status.ToLowerInvariant();
            if (status is "succeeded" or "failed" or "stopped" or "paused")
            {
                return last;
            }
            await Task.Delay(100);
        }
        throw new TimeoutException(
            $"等待终态超时；最后状态={last?.Status}, pause={last?.PauseReason}, "
            + $"failure={last?.Failure?.Message}");
    }
}
