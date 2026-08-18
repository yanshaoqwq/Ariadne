using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U176：**凭据保护的补救动作在界面上不存在。**
///
/// 后端有两个命令可以解开「凭据存储已锁定」这个状态
/// （`core/src/ipc.rs:924` / `:931`）：
///   - `set_local_secret_master_password`
///   - `allow_unprotected_local_secrets`
///
/// 而**前端三层全无**：客户端接口没有对应方法、设置页没有入口控件、
/// 语言包里没有按钮文案。可是诊断文案已经在教用户去用它：
///
///   `diagnostics.secrets.locked`
///     → 「凭据存储已锁定，保存密钥会失败。请配置本地主密码，或显式接受明文保存。」
///   `ui.settings.misc.diagnostics.recovery.secrets`
///     → 「在配置页为密钥配置本地主密码，或确认明文存储的风险。」
///
/// ⇒ **诊断把用户指向一个界面上不存在的操作。** 配置页没有那个东西。
/// 这比不给提示更糟：用户会在设置页反复找一个不存在的开关。
///
/// **与 U172-B 相扣**：新项目默认就是 `Locked`（`secrets.rs:584-588`），
/// 导致诊断总体状态是「不可用」。而用户即使想解决也无从下手——
/// 唯一的解法前端没接。修 U172-B 时若只调分级，用户看到的会从
/// 「不可用」变成「需要注意」，但**仍然解决不了**。两件事要同批做。
///
/// **判据取「客户端是否暴露了这条能力」**，不取「文案是否存在」——
/// 文案恰恰是存在的，那正是这个缺陷具有欺骗性的原因。
///
/// ⚠️ 本文件与 `SecretProtectionDiagnosticsTests` 互补而非重复：
/// 那份钉的是「诊断项显示得对不对」（组件名有映射、补救文案不落到
/// 『重启应用』那条兜底），本份钉的是「那个补救动作到底能不能做」。
/// 那份的类注释写着补救动作必须指向「**产品里真实存在**」的操作——
/// 本份就是把「真实存在」这半句变成可执行断言。
/// </summary>
public sealed class SecretProtectionRecoveryReachableTests
{
    /// <summary>
    /// 诊断承诺的补救动作必须有对应的客户端能力。
    ///
    /// U176 已修复：`IAriadneBackendClient` 现有
    /// `SetLocalSecretMasterPasswordAsync` / `AllowUnprotectedLocalSecretsAsync`
    /// / `GetSecretProtectionAsync`（`JsonLineBackendClient` 分别打到
    /// `set_local_secret_master_password` / `allow_unprotected_local_secrets` /
    /// `get_secret_protection`）。
    /// </summary>
    [Fact]
    public void SecretRecoveryCommands_MustBeReachableFromTheDesktopClient()
    {
        var methods = typeof(IAriadneBackendClient)
            .GetMethods()
            .Select(method => method.Name)
            .ToList();

        // 命名不必与后端命令逐字一致，所以按语义关键词找，
        // 避免把「改了个方法名」误报成「能力缺失」。
        var hasMasterPassword = methods.Any(name =>
            name.Contains("MasterPassword", StringComparison.OrdinalIgnoreCase));
        var hasAllowUnprotected = methods.Any(name =>
            name.Contains("Unprotected", StringComparison.OrdinalIgnoreCase));

        // 修好之后判据收紧成「两条都要有」：原来写 `||` 是因为当时两条都缺、
        // 只要接上任意一条就算脱离死胡同。现在两条都在，`||` 会让「后来删掉
        // 接受明文那条」照样绿——而那正是 Locked 状态下用户不愿设主密码时
        // 唯一的另一条出路。
        Assert.True(
            hasMasterPassword && hasAllowUnprotected,
            "`IAriadneBackendClient` 缺少凭据保护补救能力"
            + $"（设本地主密码={hasMasterPassword}、显式接受明文={hasAllowUnprotected}），"
            + "而后端两个命令都在（ipc.rs:924、:931），"
            + "且诊断文案 `diagnostics.secrets.locked` 正在让用户「配置本地主密码」。"
            + "⇒ 用户被指向一个界面上做不到的操作（U176）。");

        // 真实发出去的命令名也要钉住：接口有方法但打错命令名时，
        // 上面那条照样绿，而用户点按钮只会收到一条 method not found。
        var client = File.ReadAllText(ResolveDesktopSource("Backend", "JsonLineBackendClient.cs"));
        Assert.Contains("\"set_local_secret_master_password\"", client, StringComparison.Ordinal);
        Assert.Contains("\"allow_unprotected_local_secrets\"", client, StringComparison.Ordinal);
        // 参数字段名必须是 master_password：后端 `SetMasterPasswordParams` 按该名
        // 反序列化，写成 masterPassword 会得到「missing field」而非任何界面提示。
        Assert.Contains("master_password = masterPassword", client, StringComparison.Ordinal);
    }

