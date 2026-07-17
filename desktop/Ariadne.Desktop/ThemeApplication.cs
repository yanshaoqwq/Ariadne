using Avalonia;
using Avalonia.Controls;
using Avalonia.Media;
using Avalonia.Styling;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop;

internal sealed record ThemeColorTokens(
    Color Window,
    Color Main,
    Color Surface,
    Color Elevated,
    Color Subtle,
    Color Canvas,
    Color Editor,
    Color AccentPrimary,
    Color AccentHover,
    Color AccentPressed,
    Color TextOnAccent,
    Color TextPrimary,
    Color TextHeading,
    Color TextSecondary,
    Color TextSubtle,
    Color StatusError,
    Color StatusWarning,
    Color StatusInfo);

/// <summary>
/// 运行时应用个性化主题。
/// 预设主题 + 可选自定义三色（主底 / 表面 / 强调）；跟随系统时可分别指定昼/夜三色。
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
        "Ariadne.TextPrimary",
        "Ariadne.TextHeading",
        "Ariadne.TextSecondary",
        "Ariadne.TextSubtle",
        "Ariadne.EditorSelection",
    };

    private static string? _lastTheme;
    private static string? _lastMain;
    private static string? _lastSurface;
    private static string? _lastBrand;
    private static string? _lastMainDark;
    private static string? _lastSurfaceDark;
    private static string? _lastBrandDark;
    private static bool _lastFollowSystem;
    private static bool _variantHooked;

    public static void Apply(string? theme)
        => Apply(theme, null, null, null);

    /// <param name="mainHex">主底（可空 = 主题预设）</param>
    /// <param name="surfaceHex">表面</param>
    /// <param name="brandHex">强调</param>
    public static void Apply(string? theme, string? mainHex, string? surfaceHex, string? brandHex)
        => Apply(theme, mainHex, surfaceHex, brandHex, null, null, null, followSystemColors: false);

    /// <summary>
    /// 应用主题。若 <paramref name="followSystemColors"/> 为 true（通常 theme=system），
    /// 则按当前系统明暗选用昼/夜两套三色。
    /// </summary>
    public static void Apply(
        string? theme,
        string? mainHex,
        string? surfaceHex,
        string? brandHex,
        string? mainDarkHex,
        string? surfaceDarkHex,
        string? brandDarkHex,
        bool followSystemColors)
    {
        if (Application.Current is null)
        {
            return;
        }

        _lastTheme = theme;
        _lastMain = mainHex;
        _lastSurface = surfaceHex;
        _lastBrand = brandHex;
        _lastMainDark = mainDarkHex;
        _lastSurfaceDark = surfaceDarkHex;
        _lastBrandDark = brandDarkHex;
        _lastFollowSystem = followSystemColors;
        EnsureActualThemeVariantHook();

        var palette = ThemeCatalog.Resolve(theme);
        Application.Current.RequestedThemeVariant = palette.Id switch
        {
            "system" => ThemeVariant.Default,
            _ when palette.IsDark => ThemeVariant.Dark,
            _ => ThemeVariant.Light,
        };

        var isDark = ResolveIsDark(palette, followSystemColors || palette.Id == "system");
        // Single resolver for day/night custom colors (U5/U71) — tests and Apply share SelectActiveCustomColors.
        var selected = SelectActiveCustomColors(
            isDark,
            followSystemColors,
            mainHex,
            surfaceHex,
            brandHex,
            mainDarkHex,
            surfaceDarkHex,
            brandDarkHex);
        string? useMain = selected.Main;
        string? useSurface = selected.Surface;
        string? useBrand = selected.Brand;

        var hasCustom = HasHex(useMain) || HasHex(useSurface) || HasHex(useBrand);
        if (!hasCustom && palette.UseDictionaryOnly)
        {
            ClearOverlay();
            AppIconPainter.NotifyIconColorsChanged();
            return;
        }

        // 跟随系统且无自定义时，用 light/dark 预设色，避免 system 字典 + 近黑 surface 演示污染
        var baseMain = isDark && palette.Id == "system"
            ? ThemeCatalog.Resolve("dark").SwatchMain
            : palette.SwatchMain;
        var baseSurface = isDark && palette.Id == "system"
            ? ThemeCatalog.Resolve("dark").SwatchSurface
            : palette.SwatchSurface;
        var baseBrand = isDark && palette.Id == "system"
            ? ThemeCatalog.Resolve("dark").SwatchBrand
            : palette.SwatchBrand;

        var main = ParseHexOr(useMain, baseMain);
        var surface = ParseHexOr(useSurface, baseSurface);
        var brand = ParseHexOr(useBrand, baseBrand);
        WriteThreeColorOverlay(isDark, main, surface, brand, palette.Id);
        AppIconPainter.NotifyIconColorsChanged();
    }

    /// <summary>系统明暗热切换时重算三色与文字 token（U5）。</summary>
    private static void EnsureActualThemeVariantHook()
    {
        if (_variantHooked || Application.Current is null)
        {
            return;
        }

        Application.Current.ActualThemeVariantChanged += (_, _) =>
        {
            if (_lastFollowSystem || string.Equals(_lastTheme, "system", StringComparison.OrdinalIgnoreCase))
            {
                Apply(
                    _lastTheme,
                    _lastMain,
                    _lastSurface,
                    _lastBrand,
                    _lastMainDark,
                    _lastSurfaceDark,
                    _lastBrandDark,
                    followSystemColors: _lastFollowSystem || string.Equals(_lastTheme, "system", StringComparison.OrdinalIgnoreCase));
            }
        };
        _variantHooked = true;
    }

    /// <summary>由三色推导整套工作台 token 并写入覆盖层。</summary>
    public static void WriteThreeColorOverlay(bool isDark, Color main, Color surface, Color brand, string overlayId = "custom")
    {
        if (Application.Current is null)
        {
            return;
        }

        var tokens = BuildThreeColorTokens(isDark, main, surface, brand);

        var resources = Application.Current.Resources;
        SetBrush(resources, "Ariadne.WindowBase", tokens.Window);
        SetBrush(resources, "Ariadne.BackgroundMain", tokens.Main);
        SetBrush(resources, "Ariadne.BackgroundSurface", tokens.Surface);
        SetBrush(resources, "Ariadne.BackgroundElevated", tokens.Elevated);
        SetBrush(resources, "Ariadne.BackgroundSubtle", tokens.Subtle);
        SetBrush(resources, "Ariadne.CanvasBackground", tokens.Canvas);
        SetBrush(resources, "Ariadne.EditorBackground", tokens.Editor);
        SetBrush(resources, "Ariadne.AccentPrimary", tokens.AccentPrimary);
        SetBrush(resources, "Ariadne.AccentHover", tokens.AccentHover);
        SetBrush(resources, "Ariadne.AccentPressed", tokens.AccentPressed);
        SetBrush(resources, "Ariadne.AccentLight", WithAlpha(tokens.AccentPrimary, 0x1F));
        SetBrush(resources, "Ariadne.AccentBorder", WithAlpha(tokens.AccentPrimary, 0x66));
        SetBrush(resources, "Ariadne.FocusRing", tokens.AccentPrimary);
        SetBrush(resources, "Ariadne.NodeSelected", tokens.AccentPrimary);
        SetBrush(resources, "Ariadne.EdgeData", tokens.AccentPrimary);
        SetBrush(resources, "Ariadne.RuntimeRunning", tokens.AccentPrimary);
        SetBrush(resources, "Ariadne.GitCurrent", tokens.AccentPrimary);
        SetBrush(resources, "Ariadne.TextOnAccent", tokens.TextOnAccent);
        SetBrush(resources, "Ariadne.TextPrimary", tokens.TextPrimary);
        SetBrush(resources, "Ariadne.TextHeading", tokens.TextHeading);
        SetBrush(resources, "Ariadne.TextSecondary", tokens.TextSecondary);
        SetBrush(resources, "Ariadne.TextSubtle", tokens.TextSubtle);
        SetBrush(resources, "Ariadne.EditorSelection", WithAlpha(tokens.AccentPrimary, 0x2E));

        // U70：日志 chip 与状态色并入同一 resolver，按表面明暗派生，避免自定义主题下固定字典色失对比度。
        SetBrush(resources, "Ariadne.StatusError", tokens.StatusError);
        SetBrush(resources, "Ariadne.StatusWarning", tokens.StatusWarning);
        SetBrush(resources, "Ariadne.StatusInfo", tokens.StatusInfo);
        SetBrush(resources, "Ariadne.LogErrorBg", WithAlpha(tokens.StatusError, 0x28));
        SetBrush(resources, "Ariadne.LogWarningBg", WithAlpha(tokens.StatusWarning, 0x28));
        SetBrush(resources, "Ariadne.LogInfoBg", WithAlpha(tokens.StatusInfo, 0x28));

        resources[OverlayKey] = overlayId;
    }

    internal static ThemeColorTokens BuildThreeColorTokens(bool isDark, Color main, Color surface, Color brand)
    {
        var window = isDark ? Darken(main, 0.14) : Darken(main, 0.06);
        var subtle = isDark ? Lighten(main, 0.08) : Darken(main, 0.08);
        var elevated = isDark ? Lighten(surface, 0.10) : surface;
        var canvas = isDark ? Blend(main, surface, 0.25) : Blend(main, surface, 0.45);
        var onAccent = ThemeAccessibilityAudit.BestTextOn(brand);
        var usesDarkAccentText = onAccent == Colors.Black;
        var hover = usesDarkAccentText ? Lighten(brand, 0.10) : Darken(brand, 0.10);
        var pressed = usesDarkAccentText ? Lighten(brand, 0.04) : Darken(brand, 0.18);

        var darkText = Color.FromRgb(0x1B, 0x1F, 0x22);
        var lightText = Color.FromRgb(0xF2, 0xF4, 0xF6);
        var useLightText = ThemeAccessibilityAudit.ContrastRatio(lightText, surface)
            > ThemeAccessibilityAudit.ContrastRatio(darkText, surface);
        var textPrimary = useLightText ? lightText : darkText;
        var textHeading = useLightText
            ? Color.FromRgb(0xFA, 0xFB, 0xFC)
            : Color.FromRgb(0x12, 0x15, 0x18);
        var textSecondary = useLightText
            ? Color.FromRgb(0xB0, 0xB8, 0xC0)
            : Color.FromRgb(0x5B, 0x64, 0x69);
        var textSubtle = useLightText
            ? Color.FromRgb(0x86, 0x8E, 0x96)
            : Color.FromRgb(0x7A, 0x84, 0x8C);
        var statusError = isDark ? Color.FromRgb(0xF0, 0x71, 0x78) : Color.FromRgb(0xC9, 0x3C, 0x37);
        var statusWarning = isDark ? Color.FromRgb(0xE3, 0xB3, 0x41) : Color.FromRgb(0x9A, 0x67, 0x00);
        var statusInfo = isDark ? Color.FromRgb(0x79, 0xC0, 0xFF) : Color.FromRgb(0x09, 0x6B, 0xC0);

        return new ThemeColorTokens(
            window, main, surface, elevated, subtle, canvas, surface,
            brand, hover, pressed, onAccent,
            textPrimary, textHeading, textSecondary, textSubtle,
            statusError, statusWarning, statusInfo);
    }

    /// <summary>解析当前是否应按暗色方案应用（含跟随系统）。</summary>
    public static bool ResolveIsDark(ThemePalette palette, bool respectSystem)
    {
        if (respectSystem && palette.Id == "system" && Application.Current is not null)
        {
            return Application.Current.ActualThemeVariant == ThemeVariant.Dark;
        }

        return palette.IsDark;
    }

    /// <summary>
    /// 在昼/夜两套三色中选出当前生效的一套（纯函数，便于单测）。
    /// </summary>
    public static (string? Main, string? Surface, string? Brand) SelectActiveCustomColors(
        bool isDark,
        bool followSystemColors,
        string? mainLight,
        string? surfaceLight,
        string? brandLight,
        string? mainDark,
        string? surfaceDark,
        string? brandDark)
    {
        if (followSystemColors && isDark)
        {
            return (
                HasHex(mainDark) ? mainDark : mainLight,
                HasHex(surfaceDark) ? surfaceDark : surfaceLight,
                HasHex(brandDark) ? brandDark : brandLight);
        }

        return (mainLight, surfaceLight, brandLight);
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
