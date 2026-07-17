using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 08 工作区画布：边删除、视口 fit/pan/zoom、运行控制、脏状态与面板 chrome。
/// 驱动 shipped helpers / ViewModel，不重实现几何。
/// </summary>
public sealed class WorkspaceCanvas08Tests
{
    [Fact]
    public void W1_PreferDeleteEdge_WhenEdgeSelected()
    {
        Assert.True(CanvasSelectionHelpers.PreferDeleteEdgeOverNodes(hasSelectedEdge: true, hasSelectedNode: true));
        Assert.True(CanvasSelectionHelpers.PreferDeleteEdgeOverNodes(hasSelectedEdge: true, hasSelectedNode: false));
        Assert.False(CanvasSelectionHelpers.PreferDeleteEdgeOverNodes(hasSelectedEdge: false, hasSelectedNode: true));
    }

    [Fact]
    public void W1_DeleteSelectedEdge_RemovesEdgeKeepsNodes_AndUndoRestores()
    {
        var vm = CreateWorkspaceVm();
        vm.AddNodeAt("llm", 0, 0);
        vm.AddNodeAt("llm", 240, 0);
        Assert.Equal(2, vm.Nodes.Count);
        var a = vm.Nodes[0];
        var b = vm.Nodes[1];
        Assert.True(vm.TryConnectPorts(
            a.Id, NodePortKind.Data, NodePortDirection.Out,
            b.Id, NodePortKind.Data, NodePortDirection.In));
        Assert.Single(vm.Edges);
        var edgeId = vm.Edges[0].Id;
        Assert.True(vm.HasSelectedEdge);

        vm.DeleteSelectedEdge();

        Assert.Empty(vm.Edges);
        Assert.Equal(2, vm.Nodes.Count);
        Assert.Contains(vm.Nodes, n => n.Id == a.Id);
        Assert.Contains(vm.Nodes, n => n.Id == b.Id);
        Assert.False(vm.HasSelectedEdge);
        Assert.True(vm.HasUnsavedChanges);

        Assert.True(vm.UndoCommand.CanExecute(null));
        vm.UndoCommand.Execute(null);
        Assert.Single(vm.Edges);
        Assert.Equal(edgeId, vm.Edges[0].Id);
    }

    [Fact]
    public void W14_SelectedEdge_HasDistinctStroke()
    {
        var vm = CreateWorkspaceVm();
        vm.AddNodeAt("llm", 0, 0);
        vm.AddNodeAt("llm", 240, 0);
        Assert.True(vm.TryConnectPorts(
            vm.Nodes[0].Id, NodePortKind.Data, NodePortDirection.Out,
            vm.Nodes[1].Id, NodePortKind.Data, NodePortDirection.In));
        var edge = vm.Edges[0];
        Assert.True(edge.IsSelected);
        Assert.True(edge.StrokeThickness >= 3.0);
        Assert.Equal(1.0, edge.StrokeOpacity);

        edge.IsSelected = false;
        Assert.True(edge.StrokeThickness < 3.0);
        Assert.True(edge.StrokeOpacity < 1.0);
    }

    [Fact]
    public void W10_DirectedArrow_FollowsBezierTangent_ForRightToLeftGraph()
    {
        // 即使目标节点在左边，边的终点切线仍决定真实 source → target，而不是用节点 X 猜方向。
        var path = NodePortSpec.BuildEdgePath(420, 60, 120, 140, isCommunication: false);
        var arrow = NodePortSpec.BuildArrowHead(path.End, path.Control2);
        var incomingX = path.End.X - path.Control2.X;
        var incomingY = path.End.Y - path.Control2.Y;
        var arrowX = arrow.Tip.X - ((arrow.Left.X + arrow.Right.X) * 0.5);
        var arrowY = arrow.Tip.Y - ((arrow.Left.Y + arrow.Right.Y) * 0.5);

        Assert.True((incomingX * arrowX) + (incomingY * arrowY) > 0,
            "arrow must point along the cubic end tangent");
        Assert.True(Avalonia.Point.Distance(path.End, arrow.Tip) > 10,
            "arrow tip must sit outside the node instead of being hidden at the pin center");
    }

    [Fact]
    public void W10_CommunicationEdge_HasTwoFilledArrowHeads()
    {
        var edge = new WorkflowEdgeViewModel(
            new CanvasEdge(
                Id: "comm",
                Source: "a",
                Target: "b",
                SourceHandle: "communication",
                TargetHandle: "communication",
                Kind: "communication",
                Label: null,
                Data: null),
            DisplayNameService.LoadDefault(),
            _ => { },
            () => { });

        edge.UpdateEdgePath(sourceX: 0, sourceY: 80, targetX: 320, targetY: 100);

        var startArrow = Assert.IsType<Avalonia.Media.PathGeometry>(edge.StartArrowPath);
        var endArrow = Assert.IsType<Avalonia.Media.PathGeometry>(edge.EndArrowPath);
        Assert.True(edge.IsCommunication);
        Assert.True(Assert.Single(startArrow.Figures!).IsClosed);
        Assert.True(Assert.Single(endArrow.Figures!).IsClosed);
    }

    [Fact]
    public void F8_SummarizerPrimaryDataEdge_UsesAndTracksConfiguredChapterAlias()
    {
        var vm = CreateWorkspaceVm();
        vm.AddNodeAt("writer", 0, 0);
        vm.AddNodeAt("summarizer", 240, 0);
        var writer = vm.Nodes[0];
        var summarizer = vm.Nodes[1];
        Assert.True(summarizer.IsSummarizerNode);
        Assert.Equal("chapter_text", summarizer.SummarizerChapterTextAlias);

        Assert.True(vm.TryConnectPorts(
            writer.Id, NodePortKind.Data, NodePortDirection.Out,
            summarizer.Id, NodePortKind.Data, NodePortDirection.In));
        Assert.Equal("chapter_text", vm.Edges[0].Label);

        summarizer.SummarizerChapterTextAlias = "chapter_body";
        Assert.Equal("chapter_body", vm.Edges[0].Label);
    }

