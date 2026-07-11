using Avalonia.Media;

namespace Ariadne.Desktop;

/// <summary>
/// 个性化主题目录。对齐 <c>指导性文件/settings-theme-spec.md</c> §6：
/// 11 套主题（基础 5 + 浅色强调 4 + 深色强调 2），青绿为默认强调而非唯一品牌色。
/// 交互与布局权威见 <c>指导性文件/ui设计方案.md</c>（Avalonia 桌面，无 WebUI）。
/// </summary>
public sealed class ThemePalette
{
    public ThemePalette(
        string id,
        string group,
        bool isDark,
        bool useDictionaryOnly,
        Color swatchMain,
        Color swatchSurface,
        Color swatchBrand,
        Color windowBase,
        Color backgroundMain,
        Color backgroundSurface,
        Color backgroundElevated,
        Color backgroundSubtle,
        Color canvasBackground,
        Color editorBackground,
        Color accentPrimary,
        Color accentHover,
        Color accentPressed,
        Color textOnAccent)
    {
        Id = id;
        Group = group;
        IsDark = isDark;
        UseDictionaryOnly = useDictionaryOnly;
        SwatchMain = swatchMain;
        SwatchSurface = swatchSurface;
        SwatchBrand = swatchBrand;
        WindowBase = windowBase;
        BackgroundMain = backgroundMain;
        BackgroundSurface = backgroundSurface;
        BackgroundElevated = backgroundElevated;
        BackgroundSubtle = backgroundSubtle;
        CanvasBackground = canvasBackground;
        EditorBackground = editorBackground;
        AccentPrimary = accentPrimary;
        AccentHover = accentHover;
        AccentPressed = accentPressed;
        TextOnAccent = textOnAccent;
    }

    public string Id { get; }
    /// <summary>base | light_accent | dark_accent</summary>
    public string Group { get; }
    public bool IsDark { get; }
    /// <summary>true = 仅切 Light/Dark/Default 字典，不写运行时覆盖（system/light/dark）。</summary>
    public bool UseDictionaryOnly { get; }
    public Color SwatchMain { get; }
    public Color SwatchSurface { get; }
    public Color SwatchBrand { get; }
    public Color WindowBase { get; }
    public Color BackgroundMain { get; }
    public Color BackgroundSurface { get; }
    public Color BackgroundElevated { get; }
    public Color BackgroundSubtle { get; }
    public Color CanvasBackground { get; }
    public Color EditorBackground { get; }
    public Color AccentPrimary { get; }
    public Color AccentHover { get; }
    public Color AccentPressed { get; }
    public Color TextOnAccent { get; }

    public Color AccentLight => WithAlpha(AccentPrimary, 0x1F);
    public Color AccentBorder => WithAlpha(AccentPrimary, 0x66);
    public Color FocusRing => AccentPrimary;
    public Color NodeSelected => AccentPrimary;
    public Color EdgeData => AccentPrimary;
    public Color RuntimeRunning => AccentPrimary;
    public Color GitCurrent => AccentPrimary;
    public Color EditorSelection => WithAlpha(AccentPrimary, 0x2E);

    private static Color WithAlpha(Color color, byte alpha) =>
        Color.FromArgb(alpha, color.R, color.G, color.B);
}

public static class ThemeCatalog
{
    public const string DefaultThemeId = "system";

