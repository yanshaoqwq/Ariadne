using Avalonia.Media;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class ThemeAccessibilityTests
{
    [Fact]
    public void BuiltInThemeVariants_MeetWcagContrastMatrix()
    {
        var evidence = ThemeAccessibilityAudit.AuditBuiltInThemes();

        Assert.Equal(12, evidence.ThemeVariantCount);
        Assert.Empty(evidence.Failures);
        Assert.True(evidence.MinimumNormalTextRatio >= 4.5);
        Assert.True(evidence.MinimumLargeTextRatio >= 3.0);
        Assert.True(evidence.MinimumNonTextRatio >= 3.0);
    }

    [Theory]
    [InlineData("#FFFFFF")]
    [InlineData("#000000")]
    [InlineData("#777777")]
    [InlineData("#B4690E")]
    public void BestTextOn_AlwaysSelectsAccessibleForeground(string backgroundHex)
    {
        var background = Color.Parse(backgroundHex);
        var foreground = ThemeAccessibilityAudit.BestTextOn(background);

        Assert.True(ThemeAccessibilityAudit.ContrastRatio(foreground, background) >= 4.5);
    }
}
