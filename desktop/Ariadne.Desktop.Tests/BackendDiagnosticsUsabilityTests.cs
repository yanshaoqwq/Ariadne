using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U172：设置页「诊断信息」分区在真实后端上不可用/不可信。用户报告「那个诊断里还不可用呢」。
///
/// 本文件全部走**真实 sidecar 进程**，判据取「后端真实返回的 payload」
/// 与「前端真实渲染出的字符串」，不取「命令没报错」。
/// 理由：诊断分区的失效形态恰恰都是**成功返回**——
/// 无项目时返回 validation 错误、新项目返回 status=unavailable、
/// reason 字段带裸英文。三者都不会抛异常，进程内 mock 一律照过。
///
/// 覆盖三条已实测确认的缺陷（每条都在真实 IPC 上复现过，见各用例注释）：
///   A. 无项目时 `get_backend_diagnostics` 直接 validation 失败 ⇒ 分区永远空白
///   B. 刚建好的干净项目总体状态 = `unavailable`（不可用）
///   C. 部分 reason 是裸英文，直达中/日用户界面
/// </summary>
public sealed class BackendDiagnosticsUsabilityTests
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
    /// U172-A：**无项目时诊断分区取不到任何数据。**
    ///
    /// 实测（真实 sidecar，未打开项目）：
    ///   → `{"ok":false,"error":"project_root cannot be empty","error_code":"validation"}`
    ///
    /// 根因在 `commands.rs:5020`：`get_backend_diagnostics` 第一行就
    /// `project_root_from_state(state, None)?`，无项目直接 bail。
    /// 但它上报的组件里有**一半是全局的**——`secrets.protection`、
    /// `providers.config`、`providers.*.default` 都不依赖项目根
    /// （凭据与 Provider 都是应用级配置）。
    ///
    /// 用户可见后果：应用刚起来、还没开项目时进设置页看诊断，分区是空的，
    /// 而这正是最需要诊断的时刻（「为什么连不上模型」「密钥存哪了」）。
    ///
    /// **判据取「无项目时能否拿到报告」**，而不是「有项目时能否拿到」——
    /// 后者现在就是绿的，写了等于没写。
    /// </summary>
    [Fact]
    public async Task Diagnostics_WithoutOpenProject_MustStillReportGlobalComponents()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Diagnostics_WithoutOpenProject_MustStillReportGlobalComponents)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);

        // 前提：确认此刻真的没有项目——否则本用例测的不是「无项目」这一支。
        Assert.False(client.HasProjectRoot, "本用例的前提是尚未打开项目");

        var report = await client.GetBackendDiagnosticsAsync();

        Assert.NotNull(report);
        Assert.NotEmpty(report.Items);

        // 全局组件（不依赖项目根）必须在无项目时也可见。
        // 挑 secrets.protection：它是应用级凭据保护状态，与项目无关，
        // 且它恰好是「无项目时用户最可能想查」的那一项。
        Assert.Contains(
            report.Items,
            item => item.Component == "secrets.protection");
    }

    /// <summary>
    /// U172-B：**刚建好的干净项目，诊断总体状态是「不可用」。**
    ///
    /// 实测（真实 sidecar，create_project 之后立刻查）：
    ///   status = "unavailable"，唯一的 unavailable 项是
    ///   `{"component":"secrets.protection","reason":"diagnostics.secrets.locked"}`
    ///
    /// 链条（三处都已读代码确认）：
    ///   1. `secrets.rs:584-588` —— 新项目既无主密码也无明文许可 ⇒ `Locked`
    ///   2. `commands.rs:5081` —— `Locked` 映射成 `DiagnosticStatus::Unavailable`
    ///   3. `diagnostics/mod.rs:48` `aggregate_status` —— 取最坏项 ⇒ 总体 unavailable
    ///
    /// 每一步单独看都合理，**合起来的产品结论是错的**：全新项目还没配任何
    /// Provider、没有任何密钥要存，此时「保存凭据会失败」是**假设性**的，
    /// 不是当下的阻断。而它却把整个后端标成「不可用」。
    ///
    /// 用户可见后果：新建项目、什么都还没做，诊断页顶部写「总体：不可用」。
    /// 这会让人以为装坏了。且它使总体状态失去分辨力——
    /// 真的出故障时，状态栏文字**没有任何变化**。
    ///
    /// **判据取总体状态**，而不是「有没有 secrets 那一项」：
    /// 那一项本身是该报的（明文密钥要持续提醒，U118 已定夺），
    /// 缺陷在于**它把总体拉成不可用**。
    /// </summary>
    [Fact]
    public async Task Diagnostics_OnFreshProject_MustNotReportOverallUnavailable()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Diagnostics_OnFreshProject_MustNotReportOverallUnavailable)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-diag-fresh-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Diag Fresh");

            var report = await client.GetBackendDiagnosticsAsync();
            Assert.NotNull(report);

            // 干净项目不该被判为「不可用」。degraded 可以接受
            // （确实有「未配置模型」这类待办），unavailable 是「坏了」。
            Assert.NotEqual("unavailable", report.Status);

            // 同时钉住这条结论的来源，避免将来有人靠「删掉 secrets 那一项」
            // 来让本用例转绿——那会丢掉 U118 刻意保留的明文提醒。
            Assert.Single(report.Items, item => item.Component == "secrets.protection");
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// U172-C：**部分 reason 是裸英文，会直接显示给中/日文用户。**
    ///
    /// 实测（真实 sidecar，干净项目）这些 reason 原样返回：
    ///   "knowledge index manifest is missing"
    ///   "no enabled LLM provider is configured"
    /// 而同一份 payload 里另一些走了本地化 key：
    ///   "diagnostics.secrets.locked" / "diagnostics.providers.embedding.disabled"
    ///
    /// 前端 `SettingsPageViewModel.DiagnosticReasonLabel`（`:3095-3105`）
    /// **只对 `diagnostics.` 前缀查表**，其余一律落到按 status 分档的兜底文案。
    /// 所以裸英文不会显示出来——但代价是这些组件的具体原因**全部丢失**，
    /// 用户只看到一句「该组件仍可使用，但需要检查配置」，
    /// 而后端明明知道是「没有启用任何对话模型」。
    ///
    /// ⇒ 缺陷有两面，本用例钉的是**可诊断性**那一面：
    /// 后端给出了具体原因，前端却因为约定不统一而无法呈现。
    /// `commands.rs:5075-5076` 的注释已经写明该约定
    /// （"reason 必须是本地化 key"），但只有部分生产点遵守了。
    ///
    /// **判据取「reason 是否都可本地化」**，不取「是否含英文单词」——
    /// 后者会被将来合法的 key 名（本身就是英文标识符）绊倒。
    /// </summary>
    [Fact]
    public async Task Diagnostics_EveryReason_MustBeALocalizableKey()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Diagnostics_EveryReason_MustBeALocalizableKey)))
        {
            return;
        }

        var temp = Directory.CreateTempSubdirectory("ariadne-diag-reason-");
        try
        {
            using var client = new JsonLineBackendClient(sidecar);
            var projectRoot = Path.Combine(temp.FullName, "novel");
            await client.CreateProjectAsync(projectRoot, "Diag Reason");

            var report = await client.GetBackendDiagnosticsAsync();
            var displayNames = DisplayNameService.LoadDefault();

            var untranslatable = report.Items
                .Where(item => !string.IsNullOrWhiteSpace(item.Reason))
                .Where(item => !IsLocalizable(item.Reason!, displayNames))
                .Select(item => $"{item.Component} → \"{item.Reason}\"")
                .ToList();

            Assert.True(
                untranslatable.Count == 0,
                "以下诊断项的 reason 不是可本地化的 key，其具体原因在界面上会被兜底文案吞掉"
                + "（commands.rs:5075 已约定 reason 必须是 diagnostics.* key）：\n  "
                + string.Join("\n  ", untranslatable));
        }
        finally
        {
            TryCleanup(temp);
        }
    }

    /// <summary>
    /// reason 可本地化 = 带 `diagnostics.` 前缀**且**该 key 在语言包里真的存在。
    /// 两个条件都要：只查前缀会让一个拼错的 key 通过（界面显示 `[diagnostics.typo]`），
    /// 只查存在性则无从判断裸英文。
    /// </summary>
    private static bool IsLocalizable(string reason, DisplayNameService displayNames)
    {
        if (!reason.StartsWith("diagnostics.", StringComparison.Ordinal))
        {
            return false;
        }

        // DisplayNameService 缺 key 时返回 `[key]`，这是它约定的自查形态。
        return !string.Equals(displayNames.Text(reason), $"[{reason}]", StringComparison.Ordinal);
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
