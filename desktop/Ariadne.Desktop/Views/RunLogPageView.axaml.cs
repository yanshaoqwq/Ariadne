using Avalonia.Controls;
using Avalonia.Input;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class RunLogPageView : UserControl
{
    public RunLogPageView()
    {
        InitializeComponent();
    }

    private void OnSearchKeyDown(object? sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter || DataContext is not RunLogPageViewModel viewModel)
        {
            return;
        }

        viewModel.SearchCommand.Execute(null);
        e.Handled = true;
    }
}