    [Fact]
    public void UtilityPrimaryDataEdges_UseAndTrackSearchAndLoopAliases()
    {
        var vm = CreateWorkspaceVm();
        vm.AddNodeAt("writer", 0, 0);
        vm.AddNodeAt("search", 240, 0);
        vm.AddNodeAt("loop", 480, 0);
        var writer = vm.Nodes[0];
        var search = vm.Nodes[1];
        var loop = vm.Nodes[2];

        Assert.True(vm.TryConnectPorts(
            writer.Id, NodePortKind.Data, NodePortDirection.Out,
            search.Id, NodePortKind.Data, NodePortDirection.In));
        Assert.Equal("query", vm.Edges[0].Label);
        search.QueryAlias = "search_query";
        Assert.Equal("search_query", vm.Edges[0].Label);

        Assert.True(vm.TryConnectPorts(
            writer.Id, NodePortKind.Data, NodePortDirection.Out,
            loop.Id, NodePortKind.Data, NodePortDirection.In));
        Assert.Equal("done", vm.Edges[1].Label);
        loop.StopInputAlias = "finished";
        Assert.Equal("finished", vm.Edges[1].Label);
    }

    [Fact]
    public void F8_SummarizerChapterSelection_AtomicallyUsesWorksTreeBusinessIds()
    {
        var vm = CreateWorkspaceVm();
        vm.AddNodeAt("summarizer", 0, 0);
        var summarizer = vm.Nodes[0];
        vm.SelectNode(summarizer);
        var option = new SummarizerChapterOption(
            "chapter-business-id",
            "document-business-id",
            "第一章",
            "documents/chapter-one.md");
        vm.SummarizerChapterOptions.Add(option);

        vm.SelectedSummarizerChapterOption = option;

        Assert.Equal("chapter-business-id", summarizer.SummarizerChapterId);
        Assert.Equal("document-business-id", summarizer.SummarizerChapterDocumentId);
        Assert.Same(option, vm.SelectedSummarizerChapterOption);
    }

    [Fact]
    public void W2_FitTransform_UsesBoundsAndViewport_NotOnlyTopLeftNudge()
    {
        // Graph entirely in positive quadrant — old Math.Max(0, 48-min*z) no-ops to 0.
        var (zoom, ox, oy) = CanvasViewportHelpers.ComputeFitTransform(
            minX: 400,
            minY: 300,
            maxX: 800,
            maxY: 600,
            viewportWidth: 800,
            viewportHeight: 600,
            padding: 48);

        Assert.InRange(zoom, CanvasViewportHelpers.MinZoom, CanvasViewportHelpers.MaxZoom);
        // Content must fit available area
        Assert.True(400 * zoom <= 800 - 48 * 2 + 1e-6);
        Assert.True(300 * zoom <= 600 - 48 * 2 + 1e-6);
        // Offset is not the trivial zero of the old fake fit
        Assert.False(ox == 0 && oy == 0 && zoom == 1.0);
    }

    [Fact]
    public void W2_WheelZoom_And_Pan_ChangeTransform()
    {
        var z1 = CanvasViewportHelpers.ApplyWheelZoom(1.0, +1);
        var z2 = CanvasViewportHelpers.ApplyWheelZoom(1.0, -1);
        Assert.True(z1 > 1.0);
        Assert.True(z2 < 1.0);
        Assert.Equal(
            CanvasViewportHelpers.MaxZoom,
            CanvasViewportHelpers.ApplyWheelZoom(CanvasViewportHelpers.MaxZoom, +100));

        var (ox, oy) = CanvasViewportHelpers.ApplyPan(10, 20, 5, -3);
        Assert.Equal(15, ox);
        Assert.Equal(17, oy);
    }

    [Fact]
    public void W2_AnchoredZoom_KeepsPointerLogicalPositionStable()
    {
        const double oldZoom = 1.0;
        const double newZoom = 1.5;
        const double oldOffsetX = 40;
        const double oldOffsetY = -20;
        const double anchorX = 320;
        const double anchorY = 180;
        var logicalX = (anchorX - oldOffsetX) / oldZoom;
        var logicalY = (anchorY - oldOffsetY) / oldZoom;

        var (offsetX, offsetY) = CanvasViewportHelpers.ComputeAnchoredZoomOffset(
            oldZoom, newZoom, oldOffsetX, oldOffsetY, anchorX, anchorY);

        Assert.Equal(anchorX, logicalX * newZoom + offsetX, 6);
        Assert.Equal(anchorY, logicalY * newZoom + offsetY, 6);
    }

    [Fact]
    public void A5_ViewportSession_IsSingleOwnerForZoomOffsetAndPanLifecycle()
    {
        var session = new CanvasViewportSession();
        session.SetOffset(40, -20);
        var logicalBefore = session.ToLogical(320, 180);

        var zoomed = session.ZoomAt(1.5, 320, 180);
        var logicalAfter = session.ToLogical(320, 180);

        Assert.Equal(1.5, zoomed.Zoom, 6);
        Assert.Equal(logicalBefore.X, logicalAfter.X, 6);
        Assert.Equal(logicalBefore.Y, logicalAfter.Y, 6);

        session.BeginPan(100, 80);
        Assert.True(session.IsPanning);
        var panned = session.UpdatePan(125, 65);
        Assert.Equal(zoomed.OffsetX + 25, panned.OffsetX, 6);
        Assert.Equal(zoomed.OffsetY - 15, panned.OffsetY, 6);
        session.EndPan();
        Assert.False(session.IsPanning);
    }

