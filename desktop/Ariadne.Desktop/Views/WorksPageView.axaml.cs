using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Input.Platform;
using Avalonia.Interactivity;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using Avalonia.VisualTree;
using AvaloniaEdit;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class WorksPageView : UserControl
{
    private WorksPageViewModel? _attachedViewModel;
    /// <summary>
    /// 焦点移到项目 AI 输入框/发送按钮时，TextBox 选区可能被清空。
    /// 在 LostFocus 时固化最后一次非空选区，保证「选中 → 告诉 AI」仍可用。
    /// </summary>
    private EditorTextSelection? _stickySelection;

    public WorksPageView()
    {
        InitializeComponent();
        DocumentEditor.TextArea.SelectionChanged += OnDocumentEditorSelectionChanged;
        DocumentEditor.TextArea.Caret.PositionChanged += OnDocumentEditorCaretPositionChanged;
        DataContextChanged += (_, _) => AttachEditorActions();
        AttachEditorActions();
    }

    protected override void OnAttachedToVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnAttachedToVisualTree(e);
        AttachEditorActions();
    }

    private void OnDocumentEditorKeyDown(object? sender, KeyEventArgs e)
    {
        HandleKeyboardShortcut(sender, e);
    }

    private void OnWorksPageKeyDown(object? sender, KeyEventArgs e)
    {
        HandleKeyboardShortcut(sender, e);
    }

    private void HandleKeyboardShortcut(object? sender, KeyEventArgs e)
    {
        var hasCommandModifier = e.KeyModifiers.HasFlag(KeyModifiers.Control)
                                 || e.KeyModifiers.HasFlag(KeyModifiers.Meta);
        if (!hasCommandModifier || DataContext is not WorksPageViewModel viewModel)
        {
            return;
        }

        if (e.Key == Key.K)
        {
            CaptureStickySelection(clearWhenEmpty: false);
            e.Handled = viewModel.OpenQuickEditCommand.TryExecute();
            return;
        }

        if (e.Key == Key.S)
        {
            e.Handled = viewModel.SaveCommand.TryExecute();
        }
    }

    private void OnDocumentEditorKeyUp(object? sender, KeyEventArgs e)
    {
        CaptureStickySelection(clearWhenEmpty: false);
    }

    private void OnDocumentEditorGotFocus(object? sender, RoutedEventArgs e)
    {
        CaptureStickySelection(clearWhenEmpty: false);
    }

    private void OnDocumentEditorLostFocus(object? sender, RoutedEventArgs e)
    {
        CaptureStickySelection(clearWhenEmpty: false);
    }

    private void OnDocumentEditorPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        CaptureStickySelection(clearWhenEmpty: true);
    }

    private void OnDocumentEditorSelectionChanged(object? sender, EventArgs e)
    {
        CaptureStickySelection(clearWhenEmpty: DocumentEditor.IsKeyboardFocusWithin);
    }

    private void OnDocumentEditorCaretPositionChanged(object? sender, EventArgs e)
    {
        CaptureStickySelection(clearWhenEmpty: false);
    }

    private void AttachEditorActions()
    {
        if (_attachedViewModel is not null && !ReferenceEquals(_attachedViewModel, DataContext))
        {
            _attachedViewModel.RequestEditorCopy = null;
            _attachedViewModel.RequestEditorSelectAll = null;
            _attachedViewModel.RequestEditorSelection = null;
            _attachedViewModel.RequestRevealEditorRange = null;
            _attachedViewModel.ClearStickyEditorSelection = null;
            _attachedViewModel.RequestFocusQuickEditInstruction = null;
            _attachedViewModel.PickImportSourceFile = null;
            _attachedViewModel.OpenFolderInShell = null;
            _attachedViewModel = null;
        }

        if (DataContext is not WorksPageViewModel viewModel)
        {
            return;
        }

        viewModel.RequestEditorCopy = () => _ = CopySelectionAsync();
        viewModel.RequestEditorSelectAll = () =>
        {
            if (viewModel.IsEditMode && DocumentEditor.Document is not null)
            {
                DocumentEditor.SelectAll();
                DocumentEditor.Focus();
                CaptureStickySelection(clearWhenEmpty: false);
            }
        };
        viewModel.RequestEditorSelection = CurrentEditorSelection;
        viewModel.RequestRevealEditorRange = RevealEditorRange;
        viewModel.ClearStickyEditorSelection = ClearStickySelectionState;
        viewModel.RequestFocusQuickEditInstruction = FocusQuickEditInstruction;
        viewModel.PickImportSourceFile = PickImportSourceFileAsync;
        viewModel.OpenFolderInShell = OpenFolderInShellAsync;
        _attachedViewModel = viewModel;
    }

    private async Task OpenFolderInShellAsync(string directoryPath)
    {
        var topLevel = TopLevel.GetTopLevel(this);
        if (topLevel is null || string.IsNullOrWhiteSpace(directoryPath))
        {
            return;
        }

        var folder = await topLevel.StorageProvider.TryGetFolderFromPathAsync(directoryPath);
        if (folder is not null)
        {
            await topLevel.Launcher.LaunchFileAsync(folder);
            return;
        }

        // 回退：用 file URI 打开目录
        var uri = new Uri(Path.GetFullPath(directoryPath) + Path.DirectorySeparatorChar);
        await topLevel.Launcher.LaunchUriAsync(uri);
    }

    private async Task<string?> PickImportSourceFileAsync()
    {
        var topLevel = TopLevel.GetTopLevel(this);
        if (topLevel is null)
        {
            return null;
        }

        var files = await topLevel.StorageProvider.OpenFilePickerAsync(new FilePickerOpenOptions
        {
            Title = (DataContext as WorksPageViewModel)?.ImportSourcePathText,
            AllowMultiple = false,
            FileTypeFilter = new[]
            {
                new FilePickerFileType(Ariadne.Desktop.Localization.DisplayNameService.Current.Text("ui.file_type.markdown_text"))
                {
                    Patterns = new[] { "*.md", "*.markdown", "*.txt" },
                },
                new FilePickerFileType(Ariadne.Desktop.Localization.DisplayNameService.Current.Text("ui.file_type.all"))
                {
                    Patterns = new[] { "*.*" },
                },
            },
        });
        return files.FirstOrDefault()?.Path.LocalPath;
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        if (_attachedViewModel is not null)
        {
            _attachedViewModel.RequestEditorCopy = null;
            _attachedViewModel.RequestEditorSelectAll = null;
            _attachedViewModel.RequestEditorSelection = null;
            _attachedViewModel.RequestRevealEditorRange = null;
            _attachedViewModel.ClearStickyEditorSelection = null;
            _attachedViewModel.RequestFocusQuickEditInstruction = null;
            _attachedViewModel.PickImportSourceFile = null;
            _attachedViewModel.OpenFolderInShell = null;
            _attachedViewModel = null;
        }

        base.OnDetachedFromVisualTree(e);
    }

    private void FocusQuickEditInstruction()
    {
        Dispatcher.UIThread.Post(() =>
        {
            QuickEditInstructionBox.Focus();
            QuickEditInstructionBox.SelectionStart = QuickEditInstructionBox.Text?.Length ?? 0;
            QuickEditInstructionBox.SelectionEnd = QuickEditInstructionBox.SelectionStart;
        }, DispatcherPriority.Input);
    }

    private void RevealEditorRange(int globalStart, int globalEnd)
    {
        if (DataContext is not WorksPageViewModel viewModel
            || DocumentEditor.Document is null
            || globalStart < 0
            || globalEnd <= globalStart
            || globalEnd > DocumentEditor.Document.TextLength)
        {
            return;
        }

        viewModel.IsEditMode = true;
        Dispatcher.UIThread.Post(() =>
        {
            DocumentEditor.Select(globalStart, globalEnd - globalStart);
            var line = DocumentEditor.Document.GetLineByOffset(globalStart).LineNumber;
            DocumentEditor.ScrollToLine(line);
            DocumentEditor.Focus();
            CaptureStickySelection(clearWhenEmpty: false);
        }, DispatcherPriority.Loaded);
    }

    private void ClearStickySelectionState()
    {
        _stickySelection = null;
        if (DocumentEditor.Document is not null)
        {
            DocumentEditor.Select(0, 0);
            DocumentEditor.CaretOffset = 0;
            Dispatcher.UIThread.Post(() => DocumentEditor.ScrollToLine(1), DispatcherPriority.Loaded);
        }
    }

    private EditorTextSelection CurrentEditorSelection()
    {
        // 焦点移到 AI composer 后保留最后一次非空全局选区。
        CaptureStickySelection(clearWhenEmpty: false);
        if (_stickySelection is { } sticky
            && sticky.End > sticky.Start
            && !string.IsNullOrWhiteSpace(sticky.Text))
        {
            return sticky;
        }

        return new EditorTextSelection(0, 0, string.Empty);
    }

    private void CaptureStickySelection(bool clearWhenEmpty)
    {
        if (DataContext is not WorksPageViewModel viewModel
            || DocumentEditor.Document is null)
        {
            return;
        }

        var start = DocumentEditor.SelectionStart;
        var end = start + DocumentEditor.SelectionLength;
        var selected = DocumentEditor.SelectedText ?? string.Empty;
        var mapped = new EditorTextSelection(start, end, selected);
        viewModel.UpdateSummarySelectionFromEditor(mapped);

        if (end > start && !string.IsNullOrWhiteSpace(selected))
        {
            _stickySelection = EditorStickySelectionPolicy.Update(
                _stickySelection,
                mapped.Start,
                mapped.End,
                mapped.Text,
                clearWhenEmpty: false);
            return;
        }

        // Empty caret: only clear when caller says intentional deselect (focused PointerReleased).
        _stickySelection = EditorStickySelectionPolicy.Update(
            _stickySelection,
            mapped.Start,
            mapped.End,
            mapped.Text,
            clearWhenEmpty);
    }

    private async Task CopySelectionAsync()
    {
        try
        {
            if (DataContext is WorksPageViewModel { IsEditMode: true }
                && DocumentEditor.SelectionLength > 0)
            {
                DocumentEditor.Copy();
                return;
            }

            var selectedText = DataContext is WorksPageViewModel viewModel
                ? viewModel.DocumentContent
                : string.Empty;
            if (string.IsNullOrEmpty(selectedText))
            {
                return;
            }
            var clipboard = TopLevel.GetTopLevel(this)?.Clipboard;
            if (clipboard is not null)
            {
                await clipboard.SetTextAsync(selectedText);
            }
        }
        catch (Exception ex)
        {
            if (DataContext is WorksPageViewModel viewModel)
            {
                viewModel.StatusText = UserFacingError.Format(
                    ex,
                    Ariadne.Desktop.Localization.DisplayNameService.Current);
            }
        }
    }
}
