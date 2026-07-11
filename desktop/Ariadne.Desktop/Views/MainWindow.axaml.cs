using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Threading;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class MainWindow : Window
{
    private bool _closeConfirmed;

    public MainWindow()
    {
        InitializeComponent();
        Opened += (_, _) =>
        {
            RefreshWindowIcon();
            // 首次打开也写出当前主题色到系统图标目录
            AppIconDesktopSync.QueueSync();
        };
        AppIconPainter.IconColorsChanged += OnIconColorsChanged;
        Closed += (_, _) => AppIconPainter.IconColorsChanged -= OnIconColorsChanged;
    }

    private void OnIconColorsChanged()
    {
        // 个性化 / 主题切换后，系统窗口图标随 Accent 重绘
        Dispatcher.UIThread.Post(RefreshWindowIcon, DispatcherPriority.Background);
    }

    private void RefreshWindowIcon()
    {
        try
        {
            Icon = AppIconPainter.CreateWindowIcon(256);
        }
        catch
        {
            // 回退到打包静态 ico（构建时默认青绿）
        }
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
