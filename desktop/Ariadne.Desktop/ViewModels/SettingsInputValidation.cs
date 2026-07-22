using System.Globalization;
using Ariadne.Desktop.Backend;

namespace Ariadne.Desktop.ViewModels;

internal enum SettingsInputFailure
{
    Number,
    Positive,
    NonNegative,
    ModelLine,
    PathLine,
    Required,
}

internal sealed class SettingsInputException : Exception
{
    public SettingsInputException(SettingsInputFailure failure, string fieldKey, int? line = null)
    {
        Failure = failure;
        FieldKey = fieldKey;
        Line = line;
    }

    public SettingsInputFailure Failure { get; }
    public string FieldKey { get; }
    public int? Line { get; }
}

internal static class SettingsInputValidation
{
    private static readonly HashSet<string> ModelCapabilities = new(StringComparer.Ordinal)
    {
        "llm", "embedding", "reranker", "search",
    };

    internal static StringComparer PathComparer { get; } =
        OperatingSystem.IsWindows() ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal;

    public static int PositiveInt(string? text, string fieldKey)
    {
        var value = RequiredLong(text, fieldKey);
        if (value <= 0 || value > int.MaxValue)
        {
            throw new SettingsInputException(SettingsInputFailure.Positive, fieldKey);
        }
        return (int)value;
    }

    public static int NonNegativeInt(string? text, string fieldKey)
    {
        var value = RequiredLong(text, fieldKey);
        if (value < 0 || value > int.MaxValue)
        {
            throw new SettingsInputException(SettingsInputFailure.NonNegative, fieldKey);
        }
        return (int)value;
    }

    public static long PositiveLong(string? text, string fieldKey)
    {
        var value = RequiredLong(text, fieldKey);
        if (value <= 0)
        {
            throw new SettingsInputException(SettingsInputFailure.Positive, fieldKey);
        }
        return value;
    }

    public static double NonNegativeDouble(string? text, string fieldKey)
    {
        var value = RequiredDouble(text, fieldKey);
        if (value < 0)
        {
            throw new SettingsInputException(SettingsInputFailure.NonNegative, fieldKey);
        }
        return value;
    }

    public static IReadOnlyList<ModelConfig> Models(string? text, string fieldKey)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return Array.Empty<ModelConfig>();
        }

        var result = new List<ModelConfig>();
        var modelIds = new HashSet<string>(StringComparer.Ordinal);
        var lines = text.Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n');
        for (var index = 0; index < lines.Length; index++)
        {
            var raw = lines[index];
            if (string.IsNullOrWhiteSpace(raw))
            {
                continue;
            }

            var parts = raw.Split(',', StringSplitOptions.TrimEntries);
            if (parts.Length is < 2 or > 5
                || string.IsNullOrWhiteSpace(parts[0])
                || string.IsNullOrWhiteSpace(parts[1])
                || !ModelCapabilities.Contains(parts[1])
                || !modelIds.Add(parts[0]))
            {
                throw new SettingsInputException(SettingsInputFailure.ModelLine, fieldKey, index + 1);
            }

            int? context = null;
            double? input = null;
            double? output = null;
            if (parts.Length > 2 && !string.IsNullOrWhiteSpace(parts[2]))
            {
                context = ParseModelPositiveInt(parts[2], fieldKey, index + 1);
            }
            if (parts.Length > 3 && !string.IsNullOrWhiteSpace(parts[3]))
            {
                input = ParseModelNonNegativeDouble(parts[3], fieldKey, index + 1);
            }
            if (parts.Length > 4 && !string.IsNullOrWhiteSpace(parts[4]))
            {
                output = ParseModelNonNegativeDouble(parts[4], fieldKey, index + 1);
            }

            result.Add(new ModelConfig(parts[0], parts[1], context, input, output));
        }
        return result;
    }

    public static IReadOnlyList<string> AbsolutePaths(string? text, string fieldKey)
    {
        return Paths(text, fieldKey, requireAbsolute: true);
    }

    public static IReadOnlyList<string> RelativePaths(string? text, string fieldKey)
    {
        return Paths(text, fieldKey, requireAbsolute: false);
    }

    private static IReadOnlyList<string> Paths(string? text, string fieldKey, bool requireAbsolute)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return Array.Empty<string>();
        }

        var result = new List<string>();
        var unique = new HashSet<string>(PathComparer);
        var lines = text.Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n');
        for (var index = 0; index < lines.Length; index++)
        {
            var raw = lines[index].Trim();
            if (string.IsNullOrWhiteSpace(raw))
            {
                continue;
            }

            string normalized;
            if (requireAbsolute)
            {
                if (!Path.IsPathFullyQualified(raw)
                    || raw.Split(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar)
                        .Any(component => component == ".."))
                {
                    throw new SettingsInputException(SettingsInputFailure.PathLine, fieldKey, index + 1);
                }
                normalized = Path.GetFullPath(raw);
            }
            else
            {
                normalized = raw.Replace('\\', '/').TrimEnd('/');
                if (Path.IsPathFullyQualified(normalized)
                    || normalized.StartsWith("/", StringComparison.Ordinal)
                    || normalized.Length == 0
                    || normalized.Contains(':')
                    || normalized.Split('/').Any(component =>
                        string.IsNullOrEmpty(component) || component is "." or ".."))
                {
                    throw new SettingsInputException(SettingsInputFailure.PathLine, fieldKey, index + 1);
                }
            }

            if (!unique.Add(normalized))
            {
                throw new SettingsInputException(SettingsInputFailure.PathLine, fieldKey, index + 1);
            }
            result.Add(normalized);
        }
        return result;
    }

    private static long RequiredLong(string? text, string fieldKey)
    {
        var trimmed = text?.Trim();
        if (!long.TryParse(trimmed, NumberStyles.Integer, CultureInfo.InvariantCulture, out var value)
            && !long.TryParse(trimmed, NumberStyles.Integer, CultureInfo.CurrentCulture, out value))
        {
            throw new SettingsInputException(SettingsInputFailure.Number, fieldKey);
        }
        return value;
    }

    private static double RequiredDouble(string? text, string fieldKey)
    {
        var trimmed = text?.Trim().TrimStart('$');
        if ((!double.TryParse(trimmed, NumberStyles.Float, CultureInfo.InvariantCulture, out var value)
             && !double.TryParse(trimmed, NumberStyles.Float, CultureInfo.CurrentCulture, out value))
            || !double.IsFinite(value))
        {
            throw new SettingsInputException(SettingsInputFailure.Number, fieldKey);
        }
        return value;
    }

    private static int ParseModelPositiveInt(string text, string fieldKey, int line)
    {
        if (!int.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out var value)
            || value <= 0)
        {
            throw new SettingsInputException(SettingsInputFailure.ModelLine, fieldKey, line);
        }
        return value;
    }

    private static double ParseModelNonNegativeDouble(string text, string fieldKey, int line)
    {
        if (!double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out var value)
            || !double.IsFinite(value)
            || value < 0)
        {
            throw new SettingsInputException(SettingsInputFailure.ModelLine, fieldKey, line);
        }
        return value;
    }
}
