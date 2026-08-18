using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 桌面端**写作生产链**全流程：真实 sidecar 进程 + 真实 JSON-line IPC +
/// 自建 HTTP 端当 LLM，判据取**磁盘正文**与**导出产物字节**。
///
/// 与 <see cref="FrontendProductionJourneyTests"/> 的分工：
/// - 那个文件覆盖「通用 <c>llm</c> 节点跑到终态」——证明链路通电；
/// - 本文件覆盖产品真正的主线：**writer 节点产出 → 确认项待审 → 用户点同意
///   → 正文落盘 → 检查点入 git → 章节导出**。
///
/// **为什么必须再写一遍（Rust 侧 `patch_write_back_contracts.rs` 已覆盖同一链路）**：
/// 那 5 条用例在 Rust 进程内直调 `commands::` 函数，跳过了桌面端真正要走的那一层——
/// DTO 序列化、IPC 命令名、参数键名、AppState 的 current-project 绑定。
/// CLAUDE.md 记着这条教训：「mock 会掩盖一整类缺陷……任何跨进程边界至少要有一个
/// 真实进程的测试」。IPC 的 BOM 毒死连接那条缺陷就只在真实管道上复现。
/// 更具体地说：确认项决议在桌面端**此前只用 stub client 测过**
/// （`ConfirmationReviewPanelTests` 是内存假客户端），
/// 「点同意后正文到底有没有落盘」从桌面一侧从未验证过。
///
/// sidecar 未编译时跳过（与 <see cref="BackendColdStartTests"/> 同约定）。
///
/// ⚠️ **当前本文件 9 条全部失败，失败即缺陷存在——不是测试写错了**。
/// 全部倒在同一步 <c>RunWorkflowAsync</c>：
/// <c>invalid ipc params: invalid type: null, expected a map</c>。
/// 根因是 <c>JsonLineBackendClient.cs:328</c> 发出了 <c>"variables": null</c>，
/// 而后端 <c>RunWorkflowParams.variables</c> 是非 <c>Option</c> 的 <c>BTreeMap</c> +
/// <c>#[serde(default)]</c>——default 只对「键缺失」生效、不接受显式 null。
/// **P0 发布阻断，见 U156**（同一原因让既有的
/// <see cref="FrontendProductionJourneyTests"/> 3 条也全红，那 3 条在
/// 2026-08-08 的回归之前是绿的）。
/// U156 修好后本文件应转绿；若仍不绿，剩下的失败才是新发现。
/// </summary>
[Collection("RealSidecar")]
public sealed class FrontendWritingChainJourneyTests : IDisposable
{
    private const string ProviderId = "primary";
    private const string ModelId = "chain-model";
    private const string ChapterRelativePath = "chapters/chapter-01.md";

    private readonly DirectoryInfo _temp =
        Directory.CreateTempSubdirectory("ariadne-writing-chain-");

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
        // U142：起真实 sidecar 前先确认 app-state 已隔离，否则本测试建的项目
        // 会写进用户真实的 recent_projects.json 并把用户自己的项目挤出 20 条上限。
        SidecarAppStateIsolation.RequireIsolatedAppState();

        // 无 keychain 的开发构建里 LocalFileSecretStore 只认这个变量
        // （core/src/config/secrets.rs）。U118 跟踪「Linux 上无主密码 UI」，
        // 这里先注入以便验证其余链路。
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
    // 假 LLM：真实 HTTP 端，按轮次回放，捕获每轮请求原文
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 会发起写作工具调用的假 LLM：第 1 轮回 tool_calls，第 2 轮回终答。
    ///
    /// **两轮是必需的**：工具调用型节点的循环是「发工具 → 执行 → 把结果回喂 →
    /// 要终答」。只回一轮的假服务会让节点在第二次请求上挂到超时，
    /// 测出来的是超时而不是写作链路。
    ///
    /// 返回的 <c>Captured</c> 是**全部轮次**的请求原文，用于断言提示词真的出站
    /// （而不是占位符字面量出站——那是 U120 那一族缺陷的形状）。
    /// </summary>
    private static (string BaseUrl, Task<IReadOnlyList<string>> Captured) SpawnToolCallingLlm(
        IReadOnlyList<(string Name, object Arguments)> toolCalls,
        string finalContent = "写好了。")
    {
        var firstBody = JsonSerializer.Serialize(new
        {
            model = ModelId,
            choices = new[]
            {
                new
                {
                    message = new
                    {
                        content = "",
                        tool_calls = toolCalls.Select((call, index) => new
                        {
                            id = $"call-{index}",
                            type = "function",
                            function = new
                            {
                                name = call.Name,
                                // arguments 必须是**字符串**：OpenAI 兼容协议里它是
                                // 序列化后的 JSON 文本，直接塞对象会让后端解析失败，
                                // 症状是「工具没被调用」而非「参数不对」，极难定位。
                                arguments = JsonSerializer.Serialize(call.Arguments),
                            },
                        }).ToArray(),
                    },
                    finish_reason = "tool_calls",
                },
            },
            usage = new { prompt_tokens = 10, completion_tokens = 2 },
        });

        var secondBody = JsonSerializer.Serialize(new
        {
            model = ModelId,
            choices = new[]
            {
                new
                {
                    message = new { content = finalContent, tool_calls = Array.Empty<object>() },
                    finish_reason = "stop",
                },
            },
            usage = new { prompt_tokens = 20, completion_tokens = 3 },
        });

        return SpawnReplayLlm(new[] { firstBody, secondBody });
    }

