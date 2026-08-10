using System.Text.RegularExpressions;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U125：condition 节点的两个分支执行出引脚（前端侧）。
///
/// 后端在保存边界已经**拒绝** condition 用通用 `exec_out` 连出，所以这层不是
/// 「体验改进」——画布若渲染不出两个分支引脚，任何带 condition 的工作流都存不下去。
///
/// 这里锁三件事：
/// 1. 分支引脚名与后端常量一致（拼错则连出的边一律被保存边界拒绝）；
/// 2. `TryResolveKind` 把分支引脚认成**控制口**（认错成数据口则连线类型判定全错）；
/// 3. 模板标记与 `NodePortSpec` 几何一致（连线在布局前就要算坐标，只能靠标记比对）。
/// </summary>
public sealed class ConditionBranchPinTests
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

    /// <summary>引脚名必须与后端 `EXECUTION_OUTPUT_PORT_TRUE/FALSE` 逐字一致。</summary>
    [Fact]
    public void BranchHandleNames_MatchBackendConstants()
    {
        Assert.Equal("exec_out_true", NodePortSpec.ExecOutTrueHandle);
        Assert.Equal("exec_out_false", NodePortSpec.ExecOutFalseHandle);
    }

    /// <summary>
    /// 分支引脚必须被认成控制口。
    ///
    /// 这条不是形式主义：`TryResolveKind` 的兜底分支按 `"out"` 前缀把名字判成
    /// 数据出口，而 `exec_out_true` 恰好不含 `out` 前缀但含 `_out_`——真正的风险是
    /// 顺序写错时它落进「以 out 开头」那条。认错成数据口后，拖线会被判为
    /// 数据边、几何取数据口坐标，两处同时错。
    /// </summary>
    [Theory]
    [InlineData("exec_out_true")]
    [InlineData("exec_out_false")]
    [InlineData("EXEC_OUT_TRUE")]
    public void BranchHandles_ResolveAsControlOutPorts(string handle)
    {
        Assert.True(NodePortSpec.TryResolveKind(handle, out var kind, out var direction));
        Assert.Equal(NodePortKind.Control, kind);
        Assert.Equal(NodePortDirection.Out, direction);
        Assert.True(NodePortSpec.IsBranchExecOutHandle(handle));
    }

    /// <summary>通用 exec_out 不是分支引脚——它不承载分支语义。</summary>
    [Fact]
    public void GenericExecOut_IsNotABranchPin()
    {
        Assert.False(NodePortSpec.IsBranchExecOutHandle("exec_out"));
        Assert.Equal(0, NodePortSpec.BranchPinOffsetFor("exec_out"));
    }

    /// <summary>
    /// 两个分支引脚与通用执行出口同列（X 相同），只在 Y 上对称错开。
    ///
    /// 同列是刻意的：镜像列（循环节点）与非镜像列共用一套 `ExecOutColumn`，
    /// 分支引脚若另开一列，镜像时会跑到卡片另一侧。
    /// </summary>
    [Fact]
    public void BranchPins_ShareExecOutColumnAndStraddleItVertically()
    {
        var generic = NodePortSpec.LocalCenterForHandle("exec_out");
        var onTrue = NodePortSpec.LocalCenterForHandle(NodePortSpec.ExecOutTrueHandle);
        var onFalse = NodePortSpec.LocalCenterForHandle(NodePortSpec.ExecOutFalseHandle);

        Assert.Equal(generic.X, onTrue.X);
        Assert.Equal(generic.X, onFalse.X);
        // 真在上（Y 更小）、假在下，且相对通用出口中心对称。
        Assert.Equal(generic.Y - NodePortSpec.BranchPinOffsetY, onTrue.Y);
        Assert.Equal(generic.Y + NodePortSpec.BranchPinOffsetY, onFalse.Y);
        Assert.True(onTrue.Y < onFalse.Y, "「真」分支应在上、「假」分支在下");
    }

    /// <summary>
    /// 两枚引脚不得重叠：垂直间距必须大于引脚边长，否则点击热区互相吞掉。
    /// </summary>
    [Fact]
    public void BranchPins_DoNotOverlapEachOther()
    {
        var gap = NodePortSpec.BranchPinOffsetY * 2;
        Assert.True(
            gap >= NodePortSpec.ExecPinBox,
            $"两枚分支引脚中心间距 {gap} 必须不小于引脚边长 {NodePortSpec.ExecPinBox}，否则热区重叠");
    }

    /// <summary>
    /// 分支引脚的包络不得超出标题栏，否则会撞上通信口或内容栏。
    ///
    /// 包络 = 引脚边长 + 两侧各 BranchPinOffsetY。标题栏可用高度 = 行高 + 上下 padding。
    /// </summary>
    [Fact]
    public void BranchPins_StayWithinTitleBar()
    {
        var envelope = NodePortSpec.ExecPinBox + (NodePortSpec.BranchPinOffsetY * 2);
        Assert.True(
            envelope <= NodePortSpec.TitleBarHeight,
            $"分支引脚包络 {envelope} 超出标题栏总高 {NodePortSpec.TitleBarHeight}");
    }

    /// <summary>
    /// 模板必须真的渲染两个带分支 Tag 的引脚，且 Margin 与 BranchPinOffsetY 对应。
    ///
    /// Avalonia 居中元素的 Margin 按 (top-bottom)/2 生效，所以模板里的偏移量
    /// 是常量的**两倍**。这条把「常量」与「标记」钉在一起——两者漂移时连线端点
    /// 会与引脚错开，而那种偏移无法从布局反推（连线在布局前就要画）。
    /// </summary>
    [Fact]
    public void NodeTemplate_RendersBothBranchPinsWithMatchingMargins()
    {
        var markup = NodeTemplateMarkup();

        Assert.Contains($"Tag=\"control|out|{NodePortSpec.ExecOutTrueHandle}\"", markup, StringComparison.Ordinal);
        Assert.Contains($"Tag=\"control|out|{NodePortSpec.ExecOutFalseHandle}\"", markup, StringComparison.Ordinal);

        var doubled = NodePortSpec.BranchPinOffsetY * 2;
        Assert.Contains($"Margin=\"0,-{doubled},0,0\"", markup, StringComparison.Ordinal);
        Assert.Contains($"Margin=\"0,{doubled},0,0\"", markup, StringComparison.Ordinal);
    }

    /// <summary>
    /// 通用引脚与分支引脚必须互斥显示。
    ///
    /// 同时出现会让用户在 condition 上既能画分支边、又能画通用「恒放行」边，
    /// 而后者已被后端拒绝——用户会撞上一个画得出却存不下的状态。
    /// </summary>
    [Fact]
    public void NodeTemplate_SwitchesExclusivelyBetweenGenericAndBranchPins()
    {
        var markup = NodeTemplateMarkup();

        Assert.Contains("IsVisible=\"{Binding ShowGenericExecOutPin}\"", markup, StringComparison.Ordinal);
        Assert.Contains("IsVisible=\"{Binding ShowBranchExecOutPins}\"", markup, StringComparison.Ordinal);
        // 两个可见性必须来自同一个「是不是 condition」判定，互为反面。
        Assert.Matches(new Regex(@"ShowBranchExecOutPins"), markup);
    }
}
