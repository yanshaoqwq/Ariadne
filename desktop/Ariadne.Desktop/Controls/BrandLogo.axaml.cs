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
        catch (Exception error)
        {
            // 静默失败的**适用范围**：资源字典还没加载好、母版资源缺失、
            // 渲染平台尚不可用这一类**会抛异常**的情况。此时宁可不画 Logo，
            // 也不该让整个窗口起不来。
            //
            // ⚠️ 它**拦不住** U10000 那一类缺陷（「图案画出来了但看不见」）：
            // 那条路上 ResolveColor 正常返回、RenderLineBitmap 正常产出位图、
            // Source 正常赋值，**全程零异常**，问题只在成像的可见度上。
            // ⇒ 下一个人若在查「Logo 不显示」，**别在这个 catch 上花时间**，
            // 它查不出那类问题；去量渲染产物的可见像素占比
            // （见 BrandLogoVisibilityTests）。
            //
            // 留一条痕迹而不是纯 `catch {}`：吞掉异常且不留痕会让「资源真的缺了」
            // 与「一切正常」在现象上完全同形。照项目现有做法走 Console.Error，
            // 不引新日志框架。
            Console.Error.WriteLine(
                $"[BrandLogo] 重着色失败（OnAccent={OnAccent}），本次不更新图像：{error.Message}");
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
