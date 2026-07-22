using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class MainWindow : Window
{
    private bool _closeConfirmed;
    private bool _closeCheckRunning;
    private bool? _wasCompact;

    public MainWindow()
    {
        InitializeComponent();
        DataContextChanged += (_, _) => AttachProjectFolderPicker();
        AttachProjectFolderPicker();
        Opened += (_, _) =>
        {
            AttachProjectFolderPicker();
            RefreshWindowIcon();
            ApplyWindowChromeForState();
            ApplyResponsiveBreakpoint();
            AppIconDesktopSync.QueueSync();
        };
        PropertyChanged += OnWindowPropertyChanged;
        AppIconPainter.IconColorsChanged += OnIconColorsChanged;
        Closed += (_, _) => AppIconPainter.IconColorsChanged -= OnIconColorsChanged;
    }

    private void AttachProjectFolderPicker()
    {
        if (DataContext is MainWindowViewModel viewModel)
        {
            // 原生目录选择器属于仍附着于桌面的顶层窗口，不能挂在可能被导航切走的 WelcomeView 上。
            viewModel.Welcome.SetProjectFolderPicker(PickProjectFolderAsync);
        }
    }

    private async Task<string?> PickProjectFolderAsync(string? title)
    {
        if (!StorageProvider.CanPickFolder)
        {
            throw new BackendException("external", "the active desktop storage provider cannot pick folders");
        }

        var folders = await StorageProvider.OpenFolderPickerAsync(new FolderPickerOpenOptions
        {
            Title = string.IsNullOrWhiteSpace(title) ? null : title,
            AllowMultiple = false,
        });
        return folders.FirstOrDefault()?.Path.LocalPath;
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
            // 只在进入窄屏时自动收起；窄屏内用户重新展开后，布局刷新不能再次抢回控制权。
            if (ShouldAutoCollapseSidebar(_wasCompact, compact) && vm.SidebarExpanded)
            {
                vm.SidebarExpanded = false;
            }
        }
        _wasCompact = compact;
    }

    internal static bool ShouldAutoCollapseSidebar(bool? wasCompact, bool isCompact)
        => isCompact && wasCompact != true;

    /// <summary>最大化时去掉圆角与边框，普通态恢复悬浮圆角窗（U61）；同步最大化/还原图标。</summary>
    private void ApplyWindowChromeForState()
    {
        if (WindowChrome is null)
        {
            return;
        }

        var maximized = WindowState == WindowState.Maximized;
        if (maximized)
        {
            WindowChrome.CornerRadius = new CornerRadius(0);
            WindowChrome.BorderThickness = new Thickness(0);
        }
        else
        {
            WindowChrome.CornerRadius = new CornerRadius(10);
            WindowChrome.BorderThickness = new Thickness(1);
        }

        if (MaximizeRestoreIcon is not null
            && this.TryFindResource(
                maximized ? "Ariadne.Icon.Restore" : "Ariadne.Icon.Maximize",
                out var geometry)
            && geometry is Avalonia.Media.Geometry pathData)
        {
            MaximizeRestoreIcon.Data = pathData;
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
        if (_closeCheckRunning)
        {
            return;
        }

        _closeCheckRunning = true;
        try
        {
            if (DataContext is MainWindowViewModel { CurrentPage: IUnsavedChangesGuard guard }
                && !await guard.ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
            {
                return;
            }
            _closeConfirmed = true;
            Close();
        }
        finally
        {
            _closeCheckRunning = false;
        }
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
