namespace Ariadne.Desktop;

/// <summary>
/// 图标母版重着色纯逻辑：纸面 vs 墨线判定 + 映射到 Accent/纸面色。
/// 供 <see cref="AppIconPainter"/> 与单元测试共用。
/// </summary>
public static class AppIconRecolor
{
    /// <summary>是否视为暖纸/浅灰背景（线描图标空白）。透明像素不当纸面。</summary>
    public static bool IsPaperPixel(byte r, byte g, byte b, byte alpha = 255)
    {
        if (alpha < 8)
        {
            return false;
        }

        // 已透明底母版：近白且几乎不透明才当纸；半透明描边不算纸
        if (alpha < 200)
        {
            return false;
        }

        return r >= 200 && g >= 200 && b >= 200
               && Math.Abs(r - g) < 32
               && Math.Abs(g - b) < 32;
    }

    /// <summary>
    /// 将母版像素映射到 (R,G,B,A)。墨线 → accent（保留亮度），纸面 → paper。
    /// <paramref name="paperA"/> 为 0 时纸面变完全透明（任务栏无底板）。
    /// </summary>
    public static (byte R, byte G, byte B, byte A) MapPixel(
        byte r, byte g, byte b, byte a,
        byte accentR, byte accentG, byte accentB,
        byte paperR, byte paperG, byte paperB,
        byte paperA = 255)
    {
        if (a < 8)
        {
            return (0, 0, 0, 0);
        }

        if (IsPaperPixel(r, g, b, a))
        {
            // 任务栏：纸面请求透明 → 输出 alpha=0
            if (paperA < 8)
            {
                return (0, 0, 0, 0);
            }

            return (paperR, paperG, paperB, a);
        }

        // 实心剪影：暗部尽量整块落到 Accent（小尺寸任务栏更清晰）
        var lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
        var mix = Math.Clamp(1.08 - lum * 0.55, 0.55, 1.0);

        // 透明纸面：不混入纸色，纯 Accent 填实心
        if (paperA < 8)
        {
            var ir = (byte)Math.Clamp(accentR * mix, 0, 255);
            var ig = (byte)Math.Clamp(accentG * mix, 0, 255);
            var ib = (byte)Math.Clamp(accentB * mix, 0, 255);
            return (ir, ig, ib, a);
        }

        var nr = (byte)Math.Clamp(accentR * mix + paperR * (1 - mix) * 0.08, 0, 255);
        var ng = (byte)Math.Clamp(accentG * mix + paperG * (1 - mix) * 0.08, 0, 255);
        var nb = (byte)Math.Clamp(accentB * mix + paperB * (1 - mix) * 0.08, 0, 255);
        return (nr, ng, nb, a);
    }
}
