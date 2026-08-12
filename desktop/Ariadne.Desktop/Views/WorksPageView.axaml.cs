using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.Presenters;
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
            _attachedViewModel.CaptureReadingOffset = null;
            _attachedViewModel.RestoreReadingOffset = null;
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
        viewModel.CaptureReadingOffset = CaptureReadingOffset;
        viewModel.RestoreReadingOffset = RestoreReadingOffset;
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
            _attachedViewModel.CaptureReadingOffset = null;
            _attachedViewModel.RestoreReadingOffset = null;
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

    /// <summary>
    /// U129：读出**当前可见视图**顶部对应的全篇字符偏移。
    ///
    /// 两条路径的坐标系完全不同，所以各自换算后统一成字符偏移：
    /// - 编辑器：VerticalOffset → 视觉行 → DocumentLine.Offset（AvaloniaEdit 自带映射）
    /// - 阅读器：ScrollViewer.Offset.Y → 落在哪个块 + 块内比例 → 字符偏移
    ///
    /// 返回 null 表示此刻问不出有效位置（正文空、控件未测量），
    /// 调用方据此保留上一个锚点而不是覆盖成 0。
    /// </summary>
    private int? CaptureReadingOffset()
    {
        if (DataContext is not WorksPageViewModel viewModel)
        {
            return null;
        }

        return viewModel.IsEditMode
            ? CaptureEditorOffset()
            : CaptureReaderOffset(viewModel);
    }

    private int? CaptureEditorOffset()
    {
        var document = DocumentEditor.Document;
        if (document is null || document.TextLength == 0)
        {
            return null;
        }

        var textView = DocumentEditor.TextArea.TextView;
        // TextView 未完成测量时 VisualLines 为空，GetDocumentLineByVisualTop 会给出
        // 无意义的结果——此时宁可返回 null 让调用方保留旧锚点。
        if (textView.VisualLinesValid && textView.VisualLines.Count > 0)
        {
            var line = textView.GetDocumentLineByVisualTop(DocumentEditor.VerticalOffset);
            if (line is not null)
            {
                return line.Offset;
            }
        }

        // 回退：用光标所在偏移。它未必是屏幕顶部，但比「从头开始」接近得多。
        return Math.Clamp(DocumentEditor.CaretOffset, 0, document.TextLength);
    }

    private int? CaptureReaderOffset(WorksPageViewModel viewModel)
    {
        var blocks = viewModel.DocumentBlocks;
        if (blocks.Count == 0)
        {
            return null;
        }

        var presenter = DocumentReaderScroll.Presenter;
        var scrollTop = DocumentReaderScroll.Offset.Y;
        if (presenter is null)
        {
            return null;
        }

        // 逐块累加实际高度找出滚动线落在哪一块。不能用「块索引 ÷ 块数 × 总高」
        // 之类的比例估算——末块常常只有几行，各块高度差好几倍。
        var cumulative = 0d;
        for (var index = 0; index < blocks.Count; index++)
        {
            var height = BlockVisualHeight(index);
            if (height <= 0d)
            {
                // 虚拟化把远处的块回收了，没有实体可测。此时该块必然不在视口内，
                // 而滚动线只会落在视口附近的块上，跳过是安全的。
                continue;
            }

            if (scrollTop < cumulative + height)
            {
                var ratio = (scrollTop - cumulative) / height;
                return ReadingPositionMapper.OffsetFromBlockProgress(blocks, index, ratio);
            }
            cumulative += height;
        }

        // 滚到底：锚在末块尾部。
        return ReadingPositionMapper.OffsetFromBlockProgress(blocks, blocks.Count - 1, 1d);
    }

    /// <summary>取第 index 个阅读块的实际渲染高度；块被虚拟化回收时返回 0。</summary>
    private double BlockVisualHeight(int index)
    {
        if (DocumentReaderScroll.Presenter?.Child is not ItemsPresenter itemsPresenter
            || itemsPresenter.Panel is not { } panel
            || index < 0
            || index >= panel.Children.Count)
        {
            return 0d;
        }

        var child = panel.Children[index];
        return child.Bounds.Height;
    }

    /// <summary>
    /// U129：把字符偏移恢复到**切换后**的视图。
    ///
    /// 必须等一轮布局：切换瞬间新视图的 IsVisible 刚翻转，尚未测量，
    /// 此刻设 Offset/ScrollTo 会被随后的布局重置回 0——那正是「位置丢失」
    /// 最常见的成因，而且现象与「压根没实现」一模一样，极难分辨。
    /// 用 Loaded 优先级（低于 Layout）确保测量已完成。
    /// </summary>
    private void RestoreReadingOffset(int offset)
    {
        Dispatcher.UIThread.Post(() =>
        {
            if (DataContext is not WorksPageViewModel viewModel)
            {
                return;
            }

            if (viewModel.IsEditMode)
            {
                RestoreEditorOffset(offset);
            }
            else
            {
                RestoreReaderOffset(viewModel, offset);
            }
        }, DispatcherPriority.Loaded);
    }

    private void RestoreEditorOffset(int offset)
    {
        var document = DocumentEditor.Document;
        if (document is null || document.TextLength == 0)
        {
            return;
        }

        var clamped = Math.Clamp(offset, 0, document.TextLength);
        var line = document.GetLineByOffset(clamped);
        // ScrollToLine 把目标行**居中**，而我们要的是「原来在顶部的行仍在顶部」。
        // 所以直接按视觉顶推算 VerticalOffset：先确保该行已构造出视觉行。
        DocumentEditor.ScrollToLine(line.LineNumber);
        var textView = DocumentEditor.TextArea.TextView;
        textView.EnsureVisualLines();
        var visualTop = textView.GetVisualTopByDocumentLine(line.LineNumber);
        DocumentEditor.ScrollToVerticalOffset(visualTop);
    }

    private void RestoreReaderOffset(WorksPageViewModel viewModel, int offset)
    {
        var blocks = viewModel.DocumentBlocks;
        if (!ReadingPositionMapper.TryLocateOffset(blocks, offset, out var blockIndex, out var ratio))
        {
            return;
        }

        // 先把目标块滚进视口，虚拟化才会把它实体化，之后才量得到高度。
        DocumentReaderScroll.UpdateLayout();
        var cumulative = 0d;
        for (var index = 0; index < blockIndex; index++)
        {
            cumulative += BlockVisualHeight(index);
        }
        var targetHeight = BlockVisualHeight(blockIndex);
        var top = cumulative + (targetHeight * ratio);
        var maxOffset = Math.Max(0d, DocumentReaderScroll.Extent.Height - DocumentReaderScroll.Viewport.Height);
        DocumentReaderScroll.Offset = new Vector(
            DocumentReaderScroll.Offset.X,
            Math.Clamp(top, 0d, maxOffset));
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
