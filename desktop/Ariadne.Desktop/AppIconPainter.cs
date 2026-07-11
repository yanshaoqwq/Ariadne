using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Platform;

namespace Ariadne.Desktop;

/// <summary>
/// 应用图标重着色。
/// - 应用内 Logo：线描母版 <c>app-icon-master.png</c>
/// - 任务栏 / 桌面快捷方式：实心母版 <c>app-icon-taskbar-master.png</c>（小尺寸可读）
/// </summary>
public static class AppIconPainter
{
    private const string LineMasterAssetPath = "avares://Ariadne.Desktop/Assets/app-icon-master.png";
    private const string TaskbarMasterAssetPath = "avares://Ariadne.Desktop/Assets/app-icon-taskbar-master.png";
    private static readonly Color DefaultAccent = Color.FromRgb(0x35, 0x6F, 0x68);
    private static readonly Color DefaultPaper = Color.FromRgb(0xF6, 0xF7, 0xF6);

    private static Bitmap? _lineMaster;
    private static Bitmap? _taskbarMaster;
    private static readonly object Gate = new();

    public static event Action? IconColorsChanged;

    public static void NotifyIconColorsChanged()
    {
        IconColorsChanged?.Invoke();
        AppIconDesktopSync.QueueSync();
    }

    public static void InvalidateMasterCache()
    {
        lock (Gate)
        {
            _lineMaster?.Dispose();
            _taskbarMaster?.Dispose();
            _lineMaster = null;
            _taskbarMaster = null;
        }
    }

    /// <summary>窗口/任务栏图标：实心透明底。</summary>
    public static WindowIcon CreateWindowIcon(int size = 256)
    {
        var accent = ResolveColor("Ariadne.AccentPrimary", DefaultAccent);
        using var bmp = RenderTaskbarBitmap(accent, size);
        return new WindowIcon(bmp);
    }

    /// <summary>应用内品牌 Logo：线描 + Accent（可带纸面底色）。</summary>
    public static Bitmap CreateThemedBitmap(int size = 128)
    {
        var accent = ResolveColor("Ariadne.AccentPrimary", DefaultAccent);
        var paper = ResolveColor("Ariadne.BackgroundElevated", DefaultPaper);
        if (paper.A < 10)
        {
            paper = DefaultPaper;
        }

        return RenderLineBitmap(accent, paper, size, transparentPaper: false);
    }

    /// <summary>任务栏专用：实心剪影、透明底、画大。</summary>
    public static WriteableBitmap RenderTaskbarBitmap(Color accent, int size)
    {
        return RenderMaster(
            EnsureTaskbarMaster(),
            accent,
            Color.FromArgb(0, 0, 0, 0),
            size,
            insetFraction: 0.015);
    }

    /// <summary>线描母版渲染（应用内 Logo）。</summary>
    public static WriteableBitmap RenderLineBitmap(Color accent, Color paper, int size, bool transparentPaper = false)
    {
        var paperForMap = transparentPaper
            ? Color.FromArgb(0, 0, 0, 0)
            : paper;
        return RenderMaster(
            EnsureLineMaster(),
            accent,
            paperForMap,
            size,
            insetFraction: 0.04);
    }

    /// <summary>兼容旧调用：默认走线描（非任务栏）。</summary>
    public static WriteableBitmap RenderBitmap(Color accent, Color paper, int size)
        => RenderLineBitmap(accent, paper, size, transparentPaper: paper.A < 8);

    public static WriteableBitmap RenderBitmap(Color accent, Color paper, int size, bool transparentPaper)
        => RenderLineBitmap(accent, paper, size, transparentPaper);

    /// <summary>任意 avares 资源 → Accent 着色（空态等）。</summary>
    public static WriteableBitmap RenderAssetBitmap(string avaresPath, Color accent, int size, bool transparentPaper = true)
    {
        size = Math.Clamp(size, 16, 1024);
        using var stream = AssetLoader.Open(new Uri(avaresPath));
        using var master = new Bitmap(stream);
        return RenderMaster(master, accent,
            transparentPaper ? Color.FromArgb(0, 0, 0, 0) : ResolveColor("Ariadne.BackgroundElevated", DefaultPaper),
            size,
            insetFraction: 0.04,
            disposeMaster: false);
    }

