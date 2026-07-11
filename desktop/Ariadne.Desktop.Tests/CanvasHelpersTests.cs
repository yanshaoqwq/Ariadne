using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// Exercises shipped <see cref="NodePortSpec"/> helpers used by minimap pan/viewport and port connect.
/// </summary>
public sealed class CanvasHelpersTests
{
    [Fact]
    public void LogicalViewportToMiniMap_BottomRightOrigin_DoesNotThrowAndStaysInBounds()
    {
        // Viewport origin near bottom-right of logical space → minimap x/y leave little room.
        // Pre-fix: Math.Clamp(w, 8, maxW) threw when maxW < 8 (ArgumentException).
        var (x, y, w, h) = NodePortSpec.LogicalViewportToMiniMap(
            logicalLeft: 1400,
            logicalTop: 900,
            logicalWidth: 800,
            logicalHeight: 600);

        Assert.InRange(x, 0, NodePortSpec.MiniMapContentWidth);
        Assert.InRange(y, 0, NodePortSpec.MiniMapContentHeight);
        Assert.True(w >= 0);
        Assert.True(h >= 0);
        Assert.True(x + w <= NodePortSpec.MiniMapContentWidth + 1e-9);
        Assert.True(y + h <= NodePortSpec.MiniMapContentHeight + 1e-9);
    }

    [Fact]
    public void LogicalViewportToMiniMap_ExactContentEdge_DoesNotThrow()
    {
        // x maps to MiniMapContentWidth so maxW == 0.
        var result = NodePortSpec.LogicalViewportToMiniMap(
            logicalLeft: NodePortSpec.MiniMapContentWidth / NodePortSpec.MiniMapScale,
            logicalTop: NodePortSpec.MiniMapContentHeight / NodePortSpec.MiniMapScale,
            logicalWidth: 100,
            logicalHeight: 100);

        Assert.Equal(NodePortSpec.MiniMapContentWidth, result.X);
        Assert.Equal(NodePortSpec.MiniMapContentHeight, result.Y);
        Assert.Equal(0, result.Width);
        Assert.Equal(0, result.Height);
    }

    [Fact]
    public void LogicalViewportToMiniMap_NormalViewport_HasMinimumFrameWhenRoomAllows()
    {
        var (x, y, w, h) = NodePortSpec.LogicalViewportToMiniMap(
            logicalLeft: 0,
            logicalTop: 0,
            logicalWidth: 10, // raw 1.0 after scale — below preferred min 8
            logicalHeight: 10);

        Assert.Equal(0, x);
        Assert.Equal(0, y);
        Assert.Equal(8, w);
        Assert.Equal(6, h);
    }

    [Fact]
    public void MiniMapToLogical_RoundTripsWithScale()
    {
        var (lx, ly) = NodePortSpec.MiniMapToLogical(14, 8.4);
        Assert.Equal(140, lx, 6);
        Assert.Equal(84, ly, 6);
    }

    [Theory]
    [InlineData(NodePortKind.Data, NodePortDirection.Out, NodePortKind.Data, NodePortDirection.In, true)]
    [InlineData(NodePortKind.Control, NodePortDirection.Out, NodePortKind.Control, NodePortDirection.In, true)]
    [InlineData(NodePortKind.Communication, NodePortDirection.Both, NodePortKind.Communication, NodePortDirection.Both, true)]
    [InlineData(NodePortKind.Data, NodePortDirection.Out, NodePortKind.Control, NodePortDirection.In, false)]
    [InlineData(NodePortKind.Data, NodePortDirection.Out, NodePortKind.Data, NodePortDirection.Out, false)]
    public void TryNormalizeConnection_MatchesConnectTypeRules(
        NodePortKind aKind, NodePortDirection aDir,
        NodePortKind bKind, NodePortDirection bDir,
        bool expected)
    {
        var ok = NodePortSpec.TryNormalizeConnection(
            "n1", aKind, aDir,
            "n2", bKind, bDir,
            out var from, out var to, out _, out _, out var edgeKind);

        Assert.Equal(expected, ok);
        if (expected)
        {
            Assert.False(string.IsNullOrEmpty(from));
            Assert.False(string.IsNullOrEmpty(to));
            Assert.Equal(NodePortSpec.EdgeKindName(aKind), edgeKind);
        }
    }

    [Fact]
    public void LocalCenter_ExecPins_SitInsideTitleRowSides()
    {
        var (inx, iny) = NodePortSpec.LocalCenter(NodePortKind.Control, NodePortDirection.In);
        var (outx, outy) = NodePortSpec.LocalCenter(NodePortKind.Control, NodePortDirection.Out);

        Assert.Equal(NodePortSpec.PinInsetX, inx, 6);
        Assert.Equal(NodePortSpec.NodeWidth - NodePortSpec.PinInsetX, outx, 6);
        Assert.Equal(NodePortSpec.ExecPortY, iny, 6);
        Assert.Equal(NodePortSpec.ExecPortY, outy, 6);
        // 执行口在卡片内（X 内缩），且 Y 高于数据口
        Assert.True(inx > 0 && inx < NodePortSpec.NodeWidth / 2);
        Assert.True(NodePortSpec.ExecPortY < NodePortSpec.DataPortY);
        Assert.True(NodePortSpec.ExecPortY > NodePortSpec.CardTopOffset);
    }

