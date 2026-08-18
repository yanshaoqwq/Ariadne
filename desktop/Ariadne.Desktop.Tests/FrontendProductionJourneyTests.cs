using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 前端全链条生产旅程：**真实 sidecar 进程** + 真实 JSON-line IPC + 假 LLM HTTP 服务。
///
/// 这是唯一从桌面端视角覆盖「新建项目 → 配 Provider → 存密钥 → 建工作流 →
/// 点运行 → 轮询到终态」完整链路的测试。此前 Rust 侧的
/// `production_journey_contracts.rs` 只从命令层进入，桌面 DTO 序列化、
/// IPC 命令名、参数拼装这一整层从未被端到端验证过——按钮「点不动」
/// 的问题大多死在这一层。
///
/// sidecar 未编译时自动跳过（与 BackendColdStartTests 同约定）。
/// </summary>
[Collection("RealSidecar")]
public sealed class FrontendProductionJourneyTests : IDisposable
{
    private readonly DirectoryInfo _temp =
        Directory.CreateTempSubdirectory("ariadne-frontend-journey-");

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
        // U142：起真实 sidecar 前先确认 app-state 已隔离。此前本文件建的
        // 「我的小说 / 迭代项目 / 断网项目」全部写进了用户真实的
        // recent_projects.json，20 条上限把用户自己的项目挤没了。
        SidecarAppStateIsolation.RequireIsolatedAppState();

        // 无 keychain 的开发构建里，LocalFileSecretStore 只认这个环境变量
        // （core/src/config/secrets.rs:315）。真实产品在 Linux 上同样会踩到
        // 「保存密钥必报错且无主密码 UI」——已立 U118 跟踪；测试先注入
        // 变量以便继续验证其余链路。
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

    /// <summary>
    /// 单请求假 LLM：收一个 chat 请求，回一段固定文本，并把请求原文交回。
    /// </summary>
    private static (string BaseUrl, Task<string> Captured) SpawnFakeLlm(string replyContent)
    {
        var listener = new TcpListener(IPAddress.Loopback, 0);
        listener.Start();
        var baseUrl = $"http://127.0.0.1:{((IPEndPoint)listener.LocalEndpoint).Port}";

        var captured = Task.Run(async () =>
        {
            using var socket = await listener.AcceptSocketAsync().WaitAsync(TimeSpan.FromSeconds(30));
            using var stream = new NetworkStream(socket, ownsSocket: false);
            var buffer = new byte[262_144];
            var read = await stream.ReadAsync(buffer).AsTask().WaitAsync(TimeSpan.FromSeconds(10));
            var request = Encoding.UTF8.GetString(buffer, 0, read);

            var body = JsonSerializer.Serialize(new
            {
                model = "journey-model",
                choices = new[]
                {
                    new
                    {
                        message = new { content = replyContent, tool_calls = Array.Empty<object>() },
                        finish_reason = "stop",
                    },
                },
                usage = new { prompt_tokens = 12, completion_tokens = 6 },
            });
            var response =
                $"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {Encoding.UTF8.GetByteCount(body)}\r\n\r\n{body}";
            await stream.WriteAsync(Encoding.UTF8.GetBytes(response));
            listener.Stop();
            return request;
        });

        return (baseUrl, captured);
    }

