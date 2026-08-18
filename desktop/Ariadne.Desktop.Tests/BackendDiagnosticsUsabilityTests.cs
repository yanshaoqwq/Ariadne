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
/// 无项目时返回 validation 错误、reason 字段带裸英文。
/// 两者都不会抛异常，进程内 mock 一律照过。
///
/// 三条用例的现状（2026-08-18 逐条在真实 IPC 上复核过 payload）：
///   A. ✅ 已修（真缺陷）——无项目时 `get_backend_diagnostics` 直接 validation
///      失败 ⇒ 分区永远空白，而 `secrets.protection` 本不依赖项目根
///   B. ⚠️ **判据已改写**——原版断言「干净项目不该报 unavailable」，
///      其前提（「保存凭据会失败只是假设性的」）被实测推翻：Locked 状态下
///      `save_provider_key` **当场失败**。详见该用例注释里的完整实测链路。
///      现改为钉住「这条真实阻断报得对」。
///   C. ✅ 已修（真缺陷）——部分 reason 是裸英文，前端只对 `diagnostics.`
///      前缀查表 ⇒ 后端知道具体成因，界面只显示通用兜底文案
/// </summary>
[Collection("RealSidecar")]
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
    /// U172-B：**「凭据存储已锁定」必须报成真实阻断，而不是被降级成提醒。**
    ///
    /// ⚠️ 本用例是**改写过的**。原版断言「干净项目不该报 overall=unavailable」，
    /// 理由是「全新项目还没配 Provider、没有密钥要存，此时『保存凭据会失败』
    /// 是**假设性**的」。**这个前提被实测推翻，所以原判据是错的。**
    ///
    /// 实测（真实 sidecar，按序单条往返，不是批量——批量会被 8 worker 线程池乱序）：
    ///   create_project                        → ok
    ///   save_provider_settings(openai/gpt-4o) → ok
    ///   save_provider_key(openai, sk-…)       → **ok=false**
    ///     `validation failed: local secret store is locked:
    ///      call set_local_secret_master_password to encrypt credentials,
    ///      or allow_unprotected_local_secrets to store them in plain text`
    ///   allow_unprotected_local_secrets       → ok
    ///   save_provider_key（同一次调用重放）   → **ok=true**，
    ///     诊断随之 unavailable → degraded
    ///
    /// ⇒ Locked 状态下保存密钥**当场就失败**，不是假设性的。用户配 Provider 的
    /// 第一步就撞墙。`unavailable`（=坏了、拦住了）正是它应有的分级；
    /// 把它降成 degraded 才是缺陷——那会让「配不了任何模型」显示成
    /// 「仍可使用，只需检查配置」，与事实相反。
    ///
    /// 所以本用例改为钉住**正确的**那件事：这一项必须存在、必须是 unavailable、
    /// 且 reason 必须指向可本地化的成因。判据取「后端真实返回的该项」。
    ///
    /// 为什么不干脆删掉：`aggregate_status` 取最坏项这件事值得有回归——
    /// 若哪天有人为了「让新项目看起来干净」把 Locked 改判成 Degraded，
    /// 本用例会当场变红，并把上面这段实测记录摆在他面前。
    ///
    /// ⚠️ 真正的产品缺口不在分级，而在**解锁入口前端三层全无**
    /// （客户端无方法、配置页无控件、语言包无按钮文案），
    /// 那条由 `SecretProtectionRecoveryReachableTests`（U176）钉。
    /// 两者必须同批看：只调分级会把「配不了模型」这个事实藏起来，
    /// 用户看到「需要注意」却依然无从下手。
    /// </summary>
    [Fact]
    public async Task Diagnostics_OnFreshProject_ReportsLockedCredentialsAsARealBlock()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Diagnostics_OnFreshProject_ReportsLockedCredentialsAsARealBlock)))
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

            var secrets = Assert.Single(
                report.Items,
                item => item.Component == "secrets.protection");

            // 分级必须是 unavailable：上面的实测证明此刻保存密钥必失败。
            Assert.Equal("unavailable", secrets.Status);

            // 成因必须可本地化，否则界面上只剩按 status 的兜底文案，
            // 「凭据存储已锁定」这个唯一可据以行动的结论会丢失（同 U172-C）。
            Assert.Equal("diagnostics.secrets.locked", secrets.Reason);

            // 总体状态由最坏项决定 —— 有真实阻断时必须传导上去，
            // 否则状态栏在「配不了任何模型」时仍显示健康/需注意。
            Assert.Equal("unavailable", report.Status);
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
