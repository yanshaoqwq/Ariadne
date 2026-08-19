using System;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U182-F：「疑似未完成的操作」面板的三条出路，权重与顺序必须让普通作者先看到 retry。
///
/// **报告原判被证伪的部分**：U182-F 说「手填 JSON 是唯一出路」，并建议
/// 「新加一条放弃本次调用并重跑的出路」——那条出路（`retry`）**早就存在且已接线**，
/// 照原方案施工等于重复造已有机制。真实缺陷降级为纯信息架构：
///
/// - 72px 高的 JSON 大框排在三颗按钮**之上**，把注意力钉在最难的路上；
/// - 三颗按钮权重**恰好反了**：普通作者唯一该点的 retry 是 `secondary`，
///   而需要服务端日志才能用的 use_response 反倒是 `primary`。
///
/// **为什么判据取「顺序 + Classes」而不是「按钮存在」**：三颗按钮在缺陷版本里
/// 一个不少，断言存在性会全绿。这一条守的是**权重与先后**，那才是本次改动的内容。
/// （同型教训见 U152/U181-C：只断言类名字符串存在，样式死掉照样绿。）
/// </summary>
public sealed class InDoubtPanelAffordanceTests
{
    private static string ResolveViewsDirectory()
    {
        // 与本项目其它 XAML 守卫同法：从测试输出目录向上找到 Ariadne.slnx 再定位源码。
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return Path.Combine(dir!, "Ariadne.Desktop", "Views");
    }

    private static string ReadWorkspaceViewWithoutComments()
    {
        var path = Path.Combine(ResolveViewsDirectory(), "WorkspacePageView.axaml");
        var xaml = File.ReadAllText(path);

        // 必须剥注释：本文件的注释里就写着这三个命令名与 Classes 值（解释为什么这么排），
        // 不剥会让「注释里提到过」被当成「界面上真的这么排」而假绿。
        // 这一步是项目里反复踩过的坑（守卫扫 XAML 前一律先剥注释）。
        return Regex.Replace(xaml, "<!--.*?-->", string.Empty, RegexOptions.Singleline);
    }

    [Fact]
    public void RetryIsPrimaryAndComesBeforeTheJsonAdvancedPath()
    {
        var xaml = ReadWorkspaceViewWithoutComments();

        var retry = xaml.IndexOf("RetryInDoubtOperationCommand", StringComparison.Ordinal);
        var useResponse = xaml.IndexOf("UseInDoubtResponseCommand", StringComparison.Ordinal);
        var jsonBox = xaml.IndexOf("InDoubtResponseJson", StringComparison.Ordinal);

        // 自检：三个锚点都必须找到，否则下面的顺序断言在「面板整个被删掉」时会假绿
        // （-1 < -1 之类的比较仍可能成立）。
        Assert.True(retry > 0, "找不到 RetryInDoubtOperationCommand，面板可能被移走了");
        Assert.True(useResponse > 0, "找不到 UseInDoubtResponseCommand");
        Assert.True(jsonBox > 0, "找不到 InDoubtResponseJson");

        // 核心判据一：retry 必须排在 JSON 框和 use_response 之前。
        Assert.True(
            retry < jsonBox,
            $"「安全重试」必须排在 JSON 输入框之前（retry={retry}, jsonBox={jsonBox}）");
        Assert.True(
            retry < useResponse,
            $"「安全重试」必须排在「使用已有响应」之前（retry={retry}, useResponse={useResponse}）");
    }

    [Fact]
    public void RetryCarriesPrimaryWeightAndUseResponseDoesNot()
    {
        var xaml = ReadWorkspaceViewWithoutComments();

        // 核心判据二：权重。取「按钮元素起始 → 命令名」这一段里的 Classes。
        // 用 Singleline 是因为 Button 的属性跨行写。
        var retryButton = Regex.Match(
            xaml,
            @"<Button\s+Classes=""(?<cls>[^""]*)""[^>]*?RetryInDoubtOperationCommand",
            RegexOptions.Singleline);
        var useResponseButton = Regex.Match(
            xaml,
            @"<Button\s+Classes=""(?<cls>[^""]*)""[^>]*?UseInDoubtResponseCommand",
            RegexOptions.Singleline);

        Assert.True(retryButton.Success, "没匹配到 retry 按钮的 Classes");
        Assert.True(useResponseButton.Success, "没匹配到 use_response 按钮的 Classes");

        Assert.Contains("primary", retryButton.Groups["cls"].Value);
        Assert.DoesNotContain(
            "primary",
            useResponseButton.Groups["cls"].Value);
    }

    [Fact]
    public void JsonPathIsCollapsedBehindAnExpander()
    {
        var xaml = ReadWorkspaceViewWithoutComments();

        // 核心判据三：JSON 那条要收进 Expander（高级选项），而不是常驻占版面。
        // 判据取「JSON 框之前最近的 Expander 开标签，且其后没有先出现 </Expander>」——
        // 只断言「文件里有 Expander」不够，那在 Expander 放在别处时也成立。
        var jsonBox = xaml.IndexOf("InDoubtResponseJson", StringComparison.Ordinal);
        Assert.True(jsonBox > 0);

        var before = xaml[..jsonBox];
        var lastOpen = before.LastIndexOf("<Expander", StringComparison.Ordinal);
        var lastClose = before.LastIndexOf("</Expander>", StringComparison.Ordinal);

        Assert.True(
            lastOpen > lastClose,
            "InDoubtResponseJson 必须在一个未闭合的 <Expander> 内部（即被折叠为高级选项）");
    }

    [Fact]
    public void StopStaysDangerAndLast()
    {
        var xaml = ReadWorkspaceViewWithoutComments();

        // 停止是破坏性动作：保持 danger 权重，且排在最后。
        // 这一条不是本次改动的目标，是**防止重排时把它顺手挪到前面**的护栏。
        var stopButton = Regex.Match(
            xaml,
            @"<Button\s+Classes=""(?<cls>[^""]*)""[^>]*?StopInDoubtOperationCommand",
            RegexOptions.Singleline);
        Assert.True(stopButton.Success, "没匹配到 stop 按钮的 Classes");
        Assert.Contains("danger", stopButton.Groups["cls"].Value);

        var stop = xaml.IndexOf("StopInDoubtOperationCommand", StringComparison.Ordinal);
        var retry = xaml.IndexOf("RetryInDoubtOperationCommand", StringComparison.Ordinal);
        var useResponse = xaml.IndexOf("UseInDoubtResponseCommand", StringComparison.Ordinal);
        Assert.True(stop > retry && stop > useResponse, "「停止本次运行」应排在另两条出路之后");
    }

    [Fact]
    public void AllThreeDecisionsStayWired()
    {
        var xaml = ReadWorkspaceViewWithoutComments();

        // 这一条与上面互补：上面几条保证「排得对」，这条保证**一个都没在重排中丢掉**。
        // 若有人据「retry 优先」把 use_response 整个删了，上面的权重用例会红，
        // 但顺序用例可能因为锚点消失而以别的方式失败——判据说不清。这条说得清。
        Assert.Contains("RetryInDoubtOperationCommand", xaml);
        Assert.Contains("UseInDoubtResponseCommand", xaml);
        Assert.Contains("StopInDoubtOperationCommand", xaml);
    }
}
