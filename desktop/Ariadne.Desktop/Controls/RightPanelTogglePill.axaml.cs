using System.Windows.Input;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Threading;
using Avalonia.VisualTree;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// 工作区、作品页和版本页共用的右侧栏边缘控制器。
/// 单击开合，上下拖动只调整控制器位置，不改变面板状态。
/// </summary>
public partial class RightPanelTogglePill : UserControl
{
    private const double DragThreshold = 4;
    private Panel? _host;
    private bool _pointerDown;
    private bool _moved;
    private double _pressY;
    private double _originTop;
    private double _top = -1;

    public static readonly StyledProperty<bool> IsPanelOpenProperty =
        AvaloniaProperty.Register<RightPanelTogglePill, bool>(nameof(IsPanelOpen));

    public static readonly StyledProperty<ICommand?> ToggleCommandProperty =
        AvaloniaProperty.Register<RightPanelTogglePill, ICommand?>(nameof(ToggleCommand));

    public static readonly StyledProperty<string> AccessibleNameProperty =
        AvaloniaProperty.Register<RightPanelTogglePill, string>(nameof(AccessibleName), string.Empty);

    public RightPanelTogglePill()
    {
        InitializeComponent();
        PointerPressed += OnPointerPressed;
        PointerMoved += OnPointerMoved;
        PointerReleased += OnPointerReleased;
        PointerCaptureLost += OnPointerCaptureLost;
        KeyDown += OnKeyDown;
    }

    public bool IsPanelOpen
    {
        get => GetValue(IsPanelOpenProperty);
        set => SetValue(IsPanelOpenProperty, value);
    }

    public ICommand? ToggleCommand
    {
        get => GetValue(ToggleCommandProperty);
        set => SetValue(ToggleCommandProperty, value);
    }

    public string AccessibleName
    {
        get => GetValue(AccessibleNameProperty);
        set => SetValue(AccessibleNameProperty, value);
    }

    protected override void OnAttachedToVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnAttachedToVisualTree(e);
        _host = this.FindAncestorOfType<Panel>();
        if (_host is not null)
        {
            _host.SizeChanged += OnHostSizeChanged;
            Dispatcher.UIThread.Post(PositionWithinHost, DispatcherPriority.Loaded);
        }
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        if (_host is not null)
        {
            _host.SizeChanged -= OnHostSizeChanged;
            _host = null;
        }

        base.OnDetachedFromVisualTree(e);
    }

    private void OnHostSizeChanged(object? sender, SizeChangedEventArgs e) => PositionWithinHost();

    private void PositionWithinHost()
    {
        if (_host is null || _host.Bounds.Height <= 0)
        {
            return;
        }

        if (_top < 0)
        {
            _top = (_host.Bounds.Height - Height) / 2;
        }

        _top = Math.Clamp(_top, 0, Math.Max(0, _host.Bounds.Height - Height));
        ApplyTop();
    }

    private void OnPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (_host is null || !e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            return;
        }

        _pointerDown = true;
        _moved = false;
        _pressY = e.GetPosition(_host).Y;
        _originTop = _top < 0 ? Math.Max(0, (_host.Bounds.Height - Height) / 2) : _top;
        e.Pointer.Capture(this);
        e.Handled = true;
    }

    private void OnPointerMoved(object? sender, PointerEventArgs e)
    {
        if (!_pointerDown || _host is null)
        {
            return;
        }

        var delta = e.GetPosition(_host).Y - _pressY;
        if (!_moved && Math.Abs(delta) < DragThreshold)
        {
            return;
        }

        _moved = true;
        _top = Math.Clamp(_originTop + delta, 0, Math.Max(0, _host.Bounds.Height - Height));
        ApplyTop();
        e.Handled = true;
    }

    private void ApplyTop()
    {
        if (_host is Canvas)
        {
            Canvas.SetTop(this, _top);
            return;
        }

        Margin = new Thickness(Margin.Left, _top, Margin.Right, Margin.Bottom);
    }

    private void OnPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (!_pointerDown)
        {
            return;
        }

        if (!_moved)
        {
            ExecuteToggle();
        }

        ResetPointer(e.Pointer);
        e.Handled = true;
    }

    private void OnPointerCaptureLost(object? sender, PointerCaptureLostEventArgs e)
    {
        _pointerDown = false;
        _moved = false;
    }

    private void OnKeyDown(object? sender, KeyEventArgs e)
    {
        if (!ReferenceEquals(e.Source, this) || e.Key is not (Key.Enter or Key.Space))
        {
            return;
        }

        ExecuteToggle();
        e.Handled = true;
    }

    private void ExecuteToggle()
    {
        if (ToggleCommand?.CanExecute(null) == true)
        {
            ToggleCommand.Execute(null);
        }
    }

    private void ResetPointer(IPointer pointer)
    {
        _pointerDown = false;
        _moved = false;
        pointer.Capture(null);
    }
}
