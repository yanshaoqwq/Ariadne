using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media;
using Avalonia.VisualTree;
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

    private bool _layoutInitialized;
    private WorkspacePageViewModel? _attachedViewModel;

    public WorkspacePageView()
    {
        InitializeComponent();
        DataContextChanged += (_, _) => AttachViewActions();
        LayoutUpdated += OnFirstLayout;
        AttachViewActions();
    }

    private void OnFirstLayout(object? sender, EventArgs e)
    {
        if (_layoutInitialized || LibraryTogglePill is null || WorkspaceGrid is null)
        {
            return;
        }
        _layoutInitialized = true;
        PositionBottomPill();
        PositionRightPill();
        SyncNodeContainerPositions();
    }

    private void AttachViewActions()
    {
        if (_attachedViewModel is not null && !ReferenceEquals(_attachedViewModel, DataContext))
        {
            _attachedViewModel.RequestFitView = null;
            _attachedViewModel = null;
        }

        if (DataContext is WorkspacePageViewModel viewModel)
        {
            viewModel.RequestFitView = FitViewToNodes;
            _attachedViewModel = viewModel;
        }
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        if (_attachedViewModel is not null)
        {
            _attachedViewModel.RequestFitView = null;
            _attachedViewModel = null;
        }

        base.OnDetachedFromVisualTree(e);
    }

    // ===================== 收起/展开下栏（库底部 Pill 点击） =====================

    private void ToggleLibrary()
    {
        if (WorkspaceGrid is null || LibrarySplitter is null || LibraryContent is null || LibraryTogglePill is null)
        {
            return;
        }

        var row = WorkspaceGrid.RowDefinitions[2];
        var opening = !LibraryContent.IsVisible;

        if (opening)
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
        if (WorkspaceRightPill is null || CanvasOverlay is null)
        {
            return;
        }

        if (_rightPillTop < 0)
        {
            var h = CanvasOverlay.Bounds.Height;
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
        if (!_rightPilDragging || WorkspaceRightPill is null || CanvasOverlay is null)
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
        var maxTop = CanvasOverlay.Bounds.Height - WorkspaceRightPill.Height;
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

    // ===================== 节点拖动 =====================

    public void OnNodePointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (FindNodeDataContext(sender as Control) is not { } node
            || !e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            return;
        }

        node.SelectCommand.Execute(null);
        _draggedNode = node;
        _nodeDragging = true;
        _nodeDragStart = e.GetPosition(CanvasOverlay);
        _nodeDragOriginX = node.X;
        _nodeDragOriginY = node.Y;
        e.Pointer.Capture((IInputElement?)sender);
        e.Handled = true;
    }

    public void OnNodePointerMoved(object? sender, PointerEventArgs e)
    {
        if (!_nodeDragging || _draggedNode is not { } node || CanvasOverlay is null)
        {
            return;
        }

        var position = e.GetPosition(CanvasOverlay);
        var newX = _nodeDragOriginX + position.X - _nodeDragStart.X;
        var newY = _nodeDragOriginY + position.Y - _nodeDragStart.Y;
        node.X = Clamp(newX, 0, Math.Max(0, CanvasOverlay.Bounds.Width - 202));
        node.Y = Clamp(newY, 0, Math.Max(0, CanvasOverlay.Bounds.Height - 150));
        SyncNodeContainerPositions();
        e.Handled = true;
    }

    public void OnNodePointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (!_nodeDragging)
        {
            return;
        }

        _nodeDragging = false;
        _draggedNode = null;
        e.Pointer.Capture(null);
        e.Handled = true;
    }

    public void OnNodeSelectPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (FindNodeDataContext(sender as Control) is { } node)
        {
            node.SelectCommand.Execute(null);
        }
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

    private void SyncNodeContainerPositions()
    {
        if (NodesItemsControl is null)
        {
            return;
        }

        SyncNodeContainerPositions(NodesItemsControl);
    }

    private void FitViewToNodes()
    {
        if (DataContext is not WorkspacePageViewModel { Nodes.Count: > 0 } viewModel
            || NodesItemsControl is null)
        {
            return;
        }

        var minX = viewModel.Nodes.Min(node => node.X);
        var minY = viewModel.Nodes.Min(node => node.Y);
        if (NodesItemsControl.RenderTransform is not TranslateTransform transform)
        {
            transform = new TranslateTransform();
            NodesItemsControl.RenderTransform = transform;
        }

        transform.X = Math.Max(0, 48 - minX);
        transform.Y = Math.Max(0, 48 - minY);
        SyncNodeContainerPositions();
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

    private static double Clamp(double v, double lo, double hi) =>
        v < lo ? lo : v > hi ? hi : v;
}
