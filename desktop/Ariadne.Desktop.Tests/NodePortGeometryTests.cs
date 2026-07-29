using System.Text.RegularExpressions;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 把 NodePortSpec 的端口坐标常量与**节点模板标记**钉在一起。
///
/// 背景（真实回归）：节点从 200 加宽到 232 时，同一次改动还把标题栏 padding
/// 6,7 → 8,8、执行引脚 14 → 16，但 NodePortSpec 只跟着改了 NodeWidth。
/// 于是连线端点按旧几何算，边比引脚偏了 X 4px / Y 3px。
/// 这些常量无法在运行时从布局反推（连线在布局前就要画），只能靠这类
/// 「常量 vs 标记」一致性测试兜住。
/// </summary>
public sealed class NodePortGeometryTests
{
    private static string NodeTemplateMarkup() =>
        File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));

    private static string ResolveDesktopSource(params string[] parts)
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !Directory.Exists(Path.Combine(dir, "Ariadne.Desktop")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return Path.Combine(new[] { dir!, "Ariadne.Desktop" }.Concat(parts).ToArray());
    }

    [Fact]
    public void ExecPinInset_MatchesTitleBarPaddingAndPinBox()
    {
        // 执行引脚在标题栏内：卡片边框 + 标题栏 padding + 半个引脚
        Assert.Equal(
            NodePortSpec.CardBorderThickness
                + NodePortSpec.TitleBarPadding
                + (NodePortSpec.ExecPinBox / 2.0),
            NodePortSpec.PinInsetX);

        var (x, y) = NodePortSpec.LocalCenter(NodePortKind.Control, NodePortDirection.In);
        Assert.Equal(17, x);
        Assert.Equal(28, y);

        var (outX, _) = NodePortSpec.LocalCenter(NodePortKind.Control, NodePortDirection.Out);
        Assert.Equal(NodePortSpec.NodeWidth - 17, outX);
    }

    [Fact]
    public void DataPinInset_DiffersFromExecPin_BecauseContentPaddingDiffers()
    {
        // 数据引脚在内容栏（padding 6，pin 14），执行引脚在标题栏（padding 8，pin 16）。
        // 两者内缩不同，共用一个常量就会让其中一类边偏移。
        Assert.NotEqual(NodePortSpec.PinInsetX, NodePortSpec.DataPinInsetX);
        Assert.Equal(14, NodePortSpec.DataPinInsetX);

        var (x, _) = NodePortSpec.LocalCenter(NodePortKind.Data, NodePortDirection.In);
        Assert.Equal(NodePortSpec.DataPinInsetX, x);

        var (outX, _) = NodePortSpec.LocalCenter(NodePortKind.Data, NodePortDirection.Out);
        Assert.Equal(NodePortSpec.NodeWidth - NodePortSpec.DataPinInsetX, outX);
    }

    [Fact]
    public void MultiDataIn_StepsByPinBoxPlusSpacing()
    {
        var first = NodePortSpec.LocalCenterForHandle("input");
        var second = NodePortSpec.LocalCenterForHandle("data-in-1");
        var third = NodePortSpec.LocalCenterForHandle("data-in-2");

        Assert.Equal(NodePortSpec.DataPortSpacing, second.Y - first.Y);
        Assert.Equal(NodePortSpec.DataPortSpacing, third.Y - second.Y);
        Assert.Equal(first.X, second.X);
    }

    [Fact]
    public void NodeTemplate_UsesSameWidthAndPaddingsAsPortSpec()
    {
        var markup = NodeTemplateMarkup();

        // 节点外框宽必须与 NodeWidth 一致
        Assert.Contains($"<Grid Width=\"{NodePortSpec.NodeWidth}\"", markup, StringComparison.Ordinal);

        // 标题栏 padding：模板写 "8,8"，常量 TitleBarPadding=8
        Assert.Equal(8, NodePortSpec.TitleBarPadding);
        Assert.Contains($"Padding=\"{NodePortSpec.TitleBarPadding},{NodePortSpec.TitleBarPadding}\"",
            markup, StringComparison.Ordinal);

        // 内容栏 padding：模板写 "6,8" = ContentBarPaddingX,ContentBarPaddingY
        Assert.Contains(
            $"Padding=\"{NodePortSpec.ContentBarPaddingX},{NodePortSpec.ContentBarPaddingY}\"",
            markup,
            StringComparison.Ordinal);

        // 通信行高 = CardTopOffset
        Assert.Contains($"<RowDefinition Height=\"{NodePortSpec.CardTopOffset}\" />",
            markup, StringComparison.Ordinal);
    }

    [Fact]
    public void TitleRow_HasFixedHeight_SoStartAndPlainNodesAgree()
    {
        var markup = NodeTemplateMarkup();

        // 起始节点标题栏里有「运行」按钮（20px 高），普通节点没有。
        // 若标题行高度不固定，两类节点标题栏差 4px，数据口 Y 无法用单常量表达。
        Assert.Equal(20, NodePortSpec.TitleRowHeight);
        Assert.Matches(
            new Regex($@"ColumnDefinitions=""Auto,\*,Auto,Auto""[\s\S]{{0,200}}Height=""{NodePortSpec.TitleRowHeight}"""),
            markup);

        Assert.Equal(
            (NodePortSpec.TitleBarPadding * 2) + NodePortSpec.TitleRowHeight,
            NodePortSpec.TitleBarHeight);
    }

    [Fact]
    public void CommunicationPort_SitsAtTopCenter()
    {
        var (x, y) = NodePortSpec.LocalCenter(NodePortKind.Communication, NodePortDirection.In);

        Assert.Equal(NodePortSpec.NodeWidth / 2.0, x);
        Assert.Equal(NodePortSpec.CommPortY, y);
    }
}
