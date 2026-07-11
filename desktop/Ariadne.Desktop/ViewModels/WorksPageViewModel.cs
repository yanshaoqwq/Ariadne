using System.Collections.ObjectModel;
using System.Text;
using Avalonia.Controls;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class WorksPageViewModel : ViewModelBase, IUnsavedChangesGuard, IProjectDataReloadable
{
    private const double MinRightPanelWidth = 280;
    private const double MaxRightPanelWidth = 520;
    private const double CollapsedRightPanelWidth = 24;
    private const int TargetDocumentBlockSize = 4_000;
    private const int HardDocumentBlockSize = 6_000;
    private const int RebalanceDocumentBlockSize = HardDocumentBlockSize * 2;

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private bool _isRightPanelOpen = true;
    private GridLength _rightPanelColumnWidth = new(320);
    private bool _isNavTreeTab = true;
    private bool _isImportPanelOpen;
    private string _documentContent = string.Empty;
    private string _statusText = string.Empty;
    private string _projectAiMessage = string.Empty;
    private string _projectAiAnswer;
    private readonly List<ProjectAiChatMessage> _projectAiHistory = new();
    private string _quickEditInstruction = string.Empty;
    private string _quickEditDiff = string.Empty;
    private string _exportFormat = "markdown";
    private string _currentDocumentId = string.Empty;
    private string _currentDocumentPath = string.Empty;
    private string? _currentDocumentVersion;
    private string _documentTitle;
    private string _importChapterId = string.Empty;
    private string _importChapterTitle = string.Empty;
    private string _importOrder = "0";
    private string _importSourcePath = string.Empty;
    private string _importTargetPath = string.Empty;
    private string _savedSnapshot = string.Empty;
    private bool _hasUnsavedChanges;
    private bool _suppressDirtyTracking;
    private bool _suppressDocumentBlockChanges;
    private bool _documentDirty;
    private bool _documentContentCacheValid = true;
    private int _documentCharacterCount;
    private int _nextDocumentBlockId;
    private bool _isEditMode;
    private QuickEditSession? _pendingQuickEdit;
    private QuickEditUndoState? _quickEditUndo;
    private CancellationTokenSource? _quickEditGenerationCts;
    private long _quickEditGeneration;
    private bool _isQuickEditGenerating;

    public WorksPageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        _projectAiAnswer = displayNames.Text("ui.works.project_ai.empty");
        _documentTitle = displayNames.Text("ui.works.no_document_selected");
        WorksTreeNodes = new ObservableCollection<WorksTreeItemViewModel>();
        DocumentBlocks = new ObservableCollection<DocumentBlockViewModel>();
        ProjectAiBubbles = new ObservableCollection<ChatBubbleViewModel>();
        ToggleRightPanelCommand = new RelayCommand(() => IsRightPanelOpen = !IsRightPanelOpen);
        ShowNavTreeCommand = new RelayCommand(() => IsNavTreeTab = true);
        ShowProjectAiCommand = new RelayCommand(() => IsNavTreeTab = false);
        OpenImportPanelCommand = new RelayCommand(OpenImportPanel);
        ToggleImportPanelCommand = new RelayCommand(() => IsImportPanelOpen = !IsImportPanelOpen);
        BrowseImportSourceCommand = new RelayCommand(() => _ = BrowseImportSourceAsync());
        ImportCommand = new RelayCommand(() => _ = ImportChapterAsync(), CanImportChapter);
        ExportCommand = new RelayCommand(() => _ = ExportAsync(), () => WorksTreeNodes.Count > 0);
        SaveCommand = new RelayCommand(() => _ = SaveAsync(), () => HasCurrentDocument);
        ReadModeCommand = new RelayCommand(() => IsEditMode = false);
        EditModeCommand = new RelayCommand(() => IsEditMode = true);
        CopyCommand = new RelayCommand(() => RequestEditorCopy?.Invoke());
        SelectAllCommand = new RelayCommand(() => RequestEditorSelectAll?.Invoke());
        QuickAiCommand = new RelayCommand(() =>
        {
            IsEditMode = true;
            _ = QuickEditAsync();
        }, CanGenerateQuickEdit);
        InsertOutlineCommand = new RelayCommand(InsertOutlineReference, () => HasCurrentDocument);
        ToggleEditCommand = new RelayCommand(() => IsEditMode = !IsEditMode);
        SendProjectAiCommand = new RelayCommand(() => _ = SendProjectAiAsync(), CanSendProjectAi);
        ApplyQuickEditCommand = new RelayCommand(ApplyQuickEdit, CanApplyQuickEdit);
        UndoQuickEditCommand = new RelayCommand(UndoQuickEdit, CanUndoQuickEdit);
        ExportFormats = new ObservableCollection<ExportFormatOption>
        {
            new("markdown", displayNames.Text("ui.works.export_format.markdown")),
            new("epub", displayNames.Text("ui.works.export_format.epub")),
            new("pdf", displayNames.Text("ui.works.export_format.pdf")),
        };
        _ = InitializeAsync();
        CaptureSnapshot();
    }

    public string ToggleRightPanelText => _displayNames.Text("ui.action.toggle_right_panel");

    /// 右侧栏开合状态；收起后由悬浮左向箭头重新展开。
    public bool IsRightPanelOpen
    {
        get => _isRightPanelOpen;
        set
        {
            if (SetProperty(ref _isRightPanelOpen, value))
            {
                OnPropertyChanged(nameof(RightPanelSplitterWidth));
                OnPropertyChanged(nameof(RightPanelColumnWidth));
            }
        }
    }

    public RelayCommand ToggleRightPanelCommand { get; }

    public GridLength RightPanelSplitterWidth => IsRightPanelOpen ? new GridLength(4) : new GridLength(0);

    public GridLength RightPanelColumnWidth
    {
        get => IsRightPanelOpen ? _rightPanelColumnWidth : new GridLength(CollapsedRightPanelWidth);
        set
        {
            if (!IsRightPanelOpen)
            {
                return;
            }
            var normalized = NormalizeRightPanelWidth(value);
            if (!_rightPanelColumnWidth.Equals(normalized))
            {
                _rightPanelColumnWidth = normalized;
                OnPropertyChanged();
            }
        }
    }

    /// 右栏标签：true=导航树（含章节树/大纲），false=项目 AI。
    public bool IsNavTreeTab
    {
        get => _isNavTreeTab;
        set
        {
            if (SetProperty(ref _isNavTreeTab, value))
            {
                OnPropertyChanged(nameof(IsProjectAiTab));
            }
        }
    }

    public bool IsProjectAiTab => !_isNavTreeTab;

    public bool IsImportPanelOpen
    {
        get => _isImportPanelOpen;
        set => SetProperty(ref _isImportPanelOpen, value);
    }

    public RelayCommand ShowNavTreeCommand { get; }

    public RelayCommand ShowProjectAiCommand { get; }

    public RelayCommand OpenImportPanelCommand { get; }

    public RelayCommand ToggleImportPanelCommand { get; }

    public RelayCommand BrowseImportSourceCommand { get; }

    /// <summary>View 注入：挑选导入源文件路径。</summary>
    public Func<Task<string?>>? PickImportSourceFile { get; set; }

    /// <summary>View 注入：在文件管理器中打开目录。</summary>
    public Func<string, Task>? OpenFolderInShell { get; set; }

    public RelayCommand ImportCommand { get; }

    public RelayCommand ExportCommand { get; }

    public RelayCommand SaveCommand { get; }

    public RelayCommand ReadModeCommand { get; }

    public RelayCommand EditModeCommand { get; }

    public RelayCommand CopyCommand { get; }

    public RelayCommand SelectAllCommand { get; }

    public RelayCommand QuickAiCommand { get; }

    public RelayCommand InsertOutlineCommand { get; }

    public RelayCommand ToggleEditCommand { get; }

    public RelayCommand SendProjectAiCommand { get; }

    public RelayCommand ApplyQuickEditCommand { get; }
    public RelayCommand UndoQuickEditCommand { get; }
    public Action? RequestEditorCopy { get; set; }
    public Action? RequestEditorSelectAll { get; set; }
    public Func<EditorTextSelection>? RequestEditorSelection { get; set; }

    public ObservableCollection<WorksTreeItemViewModel> WorksTreeNodes { get; }

    public ObservableCollection<DocumentBlockViewModel> DocumentBlocks { get; }
    public bool HasDocumentBlocks => DocumentBlocks.Count > 0;
    public ObservableCollection<ChatBubbleViewModel> ProjectAiBubbles { get; }
    public bool HasProjectAiBubbles => ProjectAiBubbles.Count > 0;

    public ObservableCollection<ExportFormatOption> ExportFormats { get; }

    public bool IsWorksTreeEmpty => WorksTreeNodes.Count == 0;

    public bool IsEditMode
    {
        get => _isEditMode;
        set => SetProperty(ref _isEditMode, value);
    }

    public string DocumentContent
    {
        get => AssembleDocumentContent();
        set => ReplaceDocumentContent(value ?? string.Empty);
    }

    public bool HasUnsavedChanges
    {
        get => _hasUnsavedChanges;
        private set
        {
            if (SetProperty(ref _hasUnsavedChanges, value))
            {
                OnPropertyChanged(nameof(DocumentInfoText));
            }
        }
    }

    public bool HasCurrentDocument => !string.IsNullOrWhiteSpace(_currentDocumentId);

    public string StatusText
    {
        get => _statusText;
        set => SetProperty(ref _statusText, value);
    }

    public string ProjectAiMessage
    {
        get => _projectAiMessage;
        set
        {
            if (SetProperty(ref _projectAiMessage, value))
            {
                SendProjectAiCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public string ProjectAiAnswer
    {
        get => _projectAiAnswer;
        set => SetProperty(ref _projectAiAnswer, value);
    }

    public string QuickEditInstruction
    {
        get => _quickEditInstruction;
        set
        {
            if (SetProperty(ref _quickEditInstruction, value))
            {
                InvalidateQuickEditGeneration();
                QuickAiCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public string QuickEditDiff
    {
        get => _quickEditDiff;
        set => SetProperty(ref _quickEditDiff, value);
    }

    public bool IsQuickEditGenerating
    {
        get => _isQuickEditGenerating;
        private set
        {
            if (SetProperty(ref _isQuickEditGenerating, value))
            {
                OnPropertyChanged(nameof(QuickEditGenerateText));
                QuickAiCommand.NotifyCanExecuteChanged();
                ApplyQuickEditCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public string ExportFormat
    {
        get => _exportFormat;
        set => SetProperty(ref _exportFormat, value);
    }

    public string DocumentTitle
    {
        get => _documentTitle;
        set
        {
            if (SetProperty(ref _documentTitle, value))
            {
                OnPropertyChanged(nameof(CurrentDocumentText));
            }
        }
    }

    public string ImportChapterId { get => _importChapterId; set { if (SetProperty(ref _importChapterId, value)) ImportCommand.NotifyCanExecuteChanged(); } }
    public string ImportChapterTitle { get => _importChapterTitle; set { if (SetProperty(ref _importChapterTitle, value)) ImportCommand.NotifyCanExecuteChanged(); } }
    public string ImportOrder { get => _importOrder; set { if (SetProperty(ref _importOrder, value)) ImportCommand.NotifyCanExecuteChanged(); } }
    public string ImportSourcePath { get => _importSourcePath; set { if (SetProperty(ref _importSourcePath, value)) ImportCommand.NotifyCanExecuteChanged(); } }
    public string ImportTargetPath { get => _importTargetPath; set { if (SetProperty(ref _importTargetPath, value)) ImportCommand.NotifyCanExecuteChanged(); } }

    public string SidebarTitle => _displayNames.Text("ui.works.sidebar.title");

    public string ImportText => _displayNames.Text("ui.works.import_manuscript");

    public string ExportText => _displayNames.Text("ui.works.export_combined");

    public string ReadModeText => _displayNames.Text("ui.works.read_mode");

    public string EditModeText => _displayNames.Text("ui.works.edit_mode");

    public string SaveText => _displayNames.Text("ui.common.save");

    public string OutlineText => _displayNames.Text("ui.works.outline");

    public string NavTreeText => _displayNames.Text("ui.works.nav_tree");

    public string ProjectAiText => _displayNames.Text("ui.works.project_ai");

    public string NoDocumentText => _displayNames.Text("ui.works.no_document_selected");

    public string CurrentDocumentText => DocumentTitle;

    public string DocumentInfoText => string.IsNullOrWhiteSpace(_currentDocumentId)
        ? NoDocumentText
        : _displayNames.Format("ui.works.document_info", new Dictionary<string, string>
        {
            ["path"] = string.IsNullOrWhiteSpace(_currentDocumentPath) ? _currentDocumentId : _currentDocumentPath,
            ["version"] = ShortValue(_currentDocumentVersion),
            ["blocks"] = DocumentBlocks.Count.ToString(),
            ["state"] = HasUnsavedChanges
                ? _displayNames.Text("ui.works.save_state.unsaved")
                : _displayNames.Text("ui.works.save_state.saved"),
        });

    public string DocumentBodyText => string.IsNullOrWhiteSpace(_currentDocumentId)
        ? NoDocumentText
        : _displayNames.Text("ui.works.empty_document");

    public string CharacterCountText => _displayNames.Format("ui.works.characters_count", new Dictionary<string, string>
    {
        ["count"] = _documentCharacterCount.ToString(),
    });

    public string EmptyIndexText => _displayNames.Text("ui.works.empty_index");
    public string EmptyIndexTitle => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.works.index.title")
        : _displayNames.Text("ui.empty.need_project.title");
    public string EmptyIndexHint => _backend.HasProjectRoot
        ? _displayNames.Text("ui.empty.works.index.hint")
        : _displayNames.Text("ui.empty.need_project.hint");

    public string QuickAiHint => _displayNames.Text("ui.works.quick_ai_hint");

    public string ProjectAiPlaceholder => _displayNames.Text("ui.works.project_ai.placeholder");

    public string ExportFormatText => _displayNames.Text("ui.works.export_format");
    public string ImportTitle => _displayNames.Text("ui.works.import.title");
    public string ImportChapterIdText => _displayNames.Text("ui.works.import.chapter_id");
    public string ImportChapterTitleText => _displayNames.Text("ui.works.import.chapter_title");
    public string ImportOrderText => _displayNames.Text("ui.works.import.order");
    public string ImportSourcePathText => _displayNames.Text("ui.works.import.source_path");
    public string ImportTargetPathText => _displayNames.Text("ui.works.import.target_path");
    public string ImportSourcePlaceholder => _displayNames.Text("ui.works.import.source_placeholder");
    public string ImportTargetPlaceholder => _displayNames.Text("ui.works.import.target_placeholder");
    public string BrowseImportSourceText => _displayNames.Text("ui.works.import.browse_source");
    public string QuickEditTitle => _displayNames.Text("ui.works.quick_edit.title");
    public string QuickEditPlaceholder => _displayNames.Text("ui.works.quick_edit.placeholder");
    public string QuickEditGenerateText => _displayNames.Text(IsQuickEditGenerating
        ? "ui.works.quick_edit.generating"
        : "ui.works.quick_edit.generate");
    public string QuickEditDiffText => _displayNames.Text("ui.works.quick_edit.diff");
    public string QuickEditApplyText => _displayNames.Text("ui.works.quick_edit.apply");
    public string QuickEditUndoText => _displayNames.Text("ui.works.quick_edit.undo");

    // 右键菜单文案（阅读/修改器）
    public string CtxCopyText => _displayNames.Text("ui.works.context.copy");
    public string CtxSelectAllText => _displayNames.Text("ui.works.context.select_all");
    public string CtxQuickAiText => _displayNames.Text("ui.works.context.quick_ai");
    public string CtxInsertOutlineText => _displayNames.Text("ui.works.context.insert_outline");
    public string CtxToggleEditText => _displayNames.Text("ui.works.context.toggle_edit");

    private bool CanImportChapter()
    {
        return !string.IsNullOrWhiteSpace(ImportChapterId)
               && !string.IsNullOrWhiteSpace(ImportChapterTitle)
               && long.TryParse(ImportOrder, out _)
               && !string.IsNullOrWhiteSpace(ImportSourcePath)
               && !string.IsNullOrWhiteSpace(ImportTargetPath);
    }

    private bool CanGenerateQuickEdit()
    {
        return HasCurrentDocument
               && _documentCharacterCount > 0
               && !IsQuickEditGenerating
               && !string.IsNullOrWhiteSpace(QuickEditInstruction);
    }

    private bool CanApplyQuickEdit()
    {
        return !IsQuickEditGenerating
               && _pendingQuickEdit is not null
               && _pendingQuickEdit.MatchesCurrent(
                   _currentDocumentId,
                   _currentDocumentVersion,
                   AssembleDocumentContent());
    }

    private bool CanUndoQuickEdit()
    {
        return _quickEditUndo is not null
               && _quickEditUndo.TryUndo(
                   _currentDocumentId,
                   AssembleDocumentContent(),
                   out _);
    }

    private bool CanSendProjectAi()
    {
        return !string.IsNullOrWhiteSpace(ProjectAiMessage);
    }

    private void OnCurrentDocumentChanged()
    {
        OnPropertyChanged(nameof(HasCurrentDocument));
        OnPropertyChanged(nameof(DocumentInfoText));
        SaveCommand.NotifyCanExecuteChanged();
        InsertOutlineCommand.NotifyCanExecuteChanged();
        QuickAiCommand.NotifyCanExecuteChanged();
    }

    private void ClearPendingQuickEdit()
    {
        if (_pendingQuickEdit is null
            && string.IsNullOrEmpty(QuickEditDiff))
        {
            return;
        }

        _pendingQuickEdit = null;
        QuickEditDiff = string.Empty;
        ApplyQuickEditCommand.NotifyCanExecuteChanged();
    }

    private void InvalidateQuickEditGeneration()
    {
        Interlocked.Increment(ref _quickEditGeneration);
        _quickEditGenerationCts?.Cancel();
        _quickEditGenerationCts?.Dispose();
        _quickEditGenerationCts = null;
        IsQuickEditGenerating = false;
        ClearPendingQuickEdit();
    }

    private void ClearQuickEditUndo()
    {
        if (_quickEditUndo is null)
        {
            return;
        }
        _quickEditUndo = null;
        UndoQuickEditCommand.NotifyCanExecuteChanged();
    }

    private void ReplaceDocumentContent(string content)
    {
        _suppressDocumentBlockChanges = true;
        try
        {
            _documentContent = content;
            _documentContentCacheValid = true;
            _documentCharacterCount = content.Length;
            RebuildDocumentBlocks(content);
        }
        finally
        {
            _suppressDocumentBlockChanges = false;
        }

        OnPropertyChanged(nameof(DocumentContent));
        OnPropertyChanged(nameof(DocumentBodyText));
        OnPropertyChanged(nameof(CharacterCountText));
        OnPropertyChanged(nameof(HasDocumentBlocks));
        OnPropertyChanged(nameof(DocumentInfoText));
        QuickAiCommand.NotifyCanExecuteChanged();
        if (!_suppressDirtyTracking)
        {
            InvalidateQuickEditGeneration();
        }
    }

    private void OnDocumentBlockTextChanged(DocumentBlockViewModel block, string oldText, string newText)
    {
        if (_suppressDocumentBlockChanges)
        {
            return;
        }

        _documentCharacterCount += newText.Length - oldText.Length;
        _documentContentCacheValid = false;
        OnPropertyChanged(nameof(CharacterCountText));
        OnPropertyChanged(nameof(DocumentInfoText));
        QuickAiCommand.NotifyCanExecuteChanged();
        if (!_suppressDirtyTracking)
        {
            InvalidateQuickEditGeneration();
            ClearQuickEditUndo();
            MarkDocumentDirty();
        }
        if (newText.Length > RebalanceDocumentBlockSize)
        {
            RebalanceDocumentBlocks();
        }
    }

    private void MarkDocumentDirty()
    {
        _documentDirty = true;
        HasUnsavedChanges = true;
    }

    private string AssembleDocumentContent()
    {
        if (_documentContentCacheValid)
        {
            return _documentContent;
        }

        if (DocumentBlocks.Count == 0)
        {
            _documentContent = string.Empty;
            _documentContentCacheValid = true;
            return _documentContent;
        }

        var builder = new StringBuilder(_documentCharacterCount);
        foreach (var block in DocumentBlocks.OrderBy(block => block.Index))
        {
            builder.Append(block.Text);
        }
        _documentContent = builder.ToString();
        _documentContentCacheValid = true;
        return _documentContent;
    }

    private void RebalanceDocumentBlocks()
    {
        _suppressDocumentBlockChanges = true;
        try
        {
            RebuildDocumentBlocks(AssembleDocumentContent());
        }
        finally
        {
            _suppressDocumentBlockChanges = false;
        }
        OnPropertyChanged(nameof(HasDocumentBlocks));
        OnPropertyChanged(nameof(DocumentInfoText));
    }

    private void RebuildDocumentBlocks(string content)
    {
        DocumentBlocks.Clear();
        var index = 0;
        foreach (var block in SplitDocumentBlocks(content))
        {
            DocumentBlocks.Add(new DocumentBlockViewModel(
                $"block-{++_nextDocumentBlockId}",
                index++,
                block,
                OnDocumentBlockTextChanged));
        }
        OnPropertyChanged(nameof(DocumentInfoText));
    }

    public EditorTextSelection SelectionForBlock(DocumentBlockViewModel block, int localStart, int localEnd, string selectedText)
    {
        var start = Math.Clamp(Math.Min(localStart, localEnd), 0, block.Text.Length);
        var end = Math.Clamp(Math.Max(localStart, localEnd), 0, block.Text.Length);
        var prefixLength = 0;
        foreach (var item in DocumentBlocks.OrderBy(item => item.Index))
        {
            if (ReferenceEquals(item, block))
            {
                break;
            }
            prefixLength += item.Text.Length;
        }
        return new EditorTextSelection(prefixLength + start, prefixLength + end, selectedText);
    }

    private static IEnumerable<string> SplitDocumentBlocks(string content)
    {
        if (string.IsNullOrEmpty(content))
        {
            yield break;
        }

        var start = 0;
        while (start < content.Length)
        {
            var remaining = content.Length - start;
            if (remaining <= HardDocumentBlockSize)
            {
                yield return content[start..];
                yield break;
            }

            var limit = Math.Min(content.Length, start + HardDocumentBlockSize);
            var preferredStart = Math.Min(content.Length, start + TargetDocumentBlockSize);
            var split = content.LastIndexOf("\n\n", limit - 1, limit - start, StringComparison.Ordinal);
            if (split < preferredStart)
            {
                split = content.LastIndexOf('\n', limit - 1, limit - start);
            }
            if (split < preferredStart)
            {
                split = start + TargetDocumentBlockSize;
            }
            else
            {
                split += content[split] == '\n' ? 1 : 2;
            }

            split = Math.Clamp(split, start + 1, content.Length);
            yield return content[start..split];
            start = split;
        }
    }

    private async Task InitializeAsync()
    {
        await LoadWorksTreeAsync().ConfigureAwait(true);
    }

    private async Task LoadWorksTreeAsync()
    {
        if (!_backend.HasProjectRoot)
        {
            WorksTreeNodes.Clear();
            StatusText = string.Empty;
            OnPropertyChanged(nameof(IsWorksTreeEmpty));
            OnPropertyChanged(nameof(EmptyIndexTitle));
            OnPropertyChanged(nameof(EmptyIndexHint));
            ExportCommand.NotifyCanExecuteChanged();
            return;
        }

        try
        {
            var tree = await _backend.GetWorksTreeAsync().ConfigureAwait(true);
            WorksTreeNodes.Clear();
            foreach (var item in FlattenTree(tree))
            {
                WorksTreeNodes.Add(item);
            }
            StatusText = WorksTreeNodes.Count == 0 ? EmptyIndexText : $"{WorksTreeNodes.Count}";
        }
        catch
        {
            WorksTreeNodes.Clear();
            StatusText = string.Empty;
        }
        finally
        {
            OnPropertyChanged(nameof(IsWorksTreeEmpty));
            OnPropertyChanged(nameof(EmptyIndexTitle));
            OnPropertyChanged(nameof(EmptyIndexHint));
            ExportCommand.NotifyCanExecuteChanged();
        }
    }

    private async Task LoadDocumentAsync(WorksTreeItemViewModel item)
    {
        try
        {
            var nextDocumentId = ProjectRelativePath(item.Path);
            if (string.Equals(nextDocumentId, _currentDocumentId, StringComparison.Ordinal)
                && !string.IsNullOrWhiteSpace(_currentDocumentId))
            {
                return;
            }
            if (!await ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
            {
                return;
            }

            InvalidateQuickEditGeneration();
            ClearQuickEditUndo();

            _suppressDirtyTracking = true;
            try
            {
                var document = await _backend.GetDocumentContentDetailsByPathAsync(item.Path).ConfigureAwait(true);
                DocumentContent = document.Content;
                _currentDocumentId = nextDocumentId;
                _currentDocumentPath = document.Metadata.Path;
                _currentDocumentVersion = document.Metadata.Version;
                OnCurrentDocumentChanged();
                DocumentTitle = item.Title;
                OnPropertyChanged(nameof(DocumentBodyText));
            }
            finally
            {
                _suppressDirtyTracking = false;
            }
            CaptureSnapshot();
            StatusText = _displayNames.Text("ui.common.open");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task BrowseImportSourceAsync()
    {
        if (PickImportSourceFile is null)
        {
            StatusText = _displayNames.Text("ui.settings.browse_unavailable");
            return;
        }

        try
        {
            var path = await PickImportSourceFile().ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(path))
            {
                return;
            }

            ImportSourcePath = path;
            // 从文件名推导 id/标题/目标/排序；已填字段不覆盖
            var suggestion = WorksImportHelper.SuggestFromSourcePath(path, WorksTreeNodes.Count);
            var chapterId = ImportChapterId;
            var chapterTitle = ImportChapterTitle;
            var targetPath = ImportTargetPath;
            var order = ImportOrder;
            WorksImportHelper.ApplySuggestionIfEmpty(
                suggestion,
                ref chapterId,
                ref chapterTitle,
                ref targetPath,
                ref order);
            ImportChapterId = chapterId;
            ImportChapterTitle = chapterTitle;
            ImportTargetPath = targetPath;
            ImportOrder = order;
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task ImportChapterAsync()
    {
        try
        {
            await _backend.ImportChapterAsync(new ChapterImportRequest(
                ImportChapterId,
                ImportChapterTitle,
                long.TryParse(ImportOrder, out var order) ? order : 0,
                ImportSourcePath,
                ImportTargetPath)).ConfigureAwait(true);
            StatusText = _displayNames.Text("ui.common.import");
            IsImportPanelOpen = false;
            await LoadWorksTreeAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task SaveAsync()
    {
        try
        {
            if (string.IsNullOrWhiteSpace(_currentDocumentId))
            {
                StatusText = NoDocumentText;
                return;
            }
            var report = await _backend.SaveDocumentContentAsync(
                _currentDocumentId,
                AssembleDocumentContent(),
                _currentDocumentVersion).ConfigureAwait(true);
            _currentDocumentPath = report.Metadata.Path;
            _currentDocumentVersion = report.Metadata.Version;
            OnPropertyChanged(nameof(DocumentInfoText));
            CaptureSnapshot();
            StatusText = _displayNames.Text("ui.common.save");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task ExportAsync()
    {
        try
        {
            var report = await _backend.ExportChaptersAsync(Array.Empty<string>(), $"combined-{ExportFormat}", ExportFormat).ConfigureAwait(true);
            var path = string.IsNullOrWhiteSpace(report.StorageUri) ? report.ArtifactId : report.StorageUri;
            StatusText = _displayNames.Format("ui.works.export_done", new Dictionary<string, string>
            {
                ["format"] = report.Format,
                ["path"] = path,
            });
            // 成功后弹窗：关闭 + 打开所在文件夹（延后项：reveal 导出路径）
            var revealDir = ProjectPathHelper.ResolveRevealDirectory(path);
            var canReveal = !string.IsNullOrWhiteSpace(revealDir) && OpenFolderInShell is not null;
            var choice = await DialogService.Current.ConfirmAsync(new ConfirmDialogViewModel(
                _displayNames.Text("ui.works.export_done_title"),
                _displayNames.Format("ui.works.export_done_message", new Dictionary<string, string>
                {
                    ["format"] = report.Format,
                    ["path"] = path,
                }),
                canReveal
                    ? new[]
                    {
                        new DialogButton(_displayNames.Text("ui.works.export_open_folder"), DialogButtonVariant.Primary, 0),
                        new DialogButton(_displayNames.Text("ui.common.close"), DialogButtonVariant.Subtle, 1),
                    }
                    : new[]
                    {
                        new DialogButton(_displayNames.Text("ui.common.close"), DialogButtonVariant.Primary, 0),
                    })
            {
                CancelResultIndex = canReveal ? 1 : 0,
            }).ConfigureAwait(true);

            if (canReveal && choice == 0 && OpenFolderInShell is not null && !string.IsNullOrWhiteSpace(revealDir))
            {
                try
                {
                    await OpenFolderInShell(revealDir).ConfigureAwait(true);
                }
                catch (Exception openEx)
                {
                    StatusText = openEx.Message;
                }
            }
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task SendProjectAiAsync()
    {
        try
        {
            if (string.IsNullOrWhiteSpace(ProjectAiMessage))
            {
                StatusText = ProjectAiPlaceholder;
                return;
            }
            var result = await _backend.ProjectAiChatAsync(
                ProjectAiMessage,
                _projectAiHistory,
                workflowIdToRun: null).ConfigureAwait(true);
            ProjectAiAnswer = result.Answer;
            _projectAiHistory.Clear();
            ProjectAiBubbles.Clear();
            foreach (var message in result.ChatHistory)
            {
                _projectAiHistory.Add(message);
                ProjectAiBubbles.Add(new ChatBubbleViewModel(message.Role, message.Content));
            }
            if (ProjectAiBubbles.Count == 0 && !string.IsNullOrWhiteSpace(result.Answer))
            {
                ProjectAiBubbles.Add(new ChatBubbleViewModel("assistant", result.Answer));
            }
            OnPropertyChanged(nameof(HasProjectAiBubbles));
            ProjectAiMessage = string.Empty;
            StatusText = _displayNames.Text("ui.common.configured");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    /// <summary>测试与调试：当前会话历史条数（用户+助手成对累积）。</summary>
    internal int ProjectAiHistoryCount => _projectAiHistory.Count;

    private void InsertOutlineReference()
    {
        if (string.IsNullOrWhiteSpace(_currentDocumentId))
        {
            StatusText = NoDocumentText;
            return;
        }
        IsEditMode = true;
        DocumentContent = AssembleDocumentContent() + Environment.NewLine + "@planning/outline.md";
        StatusText = OutlineText;
    }

    private void OpenImportPanel()
    {
        IsRightPanelOpen = true;
        IsNavTreeTab = true;
        IsImportPanelOpen = true;
        // 打开时若排序仍是默认 0，用树条目数作下一序号（浏览文件时也会再填）
        if (string.IsNullOrWhiteSpace(ImportOrder) || ImportOrder.Trim() == "0")
        {
            ImportOrder = Math.Max(0, WorksTreeNodes.Count).ToString();
        }
    }

    private static GridLength NormalizeRightPanelWidth(GridLength value)
    {
        if (value.IsStar)
        {
            return new GridLength(320);
        }
        var width = value.IsAuto ? 320 : value.Value;
        return new GridLength(Math.Clamp(width, MinRightPanelWidth, MaxRightPanelWidth));
    }

    private async Task QuickEditAsync()
    {
        var documentId = _currentDocumentId;
        var baseVersion = _currentDocumentVersion;
        var documentContent = AssembleDocumentContent();
        var instruction = QuickEditInstruction;
        var selection = RequestEditorSelection?.Invoke();
        var hasSelection = selection is { } currentSelection
                           && currentSelection.End > currentSelection.Start
                           && !string.IsNullOrWhiteSpace(currentSelection.Text);
        var selectionStart = hasSelection && selection is not null
            ? Math.Clamp(Math.Min(selection.Start, selection.End), 0, documentContent.Length)
            : 0;
        var selectionEnd = hasSelection && selection is not null
            ? Math.Clamp(Math.Max(selection.Start, selection.End), 0, documentContent.Length)
            : documentContent.Length;
        var selectedText = documentContent[selectionStart..selectionEnd];
        if (string.IsNullOrWhiteSpace(documentId)
            || string.IsNullOrWhiteSpace(selectedText)
            || string.IsNullOrWhiteSpace(instruction))
        {
            StatusText = QuickAiHint;
            return;
        }

        InvalidateQuickEditGeneration();
        var generation = Interlocked.Increment(ref _quickEditGeneration);
        var cancellation = new CancellationTokenSource();
        _quickEditGenerationCts = cancellation;
        IsQuickEditGenerating = true;
        try
        {
            var result = await _backend.QuickEditAsync(new QuickEditRequest(
                selectedText,
                instruction,
                documentId), cancellation.Token).ConfigureAwait(true);
            if (generation != Volatile.Read(ref _quickEditGeneration)
                || cancellation.IsCancellationRequested)
            {
                return;
            }

            var session = new QuickEditSession(
                documentId,
                baseVersion,
                documentContent,
                selectionStart,
                selectionEnd,
                result);
            if (!session.MatchesCurrent(
                    _currentDocumentId,
                    _currentDocumentVersion,
                    AssembleDocumentContent()))
            {
                StatusText = _displayNames.Text("ui.works.quick_edit.outdated");
                return;
            }

            _pendingQuickEdit = session;
            ApplyQuickEditCommand.NotifyCanExecuteChanged();
            var preview = QuickEditPreviewBuilder.Build(result.Diff);
            QuickEditDiff = preview.Text;
            StatusText = _displayNames.Text(preview.IsTruncated
                ? "ui.works.quick_edit.preview_truncated"
                : "ui.works.quick_edit.ready");
        }
        catch (OperationCanceledException) when (cancellation.IsCancellationRequested)
        {
            // 文档切换、继续编辑或新一轮生成已使本次结果失效。
        }
        catch (Exception ex)
        {
            if (generation == Volatile.Read(ref _quickEditGeneration))
            {
                StatusText = ex.Message;
            }
        }
        finally
        {
            if (generation == Volatile.Read(ref _quickEditGeneration))
            {
                _quickEditGenerationCts?.Dispose();
                _quickEditGenerationCts = null;
                IsQuickEditGenerating = false;
            }
        }
    }

    private void ApplyQuickEdit()
    {
        if (_pendingQuickEdit is null)
        {
            StatusText = QuickAiHint;
            return;
        }
        if (string.IsNullOrWhiteSpace(_currentDocumentId))
        {
            StatusText = NoDocumentText;
            return;
        }
        var documentContent = AssembleDocumentContent();
        if (!_pendingQuickEdit.TryApply(
                _currentDocumentId,
                _currentDocumentVersion,
                documentContent,
                out var updatedContent))
        {
            InvalidateQuickEditGeneration();
            StatusText = _displayNames.Text("ui.works.quick_edit.outdated");
            return;
        }

        DocumentContent = updatedContent;
        _quickEditUndo = new QuickEditUndoState(
            _currentDocumentId,
            updatedContent,
            documentContent);
        UndoQuickEditCommand.NotifyCanExecuteChanged();
        StatusText = _displayNames.Text("ui.works.quick_edit.applied_locally");
    }

    private void UndoQuickEdit()
    {
        if (_quickEditUndo is null
            || !_quickEditUndo.TryUndo(
                _currentDocumentId,
                AssembleDocumentContent(),
                out var restoredContent))
        {
            ClearQuickEditUndo();
            StatusText = _displayNames.Text("ui.works.quick_edit.undo_unavailable");
            return;
        }

        DocumentContent = restoredContent;
        ClearQuickEditUndo();
        StatusText = _displayNames.Text("ui.works.quick_edit.undone");
    }

    public async Task<bool> ConfirmLeaveIfNeededAsync()
    {
        if (!HasUnsavedChanges)
        {
            return true;
        }

        var choice = await DialogService.Current.ConfirmUnsavedLeaveAsync().ConfigureAwait(true);
        switch (choice)
        {
            case UnsavedLeaveChoice.Save:
                if (HasUnsavedDocumentChanges)
                {
                    await SaveAsync().ConfigureAwait(true);
                }
                return !HasUnsavedChanges;
            case UnsavedLeaveChoice.Discard:
                RestoreSnapshot();
                return true;
            default:
                return false;
        }
    }

    public async Task ReloadProjectDataAsync()
    {
        InvalidateQuickEditGeneration();
        ClearQuickEditUndo();
        await LoadWorksTreeAsync().ConfigureAwait(true);
        if (string.IsNullOrWhiteSpace(_currentDocumentId))
        {
            return;
        }

        try
        {
            _suppressDirtyTracking = true;
            var document = await _backend.GetDocumentContentDetailsAsync(_currentDocumentId).ConfigureAwait(true);
            DocumentContent = document.Content;
            _currentDocumentPath = document.Metadata.Path;
            _currentDocumentVersion = document.Metadata.Version;
            DocumentTitle = Path.GetFileNameWithoutExtension(document.Metadata.Path);
        }
        catch (Exception ex)
        {
            _currentDocumentId = string.Empty;
            _currentDocumentPath = string.Empty;
            _currentDocumentVersion = null;
            DocumentContent = string.Empty;
            DocumentTitle = NoDocumentText;
            StatusText = ex.Message;
        }
        finally
        {
            _suppressDirtyTracking = false;
        }
        OnCurrentDocumentChanged();
        CaptureSnapshot();
    }

    private void CaptureSnapshot()
    {
        _savedSnapshot = AssembleDocumentContent();
        _documentDirty = false;
        HasUnsavedChanges = false;
    }

    private void RestoreSnapshot()
    {
        _suppressDirtyTracking = true;
        try
        {
            DocumentContent = _savedSnapshot;
            InvalidateQuickEditGeneration();
            ClearQuickEditUndo();
            _documentDirty = false;
            RefreshDirtyState();
        }
        finally
        {
            _suppressDirtyTracking = false;
        }
    }

    private void RefreshDirtyState()
    {
        HasUnsavedChanges = HasUnsavedDocumentChanges;
    }

    private bool HasUnsavedDocumentChanges => _documentDirty || AssembleDocumentContent() != _savedSnapshot;

    private static string ShortValue(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return "-";
        }
        return value.Length <= 12 ? value : value[..12];
    }

    protected override void OnPropertyChanged(string? propertyName = null)
    {
        base.OnPropertyChanged(propertyName);
        if (!_suppressDirtyTracking
            && propertyName is nameof(DocumentContent))
        {
            MarkDocumentDirty();
        }
    }

    private IEnumerable<WorksTreeItemViewModel> FlattenTree(WorksTreeNode root)
    {
        foreach (var item in FlattenTree(root, 0))
        {
            yield return item;
        }
    }

    private IEnumerable<WorksTreeItemViewModel> FlattenTree(WorksTreeNode node, int depth)
    {
        var title = node.Title.StartsWith("ui.", StringComparison.Ordinal)
            ? _displayNames.Text(node.Title)
            : node.Title;
        yield return new WorksTreeItemViewModel(
            node.NodeId,
            title,
            node.Path,
            new string(' ', Math.Max(0, depth) * 2),
            () => _ = LoadDocumentAsync(node.Path, title));
        foreach (var child in node.Children)
        {
            foreach (var item in FlattenTree(child, depth + 1))
            {
                yield return item;
            }
        }
    }

    private Task LoadDocumentAsync(string path, string title)
    {
        return LoadDocumentAsync(new WorksTreeItemViewModel(string.Empty, title, path, string.Empty, () => { }));
    }

    private static string ProjectRelativePath(string path) =>
        ProjectPathHelper.ToProjectRelativePath(path);

}

public sealed record ExportFormatOption(string Value, string Label);

public sealed record EditorTextSelection(int Start, int End, string Text);

public sealed class DocumentBlockViewModel : ViewModelBase
{
    private readonly Action<DocumentBlockViewModel, string, string> _textChanged;
    private string _text;

    public DocumentBlockViewModel(
        string id,
        int index,
        string text,
        Action<DocumentBlockViewModel, string, string> textChanged)
    {
        Id = id;
        Index = index;
        _text = text;
        _textChanged = textChanged;
    }

    public string Id { get; }
    public int Index { get; }

    public string Text
    {
        get => _text;
        set
        {
            if (value == _text)
            {
                return;
            }
            var oldText = _text;
            if (SetProperty(ref _text, value))
            {
                _textChanged(this, oldText, value);
            }
        }
    }
}

public sealed class WorksTreeItemViewModel
{
    public WorksTreeItemViewModel(string nodeId, string title, string path, string indent, Action open)
    {
        NodeId = nodeId;
        Title = title;
        Path = path;
        Indent = indent;
        OpenCommand = new RelayCommand(open);
    }

    public string NodeId { get; }
    public string Title { get; }
    public string Path { get; }
    public string Indent { get; }
    public string DisplayTitle => $"{Indent}{Title}";
    public string DisplayPath => Path.Replace('\\', '/');
    public RelayCommand OpenCommand { get; }
}
