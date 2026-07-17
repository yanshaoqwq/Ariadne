using Avalonia.Controls;
using Avalonia.Interactivity;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class TemplateMarketPageView : UserControl
{
    public TemplateMarketPageView()
    {
        InitializeComponent();
    }

    private async void OnLoaded(object? sender, RoutedEventArgs e)
    {
        if (DataContext is TemplateMarketPageViewModel viewModel)
        {
            await viewModel.EnsureInitialCatalogLoadedAsync().ConfigureAwait(true);
        }
    }
}
