using System.Text.Json;
using Avalonia.Media;

namespace Ariadne.Desktop;

internal sealed record ThemeContrastCheck(
    string ThemeVariant,
    string ForegroundToken,
    string BackgroundToken,
    double Ratio,
    double RequiredRatio,
    string Category);

internal sealed record ThemeContrastEvidence(
    int SchemaVersion,
    string Probe,
    string BuildProfile,
    int ThemeVariantCount,
    double MinimumNormalTextRatio,
    double MinimumLargeTextRatio,
    double MinimumNonTextRatio,
    IReadOnlyList<ThemeContrastCheck> Checks,
    IReadOnlyList<ThemeContrastCheck> Failures);

internal static class ThemeAccessibilityAudit
{
    private const double NormalTextRatio = 4.5;
    private const double LargeTextRatio = 3.0;
    private const double NonTextRatio = 3.0;
#if DEBUG
    private const string BuildProfile = "debug";
#else
    private const string BuildProfile = "release";
#endif

    public static double RelativeLuminance(Color color)
    {
        static double Linearize(byte channel)
        {
            var value = channel / 255.0;
            return value <= 0.04045
                ? value / 12.92
                : Math.Pow((value + 0.055) / 1.055, 2.4);
        }

        return 0.2126 * Linearize(color.R)
            + 0.7152 * Linearize(color.G)
            + 0.0722 * Linearize(color.B);
    }

    public static double ContrastRatio(Color first, Color second)
    {
        var firstLuminance = RelativeLuminance(first);
        var secondLuminance = RelativeLuminance(second);
        var lighter = Math.Max(firstLuminance, secondLuminance);
        var darker = Math.Min(firstLuminance, secondLuminance);
        return (lighter + 0.05) / (darker + 0.05);
    }

    public static Color BestTextOn(Color background)
    {
        return ContrastRatio(Colors.Black, background) >= ContrastRatio(Colors.White, background)
            ? Colors.Black
            : Colors.White;
    }

    public static ThemeContrastEvidence AuditBuiltInThemes()
    {
        var variants = new List<(string Id, ThemeColorTokens Tokens)>
        {
            ("system-light", DictionaryTokens(isDark: false)),
            ("system-dark", DictionaryTokens(isDark: true)),
            ("light", DictionaryTokens(isDark: false)),
            ("dark", DictionaryTokens(isDark: true)),
        };
        variants.AddRange(ThemeCatalog.All
            .Where(palette => !palette.UseDictionaryOnly)
            .Select(palette => (
                palette.Id,
                ThemeApplication.BuildThreeColorTokens(
                    palette.IsDark,
                    palette.SwatchMain,
                    palette.SwatchSurface,
                    palette.SwatchBrand))));

        var checks = variants.SelectMany(variant => AuditVariant(variant.Id, variant.Tokens)).ToArray();
        var failures = checks.Where(check => check.Ratio + 0.0001 < check.RequiredRatio).ToArray();
        return new ThemeContrastEvidence(
            SchemaVersion: 1,
            Probe: "wcag_contrast",
            BuildProfile: BuildProfile,
            ThemeVariantCount: variants.Count,
            MinimumNormalTextRatio: MinimumRatio(checks, "normal_text"),
            MinimumLargeTextRatio: MinimumRatio(checks, "large_text"),
            MinimumNonTextRatio: MinimumRatio(checks, "non_text"),
            Checks: checks,
            Failures: failures);
    }

    public static void WriteEvidence(string path)
    {
        var evidence = AuditBuiltInThemes();
        var directory = Path.GetDirectoryName(Path.GetFullPath(path));
        if (!string.IsNullOrWhiteSpace(directory))
        {
            Directory.CreateDirectory(directory);
        }
        File.WriteAllText(path, JsonSerializer.Serialize(evidence, new JsonSerializerOptions
        {
            PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
            WriteIndented = true,
        }));
        if (evidence.Failures.Count > 0)
        {
            throw new InvalidOperationException($"WCAG contrast checks failed: {evidence.Failures.Count}");
        }
    }

    private static IEnumerable<ThemeContrastCheck> AuditVariant(string id, ThemeColorTokens tokens)
    {
        foreach (var background in new[]
                 {
                     ("background_surface", tokens.Surface),
                     ("background_main", tokens.Main),
                     ("editor_background", tokens.Editor),
                 })
        {
            yield return Check(id, "text_heading", tokens.TextHeading, background.Item1, background.Item2,
                NormalTextRatio, "normal_text");
            yield return Check(id, "text_primary", tokens.TextPrimary, background.Item1, background.Item2,
                NormalTextRatio, "normal_text");
            yield return Check(id, "text_secondary", tokens.TextSecondary, background.Item1, background.Item2,
                NormalTextRatio, "normal_text");
            yield return Check(id, "text_subtle", tokens.TextSubtle, background.Item1, background.Item2,
                LargeTextRatio, "large_text");
        }

        foreach (var accent in new[]
                 {
                     ("accent_primary", tokens.AccentPrimary),
                     ("accent_hover", tokens.AccentHover),
                     ("accent_pressed", tokens.AccentPressed),
                 })
        {
            yield return Check(id, "text_on_accent", tokens.TextOnAccent, accent.Item1, accent.Item2,
                NormalTextRatio, "normal_text");
            yield return Check(id, accent.Item1, accent.Item2, "background_surface", tokens.Surface,
                NonTextRatio, "non_text");
            yield return Check(id, accent.Item1, accent.Item2, "background_main", tokens.Main,
                NonTextRatio, "non_text");
        }
    }

    private static ThemeContrastCheck Check(
        string id,
        string foregroundToken,
        Color foreground,
        string backgroundToken,
        Color background,
        double required,
        string category) =>
        new(id, foregroundToken, backgroundToken, ContrastRatio(foreground, background), required, category);

    private static double MinimumRatio(IEnumerable<ThemeContrastCheck> checks, string category) =>
        checks.Where(check => check.Category == category).Min(check => check.Ratio);

    private static ThemeColorTokens DictionaryTokens(bool isDark)
    {
        if (isDark)
        {
            return new ThemeColorTokens(
                Parse("#1C1E21"), Parse("#17191C"), Parse("#212427"), Parse("#2A2E31"),
                Parse("#1D2023"), Parse("#17191C"), Parse("#212427"), Parse("#6FB9AD"),
                Parse("#82C6BB"), Parse("#5BA89C"), Parse("#07110F"), Parse("#ECEEF0"),
                Parse("#F4F6F8"), Parse("#B4BBC1"), Parse("#8A929A"), Parse("#F87171"),
                Parse("#FBBF24"), Parse("#60A5FA"));
        }

        return new ThemeColorTokens(
            Parse("#E4E9EF"), Parse("#EEF2F6"), Parse("#FFFFFF"), Parse("#FFFFFF"),
            Parse("#E8EDF2"), Parse("#E8EEF3"), Parse("#FAFBFC"), Parse("#2E726B"),
            Parse("#25605A"), Parse("#1F524D"), Parse("#FFFFFF"), Parse("#1B1F22"),
            Parse("#15181B"), Parse("#5B6469"), Parse("#7A848C"), Parse("#DC2626"),
            Parse("#D97706"), Parse("#2563EB"));
    }

    private static Color Parse(string value) => Color.Parse(value);
}
