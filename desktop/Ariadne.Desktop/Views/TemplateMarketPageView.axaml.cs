using Avalonia.Controls;
using Avalonia.Input;
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

    private void OnSearchKeyDown(object? sender, KeyEventArgs e)
    {
        if (e.Key == Key.Enter
            && DataContext is TemplateMarketPageViewModel viewModel
            && viewModel.SearchCommand.TryExecute())
        {
            e.Handled = true;
        }
    }
}