    /// <summary>
    /// 补救文案指名「配置页」，那么配置页必须真的有那个入口。
    ///
    /// 判据取「设置页 XAML 里是否存在相关控件」。用 XAML 文本而非运行时视图树：
    /// 本机 headless 起窗口有已知风险，而「入口是否存在」在标记层就能判定。
    ///
    /// ⚠️ 这条与上一条**不是重复**：客户端有方法 ≠ 界面上有按钮。
    /// 两条都红说明整条链都没接；只有第二条红说明后端能力已接进客户端、
    /// 但设置页忘了放入口——两种情况的修法不同。
    ///
    /// U176 已修复：入口落在**权限控制页**最末一节（`secret_protection`）。
    /// </summary>
    [Fact]
    public void SettingsPage_MustOfferTheRecoveryEntryItsDiagnosticPromises()
    {
        var names = DisplayNameService.LoadDefault();

        // 前提：补救文案确实在指「配置页」。若哪天文案改了指向，
        // 本用例的前提就不成立，应当同步更新而不是硬留着。
        var recovery = names.Text("ui.settings.misc.diagnostics.recovery.secrets");
        Assert.Contains("配置页", recovery, StringComparison.Ordinal);

        var view = File.ReadAllText(ResolveDesktopSource("Views", "SettingsPageView.axaml"));

        var mentionsRecovery =
            view.Contains("MasterPassword", StringComparison.OrdinalIgnoreCase)
            || view.Contains("master_password", StringComparison.OrdinalIgnoreCase)
            || view.Contains("Unprotected", StringComparison.OrdinalIgnoreCase);

        Assert.True(
            mentionsRecovery,
            "设置页没有任何「设置本地主密码 / 接受明文保存」的入口，"
            + $"而诊断补救文案写着「{recovery}」——"
            + "用户会在配置页反复找一个不存在的开关（U176）。");

        // 判据必须落到「按钮真的接了命令」上，不能只看关键词出现。
        // 光提到 MasterPassword 的注释也能让上面那条绿，而注释点不动。
        Assert.Contains(
            "Command=\"{Binding SetSecretMasterPasswordCommand}\"",
            view,
            StringComparison.Ordinal);
        Assert.Contains(
            "Command=\"{Binding AllowUnprotectedSecretsCommand}\"",
            view,
            StringComparison.Ordinal);

        // 入口在权限页的可见性分组内，不是漂在别的页签里。
        var anchor = view.IndexOf("x:Name=\"SecretProtectionSectionAnchor\"", StringComparison.Ordinal);
        var permissionsGroup = view.IndexOf(
            "IsVisible=\"{Binding IsPermissionsSelected}\"", StringComparison.Ordinal);
        var personalizationGroup = view.IndexOf(
            "IsVisible=\"{Binding IsPersonalizationSelected}\"", StringComparison.Ordinal);
        Assert.True(anchor > 0 && permissionsGroup > 0);
        Assert.InRange(anchor, permissionsGroup, personalizationGroup);

        // 左侧页内索引必须能跳到它，否则长页里等于藏起来了。
        var section = Assert.Single(
            SettingsNavigationCatalog.Sections,
            item => item.Id == "secret_protection");
        Assert.Equal("permissions", section.TabId);
        Assert.Equal("SecretProtectionSectionAnchor", section.AnchorName);
    }

