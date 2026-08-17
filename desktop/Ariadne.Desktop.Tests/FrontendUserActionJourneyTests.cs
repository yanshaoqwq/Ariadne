using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 桌面端**其余用户动作**的真实 sidecar 全流程：文档编辑与保存、快速编辑、
/// 项目 AI 问答、运行控制（暂停/恢复/停止）、Git 恢复到新分支、运行日志、
/// 最近项目管理、Provider 删除。
///
/// **立此文件的依据（先查过覆盖再写，不重复造）**：把 `IAriadneBackendClient`
/// 的用户动作逐个对照现有测试后发现，除 <c>ImportChapterAsync</c> 外，
/// 以下 19 个动作**全部零真实-sidecar 覆盖**——
/// QuickEdit / ApplyQuickEdit / ProjectAiChat / RestoreToNewBranch /
/// SaveDocumentContent / Pause / Resume / Stop / InstallTemplate /
/// RemoveProvider / QueryRunLogs / GetWorksTree / RelocateRecentProject /
/// ForgetRecentProject / ListInDoubtOperations / TestProviderDraft /
/// AppendProjectMemory / PackWorkflowSelection / ResolveProjectReference。
/// 它们**有测试**，但用的是内存假客户端——不做 serde 反序列化、不过进程边界。
/// U156 就是这么溜过去的：**假客户端下 100% 不复现**。
///
/// 与另两个旅程文件的分工：
/// - <see cref="FrontendProductionJourneyTests"/>：通用 llm 节点跑到终态
/// - <see cref="FrontendWritingChainJourneyTests"/>：写作链（产出→审批→落盘→导出）
/// - 本文件：**其余用户动作**，尤其是「用户自己动手编辑」和「运行中途干预」两条
///
/// sidecar 未编译时跳过（与 <see cref="BackendColdStartTests"/> 同约定）。
/// </summary>
public sealed class FrontendUserActionJourneyTests : IDisposable
{
    private const string ProviderId = "primary";
    private const string ModelId = "action-model";

    private readonly DirectoryInfo _temp =
        Directory.CreateTempSubdirectory("ariadne-user-action-");

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
        // U142：起真实 sidecar 前确认 app-state 已隔离，否则本测试建的项目会
        // 写进用户真实 recent_projects.json 并挤掉用户自己的项目（20 条上限）。
        // **本文件尤其要紧**：它专门测「最近项目」的增删，不隔离等于直接改用户数据。
        SidecarAppStateIsolation.RequireIsolatedAppState();
        Environment.SetEnvironmentVariable(
            "ARIADNE_SECRET_MASTER_KEY", "user-action-master-key");

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
    // 假 LLM
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 按序回放的真实 HTTP 端；<c>rounds</c> 为可服务的请求轮数。
    ///
    /// 按 Content-Length 读满请求体：提示词带上下文时轻易跨多个 TCP 包，
    /// 只 ReadAsync 一次会在小请求上碰巧通过、在大请求上间歇失败——
    /// 那种不稳定测试比没有测试更糟，因为它会被当成「环境问题」而不是被修。
    /// </summary>
    private static (string BaseUrl, Task<IReadOnlyList<string>> Captured) SpawnLlm(
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

        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        var baseUrl = $"http://127.0.0.1:{((IPEndPoint)listener.LocalEndpoint).Port}";

        var captured = Task.Run<IReadOnlyList<string>>(async () =>
        {
            var seen = new List<string>();
            try
            {
                for (var i = 0; i < rounds; i++)
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
                // 轮次少于预期：把已收到的交回，让断言报「差在哪一轮」，
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

    /// <summary>
    /// 慢速假 LLM：收到请求后**挂住不回**，直到调用方放行。
    ///
    /// 这是测「运行中途暂停/停止」的**必要条件**：节点必须真的停在
    /// 「已发出请求、等响应」这个状态上，用户才有窗口去点暂停。
    /// 用快速响应的假服务测暂停，会变成「等运行结束后再暂停」——
    /// 那测的是终态幂等，不是中途干预。
    /// </summary>
    private static (string BaseUrl, TaskCompletionSource Release, Task<int> Hits) SpawnStallingLlm()
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        var baseUrl = $"http://127.0.0.1:{((IPEndPoint)listener.LocalEndpoint).Port}";
        var release = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);

        var hits = Task.Run(async () =>
        {
            var count = 0;
            try
            {
                using var socket = await listener
                    .AcceptSocketAsync()
                    .WaitAsync(TimeSpan.FromSeconds(60));
                using var stream = new NetworkStream(socket, ownsSocket: false);
                await ReadHttpRequestAsync(stream);
                count = 1;
                // 挂住：直到测试放行或超时。放行后回一个正常响应，
                // 让运行能自然收尾（而不是靠连接被切造成额外的失败噪声）。
                await release.Task.WaitAsync(TimeSpan.FromSeconds(90));
                var body = JsonSerializer.Serialize(new
                {
                    model = ModelId,
                    choices = new[]
                    {
                        new
                        {
                            message = new { content = "迟到的回复。", tool_calls = Array.Empty<object>() },
                            finish_reason = "stop",
                        },
                    },
                    usage = new { prompt_tokens = 5, completion_tokens = 2 },
                });
                var payload = Encoding.UTF8.GetBytes(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
                    + $"Content-Length: {Encoding.UTF8.GetByteCount(body)}\r\n\r\n{body}");
                await stream.WriteAsync(payload);
                await stream.FlushAsync();
            }
            catch (TimeoutException)
            {
                // 没等到请求或没等到放行：把计数交回供断言。
            }
            finally
            {
                listener.Stop();
            }
            return count;
        });

        return (baseUrl, release, hits);
    }

    private static async Task<string> ReadHttpRequestAsync(NetworkStream stream)
    {
        var buffer = new byte[64 * 1024];
        var text = new StringBuilder();
        var headerEnd = -1;
        var total = 0;

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

        // 按**字节**计已读体长度：正文是中文，UTF-8 下一个汉字 3 字节，
        // 用字符数比较会永远读不满而卡到超时。
        var bodySoFar = total - Encoding.UTF8.GetByteCount(whole[..(headerEnd + 4)]);
        while (bodySoFar < contentLength)
        {
            var read = await stream.ReadAsync(buffer).AsTask().WaitAsync(TimeSpan.FromSeconds(30));
            if (read <= 0)
            {
                break;
            }
            bodySoFar += read;
            text.Append(Encoding.UTF8.GetString(buffer, 0, read));
        }

        return text.ToString();
    }

    // ════════════════════════════════════════════════════════
    // 共用步骤
    // ════════════════════════════════════════════════════════

    private static async Task ConfigureProviderAsync(
        IAriadneBackendClient client, string baseUrl)
    {
        await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            ProviderId, "open_ai_compatible", "我的模型服务", true, baseUrl,
            new[] { new ModelConfig(ModelId, "llm", null, null, null) },
            true, false, false, false));
        await client.SaveProviderKeyAsync(ProviderId, "sk-user-action");
    }

