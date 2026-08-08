using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U118：凭据保护状态必须在诊断面板上说得清、且指向一个**产品里真实存在**的补救动作。
///
/// 原缺陷有两层，这里各钉一条：
/// (1) 后端 `secrets.protection` 这个组件名在前端无映射，会落到「其它运行组件」——
///     用户看到一条不知所指的告警；
/// (2) 补救文案会落到 runtime 那条兜底「重新启动应用」，而重启对明文/锁定状态
///     毫无作用。真正的出路是设主密码或显式接受明文，两者都在设置页内。
/// </summary>
public sealed class SecretProtectionDiagnosticsTests
{
    /// <summary>明文保存必须报「需要注意」，并给出设置页内的补救动作。</summary>
    [Fact]
    public void UnprotectedSecrets_AreNamedAndPointAtAnActionThatExists()
    {
        var names = DisplayNameService.LoadDefault();
        var vm = new SettingsPageViewModel(names, NoopBackend.Create());
        vm.ApplyDiagnosticsForTests(new BackendDiagnosticsReport(
            "degraded",
            new[]
            {
                new DiagnosticItem(
                    "secrets.protection",
                    "degraded",
                    "diagnostics.secrets.unprotected"),
            }));

        var item = Assert.Single(vm.DiagnosticsItems);
        Assert.Equal(
            names.Text("ui.settings.misc.diagnostics.component.secrets_protection"),
            item.Component);
        Assert.Equal(names.Text("diagnostics.secrets.unprotected"), item.Reason);
        Assert.Equal(
            names.Text("ui.settings.misc.diagnostics.recovery.secrets"),
            item.RecoveryAction);

        // 兜底文案是「重新启动应用」——对明文保存毫无作用，落到它就是把用户
        // 指向一个无效操作，正是 U118 报告点名的那类缺陷。
        Assert.NotEqual(
            names.Text("ui.settings.misc.diagnostics.recovery.runtime"),
            item.RecoveryAction);
        Assert.NotEqual(
            names.Text("ui.settings.misc.diagnostics.component.other"),
            item.Component);
    }

    /// <summary>锁定状态必须报「不可用」——该状态下保存密钥必定失败，是真实阻断。</summary>
    [Fact]
    public void LockedSecrets_ReportUnavailableNotMerelyDegraded()
    {
        var names = DisplayNameService.LoadDefault();
        var vm = new SettingsPageViewModel(names, NoopBackend.Create());
        vm.ApplyDiagnosticsForTests(new BackendDiagnosticsReport(
            "unavailable",
            new[]
            {
                new DiagnosticItem(
                    "secrets.protection",
                    "unavailable",
                    "diagnostics.secrets.locked"),
            }));

        var item = Assert.Single(vm.DiagnosticsItems);
        Assert.Equal(
            names.Text("ui.settings.misc.diagnostics.status.unavailable"),
            item.Status);
        Assert.Equal(names.Text("diagnostics.secrets.locked"), item.Reason);
    }

    /// <summary>
    /// 三种语言都必须有这两条 reason 文案。
    ///
    /// 缺 key 时 `DisplayNameService` 返回 `[key]`，UI 上就是一行方括号乱码；
    /// 而中/日用户恰是最需要读懂「你的 API Key 正躺在磁盘上」这句话的人。
    /// </summary>
    [Theory]
    [InlineData("zh")]
    [InlineData("en")]
    [InlineData("ja")]
    public void SecretDiagnosticsText_ExistsInEveryLanguage(string language)
    {
        var names = DisplayNameService.LoadDefault();
        names.SwitchLanguage(language);

        foreach (var key in new[]
                 {
                     "diagnostics.secrets.unprotected",
                     "diagnostics.secrets.locked",
                     "ui.settings.misc.diagnostics.component.secrets_protection",
                     "ui.settings.misc.diagnostics.recovery.secrets",
                 })
        {
            var text = names.Text(key);
            Assert.False(
                text == $"[{key}]",
                $"{language} 缺少文案 {key}——UI 会显示方括号乱码");
            Assert.False(string.IsNullOrWhiteSpace(text));
        }
    }

    /// 诊断标签是纯本地投影，不触碰后端；stub 保证任何真实调用都会立刻暴露。
    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args) =>
            throw new NotSupportedException(targetMethod?.Name);
    }
}
