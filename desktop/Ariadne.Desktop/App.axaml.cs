using Avalonia;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Markup.Xaml;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;

namespace Ariadne.Desktop;

public partial class App : Application
{
    public override void Initialize()
    {
        AvaloniaXamlLoader.Load(this);
    }

    public override void OnFrameworkInitializationCompleted()
    {
        DisplayNameService.Initialize(DisplayNameService.LoadDefault());
        DialogService.Initialize(DisplayNameService.Current);

        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime desktop)
        {
            if (ReleaseUiProbe.TryStart(desktop))
            {
                base.OnFrameworkInitializationCompleted();
                return;
            }

            var backend = JsonLineBackendClient.CreateDefault();
            var viewModel = new MainWindowViewModel(DisplayNameService.Current, backend);
            desktop.Exit += (_, _) => backend.Dispose();
            desktop.MainWindow = new MainWindow
            {
                DataContext = viewModel,
            };
            _ = viewModel.InitializeAsync();
        }

        base.OnFrameworkInitializationCompleted();
    }
}
