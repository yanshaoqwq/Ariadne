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

    /// <summary>
    /// U132：Ctrl+A 的「全选整章」是否仍然生效。
    ///
    /// 需要这个状态是因为虚拟化：Ctrl+A 当场只刷得到已实体化的块，
    /// 被回收的块滚进视口时是**新的控件实例**、选区为空。
    /// 没有它，「Ctrl+A 后往下滚，后半章是没选中的」——而用户看不出为什么。
    /// 任何一次改动正文或换文档都要清掉它（旧的全选态不该套到新内容上）。
    /// </summary>
    private bool _readingSelectAllActive;

    public WorksPageView()
    {
        InitializeComponent();
        DocumentEditor.TextArea.SelectionChanged += OnDocumentEditorSelectionChanged;
        DocumentEditor.TextArea.Caret.PositionChanged += OnDocumentEditorCaretPositionChanged;
        // U132：必须挂 Tunnel（预览）阶段。SelectableTextBlock 自己也处理
        // PointerPressed 来起新选区，冒泡阶段事件可能已被它标记 Handled，
        // XAML 里写 PointerPressed="..." 挂的是冒泡，会时灵时不灵。
        DocumentReaderScroll.AddHandler(
            PointerPressedEvent,
            OnReadingSurfacePointerPressed,
            RoutingStrategies.Tunnel);
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
            return;
        }

        // U132：阅读态的 Ctrl+A / Ctrl+C 必须在**页面级**处理。
        // SelectableTextBlock 自带这两个手势，但只在该块拿到键盘焦点时触发
        // （Focusable 默认 False，点正文永远拿不到焦点），且 SelectAll()/Copy()
        // 只操作自己那一个块——最多选中 4000–6000 字符，不是整章。
        // 编辑态不拦：AvaloniaEdit 自己处理得更好（有撤销栈、有列选区）。
        if (viewModel.IsEditMode)
        {
            return;
        }

        if (e.Key == Key.A)
        {
            SelectAllReadingBlocks();
            e.Handled = true;
            return;
        }

        if (e.Key == Key.C)
        {
            _ = CopySelectionAsync();
            e.Handled = true;
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
                return;
            }

            // U132：阅读态也要能全选。缺陷版本这里只有 IsEditMode 分支，
            // 阅读态点「全选」毫无反应——一个存在但不工作的入口。
            SelectAllReadingBlocks();
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
        // U132：换文档时全选态必须一并作废——否则新章节刚渲染就整片刷黑，
        // 用户从没按过 Ctrl+A。ClearStickyEditorSelection 已在三处换文档入口被调用，
        // 挂在这里就不必再找一遍那三处（漏一处就是一个只在特定路径出现的怪现象）。
        _readingSelectAllActive = false;
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

    /// <summary>
    /// U132：块被虚拟化重新实体化时补上全选态。
    ///
    /// Ctrl+A 当场只能刷到已实体化的块。没有这一步，用户 Ctrl+A 之后往下滚，
    /// 后半章是没选中的——而屏幕上看不出为什么，Ctrl+C 拿到的也就只有前半段。
    /// </summary>
    private void OnReadingBlockPrepared(object? sender, ContainerPreparedEventArgs e)
    {
        if (!_readingSelectAllActive)
        {
            return;
        }

        var block = e.Container as SelectableTextBlock
                    ?? e.Container.GetVisualDescendants().OfType<SelectableTextBlock>().FirstOrDefault();
        if (block is null)
        {
            return;
        }

        // 容器刚准备好时 Text 可能尚未由绑定填入，等一轮再刷。
        Dispatcher.UIThread.Post(() =>
        {
            if (!_readingSelectAllActive)
            {
                return;
            }
            block.SelectionStart = 0;
            block.SelectionEnd = block.Text?.Length ?? 0;
        }, DispatcherPriority.Loaded);
    }

    /// <summary>
    /// U132：在正文上按下指针即解除「全选整章」态。
    ///
    /// 用户点一下就是要重新开始选，此时若仍保留全选态，滚下去的新块会被
    /// <see cref="OnReadingBlockPrepared"/> 继续刷黑——用户会看到自己刚取消的选中
    /// 又冒出来。用 Tunnel（预览）阶段拿事件：SelectableTextBlock 自己也处理
    /// PointerPressed 来起新选区，冒泡阶段可能已被它标记 Handled。
    /// </summary>
    private void OnReadingSurfacePointerPressed(object? sender, PointerPressedEventArgs e)
    {
        _readingSelectAllActive = false;
    }

    private async Task CopySelectionAsync()
    {
        try
        {
            if (DataContext is not WorksPageViewModel viewModel)
            {
                return;
            }

            if (viewModel.IsEditMode)
            {
                if (DocumentEditor.SelectionLength > 0)
                {
                    DocumentEditor.Copy();
                }
                return;
            }

            // U132：阅读态复制**用户选中的那一段**。缺陷版本在非编辑态直接回退
            // viewModel.DocumentContent——实测把整章 51088 字符塞进剪贴板、
            // 无视用户选中的 10 个字。用户按 Ctrl+C 想要的是他刚刷黑的句子。
            var selections = CollectReadingSelections();
            var selectedText = ReadingSelectionAggregator.HasSelection(selections)
                ? ReadingSelectionAggregator.Aggregate(viewModel.DocumentBlocks, selections)
                : string.Empty;
            if (string.IsNullOrEmpty(selectedText))
            {
                // 一字未选就什么也不复制。悄悄换成整章会让用户以为复制成功了，
                // 直到粘贴出五万字才发现——比明确的「没反应」难排查得多。
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

    /// <summary>
    /// U132：把阅读态每个可见块的选区采样出来，交给页面级归并。
    ///
    /// 只能拿到**已实体化**的块——虚拟化会回收远处的块，它们没有控件实例。
    /// 这是 Ctrl+A 之外的场景（拖选）的固有限制：跨块拖选在当前结构下本就不可能
    /// （正文是 9 个独立 SelectableTextBlock），所以实际采样到的永远是视口内那一两块。
    /// </summary>
    private List<ReadingSelectionAggregator.BlockSelection> CollectReadingSelections()
    {
        var result = new List<ReadingSelectionAggregator.BlockSelection>();
        foreach (var (index, block) in EnumerateReadingBlocks())
        {
            var start = Math.Min(block.SelectionStart, block.SelectionEnd);
            var end = Math.Max(block.SelectionStart, block.SelectionEnd);
            if (end > start)
            {
                result.Add(new ReadingSelectionAggregator.BlockSelection(index, start, end));
            }
        }
        return result;
    }

    /// <summary>
    /// U132：Ctrl+A 选中**整章**，而不是当前这一块。
    ///
    /// 逐块 SelectAll 是这个结构下唯一可行的做法：正文被切成多个独立控件，
    /// 没有一个跨块的选区模型。视觉上会看到已渲染的块整片刷黑；
    /// 被虚拟化回收的块没有控件实例、当场刷不到，但它们滚进视口时会重新走
    /// 容器准备逻辑——所以这里同时记住「全选态」，由 <see cref="OnReadingBlockAttached"/>
    /// 补上。否则「Ctrl+A 后往下滚，后半章是没选中的」。
    /// </summary>
    private void SelectAllReadingBlocks()
    {
        _readingSelectAllActive = true;
        foreach (var (_, block) in EnumerateReadingBlocks())
        {
            block.SelectionStart = 0;
            block.SelectionEnd = block.Text?.Length ?? 0;
        }
    }

    private IEnumerable<(int Index, SelectableTextBlock Block)> EnumerateReadingBlocks()
    {
        if (DocumentReaderScroll.Presenter?.Child is not ItemsPresenter itemsPresenter
            || itemsPresenter.Panel is not { } panel)
        {
            yield break;
        }

        for (var index = 0; index < panel.Children.Count; index++)
        {
            // 模板根就是 SelectableTextBlock；若将来模板变了，用视觉树兜底而不是崩。
            var child = panel.Children[index];
            var block = child as SelectableTextBlock
                        ?? child.GetVisualDescendants().OfType<SelectableTextBlock>().FirstOrDefault();
            if (block is not null)
            {
                yield return (index, block);
            }
        }
    }
}
