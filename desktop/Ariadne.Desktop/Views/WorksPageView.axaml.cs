using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Input.Platform;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class WorksPageView : UserControl
{
    public WorksPageView()
    {
        InitializeComponent();
        DataContextChanged += (_, _) => AttachEditorActions();
        AttachEditorActions();
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

    private void AttachEditorActions()
    {
        if (DataContext is not WorksPageViewModel viewModel)
        {
            return;
        }

        viewModel.RequestEditorCopy = () => _ = CopySelectionAsync();
        viewModel.RequestEditorSelectAll = () =>
        {
            DocumentEditor.SelectionStart = 0;
            DocumentEditor.SelectionEnd = DocumentEditor.Text?.Length ?? 0;
            DocumentEditor.Focus();
        };
    }

    private async Task CopySelectionAsync()
    {
        var selectedText = DocumentEditor.SelectedText;
        if (string.IsNullOrEmpty(selectedText))
        {
            selectedText = DocumentEditor.Text ?? string.Empty;
        }
        var clipboard = TopLevel.GetTopLevel(this)?.Clipboard;
        if (clipboard is not null)
        {
            await clipboard.SetTextAsync(selectedText);
        }
    }
}
