using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U171：画布编排全流程——搭图 → 调节点属性 → 填运行参数 → 跑起来 →
/// **那些属性与参数真的出现在发给模型的请求里**。
///
/// 与既有 `FrontendCanvasEditJourneyTests` 的分工：那份验「编辑动作是否落盘」
/// （判据取 `workflows/default.json` 的磁盘内容）。本份接着往下验
/// **落盘之后是否生效**——判据取**真实出站 HTTP 请求体**。
///
/// 为什么必须看出站请求：这一簇的失效形态是「配置写对了、跑起来了、
/// 但模型收到的不是那份配置」。U156 就是同型（variables 为 null 导致运行恒失败）。
/// 磁盘断言与运行成功断言都拦不住它——只有把请求体抓下来读才行。
/// 所以这里自起 TcpListener 当模型服务端，捕获真实请求。
/// </summary>
public sealed class CanvasAuthoringJourneyTests : IDisposable
{
    // ⚠️ 不能带连字符：后端 `normalize_provider`（`commands.rs:14340`）
    // 会 `to_lowercase()` 且把 `-` 换成 `_` 再落盘。用 "authoring-provider"
    // 保存出来是 `authoring_provider`，而画布节点仍引用原字面量 ⇒
    // 运行时报 "references an unconfigured provider"。
    // （我第一版就踩了这个，记在此以免下次重来。）
    private const string ProviderId = "authoring_provider";
    private const string ModelId = "authoring-model";

    /// <summary>
    /// 第二个模型，只用来当「不该被选中的那个」。
    /// 它排在 Provider 模型列表首位，因此是 `select_llm_model` 回落路径的结果；
    /// 任何断言若在预设/显式设置指向 `ModelId` 时看到了它，就说明配置没生效。
    /// </summary>
    private const string FallbackModelId = "authoring-fallback-model";
    private const string CanvasWorkflowId = "default";

    private readonly DirectoryInfo _temp =
        Directory.CreateTempSubdirectory("ariadne-canvas-authoring-");

