using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Input.Platform;
using Avalonia.Interactivity;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class WorksPageView : UserControl
{
    private WorksPageViewModel? _attachedViewModel;
    private TextBox? _activeBlockEditor;
    private DocumentBlockViewModel? _activeBlock;
    /// <summary>
    /// 焦点移到项目 AI 输入框/发送按钮时，TextBox 选区可能被清空。
    /// 在 LostFocus 时固化最后一次非空选区，保证「选中 → 告诉 AI」仍可用。
    /// </summary>
    private EditorTextSelection? _stickySelection;

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

        CaptureStickySelectionFrom(sender as TextBox, clearWhenEmpty: false);
        viewModel.QuickAiCommand.Execute(null);
        e.Handled = true;
    }

    private void OnDocumentBlockEditorGotFocus(object? sender, RoutedEventArgs e)
    {
        _activeBlockEditor = sender as TextBox;
        _activeBlock = _activeBlockEditor?.DataContext as DocumentBlockViewModel;
        CaptureStickySelectionFrom(_activeBlockEditor, clearWhenEmpty: false);
    }

    private void OnDocumentBlockEditorLostFocus(object? sender, RoutedEventArgs e)
    {
        // 先固化选区，再让焦点去 AI 面板；空采样不得清 sticky。
        CaptureStickySelectionFrom(sender as TextBox, clearWhenEmpty: false);
    }

    private void OnDocumentBlockEditorPointerReleased(object? sender, PointerReleasedEventArgs e)
    {
        // 编辑器仍聚焦时若变成空选区，视为主动取消选中。
        CaptureStickySelectionFrom(sender as TextBox, clearWhenEmpty: true);
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
            if (_activeBlockEditor is not null)
            {
                _activeBlockEditor.SelectionStart = 0;
                _activeBlockEditor.SelectionEnd = _activeBlockEditor.Text?.Length ?? 0;
                _activeBlockEditor.Focus();
            }
        };
        viewModel.RequestEditorSelection = CurrentEditorSelection;
        viewModel.RequestRevealEditorRange = RevealEditorRange;
        viewModel.ClearStickyEditorSelection = ClearStickySelectionState;
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
            _attachedViewModel.PickImportSourceFile = null;
            _attachedViewModel.OpenFolderInShell = null;
            _attachedViewModel = null;
        }

        base.OnDetachedFromVisualTree(e);
    }

    private void RevealEditorRange(int globalStart, int globalEnd)
    {
        if (DataContext is not WorksPageViewModel viewModel
            || !viewModel.TryResolveBlockSelection(
                globalStart,
                globalEnd,
                out var block,
                out var localStart,
                out var localEnd)
            || block is null)
        {
            return;
        }

        viewModel.IsEditMode = true;
        Dispatcher.UIThread.Post(() =>
        {
            DocumentEditor.SelectedItem = block;
            DocumentEditor.ScrollIntoView(block);
            Dispatcher.UIThread.Post(() =>
            {
                if (DocumentEditor.ContainerFromItem(block) is not Control container)
                {
                    return;
                }

                var editor = container.GetVisualDescendants().OfType<TextBox>().FirstOrDefault();
                if (editor is null)
                {
                    return;
                }

                _activeBlock = block;
                _activeBlockEditor = editor;
                editor.SelectionStart = Math.Clamp(localStart, 0, editor.Text?.Length ?? 0);
                editor.SelectionEnd = Math.Clamp(localEnd, editor.SelectionStart, editor.Text?.Length ?? 0);
                editor.Focus();
                CaptureStickySelectionFrom(editor, clearWhenEmpty: false);
            }, DispatcherPriority.Loaded);
        }, DispatcherPriority.Loaded);
    }

    private void ClearStickySelectionState()
    {
        _stickySelection = null;
        // Keep last editor ref for next focus; indices are invalid across documents.
    }

    private EditorTextSelection CurrentEditorSelection()
    {
        // Prefer live selection on the focused (or last focused) block editor.
        // Do not clear sticky here: empty live sample during AI focus is expected.
        CaptureStickySelectionFrom(_activeBlockEditor, clearWhenEmpty: false);
        if (_stickySelection is { } sticky
            && sticky.End > sticky.Start
            && !string.IsNullOrWhiteSpace(sticky.Text))
        {
            return sticky;
        }

        return new EditorTextSelection(0, 0, string.Empty);
    }

    private void CaptureStickySelectionFrom(TextBox? editor, bool clearWhenEmpty)
    {
        if (DataContext is not WorksPageViewModel viewModel
            || editor is null
            || editor.DataContext is not DocumentBlockViewModel block)
        {
            return;
        }

        var start = Math.Min(editor.SelectionStart, editor.SelectionEnd);
        var end = Math.Max(editor.SelectionStart, editor.SelectionEnd);
        var selected = editor.SelectedText ?? string.Empty;
        if (string.IsNullOrEmpty(selected) && end > start && end <= (editor.Text?.Length ?? 0))
        {
            selected = editor.Text![start..end];
        }

        if (end > start && !string.IsNullOrWhiteSpace(selected))
        {
            _activeBlockEditor = editor;
            _activeBlock = block;
            var mapped = viewModel.SelectionForBlock(block, start, end, selected);
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
            start,
            end,
            selected,
            clearWhenEmpty);
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