    [Fact]
    public void LocalCenter_DataPins_SitInsideContentBarSides()
    {
        var (inx, iny) = NodePortSpec.LocalCenter(NodePortKind.Data, NodePortDirection.In);
        var (outx, outy) = NodePortSpec.LocalCenter(NodePortKind.Data, NodePortDirection.Out);

        Assert.Equal(NodePortSpec.PinInsetX, inx, 6);
        Assert.Equal(NodePortSpec.NodeWidth - NodePortSpec.PinInsetX, outx, 6);
        Assert.Equal(NodePortSpec.DataPortY, iny, 6);
        Assert.Equal(NodePortSpec.DataPortY, outy, 6);
        Assert.True(NodePortSpec.DataPortY > NodePortSpec.CardTopOffset + NodePortSpec.TitleBarHeight);
    }

    [Fact]
    public void LocalCenter_Communication_IsTopCenter()
    {
        var (x, y) = NodePortSpec.LocalCenter(NodePortKind.Communication, NodePortDirection.Both);
        Assert.Equal(NodePortSpec.NodeWidth / 2.0, x, 6);
        Assert.Equal(NodePortSpec.CommPortY, y, 6);
    }

    [Fact]
    public void UpdateEdgePath_IsOpenBezierOnly_NoClosingStraightSegment()
    {
        // 用真实 ViewModel 路径生成：必须只有一段贝塞尔，且 IsClosed=false（避免「曲线+直线」）
        var edge = new WorkflowEdgeViewModel(
            new Ariadne.Desktop.Backend.CanvasEdge(
                Id: "e1",
                Source: "a",
                Target: "b",
                SourceHandle: "output",
                TargetHandle: "input",
                Kind: "data",
                Label: null,
                Data: null),
            displayNames: Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            select: _ => { },
            markDirty: () => { });

        edge.UpdateEdgePath(sourceX: 0, sourceY: 0, targetX: 300, targetY: 40);
        var path = Assert.IsType<Avalonia.Media.PathGeometry>(edge.EdgePath);
        Assert.NotNull(path.Figures);
        Assert.Single(path.Figures!);
        var figure = path.Figures![0];
        Assert.False(figure.IsClosed);
        Assert.NotNull(figure.Segments);
        Assert.Single(figure.Segments!);
        Assert.IsType<Avalonia.Media.BezierSegment>(figure.Segments![0]);

        // 端点必须落在引脚中心，而非节点角点 (0,0)/(300,0)
        var (sx, sy) = NodePortSpec.LocalCenter(NodePortKind.Data, NodePortDirection.Out);
        var (tx, ty) = NodePortSpec.LocalCenter(NodePortKind.Data, NodePortDirection.In);
        Assert.Equal(new Avalonia.Point(0 + sx, 0 + sy), figure.StartPoint);
        var bezier = Assert.IsType<Avalonia.Media.BezierSegment>(figure.Segments[0]);
        Assert.Equal(new Avalonia.Point(300 + tx, 40 + ty), bezier.Point3);
    }

    [Fact]
    public void BuildCommunicationJumpPath_ArchesAboveEndpoints()
    {
        // 开口向下二次感：中点 Y 必须小于两端（更靠上），形成「从上面跳过去」
        var spec = NodePortSpec.BuildCommunicationJumpPath(100, 80, 300, 90);
        Assert.Equal(100, spec.Start.X, 6);
        Assert.Equal(80, spec.Start.Y, 6);
        Assert.Equal(300, spec.End.X, 6);
        Assert.Equal(90, spec.End.Y, 6);

        var mid = spec.Midpoint;
        Assert.True(mid.Y < Math.Min(spec.Start.Y, spec.End.Y) - 20,
            $"mid.Y={mid.Y} should jump above ends ({spec.Start.Y}, {spec.End.Y})");
        Assert.NotNull(spec.PeakY);
        Assert.True(spec.PeakY < Math.Min(spec.Start.Y, spec.End.Y));
        // 中点 X 大致在两端之间
        Assert.InRange(mid.X, 100, 300);
    }

    [Fact]
    public void BuildEdgePath_CommunicationUsesJump_DataUsesHorizontalS()
    {
        var jump = NodePortSpec.BuildEdgePath(0, 50, 200, 50, isCommunication: true);
        var data = NodePortSpec.BuildEdgePath(0, 50, 200, 50, isCommunication: false);

        Assert.True(jump.Midpoint.Y < 50 - 20);
        // 数据边水平 S：控制点 Y 贴端点，中点不应大幅上拱
        Assert.True(Math.Abs(data.Midpoint.Y - 50) < 8);
        Assert.True(data.Control1.Y == 50 && data.Control2.Y == 50);
    }
}
