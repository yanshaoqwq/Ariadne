using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
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
    /// 现在是红的：`IAriadneBackendClient` 上既没有设主密码、
    /// 也没有接受明文的方法，所以任何界面都不可能触发它们。
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

        Assert.True(
            hasMasterPassword || hasAllowUnprotected,
            "`IAriadneBackendClient` 没有暴露任何凭据保护补救能力"
            + "（设本地主密码 / 显式接受明文），"
            + "而后端两个命令都在（ipc.rs:924、:931），"
            + "且诊断文案 `diagnostics.secrets.locked` 正在让用户「配置本地主密码」。"
            + "⇒ 用户被指向一个界面上做不到的操作（U176）。");
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
    }

    /// <summary>
    /// 这条**现在就该绿**：按钮文案还没建 key，是上面两条的连带工作量。
    ///
    /// 它的作用是**记录施工清单**——修 U176 时需要新建这些 key（三份语言包），
    /// 而不是复用诊断那几条描述性文案（那些是「问题说明」，不是「按钮标签」）。
    /// 一旦有人建了 key，这条会转红，提醒把上面两条一起做完。
    /// </summary>
    [Fact]
    public void RecoveryButtonCopy_IsStillMissing_AndThatIsPartOfTheSameFix()
    {
        var names = DisplayNameService.LoadDefault();

        var candidateKeys = new[]
        {
            "ui.settings.secrets.set_master_password",
            "ui.settings.secrets.allow_plaintext",
        };

        var existing = candidateKeys
            .Where(key => names.Text(key) != $"[{key}]")
            .ToList();

        Assert.True(
            existing.Count == 0,
            "凭据保护的按钮文案已经建了：" + string.Join(", ", existing)
            + "。若入口也已接好，请删掉本用例；"
            + "若只建了文案没接入口，那是半成品（U176）。");
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
}
