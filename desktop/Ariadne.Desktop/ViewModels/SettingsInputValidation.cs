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

    public object? FocusItem { get; private set; }

    public SettingsInputException WithFocusItem(object item)
    {
        FocusItem = item;
        return this;
    }
}

internal static class SettingsInputValidation
{
    private static readonly HashSet<string> ModelCapabilities = new(StringComparer.Ordinal)
    {
        "llm", "embedding", "reranker", "search", "tool_use",
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

    /// <summary>
    /// U112：空输入表示「未设置」（不限制），返回 null；填了值则按非负金额校验。
    ///
    /// 预授权额度必须区分这两种状态：显式的 <c>0</c> 是零额度（暂停一切有成本调用），
    /// 「未设置」才是不限制。若把空值折叠成 <c>0</c>，用户只要打开设置页保存一次
    /// 无关改动，就会把「不限制」静默翻转成「全部暂停」。
    /// </summary>
    public static double? OptionalNonNegativeDouble(string? text, string fieldKey)
    {
        return string.IsNullOrWhiteSpace(text) ? null : NonNegativeDouble(text, fieldKey);
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

    /// <summary>
    /// U146：单条路径的规范化判定，不抛异常。
    ///
    /// chip 列表要在「加入时」就告诉用户这一条行不行，而整段 <see cref="Paths"/> 只能
    /// 整体成功或整体抛错。两者若各写一套规则必然漂移——那会出现「chip 显示正常、
    /// 保存却被拒」这种最难查的分裂，所以 <see cref="Paths"/> 也改成逐条调用本方法，
    /// 规则只有这一份。
    ///
    /// 首尾空白在这里就吃掉：<c>" /home/x"</c> 与 <c>"/home/x"</c> 在文本框里长得一样，
    /// 但作为路径是两个值，靠肉眼永远看不出来（U146 的三条后果之一）。
    /// </summary>
    public static bool TryNormalizePath(string? raw, bool requireAbsolute, out string normalized)
    {
        normalized = string.Empty;
        var trimmed = (raw ?? string.Empty).Trim();
        if (string.IsNullOrWhiteSpace(trimmed))
        {
            return false;
        }

        try
        {
            if (requireAbsolute)
            {
                if (!Path.IsPathFullyQualified(trimmed)
                    || trimmed.Split(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar)
                        .Any(component => component == ".."))
                {
                    return false;
                }
                normalized = Path.GetFullPath(trimmed);
                return true;
            }

            var relative = trimmed.Replace('\\', '/').TrimEnd('/');
            if (Path.IsPathFullyQualified(relative)
                || relative.StartsWith("/", StringComparison.Ordinal)
                || relative.Length == 0
                || relative.Contains(':')
                || relative.Split('/').Any(component =>
                    string.IsNullOrEmpty(component) || component is "." or ".."))
            {
                return false;
            }
            normalized = relative;
            return true;
        }
        catch (Exception ex) when (ex is ArgumentException or PathTooLongException or NotSupportedException)
        {
            // Path.GetFullPath 对含非法字符/超长的输入会抛；这类输入就是「这条不合法」，
            // 不该让整个设置页崩在校验上。
            return false;
        }
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
            var raw = lines[index];
            if (string.IsNullOrWhiteSpace(raw))
            {
                continue;
            }

            // 规则收口到 TryNormalizePath：chip 的逐条判定与这里的保存前校验共用同一份，
            // 否则「加得进去、存不下去」。
            if (!TryNormalizePath(raw, requireAbsolute, out var normalized))
            {
                throw new SettingsInputException(SettingsInputFailure.PathLine, fieldKey, index + 1);
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