    /// <summary>纯文本回复的假 LLM（不发工具调用）。</summary>
    private static (string BaseUrl, Task<IReadOnlyList<string>> Captured) SpawnPlainLlm(
        string content, int rounds = 1)
    {
        var body = JsonSerializer.Serialize(new
        {
            model = ModelId,
            choices = new[]
            {
                new
                {
                    message = new { content, tool_calls = Array.Empty<object>() },
                    finish_reason = "stop",
                },
            },
            usage = new { prompt_tokens = 12, completion_tokens = 6 },
        });
        return SpawnReplayLlm(Enumerable.Repeat(body, rounds).ToArray());
    }

    /// <summary>
    /// 按序回放若干响应的真实 HTTP 端。
    ///
    /// **必须读到请求体结束再回**：HTTP 请求可能跨多个 TCP 包到达
    /// （提示词带上下文时轻易超过一个 MSS）。只 <c>ReadAsync</c> 一次就回响应，
    /// 在小请求上碰巧能过、在大请求上间歇失败——这类不稳定测试比没有测试更糟，
    /// 因为它会被当成「环境问题」而不是被修。所以按 Content-Length 读满。
    /// </summary>
    private static (string BaseUrl, Task<IReadOnlyList<string>> Captured) SpawnReplayLlm(
        IReadOnlyList<string> bodies)
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        var baseUrl = $"http://127.0.0.1:{((IPEndPoint)listener.LocalEndpoint).Port}";

        var captured = Task.Run<IReadOnlyList<string>>(async () =>
        {
            var seen = new List<string>();
            try
            {
                foreach (var body in bodies)
                {
                    using var socket = await listener
                        .AcceptSocketAsync()
                        .WaitAsync(TimeSpan.FromSeconds(60));
                    using var stream = new NetworkStream(socket, ownsSocket: false);
                    seen.Add(await ReadHttpRequestAsync(stream));

                    var payload = Encoding.UTF8.GetBytes(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
                        + $"Content-Length: {Encoding.UTF8.GetByteCount(body)}\r\n\r\n{body}");
                    await stream.WriteAsync(payload);
                    await stream.FlushAsync();
                }
            }
            catch (TimeoutException)
            {
                // 轮次少于预期：把已收到的交回，让断言去报「差在哪一轮」，
                // 比在这里抛掉线索更有诊断价值。
            }
            finally
            {
                listener.Stop();
            }
            return seen;
        });

