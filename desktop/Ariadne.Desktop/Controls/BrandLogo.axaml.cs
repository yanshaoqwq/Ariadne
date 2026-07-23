using Avalonia;
using Avalonia.Controls;
using Avalonia.Interactivity;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Threading;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// 品牌 Logo：线描钢笔母版 + Accent 重着色（与任务栏实心图标分离）。
/// </summary>
public partial class BrandLogo : UserControl
{
    public static readonly StyledProperty<bool> OnAccentProperty =
        AvaloniaProperty.Register<BrandLogo, bool>(nameof(OnAccent), defaultValue: false);

    private Bitmap? _current;
    private bool _isAttachedToVisualTree;

    public BrandLogo()
    {
        InitializeComponent();
        PropertyChanged += (_, e) =>
        {
            if (e.Property == OnAccentProperty)
            {
                ApplyChrome();
                QueueRefresh();
            }
        };
        Loaded += OnLoaded;
        Unloaded += OnUnloaded;
    }

    public bool OnAccent
    {
        get => GetValue(OnAccentProperty);
        set => SetValue(OnAccentProperty, value);
    }

    internal bool IsAttachedForTests => _isAttachedToVisualTree;
    internal bool HasRenderedImageForTests => LogoImage?.Source is not null;

    private void OnLoaded(object? sender, RoutedEventArgs e)
    {
        _isAttachedToVisualTree = true;
        AppIconPainter.IconColorsChanged -= OnThemeColorsChanged;
        AppIconPainter.IconColorsChanged += OnThemeColorsChanged;
        ApplyChrome();
        QueueRefresh();
    }

    private void OnUnloaded(object? sender, RoutedEventArgs e)
    {
        _isAttachedToVisualTree = false;
        AppIconPainter.IconColorsChanged -= OnThemeColorsChanged;
        ClearImage();
    }

    private void ApplyChrome()
    {
        if (RootBorder is null)
        {
            return;
        }

        if (OnAccent)
        {
            RootBorder.Classes.Set("on-accent", true);
            RootBorder.Classes.Set("on-paper", false);
        }
        else
        {
            RootBorder.Classes.Set("on-paper", true);
            RootBorder.Classes.Set("on-accent", false);
        }
    }

    private void OnThemeColorsChanged()
    {
        if (!_isAttachedToVisualTree)
        {
            return;
        }

        Dispatcher.UIThread.Post(QueueRefresh, DispatcherPriority.Background);
    }

    private void QueueRefresh()
    {
        if (!_isAttachedToVisualTree || LogoImage is null)
        {
            return;
        }

        try
        {
            var px = 128;
            Bitmap next;
            if (OnAccent)
            {
                // 标题栏 Accent 底：线描用 TextOnAccent，镂空融进 Accent
                var ink = AppIconPainter.ResolveColor("Ariadne.TextOnAccent", Colors.White);
                var paper = AppIconPainter.ResolveColor("Ariadne.AccentPrimary", Color.FromRgb(0x35, 0x6F, 0x68));
                next = AppIconPainter.RenderLineBitmap(ink, paper, px, transparentPaper: false);
            }
            else
            {
                next = AppIconPainter.CreateThemedBitmap(px);
            }

            var old = _current;
            _current = next;
            LogoImage.Source = next;
            old?.Dispose();
        }
        catch
        {
            // 资源未加载时忽略
        }
    }

    private void ClearImage()
    {
        if (LogoImage is not null)
        {
            LogoImage.Source = null;
        }

        _current?.Dispose();
        _current = null;
    }
}
