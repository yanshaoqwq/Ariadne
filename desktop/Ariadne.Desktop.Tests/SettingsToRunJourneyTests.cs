using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U175：**首次配置全流程**——填 Provider（baseurl / apikey / 模型）→ 过必经设置 →
/// 编辑画布 → **用刚填的那份设置真的跑起来**。
///
/// 这是用户装完应用后的第一条路，也是最长的一条：任何一环没落到实处，
/// 表现都是「设置看着填好了，运行却用不上」。
///
/// **与既有 `FrontendSettingsAndPerfJourneyTests` 的分工**：那份验单个设置分区
/// 能否往返落盘（workflow / git / presets / permissions）。本份验两件它没覆盖的事：
///   1. **Provider 那一簇**（baseurl + apikey + 模型目录）——它是第一必经之路，
///      而既有测试完全没碰；
///   2. **设置 → 运行的贯通**：判据取**真实出站 HTTP 请求**打到了
///      设置里填的那个 baseurl、带着填的那把 key、用的是选的那个模型。
///
/// 为什么必须自起 HTTP 端：这一簇的失效形态是「配置存对了、运行成功了、
/// 但请求打到别处 / 带着空 Bearer / 用的是默认模型」。
/// 磁盘断言和「运行返回 ok」都拦不住——只有把真实请求接住才行。
/// </summary>
[Collection("RealSidecar")]
public sealed class SettingsToRunJourneyTests : IDisposable
{
    // ⚠️ provider id 不能带连字符：后端 `normalize_provider`（`commands.rs:14340`）
    // 会 `to_lowercase()` 且把 `-` 换成 `_` 再落盘，画布节点引用原字面量就匹配不上。
    //
    // ⚠️⚠️ **每条用例必须用各自的 provider id**，不能共用一个常量。
    // 原因：Provider 凭据存在**应用级** `secrets.json`
    // （`commands.rs:5274` 的 `default_app_state_root().join("secrets.json")`），
    // 而整个测试进程的 app-state 被 `SidecarAppStateIsolation` 钉在同一个目录里
    // ⇒ 四条用例共用**同一份凭据库**。`RevokingTheKey_...` 撤销
    // 「那个」provider 的密钥时，会把并行跑的主线用例刚存好的密钥一起抹掉，
    // 主线随即在 `has_key` 断言上失败。
    //
    // 这不是产品缺陷（已用裸 IPC 复刻同一序列，`has_key` 正确返回 true），
    // 是**用例之间抢同一个凭据条目**。共享项目目录不够——凭据是跨项目的。
    private const string ModelId = "first-run-model";
    private const string ApiKey = "sk-first-run-secret-value";
    private const string CanvasWorkflowId = "default";

    private readonly DirectoryInfo _temp =
        Directory.CreateTempSubdirectory("ariadne-settings-to-run-");

    private static string? ResolveSidecar()
    {
        SidecarAppStateIsolation.RequireIsolatedAppState();

        // 保存密钥前必须先解锁凭据存储，否则后端拒绝落盘
        // （`secrets.rs:584-588`：既无主密码也无明文许可 ⇒ Locked）。
        // 这也是 U172-B 的成因：新项目默认就是 Locked。
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
    /// **主线**：新建项目 → 填 Provider（baseurl/key/模型）→ 过必经设置 →
    /// 搭画布 → 运行 → **请求真的打到填的 baseurl、带填的 key、用选的模型**。
    ///
    /// 每一步的判据都取「下一步能否看到上一步的结果」，最后一步取真实出站请求。
    /// 任何一环只在内存里生效，链条会当场断在那一步。
    /// </summary>
    [Fact]
    public async Task FillProviderThenEditCanvasThenRun_RequestUsesExactlyWhatWasConfigured()
    {
        // 本用例独占的 provider id：凭据库是应用级共享的，共用 id 会互相抹掉密钥（见类注释）。
        const string providerId = "first_run_main";

        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(FillProviderThenEditCanvasThenRun_RequestUsesExactlyWhatWasConfigured)))
        {
            return;
        }

