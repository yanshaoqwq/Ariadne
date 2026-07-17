using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using Avalonia.VisualTree;
using System.Collections.Specialized;
using System.ComponentModel;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class WorkspacePageView : UserControl
{
    private const double DragThreshold = 4.0;

    private GridLength _savedLibraryHeight = new(220);

    // ---- 库底部 Pill（左右拖动） ----
    private double _togglePillLeft = -1;
    private bool _pilDragging;
    private bool _pilMoved;
    private double _pilDragStartX;
    private double _pilDragOriginLeft;

    // ---- 右侧 Pill（上下拖动） ----
    private double _rightPillTop = -1;
    private bool _rightPilDragging;
    private bool _rightPilMoved;
    private double _rightPilDragStartY;
    private double _rightPilDragOriginTop;

    // ---- 节点拖动 ----
    private bool _nodeDragging;
    private WorkflowNodeViewModel? _draggedNode;
    private Point _nodeDragStart;
    private double _nodeDragOriginX;
    private double _nodeDragOriginY;
    private bool _dragFrameScheduled;
    private readonly Dictionary<string, WorkflowNodeViewModel> _nodesById = new(StringComparer.Ordinal);
    private readonly Dictionary<string, List<WorkflowEdgeViewModel>> _edgesByNodeId = new(StringComparer.Ordinal);
    private readonly Dictionary<string, Control> _nodeContainersById = new(StringComparer.Ordinal);
    private readonly Dictionary<string, Control> _miniMapContainersById = new(StringComparer.Ordinal);
    private CanvasMiniMapTransform _miniMapTransform = CanvasMiniMapHelpers.ComputeTransform(0, 0, 1400, 840);
    private bool _edgeSyncScheduled;
    private bool _edgeLabelLayoutScheduled;
    private bool? _executionLayoutCompact;

    // ---- 左键框选（空白处按下拖动）----
    private bool _marqueePointerDown;
    private bool _marqueeActive;
    private Point _marqueeOriginLogical;
    private Point _marqueeCurrentLogical;
    private bool _marqueeAdditive;

    // ---- W2：中键 / Alt+左键 / 空格+左键平移 ----
    private bool _spacePanMode;

    // ---- 端口拖线（任意口起拖，落点类型校验 + 橡皮筋） ----
    private bool _edgeDragging;
    private WorkflowNodeViewModel? _edgeSourceNode;
    private NodePortKind _edgeSourceKind;
    private NodePortDirection _edgeSourceDirection;
    private string? _edgeSourceHandle;
    private Point _rubberBandStartLogical;

    // ---- W4：键盘端口连线（Tab 选目标，Enter/空格确认，Esc 取消） ----
    private WorkflowNodeViewModel? _keyboardEdgeSourceNode;
    private NodePortKind _keyboardEdgeSourceKind;
    private NodePortDirection _keyboardEdgeSourceDirection;
    private string? _keyboardEdgeSourceHandle;

    // ---- 节点库：页级指针手势（单击添加 / 拖到画布）----
    private bool _libraryPointerDown;
    private bool _libraryDragging;
    private bool _libraryAddedThisGesture;
    private NodeLibraryItemViewModel? _libraryDragItem;
    private Point _libraryPressOrigin;

    private bool _layoutInitialized;
    private WorkspacePageViewModel? _attachedViewModel;

    public WorkspacePageView()
    {
        InitializeComponent();
        Focusable = true;
        AddHandler(KeyDownEvent, OnWorkspaceKeyDown, Avalonia.Interactivity.RoutingStrategies.Tunnel);
        AddHandler(KeyUpEvent, OnWorkspaceKeyUp, Avalonia.Interactivity.RoutingStrategies.Tunnel);
        DataContextChanged += (_, _) => AttachViewActions();
        SizeChanged += OnWorkspaceViewSizeChanged;
        LayoutUpdated += OnFirstLayout;
        if (CanvasOverlay is not null)
        {
            CanvasOverlay.SizeChanged += OnCanvasOverlaySizeChanged;
        }
        if (WorkspaceGrid is not null)
        {
            WorkspaceGrid.SizeChanged += OnWorkspacePrimarySizeChanged;
        }
        AttachViewActions();
    }

    private void OnWorkspaceViewSizeChanged(object? sender, SizeChangedEventArgs e)
    {
        if (DataContext is WorkspacePageViewModel viewModel)
        {
            viewModel.SetAvailableWorkspaceWidth(e.NewSize.Width);
        }
        ApplyRightPanelResponsiveLayout();
    }

    private void OnWorkspacePrimarySizeChanged(object? sender, SizeChangedEventArgs e)
    {
        ApplyExecutionResponsiveLayout(e.NewSize.Width);
    }

    private void OnCanvasOverlaySizeChanged(object? sender, SizeChangedEventArgs e)
    {
        ResizeNodeLayers(e.NewSize.Width, e.NewSize.Height);
    }

    private void ResizeNodeLayers(double width, double height)
    {
        if (width <= 0 || height <= 0)
        {
            return;
        }

        // ItemsControl 作为 Canvas 子项时默认 DesiredSize 可能为 0，必须铺满宿主
        if (NodesItemsControl is not null)
        {
            NodesItemsControl.Width = width;
            NodesItemsControl.Height = height;
        }
        if (EdgesItemsControl is not null)
        {
            EdgesItemsControl.Width = width;
            EdgesItemsControl.Height = height;
        }
        ScheduleFullCanvasSync();
    }

    private void OnFirstLayout(object? sender, EventArgs e)
    {
        if (_layoutInitialized || LibraryTogglePill is null || WorkspaceGrid is null)
        {
            return;
        }
        _layoutInitialized = true;
        if (CanvasOverlay is not null)
        {
            ResizeNodeLayers(CanvasOverlay.Bounds.Width, CanvasOverlay.Bounds.Height);
        }
        PositionBottomPill();
        PositionRightPill();
        if (DataContext is WorkspacePageViewModel viewModel && Bounds.Width > 0)
        {
            viewModel.SetAvailableWorkspaceWidth(Bounds.Width);
        }
        ApplyRightPanelResponsiveLayout();
        ApplyExecutionResponsiveLayout(WorkspaceGrid.Bounds.Width);
        SyncNodeContainerPositions();
        SyncEdgePositions();
        SyncMiniMapPositions();
    }

    private void AttachViewActions()
    {
        if (_attachedViewModel is not null && !ReferenceEquals(_attachedViewModel, DataContext))
        {
            _attachedViewModel.CanvasViewport.EndPan();
            _attachedViewModel.EndPortDragHighlight();
            _keyboardEdgeSourceNode = null;
            _keyboardEdgeSourceHandle = null;
            _attachedViewModel.RequestFitView = null;
            _attachedViewModel.RequestCanvasZoomStep = null;
            _attachedViewModel.RequestResetCanvasZoom = null;
            _attachedViewModel.RequestEnsureNodeVisible = null;
            _attachedViewModel.PickFolder = null;
            _attachedViewModel.PickFile = null;
            _attachedViewModel.Nodes.CollectionChanged -= OnNodesCollectionChanged;
            _attachedViewModel.Edges.CollectionChanged -= OnEdgesCollectionChanged;
            _attachedViewModel.PropertyChanged -= OnViewModelPropertyChanged;
            foreach (var node in _attachedViewModel.Nodes)
            {
                node.PropertyChanged -= OnNodePropertyChanged;
            }
            foreach (var edge in _attachedViewModel.Edges)
            {
                edge.PropertyChanged -= OnEdgePropertyChanged;
            }
            _attachedViewModel = null;
        }

        if (DataContext is WorkspacePageViewModel viewModel)
        {
            viewModel.RequestFitView = FitViewToNodes;
            viewModel.RequestCanvasZoomStep = ZoomCanvasAtCenterBy;
            viewModel.RequestResetCanvasZoom = ResetCanvasZoomAtCenter;
            viewModel.RequestEnsureNodeVisible = EnsureNodeInSafeViewport;
            viewModel.PickFolder = PickFolderAsync;
            viewModel.PickFile = PickFileAsync;
            viewModel.Nodes.CollectionChanged += OnNodesCollectionChanged;
            viewModel.Edges.CollectionChanged += OnEdgesCollectionChanged;
            viewModel.PropertyChanged += OnViewModelPropertyChanged;
            foreach (var node in viewModel.Nodes)
            {
                node.PropertyChanged += OnNodePropertyChanged;
            }
            foreach (var edge in viewModel.Edges)
            {
                edge.PropertyChanged += OnEdgePropertyChanged;
            }
            _attachedViewModel = viewModel;
            if (Bounds.Width > 0)
            {
                viewModel.SetAvailableWorkspaceWidth(Bounds.Width);
            }
            RebuildCanvasIndexes(viewModel);
            ApplyCanvasViewportState(viewModel.CanvasViewport.Current);
            // 初始布局与 VM 开合状态对齐
            ApplyLibraryOpenState(viewModel.IsLibraryOpen);
            ApplyRightPanelResponsiveLayout();
            ApplyExecutionResponsiveLayout(WorkspaceGrid?.Bounds.Width ?? 0);
            ScheduleNodeContainerSync();
            ScheduleEdgeSync();
            ScheduleMiniMapSync();
        }
    }

    private async Task<string?> PickFolderAsync(string? title)
    {
        var topLevel = TopLevel.GetTopLevel(this);
        if (topLevel is null)
        {
            return null;
        }

        var folders = await topLevel.StorageProvider.OpenFolderPickerAsync(new FolderPickerOpenOptions
        {
            Title = string.IsNullOrWhiteSpace(title) ? null : title,
            AllowMultiple = false,
        });
        return folders.FirstOrDefault()?.Path.LocalPath;
    }

    private async Task<string?> PickFileAsync(string? title)
    {
        var topLevel = TopLevel.GetTopLevel(this);
        if (topLevel is null)
        {
            return null;
        }

        var files = await topLevel.StorageProvider.OpenFilePickerAsync(new FilePickerOpenOptions
        {
            Title = string.IsNullOrWhiteSpace(title) ? null : title,
            AllowMultiple = false,
        });
        return files.FirstOrDefault()?.Path.LocalPath;
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        _spacePanMode = false;
        _keyboardEdgeSourceNode = null;
        _keyboardEdgeSourceHandle = null;
        if (_attachedViewModel is not null)
        {
            _attachedViewModel.CanvasViewport.EndPan();
            _attachedViewModel.RequestFitView = null;
            _attachedViewModel.RequestCanvasZoomStep = null;
            _attachedViewModel.RequestResetCanvasZoom = null;
            _attachedViewModel.Nodes.CollectionChanged -= OnNodesCollectionChanged;
            _attachedViewModel.Edges.CollectionChanged -= OnEdgesCollectionChanged;
            _attachedViewModel.PropertyChanged -= OnViewModelPropertyChanged;
            _attachedViewModel.EndPortDragHighlight();
            foreach (var node in _attachedViewModel.Nodes)
            {
                node.PropertyChanged -= OnNodePropertyChanged;
            }
            foreach (var edge in _attachedViewModel.Edges)
            {
                edge.PropertyChanged -= OnEdgePropertyChanged;
            }
            _attachedViewModel = null;
        }

        base.OnDetachedFromVisualTree(e);
    }

    private void OnViewModelPropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName is nameof(WorkspacePageViewModel.CanvasZoom))
        {
            ScheduleMiniMapSync();
            ScheduleEdgeLabelLayout();
        }
        if (e.PropertyName is nameof(WorkspacePageViewModel.UseOverlayRightPanel)
            or nameof(WorkspacePageViewModel.RightPanelOverlayWidth)
            or nameof(WorkspacePageViewModel.IsRightPanelOpen))
        {
            ApplyRightPanelResponsiveLayout();
        }
        // W16：pill / ToggleLibraryCommand 均经 IsLibraryOpen 驱动布局与 glyph
        if (e.PropertyName is nameof(WorkspacePageViewModel.IsLibraryOpen)
            && sender is WorkspacePageViewModel vm)
        {
            ApplyLibraryOpenState(vm.IsLibraryOpen);
        }
        if (e.PropertyName is nameof(WorkspacePageViewModel.GraphRevision)
            && sender is WorkspacePageViewModel graphViewModel)
        {
            RebuildCanvasIndexes(graphViewModel);
            ScheduleNodeContainerSync();
            ScheduleEdgeSync();
            ScheduleMiniMapSync();
        }
    }

    private void OnNodesCollectionChanged(object? sender, NotifyCollectionChangedEventArgs e)
    {
        if (_keyboardEdgeSourceNode is not null
            && e.OldItems?.OfType<WorkflowNodeViewModel>()
                .Any(node => ReferenceEquals(node, _keyboardEdgeSourceNode)) == true)
        {
            CancelKeyboardConnection(announce: false);
        }
        if (e.OldItems is not null)
        {
            foreach (var item in e.OldItems.OfType<WorkflowNodeViewModel>())
            {
                item.PropertyChanged -= OnNodePropertyChanged;
            }
        }
        if (_attachedViewModel is not null && !_attachedViewModel.IsApplyingGraph)
        {
            RebuildCanvasIndexes(_attachedViewModel);
        }
        if (e.NewItems is not null)
        {
            foreach (var item in e.NewItems.OfType<WorkflowNodeViewModel>())
            {
                item.PropertyChanged += OnNodePropertyChanged;
            }
        }
        if (_attachedViewModel is null || !_attachedViewModel.IsApplyingGraph)
        {
            ScheduleNodeContainerSync();
            ScheduleEdgeSync();
            ScheduleMiniMapSync();
        }
    }

    private void OnEdgesCollectionChanged(object? sender, NotifyCollectionChangedEventArgs e)
    {
        if (e.OldItems is not null)
        {
            foreach (var item in e.OldItems.OfType<WorkflowEdgeViewModel>())
            {
                item.PropertyChanged -= OnEdgePropertyChanged;
            }
        }
        if (_attachedViewModel is not null && !_attachedViewModel.IsApplyingGraph)
        {
            RebuildCanvasIndexes(_attachedViewModel);
        }
        if (e.NewItems is not null)
        {
            foreach (var item in e.NewItems.OfType<WorkflowEdgeViewModel>())
            {
                item.PropertyChanged += OnEdgePropertyChanged;
            }
        }
        if (_attachedViewModel is null || !_attachedViewModel.IsApplyingGraph)
        {
            ScheduleEdgeSync();
        }
    }

    private void OnEdgePropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName is nameof(WorkflowEdgeViewModel.Label)
            or nameof(WorkflowEdgeViewModel.ForwardAlias)
            or nameof(WorkflowEdgeViewModel.IsSelected)
            or nameof(WorkflowEdgeViewModel.SourceHandle)
            or nameof(WorkflowEdgeViewModel.TargetHandle))
        {
            ScheduleEdgeSync();
        }
    }

    private void OnNodePropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName is nameof(WorkflowNodeViewModel.X)
            or nameof(WorkflowNodeViewModel.Y)
            or nameof(WorkflowNodeViewModel.CanvasHeight))
        {
            if (_nodeDragging)
            {
                return;
            }
            ScheduleNodeContainerSync();
            ScheduleEdgeSync();
            ScheduleMiniMapSync();
        }
    }

    private void ScheduleNodeContainerSync()
    {
        Dispatcher.UIThread.Post(SyncNodeContainerPositions, DispatcherPriority.Background);
    }

    private void ScheduleEdgeSync()
    {
        if (_edgeSyncScheduled)
        {
            return;
        }
        _edgeSyncScheduled = true;
        Dispatcher.UIThread.Post(() =>
        {
            _edgeSyncScheduled = false;
            SyncEdgePositions();
        }, DispatcherPriority.Background);
    }

    private void ScheduleEdgeLabelLayout()
    {
        if (_edgeLabelLayoutScheduled)
        {
            return;
        }
        _edgeLabelLayoutScheduled = true;
        Dispatcher.UIThread.Post(() =>
        {
            _edgeLabelLayoutScheduled = false;
            LayoutEdgeLabels();
        }, DispatcherPriority.Render);
    }

    private void ScheduleMiniMapSync()
    {
        Dispatcher.UIThread.Post(SyncMiniMapPositions, DispatcherPriority.Background);
    }

    private void ApplyRightPanelResponsiveLayout()
    {
        if (RightPanelHost is null || DataContext is not WorkspacePageViewModel viewModel)
        {
            return;
        }

        if (viewModel.UseOverlayRightPanel)
        {
            Grid.SetColumn(RightPanelHost, 0);
            RightPanelHost.HorizontalAlignment = Avalonia.Layout.HorizontalAlignment.Right;
            RightPanelHost.Width = viewModel.RightPanelOverlayWidth;
            RightPanelHost.ZIndex = 100;
        }
        else
        {
            Grid.SetColumn(RightPanelHost, 2);
            RightPanelHost.HorizontalAlignment = Avalonia.Layout.HorizontalAlignment.Stretch;
            RightPanelHost.Width = double.NaN;
            RightPanelHost.ZIndex = 0;
        }
    }

    private void ApplyExecutionResponsiveLayout(double primaryPaneWidth)
    {
        if (ExecutionLayoutGrid is null
            || ExecutionLayoutGrid.ColumnDefinitions.Count < 3
            || ExecutionLayoutGrid.RowDefinitions.Count < 3
            || ExecutionStartPane is null
            || ExecutionRunPane is null)
        {
            return;
        }

        var compact = WorkspaceResponsiveLayoutHelpers.UseStackedExecutionLayout(primaryPaneWidth);
        if (_executionLayoutCompact == compact)
        {
            return;
        }
        _executionLayoutCompact = compact;
        var primaryColumn = ExecutionLayoutGrid.ColumnDefinitions[0];
        var gapColumn = ExecutionLayoutGrid.ColumnDefinitions[1];
        var runColumn = ExecutionLayoutGrid.ColumnDefinitions[2];
        var primaryRow = ExecutionLayoutGrid.RowDefinitions[0];
        var gapRow = ExecutionLayoutGrid.RowDefinitions[1];
        var runRow = ExecutionLayoutGrid.RowDefinitions[2];
        primaryColumn.Width = new GridLength(1, GridUnitType.Star);
        primaryRow.Height = GridLength.Auto;
        if (compact)
        {
            gapColumn.Width = new GridLength(0);
            runColumn.Width = new GridLength(0);
            gapRow.Height = new GridLength(12);
            runRow.Height = GridLength.Auto;
            Grid.SetColumn(ExecutionStartPane, 0);
            Grid.SetRow(ExecutionStartPane, 0);
            Grid.SetColumn(ExecutionRunPane, 0);
            Grid.SetRow(ExecutionRunPane, 2);
        }
        else
        {
            gapColumn.Width = new GridLength(18);
            runColumn.Width = new GridLength(280);
            gapRow.Height = new GridLength(0);
            runRow.Height = new GridLength(0);
            Grid.SetColumn(ExecutionStartPane, 0);
            Grid.SetRow(ExecutionStartPane, 0);
            Grid.SetColumn(ExecutionRunPane, 2);
            Grid.SetRow(ExecutionRunPane, 0);
        }
        ExecutionLayoutGrid.InvalidateMeasure();
    }

    // ===================== 收起/展开下栏（库底部 Pill 点击） =====================

    /// <summary>
    /// W16 产品路径：pill 点击与 ToggleLibraryCommand 共用 IsLibraryOpen，
    /// 布局与 BottomPanelShowsCollapseGlyph 同源。
    /// </summary>
    private void ToggleLibrary()
    {
        if (DataContext is WorkspacePageViewModel vm)
        {
            // 产品入口：只改 ViewModel；布局在 PropertyChanged → ApplyLibraryOpenState
            vm.IsLibraryOpen = !vm.IsLibraryOpen;
            return;
        }

        // 无 VM 时兜底（测试/预览）
        ApplyLibraryOpenState(!(LibraryContent?.IsVisible ?? true));
    }

    /// <summary>根据 IsLibraryOpen 同步 Grid/可见性（shipped layout path）。</summary>
    private void ApplyLibraryOpenState(bool open)
    {
        if (WorkspaceGrid is null || LibrarySplitter is null || LibraryContent is null)
        {
            return;
        }

        var row = WorkspaceGrid.RowDefinitions[2];
        if (open)
        {
            LibraryContent.IsVisible = true;
            LibrarySplitter.IsVisible = true;
            row.Height = _savedLibraryHeight;
        }
        else
        {
            if (row.Height.IsAbsolute && row.Height.Value > 60)
            {
                _savedLibraryHeight = row.Height;
            }
            LibraryContent.IsVisible = false;
            LibrarySplitter.IsVisible = false;
            row.Height = GridLength.Auto;
        }

        PositionBottomPill();
    }

    // ===================== 库底部 Pill 位置 =====================

    private void PositionBottomPill()
    {
        if (LibraryTogglePill is null || WorkspaceGrid is null)
        {
            return;
        }

        if (_togglePillLeft < 0)
        {
            var canvasWidth = WorkspaceGrid.Bounds.Width;
            _togglePillLeft = canvasWidth > 0
                ? (canvasWidth - LibraryTogglePill.Width) / 2
                : 200;
        }

        Canvas.SetLeft(LibraryTogglePill, _togglePillLeft);
    }

    // ===================== 右侧 Pill 位置 =====================

    private void PositionRightPill()
    {
        if (WorkspaceRightPill is null)
        {
            return;
        }

        Control? host = CanvasHost is not null ? CanvasHost : CanvasOverlay;
        if (host is null)
        {
            return;
        }

        if (_rightPillTop < 0)
        {
            var h = host.Bounds.Height;
            _rightPillTop = h > 0 ? (h - WorkspaceRightPill.Height) / 2 : 120;
        }

        Canvas.SetTop(WorkspaceRightPill, _rightPillTop);
    }

    // ===================== 库底部 Pill 拖拽（左右） =====================

    public void OnPillPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            return;
        }
        _pilDragging = true;
        _pilMoved = false;
        _pilDragStartX = e.GetPosition(this).X;
        _pilDragOriginLeft = _togglePillLeft < 0 ? 200 : _togglePillLeft;
        e.Pointer.Capture((IInputElement?)sender);
        e.Handled = true;
    }

    public void OnPillPointerMoved(object? sender, PointerEventArgs e)
    {
        if (!_pilDragging || LibraryTogglePill is null || WorkspaceGrid is null)
        {
            return;
        }
        var dx = e.GetPosition(this).X - _pilDragStartX;
        if (!_pilMoved && Math.Abs(dx) < DragThreshold)
        {
            return;
        }
        _pilMoved = true;
        var newLeft = _pilDragOriginLeft + dx;
        var maxLeft = WorkspaceGrid.Bounds.Width - LibraryTogglePill.Width;
        _togglePillLeft = Clamp(newLeft, 0, Math.Max(0, maxLeft));
        Canvas.SetLeft(LibraryTogglePill, _togglePillLeft);
    }

    public void OnPillPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (!_pilMoved)
        {
            ToggleLibrary();
        }
        _pilDragging = false;
        _pilMoved = false;
        e.Pointer.Capture(null);
    }

    public void OnBottomPillKeyDown(object? sender, KeyEventArgs e)
    {
        if (!IsDirectActivation(sender, e))
        {
            return;
        }

        ToggleLibrary();
        e.Handled = true;
    }

    // ===================== 右侧 Pill 拖拽（上下） =====================

    public void OnRightPillPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            return;
        }
        _rightPilDragging = true;
        _rightPilMoved = false;
        _rightPilDragStartY = e.GetPosition(this).Y;
        _rightPilDragOriginTop = _rightPillTop < 0 ? 120 : _rightPillTop;
        e.Pointer.Capture((IInputElement?)sender);
        e.Handled = true;
    }

    public void OnRightPillPointerMoved(object? sender, PointerEventArgs e)
    {
        if (!_rightPilDragging || WorkspaceRightPill is null)
        {
            return;
        }

        Control? host = CanvasHost is not null ? CanvasHost : CanvasOverlay;
        if (host is null)
        {
            return;
        }

        var dy = e.GetPosition(this).Y - _rightPilDragStartY;
        if (!_rightPilMoved && Math.Abs(dy) < DragThreshold)
        {
            return;
        }
        _rightPilMoved = true;
        var newTop = _rightPilDragOriginTop + dy;
        var maxTop = host.Bounds.Height - WorkspaceRightPill.Height;
        _rightPillTop = Clamp(newTop, 0, Math.Max(0, maxTop));
        Canvas.SetTop(WorkspaceRightPill, _rightPillTop);
    }

    public void OnRightPillPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (!_rightPilMoved && DataContext is WorkspacePageViewModel vm)
        {
            vm.IsRightPanelOpen = !vm.IsRightPanelOpen;
        }
        _rightPilDragging = false;
        _rightPilMoved = false;
        e.Pointer.Capture(null);
    }

    public void OnRightPillKeyDown(object? sender, KeyEventArgs e)
    {
        if (!IsDirectActivation(sender, e) || DataContext is not WorkspacePageViewModel viewModel)
        {
            return;
        }

        viewModel.IsRightPanelOpen = !viewModel.IsRightPanelOpen;
        e.Handled = true;
    }

    private static bool IsDirectActivation(object? sender, KeyEventArgs e) =>
        ReferenceEquals(sender, e.Source) && e.Key is Key.Enter or Key.Space;

    // ===================== 节点拖动 =====================

    public void OnNodePointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (FindNodeDataContext(sender as Control) is not { } node
            || !e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            return;
        }

        // 框选进行中时不抢节点拖
        if (_marqueeActive)
        {
            return;
        }

        if (FocusOverviewNodeIfNeeded(node, e))
        {
            return;
        }

        if (DataContext is WorkspacePageViewModel viewModel)
        {
            // Shift/Ctrl 点选：加入/切换多选
            if (e.KeyModifiers.HasFlag(KeyModifiers.Shift)
                || e.KeyModifiers.HasFlag(KeyModifiers.Control))
            {
                var current = viewModel.GetSelectedNodes().ToList();
                if (node.IsSelected)
                {
                    current.Remove(node);
                    viewModel.SelectNodes(current);
                }
                else
                {
                    current.Add(node);
                    viewModel.SelectNodes(current);
                }
            }
            else if (!node.IsSelected)
            {
                node.SelectCommand.Execute(null);
            }

            viewModel.CaptureCanvasHistory();
            viewModel.BeginContinuousCanvasEdit();
        }
        else
        {
            node.SelectCommand.Execute(null);
        }

        _draggedNode = node;
        _nodeDragging = true;
        _nodeDragStart = ToLogicalCanvasPoint(e.GetPosition(CanvasOverlay));
        _nodeDragOriginX = node.X;
        _nodeDragOriginY = node.Y;
        e.Pointer.Capture((IInputElement?)sender);
        e.Handled = true;
    }

    // ===================== 节点库：单击添加 + 页级拖到画布（不用系统 DnD） =====================

    public void OnNodeLibraryItemPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if ((sender as Control)?.DataContext is not NodeLibraryItemViewModel item
            || !e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            return;
        }

        _libraryPointerDown = true;
        _libraryDragging = false;
        _libraryAddedThisGesture = false;
        _libraryDragItem = item;
        _libraryPressOrigin = e.GetPosition(this);
        // 页级捕获：拖过 ScrollViewer / 分割条不会丢
        e.Pointer.Capture(this);
        e.Handled = true;
    }

    public void OnNodeLibraryItemPointerMoved(object? sender, PointerEventArgs e)
    {
        // 捕获在页面上时，Move 由页面处理；芯片上的 Move 作兜底
        HandleLibraryPointerMove(e);
    }

    public void OnNodeLibraryItemPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        HandleLibraryPointerRelease(e);
    }

    public void OnNodeLibraryItemCaptureLost(object? sender, PointerCaptureLostEventArgs e)
    {
        // 按下时会把捕获从芯片转到页面，芯片 CaptureLost 是预期行为，绝不能在这里加节点/清状态。
        // 真正结束只走页面 OnPointerReleased。
    }

    protected override void OnPointerMoved(PointerEventArgs e)
    {
        if (_libraryPointerDown)
        {
            HandleLibraryPointerMove(e);
        }

        base.OnPointerMoved(e);
    }

    protected override void OnPointerReleased(PointerReleasedEventArgs e)
    {
        if (_libraryPointerDown)
        {
            HandleLibraryPointerRelease(e);
            return;
        }

        base.OnPointerReleased(e);
    }

    private void HandleLibraryPointerMove(PointerEventArgs e)
    {
        if (!_libraryPointerDown || _libraryDragItem is null)
        {
            return;
        }

        var pos = e.GetPosition(this);
        var dx = pos.X - _libraryPressOrigin.X;
        var dy = pos.Y - _libraryPressOrigin.Y;
        if (!_libraryDragging && (dx * dx + dy * dy) >= DragThreshold * DragThreshold)
        {
            _libraryDragging = true;
            ShowLibraryDragGhost(_libraryDragItem.Title, pos);
        }

        if (_libraryDragging)
        {
            MoveLibraryDragGhost(pos);
            e.Handled = true;
        }
    }

    private void HandleLibraryPointerRelease(PointerReleasedEventArgs e)
    {
        if (!_libraryPointerDown || _libraryDragItem is null)
        {
            ResetLibraryGesture();
            return;
        }

        try
        {
            if (DataContext is not WorkspacePageViewModel viewModel)
            {
                return;
            }

            if (_libraryDragging)
            {
                // 拖到画布：落点添加；拖到库外其它处：中心兜底
                if (CanvasOverlay is not null
                    && IsPointOver(CanvasOverlay, e.GetPosition(CanvasOverlay)))
                {
                    var logical = ToLogicalCanvasPoint(e.GetPosition(CanvasOverlay));
                    viewModel.AddNodeAt(_libraryDragItem.NodeType, logical.X - 101, logical.Y - 38);
                }
                else
                {
                    viewModel.AddNodeAt(_libraryDragItem.NodeType,
                        120 + (viewModel.Nodes.Count % 4) * 230,
                        80 + (viewModel.Nodes.Count / 4) * 170);
                }

                _libraryAddedThisGesture = true;
                ScheduleFullCanvasSync();
            }
            else if (!_libraryAddedThisGesture)
            {
                // 纯单击
                viewModel.AddNodeAt(_libraryDragItem.NodeType,
                    120 + (viewModel.Nodes.Count % 4) * 230,
                    80 + (viewModel.Nodes.Count / 4) * 170);
                _libraryAddedThisGesture = true;
                ScheduleFullCanvasSync();
            }

            e.Handled = true;
        }
        finally
        {
            e.Pointer.Capture(null);
            ResetLibraryGesture();
        }
    }

    private static bool IsPointOver(Control control, Point localPoint)
    {
        return localPoint.X >= 0
               && localPoint.Y >= 0
               && localPoint.X <= control.Bounds.Width
               && localPoint.Y <= control.Bounds.Height;
    }

    private void ResetLibraryGesture()
    {
        _libraryPointerDown = false;
        _libraryDragging = false;
        _libraryAddedThisGesture = false;
        _libraryDragItem = null;
        HideLibraryDragGhost();
    }

    private void ShowLibraryDragGhost(string title, Point positionInView)
    {
        if (LibraryDragGhost is null || LibraryDragGhostText is null)
        {
            return;
        }

        LibraryDragGhostText.Text = title;
        LibraryDragGhost.IsVisible = true;
        MoveLibraryDragGhost(positionInView);
    }

    private void MoveLibraryDragGhost(Point positionInView)
    {
        if (LibraryDragGhost is null)
        {
            return;
        }

        Canvas.SetLeft(LibraryDragGhost, positionInView.X + 12);
        Canvas.SetTop(LibraryDragGhost, positionInView.Y + 12);
    }

    private void HideLibraryDragGhost()
    {
        if (LibraryDragGhost is not null)
        {
            LibraryDragGhost.IsVisible = false;
        }
    }

    private void ScheduleFullCanvasSync()
    {
        // 容器尚未生成时立刻 Sync 会空跑；多档优先级确保落点可见。
        ScheduleNodeContainerSync();
        ScheduleEdgeSync();
        ScheduleMiniMapSync();
        Dispatcher.UIThread.Post(() =>
        {
            SyncNodeContainerPositions();
            SyncEdgePositions();
            SyncMiniMapPositions();
        }, DispatcherPriority.Loaded);
        Dispatcher.UIThread.Post(() =>
        {
            SyncNodeContainerPositions();
            SyncEdgePositions();
            SyncMiniMapPositions();
        }, DispatcherPriority.Render);
    }

    private void OnWorkspaceKeyDown(object? sender, KeyEventArgs e)
    {
        if (DataContext is not WorkspacePageViewModel viewModel)
        {
            return;
        }

        var hasCommandModifier = e.KeyModifiers.HasFlag(KeyModifiers.Control)
            || e.KeyModifiers.HasFlag(KeyModifiers.Meta);

        // 保存是页面级命令，即使当前正在编辑属性也应可用。
        if (hasCommandModifier && e.Key == Key.S)
        {
            e.Handled = viewModel.SaveCommand.TryExecute();
            return;
        }

        if (e.Key == Key.F11 && e.KeyModifiers == KeyModifiers.None)
        {
            e.Handled = viewModel.ToggleCanvasFocusModeCommand.TryExecute();
            return;
        }

        if (e.Key == Key.Escape && _keyboardEdgeSourceNode is not null)
        {
            CancelKeyboardConnection(announce: true);
            e.Handled = true;
            return;
        }
        if (e.Key == Key.Escape && viewModel.IsCanvasFocusMode)
        {
            e.Handled = viewModel.ToggleCanvasFocusModeCommand.TryExecute();
            return;
        }

        // 输入框内不劫持复制、剪切、粘贴、退格或删除。
        if (IsTextInputFocused())
        {
            return;
        }

        if (e.Key == Key.Space)
        {
            if (IsChildControlFocused())
            {
                return;
            }
            _spacePanMode = true;
            e.Handled = true;
            return;
        }

        if (e.Key == Key.Home && e.KeyModifiers == KeyModifiers.None)
        {
            viewModel.FitViewCommand.TryExecute();
            e.Handled = true;
            return;
        }

        // Delete / Backspace 删选中节点（无修饰键）
        if ((e.Key == Key.Delete || e.Key == Key.Back)
            && !e.KeyModifiers.HasFlag(KeyModifiers.Control)
            && !e.KeyModifiers.HasFlag(KeyModifiers.Meta)
            && !e.KeyModifiers.HasFlag(KeyModifiers.Alt))
        {
            if (viewModel.DeleteSelectedNodeCommand.CanExecute(null))
            {
                viewModel.DeleteSelectedNodeCommand.Execute(null);
                e.Handled = true;
            }
            return;
        }

        // Ctrl/Cmd+A 全选
        if (hasCommandModifier && e.Key is Key.D0 or Key.NumPad0)
        {
            viewModel.ResetZoomCommand.TryExecute();
            e.Handled = true;
            return;
        }
        if (hasCommandModifier && e.Key is Key.OemPlus or Key.Add)
        {
            viewModel.ZoomInCommand.TryExecute();
            e.Handled = true;
            return;
        }
        if (hasCommandModifier && e.Key is Key.OemMinus or Key.Subtract)
        {
            viewModel.ZoomOutCommand.TryExecute();
            e.Handled = true;
            return;
        }
        if (hasCommandModifier && e.Key == Key.A)
        {
            viewModel.SelectNodes(viewModel.Nodes.ToArray());
            e.Handled = true;
            return;
        }
        if (hasCommandModifier && e.Key == Key.C)
        {
            e.Handled = viewModel.CopySelectedNodeCommand.TryExecute();
            return;
        }
        if (hasCommandModifier && e.Key == Key.X)
        {
            e.Handled = viewModel.CutSelectedNodeCommand.TryExecute();
            return;
        }
        if (hasCommandModifier && e.Key == Key.V)
        {
            e.Handled = viewModel.PasteNodeCommand.TryExecute();
            return;
        }

        if (!hasCommandModifier)
        {
            return;
        }
        if (e.Key == Key.Z && e.KeyModifiers.HasFlag(KeyModifiers.Shift))
        {
            if (viewModel.RedoCommand.CanExecute(null))
            {
                viewModel.RedoCommand.Execute(null);
                e.Handled = true;
            }
            return;
        }
        if (e.Key == Key.Z)
        {
            if (viewModel.UndoCommand.CanExecute(null))
            {
                viewModel.UndoCommand.Execute(null);
                e.Handled = true;
            }
            return;
        }
        if (e.Key == Key.Y)
        {
            if (viewModel.RedoCommand.CanExecute(null))
            {
                viewModel.RedoCommand.Execute(null);
                e.Handled = true;
            }
        }
    }

    private void OnWorkspaceKeyUp(object? sender, KeyEventArgs e)
    {
        if (e.Key != Key.Space)
        {
            return;
        }

        if (!IsChildControlFocused())
        {
            _spacePanMode = false;
            e.Handled = true;
        }
    }

    private bool IsChildControlFocused()
    {
        var focus = TopLevel.GetTopLevel(this)?.FocusManager?.GetFocusedElement();
        // 空格平移只在页面/画布本身持有焦点时启用；任何可聚焦子控件都保留自己的空格激活语义。
        return focus is Control control
               && !ReferenceEquals(control, this)
               && control.Focusable;
    }

    private bool IsTextInputFocused()
    {
        var focus = TopLevel.GetTopLevel(this)?.FocusManager?.GetFocusedElement();
        if (focus is null)
        {
            return false;
        }

        // 焦点在文本输入控件内时不拦截 Delete/Backspace
        if (focus is TextBox or ComboBox)
        {
            return true;
        }

        if (focus is Control control)
        {
            for (var c = control; c is not null; c = c.Parent as Control)
            {
                if (c is TextBox or ComboBox)
                {
                    return true;
                }
            }
        }

        return false;
    }

    // ===================== 空白处左键框选 =====================

    public void OnCanvasBackgroundPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (CanvasOverlay is null || DataContext is not WorkspacePageViewModel)
        {
            return;
        }

        var point = e.GetCurrentPoint(CanvasOverlay);
        // W2：中键、Alt+左键或空格+左键开始平移（产品路径，非仅小地图）。
        if (point.Properties.IsMiddleButtonPressed
            || (point.Properties.IsLeftButtonPressed
                && (e.KeyModifiers.HasFlag(KeyModifiers.Alt) || _spacePanMode)))
        {
            BeginPan(e.GetPosition(CanvasOverlay));
            e.Pointer.Capture(CanvasOverlay);
            e.Handled = true;
            Focus();
            return;
        }

        if (!point.Properties.IsLeftButtonPressed)
        {
            return;
        }

        // 点在节点 / 边 / 端口 / pill 上时不启动框选（它们会 Handled）
        if (e.Source is Control src)
        {
            for (var c = src; c is not null && c != CanvasOverlay; c = c.Parent as Control)
            {
                if (c.DataContext is WorkflowNodeViewModel or WorkflowEdgeViewModel)
                {
                    return;
                }

                if (c.Name is "MiniMapHost" or "MiniMapCanvas" or "PillLayer"
                    or "LibraryTogglePill" or "WorkspaceRightPill" or "TopToolbar"
                    or "CanvasStatusHost")
                {
                    return;
                }
            }
        }

        _marqueePointerDown = true;
        _marqueeActive = false;
        _marqueeAdditive = e.KeyModifiers.HasFlag(KeyModifiers.Shift)
                           || e.KeyModifiers.HasFlag(KeyModifiers.Control);
        _marqueeOriginLogical = ToLogicalCanvasPoint(e.GetPosition(CanvasOverlay));
        _marqueeCurrentLogical = _marqueeOriginLogical;
        e.Pointer.Capture(CanvasOverlay);
        // 不 Handled：允许其他层仍可接收；但已 capture
        Focus();
    }

    public void OnCanvasBackgroundPointerMoved(object? sender, PointerEventArgs e)
    {
        if (DataContext is WorkspacePageViewModel { CanvasViewport.IsPanning: true } panViewModel
            && CanvasOverlay is not null
            && NodesItemsControl is not null)
        {
            var pos = e.GetPosition(CanvasOverlay);
            var state = panViewModel.CanvasViewport.UpdatePan(pos.X, pos.Y);
            ApplyCanvasViewportState(state);
            e.Handled = true;
            return;
        }

        if (!_marqueePointerDown || CanvasOverlay is null)
        {
            return;
        }

        var logical = ToLogicalCanvasPoint(e.GetPosition(CanvasOverlay));
        var dx = logical.X - _marqueeOriginLogical.X;
        var dy = logical.Y - _marqueeOriginLogical.Y;
        if (!_marqueeActive
            && !CanvasSelectionHelpers.ExceedsMarqueeThreshold(dx, dy, DragThreshold))
        {
            return;
        }

        _marqueeActive = true;
        _marqueeCurrentLogical = logical;
        UpdateMarqueeVisual();
        e.Handled = true;
    }

    public void OnCanvasBackgroundPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (DataContext is WorkspacePageViewModel { CanvasViewport.IsPanning: true })
        {
            EndPan(e.Pointer);
            e.Handled = true;
            return;
        }

        if (!_marqueePointerDown)
        {
            return;
        }

        if (_marqueeActive && DataContext is WorkspacePageViewModel vm)
        {
            var hits = vm.HitTestNodesInRect(
                _marqueeOriginLogical.X, _marqueeOriginLogical.Y,
                _marqueeCurrentLogical.X, _marqueeCurrentLogical.Y);
            vm.SelectNodes(hits, additive: _marqueeAdditive);
            e.Handled = true;
        }
        else if (!_marqueeActive && DataContext is WorkspacePageViewModel page
                 && !e.KeyModifiers.HasFlag(KeyModifiers.Shift)
                 && !e.KeyModifiers.HasFlag(KeyModifiers.Control))
        {
            // 单击空白：取消选中
            page.SelectNode(null);
        }

        EndMarquee(e.Pointer);
    }

    public void OnCanvasBackgroundCaptureLost(object? sender, PointerCaptureLostEventArgs e)
    {
        EndPan(null);
        EndMarquee(null);
    }

    /// <summary>指针滚轮缩放：保持鼠标下的逻辑点固定。</summary>
    public void OnCanvasPointerWheel(object? sender, PointerWheelEventArgs e)
    {
        if (DataContext is not WorkspacePageViewModel vm || CanvasOverlay is null)
        {
            return;
        }

        var next = CanvasViewportHelpers.ApplyWheelZoom(vm.CanvasZoom, e.Delta.Y);
        if (Math.Abs(next - vm.CanvasZoom) < 1e-9)
        {
            return;
        }

        SetCanvasZoomAt(next, e.GetPosition(CanvasOverlay));
        e.Handled = true;
    }

    private void ZoomCanvasAtCenterBy(double delta)
    {
        if (DataContext is not WorkspacePageViewModel vm)
        {
            return;
        }

        SetCanvasZoomAt(vm.CanvasZoom + delta, CanvasViewportCenter());
    }

    private void ResetCanvasZoomAtCenter()
    {
        SetCanvasZoomAt(1.0, CanvasViewportCenter());
    }

    private Point CanvasViewportCenter()
    {
        return CanvasOverlay is null
            ? default
            : new Point(CanvasOverlay.Bounds.Width * 0.5, CanvasOverlay.Bounds.Height * 0.5);
    }

    private void SetCanvasZoomAt(double requestedZoom, Point anchor)
    {
        if (DataContext is not WorkspacePageViewModel vm)
        {
            return;
        }

        var previous = vm.CanvasViewport.Current;
        var state = vm.SetCanvasZoomAt(requestedZoom, anchor.X, anchor.Y);
        if (Math.Abs(state.Zoom - previous.Zoom) < 1e-9)
        {
            return;
        }

        ApplyCanvasViewportState(state);
        vm.StatusText = vm.CanvasZoomText;
    }

    private void BeginPan(Point screen)
    {
        if (DataContext is not WorkspacePageViewModel viewModel)
        {
            return;
        }

        viewModel.CanvasViewport.BeginPan(screen.X, screen.Y);
    }

    private void EndPan(IPointer? pointer)
    {
        if (DataContext is WorkspacePageViewModel viewModel)
        {
            viewModel.CanvasViewport.EndPan();
        }
        pointer?.Capture(null);
    }

    private void ApplyCanvasOffset(double offsetX, double offsetY)
    {
        if (DataContext is not WorkspacePageViewModel viewModel)
        {
            return;
        }

        ApplyCanvasViewportState(viewModel.CanvasViewport.SetOffset(offsetX, offsetY));
    }

    private void ApplyCanvasViewportState(CanvasViewportState state)
    {
        if (NodesItemsControl is null)
        {
            return;
        }

        var transform = EnsureTranslateTransform(NodesItemsControl);
        transform.X = state.OffsetX;
        transform.Y = state.OffsetY;
        if (EdgesItemsControl is not null)
        {
            var edgeTransform = EnsureTranslateTransform(EdgesItemsControl);
            edgeTransform.X = state.OffsetX;
            edgeTransform.Y = state.OffsetY;
        }

        SyncMiniMapViewportFrame();
    }

    private void EndMarquee(IPointer? pointer)
    {
        _marqueePointerDown = false;
        _marqueeActive = false;
        pointer?.Capture(null);
        if (MarqueeRect is not null)
        {
            MarqueeRect.IsVisible = false;
        }
    }

    private void UpdateMarqueeVisual()
    {
        if (MarqueeRect is null || CanvasOverlay is null)
        {
            return;
        }

        var (lx, ly, lw, lh) = CanvasSelectionHelpers.NormalizeRect(
            _marqueeOriginLogical.X, _marqueeOriginLogical.Y,
            _marqueeCurrentLogical.X, _marqueeCurrentLogical.Y);

        // 逻辑 → 屏幕（与节点层同一缩放/平移）
        var zoom = CurrentCanvasZoom();
        var offset = CurrentCanvasOffset();
        var sx = lx * zoom + offset.X;
        var sy = ly * zoom + offset.Y;
        var sw = lw * zoom;
        var sh = lh * zoom;

        Canvas.SetLeft(MarqueeRect, sx);
        Canvas.SetTop(MarqueeRect, sy);
        MarqueeRect.Width = Math.Max(1, sw);
        MarqueeRect.Height = Math.Max(1, sh);
        MarqueeRect.IsVisible = true;
    }

    public void OnNodePointerMoved(object? sender, PointerEventArgs e)
    {
        if (!_nodeDragging || _draggedNode is not { } node || CanvasOverlay is null)
        {
            return;
        }

        var position = ToLogicalCanvasPoint(e.GetPosition(CanvasOverlay));
        var newX = _nodeDragOriginX + position.X - _nodeDragStart.X;
        var newY = _nodeDragOriginY + position.Y - _nodeDragStart.Y;
        var zoom = CurrentCanvasZoom();
        var offset = CurrentCanvasOffset();
        var (safeX, safeY) = CanvasViewportHelpers.KeepNodeReachable(
            newX,
            newY,
            NodePortSpec.NodeWidth,
            node.CanvasHeight,
            zoom,
            offset.X,
            offset.Y,
            CanvasOverlay.Bounds.Width,
            CanvasOverlay.Bounds.Height,
            CanvasOcclusionRects());
        // C5-a：只写逻辑坐标；主节点布局与相邻边 Geometry 在 Render 帧回调中合并执行。
        node.X = safeX;
        node.Y = safeY;
        if (!CanvasDragFrameHelpers.ShouldApplyMainVisualsOnPointerMoved)
        {
            ScheduleDragFrameSync();
        }
        else
        {
            SyncDraggedNode(node);
            SyncConnectedEdges(node.Id);
            ScheduleDragFrameSync();
        }
        e.Handled = true;
    }

    public void OnNodePointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (!_nodeDragging)
        {
            return;
        }

        // C5-a：松手前同步 flush 主视觉（节点 Canvas 位 + 邻边 Geometry），
        // 避免先清空 _draggedNode 导致挂起的 Render 回调空转漏最后一帧。
        FlushDragFrameSyncNow();
        if (_draggedNode is { } draggedNode)
        {
            CommitDraggedNodeLayout(draggedNode);
        }
        _nodeDragging = false;
        if (DataContext is WorkspacePageViewModel viewModel)
        {
            viewModel.EndContinuousCanvasEdit();
        }
        _draggedNode = null;
        SyncMiniMapPositions();
        ScheduleEdgeLabelLayout();
        e.Pointer.Capture(null);
        e.Handled = true;
    }

    public void OnNodePointerCaptureLost(object? sender, PointerCaptureLostEventArgs e)
    {
        if (!_nodeDragging)
        {
            return;
        }
        FlushDragFrameSyncNow();
        if (_draggedNode is { } draggedNode)
        {
            CommitDraggedNodeLayout(draggedNode);
        }
        _nodeDragging = false;
        if (DataContext is WorkspacePageViewModel viewModel)
        {
            viewModel.EndContinuousCanvasEdit();
        }
        _draggedNode = null;
        SyncMiniMapPositions();
        ScheduleEdgeLabelLayout();
    }

    public void OnNodeSelectPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (FindNodeDataContext(sender as Control) is { } node)
        {
            node.SelectCommand.Execute(null);
            FocusOverviewNodeIfNeeded(node, e);
        }
    }

    public void OnNodeCardKeyDown(object? sender, KeyEventArgs e)
    {
        if (!ReferenceEquals(sender, e.Source)
            || FindNodeDataContext(sender as Control) is not { } node
            || DataContext is not WorkspacePageViewModel viewModel)
        {
            return;
        }

        if (e.Key is Key.Enter or Key.Space)
        {
            viewModel.SelectNode(node);
            e.Handled = true;
            return;
        }

        if (e.KeyModifiers != KeyModifiers.None)
        {
            return;
        }

        var direction = e.Key switch
        {
            Key.Left => CanvasKeyboardDirection.Left,
            Key.Right => CanvasKeyboardDirection.Right,
            Key.Up => CanvasKeyboardDirection.Up,
            Key.Down => CanvasKeyboardDirection.Down,
            _ => (CanvasKeyboardDirection?)null,
        };
        if (direction is null)
        {
            return;
        }

        var candidates = viewModel.Nodes
            .Select(candidate => new CanvasKeyboardNode(
                candidate.Id,
                candidate.X,
                candidate.Y,
                NodePortSpec.NodeWidth,
                candidate.CanvasHeight))
            .ToArray();
        var targetId = CanvasKeyboardNavigationHelpers.FindDirectionalNode(
            node.Id,
            candidates,
            direction.Value);
        var target = viewModel.Nodes.FirstOrDefault(candidate =>
            string.Equals(candidate.Id, targetId, StringComparison.Ordinal));
        if (target is null)
        {
            return;
        }

        viewModel.SelectNode(target);
        FocusNodeCard(target);
        e.Handled = true;
    }

    private void FocusNodeCard(WorkflowNodeViewModel node)
    {
        if (TryFocusNodeCard(node))
        {
            return;
        }

        ScheduleNodeContainerSync();
        Dispatcher.UIThread.Post(() => TryFocusNodeCard(node), DispatcherPriority.Loaded);
    }

    private bool TryFocusNodeCard(WorkflowNodeViewModel node)
    {
        var container = FindNodeContainer(node, NodesItemsControl, _nodeContainersById);
        var focusHost = container?
            .GetVisualDescendants()
            .OfType<Control>()
            .FirstOrDefault(control => control.Name == "NodeKeyboardFocusHost");
        return focusHost?.Focus() == true;
    }

    private bool FocusOverviewNodeIfNeeded(WorkflowNodeViewModel node, PointerPressedEventArgs e)
    {
        if (DataContext is not WorkspacePageViewModel viewModel
            || viewModel.ShowCanvasPrecisionControls
            || CanvasOverlay is null)
        {
            return false;
        }

        node.SelectCommand.Execute(null);
        SetCanvasZoomAt(
            CanvasSemanticZoomHelpers.FocusZoom,
            e.GetPosition(CanvasOverlay));
        viewModel.StatusText = viewModel.CanvasOverviewFocusText;
        e.Handled = true;
        return true;
    }

    public void OnEdgePointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            return;
        }

        // 从 Path / 标签向上找边 VM
        var control = sender as Control;
        while (control is not null)
        {
            if (control.DataContext is WorkflowEdgeViewModel edge
                && DataContext is WorkspacePageViewModel page)
            {
                edge.SelectCommand.Execute(null);
                e.Handled = true;
                return;
            }
            control = control.Parent as Control;
        }
    }

    public void OnPortPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (FindNodeDataContext(sender as Control) is not { } node
            || !e.GetCurrentPoint(this).Properties.IsLeftButtonPressed
            || !TryReadPortTag(sender as Control, out var kind, out var direction, out var handle)
            || DataContext is not WorkspacePageViewModel viewModel)
        {
            return;
        }

        CancelKeyboardConnection(announce: false);

        if (!viewModel.ShowCanvasPrecisionControls)
        {
            node.SelectCommand.Execute(null);
            viewModel.StatusText = viewModel.CanvasOverviewFocusText;
            e.Handled = true;
            return;
        }

        _edgeDragging = true;
        _edgeSourceNode = node;
        _edgeSourceKind = kind;
        _edgeSourceDirection = direction;
        _edgeSourceHandle = handle ?? NodePortSpec.HandleName(kind, direction);
        var (lx, ly) = NodePortSpec.LocalCenterForHandle(_edgeSourceHandle);
        _rubberBandStartLogical = new Point(node.X + lx, node.Y + ly);
        node.SelectCommand.Execute(null);
        viewModel.BeginPortDragHighlight(node.Id, kind, direction);
        UpdateRubberBand(_rubberBandStartLogical);
        e.Pointer.Capture((IInputElement?)sender);
        e.Handled = true;
    }

    public void OnPortKeyDown(object? sender, KeyEventArgs e)
    {
        if (!ReferenceEquals(sender, e.Source)
            || e.Key is not (Key.Enter or Key.Space)
            || FindNodeDataContext(sender as Control) is not { } node
            || !TryReadPortTag(sender as Control, out var kind, out var direction, out var handle)
            || DataContext is not WorkspacePageViewModel viewModel
            || !viewModel.ShowCanvasPrecisionControls)
        {
            return;
        }

        node.SelectCommand.Execute(null);
        handle ??= NodePortSpec.HandleName(kind, direction);
        if (_keyboardEdgeSourceNode is null)
        {
            _keyboardEdgeSourceNode = node;
            _keyboardEdgeSourceKind = kind;
            _keyboardEdgeSourceDirection = direction;
            _keyboardEdgeSourceHandle = handle;
            viewModel.BeginPortDragHighlight(node.Id, kind, direction);
            viewModel.NotifyKeyboardConnectStarted();
            e.Handled = true;
            return;
        }

        if (viewModel.TryConnectPorts(
                _keyboardEdgeSourceNode.Id,
                _keyboardEdgeSourceKind,
                _keyboardEdgeSourceDirection,
                node.Id,
                kind,
                direction,
                _keyboardEdgeSourceHandle,
                handle))
        {
            CancelKeyboardConnection(announce: false);
            ScheduleFullCanvasSync();
        }

        e.Handled = true;
    }

    private void CancelKeyboardConnection(bool announce)
    {
        if (_keyboardEdgeSourceNode is null)
        {
            return;
        }

        if (DataContext is WorkspacePageViewModel viewModel)
        {
            viewModel.EndPortDragHighlight();
            if (announce)
            {
                viewModel.NotifyKeyboardConnectCancelled();
            }
        }

        _keyboardEdgeSourceNode = null;
        _keyboardEdgeSourceHandle = null;
    }

    public void OnPortPointerMoved(object? sender, PointerEventArgs e)
    {
        if (!_edgeDragging || CanvasOverlay is null)
        {
            return;
        }

        var logical = ToLogicalCanvasPoint(e.GetPosition(CanvasOverlay));
        UpdateRubberBand(logical);
        e.Handled = true;
    }

    public void OnPortPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (!_edgeDragging || _edgeSourceNode is null || DataContext is not WorkspacePageViewModel viewModel)
        {
            ResetEdgeDrag();
            e.Pointer.Capture(null);
            return;
        }

        var logical = ToLogicalCanvasPoint(e.GetPosition(CanvasOverlay));
        // 优先命中具体端口；未点到端口时，若落在节点上则尝试同类型入/双向口。
        if (TryFindPortAt(logical, out var targetNode, out var targetKind, out var targetDirection, out var targetHandle)
            && targetNode is not null)
        {
            if (viewModel.TryConnectPorts(
                    _edgeSourceNode.Id, _edgeSourceKind, _edgeSourceDirection,
                    targetNode.Id, targetKind, targetDirection,
                    _edgeSourceHandle, targetHandle))
            {
                SyncEdgePositions();
            }
        }
        else if (FindNodeAt(logical) is { } node && node != _edgeSourceNode)
        {
            // 松手在节点体上：自动落到同类型的可接收端（入/双向）；数据入优先第一个空闲 pin
            var receiveDir = _edgeSourceKind == NodePortKind.Communication
                ? NodePortDirection.Both
                : NodePortDirection.In;
            string? freeIn = null;
            if (_edgeSourceKind == NodePortKind.Data && receiveDir == NodePortDirection.In)
            {
                freeIn = node.DataInPins.FirstOrDefault(p => !p.IsConnected)?.Handle
                         ?? node.DataInPins.FirstOrDefault()?.Handle;
            }

            if (viewModel.TryConnectPorts(
                    _edgeSourceNode.Id, _edgeSourceKind, _edgeSourceDirection,
                    node.Id, _edgeSourceKind, receiveDir,
                    _edgeSourceHandle, freeIn))
            {
                SyncEdgePositions();
            }
        }
        else
        {
            viewModel.NotifyConnectMissed();
        }

        ResetEdgeDrag();
        e.Pointer.Capture(null);
        e.Handled = true;
    }

    // 兼容旧 XAML 名（若外部仍引用）
    public void OnOutputPortPointerPressed(object? sender, PointerPressedEventArgs e) => OnPortPointerPressed(sender, e);
    public void OnOutputPortPointerMoved(object? sender, PointerEventArgs e) => OnPortPointerMoved(sender, e);
    public void OnOutputPortPointerReleased(object? sender, PointerReleasedEventArgs e) => OnPortPointerReleased(sender, e);

    public void OnMiniMapPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed
            || MiniMapCanvas is null
            || NodesItemsControl is null
            || CanvasOverlay is null)
        {
            return;
        }

        var miniPos = e.GetPosition(MiniMapCanvas);
        var (logicalX, logicalY) = _miniMapTransform.MiniMapToLogical(miniPos.X, miniPos.Y);
        var zoom = CurrentCanvasZoom();
        var viewW = CanvasOverlay.Bounds.Width / zoom;
        var viewH = CanvasOverlay.Bounds.Height / zoom;
        // 点击处对齐主视口中心。
        var targetLeft = logicalX - (viewW * 0.5);
        var targetTop = logicalY - (viewH * 0.5);
        ApplyCanvasOffset(-targetLeft * zoom, -targetTop * zoom);
        e.Handled = true;
    }

    private void UpdateRubberBand(Point endLogical)
    {
        if (RubberBandPath is null)
        {
            return;
        }

        var zoom = CurrentCanvasZoom();
        var offset = CurrentCanvasOffset();
        var startScreen = new Point(
            (_rubberBandStartLogical.X * zoom) + offset.X,
            (_rubberBandStartLogical.Y * zoom) + offset.Y);
        var endScreen = new Point(
            (endLogical.X * zoom) + offset.X,
            (endLogical.Y * zoom) + offset.Y);
        // 橡皮筋与正式边同算法；通信口预览即「上拱跳线」
        var isComm = _edgeSourceKind == NodePortKind.Communication;
        var spec = NodePortSpec.BuildEdgePath(
            startScreen.X, startScreen.Y, endScreen.X, endScreen.Y, isComm);
        var geometry = new PathGeometry();
        var figure = new PathFigure
        {
            StartPoint = spec.Start,
            IsClosed = false,
            IsFilled = false,
        };
        figure.Segments ??= new PathSegments();
        figure.Segments.Add(new BezierSegment
        {
            Point1 = spec.Control1,
            Point2 = spec.Control2,
            Point3 = spec.End,
        });
        geometry.Figures ??= new PathFigures();
        geometry.Figures.Add(figure);
        RubberBandPath.Data = geometry;
        RubberBandPath.Stroke = BrushForPortKind(_edgeSourceKind);
        RubberBandPath.StrokeThickness = isComm ? 2.2 : 1.8;
        RubberBandPath.IsVisible = true;
    }

    private void ClearRubberBand()
    {
        if (RubberBandPath is null)
        {
            return;
        }

        RubberBandPath.IsVisible = false;
        RubberBandPath.Data = null;
    }

    private static IBrush BrushForPortKind(NodePortKind kind) => kind switch
    {
        NodePortKind.Control => new SolidColorBrush(Color.Parse("#8B939D")),
        NodePortKind.Communication => new SolidColorBrush(Color.Parse("#7C3AED")),
        _ => new SolidColorBrush(Color.Parse("#2E726B")),
    };

    private void ResetEdgeDrag()
    {
        if (DataContext is WorkspacePageViewModel viewModel)
        {
            viewModel.EndPortDragHighlight();
        }
        ClearRubberBand();
        _edgeDragging = false;
        _edgeSourceNode = null;
        _edgeSourceHandle = null;
    }

    private WorkflowNodeViewModel? FindNodeAt(Point canvasPosition)
    {
        if (DataContext is not WorkspacePageViewModel viewModel)
        {
            return null;
        }

        return viewModel.Nodes.LastOrDefault(node =>
            canvasPosition.X >= node.X
            && canvasPosition.X <= node.X + NodePortSpec.NodeWidth
            && canvasPosition.Y >= node.Y
            && canvasPosition.Y <= node.Y + node.CanvasHeight);
    }

    private bool TryFindPortAt(
        Point canvasPosition,
        out WorkflowNodeViewModel? node,
        out NodePortKind kind,
        out NodePortDirection direction,
        out string? handle)
    {
        node = null;
        kind = NodePortKind.Data;
        direction = NodePortDirection.In;
        handle = null;
        if (DataContext is not WorkspacePageViewModel viewModel)
        {
            return false;
        }

        WorkflowNodeViewModel? bestNode = null;
        NodePortKind bestKind = NodePortKind.Data;
        NodePortDirection bestDir = NodePortDirection.In;
        string? bestHandle = null;
        var bestDist = double.MaxValue;

        foreach (var item in viewModel.Nodes)
        {
            // 执行 + 通信 + 数据出 + 全部数据入
            var handles = new List<(string Handle, NodePortKind Kind, NodePortDirection Dir)>
            {
                ("exec_in", NodePortKind.Control, NodePortDirection.In),
                ("exec_out", NodePortKind.Control, NodePortDirection.Out),
                ("communication", NodePortKind.Communication, NodePortDirection.Both),
                ("output", NodePortKind.Data, NodePortDirection.Out),
            };
            foreach (var pin in item.DataInPins)
            {
                handles.Add((pin.Handle, NodePortKind.Data, NodePortDirection.In));
            }

            foreach (var (h, portKind, portDir) in handles)
            {
                var (lx, ly) = NodePortSpec.LocalCenterForHandle(h);
                var cx = item.X + lx;
                var cy = item.Y + ly;
                var dx = canvasPosition.X - cx;
                var dy = canvasPosition.Y - cy;
                var dist = Math.Sqrt((dx * dx) + (dy * dy));
                if (dist <= NodePortSpec.HitRadius && dist < bestDist)
                {
                    bestDist = dist;
                    bestNode = item;
                    bestKind = portKind;
                    bestDir = portDir;
                    bestHandle = h;
                }
            }
        }

        if (bestNode is null)
        {
            return false;
        }

        node = bestNode;
        kind = bestKind;
        direction = bestDir;
        handle = bestHandle;
        return true;
    }

    private static bool TryReadPortTag(
        Control? control,
        out NodePortKind kind,
        out NodePortDirection direction,
        out string? handle)
    {
        kind = NodePortKind.Data;
        direction = NodePortDirection.Out;
        handle = null;
        while (control is not null)
        {
            if (control.Tag is string tag && TryParsePortTag(tag, out kind, out direction, out handle))
            {
                return true;
            }
            control = control.Parent as Control;
        }
        return false;
    }

    private static bool TryParsePortTag(
        string tag,
        out NodePortKind kind,
        out NodePortDirection direction,
        out string? handle)
    {
        kind = NodePortKind.Data;
        direction = NodePortDirection.Out;
        handle = null;
        var parts = tag.Split('|', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        if (parts.Length < 2)
        {
            return false;
        }

        kind = parts[0].ToLowerInvariant() switch
        {
            "control" => NodePortKind.Control,
            "communication" => NodePortKind.Communication,
            _ => NodePortKind.Data,
        };
        direction = parts[1].ToLowerInvariant() switch
        {
            "in" => NodePortDirection.In,
            "both" => NodePortDirection.Both,
            _ => NodePortDirection.Out,
        };
        // data|in|data-in-1 第三段为 handle
        if (parts.Length >= 3)
        {
            handle = parts[2];
        }
        else
        {
            handle = NodePortSpec.HandleName(kind, direction);
        }

        return true;
    }

    private static WorkflowNodeViewModel? FindNodeDataContext(Control? control)
    {
        while (control is not null)
        {
            if (control.DataContext is WorkflowNodeViewModel node)
            {
                return node;
            }
            control = control.Parent as Control;
        }
        return null;
    }

    private void RebuildCanvasIndexes(WorkspacePageViewModel viewModel)
    {
        _nodesById.Clear();
        _edgesByNodeId.Clear();
        // ItemsControl 在集合变化后会重建容器，旧引用不可继续用于拖拽热路径。
        _nodeContainersById.Clear();
        _miniMapContainersById.Clear();
        foreach (var node in viewModel.Nodes)
        {
            _nodesById[node.Id] = node;
        }
        foreach (var edge in viewModel.Edges)
        {
            AddEdgeIndex(edge.Source, edge);
            if (!string.Equals(edge.Source, edge.Target, StringComparison.Ordinal))
            {
                AddEdgeIndex(edge.Target, edge);
            }
        }
    }

    private void AddEdgeIndex(string nodeId, WorkflowEdgeViewModel edge)
    {
        if (!_edgesByNodeId.TryGetValue(nodeId, out var edges))
        {
            edges = new List<WorkflowEdgeViewModel>();
            _edgesByNodeId[nodeId] = edges;
        }
        edges.Add(edge);
    }

    private void SyncDraggedNode(WorkflowNodeViewModel node)
    {
        if (FindNodeContainer(node, NodesItemsControl, _nodeContainersById) is not { } container)
        {
            return;
        }
        ApplyTransientCanvasPosition(container, node.X, node.Y);
    }

    private void SyncConnectedEdges(string nodeId)
    {
        if (!_edgesByNodeId.TryGetValue(nodeId, out var edges))
        {
            return;
        }
        foreach (var edge in edges)
        {
            if (_nodesById.TryGetValue(edge.Source, out var source)
                && _nodesById.TryGetValue(edge.Target, out var target))
            {
                edge.UpdateEdgePath(source.X, source.Y, target.X, target.Y);
            }
        }
    }

    private void ScheduleDragFrameSync()
    {
        // C5-a：一帧只调度一次主视觉同步（节点容器 + 邻接边 + 小地图）。
        if (!CanvasDragFrameHelpers.TryScheduleFrameSync(ref _dragFrameScheduled))
        {
            return;
        }

        Dispatcher.UIThread.Post(() =>
        {
            CanvasDragFrameHelpers.OnFrameSyncStarted(ref _dragFrameScheduled);
            // Only apply if still dragging; release path uses FlushDragFrameSyncNow instead.
            if (_nodeDragging && _draggedNode is { } node)
            {
                ApplyDraggedNodeVisuals(node);
            }
        }, DispatcherPriority.Render);
    }

    /// <summary>
    /// C5-a：立即应用当前拖拽节点的主视觉并取消挂起帧回调（release / capture-lost）。
    /// </summary>
    private void FlushDragFrameSyncNow()
    {
        CanvasDragFrameHelpers.OnFrameSyncStarted(ref _dragFrameScheduled);
        if (_draggedNode is { } node)
        {
            ApplyDraggedNodeVisuals(node);
        }
    }

    private void ApplyDraggedNodeVisuals(WorkflowNodeViewModel node)
    {
        SyncDraggedNode(node);
        SyncConnectedEdges(node.Id);
        SyncDraggedMiniMapNode(node);
    }

    /// <summary>发布性能探针：准备真实 ItemsControl 容器和画布索引。</summary>
    internal void PrepareReleaseProbe()
    {
        if (DataContext is not WorkspacePageViewModel viewModel)
        {
            return;
        }

        RebuildCanvasIndexes(viewModel);
    }

    /// <summary>发布性能探针：在真实 render callback 内执行拖拽视觉同步。</summary>
    internal void ApplyReleaseProbeDragFrame(WorkflowNodeViewModel node, double x, double y)
    {
        var previousDragging = _nodeDragging;
        var previousNode = _draggedNode;
        _nodeDragging = true;
        _draggedNode = node;
        try
        {
            node.X = x;
            node.Y = y;
            ApplyDraggedNodeVisuals(node);
        }
        finally
        {
            _draggedNode = previousNode;
            _nodeDragging = previousDragging;
        }
    }

    /// <summary>发布性能探针：模拟松手并将 render transform 一次性提交回布局坐标。</summary>
    internal void CompleteReleaseProbeDrag(WorkflowNodeViewModel node)
    {
        CommitDraggedNodeLayout(node);
    }

    private void SyncDraggedMiniMapNode(WorkflowNodeViewModel node)
    {
        if (FindNodeContainer(node, MiniMapItemsControl, _miniMapContainersById) is not { } container)
        {
            return;
        }
        var (miniX, miniY) = MiniMapMarkerPosition(node);
        ApplyTransientCanvasPosition(container, miniX, miniY);
    }

    private void CommitDraggedNodeLayout(WorkflowNodeViewModel node)
    {
        if (FindNodeContainer(node, NodesItemsControl, _nodeContainersById) is { } nodeContainer)
        {
            CommitCanvasPosition(nodeContainer, node.X, node.Y);
        }
        if (FindNodeContainer(node, MiniMapItemsControl, _miniMapContainersById) is { } miniMapContainer)
        {
            var (miniX, miniY) = MiniMapMarkerPosition(node);
            CommitCanvasPosition(miniMapContainer, miniX, miniY);
        }
    }

    private static void ApplyTransientCanvasPosition(Control container, double x, double y)
    {
        var layoutX = Canvas.GetLeft(container);
        var layoutY = Canvas.GetTop(container);
        var translate = EnsureTranslateTransform(container);
        translate.X = x - (double.IsNaN(layoutX) ? 0 : layoutX);
        translate.Y = y - (double.IsNaN(layoutY) ? 0 : layoutY);
    }

    private static void CommitCanvasPosition(Control container, double x, double y)
    {
        Canvas.SetLeft(container, x);
        Canvas.SetTop(container, y);
        var translate = EnsureTranslateTransform(container);
        translate.X = 0;
        translate.Y = 0;
    }

    private void SyncNodeContainerPositions()
    {
        if (NodesItemsControl is null || DataContext is not WorkspacePageViewModel viewModel)
        {
            return;
        }

        // 优先对 ItemsControl 容器设 Canvas 附加属性（DataTemplate 根上的 Canvas.Left 常不生效）
        foreach (var node in viewModel.Nodes)
        {
            if (FindNodeContainer(node, NodesItemsControl, _nodeContainersById) is { } container)
            {
                Canvas.SetLeft(container, node.X);
                Canvas.SetTop(container, node.Y);
            }
        }

        // 兜底：遍历视觉树（旧路径）
        SyncNodeContainerPositions(NodesItemsControl);
    }

    private void FitViewToNodes()
    {
        if (DataContext is not WorkspacePageViewModel { Nodes.Count: > 0 } viewModel
            || NodesItemsControl is null
            || CanvasOverlay is null)
        {
            return;
        }

        // W2：真实包围盒 + 视口尺寸；非仅非负左上角微调。
        var minX = viewModel.Nodes.Min(node => node.X);
        var minY = viewModel.Nodes.Min(node => node.Y);
        var maxX = viewModel.Nodes.Max(node => node.X + NodePortSpec.NodeWidth);
        var maxY = viewModel.Nodes.Max(node => node.Y + node.CanvasHeight);
        var safeViewport = SafeFitViewport();
        var state = viewModel.FitCanvasViewport(minX, minY, maxX, maxY, safeViewport);
        ApplyCanvasViewportState(state);

        SyncNodeContainerPositions();
        SyncEdgePositions();
        SyncMiniMapViewportFrame();
    }

    private void EnsureNodeInSafeViewport(WorkflowNodeViewModel node)
    {
        if (CanvasOverlay is null)
        {
            return;
        }

        var zoom = CurrentCanvasZoom();
        var offset = CurrentCanvasOffset();
        var (safeX, safeY) = CanvasViewportHelpers.KeepNodeReachable(
            node.X,
            node.Y,
            NodePortSpec.NodeWidth,
            node.CanvasHeight,
            zoom,
            offset.X,
            offset.Y,
            CanvasOverlay.Bounds.Width,
            CanvasOverlay.Bounds.Height,
            CanvasOcclusionRects());
        node.X = safeX;
        node.Y = safeY;
    }

    private IReadOnlyList<CanvasViewportRect> CanvasOcclusionRects()
    {
        if (CanvasOverlay is null)
        {
            return Array.Empty<CanvasViewportRect>();
        }

        var rects = new List<CanvasViewportRect>();
        AddControlRect(WorkflowSelectorHost, rects);
        AddControlRect(CanvasToolbarActions, rects);
        AddControlRect(MiniMapHost, rects);
        AddControlRect(CanvasStatusHost, rects);
        AddControlRect(LibraryTogglePill, rects);
        AddControlRect(WorkspaceRightPill, rects);
        AddControlRect(RightPanelHost, rects);
        return rects;
    }

    private void AddControlRect(Control? control, ICollection<CanvasViewportRect> target)
    {
        if (CanvasOverlay is null
            || control is null
            || !control.IsVisible
            || control.Bounds.Width <= 0
            || control.Bounds.Height <= 0
            || control.TranslatePoint(new Point(0, 0), CanvasOverlay) is not Point origin)
        {
            return;
        }

        target.Add(new CanvasViewportRect(
            origin.X,
            origin.Y,
            control.Bounds.Width,
            control.Bounds.Height));
    }

    private CanvasViewportRect SafeFitViewport()
    {
        if (CanvasOverlay is null)
        {
            return new CanvasViewportRect(0, 0, 1, 1);
        }

        const double inset = 12;
        var left = inset;
        var top = inset;
        var right = Math.Max(left + 1, CanvasOverlay.Bounds.Width - inset);
        var bottom = Math.Max(top + 1, CanvasOverlay.Bounds.Height - inset);
        foreach (var control in new Control?[] { WorkflowSelectorHost, CanvasToolbarActions })
        {
            var rects = new List<CanvasViewportRect>();
            AddControlRect(control, rects);
            if (rects.Count > 0)
            {
                top = Math.Max(top, rects[0].Bottom + inset);
            }
        }

        var miniMapRects = new List<CanvasViewportRect>();
        AddControlRect(MiniMapHost, miniMapRects);
        if (miniMapRects.Count > 0)
        {
            right = Math.Min(right, miniMapRects[0].X - inset);
            bottom = Math.Min(bottom, miniMapRects[0].Y - inset);
        }

        if (DataContext is WorkspacePageViewModel { UseOverlayRightPanel: true, IsRightPanelOpen: true })
        {
            var rightPanelRects = new List<CanvasViewportRect>();
            AddControlRect(RightPanelHost, rightPanelRects);
            if (rightPanelRects.Count > 0)
            {
                right = Math.Min(right, rightPanelRects[0].X - inset);
            }
        }

        if (right - left < 160 || bottom - top < 120)
        {
            return new CanvasViewportRect(
                inset,
                inset,
                Math.Max(1, CanvasOverlay.Bounds.Width - (inset * 2)),
                Math.Max(1, CanvasOverlay.Bounds.Height - (inset * 2)));
        }

        return new CanvasViewportRect(left, top, right - left, bottom - top);
    }

    private void SyncEdgePositions()
    {
        if (DataContext is not WorkspacePageViewModel viewModel)
        {
            return;
        }

        var nodes = viewModel.Nodes.ToDictionary(node => node.Id, node => node);
        foreach (var edge in viewModel.Edges)
        {
            if (nodes.TryGetValue(edge.Source, out var source)
                && nodes.TryGetValue(edge.Target, out var target))
            {
                edge.UpdateEdgePath(source.X, source.Y, target.X, target.Y);
            }
        }
        ScheduleEdgeLabelLayout();
    }

    private void LayoutEdgeLabels()
    {
        if (DataContext is not WorkspacePageViewModel viewModel
            || EdgesItemsControl is null)
        {
            return;
        }

        // DataTemplate 完成测量后读取真实胶囊尺寸；不可见/尚未生成时才使用文本回退尺寸。
        EdgesItemsControl.UpdateLayout();
        var measuredSizes = EdgesItemsControl
            .GetVisualDescendants()
            .OfType<Control>()
            .Where(control => string.Equals(control.Name, "EdgeLabelPlacementHost", StringComparison.Ordinal)
                              && control.DataContext is WorkflowEdgeViewModel)
            .GroupBy(control => ((WorkflowEdgeViewModel)control.DataContext!).Id, StringComparer.Ordinal)
            .ToDictionary(
                group => group.Key,
                group =>
                {
                    var control = group.First();
                    return new Size(
                        Math.Max(control.Bounds.Width, control.DesiredSize.Width),
                        Math.Max(control.Bounds.Height, control.DesiredSize.Height));
                },
                StringComparer.Ordinal);
        var requests = viewModel.Edges
            .Where(edge => edge.HasMidpointLabel)
            .Select(edge =>
            {
                var fallback = CanvasEdgeLabelLayoutHelpers.FallbackSize(edge.MidpointLabel);
                var width = measuredSizes.TryGetValue(edge.Id, out var measured)
                            && measured.Width > 0
                    ? measured.Width
                    : fallback.Width;
                var height = measuredSizes.TryGetValue(edge.Id, out measured)
                             && measured.Height > 0
                    ? measured.Height
                    : fallback.Height;
                return new CanvasEdgeLabelRequest(
                    edge.Id,
                    edge.LabelAnchorX,
                    edge.LabelAnchorY,
                    edge.LabelTangentX,
                    edge.LabelTangentY,
                    width,
                    height,
                    edge.IsSelected);
            })
            .ToArray();
        if (requests.Length == 0)
        {
            return;
        }

        var nodeBounds = viewModel.Nodes
            .Select(node => new CanvasViewportRect(
                node.X,
                node.Y,
                NodePortSpec.NodeWidth,
                node.CanvasHeight))
            .ToArray();
        var edgesById = viewModel.Edges.ToDictionary(edge => edge.Id, StringComparer.Ordinal);
        foreach (var placement in CanvasEdgeLabelLayoutHelpers.PlaceLabels(requests, nodeBounds))
        {
            if (edgesById.TryGetValue(placement.Id, out var edge))
            {
                edge.SetLabelLayout(placement.X, placement.Y, placement.IsVisible);
            }
        }
    }

    private void SyncMiniMapPositions()
    {
        if (MiniMapItemsControl is null || DataContext is not WorkspacePageViewModel viewModel)
        {
            return;
        }

        // 与主画布一致：对 item 容器设 Canvas 附加属性（DataTemplate 根上的 Canvas.Left 不生效）
        _miniMapTransform = ComputeMiniMapTransform(viewModel);
        EnsureMiniMapItemsControlSize();
        MiniMapItemsControl.UpdateLayout();
        var missing = 0;
        foreach (var node in viewModel.Nodes)
        {
            if (FindNodeContainer(node, MiniMapItemsControl, _miniMapContainersById) is { } container)
            {
                var (miniX, miniY) = MiniMapMarkerPosition(node);
                Canvas.SetLeft(container, miniX);
                Canvas.SetTop(container, miniY);
                // 容器默认可能铺满，压成点状
                container.Width = 10;
                container.Height = 6;
                container.IsVisible = true;
            }
            else
            {
                missing++;
            }
        }

        // 兜底：遍历视觉树
        SyncMiniMapContainerPositions(MiniMapItemsControl);
        SyncMiniMapViewportFrame();

        // 容器尚未生成时再补一帧
        if (missing > 0 && viewModel.Nodes.Count > 0)
        {
            Dispatcher.UIThread.Post(() =>
            {
                if (MiniMapItemsControl is null || DataContext is not WorkspacePageViewModel vm)
                {
                    return;
                }

                MiniMapItemsControl.UpdateLayout();
                foreach (var node in vm.Nodes)
                {
                    if (FindNodeContainer(node, MiniMapItemsControl, _miniMapContainersById) is { } container)
                    {
                        var (miniX, miniY) = MiniMapMarkerPosition(node);
                        Canvas.SetLeft(container, miniX);
                        Canvas.SetTop(container, miniY);
                        container.Width = 10;
                        container.Height = 6;
                    }
                }

                SyncMiniMapViewportFrame();
            }, DispatcherPriority.Loaded);
        }
    }

    private static Control? FindNodeContainer(
        WorkflowNodeViewModel node,
        ItemsControl? itemsControl,
        Dictionary<string, Control> containersById)
    {
        if (containersById.TryGetValue(node.Id, out var cached)
            && ReferenceEquals(cached.DataContext, node))
        {
            return cached;
        }

        containersById.Remove(node.Id);
        if (itemsControl?.ContainerFromItem(node) is not Control container)
        {
            return null;
        }

        containersById[node.Id] = container;
        return container;
    }

    private void EnsureMiniMapItemsControlSize()
    {
        if (MiniMapItemsControl is null)
        {
            return;
        }

        // ItemsControl 嵌在 Canvas 上时 DesiredSize 常为 0，必须显式尺寸才能画子项
        var w = NodePortSpec.MiniMapContentWidth;
        var h = NodePortSpec.MiniMapContentHeight;
        if (Math.Abs(MiniMapItemsControl.Width - w) > 0.5)
        {
            MiniMapItemsControl.Width = w;
        }

        if (Math.Abs(MiniMapItemsControl.Height - h) > 0.5)
        {
            MiniMapItemsControl.Height = h;
        }

        if (MiniMapCanvas is not null)
        {
            if (Math.Abs(MiniMapCanvas.Width - w) > 0.5)
            {
                MiniMapCanvas.Width = w;
            }

            if (Math.Abs(MiniMapCanvas.Height - h) > 0.5)
            {
                MiniMapCanvas.Height = h;
            }
        }
    }

    private void SyncMiniMapViewportFrame()
    {
        if (MiniMapViewportFrame is null || CanvasOverlay is null || NodesItemsControl is null)
        {
            return;
        }

        var zoom = CurrentCanvasZoom();
        var offset = CurrentCanvasOffset();
        // screen = logical * zoom + offset → logical visible origin
        var logicalLeft = -offset.X / zoom;
        var logicalTop = -offset.Y / zoom;
        var logicalW = CanvasOverlay.Bounds.Width / zoom;
        var logicalH = CanvasOverlay.Bounds.Height / zoom;
        var (mx, my, mw, mh) = _miniMapTransform.ViewportFrame(
            logicalLeft, logicalTop, logicalW, logicalH);
        Canvas.SetLeft(MiniMapViewportFrame, mx);
        Canvas.SetTop(MiniMapViewportFrame, my);
        MiniMapViewportFrame.Width = mw;
        MiniMapViewportFrame.Height = mh;
    }

    private static void SyncNodeContainerPositions(Control control)
    {
        if (control.DataContext is WorkflowNodeViewModel node)
        {
            Canvas.SetLeft(control, node.X);
            Canvas.SetTop(control, node.Y);
        }

        foreach (var child in control.GetVisualChildren().OfType<Control>())
        {
            SyncNodeContainerPositions(child);
        }
    }

    private void SyncMiniMapContainerPositions(Control control)
    {
        if (control.DataContext is WorkflowNodeViewModel node)
        {
            var (miniX, miniY) = MiniMapMarkerPosition(node);
            Canvas.SetLeft(control, miniX);
            Canvas.SetTop(control, miniY);
        }

        foreach (var child in control.GetVisualChildren().OfType<Control>())
        {
            SyncMiniMapContainerPositions(child);
        }
    }

    private static CanvasMiniMapTransform ComputeMiniMapTransform(WorkspacePageViewModel viewModel)
    {
        if (viewModel.Nodes.Count == 0)
        {
            return CanvasMiniMapHelpers.ComputeTransform(0, 0, 1400, 840);
        }

        return CanvasMiniMapHelpers.ComputeTransform(
            viewModel.Nodes.Min(node => node.X),
            viewModel.Nodes.Min(node => node.Y),
            viewModel.Nodes.Max(node => node.X + NodePortSpec.NodeWidth),
            viewModel.Nodes.Max(node => node.Y + node.CanvasHeight));
    }

    private (double X, double Y) MiniMapMarkerPosition(WorkflowNodeViewModel node) =>
        _miniMapTransform.NodeMarkerPosition(
            node.X,
            node.Y,
            NodePortSpec.NodeWidth,
            node.CanvasHeight);

    private Point ToLogicalCanvasPoint(Point canvasPosition)
    {
        if (DataContext is not WorkspacePageViewModel viewModel)
        {
            return canvasPosition;
        }

        var (x, y) = viewModel.CanvasViewport.ToLogical(canvasPosition.X, canvasPosition.Y);
        return new Point(x, y);
    }

    private double CurrentCanvasZoom()
    {
        return DataContext is WorkspacePageViewModel viewModel
            ? Math.Max(0.1, viewModel.CanvasZoom)
            : 1.0;
    }

    private Point CurrentCanvasOffset()
    {
        return DataContext is WorkspacePageViewModel viewModel
            ? new Point(viewModel.CanvasViewport.OffsetX, viewModel.CanvasViewport.OffsetY)
            : default;
    }

    private static TranslateTransform EnsureTranslateTransform(Control control)
    {
        if (control.RenderTransform is TranslateTransform translate)
        {
            return translate;
        }
        if (control.RenderTransform is TransformGroup group)
        {
            var existing = group.Children.OfType<TranslateTransform>().FirstOrDefault();
            if (existing is not null)
            {
                return existing;
            }
            var added = new TranslateTransform();
            group.Children.Add(added);
            return added;
        }
        var replacement = new TransformGroup();
        replacement.Children.Add(new TranslateTransform());
        control.RenderTransform = replacement;
        return replacement.Children.OfType<TranslateTransform>().First();
    }

    private static double Clamp(double v, double lo, double hi) =>
        v < lo ? lo : v > hi ? hi : v;
}
