using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U185-A：加载态与「进行中」指示基元的**覆盖面**守卫。
///
/// <para>立项理由是一次实测：U178-F 造出了 <c>BusyDots</c> 基元，但只接了
/// 10 处加载态里的 1 处（作品页摘要区）。剩下 9 处仍是纯静态文字——
/// 包含 Git 提交（耗时最不可预测）、首启最近项目（第一眼观感）、
/// 切章节（正文可能几万字）这三个**等待感最强**的场景。</para>
///
/// <para><b>为什么半接线比完全没做更糟</b>：有一处能看见呼吸点，
/// 会让人（包括下一个施工者）以为加载指示已经统一，从而不再复核其余场景。
/// 所以这份用例守的不是「某处有没有」，而是**覆盖面本身**。</para>
///
/// <para>⚠️ <b>判据刻意不是「BusyDots 在 XAML 里出现 N 次」</b>：那种断言
/// 挪一行注释、加一处装饰性用法都能让它绿，而真正要防的是
/// 「新增了一个加载态、但忘了给它配指示」。所以判据取
/// <b>「每一个加载态可见性绑定，都能在同一段标记里找到驱动同一属性的 BusyDots」</b>——
/// 与绑定的属性名逐一对上，加载态多一个、守卫就多要求一个。</para>
/// </summary>
public sealed class BusyIndicatorCoverageTests
{
    /// <summary>
    /// 认定为「加载态」的属性名形态。<c>Is*Loading</c> / <c>IsBusy</c> 两族，
    /// 与全仓现有命名一致。⚠️ 这里刻意不含 <c>IsRunning</c>：
    /// 工作流的 running 是**长任务状态**（有自己的运行态可视化：流动环、节点高亮），
    /// 不是「等一下就好」的加载态，给它配呼吸点是噪声。
    /// </summary>
    private static readonly Regex BusyStateBinding = new(
        @"IsVisible=""\{Binding (?<prop>Is[A-Za-z]*(?:Loading|Busy)[A-Za-z]*)\}""",
        RegexOptions.Compiled);

    private static string ViewsDir => Path.Combine(ResolveDesktopRoot(), "Views");

    /// <summary>
    /// 从测试程序集所在目录向上找 <c>Ariadne.slnx</c> 定位解决方案根。
    /// 与 <c>ThemeStyleUsageTests</c> 同一约定（该项目里每个类各自实现一份，
    /// 不引第三方辅助类）——沿用而非另造，免得两套路径逻辑漂移。
    /// </summary>
    private static string ResolveDesktopRoot()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return Path.Combine(dir!, "Ariadne.Desktop");
    }

    private static IEnumerable<string> ViewFiles()
        => Directory.EnumerateFiles(ViewsDir, "*.axaml", SearchOption.AllDirectories);

    /// <summary>
    /// 每个加载态绑定，都必须有一个 <c>BusyDots</c> 由**同一个属性**驱动。
    ///
    /// <para>⚠️ 允许「一个属性多处绑定、每处各配一个 BusyDots」，也允许
    /// 同一属性在同文件里出现多次（<c>IsWorksTreeLoading</c> 就有两处：
    /// 大空态面板 + 树栏内窄行）。所以校验的是**属性集合**的包含关系，
    /// 而非出现次数相等——后者会把「同属性两处展示」误判为缺一个。</para>
    /// </summary>
    [Fact]
    public void EveryBusyState_HasABusyDotsDrivenByTheSameProperty()
    {
        var gaps = new List<string>();
        var checkedStates = 0;

        foreach (var file in ViewFiles())
        {
            var markup = File.ReadAllText(file);
            var states = BusyStateBinding.Matches(markup)
                .Select(m => m.Groups["prop"].Value)
                .ToHashSet(StringComparer.Ordinal);
            if (states.Count == 0)
            {
                continue;
            }

            var driven = BusyDotsDrivers(markup);
            foreach (var state in states.OrderBy(s => s, StringComparer.Ordinal))
            {
                checkedStates++;
                if (!driven.Contains(state))
                {
                    gaps.Add($"{Path.GetFileName(file)} 的 {state}");
                }
            }
        }

        // 自检：扫描器若因路径/正则问题一无所获，上面的循环会**空跑通过**。
        // 现已知全仓 10 处加载态，取 8 作下限（留出重构余量，但足以发现「扫了 0 个」）。
        Assert.True(
            checkedStates >= 8,
            $"只扫到 {checkedStates} 处加载态绑定，远少于已知的 10 处——" +
            $"先查 {ViewsDir} 路径与正则是否还匹配当前写法，" +
            "不要以为是加载态变少了（这个用例空跑也会绿）。");

        Assert.True(
            gaps.Count == 0,
            "下列加载态只有静态文字、没有「进行中」指示，用户无法区分「还在跑」与「卡住了」：\n  "
            + string.Join("\n  ", gaps)
            + "\n\n修法：在同一段标记里加 <ctl:BusyDots IsActive=\"{Binding <同名属性>}\" />。"
            + "\nchip 内（Padding 10,3）请一并给 DotDiameter=\"4\"，默认 5px 会顶到胶囊边缘。");
    }

    /// <summary>
    /// 找出这份标记里所有 <c>BusyDots</c> 的 <c>IsActive</c> 绑定源属性。
    ///
    /// <para>⚠️ <b>先剥注释再匹配</b>：文档里会成段引用 XAML 写法
    /// （本轮就有多处注释在解释 BusyDots 该怎么挂），注释里的示例
    /// 会让守卫把「只写在注释里的接线」当成真接线——那正是它要抓的缺陷形态。</para>
    /// </summary>
    private static HashSet<string> BusyDotsDrivers(string markup)
    {
        var withoutComments = Regex.Replace(markup, @"<!--.*?-->", string.Empty, RegexOptions.Singleline);
        return Regex.Matches(
                withoutComments,
                @"<ctl:BusyDots[^>]*?IsActive=""\{Binding (?<prop>[A-Za-z\.]+)\}""",
                RegexOptions.Singleline)
            .Select(m => m.Groups["prop"].Value)
            .ToHashSet(StringComparer.Ordinal);
    }
}
