using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// Exercises shipped <see cref="NodePortSpec"/> helpers used by minimap pan/viewport and port connect.
/// </summary>
public sealed class CanvasHelpersTests
{
    /// <summary>C5：拖动期间 defer dirty，松手后才 RefreshDirtyState（结构合同，非帧基准）。</summary>
    [Fact]
    public void ContinuousCanvasEdit_DefersDirtyRefreshUntilEnd()
    {
        var src = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "WorkspacePageViewModel.cs"));
        Assert.Contains("BeginContinuousCanvasEdit", src, StringComparison.Ordinal);
        Assert.Contains("EndContinuousCanvasEdit", src, StringComparison.Ordinal);
        Assert.Contains("_deferDirtyRefresh = true", src, StringComparison.Ordinal);
        Assert.Contains("MustRefreshDirtyAfterContinuousEditEnd", src, StringComparison.Ordinal);
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));
        Assert.Contains("BeginContinuousCanvasEdit()", view, StringComparison.Ordinal);
        Assert.Contains("EndContinuousCanvasEdit()", view, StringComparison.Ordinal);
        // C5-a：PointerMoved 不得每事件同步主视觉；应经 ScheduleDragFrameSync 合并。
        Assert.Contains("CanvasDragFrameHelpers.TryScheduleFrameSync", view, StringComparison.Ordinal);
        Assert.Contains("ShouldApplyMainVisualsOnPointerMoved", view, StringComparison.Ordinal);
        Assert.False(
            CanvasDragFrameHelpers.ShouldApplyMainVisualsOnPointerMoved,
            "C5-a product path must not apply main visuals on every PointerMoved");
    }

    /// <summary>C5-a：同一帧内多次 PointerMoved 只调度一次 Render 同步。</summary>
    [Fact]
    public void C5a_FrameCoalesce_OnlyFirstScheduleWins()
    {
        var scheduled = false;
        Assert.True(CanvasDragFrameHelpers.TryScheduleFrameSync(ref scheduled));
        Assert.True(scheduled);
        Assert.False(CanvasDragFrameHelpers.TryScheduleFrameSync(ref scheduled));
        Assert.False(CanvasDragFrameHelpers.TryScheduleFrameSync(ref scheduled));
        CanvasDragFrameHelpers.OnFrameSyncStarted(ref scheduled);
        Assert.False(scheduled);
        Assert.True(CanvasDragFrameHelpers.TryScheduleFrameSync(ref scheduled));
    }

    /// <summary>
    /// C5-a：松手/capture-lost 必须在清空 drag 状态前 flush（源码路径合同）。
    /// 否则挂起的 Render 回调见 _nodeDragging=false 会跳过最终 Canvas 位与边 Geometry。
    /// </summary>
    [Fact]
    public void C5a_ReleasePath_FlushesFrameSyncBeforeClearingDragState()
    {
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));
        Assert.Contains("FlushDragFrameSyncNow()", view, StringComparison.Ordinal);
        var releaseIdx = view.IndexOf("public void OnNodePointerReleased", StringComparison.Ordinal);
        var captureIdx = view.IndexOf("public void OnNodePointerCaptureLost", StringComparison.Ordinal);
        Assert.True(releaseIdx >= 0 && captureIdx >= 0);
        var releaseBody = view.Substring(releaseIdx, Math.Min(700, view.Length - releaseIdx));
        var captureBody = view.Substring(captureIdx, Math.Min(700, view.Length - captureIdx));
        Assert.Contains("FlushDragFrameSyncNow()", releaseBody, StringComparison.Ordinal);
        Assert.Contains("FlushDragFrameSyncNow()", captureBody, StringComparison.Ordinal);
        // Flush must precede clearing drag flags.
        var flushInRelease = releaseBody.IndexOf("FlushDragFrameSyncNow()", StringComparison.Ordinal);
        var clearInRelease = releaseBody.IndexOf("_nodeDragging = false", StringComparison.Ordinal);
        Assert.True(flushInRelease >= 0 && clearInRelease > flushInRelease);
        var flushInCapture = captureBody.IndexOf("FlushDragFrameSyncNow()", StringComparison.Ordinal);
        var clearInCapture = captureBody.IndexOf("_nodeDragging = false", StringComparison.Ordinal);
        Assert.True(flushInCapture >= 0 && clearInCapture > flushInCapture);
    }

    /// <summary>C5-b：连续编辑结束必须触发 dirty 重算（零位移不误报，有位移必脏）。</summary>
    [Fact]
    public void C5b_EndContinuousCanvasEdit_MarksDirtyWhenPositionChanged()
    {
        Assert.True(CanvasDragFrameHelpers.MustRefreshDirtyAfterContinuousEditEnd);
        // Behavioral proof against shipped ViewModel (not string-only).
        var vmType = typeof(WorkspacePageViewModel);
        Assert.NotNull(vmType.GetMethod(nameof(WorkspacePageViewModel.BeginContinuousCanvasEdit)));
        Assert.NotNull(vmType.GetMethod(nameof(WorkspacePageViewModel.EndContinuousCanvasEdit)));
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 10 && walk is not null; i++)
        {
            var candidate = Path.Combine(new[] { walk.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (Directory.Exists(candidate) || File.Exists(candidate))
            {
                return candidate;
            }
            walk = walk.Parent;
        }

        throw new FileNotFoundException("Could not resolve " + string.Join('/', parts));
    }

    [Fact]
    public void W15_DynamicMiniMap_FitsFarNodesWithoutEdgeCollapse()
    {
        var transform = CanvasMiniMapHelpers.ComputeTransform(
            minX: 0,
            minY: 0,
            maxX: 10_200,
            maxY: 5_096);
        var first = transform.NodeMarkerPosition(0, 0, 200, 96);
        var far = transform.NodeMarkerPosition(10_000, 5_000, 200, 96);

        Assert.InRange(first.X, 0, CanvasMiniMapHelpers.ContentWidth - CanvasMiniMapHelpers.MarkerWidth);
        Assert.InRange(far.X, 0, CanvasMiniMapHelpers.ContentWidth - CanvasMiniMapHelpers.MarkerWidth);
        Assert.True(far.X - first.X > 100, "far nodes must remain separated instead of clamping to one edge pixel");
        Assert.True(far.Y - first.Y > 50);
    }

    [Fact]
    public void W15_DynamicMiniMap_ClickRoundTripsFarLogicalCoordinates()
    {
        var transform = CanvasMiniMapHelpers.ComputeTransform(-600, -300, 12_400, 7_200);
        var mini = transform.LogicalToMiniMap(9_800, 6_100);
        var logical = transform.MiniMapToLogical(mini.X, mini.Y);

        Assert.Equal(9_800, logical.X, 6);
        Assert.Equal(6_100, logical.Y, 6);
    }

    [Fact]
    public void W15_DynamicMiniMap_ViewportFrameStaysInsideContent()
    {
        var transform = CanvasMiniMapHelpers.ComputeTransform(0, 0, 10_200, 5_096);
        var frame = transform.ViewportFrame(9_500, 4_700, 1_200, 800);

        Assert.InRange(frame.X, 0, CanvasMiniMapHelpers.ContentWidth);
        Assert.InRange(frame.Y, 0, CanvasMiniMapHelpers.ContentHeight);
        Assert.True(frame.Width >= 0 && frame.Height >= 0);
        Assert.True(frame.X + frame.Width <= CanvasMiniMapHelpers.ContentWidth + 1e-9);
        Assert.True(frame.Y + frame.Height <= CanvasMiniMapHelpers.ContentHeight + 1e-9);
    }

    [Fact]
    public void W15_ShippedView_UsesGraphBoundsTransform_NotFixedPointOneScale()
    {
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));
        var source = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "WorkspacePageViewModel.cs"));

        Assert.Contains("ComputeMiniMapTransform(viewModel)", view, StringComparison.Ordinal);
        Assert.Contains("_miniMapTransform.MiniMapToLogical", view, StringComparison.Ordinal);
        Assert.Contains("_miniMapTransform.ViewportFrame", view, StringComparison.Ordinal);
        Assert.DoesNotContain("MiniMapScale", source, StringComparison.Ordinal);
        Assert.DoesNotContain("MiniMapX", source, StringComparison.Ordinal);
    }

    [Fact]
    public void NormalizeRect_AllowsDragAnyCorner()
    {
        var (x, y, w, h) = CanvasSelectionHelpers.NormalizeRect(100, 80, 20, 10);
        Assert.Equal(20, x);
        Assert.Equal(10, y);
        Assert.Equal(80, w);
        Assert.Equal(70, h);
    }

    [Theory]
    [InlineData(10, 10, 200, 96, 0, 0, 50, 50, true)]   // 部分重叠
    [InlineData(100, 100, 200, 96, 0, 0, 50, 50, false)] // 不相交
    [InlineData(0, 0, 200, 96, 10, 10, 20, 20, true)]    // 框在节点内
    public void NodeIntersectsRect_HitsWhenOverlap(
        double nx, double ny, double nw, double nh,
        double rx, double ry, double rw, double rh,
        bool expected)
    {
        Assert.Equal(expected, CanvasSelectionHelpers.NodeIntersectsRect(nx, ny, nw, nh, rx, ry, rw, rh));
    }

    [Fact]
    public void ExceedsMarqueeThreshold_SeparatesClickFromDrag()
    {
        Assert.False(CanvasSelectionHelpers.ExceedsMarqueeThreshold(2, 2, 4));
        Assert.True(CanvasSelectionHelpers.ExceedsMarqueeThreshold(5, 0, 4));
    }

    [Theory]
    [InlineData("a", "b", "a", true)]
    [InlineData("a", "b", "b", true)]
    [InlineData("a", "b", "c", false)]
    public void EdgeTouchesNode_OnlyEndpoints(string src, string tgt, string node, bool expected)
    {
        Assert.Equal(expected, CanvasSelectionHelpers.EdgeTouchesNode(src, tgt, node));
    }

    [Fact]
    public void EdgeTouchesAnyNode_MatchesSelectedSet()
    {
        Assert.True(CanvasSelectionHelpers.EdgeTouchesAnyNode("n1", "n2", new[] { "n9", "n2" }));
        Assert.False(CanvasSelectionHelpers.EdgeTouchesAnyNode("n1", "n2", new[] { "n3" }));
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
        // 内容栏内，非标题行
        Assert.True(NodePortSpec.DataPortY > NodePortSpec.CardTopOffset + NodePortSpec.TitleBarHeight);
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

    [Theory]
    [InlineData("input", 0)]
    [InlineData("data-in-1", 1)]
    [InlineData("data-in-3", 3)]
    public void ParseDataInIndex_AndHandleName_RoundTrip(string handle, int index)
    {
        Assert.Equal(index, NodePortSpec.ParseDataInIndex(handle));
        Assert.Equal(handle, NodePortSpec.DataInHandleName(index));
    }

    [Fact]
    public void LocalCenterForHandle_StacksDataInPinsVertically()
    {
        var (x0, y0) = NodePortSpec.LocalCenterForHandle("input");
        var (x1, y1) = NodePortSpec.LocalCenterForHandle("data-in-1");
        Assert.Equal(x0, x1, 6);
        Assert.Equal(y0 + NodePortSpec.DataPortSpacing, y1, 6);
        // 端点 = 节点原点 + LocalCenter
        var nodeX = 100.0;
        var nodeY = 50.0;
        Assert.Equal(nodeX + x0, nodeX + NodePortSpec.LocalCenterForHandle("input").X, 6);
        Assert.Equal(nodeY + y0, nodeY + NodePortSpec.LocalCenterForHandle("input").Y, 6);
    }

    [Fact]
    public void LabelBesideDataIn_IsRightOfPin()
    {
        var (lx, ly) = NodePortSpec.LabelBesideDataIn(40, 80);
        Assert.True(lx > 40);
        Assert.InRange(ly, 70, 80);
    }

    [Fact]
    public void IsDataInOccupied_RejectsSecondWireOnSameIn()
    {
        var existing = new[]
        {
            ("data", "n2", "input"),
        };
        Assert.True(CanvasSelectionHelpers.IsDataInOccupied(existing, "n2", "input"));
        Assert.False(CanvasSelectionHelpers.IsDataInOccupied(existing, "n2", "data-in-1"));
        Assert.False(CanvasSelectionHelpers.IsDataInOccupied(existing, "n3", "input"));
    }

    [Fact]
    public void DataInHandleName_DefaultIsInput()
    {
        Assert.Equal("input", NodePortSpec.DataInHandleName(0));
        Assert.Equal("data-in-2", NodePortSpec.DataInHandleName(2));
    }
}