    private static WriteableBitmap RenderMaster(
        Bitmap master,
        Color accent,
        Color paper,
        int size,
        double insetFraction,
        bool disposeMaster = false)
    {
        try
        {
            size = Math.Clamp(size, 16, 1024);
            using var scaled = new RenderTargetBitmap(new PixelSize(size, size), new Vector(96, 96));
            using (var ctx = scaled.CreateDrawingContext(true))
            {
                ctx.DrawRectangle(Brushes.Transparent, null, new Rect(0, 0, size, size));
                var inset = size * insetFraction;
                ctx.DrawImage(master, new Rect(inset, inset, size - inset * 2, size - inset * 2));
            }

            return RecolorFromRenderTarget(scaled, accent, paper);
        }
        finally
        {
            if (disposeMaster)
            {
                master.Dispose();
            }
        }
    }

    private static Bitmap EnsureLineMaster()
    {
        lock (Gate)
        {
            if (_lineMaster is not null)
            {
                return _lineMaster;
            }

            using var stream = AssetLoader.Open(new Uri(LineMasterAssetPath));
            _lineMaster = new Bitmap(stream);
            return _lineMaster;
        }
    }

    private static Bitmap EnsureTaskbarMaster()
    {
        lock (Gate)
        {
            if (_taskbarMaster is not null)
            {
                return _taskbarMaster;
            }

            using var stream = AssetLoader.Open(new Uri(TaskbarMasterAssetPath));
            _taskbarMaster = new Bitmap(stream);
            return _taskbarMaster;
        }
    }

    private static WriteableBitmap RecolorFromRenderTarget(RenderTargetBitmap source, Color accent, Color paper)
    {
        var size = source.PixelSize;
        var dest = new WriteableBitmap(size, new Vector(96, 96), PixelFormat.Bgra8888, AlphaFormat.Unpremul);

        using (var fb = dest.Lock())
        {
            source.CopyPixels(fb);
        }

        using (var fb = dest.Lock())
        {
            var buffer = new byte[fb.RowBytes * size.Height];
            System.Runtime.InteropServices.Marshal.Copy(fb.Address, buffer, 0, buffer.Length);

            for (var y = 0; y < size.Height; y++)
            {
                var row = y * fb.RowBytes;
                for (var x = 0; x < size.Width; x++)
                {
                    var i = row + x * 4;
                    var b = buffer[i];
                    var g = buffer[i + 1];
                    var r = buffer[i + 2];
                    var a = buffer[i + 3];
                    var mapped = AppIconRecolor.MapPixel(
                        r, g, b, a,
                        accent.R, accent.G, accent.B,
                        paper.R, paper.G, paper.B,
                        paper.A);
                    buffer[i] = mapped.B;
                    buffer[i + 1] = mapped.G;
                    buffer[i + 2] = mapped.R;
                    buffer[i + 3] = mapped.A;
                }
            }

            System.Runtime.InteropServices.Marshal.Copy(buffer, 0, fb.Address, buffer.Length);
        }

        return dest;
    }

    public static Color ResolveColor(string resourceKey, Color fallback)
    {
        if (Application.Current is null)
        {
            return fallback;
        }

        try
        {
            var variant = Application.Current.ActualThemeVariant;
            if (Application.Current.TryGetResource(resourceKey, variant, out var res)
                && TryColor(res, out var c))
            {
                return c;
            }

            if (Application.Current.Resources.TryGetResource(resourceKey, variant, out res)
                && TryColor(res, out c))
            {
                return c;
            }
        }
        catch
        {
            // ignore
        }

        return fallback;
    }

    private static bool TryColor(object? res, out Color color)
    {
        switch (res)
        {
            case ISolidColorBrush brush:
                color = brush.Color;
                return true;
            case Color c:
                color = c;
                return true;
            default:
                color = default;
                return false;
        }
    }
}
