using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U169：模板市场「搜索 → 看详情 → 安装 → 装完能用」全流程。
///
/// **自带真实 HTTP 模板仓库**（`TcpListener` 起在 loopback 上），
/// 后端通过真实 `reqwest` 出站抓取，再经真实 sidecar 落盘。
/// 全链路无 mock：唯一「假」的是仓库里的模板内容本身。
///
/// 为什么必须自写 HTTP 端：模板安装的失效形态是
/// 「HTTP 拿到了 → 落盘了 → 但画布里没有」——三段之间各有一次转换
/// （manifest → workflows/ 文件 → 合并进项目画布）。
/// 只有真出站 + 真落盘 + 再从画布读回，才能确认三段都通。
///
/// ⚠️ 后端默认**拒绝**指向本机地址的模板仓库
/// （`service.rs:3215` 起，防 SSRF）。测试必须显式设
/// `ARIADNE_ALLOW_LOCAL_TEMPLATE_REPOSITORY` 才能用 loopback——
/// 这个开关是**给测试用的**，不是把防线关掉：它只放宽 host 检查。
/// </summary>
[Collection("RealSidecar")]
public sealed class TemplateInstallJourneyTests
{
    /// <summary>协议由 `frontend/service.rs:2706/2720/2733` 定：三个端点。</summary>
    private const string TemplateId = "journey-template";

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
    /// 全流程主线：搜索 → 详情 → 安装 → **装完的工作流出现在项目画布里**。
    ///
    /// 最后一步是关键判据。前三步都只证明「HTTP 通了」，
    /// 而用户要的是「装完能在画布上跑」——`install_template_for_active_project`
    /// （`commands.rs:4923`）在落盘后还要 merge 进项目画布，
    /// 那一步失败时前三步照样成功。
    /// </summary>
    [Fact]
    public async Task SearchThenDetailThenInstall_TemplateEndsUpOnTheProjectCanvas()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(SearchThenDetailThenInstall_TemplateEndsUpOnTheProjectCanvas)))
        {
            return;
        }

        using var repo = TemplateRepositoryStub.Start();
        var temp = Directory.CreateTempSubdirectory("ariadne-template-journey-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Template Journey");

            // ── 1. 搜索：必须真的打到我们的 HTTP 端
            var results = await client.SearchTemplatesAsync(repo.BaseUrl, "", Array.Empty<string>());
            Assert.Contains(results, summary => summary.Id == TemplateId);

            // ── 2. 详情
            var detail = await client.GetTemplateDetailAsync(repo.BaseUrl, TemplateId);
            Assert.Equal(TemplateId, detail.Id);

            // ── 3. 安装
            var report = await client.InstallTemplateAsync(repo.BaseUrl, TemplateId, projectRoot);
            Assert.NotNull(report);

            // manifest 必须真的落在项目内的 workflows 目录下，而不是临时目录。
            Assert.True(
                File.Exists(report.ManifestPath),
                $"安装回执给了 manifest_path 但文件不存在：{report.ManifestPath}");
            Assert.StartsWith(projectRoot, report.ManifestPath, StringComparison.Ordinal);

            // ── 4. 关键判据：装完的节点必须出现在**项目画布**上。
            // 前三步全绿而这一步红 = HTTP 与落盘都对、合并进画布那段断了。
            //
            // ⚠️ 判据取**节点集合**而不是「序列化后的 JSON 里含某个子串」
            // （第一版就是后者，属于弱判据）：边的 source/target 里也写着节点 id，
            // 所以「只并入了边、节点一个都没进来」这种半合并状态照样能让子串命中。
            // 合并会给 id 加命名空间前缀（`project_canvas.rs:171` 的 `unique_id`
            // 拼成 `{namespace}--{原 id}`），因此按后缀匹配而非全等。
            var canvas = await client.LoadProjectCanvasAsync();
            var installedNodes = canvas.Nodes
                .Where(node => node.Id.EndsWith("journey-writer", StringComparison.Ordinal))
                .ToList();
            Assert.True(
                installedNodes.Count == 1,
                "项目画布里的 journey-writer 节点数应为 1，实际 "
                + $"{installedNodes.Count}。画布现有节点：["
                + string.Join(", ", canvas.Nodes.Select(node => node.Id))
                + "]。装完的模板没合并进画布 ⇒ 用户在画布上找不到刚装的模板。");

            // 节点的类型也必须跟着过来——只有 id 而 type 丢了，画布上是个渲染不出的空壳。
            Assert.Equal("writer", installedNodes[0].Type);

            // 边也必须一起并进来，且两端都指向真实存在的节点（不能有悬空引用）。
            var installedEdge = Assert.Single(
                canvas.Edges,
                edge => edge.Id.EndsWith("journey-plan-to-write", StringComparison.Ordinal));
            var nodeIds = canvas.Nodes.Select(node => node.Id).ToHashSet(StringComparer.Ordinal);
            Assert.Contains(installedEdge.Source, nodeIds);
            Assert.Contains(installedEdge.Target, nodeIds);

            // ── 5. 装完的图必须能通过后端自己的校验（否则装了个跑不起来的图）。
            //
            // ⚠️ 这里**不能**用 `ListWorkflowGraphsAsync()` + `Assert.NotEmpty`
            // （第一版如此，是恒真断言）：`list_workflow_graphs_impl`
            // （`commands.rs:5721`）无条件返回**恰好一个** WorkflowSummary，
            // 它描述的是项目画布本身、与装了什么模板无关，永远非空。
            // 改成把画布回喂给后端的拓扑/契约校验，那才是「装进来的图跑得起来」。
            await client.ValidateWorkflowGraphAsync(canvas);

            // 顺带钉住摘要的节点计数与画布一致：这两个数字来自不同代码路径，
            // 对不上说明列表页显示的规模与画布真实内容脱节。
            var summary = Assert.Single(await client.ListWorkflowGraphsAsync());
            Assert.Equal(canvas.Nodes.Count, summary.NodeCount);
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// 仓库返回 5xx 时，安装必须**明确失败**且不留半装状态。
    ///
    /// 判据取「抛错 + 画布未被改动 + workflows/ 下没有半个 manifest」三条。
    ///
    /// ⚠️ 关于原注释里「下载与画布合并是两步、中途失败会留下半装」那个说法：
    /// 我查证后确认**这条用例覆盖不到那个场景**。5xx 挂在
    /// `template_client(...).download(&id)`（`commands.rs:4931`）这一步，
    /// 而落盘发生在它之后的 `install_workflow_template_manifest`
    /// （`frontend/service.rs:2908-2910`）。也就是说下载失败时后端**根本还没开始写**，
    /// 「画布未变」在这条路径上近乎恒真——它是个弱判据，不是护栏。
    /// 保留它的价值只剩「fail loud」那一半（变异验证过：把仓库换成 200 后
    /// `ThrowsAsync` 确实红）。
    /// 补上的 manifest 目录断言把「没写」这件事也钉住：真实的半装形态是
    /// manifest 已 `atomic_write` 落盘、后续拓扑校验或画布合并失败，
    /// 那时 workflows/{id}/ 会留下一个用户在画布上看不到、却真实存在的目录。
    /// </summary>
    [Fact]
    public async Task RepositoryFailure_FailsLoudlyAndLeavesCanvasUnchanged()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(RepositoryFailure_FailsLoudlyAndLeavesCanvasUnchanged)))
        {
            return;
        }

        using var repo = TemplateRepositoryStub.Start(failWithStatus: 500);
        var temp = Directory.CreateTempSubdirectory("ariadne-template-fail-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Template Fail");

            var before = JsonSerializer.Serialize(await client.LoadProjectCanvasAsync());

            await Assert.ThrowsAsync<BackendException>(() =>
                client.InstallTemplateAsync(repo.BaseUrl, TemplateId, projectRoot));

            // 画布必须逐字未变——安装失败不能留下半个模板。
            var after = JsonSerializer.Serialize(await client.LoadProjectCanvasAsync());
            Assert.Equal(before, after);

            // workflows/{模板 id}/ 也不能存在：留下它就是「用户在画布上看不到、
            // 但磁盘上真有一份」的半装状态，下次安装还会撞上它。
            var installedDir = Path.Combine(projectRoot, "workflows", TemplateId);
            Assert.False(
                Directory.Exists(installedDir),
                $"安装失败却留下了 {installedDir}：半装状态，用户在画布上看不到它");
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// 安装到「与当前项目不符」的 expectedProjectRoot 必须被拒。
    ///
    /// 这是 `ensure_active_project_identity`（`commands.rs:4930`）的护栏：
    /// 下载是慢操作，期间用户可能切了项目；把迟到的下载写回**已经离开的项目**
    /// 会污染另一个作品。判据取「拒绝」+「两个项目的画布都没被改」。
    ///
    /// ⚠️ 第一版只查了 active 的画布，而**受害者是 other**（被越权写入的那个），
    /// 所以真正该钉住的那一边反倒漏了。现在两边都查。
    /// other 一侧的判据取「`workflows/` 下的文件清单不变」：
    /// 新建项目只会**创建空的 `workflows/` 目录**，`default.json` 是首次保存时
    /// 才惰性写出的（实测 `create_project` 的 created_dirs 里有 workflows、
    /// 但目录里一个文件都没有），所以不能去读它的 default.json——
    /// 我第一版就这么写，测试因 FileNotFound 而红，那是**断言错**不是产品缺陷。
    /// 比清单而不是比某个文件的内容，越权写入无论落成哪个文件都跑不掉。
    /// </summary>
    [Fact]
    public async Task InstallingIntoAMismatchedProjectRoot_IsRejected()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(InstallingIntoAMismatchedProjectRoot_IsRejected)))
        {
            return;
        }

        using var repo = TemplateRepositoryStub.Start();
        var temp = Directory.CreateTempSubdirectory("ariadne-template-mismatch-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var active = Path.Combine(temp.FullName, "active");
            var other = Path.Combine(temp.FullName, "other");
            await client.CreateProjectAsync(other, "Other");
            await client.CreateProjectAsync(active, "Active");

            var before = JsonSerializer.Serialize(await client.LoadProjectCanvasAsync());
            var otherWorkflows = Path.Combine(other, "workflows");
            var otherFilesBefore = SnapshotWorkflowsDir(otherWorkflows);

            // 当前活动项目是 active，却声称要装进 other。
            await Assert.ThrowsAsync<BackendException>(() =>
                client.InstallTemplateAsync(repo.BaseUrl, TemplateId, other));

            Assert.Equal(before, JsonSerializer.Serialize(await client.LoadProjectCanvasAsync()));

            // 受害项目 other 的 workflows/ 必须一个字节都没多——这才是这条护栏保护的对象。
            Assert.Equal(otherFilesBefore, SnapshotWorkflowsDir(otherWorkflows));
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// 取某个项目 `workflows/` 目录下的「相对路径 → 内容」快照，用于比对是否被越权写入。
    /// 目录不存在时返回空表（新建项目可能还没有任何工作流文件）。
    /// </summary>
    private static SortedDictionary<string, string> SnapshotWorkflowsDir(string workflowsRoot)
    {
        var snapshot = new SortedDictionary<string, string>(StringComparer.Ordinal);
        if (!Directory.Exists(workflowsRoot))
        {
            return snapshot;
        }

        foreach (var file in Directory.EnumerateFiles(
            workflowsRoot, "*", SearchOption.AllDirectories))
        {
            snapshot[Path.GetRelativePath(workflowsRoot, file)] = File.ReadAllText(file);
        }
        return snapshot;
    }

    /// <summary>
    /// 模板仓库地址设置必须往返落盘（这是模板页唯一的配置项）。
    ///
    /// 判据取「换一个客户端进程再读回」——同进程读回可能只是读到内存缓存，
    /// 那测不出「设置没落盘」这种最常见的形态。
    /// </summary>
    [Fact]
    public async Task TemplateRepositorySetting_PersistsAcrossClientProcesses()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(TemplateRepositorySetting_PersistsAcrossClientProcesses)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-template-setting-");
        try
        {
            var projectRoot = Path.Combine(temp.FullName, "novel");
            const string custom = "https://templates.example.com/v9";

            using (var first = new JsonLineBackendClient(sidecar))
            {
                await first.CreateProjectAsync(projectRoot, "Template Setting");
                await first.SaveTemplateRepositorySettingsAsync(
                    new TemplateRepositorySettings(custom));
            }

            // 新进程重新打开同一项目：设置必须还在。
            using var second = new JsonLineBackendClient(sidecar);
            await second.OpenProjectAsync(projectRoot);
            var reread = await second.GetTemplateRepositorySettingsAsync();

            Assert.Equal(custom, reread.BaseUrl);
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    // ════════════════════════════════════════════════════════
    // 真实 HTTP 模板仓库
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 在 loopback 上起一个**真实**的模板仓库，实现后端约定的三个端点：
    /// `GET /templates/search`、`GET /templates/{id}`、`GET /templates/{id}/download`。
    ///
    /// 刻意不用 `HttpListener`：它在部分 Linux 环境需要额外权限，
    /// 而本仓库既有测试（`FrontendUserActionJourneyTests.SpawnLlm`）已经确立了
    /// 裸 `TcpListener` 手写 HTTP 的做法，沿用同一套更稳。
    /// </summary>
    private sealed class TemplateRepositoryStub : IDisposable
    {
        private readonly TcpListener _listener;
        private readonly CancellationTokenSource _shutdown = new();
        private readonly Task _loop;

        private TemplateRepositoryStub(TcpListener listener, int? failWithStatus)
        {
            // 后端默认拒绝本机地址的模板仓库（防 SSRF，`service.rs:3215`）。
            // sidecar 以 UseShellExecute=false 启动，继承本进程环境，
            // 所以在这里设变量即可让它放宽 host 检查——
            // 只放宽 host，scheme/userinfo 那几道校验仍然生效。
            Environment.SetEnvironmentVariable("ARIADNE_ALLOW_LOCAL_TEMPLATE_REPOSITORY", "1");
            _listener = listener;
            BaseUrl = $"http://127.0.0.1:{((IPEndPoint)listener.LocalEndpoint).Port}";
            _loop = Task.Run(() => ServeAsync(failWithStatus, _shutdown.Token));
        }

        public string BaseUrl { get; }

        public static TemplateRepositoryStub Start(int? failWithStatus = null)
        {
            var listener = new TcpListener(IPAddress.Loopback, 0);
            listener.Start();
            return new TemplateRepositoryStub(listener, failWithStatus);
        }

        private async Task ServeAsync(int? failWithStatus, CancellationToken cancellationToken)
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                Socket socket;
                try
                {
                    socket = await _listener.AcceptSocketAsync(cancellationToken);
                }
                catch (OperationCanceledException)
                {
                    return;
                }
                catch (SocketException)
                {
                    return;
                }
                catch (ObjectDisposedException)
                {
                    return;
                }

                // 每个连接独立处理：后端的三个请求可能并发或复用不同连接，
                // 串行 accept 会让第二个请求等到第一个彻底结束。
                _ = Task.Run(async () =>
                {
                    try
                    {
                        using (socket)
                        using (var stream = new NetworkStream(socket, ownsSocket: false))
                        {
                            var request = await ReadRequestLineAsync(stream);
                            var body = failWithStatus is null
                                ? RouteToBody(request)
                                : null;

                            var payload = body is null
                                ? BuildResponse(failWithStatus ?? 404, "{\"error\":\"stub\"}")
                                : BuildResponse(200, body);

                            await stream.WriteAsync(payload, cancellationToken);
                            await stream.FlushAsync(cancellationToken);
                        }
                    }
                    catch
                    {
                        // 连接层异常不该让整个 stub 停摆——
                        // 断言由测试主体负责，这里吞掉只影响单个连接。
                    }
                }, cancellationToken);
            }
        }

        /// <summary>按请求行路由。返回 null 表示 404。</summary>
        private static string? RouteToBody(string requestLine)
        {
            if (requestLine.Contains("/templates/search", StringComparison.Ordinal))
            {
                return JsonSerializer.Serialize(new[]
                {
                    new
                    {
                        id = TemplateId,
                        name = "旅程模板",
                        tags = new[] { "test" },
                        requires_permissions = false,
                    },
                });
            }

            if (requestLine.Contains($"/templates/{TemplateId}/download", StringComparison.Ordinal))
            {
                return JsonSerializer.Serialize(Manifest());
            }

            if (requestLine.Contains($"/templates/{TemplateId}", StringComparison.Ordinal))
            {
                return JsonSerializer.Serialize(new
                {
                    id = TemplateId,
                    name = "旅程模板",
                    version = "1.0.0",
                    manifest = Manifest(),
                    requires_permissions = false,
                });
            }

            return null;
        }

        /// <summary>
        /// 与 `core/resources/template_repository/v1/novel_starter.json` 同构的 manifest。
        ///
        /// 节点 id 用 `journey-*` 前缀：主用例靠它在项目画布里认出
        /// 「这个模板真的被合并进来了」。改前缀要同步改那条断言。
        /// </summary>
        private static object Manifest() => new
        {
            workflow_id = TemplateId,
            name = "旅程模板",
            version = "1.0.0",
            workflow = new
            {
                id = TemplateId,
                name = "旅程模板",
                nodes = new object[]
                {
                    new
                    {
                        id = "journey-planner",
                        type_name = "planner",
                        config = new { },
                        position = new { x = 120.0, y = 120.0 },
                    },
                    new
                    {
                        id = "journey-writer",
                        type_name = "writer",
                        config = new { },
                        position = new { x = 390.0, y = 120.0 },
                    },
                },
                edges = new object[]
                {
                    new
                    {
                        id = "journey-plan-to-write",
                        kind = "control",
                        from = new { node_id = "journey-planner", port_name = "exec_out" },
                        to = new { node_id = "journey-writer", port_name = "exec_in" },
                    },
                },
                metadata = new { },
            },
            prompt_templates = Array.Empty<object>(),
            required_node_types = new[] { "planner", "writer" },
            required_tools = Array.Empty<string>(),
            required_permissions = Array.Empty<string>(),
            minimum_ariadne_version = "0.1.0",
            metadata = new { },
        };

        private static byte[] BuildResponse(int status, string body)
        {
            var reason = status switch
            {
                200 => "OK",
                404 => "Not Found",
                500 => "Internal Server Error",
                _ => "Error",
            };
            var bytes = Encoding.UTF8.GetByteCount(body);
            return Encoding.UTF8.GetBytes(
                $"HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n"
                + $"Content-Length: {bytes}\r\nConnection: close\r\n\r\n{body}");
        }

        /// <summary>
        /// 只读到请求行即可路由——这三个端点都是 GET、无请求体。
        /// 但仍须读到 header 结束，否则某些客户端会因为我们提前回包而报错。
        /// </summary>
        private static async Task<string> ReadRequestLineAsync(NetworkStream stream)
        {
            var buffer = new byte[8 * 1024];
            var text = new StringBuilder();
            while (text.ToString().IndexOf("\r\n\r\n", StringComparison.Ordinal) < 0)
            {
                var read = await stream
                    .ReadAsync(buffer)
                    .AsTask()
                    .WaitAsync(TimeSpan.FromSeconds(30));
                if (read <= 0)
                {
                    break;
                }
                text.Append(Encoding.UTF8.GetString(buffer, 0, read));
            }

            var whole = text.ToString();
            var lineEnd = whole.IndexOf("\r\n", StringComparison.Ordinal);
            return lineEnd < 0 ? whole : whole[..lineEnd];
        }

        public void Dispose()
        {
            _shutdown.Cancel();
            try
            {
                _listener.Stop();
            }
            catch
            {
                // 已停就算了。
            }

            try
            {
                _loop.Wait(TimeSpan.FromSeconds(5));
            }
            catch
            {
                // 服务循环退出异常不影响测试结论。
            }

            _shutdown.Dispose();
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