    private static readonly ThemePalette[] Palettes =
    {
        // 基础 · 跟随系统：演示色块用「昼」方案，禁止近纯黑表面（否则看起来像整块死黑不可用）
        Entry("system", "base", isDark: false, dictOnly: true,
            main: "#F6F7F6", surface: "#FFFFFF", brand: "#356F68",
            window: "#EFF2F5", bgMain: "#F6F8FA", bgSurface: "#FFFFFF", bgElev: "#FFFFFF", bgSubtle: "#EEF2F6",
            canvas: "#F2F6F8", editor: "#FAFBFC", onAccent: "#FFFFFF"),
        Entry("light", "base", isDark: false, dictOnly: true,
            main: "#F6F7F6", surface: "#FFFFFF", brand: "#356F68",
            window: "#EFF2F5", bgMain: "#F6F8FA", bgSurface: "#FFFFFF", bgElev: "#FFFFFF", bgSubtle: "#EEF2F6",
            canvas: "#F2F6F8", editor: "#FAFBFC", onAccent: "#FFFFFF"),
        Entry("dark", "base", isDark: true, dictOnly: true,
            main: "#121417", surface: "#1B1F23", brand: "#70B8AC",
            window: "#1C1E21", bgMain: "#17191C", bgSurface: "#212427", bgElev: "#2A2E31", bgSubtle: "#1D2023",
            canvas: "#17191C", editor: "#212427", onAccent: "#07110F"),
        Entry("ink", "base", isDark: false, dictOnly: false,
            main: "#F3F2EC", surface: "#FAF9F4", brand: "#2F6760",
            window: "#EDECE4", bgMain: "#F3F2EC", bgSurface: "#FAF9F4", bgElev: "#FFFEFA", bgSubtle: "#EBEAE3",
            canvas: "#F0EFE8", editor: "#FAF9F4", onAccent: "#FFFFFF",
            hover: "#275750", pressed: "#1F4842"),
        Entry("contrast", "base", isDark: false, dictOnly: false,
            main: "#FFFFFF", surface: "#F8FAFC", brand: "#0F766E",
            window: "#F1F5F9", bgMain: "#FFFFFF", bgSurface: "#F8FAFC", bgElev: "#FFFFFF", bgSubtle: "#F1F5F9",
            canvas: "#F8FAFC", editor: "#FFFFFF", onAccent: "#FFFFFF",
            hover: "#0D9488", pressed: "#0F766E"),

        // 浅色强调
        Entry("azure", "light_accent", isDark: false, dictOnly: false,
            main: "#F5F7FA", surface: "#FFFFFF", brand: "#2563EB",
            window: "#EEF2F7", bgMain: "#F5F7FA", bgSurface: "#FFFFFF", bgElev: "#FFFFFF", bgSubtle: "#E8EEF6",
            canvas: "#F0F4F9", editor: "#FFFFFF", onAccent: "#FFFFFF",
            hover: "#1D4ED8", pressed: "#1E40AF"),
        Entry("rose", "light_accent", isDark: false, dictOnly: false,
            main: "#FAF6F7", surface: "#FFFFFF", brand: "#C43D63",
            window: "#F4EEF0", bgMain: "#FAF6F7", bgSurface: "#FFFFFF", bgElev: "#FFFFFF", bgSubtle: "#F3EAED",
            canvas: "#F7F0F2", editor: "#FFFFFF", onAccent: "#FFFFFF",
            hover: "#A83254", pressed: "#8F2A48"),
        Entry("amber", "light_accent", isDark: false, dictOnly: false,
            main: "#FAF8F3", surface: "#FFFEFB", brand: "#B4690E",
            window: "#F3EFE6", bgMain: "#FAF8F3", bgSurface: "#FFFEFB", bgElev: "#FFFFFF", bgSubtle: "#F2EEE4",
            canvas: "#F6F2E9", editor: "#FFFEFB", onAccent: "#FFFFFF",
            hover: "#9A5A0C", pressed: "#7C480A"),
        Entry("violet", "light_accent", isDark: false, dictOnly: false,
            main: "#F7F6FA", surface: "#FFFFFF", brand: "#6B4AC4",
            window: "#EFEDF5", bgMain: "#F7F6FA", bgSurface: "#FFFFFF", bgElev: "#FFFFFF", bgSubtle: "#EDEAF5",
            canvas: "#F2F0F8", editor: "#FFFFFF", onAccent: "#FFFFFF",
            hover: "#5B3EAD", pressed: "#4C3491"),

        // 深色强调
        Entry("dusk", "dark_accent", isDark: true, dictOnly: false,
            main: "#17141A", surface: "#211D26", brand: "#E07A9E",
            window: "#141117", bgMain: "#17141A", bgSurface: "#211D26", bgElev: "#2A2430", bgSubtle: "#1C1820",
            canvas: "#17141A", editor: "#211D26", onAccent: "#1A0F14",
            hover: "#E892B0", pressed: "#C9668A"),
        Entry("slate", "dark_accent", isDark: true, dictOnly: false,
            main: "#0F1419", surface: "#171E26", brand: "#5B9DF0",
            window: "#0C1014", bgMain: "#0F1419", bgSurface: "#171E26", bgElev: "#1F2833", bgSubtle: "#121820",
            canvas: "#0F1419", editor: "#171E26", onAccent: "#061018",
            hover: "#7BB0F3", pressed: "#4A8AD6"),
    };

    public static IReadOnlyList<ThemePalette> All => Palettes;

    public static ThemePalette Resolve(string? themeId)
    {
        var id = Normalize(themeId);
        return Palettes.FirstOrDefault(p => p.Id == id) ?? Palettes[0];
    }

    public static string Normalize(string? themeId)
    {
        var id = (themeId ?? string.Empty).Trim().ToLowerInvariant();
        if (string.IsNullOrEmpty(id))
        {
            return DefaultThemeId;
        }

        return Palettes.Any(p => p.Id == id) ? id : DefaultThemeId;
    }

    /// <summary>
    /// 判断主题演示条是否「不可用」：主底与表面都接近纯黑，或表面本身是纯黑。
    /// 供测试与 UI 校验「跟随系统」演示色。
    /// </summary>
    public static bool IsUnusableDemoSwatch(Color main, Color surface)
    {
        static bool NearBlack(Color c) => c.R < 24 && c.G < 24 && c.B < 24;
        return NearBlack(surface) || (NearBlack(main) && NearBlack(surface));
    }

    /// <summary>跟随系统预设的演示 swatch（昼侧，保证非纯黑）。</summary>
    public static (Color Main, Color Surface, Color Brand) SystemDemoSwatches()
    {
        var p = Resolve("system");
        return (p.SwatchMain, p.SwatchSurface, p.SwatchBrand);
    }

    private static ThemePalette Entry(
        string id,
        string group,
        bool isDark,
        bool dictOnly,
        string main,
        string surface,
        string brand,
        string window,
        string bgMain,
        string bgSurface,
        string bgElev,
        string bgSubtle,
        string canvas,
        string editor,
        string onAccent,
        string? hover = null,
        string? pressed = null)
    {
        var brandColor = Color.Parse(brand);
        return new ThemePalette(
            id,
            group,
            isDark,
            dictOnly,
            Color.Parse(main),
            Color.Parse(surface),
            brandColor,
            Color.Parse(window),
            Color.Parse(bgMain),
            Color.Parse(bgSurface),
            Color.Parse(bgElev),
            Color.Parse(bgSubtle),
            Color.Parse(canvas),
            Color.Parse(editor),
            brandColor,
            Color.Parse(hover ?? brand),
            Color.Parse(pressed ?? brand),
            Color.Parse(onAccent));
    }
}
