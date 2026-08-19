using Avalonia;
using Avalonia.Animation;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Interactivity;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class MainWindow : Window
{
    private bool _closeConfirmed;
    private bool _closeCheckRunning;
    private bool? _wasCompact;
    private readonly Func<string?, Task<string?>> _projectFolderPicker;
    private WelcomeViewModel? _attachedWelcome;

    public MainWindow()
    {
        InitializeComponent();
        _projectFolderPicker = PickProjectFolderAsync;
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
        MotionPreferences.Changed += OnMotionPreferencesChanged;
        ApplyPageTransition();
        ApplyMotionPreferenceClass();
        Closed += (_, _) =>
        {
            DetachProjectFolderPicker();
            AppIconPainter.IconColorsChanged -= OnIconColorsChanged;
            MotionPreferences.Changed -= OnMotionPreferencesChanged;
        };
    }

    private void OnMotionPreferencesChanged(object? sender, EventArgs e)
        => Dispatcher.UIThread.Post(() =>
        {
            ApplyPageTransition();
            ApplyMotionPreferenceClass();
        });

    /// <summary>
    /// U178-D/E/F：把「减少动态效果」偏好投射成 Window 上的一个 class，
    /// 供主题里 `Window.reduce-motion …` 那组样式把 `Transitions` 置空。
    ///
    /// **为什么用 class 而不是像 <see cref="ApplyPageTransition"/> 那样直接赋值**：
    /// 这轮的 5 条过渡分散在主题文件的 5 个选择器上（侧栏宽度 / rail-face /
    /// 遮罩 / 弹窗面板 / Expander 头部），其中两条还落在 Fluent 的模板内部
    /// （`/template/` 层），code-behind 根本拿不到那些控件实例。
    /// 而 class 是声明式的，能一次覆盖到模板内部——这是唯一可行的路径。
    ///
    /// 反过来 `PageTransition` 必须走 code-behind：它是对象属性、没有样式承载。
    /// 两种做法并存不是不一致，是各自唯一可行的那条。
    ///
    /// ⚠️ 覆盖能赢是因为**选择器更具体**（`Window.reduce-motion Border.app-rail`
    /// 比 `Border.app-rail` 多一层带类祖先），不是因为声明在后面——
    /// 后者那个说法被变异测试推翻了（把门控块搬到文件最前面，门控照样生效）。
    /// 已实测可逆：加类 ⇒ Transitions 变空集，摘类 ⇒ 恢复原过渡。
    /// </summary>
    private void ApplyMotionPreferenceClass()
        => Classes.Set("reduce-motion", MotionPreferences.ReduceMotion);

    /// <summary>
    /// U178-A：切页过渡随「减少动效」偏好挂/摘。
    ///
    /// **为什么在 code-behind 而不是 XAML**：`PageTransition` 是个对象属性，
    /// 没有主题令牌可以承载它，写死在 XAML 里就等于让 ReduceMotion 在
    /// 「切页」这条最显眼的路径上失效——而这正是该偏好最该管住的一处。
    /// 关闭时置 null（不是把 Duration 设 0）：null 让 TransitioningContentControl
    /// 走同步换内容的快路径，0 时长仍要跑一遍动画调度。
    ///
    /// 150ms 是既有尺度的中段（全仓 74 处过渡里 67 处落在 120–180ms），
    /// 刻意不自创时长——同一个产品里的过渡应当是同一种节奏。
    /// </summary>
    private void ApplyPageTransition()
    {
        if (PageHost is null)
        {
            return;
        }

        PageHost.PageTransition = MotionPreferences.ReduceMotion
            ? null
            : new CrossFade(TimeSpan.FromMilliseconds(150));
    }

    private void AttachProjectFolderPicker()
    {
        DetachProjectFolderPicker();
        if (DataContext is MainWindowViewModel viewModel)
        {
            // 原生目录选择器属于仍附着于桌面的顶层窗口，不能挂在可能被导航切走的 WelcomeView 上。
            viewModel.Welcome.SetProjectFolderPicker(_projectFolderPicker);
            _attachedWelcome = viewModel.Welcome;
        }
    }

    private void DetachProjectFolderPicker()
    {
        _attachedWelcome?.ClearProjectFolderPicker(_projectFolderPicker);
        _attachedWelcome = null;
    }

    private async Task<string?> PickProjectFolderAsync(string? title)
    {
        if (!StorageProvider.CanPickFolder)
        {
            throw new BackendException(
                "external",
                DisplayNameService.Current.Text("ui.settings.browse_unavailable"));
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
            if (DataContext is MainWindowViewModel viewModel
                && !await viewModel.ConfirmCloseAsync().ConfigureAwait(true))
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
        if (!_closeConfirmed
            && DataContext is MainWindowViewModel { HasCachedUnsavedChanges: true })
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
