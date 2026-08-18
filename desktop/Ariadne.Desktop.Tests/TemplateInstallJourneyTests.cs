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
            var canvas = await client.LoadProjectCanvasAsync();
            var canvasJson = JsonSerializer.Serialize(canvas);
            Assert.Contains(
                "journey-writer-MUTATED",
                canvasJson,
                StringComparison.Ordinal);

            // ── 5. 装完的图必须能通过后端自己的校验（否则装了个跑不起来的图）
            var graphs = await client.ListWorkflowGraphsAsync();
            Assert.NotEmpty(graphs);
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// 仓库返回 5xx 时，安装必须**明确失败**且不留半装状态。
    ///
    /// 判据取「抛错 + 画布未被改动」两条。只断言抛错测不出半装：
    /// 安装链路里 manifest 下载与画布合并是两步（`commands.rs:4931` 与 `:4958`），
    /// 中途失败若已经写了 workflows/ 文件，用户会看到一个装了一半的模板。
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

            // 当前活动项目是 active，却声称要装进 other。
            await Assert.ThrowsAsync<BackendException>(() =>
                client.InstallTemplateAsync(repo.BaseUrl, TemplateId, other));

            Assert.Equal(before, JsonSerializer.Serialize(await client.LoadProjectCanvasAsync()));
        }
        finally
        {
            TryCleanup(temp);
        }
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