        var model = FakeModelEndpoint.Start("夜色像一封没写完的信。");
        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "首次配置");

        // ── 1. 欢迎页「新建项目」
        await client.CreateProjectAsync(projectRoot, "首次配置");

        // ── 2. 设置页 Provider 分区：填 baseurl + 模型，保存
        var afterProvider = await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            ProviderId: providerId,
            ProviderType: "open_ai_compatible",
            DisplayName: "我的模型服务",
            Enabled: true,
            BaseUrl: model.BaseUrl,
            Models: new[] { new ModelConfig(ModelId, "llm", null, null, null) },
            MakeDefaultLlm: true,
            MakeDefaultEmbedding: false,
            MakeDefaultReranker: false,
            MakeDefaultSearch: false));

        // 判据：保存后立刻回读，baseurl 与模型必须在，且**此时还没有 key**。
        var entry = Assert.Single(
            afterProvider.Providers,
            candidate => candidate.Provider == providerId);
        Assert.Equal(model.BaseUrl, entry.BaseUrl);
        Assert.Contains(entry.Models, candidate => candidate.ModelId == ModelId);
        Assert.False(entry.HasKey, "还没填密钥，has_key 就已经是 true——状态是假的");

        // ── 3. 设置页「保存密钥」
        var afterKey = await client.SaveProviderKeyAsync(providerId, ApiKey);
        var keyed = Assert.Single(
            afterKey.Providers,
            candidate => candidate.Provider == providerId);
        Assert.True(keyed.HasKey, "填了密钥但 has_key 仍为 false——设置页会显示成未配置");

        // ── 4. 默认路由必须指向刚填的那个（MakeDefaultLlm: true 的承诺）
        Assert.Equal(providerId, afterKey.DefaultLlmProviderId);
        Assert.Equal(ModelId, afterKey.DefaultLlmModelId);

        // ── 5. 必经设置：Provider 配置必须真的落盘，不能只活在内存里。
        //
        // ⚠️ **落盘位置是 app-state，不是项目目录**（我先后错过两次，记此以免重来）：
        //   - `{app_state_root}/provider_catalog.json` ← Provider 配置真正在这里
        //   - `{项目}/.config/providers.yaml` ← 这份是**项目级**的，实测内容是
        //     `providers: []`，因为 Provider 是**应用级**配置（换项目也该还在）
        // 判据打在项目那份上会永远失败，且失败原因会被误读成「没落盘」。
        //
        // ⚠️ 也不要用「再起一个 sidecar 打开同一项目」来验持久化：
        // 会撞 tantivy 的索引写锁（`Failed to acquire Lockfile: LockBusy`）。
        // 两个 sidecar 同时持有同一项目的 IndexWriter 本来就不允许，
        // 那是**正确行为**；真实应用同一时刻只有一个 sidecar。
        var appStateRoot = Path.Combine(SidecarAppStateIsolation.Root, "Ariadne");
        var catalogPath = Path.Combine(appStateRoot, "provider_catalog.json");
        Assert.True(
            File.Exists(catalogPath),
            $"Provider 配置没有落盘：{catalogPath} 不存在 ⇒ 下次启动要重填");

        var catalog = await File.ReadAllTextAsync(catalogPath);
        Assert.Contains(model.BaseUrl, catalog, StringComparison.Ordinal);
        Assert.Contains(ModelId, catalog, StringComparison.Ordinal);

        // ⚠️ 顺带钉住一条安全性质：**密钥不得与配置同文件明文落盘**。
        // 它走 SecretStore（另存 `secrets.json`，本测试用
        // ARIADNE_SECRET_MASTER_KEY 加密）。若哪天有人为了省事把 api_key
        // 一起塞进 provider_catalog.json，这条会当场红——
        // 那份文件不加密，等于把所有人的 API Key 摊在磁盘上。
        Assert.DoesNotContain(ApiKey, catalog, StringComparison.Ordinal);

        // ── 6. 画布页：搭一个用这个 Provider 的节点并保存
        var current = await client.LoadProjectCanvasAsync();
        await client.SaveProjectCanvasAsync(new WorkflowGraphData(
            CanvasWorkflowId, "首次配置", new[]
            {
                new CanvasNode("node-写", "llm", null, new Dictionary<string, object?>
                {
                    ["provider_id"] = providerId,
                    ["model_id"] = ModelId,
                    ["prompt_template"] = "写一句开场。",
                }, new CanvasPosition(40, 60)),
            },
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: null,
            ExpectedRevision: current.ContentRevision));

        // ── 7. 运行
        var started = await client.RunWorkflowAsync(CanvasWorkflowId);
        Assert.False(string.IsNullOrWhiteSpace(started.RunId), "运行必须返回 run_id");

        // ── 8. **最终判据：出站请求用的正是设置里填的三样东西。**
        var request = await model.FirstRequestAsync();

        // 8a. 打到了填的 baseurl。能收到请求本身就证明了这一点——
        //     这个端口是本用例独占的，请求打错地方就永远等不到（60 秒后超时报错）。
        // 8b. 带着填的那把 key。
        Assert.Contains($"Bearer {ApiKey}", request, StringComparison.Ordinal);
        // 8c. 用的是选的那个模型。
        Assert.Contains(ModelId, request, StringComparison.Ordinal);
        // 8d. 提示词是画布上那个节点的。
        Assert.Contains("写一句开场", request, StringComparison.Ordinal);
    }

    /// <summary>
    /// **节点上选的模型必须压过项目默认路由。**
    ///
    /// 为什么单独立一条而不是塞进主线：变异测试暴露了主线 8c 的一个盲区——
    /// 把画布节点的 `model_id` 摘掉，主线**照样全绿**，因为
    /// `MakeDefaultLlm: true` 让默认路由供出同一个模型名，
    /// 两条来源撞在同一个值上，断言分不出是哪条生效的。
    /// （这正是 CLAUDE.md 说的弱判据：断言被一个无关原因满足了。）
    ///
    /// 这条用**两个不同的模型名**把它们分开：默认路由指 A，节点上选 B，
    /// 出站必须是 B。这样「节点选择被忽略、一律用默认」这个真实缺陷才拦得住——
    /// 那种缺陷下作者在画布上换模型不会有任何效果，而界面显示换成功了。
    /// </summary>
    [Fact]
    public async Task NodeLevelModelChoice_OverridesTheProjectDefaultRoute()
    {
        // 本用例独占的 provider id：凭据库是应用级共享的，共用 id 会互相抹掉密钥（见类注释）。
        const string providerId = "first_run_override";

        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(NodeLevelModelChoice_OverridesTheProjectDefaultRoute)))
        {
            return;
        }

        const string defaultModel = "route-default-model";
        const string chosenModel = "node-chosen-model";

        var model = FakeModelEndpoint.Start("好的。");
        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "节点覆盖默认");
        await client.CreateProjectAsync(projectRoot, "节点覆盖默认");

        // 同一个 Provider 上挂两个模型；把 **defaultModel** 设成默认 LLM 路由。
        await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            providerId, "open_ai_compatible", "服务", true, model.BaseUrl,
            new[]
            {
                new ModelConfig(defaultModel, "llm", null, null, null),
                new ModelConfig(chosenModel, "llm", null, null, null),
            },
            MakeDefaultLlm: true,
            MakeDefaultEmbedding: false,
            MakeDefaultReranker: false,
            MakeDefaultSearch: false));
        await client.SaveProviderKeyAsync(providerId, ApiKey);

        // 画布节点刻意选**另一个**模型。
        var current = await client.LoadProjectCanvasAsync();
        await client.SaveProjectCanvasAsync(new WorkflowGraphData(
            CanvasWorkflowId, "节点覆盖默认", new[]
            {
                new CanvasNode("node-写", "llm", null, new Dictionary<string, object?>
                {
                    ["provider_id"] = providerId,
                    ["model_id"] = chosenModel,
                    ["prompt_template"] = "写一句。",
                }, new CanvasPosition(0, 0)),
            },
            Array.Empty<CanvasEdge>(), new Dictionary<string, object?>(),
            ContentRevision: null, ExpectedRevision: current.ContentRevision));

        await client.RunWorkflowAsync(CanvasWorkflowId);
        var request = await model.FirstRequestAsync();

        // 关键判据：出站是节点选的那个，**且不是**默认路由那个。
        // 两条一起才有鉴别力——只断言前者时，若实现把两个模型名都塞进请求也会通过。
        Assert.Contains($"\"model\":\"{chosenModel}\"", request, StringComparison.Ordinal);
        Assert.DoesNotContain($"\"model\":\"{defaultModel}\"", request, StringComparison.Ordinal);
    }

    /// <summary>
    /// **API key 绝不能从任何读取接口回流。**
    ///
    /// 判据取「把整个 provider 配置序列化成 JSON，里面不得出现密钥明文」。
    /// 这比逐字段断言强：新增字段时不需要回来补断言，
    /// 而漏一个字段就是一次真实的凭据泄露。
    ///
    /// 后端只暴露 `has_key: bool`（`commands.rs:9170-9182`），本用例把它钉住。
    /// </summary>
    [Fact]
    public async Task ApiKey_IsNeverEchoedBackByAnyReadPath()
    {
        // 本用例独占的 provider id：凭据库是应用级共享的，共用 id 会互相抹掉密钥（见类注释）。
        const string providerId = "first_run_echo";

        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(ApiKey_IsNeverEchoedBackByAnyReadPath)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "密钥不回流");
        await client.CreateProjectAsync(projectRoot, "密钥不回流");

        await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            providerId, "open_ai_compatible", "服务", true, "http://127.0.0.1:1",
            new[] { new ModelConfig(ModelId, "llm", null, null, null) },
            true, false, false, false));

        var afterSave = await client.SaveProviderKeyAsync(providerId, ApiKey);
        var afterRead = await client.GetProviderConfigAsync();

        foreach (var (label, status) in new[]
        {
            ("保存密钥的返回值", afterSave),
            ("重新读取配置", afterRead),
        })
        {
            var json = JsonSerializer.Serialize(status);
            Assert.DoesNotContain(ApiKey, json, StringComparison.Ordinal);
            Assert.True(
                status.Providers.Single(p => p.Provider == providerId).HasKey,
                $"{label}：has_key 应为 true（能证明密钥确实存进去了，"
                + "否则「不含明文」可能只是因为压根没存）");
        }
    }

    /// <summary>
    /// 撤销密钥后，后续出站请求**不得再带那把已撤销的凭据**。
    ///
    /// ⚠️ **判据不能是「一次出站都没有」**（我第一版就写错了）。实测撤销后运行
    /// 仍会发出请求，且请求里**完全没有 `authorization` 头**——
    /// 查证后确认这是**设计而非缺陷**：无密钥的本地模型服务（Ollama 之类）
    /// 是产品要支持的场景，既有用例
    /// `FrontendWritingChainJourneyTests.WritingChain_MissingApiKey_DoesNotSendAnEmptyBearerToken`
    /// 明文写着「无密钥的本地服务能跑通，这是产品要支持的场景」。
    /// 所以「撤销后还能跑」正确，把它断言成「不能跑」会要求砍掉那个能力。
    ///
    /// 真正该守的性质是**凭据不再外泄**：撤销之后那把 key 不能再出现在任何出站请求里。
    /// 这才是撤销的语义，也是密钥类缺陷真正的危险面
    /// （「界面显示已撤销，实际仍在磁盘上被使用」）。
    ///
    /// 判据两层：`has_key` 翻为 false（状态位），**且**出站请求里不含那把 key（实效）。
    /// 只验前者测不出「状态翻了但仍在用旧凭据」。
    /// </summary>
    [Fact]
    public async Task RevokingTheKey_StopsSendingThatCredential()
    {
        // 本用例独占的 provider id：凭据库是应用级共享的，共用 id 会互相抹掉密钥（见类注释）。
        const string providerId = "first_run_revoke";

        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(RevokingTheKey_StopsSendingThatCredential)))
        {
            return;
        }

        var model = FakeModelEndpoint.Start("撤销后仍可跑，但不该带旧凭据。");
        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "撤销密钥");
        await client.CreateProjectAsync(projectRoot, "撤销密钥");

        await client.SaveProviderSettingsAsync(new ProviderSettingsUpdate(
            providerId, "open_ai_compatible", "服务", true, model.BaseUrl,
            new[] { new ModelConfig(ModelId, "llm", null, null, null) },
            true, false, false, false));
        await client.SaveProviderKeyAsync(providerId, ApiKey);

        var current = await client.LoadProjectCanvasAsync();
        await client.SaveProjectCanvasAsync(new WorkflowGraphData(
            CanvasWorkflowId, "撤销密钥", new[]
            {
                new CanvasNode("node-写", "llm", null, new Dictionary<string, object?>
                {
                    ["provider_id"] = providerId,
                    ["model_id"] = ModelId,
                    ["prompt_template"] = "写一句。",
                }, new CanvasPosition(0, 0)),
            },
            Array.Empty<CanvasEdge>(), new Dictionary<string, object?>(),
            ContentRevision: null, ExpectedRevision: current.ContentRevision));

        // 判据一（状态位）：撤销后 has_key 必须翻为 false。
        var afterRevoke = await client.RevokeProviderKeyAsync(providerId);
        Assert.False(
            afterRevoke.Providers.Single(p => p.Provider == providerId).HasKey,
            "撤销后 has_key 仍为 true——设置页会显示成仍已配置");

        try
        {
            await client.RunWorkflowAsync(CanvasWorkflowId);
        }
        catch (BackendException)
        {
            // 拒绝运行也是可接受的形态；下面的判据对两种形态都成立。
        }

        // 判据二（实效）：若真的发出了请求，里面绝不能带那把已撤销的 key。
        // 没发出请求同样通过——本用例守的是「不外泄」，不是「必须跑」。
        foreach (var request in await model.RequestsWithinAsync(TimeSpan.FromSeconds(6)))
        {
            Assert.DoesNotContain(ApiKey, request, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// 「测试连接」按钮（`test_provider_draft`）必须用**未保存的表单**出站，
    /// 且**不得把草稿写进配置**。
    ///
    /// 这是设置页的关键体验：填错了要能在保存前发现。
    /// 后端注释（`commands.rs:3493-3496`）写明「API key 只在本次请求中保留在内存，
    /// 不获取写锁、不保存配置、不触碰 SecretStore」。本用例把这三条钉住。
    ///
    /// 判据两层：出站请求带的是**草稿里那把 key**；调用后配置里**没有**这个 provider。
    /// 只验前者会漏掉「顺手存了草稿」——那会让用户在点保存前就已经改了配置。
    /// </summary>
    [Fact]
    public async Task TestConnection_UsesDraftWithoutPersistingIt()
    {
        // 这条用例不需要独占 provider id：它测的是「草稿不落盘」，
        // 用的是就地字面量 "draft_provider"，而且断言之一正是
        // 「调用后配置里没有这个 provider」——它从不写凭据库，因此不与他人相争。
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(TestConnection_UsesDraftWithoutPersistingIt)))
        {
            return;
        }

        var model = FakeModelEndpoint.Start("ok", modelsCatalog: true);
        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "测试连接");
        await client.CreateProjectAsync(projectRoot, "测试连接");

        const string draftKey = "sk-draft-only-never-saved";
        try
        {
            await client.TestProviderDraftAsync(new ProviderDraftProbe(
                new ProviderSettingsUpdate(
                    "draft_provider", "open_ai_compatible", "草稿服务", true,
                    model.BaseUrl,
                    new[] { new ModelConfig(ModelId, "llm", null, null, null) },
                    false, false, false, false),
                draftKey));
        }
        catch (BackendException)
        {
            // 假服务端的模型目录格式可能不被接受；本用例关心的是
            // 「出站带了草稿 key」与「没有落盘」，不关心探测是否成功。
        }

        // 判据一：出站请求带的是草稿里那把 key。
        var request = await model.FirstRequestAsync();
        Assert.Contains(draftKey, request, StringComparison.Ordinal);

        // 判据二：草稿**没有**被写进配置。
        var config = await client.GetProviderConfigAsync();
        Assert.DoesNotContain(config.Providers, p => p.Provider == "draft_provider");
    }

    // ════════════════════════════════════════════════════════
    // 脚手架
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 真实 HTTP 模型服务端：捕获出站请求体，回一个 OpenAI 兼容响应。
    /// 与 `CanvasAuthoringJourneyTests.FakeModelEndpoint` 同源。
    /// </summary>
    private sealed class FakeModelEndpoint
    {
        private readonly List<string> _requests = new();
        private readonly object _sync = new();
        private readonly TaskCompletionSource<string> _first =
            new(TaskCreationOptions.RunContinuationsAsynchronously);

        private FakeModelEndpoint(TcpListener listener, string reply, bool modelsCatalog)
        {
            BaseUrl = $"http://127.0.0.1:{((IPEndPoint)listener.LocalEndpoint).Port}";
            _ = Task.Run(() => ServeAsync(listener, reply, modelsCatalog));
        }

        public string BaseUrl { get; }

        public static FakeModelEndpoint Start(string reply, bool modelsCatalog = false)
        {
            var listener = new TcpListener(IPAddress.Loopback, 0);
            listener.Start();
            return new FakeModelEndpoint(listener, reply, modelsCatalog);
        }

        public async Task<string> FirstRequestAsync()
        {
            var done = await Task.WhenAny(_first.Task, Task.Delay(TimeSpan.FromSeconds(60)));
            Assert.True(
                ReferenceEquals(done, _first.Task),
                "60 秒内模型服务端没有收到任何请求——"
                + "要么请求打到了别的地址，要么运行压根没发出出站调用");
            return await _first.Task;
        }

        /// <summary>
        /// 等一段时间后返回请求数。「零次」无法瞬时判定，只能给足窗口再看；
        /// 本机一次真实运行发出请求约 1 秒内，6 秒足以区分「被拦住」与「只是慢」。
        /// </summary>
        /// <summary>
        /// 等一段时间后返回收到的全部请求。
        ///
        /// 「零次 / 只有一次」都无法瞬时判定，只能给足窗口再看。
        /// 窗口取 6 秒：本机一次真实运行发出请求约 1 秒内，
        /// 6 秒足以区分「被拦住了」与「只是慢」。
        /// </summary>
        public async Task<IReadOnlyList<string>> RequestsWithinAsync(TimeSpan window)
        {
            await Task.Delay(window);
            lock (_sync)
            {
                return _requests.ToList();
            }
        }

        public async Task<int> CountRequestsWithinAsync(TimeSpan window)
        {
            await Task.Delay(window);
            lock (_sync)
            {
                return _requests.Count;
            }
        }

        private async Task ServeAsync(TcpListener listener, string reply, bool modelsCatalog)
        {
            var chat = JsonSerializer.Serialize(new
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
            var models = JsonSerializer.Serialize(new
            {
                data = new[] { new { id = ModelId, @object = "model" } },
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

                    // 「测试连接」走的是模型目录端点，聊天走 chat/completions。
                    var body = modelsCatalog && request.Contains("/models", StringComparison.Ordinal)
                        ? models
                        : chat;
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
        /// 按 Content-Length 读满请求体。**必须按字节计**：提示词是中文，
        /// UTF-8 下一个汉字 3 字节，用字符数比较会永远读不满而卡到超时。
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
