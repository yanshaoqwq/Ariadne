using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Input.Platform;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class WorksPageView : UserControl
{
    private WorksPageViewModel? _attachedViewModel;

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
        if (_attachedViewModel is not null && !ReferenceEquals(_attachedViewModel, DataContext))
        {
            _attachedViewModel.RequestEditorCopy = null;
            _attachedViewModel.RequestEditorSelectAll = null;
            _attachedViewModel.RequestEditorSelection = null;
            _attachedViewModel = null;
        }

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
        viewModel.RequestEditorSelection = CurrentEditorSelection;
        _attachedViewModel = viewModel;
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        if (_attachedViewModel is not null)
        {
            _attachedViewModel.RequestEditorCopy = null;
            _attachedViewModel.RequestEditorSelectAll = null;
            _attachedViewModel.RequestEditorSelection = null;
            _attachedViewModel = null;
        }

        base.OnDetachedFromVisualTree(e);
    }

    private EditorTextSelection CurrentEditorSelection()
    {
        var start = Math.Min(DocumentEditor.SelectionStart, DocumentEditor.SelectionEnd);
        var end = Math.Max(DocumentEditor.SelectionStart, DocumentEditor.SelectionEnd);
        return new EditorTextSelection(start, end, DocumentEditor.SelectedText ?? string.Empty);
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
