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
        Assert.Contains("CanvasViewportHelpers.ComputeFitTransform", view, StringComparison.Ordinal);
        Assert.Contains("OnCanvasPointerWheel", view, StringComparison.Ordinal);
        Assert.Contains("ApplyPan", view, StringComparison.Ordinal);
        var axaml = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        Assert.Contains("PointerWheelChanged=\"OnCanvasPointerWheel\"", axaml, StringComparison.Ordinal);
        Assert.Contains("BottomPanelToggleText", axaml, StringComparison.Ordinal);
        Assert.Contains("HasUnsavedChanges", axaml, StringComparison.Ordinal);
        Assert.Contains("StrokeOpacity", axaml, StringComparison.Ordinal);
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
