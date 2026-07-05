using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;

namespace Ariadne.Desktop.Views;

public partial class GitPageView : UserControl
{
    // 拖拽阈值：移动超过此像素才算拖，否则视为点击
    private const double DragThreshold = 4.0;

    private bool _pilDragging;
    private bool _pilMoved;          // 是否超过阈值（拖 vs 点）
    private double _pilDragStartY;
    private double _pilDragOriginTop;
    private double _togglePillTop = -1;
    private bool _layoutInitialized;

    public GitPageView()
    {
        InitializeComponent();
        LayoutUpdated += OnFirstLayout;
    }

    public void OnCommitPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (sender is Control { DataContext: ViewModels.GitHistoryItemViewModel item })
        {
            item.SelectCommand.Execute(null);
        }
    }

    private void OnFirstLayout(object? sender, EventArgs e)
    {
        if (_layoutInitialized || GitTogglePill is null || GitCanvas is null)
        {
            return;
        }
        _layoutInitialized = true;
        if (_togglePillTop < 0)
        {
            _togglePillTop = (GitCanvas.Bounds.Height - GitTogglePill.Height) / 2;
            if (_togglePillTop < 0)
            {
                _togglePillTop = 120;
            }
        }
        Canvas.SetTop(GitTogglePill, _togglePillTop);
    }

    // ===================== 右侧 Pill 点击 & 拖拽 =====================

    /// PointerPressed：捕获指针，记录起始位置，重置拖拽标志。
    public void OnPillPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            return;
        }
        _pilDragging = true;
        _pilMoved = false;
        _pilDragStartY = e.GetPosition(this).Y;
        _pilDragOriginTop = _togglePillTop < 0 ? 120 : _togglePillTop;
        e.Pointer.Capture((IInputElement?)sender);
        e.Handled = true;
    }

    /// PointerMoved：超过阈值才更新位置，防止微小抖动被误判为拖拽。
    public void OnPillPointerMoved(object? sender, PointerEventArgs e)
    {
        if (!_pilDragging || GitTogglePill is null || GitCanvas is null)
        {
            return;
        }
        var dy = e.GetPosition(this).Y - _pilDragStartY;
        if (!_pilMoved && Math.Abs(dy) < DragThreshold)
        {
            return;
        }
        _pilMoved = true;
        var newTop = _pilDragOriginTop + dy;
        var maxTop = GitCanvas.Bounds.Height - GitTogglePill.Height;
        _togglePillTop = Clamp(newTop, 0, Math.Max(0, maxTop));
        Canvas.SetTop(GitTogglePill, _togglePillTop);
    }

    /// PointerReleased：未移动超阈值 = 点击，切换右栏；移动超阈值 = 拖拽，只释放。
    public void OnPillPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (!_pilMoved && DataContext is ViewModels.GitPageViewModel vm)
        {
            vm.IsRightPanelOpen = !vm.IsRightPanelOpen;
        }
        _pilDragging = false;
        _pilMoved = false;
        e.Pointer.Capture(null);
    }

    private static double Clamp(double v, double lo, double hi) =>
        v < lo ? lo : v > hi ? hi : v;
}
