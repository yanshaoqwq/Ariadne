using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class MainWindow : Window
{
    private bool _closeConfirmed;

    public MainWindow()
    {
        InitializeComponent();
    }

    private void OnTitleBarPointerPressed(object? sender, PointerPressedEventArgs e)
    {
        if (e.GetCurrentPoint(this).Properties.IsLeftButtonPressed)
        {
            BeginMoveDrag(e);
        }
    }

    private void OnMinimizeClicked(object? sender, RoutedEventArgs e)
    {
        WindowState = WindowState.Minimized;
    }

    private void OnMaximizeClicked(object? sender, RoutedEventArgs e)
    {
        WindowState = WindowState == WindowState.Maximized ? WindowState.Normal : WindowState.Maximized;
    }

    private void OnCloseClicked(object? sender, RoutedEventArgs e)
    {
        _ = CloseWithUnsavedCheckAsync();
    }

    private async Task CloseWithUnsavedCheckAsync()
    {
        if (DataContext is MainWindowViewModel { CurrentPage: IUnsavedChangesGuard guard }
            && !await guard.ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
        {
            return;
        }
        _closeConfirmed = true;
        Close();
    }

    protected override void OnClosing(WindowClosingEventArgs e)
    {
        if (!_closeConfirmed && DataContext is MainWindowViewModel { CurrentPage: IUnsavedChangesGuard { HasUnsavedChanges: true } })
        {
            e.Cancel = true;
            _ = CloseWithUnsavedCheckAsync();
            return;
        }

        base.OnClosing(e);
    }

    // 点击遮罩空白处=取消；点在弹窗卡片上（e.Source 非遮罩本身）则忽略。
    private void OnDialogScrimPressed(object? sender, PointerPressedEventArgs e)
    {
        if (ReferenceEquals(e.Source, sender))
        {
            DialogService.Current.RequestCancelActive();
        }
    }

    // Esc 关闭当前弹窗（走取消语义）。
    protected override void OnKeyDown(KeyEventArgs e)
    {
        if (e.Key == Key.Escape && DialogService.Current.IsOpen)
        {
            DialogService.Current.RequestCancelActive();
            e.Handled = true;
            return;
        }

        base.OnKeyDown(e);
    }
}
