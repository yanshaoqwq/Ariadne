using Avalonia;
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
            ApplyWindowChromeForState();
            AppIconDesktopSync.QueueSync();
        };
        PropertyChanged += OnWindowPropertyChanged;
        LayoutUpdated += (_, _) => ApplyResponsiveBreakpoint();
        AppIconPainter.IconColorsChanged += OnIconColorsChanged;
        Closed += (_, _) => AppIconPainter.IconColorsChanged -= OnIconColorsChanged;
    }

    private void OnWindowPropertyChanged(object? sender, AvaloniaPropertyChangedEventArgs e)
    {
        if (e.Property == WindowStateProperty)
        {
            ApplyWindowChromeForState();
        }
        if (e.Property == BoundsProperty)
        {
            ApplyResponsiveBreakpoint();
        }
    }

    /// <summary>
    /// U60: width breakpoints drive compact chrome / auto-collapse of the global sidebar.
    /// Compact &lt; 900; medium 900–1199; wide ≥ 1200.
    /// </summary>
    private void ApplyResponsiveBreakpoint()
    {
        var width = Bounds.Width;
        if (width <= 0)
        {
            return;
        }

        var compact = width < 900;
        Classes.Set("compact", compact);
        Classes.Set("medium", width is >= 900 and < 1200);
        Classes.Set("wide", width >= 1200);

        if (DataContext is MainWindowViewModel vm)
        {
            // Auto-collapse only when entering compact; user may re-expand via toggle.
            if (compact && vm.SidebarExpanded)
            {
                vm.SidebarExpanded = false;
            }
        }
    }

    /// <summary>最大化时去掉圆角与边框，普通态恢复悬浮圆角窗（U61）。</summary>
    private void ApplyWindowChromeForState()
    {
        if (WindowChrome is null)
        {
            return;
        }

        if (WindowState == WindowState.Maximized)
        {
            WindowChrome.CornerRadius = new CornerRadius(0);
            WindowChrome.BorderThickness = new Thickness(0);
        }
        else
        {
            WindowChrome.CornerRadius = new CornerRadius(10);
            WindowChrome.BorderThickness = new Thickness(1);
        }
    }

    private void OnIconColorsChanged()
    {
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
            // 回退到打包静态 ico
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

    private void OnDialogScrimPressed(object? sender, PointerPressedEventArgs e)
    {
        if (ReferenceEquals(e.Source, sender))
        {
            DialogService.Current.RequestCancelActive();
        }
    }

    // Esc 取消；Enter 确认（危险弹窗由 VM 拒绝）（U64）
    protected override void OnKeyDown(KeyEventArgs e)
    {
        if (DialogService.Current.IsOpen)
        {
            if (e.Key == Key.Escape)
            {
                DialogService.Current.RequestCancelActive();
                e.Handled = true;
                return;
            }

            if (e.Key == Key.Enter)
            {
                DialogService.Current.RequestConfirmActive();
                e.Handled = true;
                return;
            }
        }

        base.OnKeyDown(e);
    }
}
