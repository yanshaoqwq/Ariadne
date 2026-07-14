using Avalonia;
using Avalonia.Fonts.Inter;

namespace Ariadne.Desktop;

internal static class Program
{
    [STAThread]
    public static int Main(string[] args)
    {
        if (args is ["--verify-installation"])
        {
            if (!ReleaseLayoutValidator.TryValidate(AppContext.BaseDirectory, out var error))
            {
                Console.Error.WriteLine(error);
                return 1;
            }

            Console.WriteLine("Ariadne release layout is valid.");
            return 0;
        }

        BuildAvaloniaApp()
            .StartWithClassicDesktopLifetime(args);
        return 0;
    }

    public static AppBuilder BuildAvaloniaApp()
    {
        return AppBuilder.Configure<App>()
            .UsePlatformDetect()
            .WithInterFont()
            .LogToTrace();
    }
}