    /// <summary>
    /// 按钮文案必须三份语言包齐备。
    ///
    /// 这条**取代**了原先那条「文案还没建、故意绿着当施工清单」的标记用例
    /// （`RecoveryButtonCopy_IsStillMissing_AndThatIsPartOfTheSameFix`）：
    /// 入口已接好，那条按它自己的注释说明应当删掉，否则它会永远红着。
    /// 判据由「文案还不存在」翻成「文案必须存在且不是描述性诊断文案」——
    /// 后者才是接线之后需要长期守住的性质。
    ///
    /// ⚠️ 不检查 en/ja 的**内容**（用户会另行补翻译），只检查 key 存在：
    /// `DisplayNameService` 缺键时静默回落中文，缺口只有键集合守卫能发现。
    /// </summary>
    [Theory]
    [InlineData("zh")]
    [InlineData("en")]
    [InlineData("ja")]
    public void RecoveryButtonCopy_ExistsInEveryLanguage(string language)
    {
        var names = DisplayNameService.LoadDefault();
        names.SwitchLanguage(language);

        foreach (var key in new[]
                 {
                     "ui.settings.secrets.title",
                     "ui.settings.secrets.status",
                     "ui.settings.secrets.status.managed",
                     "ui.settings.secrets.status.encrypted",
                     "ui.settings.secrets.status.unprotected",
                     "ui.settings.secrets.status.locked",
                     "ui.settings.secrets.status.unknown",
                     "ui.settings.secrets.locked_hint",
                     "ui.settings.secrets.master_password_label",
                     "ui.settings.secrets.master_password_placeholder",
                     "ui.settings.secrets.set_master_password",
                     "ui.settings.secrets.master_password_hint",
                     "ui.settings.secrets.allow_plaintext",
                     "ui.settings.secrets.allow_plaintext_warning",
                     "ui.settings.secrets.master_password_required",
                     "ui.settings.secrets.master_password_applied",
                     "ui.settings.secrets.plaintext_applied",
                     "ui.dialog.settings.allow_plaintext.title",
                     "ui.dialog.settings.allow_plaintext.message",
                     "ui.settings.section.secret_protection",
                 })
        {
            var text = names.Text(key);
            Assert.False(
                text == $"[{key}]",
                $"{language} 缺少凭据保护文案 {key}——UI 上会显示方括号乱码（U176）");
            Assert.False(string.IsNullOrWhiteSpace(text));
        }

        names.SwitchLanguage("zh");

        // 按钮标签不能是那句诊断描述：混用会让按钮上出现一整句「请配置本地
        // 主密码，或显式接受明文保存。」——文档第 3 节点名要避免的形态。
        Assert.NotEqual(
            names.Text("diagnostics.secrets.locked"),
            names.Text("ui.settings.secrets.set_master_password"));
        Assert.NotEqual(
            names.Text("ui.settings.misc.diagnostics.recovery.secrets"),
            names.Text("ui.settings.secrets.allow_plaintext"));
    }

    /// <summary>
    /// **本批最重要的一条**：点「设置本地主密码」→ 口令真的出站 →
    /// 保护状态转 Encrypted → **诊断分区当场重取**。
    ///
    /// 判据刻意取三样**真实运行态**产物，而不是「命令能否执行」：
    /// (1) 后端收到的口令等于用户敲进去的那串（不是某个常量、不是空串）；
    /// (2) `DiagnosticsItems` 里那条 `secrets.protection` 的状态标签从
    ///     「不可用」翻成「健康」；
    /// (3) 后端诊断被**多问了一次**。
    ///
    /// 第 (3) 条是这条用例的核心：`protection_status()` 转 Encrypted 后后端就报
    /// Healthy（`commands.rs` 的 `secret_protection_diagnostic_item` 早已就绪），但前端那份诊断报告是缓存快照。
    /// 不主动重取，用户设完主密码仍看到「凭据存储已锁定」，会以为没生效然后再点一次。
    /// 只断言 (1)(2) 挡不住这个：状态属性是命令返回值直接写的，
    /// 摘掉 `RefreshDiagnosticsAsync` 后它俩照样对。
    /// </summary>
    [Fact]
    public async Task SettingMasterPassword_SendsItOutAndTurnsTheDiagnosticHealthy()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = SecretProtectionBackend.Create();
        var vm = new SettingsPageViewModel(names, (IAriadneBackendClient)(object)backend);

        // 干净基线：从 locked 起跑（真实装机后的形态），并把诊断快照灌成
        // 后端此刻真会给的那份「不可用」。
        await vm.RefreshSecretProtectionForTestsAsync();
        Assert.Equal("locked", vm.SecretProtectionStatus);
        Assert.True(vm.IsSecretStoreLocked);

        await vm.RefreshDiagnosticsForTestsAsync();
        var lockedItem = Assert.Single(vm.DiagnosticsItems);
        Assert.Equal(
            names.Text("ui.settings.misc.diagnostics.status.unavailable"),
            lockedItem.Status);
        var diagnosticsBefore = backend.DiagnosticsCalls;

