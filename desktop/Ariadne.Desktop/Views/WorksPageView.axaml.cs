using Avalonia.Controls;
using Avalonia.Input;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class WorksPageView : UserControl
{
    public WorksPageView()
    {
        InitializeComponent();
    }

    private void OnDocumentEditorKeyDown(object? sender, KeyEventArgs e)
    {
        var hasCommandModifier = e.KeyModifiers.HasFlag(KeyModifiers.Control)
                                 || e.KeyModifiers.HasFlag(KeyModifiers.Meta);
        if (!hasCommandModifier || e.Key != Key.K || DataContext is not WorksPageViewModel viewModel)
        {
            return;
        }

        viewModel.QuickAiCommand.Execute(null);
        e.Handled = true;
    }
}