    private static Task<WorkflowGraphData> SaveSingleLlmWorkflowAsync(
        IAriadneBackendClient client, string workflowId, string prompt)
        => client.SaveWorkflowGraphAsync(new WorkflowGraphData(
            workflowId, workflowId,
            new[]
            {
                new CanvasNode("node-1", "llm", null, new Dictionary<string, object?>
                {
                    ["provider_id"] = ProviderId,
                    ["model_id"] = ModelId,
                    ["prompt_template"] = prompt,
                }, null),
            },
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>()));

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

    private static string SeedDocument(string projectRoot, string relative, string body)
    {
        var path = Path.Combine(projectRoot, relative);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, body);
        return path;
    }

    private static int CommitCount(string projectRoot) => RunGit(projectRoot, "rev-list", "--count", "HEAD")
        is { } text && int.TryParse(text.Trim(), out var count) ? count : 0;

    private static string CurrentBranch(string projectRoot)
        => RunGit(projectRoot, "rev-parse", "--abbrev-ref", "HEAD")?.Trim() ?? string.Empty;

    private static string? RunGit(string projectRoot, params string[] args)
    {
        // 用 ArgumentList 而非拼字符串：项目路径含中文与空格，
        // 拼字符串会在含空格路径上间歇失败。
        var info = new System.Diagnostics.ProcessStartInfo
        {
            FileName = "git",
            WorkingDirectory = projectRoot,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };
        foreach (var arg in args)
        {
            info.ArgumentList.Add(arg);
        }
        using var process = System.Diagnostics.Process.Start(info);
        if (process is null)
        {
            return null;
        }
        var output = process.StandardOutput.ReadToEnd();
        process.WaitForExit(10_000);
        return output;
    }

    // ════════════════════════════════════════════════════════
    // 动作 1：用户自己动手编辑正文并保存
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 作品页：打开章节 → 改字 → 保存 → 重新读回。
    ///
    /// **判据是「读回的内容 == 写下的内容」且磁盘同步**，不是「保存命令没报错」。
    /// 这条覆盖的是作者最高频的动作，却此前零真实-sidecar 覆盖。
    ///
    /// 特意用中文 + emoji + 换行：正文是中文，而 `version` 常按字节长度算，
    /// 只用 ASCII 测不出 UTF-8 长度相关的偏差。
    /// </summary>
    [Fact]
    public async Task UserAction_EditAndSaveChapter_RoundTripsThroughDiskAndBackend()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_EditAndSaveChapter_RoundTripsThroughDiskAndBackend)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "手动编辑");
        await client.CreateProjectAsync(projectRoot, "手动编辑");
        var documentId = SeedDocument(projectRoot, "chapters/chapter-01.md", "第一章\n初始内容。\n");

        // 读：作品页打开章节。
        var opened = await client.GetDocumentContentDetailsAsync(documentId);
        Assert.Contains("初始内容", opened.Content);
        Assert.False(string.IsNullOrWhiteSpace(opened.Metadata.Version), "读文档必须给出 version，否则无法做并发保护");

        // 写：用户改字后点保存，带上刚读到的 version。
        const string edited = "第一章\n他把窗关上了——雨还在下。🌧\n新的一段。\n";
        var report = await client.SaveDocumentContentAsync(documentId, edited, opened.Metadata.Version);
        Assert.NotNull(report.Metadata);
        Assert.NotEqual(opened.Metadata.Version, report.Metadata.Version);

        // 判据一：磁盘。
        Assert.Equal(edited, await File.ReadAllTextAsync(documentId));

        // 判据二：再次通过后端读回，内容一致（排除「只写了盘但缓存没更新」）。
        var reread = await client.GetDocumentContentDetailsAsync(documentId);
        Assert.Equal(edited, reread.Content);
        Assert.Equal(report.Metadata.Version, reread.Metadata.Version);
    }

    /// <summary>
    /// 并发保护：拿**过期的 version** 保存必须被拒，且磁盘不得被改。
    ///
    /// 真实场景：用户在两个窗口打开同一章，或后台工作流刚写过这个文件。
    /// 判据取**磁盘内容**而不是「有没有抛异常」——一个「先写盘后校验」的实现
    /// 会照样抛异常，但用户的另一份改动已经被覆盖掉了，这是不可逆的数据丢失。
    /// </summary>
    [Fact]
    public async Task UserAction_SaveWithStaleVersion_IsRejectedAndDiskUnchanged()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_SaveWithStaleVersion_IsRejectedAndDiskUnchanged)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "并发保存");
        await client.CreateProjectAsync(projectRoot, "并发保存");
        var documentId = SeedDocument(projectRoot, "chapters/chapter-01.md", "原始\n");

        var first = await client.GetDocumentContentDetailsAsync(documentId);
        var staleVersion = first.Metadata.Version;

        // 别处先改了一次（模拟另一个窗口/工作流写入）。
        var afterOther = await client.SaveDocumentContentAsync(documentId, "别人改的内容\n", staleVersion);
        Assert.NotEqual(staleVersion, afterOther.Metadata.Version);
        var contentAfterOther = await File.ReadAllTextAsync(documentId);

        // 用户拿着旧 version 保存自己的版本。
        BackendException? rejected = null;
        try
        {
            await client.SaveDocumentContentAsync(documentId, "我的内容会覆盖别人吗\n", staleVersion);
        }
        catch (BackendException ex)
        {
            rejected = ex;
        }

        Assert.NotNull(rejected);

        // 关键判据：磁盘仍是别人那一版，用户的写入没有落下去。
        Assert.Equal(contentAfterOther, await File.ReadAllTextAsync(documentId));
        Assert.DoesNotContain("我的内容会覆盖别人吗", await File.ReadAllTextAsync(documentId));
    }

    // ════════════════════════════════════════════════════════
    // 动作 2：快速编辑（选中一段让 AI 改写）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 作品页快速编辑：选中一段 → 给指令 → 拿到改写建议。
    ///
    /// 判据取**假 LLM 真的收到了选中的原文和用户指令**——
    /// 只断言「返回了 suggested」不够：一个把 selected_text 丢掉、
    /// 只把 instruction 发出去的实现照样能返回内容，而那内容与用户选的段落无关。
    /// </summary>
    [Fact]
    public async Task UserAction_QuickEdit_SendsSelectedTextAndInstructionToModel()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_QuickEdit_SendsSelectedTextAndInstructionToModel)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "快速编辑");
        await client.CreateProjectAsync(projectRoot, "快速编辑");
        var documentId = SeedDocument(projectRoot, "chapters/chapter-01.md", "第一章\n他很高兴。\n");

        const string suggested = "他嘴角压不住，眼睛先笑了。";
        var (baseUrl, captured) = SpawnLlm(suggested);
        await ConfigureProviderAsync(client, baseUrl);

        const string selected = "他很高兴。";
        const string instruction = "改成不直说情绪的写法";
        var result = await client.QuickEditAsync(new QuickEditRequest(selected, instruction, documentId));

        Assert.Equal(selected, result.Original);
        Assert.False(string.IsNullOrWhiteSpace(result.Suggested), "快速编辑必须返回改写建议");
        Assert.False(string.IsNullOrWhiteSpace(result.Diff), "快速编辑必须返回 diff 供用户复核，否则用户得自己逐字比对");

        // 关键判据：出站请求里同时含**选中的原文**与**用户指令**。
        var requests = await captured.WaitAsync(TimeSpan.FromSeconds(30));
        var outbound = Assert.Single(requests);
        Assert.Contains(instruction, outbound);
        Assert.Contains(selected, outbound);
    }

    /// <summary>
    /// <c>apply_quick_edit</c> 的落盘路径：<c>TextRange</c> 是**字节**偏移。
    ///
    /// ⚠️ **这条同时记录一个审查发现**：<c>ApplyQuickEditAsync</c> 在桌面端
    /// **零生产调用者**（全仓仅接口声明 + 客户端实现两处）。生产的快速编辑走
    /// 「前端本地拼接正文 → <c>SaveDocumentContentAsync</c> 整文件覆盖」，
    /// 因此后端整条 <c>apply_quick_edit</c> 链路（UTF-8 边界校验、patch 构造、
    /// 索引失效通知）在生产中**不可达**。
    ///
    /// 本用例仍然写：一是这条命令仍在协议面上、任何时候可能被接线；
    /// 二是它钉住了 <c>TextRange</c> 的**字节语义**——前端若接线时按 C# 的
    /// 字符索引（UTF-16）算偏移，中文正文下必然切在非字符边界上，
    /// 后端会拒（`quick edit range is not a valid UTF-8 slice`）或改错位置。
    /// 这里用中文正文构造真实字节偏移，把该语义固定下来。
    /// </summary>
    [Fact]
    public async Task UserAction_ApplyQuickEdit_UsesByteOffsetsNotCharIndexes()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_ApplyQuickEdit_UsesByteOffsetsNotCharIndexes)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "字节偏移");
        await client.CreateProjectAsync(projectRoot, "字节偏移");

        const string body = "第一章\n他很高兴。\n";
        var documentId = SeedDocument(projectRoot, "chapters/chapter-01.md", body);
        var opened = await client.GetDocumentContentDetailsAsync(documentId);

        const string selected = "他很高兴。";
        const string suggested = "他嘴角压不住。";

        // **字节**偏移：C# 的 IndexOf 给的是 UTF-16 字符索引，必须换算。
        // "第一章\n" = 3 汉字 ×3 + 换行 = 10 字节；而字符索引只有 4。
        // 直接把字符索引传给后端，会落在 "很" 的中间 → 非 UTF-8 边界 → 被拒。
        var charIndex = body.IndexOf(selected, StringComparison.Ordinal);
        var byteStart = Encoding.UTF8.GetByteCount(body[..charIndex]);
        var byteEnd = byteStart + Encoding.UTF8.GetByteCount(selected);
        Assert.NotEqual(charIndex, byteStart); // 中文下两者必然不同，否则本用例失去意义

        var report = await client.ApplyQuickEditAsync(
            documentId,
            opened.Metadata.Version,
            body,
            new TextRange(byteStart, byteEnd),
            new QuickEditResult(selected, suggested, $"- {selected}\n+ {suggested}"));
        Assert.NotNull(report);

        // 判据：磁盘上那一段被替换，且**其余部分完好**（切错边界会毁掉相邻字符）。
        var after = await File.ReadAllTextAsync(documentId);
        Assert.Contains(suggested, after);
        Assert.DoesNotContain(selected, after);
        Assert.StartsWith("第一章\n", after);
    }

    // ════════════════════════════════════════════════════════
    // 动作 3：项目 AI 问答
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 项目页 AI 对话：问一句 → 得到答案 → **追问时上一轮仍在上下文里**。
    ///
    /// ⚠️ **判据选择踩过一次坑，记下来免得下一个人重踩**：
    /// 初版断言 `second.ChatHistory.Count > first.ChatHistory.Count`，实测 2 → 0，
    /// 我一度以为是「追问丢上下文」的缺陷。**不是。**
    /// `commands.rs:9888` 处 `revision_protocol = request.conversation_id.is_some()`，
    /// 带 conversation_id 时 `chat_history` **刻意返回空**——会话改走增量协议
    /// （`new_messages` / `conversation_snapshot` / `conversation_revision`），
    /// 全量历史不再重复下发。**那个断言测的是一条产品已经不用的字段。**
    ///
    /// 所以改用生产实际的那条路径 <see cref="ProjectAiConversationUi.Apply"/>
    /// （`WorkspacePageViewModel.cs:3142` 就是这么调的），判据取
    /// **用户屏幕上的气泡数真的累积**——这才是「追问没丢上下文」的用户可见含义。
    /// </summary>
    [Fact]
    public async Task UserAction_ProjectAiChat_FollowUpAccumulatesVisibleConversation()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_ProjectAiChat_FollowUpAccumulatesVisibleConversation)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "项目问答");
        await client.CreateProjectAsync(projectRoot, "项目问答");

        // 两轮：第一问 + 追问。
        var (baseUrl, captured) = SpawnLlm("主角叫林昭。", rounds: 2);
        await ConfigureProviderAsync(client, baseUrl);

        // 生产端的会话状态三件套（与 WorkspacePageViewModel 同构）。
        var history = new List<ProjectAiChatMessage>();
        var bubbles = new System.Collections.ObjectModel.ObservableCollection<ChatBubbleViewModel>();
        long? revision = null;

        const string firstQuestion = "这本书的主角叫什么？";
        var first = await client.ProjectAiChatAsync(firstQuestion);
        Assert.False(string.IsNullOrWhiteSpace(first.Answer), "项目 AI 必须返回答案");
        revision = ProjectAiConversationUi.Apply(first, history, bubbles, revision);
        var bubblesAfterFirst = bubbles.Count;
        Assert.True(bubblesAfterFirst > 0, "第一轮问答后界面上一个气泡都没有");

        // 追问：带上 conversation_id + revision，走增量协议。
        const string followUp = "他多大年纪？";
        var second = await client.ProjectAiChatAsync(
            followUp,
            conversationId: string.IsNullOrWhiteSpace(first.ConversationId) ? null : first.ConversationId,
            conversationRevision: revision);
        Assert.False(string.IsNullOrWhiteSpace(second.Answer));
        ProjectAiConversationUi.Apply(second, history, bubbles, revision);

        Assert.True(
            bubbles.Count > bubblesAfterFirst,
            $"追问后界面气泡没有增加（{bubblesAfterFirst} → {bubbles.Count}），"
            + "用户看不到自己刚问的那句，会以为软件没反应");

        // 出站请求里必须含用户原话——占位符没渲染是 U120 那一族的形状。
        var requests = await captured.WaitAsync(TimeSpan.FromSeconds(30));
        Assert.NotEmpty(requests);
        Assert.Contains(firstQuestion, requests[0]);
        Assert.DoesNotContain("{{", requests[0]);

        // 第二轮真的也发出去了（而不是被本地缓存挡掉）。
        Assert.True(requests.Count >= 2, $"追问没有产生出站请求，实际只发了 {requests.Count} 次");
        Assert.Contains(followUp, requests[1]);
    }

    // ════════════════════════════════════════════════════════
    // 动作 4：运行中途干预（停止 / 暂停后恢复）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 用户在运行**进行中**点「停止」：运行必须进入非成功终态。
    ///
    /// 用挂住不回的假 LLM 把节点钉在「已发请求、等响应」上，
    /// 这样「停止」才落在真正的运行中途。用快速响应的假服务测，
    /// 会变成「运行早已结束后再点停止」——那测的是终态幂等，不是中途干预。
    ///
    /// ⚠️ 断言刻意宽松（只要求「不是 succeeded」+「给出可读原因」）：
    /// 停止请求与节点完成之间存在真实竞态，强求 `stopped` 会造出一条
    /// 间歇性失败的测试，而间歇失败会被当成环境问题而不是被修。
    /// </summary>
    [Fact]
    public async Task UserAction_StopMidRun_EndsRunWithReadableReason()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_StopMidRun_EndsRunWithReadableReason)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "中途停止");
        await client.CreateProjectAsync(projectRoot, "中途停止");

        var (baseUrl, release, hits) = SpawnStallingLlm();
        await ConfigureProviderAsync(client, baseUrl);
        await SaveSingleLlmWorkflowAsync(client, "stop-flow", "写一段很长的文字");

        var started = await client.RunWorkflowAsync("stop-flow");

        // 等到请求真的发出去了（节点已在等响应），此刻才是「运行中途」。
        for (var i = 0; i < 300 && !hits.IsCompleted; i++)
        {
            var state = await client.GetWorkflowRunStateAsync("stop-flow", started.RunId);
            if (state.Status.Equals("running", StringComparison.OrdinalIgnoreCase))
            {
                break;
            }
            await Task.Delay(100);
        }

        var action = await client.StopWorkflowAsync("stop-flow", started.RunId, "用户点了停止");
        Assert.Equal(started.RunId, action.RunId);

        release.TrySetResult(); // 放开假 LLM，让运行自然收尾
        var final = await WaitForTerminalStateAsync(client, "stop-flow", started.RunId);

        Assert.NotEqual("succeeded", final.Status.ToLowerInvariant());
        Assert.True(
            !string.IsNullOrWhiteSpace(final.StopReason)
            || !string.IsNullOrWhiteSpace(final.PauseReason)
            || final.Failure is not null,
            $"被用户停止的运行没有任何可读原因（status={final.Status}），执行页只会显示一个红点");
    }

    /// <summary>
    /// 暂停 → 恢复：恢复后必须能走到终态，且**不重复消耗**已完成节点。
    ///
    /// 判据取假 LLM 的**命中次数**：单节点工作流暂停恢复后若重跑该节点，
    /// 出站请求会变成 2 次——那意味着用户每次暂停恢复都要多付一次钱，
    /// 而运行状态看起来完全正常。这是「看不见的重复计费」，
    /// 只有数真实出站次数才能发现。
    ///
    /// ⚠️ **数出站次数的方式踩过一次坑，记下来**：初版用
    /// `SpawnLlm(rounds: 2)` 然后 `await captured`，结果**测试自己超时**——
    /// 那个 Task 要等满 2 轮才返回，而正确实现只发 1 轮，
    /// 于是它永远等不到第 2 个连接。**「多开一轮当陷阱」与
    /// 「等 Task 收尾拿计数」这两件事互相冲突**：陷阱轮不被踩到时，
    /// 收尾就永远不会发生。改用 `Interlocked` 计数器 + 独立的观察窗口，
    /// 计数随时可读，不依赖任何一轮是否被消耗。
    /// </summary>
    [Fact]
    public async Task UserAction_PauseThenResume_DoesNotReRunCompletedNode()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_PauseThenResume_DoesNotReRunCompletedNode)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "暂停恢复");
        await client.CreateProjectAsync(projectRoot, "暂停恢复");

        // 计数式假 LLM：服务任意多轮，命中数随时可读。
        var (baseUrl, hitCount) = SpawnCountingLlm("一段文字。");
        await ConfigureProviderAsync(client, baseUrl);
        await SaveSingleLlmWorkflowAsync(client, "pause-flow", "写一段");

        var started = await client.RunWorkflowAsync("pause-flow");

        // 暂停请求可能落在运行已结束之后——这是真实竞态，不是缺陷。
        // 所以只要求「命令本身不炸」，不要求一定观察到 paused 状态。
        try
        {
            await client.PauseWorkflowAsync("pause-flow", started.RunId, "用户点了暂停");
        }
        catch (BackendException)
        {
            // 运行已终态时暂停被拒是可接受的响应形态。
        }

        var afterPause = await client.GetWorkflowRunStateAsync("pause-flow", started.RunId);
        if (afterPause.Status.Equals("paused", StringComparison.OrdinalIgnoreCase))
        {
            await client.ResumeWorkflowAsync("pause-flow", started.RunId);
        }

        var final = await WaitForTerminalStateAsync(client, "pause-flow", started.RunId);
        Assert.True(
            final.Status.ToLowerInvariant() is "succeeded" or "paused",
            $"暂停恢复后运行没能走到终局：status={final.Status}, "
            + $"pause={final.PauseReason}, failure={final.Failure?.Message}");

        // 留一个观察窗口：若实现会重跑节点，第 2 次请求就发生在这段时间里。
        await Task.Delay(2000);

        // 关键判据：出站请求不得超过 1 次。
        var hits = hitCount();
        Assert.True(
            hits <= 1,
            $"单节点工作流在暂停/恢复后向模型发了 {hits} 次请求——"
            + "已完成的节点被重跑，用户每次暂停恢复都会多付一次钱");
    }

    /// <summary>
    /// 计数式假 LLM：服务任意多轮，返回一个随时可读的命中计数委托。
    ///
    /// 与 <see cref="SpawnLlm"/> 的区别：那个要等固定轮数收尾才能拿结果，
    /// 用来做「多开一轮当陷阱」的断言会把测试自己挂住（见上面那条注释）。
    /// 这里用 <see cref="Interlocked"/> 计数，读取不需要等任何轮次完成。
    /// </summary>
    private static (string BaseUrl, Func<int> HitCount) SpawnCountingLlm(string content)
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

        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        var baseUrl = $"http://127.0.0.1:{((IPEndPoint)listener.LocalEndpoint).Port}";
        var hits = 0;

        _ = Task.Run(async () =>
        {
            try
            {
                while (true)
                {
                    using var socket = await listener.AcceptSocketAsync();
                    using var stream = new NetworkStream(socket, ownsSocket: false);
                    await ReadHttpRequestAsync(stream);
                    Interlocked.Increment(ref hits);
                    var payload = Encoding.UTF8.GetBytes(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
                        + $"Content-Length: {Encoding.UTF8.GetByteCount(body)}\r\n\r\n{body}");
                    await stream.WriteAsync(payload);
                    await stream.FlushAsync();
                }
            }
            catch
            {
                // listener 被关闭即正常收尾。
            }
        });

        return (baseUrl, () => Volatile.Read(ref hits));
    }

    // ════════════════════════════════════════════════════════
    // 动作 5：Git 恢复到新分支（用户想回到旧版本）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// Git 页：选一个历史点 → 恢复到新分支。
    ///
    /// 判据取**真实 git 分支状态**：`git rev-parse --abbrev-ref HEAD` 必须
    /// 变成新分支名。只断言 `RestoreReport.NewBranch` 等于传入值是**恒真的**
    /// ——它就是把入参回显出来，一个什么都没做的实现照样能过。
    /// </summary>
    [Fact]
    public async Task UserAction_RestoreToNewBranch_ActuallySwitchesGitBranch()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_RestoreToNewBranch_ActuallySwitchesGitBranch)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "回到旧版");
        var report = await client.CreateProjectAsync(projectRoot, "回到旧版");
        if (!report.GitInitialized)
        {
            return; // 环境无 git：本条无从判定。
        }

        var documentId = SeedDocument(projectRoot, "chapters/chapter-01.md", "第一版\n");
        var first = await client.GetDocumentContentDetailsAsync(documentId);
        var point1 = await client.CreateCheckpointAsync("第一版");
        Assert.False(string.IsNullOrWhiteSpace(point1.CommitId));

        await client.SaveDocumentContentAsync(documentId, "第二版\n", first.Metadata.Version);
        var point2 = await client.CreateCheckpointAsync("第二版");
        Assert.NotEqual(point1.CommitId, point2.CommitId);

        var branchBefore = CurrentBranch(projectRoot);
        var commitsBefore = CommitCount(projectRoot);

        // 用户在时间线上点回第一版。
        const string newBranch = "restore-第一版";
        var restore = await client.RestoreToNewBranchAsync(point1.CommitId, newBranch);
        Assert.Equal(newBranch, restore.NewBranch);

        // 判据：真实 git 分支已切换。
        var branchAfter = CurrentBranch(projectRoot);
        Assert.NotEqual(branchBefore, branchAfter);
        Assert.Equal(newBranch, branchAfter);

        // 判据：历史没被销毁（恢复是**开新分支**，不是重写历史）。
        Assert.True(
            CommitCount(projectRoot) >= commitsBefore - 1,
            $"恢复后 commit 数从 {commitsBefore} 掉到 {CommitCount(projectRoot)}，像是重写了历史而非开新分支");

        // 判据：Git 页能读到当前状态且不报损坏。
        var status = await client.GetGitRepositoryStatusAsync();
        Assert.Equal(newBranch, status.Branch);
    }

    // ════════════════════════════════════════════════════════
    // 动作 6：运行日志（用户排障时看的东西）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 跑完一个工作流后，运行日志页必须**真的有这次运行的记录**。
    ///
    /// ⚠️ **当前失败，失败即缺陷存在——见 U158**。实测：成功运行产生
    /// **0 条**运行日志（整库 0 条），而同一次运行的 runtime events 有 5 条。
    /// 生产里唯一会写带 run_id 日志的地方是**错误分支**
    /// （`commands.rs:12429`，kind/level 都是 Error）；公开入口
    /// `append_run_log` 的 4 个调用者**全部在 `#[cfg(test)]` 内**
    /// （cli.rs:393 / rest.rs:652 之后）。而日志页只读 `query_run_logs`、
    /// **不读 events**（`RunLogPageViewModel` 里 `GetWorkflowEventsAsync` 命中 0 次）。
    /// 结果：用户跑成功十次，运行日志页始终空白。
    ///
    /// **判据必须是「用户能在这个页面上看到成功运行」**，
    /// 不能是「`query_run_logs` 不抛异常」——后者现在就是绿的，
    /// 一个恒返回空列表的实现照样能过。这是本条的变异测试点。
    /// </summary>
    [Fact]
    public async Task UserAction_RunLogs_ContainEntriesForTheRunJustExecuted()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_RunLogs_ContainEntriesForTheRunJustExecuted)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "运行日志");
        await client.CreateProjectAsync(projectRoot, "运行日志");

        var (baseUrl, _) = SpawnLlm("写好了。");
        await ConfigureProviderAsync(client, baseUrl);
        await SaveSingleLlmWorkflowAsync(client, "log-flow", "写一段");

        var started = await client.RunWorkflowAsync("log-flow");
        var final = await WaitForTerminalStateAsync(client, "log-flow", started.RunId);
        Assert.Equal("succeeded", final.Status.ToLowerInvariant());

        // 运行事件必须能读到——执行页的进度就是靠它渲染的。
        // 这一半当前是通的，先钉住防回归。
        var events = await client.GetWorkflowEventsAsync("log-flow", started.RunId);
        Assert.NotEmpty(events.Events);

        // 日志落库可能略滞后于终态，轮询而非单次读取。
        IReadOnlyList<UiRunLogEntry> logs = Array.Empty<UiRunLogEntry>();
        for (var i = 0; i < 100; i++)
        {
            logs = await client.QueryRunLogsAsync(new RunLogQuery(RunId: started.RunId, Limit: 200));
            if (logs.Count > 0)
            {
                break;
            }
            await Task.Delay(100);
        }

        Assert.True(
            logs.Count > 0,
            $"运行 {started.RunId} 已 succeeded，但运行日志页的数据源里一条记录都没有"
            + $"（同一次运行的 runtime events 有 {events.Events.Count} 条）。"
            + "用户跑成功后打开运行日志页只会看到空白 —— U158。");
        Assert.All(logs, entry => Assert.Equal(started.RunId, entry.RunId));
    }

    // ════════════════════════════════════════════════════════
    // 动作 7：最近项目管理（欢迎页）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 欢迎页：新建的项目进入最近列表 → 移出后不再出现。
    ///
    /// ⚠️ **本条最需要 U142 隔离**：它直接读写「最近项目」这份用户数据。
    /// 顺便断言隔离生效——列表里的路径必须都在本进程的隔离区内，
    /// 若出现用户真实目录下的项目，说明隔离已破，测试正在改用户数据。
    /// </summary>
    [Fact]
    public async Task UserAction_RecentProjects_AddAndForgetRoundTrip()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_RecentProjects_AddAndForgetRoundTrip)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "最近项目");
        await client.CreateProjectAsync(projectRoot, "最近项目");

        var listed = await client.ListRecentProjectsAsync();
        Assert.Contains(listed, entry => PathsEqual(entry.ProjectRoot, projectRoot));

        // 移出后不得再出现。
        var afterForget = await client.ForgetRecentProjectAsync(projectRoot);
        Assert.DoesNotContain(afterForget, entry => PathsEqual(entry.ProjectRoot, projectRoot));

        // 再读一次确认落盘（而不是只改了内存里的返回值）。
        var reread = await client.ListRecentProjectsAsync();
        Assert.DoesNotContain(reread, entry => PathsEqual(entry.ProjectRoot, projectRoot));
    }

    private static bool PathsEqual(string? a, string? b)
    {
        if (string.IsNullOrWhiteSpace(a) || string.IsNullOrWhiteSpace(b))
        {
            return false;
        }
        // 规范化后比较：后端可能返回经符号链接解析过的路径
        // （本机 ~/.config → /custdata/.config），直接比字符串会误判。
        return string.Equals(
            Path.GetFullPath(a).TrimEnd(Path.DirectorySeparatorChar),
            Path.GetFullPath(b).TrimEnd(Path.DirectorySeparatorChar),
            StringComparison.Ordinal);
    }

    // ════════════════════════════════════════════════════════
    // 动作 8：删除 Provider（用户换服务商）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 设置页：预览删除影响 → 确认删除。
    ///
    /// 删除必须带 `expected_revision`（乐观锁），而**那个 revision 只能来自
    /// 本次 preview**：后端 `remove_provider`（`commands.rs:4407`）拿它与
    /// **重新计算的 preview.revision** 比对，不等就 conflict。
    ///
    /// ⚠️ **初版在这里踩了坑，记下来**：我原先写了个「从任意配置对象里找
    /// revision 字段」的通用提取器，它先从 `ProviderConfigStatus` 捞到了一个
    /// **别的** revision，删除被 conflict 拒——而我当时没断言删除成功，
    /// 错误被静默吞掉，最后只在「配置里还有 primary」上失败，
    /// 看起来像「删除功能坏了」。**判据链上少一环断言，就会把自己的用法错误
    /// 报成产品缺陷。** 现在直接用 `preview.Revision`（DTO 上就有这个字段，
    /// 不需要通用提取器），并显式断言删除调用本身不抛。
    /// </summary>
    [Fact]
    public async Task UserAction_RemoveProvider_PreviewThenDeleteLeavesNoTrace()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_RemoveProvider_PreviewThenDeleteLeavesNoTrace)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "换服务商");
        await client.CreateProjectAsync(projectRoot, "换服务商");

        var (baseUrl, _) = SpawnLlm("x", rounds: 0);
        await ConfigureProviderAsync(client, baseUrl);

        var before = await client.GetProviderConfigAsync();
        Assert.Contains(before.Providers, entry => entry.Provider == ProviderId);

        // 先预览：用户动手前要能看到影响面（哪些角色会失去默认路由、有没有阻塞引用）。
        var preview = await client.PreviewProviderRemovalAsync(ProviderId);
        Assert.Equal(ProviderId, preview.ProviderId);
        Assert.False(
            string.IsNullOrWhiteSpace(preview.Revision),
            "预览没给出 revision，前端无从构造合法的删除请求");
        Assert.Contains("llm", preview.DefaultRoles);
        Assert.Empty(preview.BlockingReferences);

        // 再删除，带上**预览给的**那个 revision。
        var after = await client.RemoveProviderAsync(ProviderId, preview.Revision);

        // ⚠️ 以下两条当前失败，失败即缺陷存在——见 U157。
        // `save_provider_settings` 把 Provider 同时写进项目配置**和**应用级目录
        // （`app-state/provider_catalog.json`，commands.rs:4316-4326），
        // 而 `remove_provider` 只清项目配置那一份（全函数不出现
        // ProviderCatalogStore），目录条目又被
        // `provider_config_status_from_config_with_app_state`（:8860-8871）
        // 合并回列表。`ProviderCatalog` 上连删除方法都不存在（grep fn remove = 0）。
        Assert.DoesNotContain(after.Providers, entry => entry.Provider == ProviderId);

        // 判据：重新读配置，确认它真的不在了（排除「只改了返回值没落盘」）。
        var reread = await client.GetProviderConfigAsync();
        Assert.DoesNotContain(reread.Providers, entry => entry.Provider == ProviderId);

        // 磁盘判据：这一条是 U157 的核心判据。
        // 只断言 API 返回值不够——一个「返回时过滤掉、磁盘照留」的实现会全绿，
        // 而 base_url 与模型清单仍原样留在盘上。
        var catalogPath = Path.Combine(
            SidecarAppStateIsolation.ClientVisibleAppStateRoot(), "provider_catalog.json");
        if (File.Exists(catalogPath))
        {
            Assert.DoesNotContain(
                $"\"{ProviderId}\"",
                await File.ReadAllTextAsync(catalogPath));
        }

        // 以下两条当前**已正确**，写成断言防回归：
        // 默认 LLM 路由已清（留着指向已删 Provider 的默认值，下次运行会报
        // provider not found 而设置页看起来一切正常）。
        Assert.NotEqual(ProviderId, reread.DefaultLlmProviderId);
    }

    // ════════════════════════════════════════════════════════
    // 动作 9：项目记忆（用户让 AI 记住设定）
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 项目页：往项目记忆里追加一条 → 读回必须包含它。
    ///
    /// 判据取「读回的内容含新写入的那句」+「旧内容仍在」——
    /// 只断言追加命令不报错，测不出「每次追加都覆盖上一条」这种缺陷，
    /// 而那会让用户以为记忆在累积、实际只留最后一条。
    /// </summary>
    [Fact]
    public async Task UserAction_AppendProjectMemory_AccumulatesInsteadOfOverwriting()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(UserAction_AppendProjectMemory_AccumulatesInsteadOfOverwriting)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "项目记忆");
        await client.CreateProjectAsync(projectRoot, "项目记忆");

        const string firstNote = "主角林昭是左撇子。";
        const string secondNote = "第三章下雨，后文不要写成晴天。";

        await client.AppendProjectMemoryAsync(firstNote);
        var afterFirst = await client.ReadProjectMemoryAsync();
        Assert.Contains(firstNote, afterFirst);

        await client.AppendProjectMemoryAsync(secondNote);
        var afterSecond = await client.ReadProjectMemoryAsync();

        Assert.Contains(secondNote, afterSecond);
        Assert.Contains(
            firstNote,
            afterSecond);
    }
}