        // 用户敲的口令：刻意不是任何默认值/空串，否则「口令送出去了」会恒真。
        vm.SecretMasterPassword = "u176-typed-by-the-user";
        vm.SetSecretMasterPasswordCommand.Execute(null);
        await DrainAsync(() => !vm.IsSecretProtectionBusy && backend.LastMasterPassword is not null);

        // (1) 真实出站的正是用户敲的那串。
        Assert.Equal("u176-typed-by-the-user", backend.LastMasterPassword);
        // 口令用完即清，不留在屏幕上。
        Assert.Equal(string.Empty, vm.SecretMasterPassword);

        // (2) 保护状态翻转，锁定提示随之消失。
        Assert.Equal("encrypted", vm.SecretProtectionStatus);
        Assert.False(vm.IsSecretStoreLocked);
        Assert.Equal(
            names.Format(
                "ui.settings.secrets.status",
                new Dictionary<string, string>
                {
                    ["status"] = names.Text("ui.settings.secrets.status.encrypted"),
                }),
            vm.SecretProtectionStatusText);

        // (3) 诊断被重取，且那条 secrets.protection 现在是「健康」。
        Assert.True(
            backend.DiagnosticsCalls > diagnosticsBefore,
            "设完主密码后设置页没有重取诊断——用户会继续看到「凭据存储已锁定」"
            + "那条旧告警，从而以为没生效（U176）。");
        var healthyItem = Assert.Single(vm.DiagnosticsItems);
        Assert.Equal(
            names.Text("ui.settings.misc.diagnostics.status.healthy"),
            healthyItem.Status);
    }

    /// <summary>
    /// 「接受明文」必须先过确认弹窗；**取消就什么都不做**。
    ///
    /// 判据取「后端一次都没被调」而不是「状态没变」：后者在
    /// 「调了后端但状态刚好没翻」时也绿，而那已经是明文落盘了。
    /// `DialogService` 无人应答时 `Completion` 永不完成，因此本用例
    /// 只需确认命令**卡在弹窗上**、没有直接打后端。
    /// </summary>
    [Fact]
    public async Task AllowingPlaintext_RequiresAnExplicitConfirmation()
    {
        var names = DisplayNameService.LoadDefault();
        DialogService.Initialize(names);
        var backend = SecretProtectionBackend.Create();
        var vm = new SettingsPageViewModel(names, (IAriadneBackendClient)(object)backend);

        await vm.RefreshSecretProtectionForTestsAsync();
        Assert.Equal("locked", vm.SecretProtectionStatus);

        vm.AllowUnprotectedSecretsCommand.Execute(null);

        // 判据分两段。第一段：命令**必须先卡在弹窗上**。
        // 摘掉确认弹窗的话它会直接打后端、把状态推成 unprotected，
        // 此时 IsOpen 永远不为真——所以这里不能只 DrainAsync 到超时了事，
        // 要显式区分「弹窗没弹出来」和「弹窗弹了但用户还没点」。
        for (var attempt = 0; attempt < 500 && !DialogService.Current.IsOpen; attempt++)
        {
            // ConfigureAwait(true)：xUnit1030 —— (false) 会绕过并行度限制。
            // 这里等的是 DialogService 的进程内状态，不需要脱离同步上下文。
            await Task.Delay(2).ConfigureAwait(true);
        }

        Assert.True(
            DialogService.Current.IsOpen,
            "点「接受明文保存」没有弹确认框"
            + $"（当前保护状态已经是「{vm.SecretProtectionStatus}」）——"
            + "明文许可被静默施加了。`Unprotected` 与 `Locked` 分成两个状态是"
            + "secrets.rs:87-103 刻意的设计：不带警告的开关等于让用户在不知情的"
            + "情况下把 API Key 摊在磁盘上（U176）。");

        var dialog = Assert.IsType<ConfirmDialogViewModel>(DialogService.Current.ActiveDialog);
        // 危险语义：Enter 不绑确认键，避免「一路回车」把 API Key 摊到磁盘上。
        Assert.Equal(DialogSeverity.Danger, dialog.Severity);
        Assert.False(dialog.AllowEnterConfirm);
        Assert.Equal(
            names.Text("ui.dialog.settings.allow_plaintext.title"),
            dialog.Title);

        // 用户取消 ⇒ 后端一次都不该被碰，状态留在 locked。
        dialog.Cancel();
        await DrainAsync(() => !DialogService.Current.IsOpen);
        Assert.Equal("locked", vm.SecretProtectionStatus);
        Assert.False(vm.IsSecretStorePlaintext);
    }



    /// <summary>轮询等待异步命令跑完；超时按失败处理而不是静默放过。</summary>
    private static async Task DrainAsync(Func<bool> done)
    {
        for (var attempt = 0; attempt < 500; attempt++)
        {
            if (done())
            {
                return;
            }

            await Task.Delay(2).ConfigureAwait(false);
        }

        Assert.Fail("等待凭据保护命令完成超时（500 × 2ms）");
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 12 && walk is not null; i++)
        {
            var candidate = Path.Combine(
                new[] { walk.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate) || Directory.Exists(candidate))
            {
                return candidate;
            }

            walk = walk.Parent;
        }

        throw new FileNotFoundException(string.Join('/', parts));
    }

    /// <summary>
    /// U176：模拟真实的凭据保护状态机——**从锁定开始**。
    ///
    /// 起始状态刻意是 `locked`，与干净装机后的真实形态一致
    /// （`secrets.rs:584-588`：既无主密码也无明文许可 ⇒ Locked）。
    /// 用 `encrypted` 当起点会让用例跑在一个真实用户从未见过的状态上，
    /// 那正是这个缺陷长期藏身的原因：所有测试都靠
    /// `ARIADNE_SECRET_MASTER_KEY` 从加密态起跑。
    ///
    /// 诊断报告**按当前保护状态现算**，不是固定桩：固定桩会让
    /// 「设完主密码后诊断转健康」这条断言变成断言桩本身，
    /// 摘掉前端的重取也照样绿。
    ///
    /// `DispatchProxy` 宿主不能 `sealed`（运行时要派生它）。
    /// </summary>
    private class SecretProtectionBackend : DispatchProxy
    {
        private string _status = "locked";

        /// <summary>后端被问了几次诊断。基线差值就是「有没有重取」的判据。</summary>
        public int DiagnosticsCalls { get; private set; }

        /// <summary>真实出站的口令，用来钉住「按钮把用户敲的那串送出去了」。</summary>
        public string? LastMasterPassword { get; private set; }

        public static SecretProtectionBackend Create() =>
            (SecretProtectionBackend)(object)Create<IAriadneBackendClient, SecretProtectionBackend>()!;

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            switch (targetMethod?.Name)
            {
                case nameof(IAriadneBackendClient.GetSecretProtectionAsync):
                    return Task.FromResult(Report());

                case nameof(IAriadneBackendClient.SetLocalSecretMasterPasswordAsync):
                    LastMasterPassword = args?.Length > 0 ? args[0] as string : null;
                    // 与后端一致：设了主密码即 Encrypted（secrets.rs:585）。
                    _status = "encrypted";
                    return Task.FromResult(Report());

                case nameof(IAriadneBackendClient.AllowUnprotectedLocalSecretsAsync):
                    _status = "unprotected";
                    return Task.FromResult(Report());

                case nameof(IAriadneBackendClient.GetBackendDiagnosticsAsync):
                    DiagnosticsCalls++;
                    return Task.FromResult(Diagnostics());

                default:
                    // 其余方法本用例不该碰。返回 null 会让生产代码吃 NRE
                    // （mock 违约），所以直接炸掉：谁多调了一条立刻看得见。
                    throw new NotSupportedException(targetMethod?.Name);
            }
        }

        private SecretProtectionReport Report() =>
            new(_status, string.Equals(_status, "locked", StringComparison.Ordinal));

        /// <summary>
        /// 诊断报告随保护状态现算，映射照抄后端 `secret_protection_diagnostic_item`
        /// （`core/src/commands.rs`，按函数名找而非行号——行号已过期过两次）：
        /// encrypted/managed ⇒ healthy、unprotected ⇒ degraded、locked ⇒ unavailable。
        /// </summary>
        private BackendDiagnosticsReport Diagnostics()
        {
            var (status, reason) = _status switch
            {
                "encrypted" or "managed" => ("healthy", (string?)null),
                "unprotected" => ("degraded", "diagnostics.secrets.unprotected"),
                _ => ("unavailable", "diagnostics.secrets.locked"),
            };

            return new BackendDiagnosticsReport(
                status,
                new[] { new DiagnosticItem("secrets.protection", status, reason) });
        }
    }
}