        return (baseUrl, captured);
    }

    /// <summary>读满一个 HTTP 请求（头 + 按 Content-Length 的体）。</summary>
    private static async Task<string> ReadHttpRequestAsync(NetworkStream stream)
    {
        var buffer = new byte[64 * 1024];
        var text = new StringBuilder();
        var headerEnd = -1;
        var total = 0;

        // 先读到头结束。
        while (headerEnd < 0)
        {
            var read = await stream.ReadAsync(buffer).AsTask().WaitAsync(TimeSpan.FromSeconds(30));
            if (read <= 0)
            {
                return text.ToString();
            }
            total += read;
            text.Append(Encoding.UTF8.GetString(buffer, 0, read));
            headerEnd = text.ToString().IndexOf("\r\n\r\n", StringComparison.Ordinal);
        }

        var whole = text.ToString();
        var contentLength = 0;
        foreach (var line in whole[..headerEnd].Split("\r\n"))
        {
            if (line.StartsWith("Content-Length:", StringComparison.OrdinalIgnoreCase))
            {
                _ = int.TryParse(line["Content-Length:".Length..].Trim(), out contentLength);
            }
        }

        // 已读到的体字节数按**字节**算，不能按字符：正文是中文，
        // UTF-8 下一个汉字 3 字节，用字符数比会永远读不满而卡到超时。
        var bodyBytesSoFar = total - Encoding.UTF8.GetByteCount(whole[..(headerEnd + 4)]);
        while (bodyBytesSoFar < contentLength)
        {
            var read = await stream.ReadAsync(buffer).AsTask().WaitAsync(TimeSpan.FromSeconds(30));
            if (read <= 0)
            {
                break;
            }
            bodyBytesSoFar += read;
            text.Append(Encoding.UTF8.GetString(buffer, 0, read));
        }

        return text.ToString();
    }

    // ════════════════════════════════════════════════════════
    // 用户操作序列（每个方法 = 界面上的一个按钮）
    // ════════════════════════════════════════════════════════

    /// <summary>设置页：保存 Provider + 保存密钥。</summary>
    private static async Task UserConfiguresProviderAsync(
        IAriadneBackendClient client, string baseUrl)
    {
        await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            ProviderId: ProviderId,
            ProviderType: "open_ai_compatible",
            DisplayName: "我的模型服务",
            Enabled: true,
            BaseUrl: baseUrl,
            Models: new[] { new ModelConfig(ModelId, "llm", null, null, null) },
            MakeDefaultLlm: true,
            MakeDefaultEmbedding: false,
            MakeDefaultReranker: false,
            MakeDefaultSearch: false));
        await client.SaveProviderKeyAsync(ProviderId, "sk-writing-chain");
    }

    /// <summary>
    /// 设置页-权限：放开写作工具。
    ///
    /// 走 get→改→save 而不是构造一份全新 <c>PermissionsSettings</c>：
    /// 后者会把用户其它权限设置一起覆盖成默认值，测出来的就不是真实起点了。
    /// </summary>
    private static async Task UserAllowsWritingToolsAsync(IAriadneBackendClient client)
    {
        var settings = await client.GetPermissionsSettingsAsync();
        var controls = settings.ToolControls.ToDictionary(
            pair => pair.Key,
            pair => (IReadOnlyDictionary<string, bool?>)pair.Value.ToDictionary(
                inner => inner.Key, inner => inner.Value));

        var global = controls.TryGetValue("global", out var existing)
            ? existing.ToDictionary(pair => pair.Key, pair => pair.Value)
            : new Dictionary<string, bool?>();
        global["write"] = true;
        global["register"] = true;
        controls["global"] = global;

        await client.SavePermissionsSettingsAsync(
            settings with { ToolControls = controls });
    }

    /// <summary>画布：拖一个 writer 节点并配好模型、提示词、目标文档。</summary>
    private static Task<WorkflowGraphData> UserBuildsWriterWorkflowAsync(
        IAriadneBackendClient client,
        string workflowId,
        string prompt,
        string documentId,
        string? expectedRevision = null)
        => client.SaveWorkflowGraphAsync(new WorkflowGraphData(
            WorkflowId: workflowId,
            Name: workflowId,
            Nodes: new[]
            {
                new CanvasNode(
                    Id: "node-1",
                    Type: "writer",
                    Label: null,
                    Data: new Dictionary<string, object?>
                    {
                        ["provider_id"] = ProviderId,
                        ["model_id"] = ModelId,
                        ["prompt_template"] = prompt,
                        // U108：正文从磁盘读，节点 config 指定 document_id。
                        ["document_id"] = documentId,
                        ["chapter_id"] = "chapter-01",
                    },
                    Position: null),
            },
            Edges: Array.Empty<CanvasEdge>(),
            Metadata: new Dictionary<string, object?>(),
            ContentRevision: null,
            ExpectedRevision: expectedRevision));

    /// <summary>
    /// 执行页：轮询到终态。
    ///
    /// <c>paused</c> 也算终局：写作节点产出确认项后运行会挂起等人审——
    /// 把它排除在终态之外，测试会在正常的待审状态上超时并报成「卡死」。
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
            + $"failure={last?.Failure?.Message}, events=[{string.Join(" | ", last?.Events ?? Array.Empty<string>())}]");
    }

    /// <summary>
    /// 确认项面板：找出这次运行的待审项。
    ///
    /// 轮询而非单次读取：确认项由后台 worker 在节点结束时入库，
    /// 运行态刚变 paused 的一瞬间可能还没落库。单次读取会得到间歇性空结果。
    /// </summary>
    private static async Task<ConfirmationLogEntry> WaitForPendingConfirmationAsync(
        IAriadneBackendClient client, string runId)
    {
        IReadOnlyList<ConfirmationLogEntry> all = Array.Empty<ConfirmationLogEntry>();
        for (var i = 0; i < 100; i++)
        {
            all = await client.ListConfirmationsAsync();
            var pending = all.FirstOrDefault(entry =>
                string.Equals(entry.RunId, runId, StringComparison.Ordinal)
                && entry.State.Equals("pending", StringComparison.OrdinalIgnoreCase));
            if (pending is not null)
            {
                return pending;
            }
            await Task.Delay(100);
        }
        throw new InvalidOperationException(
            $"运行 {runId} 没有待审确认项；确认项列表="
            + string.Join(" | ", all.Select(e => $"{e.ConfirmationId}/{e.State}/run={e.RunId}")));
    }

    private static string ReadChapter(string projectRoot)
    {
        var path = Path.Combine(projectRoot, ChapterRelativePath);
        return File.Exists(path) ? File.ReadAllText(path) : string.Empty;
    }

    private static string SeedChapter(string projectRoot, string body)
    {
        var path = Path.Combine(projectRoot, ChapterRelativePath);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, body);
        // document_id 必须是**绝对**路径：apply_patch 直接 PathBuf::from，
        // 沙箱拒相对路径（CLAUDE.md U108 阶段 3）。这是桌面端最容易传错的一个参数
        // ——传相对路径时症状是「同意了但正文没变」，与「patch 丢失」难以区分。
        return path;
    }

    private static int CommitCount(string projectRoot)
    {
        var process = System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo
        {
            FileName = "git",
            WorkingDirectory = projectRoot,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        }.WithArgs("rev-list", "--count", "HEAD"));
        if (process is null)
        {
            return 0;
        }
        var output = process.StandardOutput.ReadToEnd().Trim();
        process.WaitForExit(10_000);
        return int.TryParse(output, out var count) ? count : 0;
    }

    // ════════════════════════════════════════════════════════
    // 主线 1：写作产出 → 待审 → 同意 → 正文落盘
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// **本文件最重要的一条**：从桌面端走完整条写作链，判据是**磁盘正文**。
    ///
    /// 判据为什么必须取磁盘：CLAUDE.md 记着 U108/U114/U117 那一族
    /// 「实现完整 + 有测试覆盖 + 生产零调用者」的缺陷——
    /// 「确认项存在」「工具返回成功」「运行态是 Applied」这些判据在 patch
    /// 压根没被保留时都可能照样为真。只有磁盘正文骗不过去。
    /// </summary>
    [Fact]
    public async Task WritingChain_ApprovedPatch_LandsOnDiskThroughRealIpc()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(WritingChain_ApprovedPatch_LandsOnDiskThroughRealIpc)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "落盘验证");
        await client.CreateProjectAsync(projectRoot, "落盘验证");
        var documentId = SeedChapter(projectRoot, "第一章\n");

        var (baseUrl, captured) = SpawnToolCallingLlm(new (string, object)[]
        {
            ("writer-insert-lines", new
            {
                document_id = documentId,
                after_line = 1,
                text = "夜里下起了雨。\n",
            }),
        });
        await UserConfiguresProviderAsync(client, baseUrl);
        await UserAllowsWritingToolsAsync(client);
        await UserBuildsWriterWorkflowAsync(
            client, "writer-flow", "续写第一章的第一段", documentId);

        var started = await client.RunWorkflowAsync("writer-flow");
        Assert.False(string.IsNullOrWhiteSpace(started.RunId), "运行必须返回 run_id");
        await WaitForTerminalStateAsync(client, "writer-flow", started.RunId);

        // ── 门禁：审批之前正文不得被改动（U117 语义） ──
        Assert.DoesNotContain("夜里下起了雨", ReadChapter(projectRoot));

        // ── 用户在确认项面板点「同意」 ──
        var pending = await WaitForPendingConfirmationAsync(client, started.RunId);
        var resolved = await client.ResolveConfirmationAsync(
            "writer-flow", started.RunId, pending.ConfirmationId, "approve");
        Assert.NotNull(resolved);

        // ── 终局判据：磁盘正文 ──
        var body = ReadChapter(projectRoot);
        Assert.Contains("夜里下起了雨", body);

        // 提示词真的出站了（不是占位符字面量——U120 那一族的形状）。
        var requests = await captured.WaitAsync(TimeSpan.FromSeconds(30));
        Assert.NotEmpty(requests);
        Assert.Contains("续写第一章的第一段", requests[0]);
        Assert.DoesNotContain("{{", requests[0]);
    }

    /// <summary>
    /// 拒绝路径：点「拒绝」后正文必须**一字不差**。
    ///
    /// 与上一条是同一枚硬币的两面。只测同意路径的话，一个「无论决议如何都写盘」
    /// 的实现会照样全绿——那等于把审批做成纯装饰的确认框。
    /// </summary>
    [Fact]
    public async Task WritingChain_RejectedPatch_LeavesChapterByteIdentical()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(WritingChain_RejectedPatch_LeavesChapterByteIdentical)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "拒绝验证");
        await client.CreateProjectAsync(projectRoot, "拒绝验证");

        const string original = "第一章\n原有的一段。\n";
        var documentId = SeedChapter(projectRoot, original);

        var (baseUrl, _) = SpawnToolCallingLlm(new (string, object)[]
        {
            ("writer-insert-lines", new
            {
                document_id = documentId,
                after_line = 1,
                text = "这段不该出现。\n",
            }),
        });
        await UserConfiguresProviderAsync(client, baseUrl);
        await UserAllowsWritingToolsAsync(client);
        await UserBuildsWriterWorkflowAsync(client, "reject-flow", "写一段", documentId);

        var started = await client.RunWorkflowAsync("reject-flow");
        await WaitForTerminalStateAsync(client, "reject-flow", started.RunId);

        var pending = await WaitForPendingConfirmationAsync(client, started.RunId);
        await client.ResolveConfirmationAsync(
            "reject-flow", started.RunId, pending.ConfirmationId, "reject", "不要这段");

        Assert.Equal(original, ReadChapter(projectRoot));
    }

    /// <summary>
    /// 幂等：**重复点同意**不得把 patch 叠加两次。
    ///
    /// 这条针对的是真实误操作——审批面板刷新慢时用户会连点，或双击。
    /// 叠加两次的症状是正文出现重复段落，而且是在用户看不到的地方
    /// （patch 落在文件中间），发现时往往已经写了好几章。
    /// <c>PatchWriteBackState::Applied</c> 是这条的哨兵；断言取
    /// **段落出现次数**，而不是「第二次调用是否报错」——后者在
    /// 「静默重复写盘」的实现下照样能过。
    /// </summary>
    [Fact]
    public async Task WritingChain_DoubleApprove_DoesNotDuplicateParagraph()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(WritingChain_DoubleApprove_DoesNotDuplicateParagraph)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "幂等验证");
        await client.CreateProjectAsync(projectRoot, "幂等验证");
        var documentId = SeedChapter(projectRoot, "第一章\n");

        const string inserted = "只应出现一次的段落。";
        var (baseUrl, _) = SpawnToolCallingLlm(new (string, object)[]
        {
            ("writer-insert-lines", new
            {
                document_id = documentId,
                after_line = 1,
                text = inserted + "\n",
            }),
        });
        await UserConfiguresProviderAsync(client, baseUrl);
        await UserAllowsWritingToolsAsync(client);
        await UserBuildsWriterWorkflowAsync(client, "idem-flow", "写一段", documentId);

        var started = await client.RunWorkflowAsync("idem-flow");
        await WaitForTerminalStateAsync(client, "idem-flow", started.RunId);

        var pending = await WaitForPendingConfirmationAsync(client, started.RunId);
        await client.ResolveConfirmationAsync(
            "idem-flow", started.RunId, pending.ConfirmationId, "approve");

        // 第二次点同意：可以报错、也可以静默无操作，但**磁盘不能变**。
        try
        {
            await client.ResolveConfirmationAsync(
                "idem-flow", started.RunId, pending.ConfirmationId, "approve");
        }
        catch (BackendException)
        {
            // 「已处理过」是可接受的响应形态，不是本条要断言的东西。
        }

        var body = ReadChapter(projectRoot);
        var occurrences = body.Split(inserted).Length - 1;
        Assert.Equal(1, occurrences);
    }

    // ════════════════════════════════════════════════════════
    // 主线 2：同一节点内多次 patch 的行号合成
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 同节点内**两次**插入：第二次的行号必须基于第一次之后的状态。
    ///
    /// 这是 <c>PatchSession.simulated</c> 存在的全部理由。若实现改成
    /// 「各自按原始快照算行号、依次应用」，第二次就插错位置，正文顺序被打乱，
    /// 且症状随插入点变化——属于极难事后定位的一类。
    ///
    /// 原文三行 甲/乙/丙：
    /// - 第 1 次在第 1 行后插 A → 甲/A/乙/丙
    /// - 第 2 次在第 3 行后插 B → 甲/A/乙/B/丙（「第 3 行」= 插入后的「乙」）
    ///
    /// 按原始快照算则「第 3 行」是「丙」，结果为 甲/A/乙/丙/B。
    /// 两种结果不同，本用例即可区分。
    /// 插入文本自带 <c>\n</c>：行号 patch 按 <c>split_inclusive('\n')</c> 定位，
    /// 不带换行会粘到下一行开头而使行数不变，本用例反而丧失区分能力。
    /// </summary>
    [Fact]
    public async Task WritingChain_TwoPatchesInOneNode_ComposeBySimulatedLineNumbers()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(WritingChain_TwoPatchesInOneNode_ComposeBySimulatedLineNumbers)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "行号合成");
        await client.CreateProjectAsync(projectRoot, "行号合成");
        var documentId = SeedChapter(projectRoot, "甲\n乙\n丙\n");

        var (baseUrl, _) = SpawnToolCallingLlm(new (string, object)[]
        {
            ("writer-insert-lines", new { document_id = documentId, after_line = 1, text = "A\n" }),
            ("writer-insert-lines", new { document_id = documentId, after_line = 3, text = "B\n" }),
        });
        await UserConfiguresProviderAsync(client, baseUrl);
        await UserAllowsWritingToolsAsync(client);
        await UserBuildsWriterWorkflowAsync(client, "compose-flow", "改两处", documentId);

        var started = await client.RunWorkflowAsync("compose-flow");
        await WaitForTerminalStateAsync(client, "compose-flow", started.RunId);

        var pending = await WaitForPendingConfirmationAsync(client, started.RunId);
        await client.ResolveConfirmationAsync(
            "compose-flow", started.RunId, pending.ConfirmationId, "approve");

        Assert.Equal("甲\nA\n乙\nB\n丙\n", ReadChapter(projectRoot));
    }

    // ════════════════════════════════════════════════════════
    // 主线 3：落盘 → 检查点 → 导出（用户拿到成品）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 全链末端：正文落盘后**建检查点**，git 历史必须真的多一条。
    ///
    /// 判据取 <c>git rev-list --count</c> 而非 <c>CreateCheckpointAsync</c> 的返回值：
    /// U111 的教训是「断言请求被构造只能证明代码路径走到了，
    /// 证明不了 git 里到底有没有那条记录」。
    /// </summary>
    [Fact]
    public async Task WritingChain_CheckpointAfterApproval_AppearsInRealGitHistory()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(WritingChain_CheckpointAfterApproval_AppearsInRealGitHistory)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "检查点验证");
        var report = await client.CreateProjectAsync(projectRoot, "检查点验证");
        if (!report.GitInitialized)
        {
            return; // 环境无 git：本条无从判定，跳过而非误报。
        }

        var documentId = SeedChapter(projectRoot, "第一章\n");
        var (baseUrl, _) = SpawnToolCallingLlm(new (string, object)[]
        {
            ("writer-insert-lines", new
            {
                document_id = documentId,
                after_line = 1,
                text = "风从窗缝里挤进来。\n",
            }),
        });
        await UserConfiguresProviderAsync(client, baseUrl);
        await UserAllowsWritingToolsAsync(client);
        await UserBuildsWriterWorkflowAsync(client, "ckpt-flow", "续写", documentId);

        var started = await client.RunWorkflowAsync("ckpt-flow");
        await WaitForTerminalStateAsync(client, "ckpt-flow", started.RunId);
        var pending = await WaitForPendingConfirmationAsync(client, started.RunId);
        await client.ResolveConfirmationAsync(
            "ckpt-flow", started.RunId, pending.ConfirmationId, "approve");
        Assert.Contains("风从窗缝里挤进来", ReadChapter(projectRoot));

        var before = CommitCount(projectRoot);
        var point = await client.CreateCheckpointAsync("第一章初稿");
        Assert.False(string.IsNullOrWhiteSpace(point.CommitId), "检查点必须返回 commit_id");

        var after = CommitCount(projectRoot);
        Assert.True(
            after > before,
            $"建检查点后 git 历史没有增加：before={before}, after={after}, "
            + $"commit_id={point.CommitId}");

        // 历史必须能被界面读回——Git 页读的是这个命令，不是 git CLI。
        var history = await client.GetGitHistoryAsync();
        Assert.Contains(history, entry =>
            entry.CommitId.StartsWith(point.CommitId, StringComparison.Ordinal)
            || point.CommitId.StartsWith(entry.CommitId, StringComparison.Ordinal));
    }

    /// <summary>
    /// 全链末端：导出成品，判据是**产物文件里真的含正文**。
    ///
    /// 只断言 <c>CombinedExportReport</c> 非空是不够的——章节索引为空时导出
    /// 会「成功」地产出一个空文件，报告字段一样齐全。所以必须读产物字节。
    /// </summary>
    [Fact]
    public async Task WritingChain_ExportAfterApproval_ArtifactContainsWrittenProse()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(WritingChain_ExportAfterApproval_ArtifactContainsWrittenProse)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "导出验证");
        await client.CreateProjectAsync(projectRoot, "导出验证");
        var documentId = SeedChapter(projectRoot, "第一章\n");

        const string prose = "灯亮到很晚。";
        var (baseUrl, _) = SpawnToolCallingLlm(new (string, object)[]
        {
            ("writer-insert-lines", new
            {
                document_id = documentId,
                after_line = 1,
                text = prose + "\n",
            }),
        });
        await UserConfiguresProviderAsync(client, baseUrl);
        await UserAllowsWritingToolsAsync(client);

        // 章节必须先登记进索引，导出才看得见它——导出走 ChapterDocumentIndex，
        // 不是扫目录。跳过这步会导出一个空文件而报告照样成功。
        await client.ImportChapterAsync(new ChapterImportRequest(
            ChapterId: "chapter-01",
            Title: "第一章",
            Order: 1,
            SourcePath: documentId,
            TargetPath: ChapterRelativePath,
            Overwrite: true));

        await UserBuildsWriterWorkflowAsync(client, "export-flow", "续写", documentId);
        var started = await client.RunWorkflowAsync("export-flow");
        await WaitForTerminalStateAsync(client, "export-flow", started.RunId);
        var pending = await WaitForPendingConfirmationAsync(client, started.RunId);
        await client.ResolveConfirmationAsync(
            "export-flow", started.RunId, pending.ConfirmationId, "approve");
        Assert.Contains(prose, ReadChapter(projectRoot));

        var export = await client.ExportChaptersAsync(
            new[] { "chapter-01" }, artifactId: null, format: "markdown");
        Assert.Contains("chapter-01", export.ExportedChapterIds);
        Assert.False(string.IsNullOrWhiteSpace(export.StorageUri), "导出必须给出产物位置");

        // 判据：产物文件里真的有刚写进去的那段。
        var artifactPath = ResolveArtifactPath(projectRoot, export.StorageUri);
        Assert.True(File.Exists(artifactPath), $"导出产物不存在：{artifactPath}（uri={export.StorageUri}）");
        var exported = await File.ReadAllTextAsync(artifactPath);
        Assert.Contains(prose, exported);
        Assert.True(export.SizeBytes is > 0, $"导出产物大小异常：{export.SizeBytes}");
    }

    /// <summary>
    /// 把 <c>storage_uri</c> 还原成本地路径。
    ///
    /// 后端可能给 <c>file://</c> URI 也可能给项目内相对路径，两种都要认——
    /// 只认一种会在另一种形态下把「导出功能正常」误报成缺陷。
    /// </summary>
    private static string ResolveArtifactPath(string projectRoot, string storageUri)
    {
        if (Uri.TryCreate(storageUri, UriKind.Absolute, out var uri) && uri.IsFile)
        {
            return uri.LocalPath;
        }
        return Path.IsPathRooted(storageUri)
            ? storageUri
            : Path.Combine(projectRoot, storageUri);
    }

    // ════════════════════════════════════════════════════════
    // 主线 4：预算与错误面（用户最容易撞上的两堵墙）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 用户在设置页把日预算设成**显式零**后点运行：必须被挡住，且不得出站。
    ///
    /// U112：<c>preauthorized_usd = 0</c> 是「显式零额度」，不是「不限制」。
    /// 判据取**假 LLM 有没有收到请求**——只断言运行状态非成功是不够的，
    /// 一个「先花钱后检查」的实现会照样落到失败态，但钱已经花了。
    /// </summary>
    [Fact]
    public async Task WritingChain_ZeroBudget_BlocksRunBeforeAnyOutboundCall()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(WritingChain_ZeroBudget_BlocksRunBeforeAnyOutboundCall)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "零预算");
        await client.CreateProjectAsync(projectRoot, "零预算");
        var documentId = SeedChapter(projectRoot, "第一章\n");

        var (baseUrl, captured) = SpawnPlainLlm("不该被生成的内容。");
        await UserConfiguresProviderAsync(client, baseUrl);
        await UserAllowsWritingToolsAsync(client);

        // 设置页：日预算设成一个极小的正值（0 = 不设上限，见 U112 的双重语义）。
        var budget = await client.UpdateBudgetConfigAsync(0.000_001, 0.0);
        Assert.NotNull(budget);

        await UserBuildsWriterWorkflowAsync(client, "budget-flow", "写一段", documentId);
        var started = await client.RunWorkflowAsync("budget-flow");
        var final = await WaitForTerminalStateAsync(client, "budget-flow", started.RunId);

        Assert.NotEqual("succeeded", final.Status.ToLowerInvariant());
        Assert.True(
            final.Failure is not null || !string.IsNullOrWhiteSpace(final.PauseReason),
            "被预算挡住的运行必须携带 failure 或 pause_reason，否则用户只看到一个红点。"
            + $"实际：status={final.Status} pause={final.PauseReason} "
            + $"failure={final.Failure?.Code}/{final.Failure?.Message}");

        // 正文不得被改动。
        Assert.Equal("第一章\n", ReadChapter(projectRoot));

        // 关键判据：一次出站请求都不该发生。
        //
        // ⚠️ **不能 `await captured`**：`SpawnReplayLlm` 的捕获任务要等满 `rounds`
        // 个请求（或内部 60s 超时）才完成，零请求时它就是不完成 ⇒
        // `WaitAsync(8s)` 抛 TimeoutException，用例以「超时」失败而不是以
        // 「有请求发出」失败——**失败原因完全指错方向**（我按这条去查了半天预算门禁）。
        //
        // 改为：给足时间让「如果会发请求，它已经发了」，然后断言捕获任务
        // **仍未完成**。零请求 ⇒ 任务卡在 AcceptSocket ⇒ 未完成，这正是要证明的。
        await Task.Delay(TimeSpan.FromSeconds(3));
        Assert.False(
            captured.IsCompleted,
            "假 LLM 已经收到请求 ⇒ 预算门禁没有在**发出请求之前**拦住这次运行。"
            + "预算的意义就是「不该花的钱一分都不花」，事后拦住等于没拦。");
    }

    /// <summary>
    /// 用户配了 Provider 但**没存密钥**就点运行。
    ///
    /// ⚠️ **本用例的原前提是错的，已重写。**
    /// 原版断言「运行必须失败且错误指向凭据」，实测 `node node-1 succeeded`——
    /// 而那是**正确行为**：仓库对此早有定论（见
    /// `production_journey_contracts.rs` 的 `journey_provider_without_key_fails_loudly`
    /// 注释「对 OpenAiCompatible 缺密钥是合法配置」）。
    /// 本地自建服务（Ollama / LM Studio / vLLM）通常**不需要**密钥，
    /// 强制要求密钥会让这批用户完全用不了产品。
    ///
    /// 所以真正该钉的性质不是「缺密钥要失败」，而是
    /// **「缺密钥时不得把空凭据当成有效凭据发出去」**：
    /// 请求里要么不带 Authorization 头，要么带一个真实的值，
    /// 绝不能出现 `Authorization: Bearer `（空值）——那种请求会被真实
    /// OpenAI 端点以 401 拒掉，而用户看到的是一条含混的连接错误，
    /// 完全猜不到是「密钥没存上」。
    [Fact]
    public async Task WritingChain_MissingApiKey_DoesNotSendAnEmptyBearerToken()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(WritingChain_MissingApiKey_DoesNotSendAnEmptyBearerToken)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "缺密钥");
        await client.CreateProjectAsync(projectRoot, "缺密钥");
        var documentId = SeedChapter(projectRoot, "第一章\n");

        var (baseUrl, captured) = SpawnPlainLlm("本地服务无需密钥。");
        // 只保存 Provider，**不**调 SaveProviderKeyAsync。
        await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            ProviderId, "open_ai_compatible", "无密钥服务", true, baseUrl,
            new[] { new ModelConfig(ModelId, "llm", null, null, null) },
            true, false, false, false));
        await UserAllowsWritingToolsAsync(client);
        await UserBuildsWriterWorkflowAsync(client, "nokey-flow", "写一段", documentId);

        var started = await client.RunWorkflowAsync("nokey-flow");
        var final = await WaitForTerminalStateAsync(client, "nokey-flow", started.RunId);

        // 无密钥的本地服务能跑通，这是产品要支持的场景。
        Assert.Equal("succeeded", final.Status.ToLowerInvariant());

        var requests = await captured.WaitAsync(TimeSpan.FromSeconds(20));
        var request = Assert.Single(requests);

        // 判据：不得出现空的 Bearer。用逐行解析而不是整体 Contains——
        // 请求体里可能恰好含 "Bearer " 之类的字样（提示词是用户可控内容）。
        var authHeaders = request
            .Split('\n')
            .Select(line => line.TrimEnd('\r'))
            .Where(line => line.StartsWith("Authorization:", StringComparison.OrdinalIgnoreCase))
            .ToList();

        foreach (var header in authHeaders)
        {
            var value = header["Authorization:".Length..].Trim();
            Assert.False(
                string.IsNullOrWhiteSpace(value)
                || value.Equals("Bearer", StringComparison.OrdinalIgnoreCase)
                || value.Equals("Bearer ", StringComparison.Ordinal),
                $"没存密钥却发出了空的 Authorization：`{header}`。"
                + "真实 OpenAI 端点会以 401 拒掉，而用户看到的是含混的连接错误，"
                + "完全猜不到是「密钥没存上」——要么别带这个头，要么带真实值。");
        }
    }

    // ════════════════════════════════════════════════════════
    // 主线 5：重开项目后状态仍在（用户第二天回来接着写）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 关掉应用再打开：正文、确认项历史、git 历史、Provider 配置都必须还在。
    ///
    /// 用**新建一个客户端**（新 sidecar 进程）来模拟重启，而不是复用同一连接——
    /// 复用连接测不出「状态只活在内存里」这一整类缺陷。
    /// </summary>
    [Fact]
    public async Task WritingChain_ReopenProjectInNewProcess_PreservesProseAndHistory()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(WritingChain_ReopenProjectInNewProcess_PreservesProseAndHistory)))
        {
            return;
        }

        var projectRoot = Path.Combine(_temp.FullName, "重开项目");
        const string prose = "第二天他还是来了。";
        string runId;

        using (var first = new JsonLineBackendClient(sidecar))
        {
            await first.CreateProjectAsync(projectRoot, "重开项目");
            var documentId = SeedChapter(projectRoot, "第一章\n");
            var (baseUrl, _) = SpawnToolCallingLlm(new (string, object)[]
            {
                ("writer-insert-lines", new
                {
                    document_id = documentId,
                    after_line = 1,
                    text = prose + "\n",
                }),
            });
            await UserConfiguresProviderAsync(first, baseUrl);
            await UserAllowsWritingToolsAsync(first);
            await UserBuildsWriterWorkflowAsync(first, "reopen-flow", "续写", documentId);

            var started = await first.RunWorkflowAsync("reopen-flow");
            runId = started.RunId;
            await WaitForTerminalStateAsync(first, "reopen-flow", runId);
            var pending = await WaitForPendingConfirmationAsync(first, runId);
            await first.ResolveConfirmationAsync(
                "reopen-flow", runId, pending.ConfirmationId, "approve");
            Assert.Contains(prose, ReadChapter(projectRoot));
        }

        // ── 用户第二天重开应用 ──
        using var second = new JsonLineBackendClient(sidecar);
        var status = await second.OpenProjectAsync(projectRoot, "重开项目");
        Assert.NotNull(status);

        // 正文还在（这条最基本，但也最该钉住）。
        Assert.Contains(prose, ReadChapter(projectRoot));

        // Provider 配置还在——重启后要求用户重配一遍等于配置没落盘。
        var providers = await second.GetProviderConfigAsync();
        Assert.NotNull(providers);

        // 工作流图还在，且节点配置没丢。
        var graph = await second.LoadWorkflowGraphAsync("reopen-flow");
        var node = Assert.Single(graph.Nodes);
        Assert.Equal("writer", node.Type);
        Assert.True(
            node.Data.ContainsKey("document_id"),
            $"重开后 writer 节点丢了 document_id，下次运行会不知道改哪个文件；实际键={string.Join(",", node.Data.Keys)}");

        // 确认项历史还在，且已是已决议状态——审计链不能因重启而断。
        var confirmations = await second.ListConfirmationsAsync();
        var mine = confirmations.Where(entry =>
            string.Equals(entry.RunId, runId, StringComparison.Ordinal)).ToList();
        Assert.NotEmpty(mine);
        Assert.DoesNotContain(mine, entry =>
            entry.State.Equals("pending", StringComparison.OrdinalIgnoreCase));
    }
}

/// <summary>
/// <c>ProcessStartInfo</c> 的参数追加小工具。
///
/// 单独写一个是为了避免在测试体里用字符串拼 <c>Arguments</c>——
/// 项目路径含中文和空格，拼字符串会在含空格路径上间歇失败。
/// </summary>
internal static class ProcessStartInfoExtensions
{
    internal static System.Diagnostics.ProcessStartInfo WithArgs(
        this System.Diagnostics.ProcessStartInfo info, params string[] args)
    {
        foreach (var arg in args)
        {
            info.ArgumentList.Add(arg);
        }
        return info;
    }
}