    [Fact]
    public void A5_ViewportSession_FitCommitsZoomAndOffsetAtomically()
    {
        var session = new CanvasViewportSession();
        var state = session.Fit(
            minX: 100,
            minY: 50,
            maxX: 500,
            maxY: 250,
            safeViewport: new CanvasViewportRect(20, 90, 580, 330));

        Assert.Equal(state, session.Current);
        Assert.Equal(state.Zoom, session.Zoom);
        Assert.Equal(state.OffsetX, session.OffsetX);
        Assert.Equal(state.OffsetY, session.OffsetY);
    }

    [Fact]
    public void W6_NodePlacementAvoidsToolbarAndMiniMapOcclusions()
    {
        var blockers = new[]
        {
            new CanvasViewportRect(0, 0, 800, 80),
            new CanvasViewportRect(640, 450, 160, 150),
        };

        var (x, y) = CanvasViewportHelpers.KeepNodeReachable(
            logicalX: 680,
            logicalY: 490,
            nodeWidth: 120,
            nodeHeight: 80,
            zoom: 1,
            offsetX: 0,
            offsetY: 0,
            viewportWidth: 800,
            viewportHeight: 600,
            blockers);
        var placed = new CanvasViewportRect(x, y, 120, 80);

        Assert.InRange(placed.X, 8, 672);
        Assert.InRange(placed.Y, 8, 512);
        Assert.DoesNotContain(blockers.Select(blocker => blocker.Inflate(8)), placed.Intersects);
    }

    [Fact]
    public void W6_FitUsesUnobscuredSafeViewport()
    {
        var safeViewport = new CanvasViewportRect(20, 90, 580, 330);
        var (zoom, offsetX, offsetY) = CanvasViewportHelpers.ComputeFitTransform(
            minX: 100,
            minY: 50,
            maxX: 500,
            maxY: 250,
            safeViewport,
            padding: 24);

        Assert.True((100 * zoom) + offsetX >= safeViewport.X + 24 - 1e-6);
        Assert.True((500 * zoom) + offsetX <= safeViewport.Right - 24 + 1e-6);
        Assert.True((50 * zoom) + offsetY >= safeViewport.Y + 24 - 1e-6);
        Assert.True((250 * zoom) + offsetY <= safeViewport.Bottom - 24 + 1e-6);
    }

