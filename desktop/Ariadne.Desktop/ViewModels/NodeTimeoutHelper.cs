using System.Globalization;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 节点超时：UI 用秒、存储/后端用 ms。统一解析，避免文化格式与双倍换算错误。
/// </summary>
public static class NodeTimeoutHelper
{
    /// <summary>ms 文本 → 秒数字符串（作者向）。</summary>
    public static string FormatSecondsFromMs(string? timeoutMs)
    {
        if (string.IsNullOrWhiteSpace(timeoutMs))
        {
            return string.Empty;
        }

        var text = timeoutMs.Trim();
        if (long.TryParse(text, NumberStyles.Integer, CultureInfo.InvariantCulture, out var ms)
            && ms >= 0)
        {
            return (ms / 1000.0).ToString("0.###", CultureInfo.InvariantCulture);
        }

        // 非数字原样返回，避免静默清空用户输入
        return text;
    }

    /// <summary>秒数字符串 → ms 文本（写回 TimeoutMs / 图数据）。</summary>
    public static string ParseSecondsToMs(string? secondsText)
    {
        if (string.IsNullOrWhiteSpace(secondsText))
        {
            return string.Empty;
        }

        var text = secondsText.Trim();
        if (TryParseNonNegativeDouble(text, out var seconds))
        {
            return ((long)Math.Round(seconds * 1000.0)).ToString(CultureInfo.InvariantCulture);
        }

        return text;
    }

    public static double? ParseNullableDouble(string? text)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return null;
        }

        return TryParseNonNegativeDouble(text.Trim(), out var value) ? value : null;
    }

    /// <summary>解析已是 ms 的整数字符串（节点 TimeoutMs 字段）。</summary>
    public static long? ParseNullableLongMs(string? text)
    {
        if (string.IsNullOrWhiteSpace(text))
        {
            return null;
        }

        if (long.TryParse(text.Trim(), NumberStyles.Integer, CultureInfo.InvariantCulture, out var value)
            && value >= 0)
        {
            return value;
        }

        return null;
    }

    private static bool TryParseNonNegativeDouble(string text, out double value)
    {
        if (double.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out value)
            && value >= 0
            && double.IsFinite(value))
        {
            return true;
        }

        if (double.TryParse(text, NumberStyles.Float, CultureInfo.CurrentCulture, out value)
            && value >= 0
            && double.IsFinite(value))
        {
            return true;
        }

        value = 0;
        return false;
    }
}