    private static string? ResolveSidecar()
    {
        SidecarAppStateIsolation.RequireIsolatedAppState();

        // 保存 Provider 密钥前必须先解锁凭据存储，否则后端拒绝落盘
        // （`secrets.rs:584-588`：既无主密码也无明文许可 ⇒ Locked）。
        // 用主密码而非「允许明文」：前者走的是真实加密路径，
        // 与用户配好主密码后的生产形态一致。
        Environment.SetEnvironmentVariable(
            "ARIADNE_SECRET_MASTER_KEY", "canvas-authoring-master-key");

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
    /// 主线：搭两节点图 → 用节点详情面板改提示词与模型 → 跑 →
    /// **出站请求里必须是改后的提示词，且模型名是改后的那个**。
    ///
    /// 这是「改了属性到底生不生效」的唯一可信判据。
    /// 只看磁盘 config 会漏掉「读取时用了另一个字段名」这类缺陷；
    /// 只看运行成功会漏掉「用默认值跑通了」——两者都会让作者
    /// 以为自己调的参数在起作用。
    /// </summary>
    [Fact]
    public async Task EditedPromptAndModel_ActuallyReachTheOutboundRequest()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(EditedPromptAndModel_ActuallyReachTheOutboundRequest)))
        {
            return;
        }

        var model = FakeModelEndpoint.Start("好的。");
        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = await SeedProjectAsync(client, "属性生效", model.BaseUrl);

        // 搭一张单节点图（先用占位提示词，稍后用节点详情面板改掉）。
        await SaveCanvasAsync(client, "属性生效", new[]
        {
            LlmNode("node-写", "占位提示词，应当被改掉。"),
        });

        // 作者在节点详情面板里改提示词 + 换模型 + 设预算与超时。
        const string authoredPrompt = "请以第三人称写下开场：{章节}";
        await client.ApplyNodeDetailPatchAsync(CanvasWorkflowId, new NodeDetailPatch(
            NodeId: "node-写",
            PromptTemplate: authoredPrompt,
            InputAliases: new Dictionary<string, string>(),
            ToolEnabled: new Dictionary<string, bool>(),
            ApprovalPolicy: new Dictionary<string, string>(),
            ModelId: ModelId,
            BudgetUsd: 5.0,
            TimeoutMs: 60_000));

        // 先确认落盘了（这一层既有测试已覆盖，这里只作为前提校验：
        // 若这里就没写进去，下面的出站断言测的不是「生效」而是「没写对」）。
        Assert.Contains(
            authoredPrompt,
            await File.ReadAllTextAsync(CanvasPath(projectRoot)),
            StringComparison.Ordinal);

        await client.RunWorkflowAsync(CanvasWorkflowId);

        var request = await model.FirstRequestAsync();

        // 判据一：作者写的提示词真的到了模型那里。
        Assert.Contains(authoredPrompt, request, StringComparison.Ordinal);
        Assert.DoesNotContain("占位提示词", request, StringComparison.Ordinal);

        // 判据二：模型名是作者选的那个，而不是某个默认值。
        // 必须同时排除 `FallbackModelId`：只断言「含 ModelId」在单模型配置下恒真，
        // 有了第二个模型这条才真的在分辨「显式设置生效」与「回落恰好同名」。
        Assert.Contains(ModelId, request, StringComparison.Ordinal);
        Assert.DoesNotContain(FallbackModelId, request, StringComparison.Ordinal);
    }

    /// <summary>
    /// 运行参数（variables）必须被**声明校验**接住：声明过的放行、没声明的当场拒绝。
    ///
    /// ⚠️ **本用例刻意不断言「值被代入提示词」**，理由是我查证后发现
    /// 普通 `llm` 节点**按设计不做模板渲染**——
    /// `render_writing_node_prompt`（`integration.rs:1169`）在
    /// `writing_tools` 为 `None` 时原样返回，注释写明「普通 llm 节点没有
    /// agent 身份，也没有知识库，不能凭空给它装配上下文」。
    /// 我第一版把「`{{var.topic}}` 原样发给模型」当成缺陷写了断言，
    /// 那是误判：那条路径上的节点本来就不渲染。
    /// 变量代入的渲染契约已由 `core/tests/workflow_variable_contracts.rs`
    /// 在单元层覆盖（`{{var.名字}}` → 值，未知变量 fail-loud）。
    ///
    /// 于是这条改测**真正跨进程可验、且用户可感知**的那一半：
    /// 声明过的变量能让运行启动，没声明的被拒。后者是关键——
    /// 它防的是「变量随便传都收下、然后静默不生效」。
    ///
    /// 另两条前提也是我踩过的坑，记此避免重来：
    /// (1) 变量必须在 `start` 节点的 `config.variables` 里声明
    ///     （`runtime.rs:612` `collect_variable_decls`：只允许挂 start 节点，
    ///     否则同名变量的归属层无法确定）。
    /// (2) 变量名只能是 ASCII 字母/数字/下划线，不能以数字开头
    ///     （`workflow.rs:778` `validate_variable_name`）。中文名会被拒。
    /// </summary>
    [Fact]
    public async Task RunVariables_DeclaredOnesAreAccepted_UndeclaredOnesAreRejected()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(RunVariables_DeclaredOnesAreAccepted_UndeclaredOnesAreRejected)))
        {
            return;
        }

        var model = FakeModelEndpoint.Start("好的。");
        using var client = new JsonLineBackendClient(sidecar);
        await SeedProjectAsync(client, "参数校验", model.BaseUrl);

        var start = new CanvasNode("node-start", "start", null, new Dictionary<string, object?>
        {
            ["variables"] = new object[]
            {
                new { name = "topic", kind = "string", @default = "", required = false, hidden = false },
            },
        }, new CanvasPosition(0, 0));

        await SaveCanvasAsync(client, "参数校验", new[]
        {
            start,
            LlmNode("node-写", "写一段关于{{var.topic}}的开头。"),
        }, new[]
        {
            new CanvasEdge("e1", "node-start", "node-写", "exec_out", "exec_in", "control", null, null),
        });

        // 声明过的变量：运行必须能启动。
        var started = await client.RunWorkflowAsync(
            CanvasWorkflowId,
            startNodeId: null,
            variables: new Dictionary<string, object?> { ["topic"] = "雪夜归人" });
        Assert.False(
            string.IsNullOrWhiteSpace(started.RunId),
            "声明过的变量被拒了——执行页填好参数也跑不起来");

        // 没声明的变量：必须当场拒绝，而不是收下后静默不生效。
        var rejected = await Assert.ThrowsAsync<BackendException>(() =>
            client.RunWorkflowAsync(
                CanvasWorkflowId,
                startNodeId: null,
                variables: new Dictionary<string, object?> { ["nope"] = "随便" }));

        Assert.Contains("nope", rejected.Message, StringComparison.Ordinal);
    }

    /// <summary>
    /// 节点级预算必须在**发出请求之前**拦住超支，而不是发完再算。
    ///
    /// 判据取「模型端一次请求都没收到」。这是预算类缺陷唯一可信的判据：
    /// 若断言只看「运行失败」，一个「先花钱再报错」的实现照样通过——
    /// 而那正是预算功能要防的事（钱已经付了）。
    /// </summary>
    [Fact]
    public async Task ZeroNodeBudget_BlocksTheRunBeforeAnyOutboundCall()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(ZeroNodeBudget_BlocksTheRunBeforeAnyOutboundCall)))
        {
            return;
        }

        var model = FakeModelEndpoint.Start("不该被调用。");
        using var client = new JsonLineBackendClient(sidecar);
        await SeedProjectAsync(client, "预算拦截", model.BaseUrl);

        await SaveCanvasAsync(client, "预算拦截", new[]
        {
            LlmNode("node-写", "任意提示词。"),
        });

        // 把日预算设成一个极小的正值：0 在本项目里表示「不限」
        // （CLAUDE.md 记着 budget_usd 的 0 = 不设上限，这是刻意的语义），
        // 所以要用极小正值才能表达「几乎没有额度」。
        await client.UpdateBudgetConfigAsync(0.000001, null);

        try
        {
            await client.RunWorkflowAsync(CanvasWorkflowId);
        }
        catch (BackendException)
        {
            // 同步拒绝也是可接受的形态——判据在下面那条。
        }

        // 关键判据：模型端一次都没被打到。
        Assert.Equal(0, await model.CountRequestsWithinAsync(TimeSpan.FromSeconds(6)));
    }

    /// <summary>
    /// 节点类型预设（模型/超时/预算）必须成为**未单独设置**节点的实际取值。
    ///
    /// 预设是「一处改、全类型生效」的承诺。它的失效形态是
    /// 「预设存下来了，但运行时读的是内置默认值」——
    /// 既有 `Settings_NodePresets_RoundTripPerNodeTypeValues` 只验了往返落盘，
    /// 没验「运行时是否采用」。判据取出站请求里的模型名。
    ///
    /// ⚠️ 两处前提是查证后定下的，改之前先读：
    /// (1) **节点连 `provider_id` 都不能写**。`commands.rs:7654-7662` 的规则是
    ///     「任一节点级字段出现即进入显式覆盖层」——写了 `provider_id` 就会让
    ///     `preferred_model_id` 直接取 `None`（**刻意不再拼入预设的 model**，
    ///     以免混搭出属于别的 Provider 的模型），于是模型落到 `select_llm_model`
    ///     的回落值而不是预设值。第一版写了 `provider_id`，等于在测回落路径、
    ///     而它恰好和预设值相同 ⇒ 断言恒真。Provider 由项目默认补齐
    ///     （`SeedProjectAsync` 里 make_default_llm=true）。
    /// (2) 判据必须同时断言「**不是** `FallbackModelId`」。只断言「含 ModelId」
    ///     在单模型配置下永远成立，见 `SeedProjectAsync` 的注释。
    /// </summary>
    [Fact]
    public async Task NodeTypePreset_SuppliesTheModelForNodesWithoutAnExplicitOne()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(NodeTypePreset_SuppliesTheModelForNodesWithoutAnExplicitOne)))
        {
            return;
        }

        var model = FakeModelEndpoint.Start("好的。");
        using var client = new JsonLineBackendClient(sidecar);
        await SeedProjectAsync(client, "预设生效", model.BaseUrl);

        // 关键：节点 data 里**既不写 model_id 也不写 provider_id**，
        // 让模型只能来自节点类型预设（理由见方法注释第 (1) 条）。
        await SaveCanvasAsync(client, "预设生效", new[]
        {
            new CanvasNode("node-写", "llm", null, new Dictionary<string, object?>
            {
                ["prompt_template"] = "写点什么。",
            }, new CanvasPosition(40, 60)),
        });

        var presets = await client.GetNodePresetSettingsAsync();
        // 每个节点类型预设自带 model_id（出厂值是 gpt-4.1-mini），
        // 只改 DefaultModelId 会被「预设引用了未配置的模型」这条校验拒掉。
        // 必须把每一条都指到本测试真实配置过的模型上。
        await client.SaveNodePresetSettingsAsync(presets with
        {
            DefaultModelId = ModelId,
            DefaultProviderId = ProviderId,
            Presets = presets.Presets
                .Select(preset => preset with { ModelId = ModelId, ProviderId = ProviderId })
                .ToList(),
        });

        await client.RunWorkflowAsync(CanvasWorkflowId);

        var request = await model.FirstRequestAsync();
        Assert.Contains(ModelId, request, StringComparison.Ordinal);

        // 关键对照：不能是回落模型。少了这条，单模型配置下断言恒真。
        Assert.DoesNotContain(FallbackModelId, request, StringComparison.Ordinal);
    }

    // ════════════════════════════════════════════════════════
    // 脚手架
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 起一个真实项目 + 真实 Provider。
    ///
    /// **必须配两个模型**（`ModelId` 与 `FallbackModelId`）：只配一个时，
    /// 「预设生效」这类判据是恒真的——`select_llm_model`（`commands.rs:13090-13096`）
    /// 在找不到指定模型时会回落到「该 Provider 第一个 llm 能力的模型」，
    /// 唯一模型的情况下无论走哪条解析路径，出站请求里都是同一个名字，
    /// 断言分辨不出「预设真的被采用」与「预设被忽略、恰好回落到同一个」。
    /// 把 `FallbackModelId` 放在**列表首位**，让回落路径与预设路径指向不同的值。
    /// </summary>
    private async Task<string> SeedProjectAsync(
        IAriadneBackendClient client, string name, string modelBaseUrl)
    {
        var projectRoot = Path.Combine(_temp.FullName, name);
        await client.CreateProjectAsync(projectRoot, name);
        await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            ProviderId, "open_ai_compatible", "画布测试服务", true,
            modelBaseUrl,
            new[]
            {
                new ModelConfig(FallbackModelId, "llm", null, null, null),
                new ModelConfig(ModelId, "llm", null, null, null),
            },
            true, false, false, false));
        await client.SaveProviderKeyAsync(ProviderId, "sk-canvas-authoring");
        return projectRoot;
    }

    /// <summary>
    /// 保存画布。必须先读一次拿 `content_revision`——
    /// 新建项目自带默认画布，后端在 `expected_revision` 为 null 时会拒绝覆盖
    /// （CAS 保护，`commands.rs:5713`）。
    /// </summary>
    private static async Task SaveCanvasAsync(
        IAriadneBackendClient client,
        string name,
        IReadOnlyList<CanvasNode> nodes,
        IReadOnlyList<CanvasEdge>? edges = null)
    {
        var current = await client.LoadProjectCanvasAsync();
        await client.SaveProjectCanvasAsync(new WorkflowGraphData(
            CanvasWorkflowId, name, nodes, edges ?? Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: null,
            ExpectedRevision: current.ContentRevision));
    }

    private static CanvasNode LlmNode(string id, string prompt)
        => new(id, "llm", null, new Dictionary<string, object?>
        {
            ["provider_id"] = ProviderId,
            ["model_id"] = ModelId,
            ["prompt_template"] = prompt,
        }, new CanvasPosition(40, 60));

    private static string CanvasPath(string projectRoot)
        => Path.Combine(projectRoot, "workflows", "default.json");

    /// <summary>
    /// 真实 HTTP 模型服务端：捕获出站请求体，回一个 OpenAI 兼容响应。
    ///
    /// 与 `FrontendUserActionJourneyTests.SpawnLlm` 同源（裸 TcpListener），
    /// 但这里额外提供「数一数到底被打了几次」的能力——
    /// 预算那条用例需要断言**零次调用**，而「零次」只能靠等一段时间来确认。
    /// </summary>
    private sealed class FakeModelEndpoint
    {
        private readonly List<string> _requests = new();
        private readonly object _sync = new();
        private readonly TaskCompletionSource<string> _first =
            new(TaskCreationOptions.RunContinuationsAsynchronously);

        private FakeModelEndpoint(TcpListener listener, string reply)
        {
            BaseUrl = $"http://127.0.0.1:{((IPEndPoint)listener.LocalEndpoint).Port}";
            _ = Task.Run(() => ServeAsync(listener, reply));
        }

        public string BaseUrl { get; }

        public static FakeModelEndpoint Start(string reply)
        {
            var listener = new TcpListener(IPAddress.Loopback, 0);
            listener.Start();
            return new FakeModelEndpoint(listener, reply);
        }

        public async Task<string> FirstRequestAsync()
        {
            var completed = await Task.WhenAny(_first.Task, Task.Delay(TimeSpan.FromSeconds(60)));
            Assert.True(
                ReferenceEquals(completed, _first.Task),
                "60 秒内模型服务端没有收到任何请求——运行没有真的发出出站调用");
            return await _first.Task;
        }

        /// <summary>
        /// 等一段时间后返回收到的请求数。
        ///
        /// 「零次调用」无法瞬时判定，只能给足够窗口再看。窗口取 6 秒：
        /// 本机上一次真实运行发出请求约 1 秒内，6 秒足够区分
        /// 「被拦住了」与「只是慢」。
        /// </summary>
        public async Task<int> CountRequestsWithinAsync(TimeSpan window)
        {
            await Task.Delay(window);
            lock (_sync)
            {
                return _requests.Count;
            }
        }

        private async Task ServeAsync(TcpListener listener, string reply)
        {
            var body = JsonSerializer.Serialize(new
            {
                model = ModelId,
                choices = new[]
                {
                    new
                    {
                        message = new { content = reply, tool_calls = Array.Empty<object>() },
                        finish_reason = "stop",
                    },
                },
                usage = new { prompt_tokens = 12, completion_tokens = 6 },
            });

            try
            {
                while (true)
                {
                    using var socket = await listener.AcceptSocketAsync();
                    using var stream = new NetworkStream(socket, ownsSocket: false);
                    var request = await ReadHttpRequestAsync(stream);

                    lock (_sync)
                    {
                        _requests.Add(request);
                    }
                    _first.TrySetResult(request);

                    var payload = Encoding.UTF8.GetBytes(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
                        + $"Content-Length: {Encoding.UTF8.GetByteCount(body)}\r\n\r\n{body}");
                    await stream.WriteAsync(payload);
                    await stream.FlushAsync();
                }
            }
            catch
            {
                // listener 关闭或连接异常：断言由测试主体负责。
            }
        }

        /// <summary>
        /// 按 Content-Length 读满请求体。
        ///
        /// **必须按字节计**：提示词是中文，UTF-8 下一个汉字 3 字节，
        /// 用字符数比较会永远读不满而卡到超时
        /// （这个坑 `FrontendUserActionJourneyTests` 的同名函数已记过）。
        /// </summary>
        private static async Task<string> ReadHttpRequestAsync(NetworkStream stream)
        {
            var buffer = new byte[64 * 1024];
            var text = new StringBuilder();
            var headerEnd = -1;
            var total = 0;

            while (headerEnd < 0)
            {
                var read = await stream.ReadAsync(buffer).AsTask()
                    .WaitAsync(TimeSpan.FromSeconds(30));
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

            var bodySoFar = total - Encoding.UTF8.GetByteCount(whole[..(headerEnd + 4)]);
            while (bodySoFar < contentLength)
            {
                var read = await stream.ReadAsync(buffer).AsTask()
                    .WaitAsync(TimeSpan.FromSeconds(30));
                if (read <= 0)
                {
                    break;
                }
                bodySoFar += read;
                text.Append(Encoding.UTF8.GetString(buffer, 0, read));
            }

            return text.ToString();
        }
    }

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
}
