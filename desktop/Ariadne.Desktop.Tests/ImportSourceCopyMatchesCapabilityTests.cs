using System.Text.Json;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U179-A：功能放行了项目外路径，界面文案还在劝退。
///
/// **这类缺陷比功能坏更隐蔽**：没有报错、没有失败，用户只是**不会去用**。
/// 作者要从下载目录 / U 盘 / 别的写作软件的导出目录挑稿子导入，
/// 而输入框写着「项目内路径，例如 planning/imports/ch1.md」，
/// 于是他认为「必须先手工拷进项目」——这恰恰是 `WorksImportHelper.cs:316`
/// 的注释明确要否定的事（放宽了「位置」这一维）。
///
/// 成因是 U163-B 那一轮**只改了校验逻辑、没改文案**。
///
/// ## 判据为什么不取「文案里没有『项目内』字样」
///
/// 那种判据用改错字也能通过（把「项目内」写成「项目里」就绿了），
/// 而真正要守的是**示例路径本身的形态**：placeholder 里举的例子若还是
/// `planning/imports/ch1.md`，换了描述文字也等于没改——例子本身就在
/// 传达「得放项目里」。所以判据取「示例是项目外形态」（以 ~ 或 / 开头）,
/// 它和功能语义绑在一起，改错字改不出来。
/// </summary>
public sealed class ImportSourceCopyMatchesCapabilityTests
{
    /// <summary>
    /// 导入**源**的 placeholder 必须举一个项目外的例子。
    /// </summary>
    [Fact]
    public void ImportSourcePlaceholder_ShowsAnOutsideProjectExample()
    {
        var text = DisplayName("ui.works.import.source_placeholder");

        // 判据一：示例路径是项目外形态。~ 是家目录、/ 是绝对路径，
        // 两者都无法被解读成「项目内的相对路径」。
        Assert.True(
            text.Contains('~') || text.Contains(" /"),
            $"导入源 placeholder 没有举项目外的例子，作者会以为必须先把稿子拷进项目：\n  {text}\n"
            + "代码早已放行项目外源（WorksImportHelper.cs:189 → ValidateOutsideProjectSource），"
            + "文案却还在劝退（U179-A，是 U163-B 只改校验没改文案的遗漏）。");

        // 判据二：不能再拿项目内相对路径当例子。
        // 单独列一条是因为它和判据一可以同时不满足也可以只坏一个——
        // 有人可能加了 ~ 的例子却把旧例子也留着，那样一样有误导。
        Assert.DoesNotContain("planning/imports", text, StringComparison.Ordinal);
    }

    /// <summary>
    /// 反向：导入**落点**的文案必须继续说「项目内」。
    ///
    /// 源与落点是**两套约束**：`target_path` 仍必须在项目内
    /// （`commands.rs:154` 的 `project_path_buf` + `ensure_path_under_root`）。
    /// 这条防的是「一起放宽」——把真实的安全边界当成文案问题一并改掉，
    /// 那会让用户以为可以往项目外写，然后撞上后端拒绝。
    /// </summary>
    [Fact]
    public void ImportTargetPlaceholder_StillSaysInsideProject()
    {
        var text = DisplayName("ui.works.import.target_placeholder");
        Assert.Contains("项目内", text, StringComparison.Ordinal);
    }

    /// <summary>
    /// U179-B：控制类端口提示不得泄漏后端内部标识符。
    ///
    /// **关键不在「有英文」，而在同一组文案里不一致**：
    /// `data_in` / `data_out` / `communication` 三个都干净，
    /// 只有 control 那 4 个带 `（exec_in）` 这种括号标识符 ⇒ 是疏漏而非风格。
    /// `exec_in` 是后端 `PortValue` 的内部名，对写小说的人是纯噪音。
    /// </summary>
    [Theory]
    [InlineData("ui.workspace.port.control_in")]
    [InlineData("ui.workspace.port.control_out")]
    [InlineData("ui.workspace.port.control_out_true")]
    [InlineData("ui.workspace.port.control_out_false")]
    public void ControlPortTips_DoNotLeakBackendIdentifiers(string key)
    {
        var text = DisplayName(key);
        Assert.DoesNotContain("exec_", text, StringComparison.Ordinal);
    }

    /// <summary>
    /// 同组对照：数据口与通信口本来就干净，钉住它们不被「统一风格」改坏。
    ///
    /// 这条不是重复上一条。它是**基准线**：若哪天有人为了「一致」
    /// 给全部端口都补上标识符，上一条会红、这一条也会红，
    /// 两条一起红才说清了「往哪个方向统一」。
    /// </summary>
    [Theory]
    [InlineData("ui.workspace.port.data_in")]
    [InlineData("ui.workspace.port.data_out")]
    [InlineData("ui.workspace.port.communication")]
    public void DataAndCommunicationPortTips_StayClean(string key)
    {
        Assert.DoesNotContain("exec_", DisplayName(key), StringComparison.Ordinal);
    }

    /// <summary>
    /// 直接读 zh 文案文件，不经 <c>DisplayNameService</c>。
    ///
    /// 刻意绕开服务层：服务在缺键时返回 <c>[key]</c>，
    /// 那会让「键被删了」表现成「文案内容不含某字符串」而误判为通过。
    /// 这里键不存在就是 <c>KeyNotFoundException</c>，当场炸。
    /// </summary>
    private static string DisplayName(string key)
    {
        var path = Path.Combine(ResolveRepoRoot(), "core", "resources", "display_name.json");
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        return document.RootElement.GetProperty(key).GetString()
            ?? throw new InvalidOperationException($"{key} 的值是 null");
    }

    private static string ResolveRepoRoot()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null
            && !File.Exists(Path.Combine(dir, "core", "resources", "display_name.json")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return dir!;
    }
}