    /// <summary>
    /// **旅程主线**：桌面端从零到跑通第一个工作流。
    ///
    /// 每一步都是前端按钮实际触发的 IPC 调用；任何一步失败即说明
    /// 「按钮点了没反应 / 报错」的链路断点在哪。
    /// </summary>
    [Fact]
    public async Task DesktopJourney_CreateProject_ConfigureProvider_RunWorkflow_ReachesTerminalState()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(DesktopJourney_CreateProject_ConfigureProvider_RunWorkflow_ReachesTerminalState)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "我的小说");

        // ── 第 1 步：欢迎页「新建项目」按钮 ──
        var report = await client.CreateProjectAsync(projectRoot, "我的小说");
        Assert.NotNull(report);
        Assert.True(Directory.Exists(projectRoot), "新建项目后目录必须真实存在");

        // ── 第 2 步：设置页「保存 Provider」按钮 ──
        var (baseUrl, capturedRequest) = SpawnFakeLlm("夜色像一封没写完的信。");
        var providerStatus = await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            ProviderId: "primary",
            ProviderType: "open_ai_compatible",
            DisplayName: "我的模型服务",
            Enabled: true,
            BaseUrl: baseUrl,
            Models: new[]
            {
                new ModelConfig("journey-model", "llm", null, null, null),
            },
            MakeDefaultLlm: true,
            MakeDefaultEmbedding: false,
            MakeDefaultReranker: false,
            MakeDefaultSearch: false));
        Assert.NotNull(providerStatus);

        // ── 第 3 步：设置页「保存密钥」按钮 ──
        var keyStatus = await client.SaveProviderKeyAsync("primary", "sk-frontend-journey");
        Assert.NotNull(keyStatus);

        // ── 第 4 步：画布「保存工作流」（拖一个 LLM 节点，配好路由与提示词） ──
        var graph = await client.SaveWorkflowGraphAsync(new WorkflowGraphData(
            WorkflowId: "first-flow",
            Name: "第一个工作流",
            Nodes: new[]
            {
                new CanvasNode(
                    Id: "node-1",
                    Type: "llm",
                    Label: null,
                    Data: new Dictionary<string, object?>
                    {
                        ["provider_id"] = "primary",
                        ["model_id"] = "journey-model",
                        ["prompt_template"] = "写一句开场",
                    },
                    Position: null),
            },
            Edges: Array.Empty<CanvasEdge>(),
            Metadata: new Dictionary<string, object?>()));
        Assert.NotNull(graph.ContentRevision);

        // ── 第 5 步：执行页「运行」按钮 ──
        var started = await client.RunWorkflowAsync("first-flow");
        Assert.False(string.IsNullOrWhiteSpace(started.RunId), "运行必须返回 run_id");

        // ── 第 6 步：执行页轮询运行状态直到终态（用户看着进度条） ──
        var status = await WaitForTerminalStateAsync(client, "first-flow", started.RunId);
        Assert.Equal("succeeded", status);

        // ── 终局断言：LLM 真的收到了请求，且带着用户的提示词 ──
        var outbound = await capturedRequest.WaitAsync(TimeSpan.FromSeconds(10));
        Assert.Contains("写一句开场", outbound);
        Assert.Contains("journey-model", outbound);
    }

    /// <summary>
    /// 旅程支线：用户配错端口（服务不可达）后点运行——
    /// 必须报出可诊断的失败，绝不能报成功。
    /// </summary>
    [Fact]
    public async Task DesktopJourney_UnreachableProvider_RunFailsLoudly()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(DesktopJourney_UnreachableProvider_RunFailsLoudly)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "断网项目");
        await client.CreateProjectAsync(projectRoot, "断网项目");

        await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            "primary", "open_ai_compatible", "不可达服务", true,
            "http://127.0.0.1:1",
            new[] { new ModelConfig("m", "llm", null, null, null) },
            true, false, false, false));
        await client.SaveProviderKeyAsync("primary", "sk-x");

        await client.SaveWorkflowGraphAsync(new WorkflowGraphData(
            "dead-flow", "断网流",
            new[]
            {
                new CanvasNode("node-1", "llm", null, new Dictionary<string, object?>
                {
                    ["provider_id"] = "primary",
                    ["model_id"] = "m",
                    ["prompt_template"] = "写一段",
                }, null),
            },
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>()));

        var started = await client.RunWorkflowAsync("dead-flow");
        var status = await WaitForTerminalStateAsync(client, "dead-flow", started.RunId);

        // 连接失败被分类为 Retryable（core/src/workflow/runtime.rs:2695），重试 3 次
        // （退避 1s/2s/4s）耗尽后落 `Paused` 而非 `Failed`——见 runtime.rs:1888-1894 的
        // 三分支：可重试但已耗尽 ⇒ 等人工介入。因此这里只断言「不是成功」，
        // 并要求携带可诊断信息，不强求 failed。
        Assert.NotEqual("succeeded", status);

        // 失败细节必须可诊断：用户要能从运行状态里看出是连不上服务。
        var state = await client.GetWorkflowRunStateAsync("dead-flow", started.RunId);
        Assert.True(
            state.Failure is not null || !string.IsNullOrWhiteSpace(state.PauseReason),
            "失败的运行必须携带 failure 或 pause_reason，否则用户只看到一个红点");
    }

    /// <summary>
    /// 旅程支线：同一客户端连续跑两个工作流（用户改完再跑是最常见循环）。
    /// 覆盖「第二次运行」的状态残留问题——第一次的运行态不得污染第二次。
    /// </summary>
    [Fact]
    public async Task DesktopJourney_SecondRunAfterEdit_StartsCleanly()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(DesktopJourney_SecondRunAfterEdit_StartsCleanly)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "迭代项目");
        await client.CreateProjectAsync(projectRoot, "迭代项目");

        var (baseUrl1, captured1) = SpawnFakeLlm("初稿。");
        await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            "primary", "open_ai_compatible", "服务", true, baseUrl1,
            new[] { new ModelConfig("m", "llm", null, null, null) },
            true, false, false, false));
        await client.SaveProviderKeyAsync("primary", "sk-x");

        var graph = await client.SaveWorkflowGraphAsync(BuildSingleLlmGraph("iter-flow", "写初稿", revision: null));
        var run1 = await client.RunWorkflowAsync("iter-flow");
        Assert.Equal("succeeded", await WaitForTerminalStateAsync(client, "iter-flow", run1.RunId));
        _ = await captured1.WaitAsync(TimeSpan.FromSeconds(10));

        // 用户修改提示词并保存（带上上次的 revision，模拟真实编辑流）
        var (baseUrl2, captured2) = SpawnFakeLlm("修改稿。");
        await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            "primary", "open_ai_compatible", "服务", true, baseUrl2,
            new[] { new ModelConfig("m", "llm", null, null, null) },
            true, false, false, false));
        await client.SaveWorkflowGraphAsync(BuildSingleLlmGraph(
            "iter-flow", "按编辑意见重写第一段", revision: graph.ContentRevision));

        var run2 = await client.RunWorkflowAsync("iter-flow");
        Assert.NotEqual(run1.RunId, run2.RunId);
        Assert.Equal("succeeded", await WaitForTerminalStateAsync(client, "iter-flow", run2.RunId));

        var outbound2 = await captured2.WaitAsync(TimeSpan.FromSeconds(10));
        Assert.Contains("按编辑意见重写第一段", outbound2);
    }

    private static WorkflowGraphData BuildSingleLlmGraph(string id, string prompt, string? revision)
        => new(
            id, id,
            new[]
            {
                new CanvasNode("node-1", "llm", null, new Dictionary<string, object?>
                {
                    ["provider_id"] = "primary",
                    ["model_id"] = "m",
                    ["prompt_template"] = prompt,
                }, null),
            },
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: null,
            ExpectedRevision: revision);

    private static async Task<string> WaitForTerminalStateAsync(
        IAriadneBackendClient client,
        string workflowId,
        string runId)
    {
        // 窗口必须覆盖「连接超时 + 完整退避序列」：NodeRetryPolicy 默认
        // max_attempts=3、退避 1s/2s/4s（core/src/workflow/runtime.rs），
        // 再叠加不可达端点的 TCP 超时。窗口过短会把「慢」误报成「卡死」。
        WorkflowRunState? last = null;
        for (var i = 0; i < 600; i++)
        {
            last = await client.GetWorkflowRunStateAsync(workflowId, runId);
            var status = last.Status.ToLowerInvariant();
            // `paused` 也是终局：重试耗尽后运行进入暂停并带上 pause_reason，
            // 等待用户决定重试还是放弃。把它排除在终态之外会让「已经给出
            // 可诊断结论」的运行被误判成卡死。
            if (status is "succeeded" or "failed" or "stopped" or "paused")
            {
                return status;
            }
            await Task.Delay(100);
        }
        throw new TimeoutException(
            $"等待工作流终态超时；最后状态={last?.Status}, pause={last?.PauseReason}, "
            + $"failure={last?.Failure?.Message}, events=[{string.Join(" | ", last?.Events ?? Array.Empty<string>())}]");
    }
}
