using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Styling;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop;

/// <summary>
/// 运行时应用个性化主题。
/// 预设主题 + 可选自定义三色（主底 / 表面 / 强调）。
/// </summary>
public static class ThemeApplication
{
    private const string OverlayKey = "Ariadne.ThemeOverlay.Active";

    private static readonly string[] OverlayBrushKeys =
    {
        "Ariadne.WindowBase",
        "Ariadne.BackgroundMain",
        "Ariadne.BackgroundSurface",
        "Ariadne.BackgroundElevated",
        "Ariadne.BackgroundSubtle",
        "Ariadne.CanvasBackground",
        "Ariadne.EditorBackground",
        "Ariadne.AccentPrimary",
        "Ariadne.AccentHover",
        "Ariadne.AccentPressed",
        "Ariadne.AccentLight",
        "Ariadne.AccentBorder",
        "Ariadne.FocusRing",
        "Ariadne.NodeSelected",
        "Ariadne.EdgeData",
        "Ariadne.RuntimeRunning",
        "Ariadne.GitCurrent",
        "Ariadne.TextOnAccent",
        "Ariadne.EditorSelection",
    };

    public static void Apply(string? theme)
        => Apply(theme, null, null, null);

    /// <param name="mainHex">主底（可空 = 主题预设）</param>
    /// <param name="surfaceHex">表面</param>
    /// <param name="brandHex">强调</param>
    public static void Apply(string? theme, string? mainHex, string? surfaceHex, string? brandHex)
    {
        if (Application.Current is null)
        {
            return;
        }

        var palette = ThemeCatalog.Resolve(theme);
        Application.Current.RequestedThemeVariant = palette.Id switch
        {
            "system" => ThemeVariant.Default,
            _ when palette.IsDark => ThemeVariant.Dark,
            _ => ThemeVariant.Light,
        };

        var hasCustom = HasHex(mainHex) || HasHex(surfaceHex) || HasHex(brandHex);
        if (!hasCustom && palette.UseDictionaryOnly)
        {
            ClearOverlay();
            AppIconPainter.NotifyIconColorsChanged();
            return;
        }

        var main = ParseHexOr(mainHex, palette.SwatchMain);
        var surface = ParseHexOr(surfaceHex, palette.SwatchSurface);
        var brand = ParseHexOr(brandHex, palette.SwatchBrand);
        WriteThreeColorOverlay(palette.IsDark, main, surface, brand, palette.Id);
        AppIconPainter.NotifyIconColorsChanged();
    }

    /// <summary>由三色推导整套工作台 token 并写入覆盖层。</summary>
    public static void WriteThreeColorOverlay(bool isDark, Color main, Color surface, Color brand, string overlayId = "custom")
    {
        if (Application.Current is null)
        {
            return;
        }

        var window = isDark ? Darken(main, 0.12) : Lighten(main, 0.04);
        var subtle = isDark ? Lighten(main, 0.06) : Darken(main, 0.04);
        var elevated = isDark ? Lighten(surface, 0.08) : Lighten(surface, 0.02);
        var canvas = isDark ? main : Blend(main, surface, 0.35);
        var editor = surface;
        var hover = isDark ? Lighten(brand, 0.12) : Darken(brand, 0.10);
        var pressed = isDark ? Darken(brand, 0.10) : Darken(brand, 0.18);
        var onAccent = Luminance(brand) > 0.55
            ? Color.FromRgb(0x08, 0x10, 0x12)
            : Colors.White;

        var resources = Application.Current.Resources;
        SetBrush(resources, "Ariadne.WindowBase", window);
        SetBrush(resources, "Ariadne.BackgroundMain", main);
        SetBrush(resources, "Ariadne.BackgroundSurface", surface);
        SetBrush(resources, "Ariadne.BackgroundElevated", elevated);
        SetBrush(resources, "Ariadne.BackgroundSubtle", subtle);
        SetBrush(resources, "Ariadne.CanvasBackground", canvas);
        SetBrush(resources, "Ariadne.EditorBackground", editor);
        SetBrush(resources, "Ariadne.AccentPrimary", brand);
        SetBrush(resources, "Ariadne.AccentHover", hover);
        SetBrush(resources, "Ariadne.AccentPressed", pressed);
        SetBrush(resources, "Ariadne.AccentLight", WithAlpha(brand, 0x1F));
        SetBrush(resources, "Ariadne.AccentBorder", WithAlpha(brand, 0x66));
        SetBrush(resources, "Ariadne.FocusRing", brand);
        SetBrush(resources, "Ariadne.NodeSelected", brand);
        SetBrush(resources, "Ariadne.EdgeData", brand);
        SetBrush(resources, "Ariadne.RuntimeRunning", brand);
        SetBrush(resources, "Ariadne.GitCurrent", brand);
        SetBrush(resources, "Ariadne.TextOnAccent", onAccent);
        SetBrush(resources, "Ariadne.EditorSelection", WithAlpha(brand, 0x2E));
        resources[OverlayKey] = overlayId;
    }

    private static void ClearOverlay()
    {
        if (Application.Current is null)
        {
            return;
        }

        var resources = Application.Current.Resources;
        foreach (var key in OverlayBrushKeys)
        {
            resources.Remove(key);
        }
        resources.Remove(OverlayKey);
    }

    private static void SetBrush(IResourceDictionary resources, string key, Color color)
    {
        resources[key] = new SolidColorBrush(color);
    }

    public static bool HasHex(string? hex) =>
        !string.IsNullOrWhiteSpace(hex) && ColorChannelEditor.TryParseHex(hex, out _, out _, out _);

    public static Color ParseHexOr(string? hex, Color fallback)
    {
        if (ColorChannelEditor.TryParseHex(hex, out var r, out var g, out var b))
        {
            return Color.FromRgb(r, g, b);
        }

        return fallback;
    }

    public static string ToHex(Color c) => ColorChannelEditor.ToHex(c.R, c.G, c.B);

    private static Color WithAlpha(Color color, byte alpha) =>
        Color.FromArgb(alpha, color.R, color.G, color.B);

    private static double Luminance(Color c) =>
        (0.2126 * c.R + 0.7152 * c.G + 0.0722 * c.B) / 255.0;

    private static Color Darken(Color c, double amount)
    {
        amount = Math.Clamp(amount, 0, 1);
        return Color.FromRgb(
            (byte)(c.R * (1 - amount)),
            (byte)(c.G * (1 - amount)),
            (byte)(c.B * (1 - amount)));
    }

    private static Color Lighten(Color c, double amount)
    {
        amount = Math.Clamp(amount, 0, 1);
        return Color.FromRgb(
            (byte)(c.R + (255 - c.R) * amount),
            (byte)(c.G + (255 - c.G) * amount),
            (byte)(c.B + (255 - c.B) * amount));
    }

    private static Color Blend(Color a, Color b, double t)
    {
        t = Math.Clamp(t, 0, 1);
        return Color.FromRgb(
            (byte)(a.R + (b.R - a.R) * t),
            (byte)(a.G + (b.G - a.G) * t),
            (byte)(a.B + (b.B - a.B) * t));
    }
}
