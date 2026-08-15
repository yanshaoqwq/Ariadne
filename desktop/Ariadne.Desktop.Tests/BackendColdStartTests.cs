using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 用**真实** sidecar 进程验证「应用刚启动、尚未打开项目」时的后端可用性。
///
/// 这是用户报告的 P0 现场：点大多数按钮报「无法连接到后端服务」，
/// 连新建项目都不行。此前所有桌面测试都用 mock backend，因此这条
/// 真实进程链路从未被覆盖过。
///
/// sidecar 不存在时自动跳过，避免在未编译后端的环境里误报失败。
/// </summary>
public sealed class BackendColdStartTests
{
    private static string? ResolveSidecar()
    {
        // U142：起真实 sidecar 前先确认 app-state 已隔离。缺了它，
        // 本文件建的 "Cold Start" 项目会写进用户真实的 recent_projects.json，
        // 而该列表只留 20 条，测试残留会把用户自己的项目挤出去。
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

    [Fact]
    public async Task ColdStart_WithoutOpenProject_DoesNotReportTransportFailure()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null)
        {
            // sidecar 未编译时不做断言：避免在纯前端环境里误报失败。
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);

        // 冷启动序列：应用起来后第一批请求，此时还没有任何项目。
        var appStatus = await client.GetAppStatusAsync();
        Assert.NotNull(appStatus);

        // 最近项目列表是欢迎页的数据源，必须在无项目时可用。
        var recents = await client.ListRecentProjectsAsync();
        Assert.NotNull(recents);

        // 个性化是「全局」设置（项目树外），无项目时必须可读——
        // 否则设置页个性化分区 never loaded，保存按钮永久置灰。
        var prefs = await client.GetUiPreferencesAsync();
        Assert.NotNull(prefs);
    }

    [Fact]
    public async Task ColdStart_CanCreateProjectAndThenReadProjectScopedSettings()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null)
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-coldstart-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");

            // 新建项目是用户报告「甚至无法新建项目」的那一步。
            var report = await client.CreateProjectAsync(projectRoot, "Cold Start");
            Assert.NotNull(report);
            Assert.True(client.HasProjectRoot, "新建项目后客户端必须记住项目根");

            // 项目建好后，项目作用域的设置才应该可读。
            var appSettings = await client.GetAppSettingsAsync();
            Assert.NotNull(appSettings);
        }
        finally
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

    /// <summary>
    /// sidecar 找不到时，错误信息必须能指导排查，而不是一句「无法连接」。
    /// </summary>
    [Fact]
    public void MissingSidecar_ProducesActionableDiagnostic()
    {
        var empty = Directory.CreateTempSubdirectory("ariadne-nosidecar-");
        try
        {
            var found = JsonLineBackendClient.DiscoverBackendCommand(empty.FullName, empty.FullName);
            Assert.Null(found);

            var report = JsonLineBackendClient.LastDiscoveryReport;
            Assert.False(string.IsNullOrWhiteSpace(report), "查找失败必须留下诊断报告");
            Assert.Contains("ARIADNE_BACKEND_IPC", report!, StringComparison.Ordinal);
            Assert.Contains("Searched", report, StringComparison.Ordinal);
        }
        finally
        {
            try
            {
                empty.Delete(recursive: true);
            }
            catch
            {
                // 忽略清理失败。
            }
        }
    }
}
