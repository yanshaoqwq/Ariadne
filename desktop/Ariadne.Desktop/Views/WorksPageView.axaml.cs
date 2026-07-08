using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Input.Platform;
using Avalonia.Interactivity;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class WorksPageView : UserControl
{
    private WorksPageViewModel? _attachedViewModel;
    private TextBox? _activeBlockEditor;
    private DocumentBlockViewModel? _activeBlock;

    public WorksPageView()
    {
        InitializeComponent();
        DataContextChanged += (_, _) => AttachEditorActions();
        AttachEditorActions();
    }

    private void OnDocumentBlockEditorKeyDown(object? sender, KeyEventArgs e)
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

    private void OnDocumentBlockEditorGotFocus(object? sender, RoutedEventArgs e)
    {
        _activeBlockEditor = sender as TextBox;
        _activeBlock = _activeBlockEditor?.DataContext as DocumentBlockViewModel;
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
            if (_activeBlockEditor is not null)
            {
                _activeBlockEditor.SelectionStart = 0;
                _activeBlockEditor.SelectionEnd = _activeBlockEditor.Text?.Length ?? 0;
                _activeBlockEditor.Focus();
            }
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
        if (DataContext is WorksPageViewModel viewModel
            && _activeBlockEditor is not null
            && _activeBlock is not null)
        {
            var start = Math.Min(_activeBlockEditor.SelectionStart, _activeBlockEditor.SelectionEnd);
            var end = Math.Max(_activeBlockEditor.SelectionStart, _activeBlockEditor.SelectionEnd);
            return viewModel.SelectionForBlock(_activeBlock, start, end, _activeBlockEditor.SelectedText ?? string.Empty);
        }
        return new EditorTextSelection(0, 0, string.Empty);
    }

    private async Task CopySelectionAsync()
    {
        var selectedText = _activeBlockEditor?.SelectedText;
        if (string.IsNullOrEmpty(selectedText))
        {
            selectedText = DataContext is WorksPageViewModel viewModel
                ? viewModel.DocumentContent
                : string.Empty;
        }
        var clipboard = TopLevel.GetTopLevel(this)?.Clipboard;
        if (clipboard is not null)
        {
            await clipboard.SetTextAsync(selectedText);
        }
    }
}
