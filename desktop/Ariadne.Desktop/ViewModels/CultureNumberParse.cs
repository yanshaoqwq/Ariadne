using System.Globalization;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 设置/预算等表单数字解析：优先 InvariantCulture，再回退当前文化，避免 "1.5" / "1,5" 静默变 0。
/// </summary>
public static class CultureNumberParse
{
    public static double ParseDouble(string? text, double fallback)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return fallback;
        }

        var trimmed = text.Trim().TrimStart('$');
        if (double.TryParse(trimmed, NumberStyles.Float, CultureInfo.InvariantCulture, out var value)
            && double.IsFinite(value))
        {
            return value;
        }

        if (double.TryParse(trimmed, NumberStyles.Float, CultureInfo.CurrentCulture, out value)
            && double.IsFinite(value))
        {
            return value;
        }

        return fallback;
    }

    public static long ParseLong(string? text, long fallback)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return fallback;
        }

        var trimmed = text.Trim();
        if (long.TryParse(trimmed, NumberStyles.Integer, CultureInfo.InvariantCulture, out var value))
        {
            return value;
        }

        if (long.TryParse(trimmed, NumberStyles.Integer, CultureInfo.CurrentCulture, out value))
        {
            return value;
        }

        // "300000.0" 等
        if (double.TryParse(trimmed, NumberStyles.Float, CultureInfo.InvariantCulture, out var d)
            && double.IsFinite(d)
            && d >= long.MinValue
            && d <= long.MaxValue)
        {
            return (long)Math.Round(d);
        }

        return fallback;
    }

    public static int ParseInt(string? text, int fallback)
    {
        var asLong = ParseLong(text, fallback);
        if (asLong < int.MinValue || asLong > int.MaxValue)
        {
            return fallback;
        }

        return (int)asLong;
    }
}