    [Fact]
    public void W6_ShippedViewUsesSharedSafePlacementForDragAddPasteAndFit()
    {
        var axaml = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));
        var viewModel = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "WorkspacePageViewModel.cs"));

        Assert.Contains("x:Name=\"WorkflowSelectorHost\"", axaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"CanvasToolbarActions\"", axaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"CanvasStatusHost\"", axaml, StringComparison.Ordinal);
        Assert.Contains("TextTrimming=\"CharacterEllipsis\"", axaml, StringComparison.Ordinal);
        Assert.Contains("CanvasViewportHelpers.KeepNodeReachable", view, StringComparison.Ordinal);
        Assert.Contains("CanvasOcclusionRects()", view, StringComparison.Ordinal);
        Assert.Contains("SafeFitViewport()", view, StringComparison.Ordinal);
        Assert.Contains("RequestEnsureNodeVisible = EnsureNodeInSafeViewport", view, StringComparison.Ordinal);
        Assert.True(
            viewModel.Split("RequestEnsureNodeVisible?.Invoke(node)", StringSplitOptions.None).Length - 1 >= 2,
            "new-node and paste paths must both use the shared safe-placement callback");
    }

    [Fact]
    public void W9_SemanticZoom_HidesPrecisionControlsUntilEditableScale()
    {
        Assert.False(CanvasSemanticZoomHelpers.ShowDetails(0.4));
        Assert.False(CanvasSemanticZoomHelpers.AllowPrecisionControls(0.6));
        Assert.True(CanvasSemanticZoomHelpers.ShowDetails(0.75));
        Assert.True(CanvasSemanticZoomHelpers.AllowPrecisionControls(0.8));

        var vm = CreateWorkspaceVm();
        vm.SetCanvasZoom(0.4);
        Assert.False(vm.ShowCanvasDetails);
        Assert.False(vm.ShowCanvasPrecisionControls);
        vm.SetCanvasZoom(1.0);
        Assert.True(vm.ShowCanvasDetails);
        Assert.True(vm.ShowCanvasPrecisionControls);
    }

    [Fact]
    public void W9_ShippedView_HidesNoiseAndFocusesOverviewNodeBeforeEditing()
    {
        var axaml = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));

        Assert.Contains("ShowCanvasDetails", axaml, StringComparison.Ordinal);
        Assert.Contains("ShowCanvasPrecisionControls", axaml, StringComparison.Ordinal);
        Assert.Contains("FocusOverviewNodeIfNeeded", view, StringComparison.Ordinal);
        Assert.Contains("CanvasSemanticZoomHelpers.FocusZoom", view, StringComparison.Ordinal);
        Assert.Contains("!viewModel.ShowCanvasPrecisionControls", view, StringComparison.Ordinal);
    }

    [Fact]
    public void W7_NodeHeight_TracksDataPins_AndMarqueeUsesFullGeometry()
    {
        var vm = CreateWorkspaceVm();
        vm.AddNodeAt("llm", 40, 60);
        var node = vm.Nodes[0];
        Assert.Equal(NodePortSpec.MinimumNodeHeight, node.CanvasHeight, 6);

        node.AddDataInPin();
        node.AddDataInPin();
        Assert.True(node.CanvasHeight > NodePortSpec.MinimumNodeHeight);
        var lastPinCenter = NodePortSpec.DataPortY
            + ((node.DataInPins.Count - 1) * NodePortSpec.DataPortSpacing);
        Assert.True(node.CanvasHeight >= lastPinCenter + NodePortSpec.DataPortBottomInset);

        var hits = vm.HitTestNodesInRect(
            node.X + 10,
            node.Y + node.CanvasHeight - 4,
            node.X + 40,
            node.Y + node.CanvasHeight + 4);
        Assert.Contains(node, hits);
    }

    [Fact]
    public void W7_ShippedGeometry_UsesCanvasHeightAcrossViewAndFit()
    {
        var axaml = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));
        var source = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "WorkspacePageViewModel.cs"));

        Assert.Contains("Height=\"{Binding CanvasHeight}\"", axaml, StringComparison.Ordinal);
        Assert.Contains("TextWrapping=\"NoWrap\"", axaml, StringComparison.Ordinal);
        Assert.Contains("node.Y + node.CanvasHeight", view, StringComparison.Ordinal);
        Assert.Contains("n.CanvasHeight", source, StringComparison.Ordinal);
        Assert.DoesNotContain("NodeBodyHeight", view, StringComparison.Ordinal);
        Assert.DoesNotContain("NodeBodyHeight", source, StringComparison.Ordinal);
    }

    [Fact]
    public void W8_RunControl_Matrix_ByLifecycle()
    {
        Assert.True(CanvasRunControlHelpers.CanPause("running"));
        Assert.False(CanvasRunControlHelpers.CanPause("paused"));
        Assert.False(CanvasRunControlHelpers.CanPause(null));

        Assert.True(CanvasRunControlHelpers.CanResume("paused"));
        Assert.False(CanvasRunControlHelpers.CanResume("running"));

        Assert.True(CanvasRunControlHelpers.CanStop("running"));
        Assert.True(CanvasRunControlHelpers.CanStop("paused"));
        Assert.False(CanvasRunControlHelpers.CanStop("succeeded"));
        Assert.False(CanvasRunControlHelpers.CanStop(""));
    }

    [Fact]
    public void W8_WorkspaceVm_WiresLifecycleCanExecute()
    {
        var src = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "WorkspacePageViewModel.cs"));
        Assert.Contains("CanPauseWorkflow", src, StringComparison.Ordinal);
        Assert.Contains("CanResumeWorkflow", src, StringComparison.Ordinal);
        Assert.Contains("CanStopWorkflow", src, StringComparison.Ordinal);
        Assert.Contains("CanvasRunControlHelpers.CanPause", src, StringComparison.Ordinal);
        Assert.DoesNotContain(
            "PauseWorkflowCommand = new RelayCommand(() => _ = PauseWorkflowAsync(), HasCurrentRun);",
            src,
            StringComparison.Ordinal);
    }

    [Fact]
    public void N9_AllVisibleRunEntries_UsePageRunSessionCoordinator()
    {
        var source = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "WorkspacePageViewModel.cs"));
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));

        Assert.Equal(2, view.Split("Command=\"{Binding RunCommand}\"", StringSplitOptions.None).Length - 1);
        Assert.Contains("Command=\"{Binding RunSelectedNodeCommand}\"", view, StringComparison.Ordinal);
        Assert.Contains("runRequested: runNode => _ = RunNodeAsync(runNode)", source, StringComparison.Ordinal);
        Assert.Contains("RunCommand = new RelayCommand(() => runRequested(this));", source, StringComparison.Ordinal);
        Assert.DoesNotContain("RunWorkflowAsync(_currentWorkflowId()", source, StringComparison.Ordinal);

        Assert.Contains("ValidateWorkflowGraphAsync(graph)", source, StringComparison.Ordinal);
        Assert.Contains("SaveWorkflowGraphAsync(graph)", source, StringComparison.Ordinal);
        Assert.Contains("_runSession", source, StringComparison.Ordinal);
        Assert.Contains("var workflowId = CurrentWorkflowId;", source, StringComparison.Ordinal);
        Assert.Contains(".StartAsync(workflowId, startNodeId)", source, StringComparison.Ordinal);
        Assert.Contains("_runSession.ThrowIfStale(sessionFence)", source, StringComparison.Ordinal);
        Assert.Contains("_runSession.EventsReceived += ApplyWorkflowEvents", source, StringComparison.Ordinal);
        Assert.DoesNotContain("GetWorkflowEventsAsync(CurrentWorkflowId", source, StringComparison.Ordinal);
    }

    [Fact]
    public void W5_Dirty_Surfaces_OnConnect()
    {
        var vm = CreateWorkspaceVm();
        vm.AddNodeAt("llm", 0, 0);
        vm.AddNodeAt("llm", 240, 0);
        Assert.True(vm.TryConnectPorts(
            vm.Nodes[0].Id, NodePortKind.Data, NodePortDirection.Out,
            vm.Nodes[1].Id, NodePortKind.Data, NodePortDirection.In));
        Assert.True(vm.HasUnsavedChanges);
        Assert.False(string.IsNullOrEmpty(vm.UnsavedChangesBadgeText));
        Assert.NotEqual(vm.SaveText, vm.SaveToolTipText);
    }

    [Fact]
    public void U3_EmptyCanvasHint_SaysBottomLibrary_NotLeft()
    {
        var json = File.ReadAllText(Path.Combine(ResolveRepoRoot(), "core", "resources", "display_name.json"));
        Assert.Contains("从底部节点库", json, StringComparison.Ordinal);
        Assert.DoesNotContain("从左边拖", json, StringComparison.Ordinal);
        var names = DisplayNameService.LoadDefault();
        var hint = names.Text("ui.empty.workspace.start.hint");
        Assert.Contains("底部", hint, StringComparison.Ordinal);
        Assert.DoesNotContain("左边", hint, StringComparison.Ordinal);
    }

    [Fact]
    public void W16_BottomPanelToggle_TracksModeAndOpenState()
    {
        var vm = CreateWorkspaceVm();
        Assert.Contains("节点库", vm.BottomPanelToggleText, StringComparison.Ordinal);
        vm.IsExecutionPanel = true;
        Assert.Contains("执行", vm.BottomPanelToggleText, StringComparison.Ordinal);

        // 产品路径：ToggleLibraryCommand（pill 与 command 同源 IsLibraryOpen）
        Assert.True(vm.IsLibraryOpen);
        Assert.True(vm.BottomPanelShowsCollapseGlyph);
        Assert.True(vm.ToggleLibraryCommand.CanExecute(null));
        vm.ToggleLibraryCommand.Execute(null);
        Assert.False(vm.IsLibraryOpen);
        Assert.False(vm.BottomPanelShowsCollapseGlyph);
        vm.ToggleLibraryCommand.Execute(null);
        Assert.True(vm.IsLibraryOpen);
        Assert.True(vm.BottomPanelShowsCollapseGlyph);
    }

    [Fact]
    public void W16_Pill_ToggleLibrary_Wires_IsLibraryOpen_InShippedView()
    {
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));
        // pill click → ToggleLibrary → flips IsLibraryOpen (not only LibraryContent.IsVisible)
        Assert.Contains("vm.IsLibraryOpen = !vm.IsLibraryOpen", view, StringComparison.Ordinal);
        Assert.Contains("ApplyLibraryOpenState", view, StringComparison.Ordinal);
        Assert.Contains("nameof(WorkspacePageViewModel.IsLibraryOpen)", view, StringComparison.Ordinal);
        Assert.DoesNotContain(
            "var opening = !LibraryContent.IsVisible;",
            view,
            StringComparison.Ordinal);
    }

    [Fact]
    public void W12_ConnectMiss_SetsAuthorFacingStatus()
    {
        var vm = CreateWorkspaceVm();
        vm.NotifyConnectMissed();
        Assert.False(string.IsNullOrWhiteSpace(vm.StatusText));
        Assert.DoesNotContain("Exception", vm.StatusText, StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>
    /// C5-b：仅拖动改 X/Y 后 EndContinuousCanvasEdit 必须 dirty（离开保护依赖 HasUnsavedChanges）。
    /// 零位移松手不得误报 dirty。
    /// </summary>
    [Fact]
    public void C5b_DragPositionChange_MarksDirtyOnEndContinuousEdit_ZeroMoveDoesNot()
    {
        var vm = CreateWorkspaceVm();
        vm.AddNodeAt("llm", 40, 50);
        // Baseline as if just saved / loaded clean.
        var markClean = typeof(WorkspacePageViewModel).GetMethod(
            "MarkCanvasCleanForTests",
            BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
        if (markClean is not null)
        {
            markClean.Invoke(vm, null);
        }
        else
        {
            // Fallback: save-path method that captures _savedSnapshot when present.
            var afterSave = typeof(WorkspacePageViewModel).GetMethod(
                "AcceptSavedSnapshot",
                BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
            afterSave?.Invoke(vm, null);
        }

        // Force clean baseline via public End after no-op continuous edit when helpers missing.
        vm.BeginContinuousCanvasEdit();
        vm.EndContinuousCanvasEdit();
        // If still dirty from AddNodeAt, re-baseline by reflecting private field is too brittle —
        // use Capture + compare path: clear by re-serializing as saved.
        ForceCleanBaseline(vm);
        Assert.False(vm.HasUnsavedChanges, "precondition: canvas clean before drag");

        var node = vm.Nodes[0];
        var originX = node.X;
        var originY = node.Y;

        // Zero-displacement continuous edit must stay clean.
        vm.BeginContinuousCanvasEdit();
        node.X = originX;
        node.Y = originY;
        vm.EndContinuousCanvasEdit();
        Assert.False(vm.HasUnsavedChanges, "zero move must not mark dirty");

        // Real drag displacement must mark dirty for leave/save protection.
        vm.BeginContinuousCanvasEdit();
        node.X = originX + 32;
        node.Y = originY + 18;
        // Still deferred mid-drag is ok; end must commit dirty.
        vm.EndContinuousCanvasEdit();
        Assert.True(vm.HasUnsavedChanges, "position change after drag must set HasUnsavedChanges");
        Assert.False(string.IsNullOrEmpty(vm.UnsavedChangesBadgeText));
    }

    [Fact]
    public void W11_LabelLayout_AvoidsNodesAndOtherMeasuredLabels()
    {
        var requests = new[]
        {
            new CanvasEdgeLabelRequest("e1", 100, 100, 1, 0, 80, 20),
            new CanvasEdgeLabelRequest("e2", 100, 100, 1, 0, 80, 20),
        };
        var node = new CanvasViewportRect(55, 104, 90, 34);

        var placements = CanvasEdgeLabelLayoutHelpers.PlaceLabels(requests, new[] { node });
        var first = Assert.Single(placements, placement => placement.Id == "e1");
        var second = Assert.Single(placements, placement => placement.Id == "e2");

        Assert.True(first.IsVisible);
        Assert.True(second.IsVisible);
        Assert.False(first.Bounds.Intersects(node));
        Assert.False(second.Bounds.Intersects(node));
        Assert.False(first.Bounds.Intersects(second.Bounds));
    }

    [Fact]
    public void W11_DenseLayout_KeepsSelectedLabelAndHidesLowerPriorityCollision()
    {
        var requests = new[]
        {
            new CanvasEdgeLabelRequest("ordinary", 0, 0, 1, 0, 90, 20),
            new CanvasEdgeLabelRequest("selected", 0, 0, 1, 0, 90, 20, IsPriority: true),
        };
        var impossibleNode = new CanvasViewportRect(-1000, -1000, 2000, 2000);

        var placements = CanvasEdgeLabelLayoutHelpers.PlaceLabels(requests, new[] { impossibleNode });

        Assert.False(Assert.Single(placements, placement => placement.Id == "ordinary").IsVisible);
        Assert.True(Assert.Single(placements, placement => placement.Id == "selected").IsVisible);
    }

    [Fact]
    public void W11_ShippedView_UsesMeasuredLayoutAndFullTextTooltip()
    {
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));
        Assert.Contains("GetVisualDescendants()", view, StringComparison.Ordinal);
        Assert.Contains("EdgeLabelPlacementHost", view, StringComparison.Ordinal);
        Assert.Contains("CanvasEdgeLabelLayoutHelpers.PlaceLabels", view, StringComparison.Ordinal);
        Assert.Contains("edge.SetLabelLayout", view, StringComparison.Ordinal);
        Assert.Contains("_edgeSyncScheduled", view, StringComparison.Ordinal);
        Assert.DoesNotContain(
            "or nameof(WorkflowEdgeViewModel.MidpointLabel)",
            view,
            StringComparison.Ordinal);

        var axaml = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        Assert.Contains("x:Name=\"EdgeLabelPlacementHost\"", axaml, StringComparison.Ordinal);
        Assert.Contains("ToolTip.Tip=\"{Binding MidpointLabel}\"", axaml, StringComparison.Ordinal);
        Assert.Contains("MaxWidth=\"180\"", axaml, StringComparison.Ordinal);
        Assert.Contains("IsVisible=\"{Binding IsCanvasLabelVisible}\"", axaml, StringComparison.Ordinal);
    }

    [Fact]
    public void W13_DockedRightPanel_PreservesMinimumCanvasWidth()
    {
        var vm = CreateWorkspaceVm();
        vm.SetAvailableWorkspaceWidth(912);
        vm.IsRightPanelOpen = true;
        vm.RightPanelColumnWidth = new Avalonia.Controls.GridLength(560);

        Assert.False(vm.UseOverlayRightPanel);
        Assert.True(vm.IsRightPanelDocked);
        Assert.Equal(388, vm.RightPanelMaximumWidth, 6);
        Assert.Equal(388, vm.RightPanelColumnWidth.Value, 6);
        Assert.True(
            912 - vm.RightPanelColumnWidth.Value - vm.RightPanelSplitterWidth.Value
            >= WorkspaceResponsiveLayoutHelpers.MinimumCanvasWidth);
    }

    [Fact]
    public void W13_NarrowWorkspace_UsesOverlayAndStacksExecution()
    {
        var layout = WorkspaceResponsiveLayoutHelpers.Compute(
            availableWidth: 760,
            requestedRightPanelWidth: 560,
            isRightPanelOpen: true);

        Assert.True(layout.UseOverlayRightPanel);
        Assert.Equal(0, layout.DockedRightPanelWidth);
        Assert.InRange(
            layout.OverlayRightPanelWidth,
            WorkspaceResponsiveLayoutHelpers.MinimumRightPanelWidth,
            WorkspaceResponsiveLayoutHelpers.MaximumOverlayWidth);
        Assert.True(WorkspaceResponsiveLayoutHelpers.UseStackedExecutionLayout(640));
        Assert.False(WorkspaceResponsiveLayoutHelpers.UseStackedExecutionLayout(800));
    }

    [Fact]
    public void W13_ShippedView_WiresResponsiveRightPanelAndExecutionBreakpoints()
    {
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));
        Assert.Contains("SetAvailableWorkspaceWidth", view, StringComparison.Ordinal);
        Assert.Contains("ApplyRightPanelResponsiveLayout", view, StringComparison.Ordinal);
        Assert.Contains("ApplyExecutionResponsiveLayout", view, StringComparison.Ordinal);
        Assert.Contains("WorkspaceResponsiveLayoutHelpers.UseStackedExecutionLayout", view, StringComparison.Ordinal);
        Assert.Contains("Grid.SetColumn(RightPanelHost, 0)", view, StringComparison.Ordinal);
        Assert.Contains("AddControlRect(RightPanelHost, rects)", view, StringComparison.Ordinal);

        var axaml = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        Assert.Contains("x:Name=\"RightPanelHost\"", axaml, StringComparison.Ordinal);
        Assert.Contains("MaxWidth=\"{Binding RightPanelMaximumWidth}\"", axaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"ExecutionStartPane\"", axaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"ExecutionRunPane\"", axaml, StringComparison.Ordinal);
        Assert.Contains("IsVisible=\"{Binding IsRightPanelDocked}\"", axaml, StringComparison.Ordinal);
    }

    [Fact]
    public void W4_DirectionalKeyboardNavigation_SelectsNearestNodeOnRequestedAxis()
    {
        var nodes = new[]
        {
            new CanvasKeyboardNode("center", 200, 200, 200, 100),
            new CanvasKeyboardNode("left", 0, 215, 200, 100),
            new CanvasKeyboardNode("right", 430, 190, 200, 100),
            new CanvasKeyboardNode("up", 210, 20, 200, 100),
            new CanvasKeyboardNode("down", 205, 410, 200, 100),
            new CanvasKeyboardNode("right-off-axis", 410, 700, 200, 100),
        };

        Assert.Equal("left", CanvasKeyboardNavigationHelpers.FindDirectionalNode(
            "center", nodes, CanvasKeyboardDirection.Left));
        Assert.Equal("right", CanvasKeyboardNavigationHelpers.FindDirectionalNode(
            "center", nodes, CanvasKeyboardDirection.Right));
        Assert.Equal("up", CanvasKeyboardNavigationHelpers.FindDirectionalNode(
            "center", nodes, CanvasKeyboardDirection.Up));
        Assert.Equal("down", CanvasKeyboardNavigationHelpers.FindDirectionalNode(
            "center", nodes, CanvasKeyboardDirection.Down));
    }

    [Fact]
    public void W4_LibraryCommandsAndSharedPortRules_CreateGraphWithoutPointerInput()
    {
        var vm = CreateWorkspaceVm();
        var llm = Assert.Single(vm.UtilityNodes, item => item.NodeType == "llm");

        Assert.True(llm.AddCommand.TryExecute());
        Assert.True(llm.AddCommand.TryExecute());
        Assert.Equal(2, vm.Nodes.Count);

        vm.NotifyKeyboardConnectStarted();
        Assert.Contains("Tab", vm.StatusText, StringComparison.Ordinal);
        Assert.True(vm.TryConnectPorts(
            vm.Nodes[0].Id, NodePortKind.Data, NodePortDirection.Out,
            vm.Nodes[1].Id, NodePortKind.Data, NodePortDirection.In));
        Assert.Single(vm.Edges);

        vm.NotifyKeyboardConnectCancelled();
        Assert.Contains("取消", vm.StatusText, StringComparison.Ordinal);
    }

    [Fact]
    public void W4_ShippedView_ExposesFocusableSemanticControlsAndAutomationNames()
    {
        var axaml = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        Assert.Contains("x:Name=\"NodeKeyboardFocusHost\"", axaml, StringComparison.Ordinal);
        Assert.Contains("KeyDown=\"OnNodeCardKeyDown\"", axaml, StringComparison.Ordinal);
        Assert.Contains("KeyDown=\"OnPortKeyDown\"", axaml, StringComparison.Ordinal);
        Assert.Contains("Classes=\"pin-glass keyboard-target\"", axaml, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.Name=\"{Binding $parent[UserControl].DataContext.PortDataInTip}\"", axaml, StringComparison.Ordinal);
        Assert.Contains("<Button Classes=\"library-chip entry keyboard-target\"", axaml, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding AddCommand}\"", axaml, StringComparison.Ordinal);
        Assert.Contains("KeyDown=\"OnBottomPillKeyDown\"", axaml, StringComparison.Ordinal);
        Assert.Contains("KeyDown=\"OnRightPillKeyDown\"", axaml, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.LiveSetting=\"Polite\"", axaml, StringComparison.Ordinal);
    }

    [Fact]
    public void W4_ShippedKeyboardPath_ReusesConnectionRulesAndStandardShortcuts()
    {
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));
        Assert.Contains("CanvasKeyboardNavigationHelpers.FindDirectionalNode", view, StringComparison.Ordinal);
        Assert.Contains("viewModel.BeginPortDragHighlight", view, StringComparison.Ordinal);
        Assert.Contains("viewModel.TryConnectPorts", view, StringComparison.Ordinal);
        Assert.Contains("CancelKeyboardConnection(announce: true)", view, StringComparison.Ordinal);
        Assert.Contains("e.Key == Key.S", view, StringComparison.Ordinal);
        Assert.Contains("e.Key == Key.C", view, StringComparison.Ordinal);
        Assert.Contains("e.Key == Key.X", view, StringComparison.Ordinal);
        Assert.Contains("e.Key == Key.V", view, StringComparison.Ordinal);
        Assert.Contains("viewModel.DeleteSelectedNodeCommand", view, StringComparison.Ordinal);
    }

    [Fact]
    public void U4_FirstScreen_OpensOnlyLibrary_AndFocusModeRestoresPanelState()
    {
        var vm = CreateWorkspaceVm();

        Assert.True(vm.IsLibraryOpen);
        Assert.False(vm.IsRightPanelOpen);
        Assert.False(vm.IsCanvasFocusMode);

        Assert.True(vm.ToggleCanvasFocusModeCommand.TryExecute());
        Assert.True(vm.IsCanvasFocusMode);
        Assert.False(vm.IsLibraryOpen);
        Assert.False(vm.IsRightPanelOpen);

        Assert.True(vm.ToggleCanvasFocusModeCommand.TryExecute());
        Assert.False(vm.IsCanvasFocusMode);
        Assert.True(vm.IsLibraryOpen);
        Assert.False(vm.IsRightPanelOpen);
    }

    [Fact]
    public void U4_SelectingNode_OpensDetailsOnlyAfterExplicitCanvasAction()
    {
        var vm = CreateWorkspaceVm();
        Assert.False(vm.IsRightPanelOpen);

        Assert.True(vm.AddStartNodeCommand.TryExecute());

        Assert.Single(vm.Nodes);
        Assert.True(vm.IsRightPanelOpen);
        Assert.True(vm.IsNodeDetailsTab);
    }

    [Fact]
    public void U4_ShippedView_GroupsToolbarAndOffersEmptyStateAndFocusActions()
    {
        var axaml = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        Assert.Contains("x:Name=\"CanvasEditActions\"", axaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"CanvasFileActions\"", axaml, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"CanvasViewActions\"", axaml, StringComparison.Ordinal);
        Assert.Contains("Content=\"{Binding CtxAddStartText}\"", axaml, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding AddStartNodeCommand}\"", axaml, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding ToggleCanvasFocusModeCommand}\"", axaml, StringComparison.Ordinal);

        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));
        Assert.Contains("e.Key == Key.F11", view, StringComparison.Ordinal);
        Assert.Contains("viewModel.IsCanvasFocusMode", view, StringComparison.Ordinal);
    }

    private static void ForceCleanBaseline(WorkspacePageViewModel vm)
    {
        // Sets private _savedSnapshot = CurrentSnapshot and HasUnsavedChanges = false.
        var field = typeof(WorkspacePageViewModel).GetField(
            "_savedSnapshot",
            BindingFlags.Instance | BindingFlags.NonPublic);
        var current = typeof(WorkspacePageViewModel).GetMethod(
            "CurrentSnapshot",
            BindingFlags.Instance | BindingFlags.NonPublic);
        Assert.NotNull(field);
        Assert.NotNull(current);
        var snap = current!.Invoke(vm, null);
        field!.SetValue(vm, snap);
        var dirty = typeof(WorkspacePageViewModel).GetProperty(nameof(WorkspacePageViewModel.HasUnsavedChanges));
        dirty!.SetValue(vm, false);
    }

    [Fact]
    public void FitView_And_PanWheel_WiredInShippedView()
    {
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml.cs"));
        var session = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "CanvasViewportSession.cs"));
        Assert.Contains("CanvasViewportHelpers.ComputeFitTransform", session, StringComparison.Ordinal);
        Assert.Contains("CanvasViewportHelpers.ComputeAnchoredZoomOffset", session, StringComparison.Ordinal);
        Assert.Contains("CanvasViewportHelpers.ApplyPan", session, StringComparison.Ordinal);
        Assert.Contains("OnCanvasPointerWheel", view, StringComparison.Ordinal);
        Assert.Contains("CanvasViewport.UpdatePan", view, StringComparison.Ordinal);
        Assert.Contains("CanvasViewport.SetOffset", view, StringComparison.Ordinal);
        Assert.Contains("CanvasViewport.ToLogical", view, StringComparison.Ordinal);
        Assert.DoesNotContain("_panOriginX", view, StringComparison.Ordinal);
        Assert.DoesNotContain("_panOriginY", view, StringComparison.Ordinal);
        Assert.DoesNotContain("_panStartScreen", view, StringComparison.Ordinal);
        Assert.Contains("_spacePanMode", view, StringComparison.Ordinal);
        Assert.Contains("Key.Home", view, StringComparison.Ordinal);
        Assert.Contains("Key.NumPad0", view, StringComparison.Ordinal);
        var axaml = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        Assert.Contains("PointerWheelChanged=\"OnCanvasPointerWheel\"", axaml, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding FitViewCommand}\"", axaml, StringComparison.Ordinal);
        Assert.Contains("Ariadne.Icon.FitView", axaml, StringComparison.Ordinal);
        Assert.Contains("BottomPanelToggleText", axaml, StringComparison.Ordinal);
        Assert.Contains("HasUnsavedChanges", axaml, StringComparison.Ordinal);
        Assert.Contains("StrokeOpacity", axaml, StringComparison.Ordinal);
        Assert.Contains("Data=\"{Binding EndArrowPath}\"", axaml, StringComparison.Ordinal);
        Assert.Contains("Data=\"{Binding StartArrowPath}\"", axaml, StringComparison.Ordinal);
        Assert.Contains("IsVisible=\"{Binding IsCommunication}\"", axaml, StringComparison.Ordinal);
    }

    private static WorkspacePageViewModel CreateWorkspaceVm()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = DispatchProxy.Create<IAriadneBackendClient, SoftBackendProxy>();
        return new WorkspacePageViewModel(names, backend);
    }

    /// <summary>HasProjectRoot=true so empty-canvas strings use workspace keys; other methods no-op.</summary>
    private class SoftBackendProxy : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            if (targetMethod.ReturnType == typeof(bool) && targetMethod.Name == "get_HasProjectRoot")
            {
                return true;
            }

            if (targetMethod.ReturnType == typeof(void) || targetMethod.ReturnType == typeof(Task))
            {
                return targetMethod.ReturnType == typeof(Task) ? Task.CompletedTask : null;
            }

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var t = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(t)
                    .Invoke(null, new object?[] { t.IsValueType ? Activator.CreateInstance(t) : null });
            }

            if (targetMethod.ReturnType.IsValueType)
            {
                return Activator.CreateInstance(targetMethod.ReturnType);
            }

            return null;
        }
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 12 && walk is not null; i++)
        {
            var candidate = Path.Combine(new[] { walk.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (Directory.Exists(candidate) || File.Exists(candidate))
            {
                return candidate;
            }

            walk = walk.Parent;
        }

        throw new FileNotFoundException(string.Join('/', parts));
    }

    private static string ResolveRepoRoot()
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 12 && walk is not null; i++)
        {
            if (File.Exists(Path.Combine(walk.FullName, "Cargo.toml")))
            {
                return walk.FullName;
            }

            walk = walk.Parent;
        }

        throw new DirectoryNotFoundException("repo root");
    }
}
