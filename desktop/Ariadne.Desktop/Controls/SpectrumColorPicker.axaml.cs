using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Media;
using Avalonia.Controls.Primitives;
using Avalonia.Threading;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// 折叠色块 + Flyout 调色板（SV 色图 / 色相条 / RGB / Hex）。
/// 默认只显示色块；点击在色块左下角打开完整色板，无页面变黑遮罩。
/// </summary>
public partial class SpectrumColorPicker : UserControl
{
    public static readonly StyledProperty<string> SelectedHexProperty =
        AvaloniaProperty.Register<SpectrumColorPicker, string>(
            nameof(SelectedHex),
            defaultValue: "#2E726B",
            defaultBindingMode: Avalonia.Data.BindingMode.TwoWay);

    private bool _suppress;
    private bool _svDragging;
    private bool _hueDragging;
    private double _hue = 168; // degrees
    private double _sat = 0.6;
    private double _val = 0.45;

    public SpectrumColorPicker()
    {
        InitializeComponent();
        Loaded += (_, _) =>
        {
            ApplyFromHex(SelectedHex, pushProperty: false);
            UpdateVisuals();
        };
        SizeChanged += (_, _) => UpdateCursors();
    }

    private void OnSwatchClick(object? sender, RoutedEventArgs e)
    {
        if (SpectrumPopup is null || CollapsedSwatch is null)
        {
            return;
        }

        // 无全页变黑；锚点：色块左下 ↔ 调色板左下；不够空间则右下 ↔ 右下。
        SpectrumPopup.IsOpen = !SpectrumPopup.IsOpen;
        if (SpectrumPopup.IsOpen)
        {
            Dispatcher.UIThread.Post(() =>
            {
                PositionSpectrumPopup();
                UpdateCursors();
            }, DispatcherPriority.Loaded);
        }
    }

    /// <summary>
    /// 色块左下角对齐调色板左下角（弹窗向上展开）；若横向/纵向放不下则改右下角对齐。
    /// </summary>
    private void PositionSpectrumPopup()
    {
        if (SpectrumPopup is null || CollapsedSwatch is null || SpectrumPopup.Child is not Control child)
        {
            return;
        }

        SpectrumPopup.PlacementTarget = CollapsedSwatch;
        SpectrumPopup.Placement = PlacementMode.BottomEdgeAlignedLeft;

        child.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
        var popupW = child.DesiredSize.Width;
        var popupH = child.DesiredSize.Height;
        if (popupW <= 1 || popupH <= 1)
        {
            popupW = Math.Max(popupW, child.Bounds.Width);
            popupH = Math.Max(popupH, child.Bounds.Height);
        }

        if (popupW <= 1)
        {
            popupW = 280;
        }

        if (popupH <= 1)
        {
            popupH = 260;
        }

        var topLevel = TopLevel.GetTopLevel(this);
        var swatch = CollapsedSwatch;
        var swatchH = Math.Max(1, swatch.Bounds.Height);
        var swatchW = Math.Max(1, swatch.Bounds.Width);

        // BottomEdge 把 popup 顶边贴色块底边；VerticalOffset = -popupH → 底边重合（左下/右下角对齐）
        var preferLeft = true;
        if (topLevel is not null)
        {
            var bl = swatch.TranslatePoint(new Point(0, swatchH), topLevel);
            var br = swatch.TranslatePoint(new Point(swatchW, swatchH), topLevel);
            if (bl is Point blp && br is Point brp)
            {
                var corner = SpectrumPopupAnchor.ChooseCorner(
                    swatchLeft: blp.X,
                    swatchRight: brp.X,
                    swatchBottom: blp.Y,
                    popupW: popupW,
                    popupH: popupH,
                    viewW: topLevel.ClientSize.Width,
                    viewH: topLevel.ClientSize.Height);
                preferLeft = corner == "bl";
            }
        }

        SpectrumPopup.Placement = preferLeft
            ? PlacementMode.BottomEdgeAlignedLeft
            : PlacementMode.BottomEdgeAlignedRight;
        SpectrumPopup.HorizontalOffset = 0;
        SpectrumPopup.VerticalOffset = -popupH;
    }

    public string SelectedHex
    {
        get => GetValue(SelectedHexProperty);
        set => SetValue(SelectedHexProperty, value);
    }

    protected override void OnPropertyChanged(AvaloniaPropertyChangedEventArgs change)
    {
        base.OnPropertyChanged(change);
        if (change.Property == SelectedHexProperty && !_suppress)
        {
            ApplyFromHex(change.GetNewValue<string>(), pushProperty: false);
            UpdateVisuals();
        }
    }

