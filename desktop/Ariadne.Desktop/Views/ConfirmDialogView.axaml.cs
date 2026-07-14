using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Media;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop.ViewModels;
// GetVisualRoot lives on VisualTreeExtensions

namespace Ariadne.Desktop.Views;

public partial class ConfirmDialogView : UserControl
{
    public ConfirmDialogView()
    {
        InitializeComponent();
        DataContextChanged += OnDataContextChanged;
        AttachedToVisualTree += OnAttachedToVisualTree;
    }

    private void OnDataContextChanged(object? sender, EventArgs e)
    {
        ApplySeverityVisuals();
            }

    private void OnAttachedToVisualTree(object? sender, VisualTreeAttachmentEventArgs e)
    {
        ApplySeverityVisuals();
                ApplyHostSizeConstraints();
        Dispatcher.UIThread.Post(FocusPrimaryTarget, DispatcherPriority.Loaded);
    }

    private void ApplySeverityVisuals()
    {
        if (DataContext is not ConfirmDialogViewModel vm || SeverityIcon is null)
        {
            return;
        }

        try
        {
            SeverityIcon.Data = Geometry.Parse(vm.IconData);
        }
        catch
        {
            // 保留 XAML 默认 path
        }

        if (this.TryFindResource(vm.IconBrushKey, out var brush) && brush is IBrush b)
        {
            SeverityIcon.Stroke = b;
            // 填充仅用于实心感较弱的问号/感叹号外框，保持线稿风格
            SeverityIcon.Fill = Brushes.Transparent;
        }
    }


    private void ApplyHostSizeConstraints()
    {
        // U68: dialog max size from host client area (not fixed 520 on a 480-tall window).
        var host = TopLevel.GetTopLevel(this) as Window;
        if (host is null)
        {
            return;
        }
        var availH = Math.Max(240, host.Bounds.Height - 48);
        var availW = Math.Max(280, host.Bounds.Width - 48);
        if (DialogChrome is not null)
        {
            DialogChrome.MaxHeight = availH;
            DialogChrome.MaxWidth = Math.Min(560, availW);
        }
        if (BodyScroll is not null)
        {
            BodyScroll.MaxHeight = Math.Max(120, availH - 160);
        }
    }

    private void FocusPrimaryTarget()
    {
        if (DataContext is not ConfirmDialogViewModel vm)
        {
            return;
        }

        if (vm.HasInput && InputBox is not null)
        {
            InputBox.Focus();
            InputBox.SelectAll();
            return;
        }

        // 无输入：优先聚焦取消（危险）或主按钮
        var targetIndex = vm.Severity == DialogSeverity.Danger
            ? vm.CancelResultIndex
            : vm.ConfirmResultIndex;
        FocusButtonByResult(targetIndex);
    }

    private void FocusButtonByResult(int resultIndex)
    {
        if (resultIndex < 0)
        {
            return;
        }

        foreach (var btn in this.GetVisualDescendants().OfType<Button>())
        {
            if (btn.DataContext is DialogButton db && db.ResultIndex == resultIndex)
            {
                btn.Focus();
                return;
            }
        }
    }

    protected override void OnKeyDown(KeyEventArgs e)
    {
        if (DataContext is ConfirmDialogViewModel vm && e.Key == Key.Enter && vm.AllowEnterConfirm)
        {
            // 输入框内 Enter 也提交
            vm.RequestConfirm();
            e.Handled = true;
            return;
        }

        base.OnKeyDown(e);
    }
}
