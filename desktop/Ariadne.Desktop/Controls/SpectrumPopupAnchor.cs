namespace Ariadne.Desktop.Controls;

/// <summary>
/// 调色板弹出锚点：色块左下对齐调色板左下；空间不够则右下对齐。
/// 纯函数便于单测。
/// </summary>
public static class SpectrumPopupAnchor
{
    /// <returns>"bl" 左下对齐；"br" 右下对齐。</returns>
    public static string ChooseCorner(
        double swatchLeft,
        double swatchRight,
        double swatchBottom,
        double popupW,
        double popupH,
        double viewW,
        double viewH)
    {
        // 左下：popup 左上 = (swatchLeft, swatchBottom - popupH)
        var leftX = swatchLeft;
        var topY = swatchBottom - popupH;
        var leftFits = leftX >= -0.5
                       && leftX + popupW <= viewW + 0.5
                       && topY >= -0.5
                       && swatchBottom <= viewH + 0.5;

        if (leftFits)
        {
            return "bl";
        }

        // 右下：popup 右下 = (swatchRight, swatchBottom)
        var rightX = swatchRight - popupW;
        var rightTopY = swatchBottom - popupH;
        var rightFits = rightX >= -0.5
                        && swatchRight <= viewW + 0.5
                        && rightTopY >= -0.5;

        return rightFits || !leftFits ? "br" : "bl";
    }
}
