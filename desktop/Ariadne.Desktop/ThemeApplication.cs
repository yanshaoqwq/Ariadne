using Avalonia;
using Avalonia.Styling;

namespace Ariadne.Desktop;

public static class ThemeApplication
{
    public static void Apply(string? theme)
    {
        if (Application.Current is null)
        {
            return;
        }

        Application.Current.RequestedThemeVariant = theme?.Trim().ToLowerInvariant() switch
        {
            "light" => ThemeVariant.Light,
            "dark" => ThemeVariant.Dark,
            _ => ThemeVariant.Default,
        };
    }
}
