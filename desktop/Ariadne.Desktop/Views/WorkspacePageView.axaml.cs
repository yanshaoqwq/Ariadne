using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;

namespace Ariadne.Desktop.Views;

public partial class WorkspacePageView : UserControl
{
    private GridLength _savedLibraryHeight = new(220);

    // 浮动收起/展开小块（类似右侧栏 panel-float）的水平位置
    private double _togglePillLeft = -1;    // -1 表示尚未初始化，首次用居中值
    private bool _pilDragging;
    private double _pilDragStartX;
    private double _pilDragOriginLeft;

    public WorkspacePageView()
    {
        InitializeComponent();
        // 布局完成后初始化小块位置居中
        LayoutUpdated += OnFirstLayout;
    }

    private bool _layoutInitialized;
    private void OnFirstLayout(object? sender, EventArgs e)
    {
        if (_layoutInitialized || LibraryTogglePill is null || WorkspaceGrid is null)
        {
            return;
        }
        _layoutInitialized = true;
        PositionTogglePill();
    }

    // ===================== 收起/展开下栏 =====================

    /// 收起/展开下栏节点库（从浮动小块或栏内 chevron 触发）。
    public void OnToggleLibrary(object? sender, Avalonia.Interactivity.RoutedEventArgs e)
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

        // 更新小块的 chevron 方向（通过重新定位也触发 UI 刷新）
        PositionTogglePill();
    }

    // ===================== 浮动小块位置初始化 & 同步 =====================

    private void PositionTogglePill()
    {
        if (LibraryTogglePill is null || WorkspaceGrid is null)
        {
            return;
        }

        // 如果小块位置未初始化，居中放置
        if (_togglePillLeft < 0)
        {
            var canvasWidth = WorkspaceGrid.Bounds.Width;
            _togglePillLeft = canvasWidth > 0
                ? (canvasWidth - LibraryTogglePill.Width) / 2
                : 200;
        }

        Canvas.SetLeft(LibraryTogglePill, _togglePillLeft);
    }

    // ===================== 拖拽浮动小块（左右） =====================

    public void OnPillPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (!e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            return;
        }
        _pilDragging = true;
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
        var newLeft = _pilDragOriginLeft + dx;
        var maxLeft = WorkspaceGrid.Bounds.Width - LibraryTogglePill.Width;
        _togglePillLeft = Clamp(newLeft, 0, Math.Max(0, maxLeft));
        Canvas.SetLeft(LibraryTogglePill, _togglePillLeft);
    }

    public void OnPillPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        _pilDragging = false;
        e.Pointer.Capture(null);
    }

    private static double Clamp(double v, double lo, double hi) =>
        v < lo ? lo : v > hi ? hi : v;
}