    private void OnSvPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (!e.GetCurrentPoint(SvField).Properties.IsLeftButtonPressed || SvField is null)
        {
            return;
        }

        _svDragging = true;
        e.Pointer.Capture(SvField);
        PickSv(e.GetPosition(SvField));
        e.Handled = true;
    }

    private void OnSvPointerMoved(object? sender, PointerEventArgs e)
    {
        if (!_svDragging || SvField is null)
        {
            return;
        }

        PickSv(e.GetPosition(SvField));
        e.Handled = true;
    }

    private void OnSvPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (!_svDragging)
        {
            return;
        }

        _svDragging = false;
        e.Pointer.Capture(null);
        e.Handled = true;
    }

    private void OnSvCaptureLost(object? sender, PointerCaptureLostEventArgs e) => _svDragging = false;

    private void OnHuePointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (!e.GetCurrentPoint(HueBar).Properties.IsLeftButtonPressed || HueBar is null)
        {
            return;
        }

        _hueDragging = true;
        e.Pointer.Capture(HueBar);
        PickHue(e.GetPosition(HueBar));
        e.Handled = true;
    }

    private void OnHuePointerMoved(object? sender, PointerEventArgs e)
    {
        if (!_hueDragging || HueBar is null)
        {
            return;
        }

        PickHue(e.GetPosition(HueBar));
        e.Handled = true;
    }

    private void OnHuePointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        if (!_hueDragging)
        {
            return;
        }

        _hueDragging = false;
        e.Pointer.Capture(null);
        e.Handled = true;
    }

    private void OnHueCaptureLost(object? sender, PointerCaptureLostEventArgs e) => _hueDragging = false;

    private void OnRgbSliderChanged(object? sender, RangeBaseValueChangedEventArgs e)
    {
        if (_suppress)
        {
            return;
        }

        var r = (byte)Math.Clamp((int)Math.Round(RSlider?.Value ?? 0), 0, 255);
        var g = (byte)Math.Clamp((int)Math.Round(GSlider?.Value ?? 0), 0, 255);
        var b = (byte)Math.Clamp((int)Math.Round(BSlider?.Value ?? 0), 0, 255);
        RgbToHsv(r, g, b, out _hue, out _sat, out _val);
        PushHex(ColorChannelEditor.ToHex(r, g, b));
        UpdateVisuals(skipRgbSliders: true);
    }

    private void OnHexLostFocus(object? sender, RoutedEventArgs e) => CommitHexBox();

    private void OnHexKeyDown(object? sender, KeyEventArgs e)
    {
        if (e.Key == Key.Enter)
        {
            CommitHexBox();
            e.Handled = true;
        }
    }

    private void CommitHexBox()
    {
        if (HexBox is null)
        {
            return;
        }

        if (ColorChannelEditor.TryParseHex(HexBox.Text, out var r, out var g, out var b))
        {
            RgbToHsv(r, g, b, out _hue, out _sat, out _val);
            PushHex(ColorChannelEditor.ToHex(r, g, b));
            UpdateVisuals();
        }
        else
        {
            // 恢复为当前有效值
            if (HexBox is not null)
            {
                HexBox.Text = SelectedHex;
            }
        }
    }

    private void PickSv(Point p)
    {
        if (SvField is null)
        {
            return;
        }

        var w = Math.Max(1, SvField.Bounds.Width);
        var h = Math.Max(1, SvField.Bounds.Height);
        _sat = Math.Clamp(p.X / w, 0, 1);
        _val = Math.Clamp(1.0 - (p.Y / h), 0, 1);
        HsvToRgb(_hue, _sat, _val, out var r, out var g, out var b);
        PushHex(ColorChannelEditor.ToHex(r, g, b));
        UpdateVisuals();
    }

    private void PickHue(Point p)
    {
        if (HueBar is null)
        {
            return;
        }

        var h = Math.Max(1, HueBar.Bounds.Height);
        _hue = Math.Clamp(p.Y / h, 0, 1) * 360.0;
        HsvToRgb(_hue, _sat, _val, out var r, out var g, out var b);
        PushHex(ColorChannelEditor.ToHex(r, g, b));
        UpdateVisuals();
    }

    private void PushHex(string hex)
    {
        _suppress = true;
        try
        {
            SelectedHex = hex;
        }
        finally
        {
            _suppress = false;
        }
    }

    private void ApplyFromHex(string? hex, bool pushProperty)
    {
        if (!ColorChannelEditor.TryParseHex(hex, out var r, out var g, out var b))
        {
            return;
        }

        RgbToHsv(r, g, b, out _hue, out _sat, out _val);
        if (pushProperty)
        {
            PushHex(ColorChannelEditor.ToHex(r, g, b));
        }
    }

    private void UpdateVisuals(bool skipRgbSliders = false)
    {
        HsvToRgb(_hue, _sat, _val, out var r, out var g, out var b);
        var hex = ColorChannelEditor.ToHex(r, g, b);

        if (SvHueBase is not null)
        {
            HsvToRgb(_hue, 1, 1, out var hr, out var hg, out var hb);
            SvHueBase.Background = new SolidColorBrush(Color.FromRgb(hr, hg, hb));
        }

        var brush = new SolidColorBrush(Color.FromRgb(r, g, b));
        if (PreviewSwatch is not null)
        {
            PreviewSwatch.Background = brush;
        }

        if (CollapsedSwatch is not null)
        {
            CollapsedSwatch.Background = brush;
        }

        if (HexBox is not null && !HexBox.IsFocused)
        {
            HexBox.Text = hex;
        }

        if (RgbLabel is not null)
        {
            RgbLabel.Text = $"R {r}  G {g}  B {b}";
        }

        if (!skipRgbSliders)
        {
            _suppress = true;
            try
            {
                if (RSlider is not null) RSlider.Value = r;
                if (GSlider is not null) GSlider.Value = g;
                if (BSlider is not null) BSlider.Value = b;
            }
            finally
            {
                _suppress = false;
            }
        }

        if (RValue is not null) RValue.Text = r.ToString();
        if (GValue is not null) GValue.Text = g.ToString();
        if (BValue is not null) BValue.Text = b.ToString();

        UpdateCursors();
    }

    private void UpdateCursors()
    {
        if (SvField is not null && SvCursor is not null)
        {
            var w = Math.Max(1, SvField.Bounds.Width);
            var h = Math.Max(1, SvField.Bounds.Height);
            var cx = _sat * w - 7;
            var cy = (1.0 - _val) * h - 7;
            Canvas.SetLeft(SvCursor, cx);
            Canvas.SetTop(SvCursor, cy);
        }

        if (HueBar is not null && HueCursor is not null)
        {
            var h = Math.Max(1, HueBar.Bounds.Height);
            var y = (_hue / 360.0) * h - 2;
            Canvas.SetLeft(HueCursor, (HueBar.Bounds.Width - 16) / 2);
            Canvas.SetTop(HueCursor, y);
        }
    }

    private static void HsvToRgb(double h, double s, double v, out byte r, out byte g, out byte b)
    {
        h = ((h % 360) + 360) % 360;
        s = Math.Clamp(s, 0, 1);
        v = Math.Clamp(v, 0, 1);
        var c = v * s;
        var x = c * (1 - Math.Abs((h / 60) % 2 - 1));
        var m = v - c;
        double r1, g1, b1;
        if (h < 60) { r1 = c; g1 = x; b1 = 0; }
        else if (h < 120) { r1 = x; g1 = c; b1 = 0; }
        else if (h < 180) { r1 = 0; g1 = c; b1 = x; }
        else if (h < 240) { r1 = 0; g1 = x; b1 = c; }
        else if (h < 300) { r1 = x; g1 = 0; b1 = c; }
        else { r1 = c; g1 = 0; b1 = x; }

        r = (byte)Math.Clamp((int)Math.Round((r1 + m) * 255), 0, 255);
        g = (byte)Math.Clamp((int)Math.Round((g1 + m) * 255), 0, 255);
        b = (byte)Math.Clamp((int)Math.Round((b1 + m) * 255), 0, 255);
    }

    private static void RgbToHsv(byte r, byte g, byte b, out double h, out double s, out double v)
    {
        var rf = r / 255.0;
        var gf = g / 255.0;
        var bf = b / 255.0;
        var max = Math.Max(rf, Math.Max(gf, bf));
        var min = Math.Min(rf, Math.Min(gf, bf));
        var delta = max - min;
        v = max;
        s = max <= 1e-9 ? 0 : delta / max;
        if (delta <= 1e-9)
        {
            h = 0;
            return;
        }

        if (Math.Abs(max - rf) < 1e-9)
        {
            h = 60 * (((gf - bf) / delta) % 6);
        }
        else if (Math.Abs(max - gf) < 1e-9)
        {
            h = 60 * (((bf - rf) / delta) + 2);
        }
        else
        {
            h = 60 * (((rf - gf) / delta) + 4);
        }

        if (h < 0)
        {
            h += 360;
        }
    }
}
