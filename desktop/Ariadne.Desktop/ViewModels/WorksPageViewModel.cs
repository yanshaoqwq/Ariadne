using System.Collections.ObjectModel;
using System.Text;
using Avalonia.Controls;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class WorksPageViewModel : ViewModelBase, IUnsavedChangesGuard
{
    private const double MinRightPanelWidth = 280;
    private const double MaxRightPanelWidth = 520;
    private const double CollapsedRightPanelWidth = 24;

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
    private string _quickEditInstruction = string.Empty;
    private string _quickEditDiff = string.Empty;
    private string _exportFormat = "markdown";
    private string _currentDocumentId = string.Empty;
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
    private int _documentCharacterCount;
    private int _nextDocumentBlockId;
    private bool _isEditMode;
    private TextRange? _pendingQuickEditRange;
    private string? _pendingQuickEditBaseVersion;

    public WorksPageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        _projectAiAnswer = displayNames.Text("ui.works.project_ai.empty");
        _documentTitle = displayNames.Text("ui.works.no_document_selected");
        WorksTreeNodes = new ObservableCollection<WorksTreeItemViewModel>();
        DocumentBlocks = new ObservableCollection<DocumentBlockViewModel>();
        ToggleRightPanelCommand = new RelayCommand(() => IsRightPanelOpen = !IsRightPanelOpen);
        ShowNavTreeCommand = new RelayCommand(() => IsNavTreeTab = true);
        ShowProjectAiCommand = new RelayCommand(() => IsNavTreeTab = false);
        OpenImportPanelCommand = new RelayCommand(OpenImportPanel);
        ToggleImportPanelCommand = new RelayCommand(() => IsImportPanelOpen = !IsImportPanelOpen);
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
        ApplyQuickEditCommand = new RelayCommand(() => _ = ApplyQuickEditAsync(), () => _pendingQuickEdit is not null);
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
    public Action? RequestEditorCopy { get; set; }
    public Action? RequestEditorSelectAll { get; set; }
    public Func<EditorTextSelection>? RequestEditorSelection { get; set; }

    public ObservableCollection<WorksTreeItemViewModel> WorksTreeNodes { get; }

    public ObservableCollection<DocumentBlockViewModel> DocumentBlocks { get; }
    public bool HasDocumentBlocks => DocumentBlocks.Count > 0;

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
        private set => SetProperty(ref _hasUnsavedChanges, value);
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
                ClearPendingQuickEdit();
                QuickAiCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public string QuickEditDiff
    {
        get => _quickEditDiff;
        set => SetProperty(ref _quickEditDiff, value);
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

    public string DocumentBodyText => _documentCharacterCount == 0
        ? (string.IsNullOrWhiteSpace(_currentDocumentId)
            ? NoDocumentText
            : _displayNames.Text("ui.works.empty_document"))
        : AssembleDocumentContent();

    public string CharacterCountText => _displayNames.Format("ui.works.characters_count", new Dictionary<string, string>
    {
        ["count"] = _documentCharacterCount.ToString(),
    });

    public string EmptyIndexText => _displayNames.Text("ui.works.empty_index");

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
    public string QuickEditTitle => _displayNames.Text("ui.works.quick_edit.title");
    public string QuickEditPlaceholder => _displayNames.Text("ui.works.quick_edit.placeholder");
    public string QuickEditGenerateText => _displayNames.Text("ui.works.quick_edit.generate");
    public string QuickEditDiffText => _displayNames.Text("ui.works.quick_edit.diff");
    public string QuickEditApplyText => _displayNames.Text("ui.works.quick_edit.apply");

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
               && !string.IsNullOrWhiteSpace(QuickEditInstruction);
    }

    private bool CanSendProjectAi()
    {
        return !string.IsNullOrWhiteSpace(ProjectAiMessage);
    }

    private void OnCurrentDocumentChanged()
    {
        OnPropertyChanged(nameof(HasCurrentDocument));
        SaveCommand.NotifyCanExecuteChanged();
        InsertOutlineCommand.NotifyCanExecuteChanged();
        QuickAiCommand.NotifyCanExecuteChanged();
    }

    private void ClearPendingQuickEdit()
    {
        if (_pendingQuickEdit is null
            && _pendingQuickEditRange is null
            && _pendingQuickEditBaseVersion is null
            && string.IsNullOrEmpty(QuickEditDiff))
        {
            return;
        }

        _pendingQuickEdit = null;
        _pendingQuickEditRange = null;
        _pendingQuickEditBaseVersion = null;
        QuickEditDiff = string.Empty;
        ApplyQuickEditCommand.NotifyCanExecuteChanged();
    }

    private void ReplaceDocumentContent(string content)
    {
        _suppressDocumentBlockChanges = true;
        try
        {
            _documentContent = content;
            _documentCharacterCount = content.Length;
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
        }
        finally
        {
            _suppressDocumentBlockChanges = false;
        }

        OnPropertyChanged(nameof(DocumentContent));
        OnPropertyChanged(nameof(DocumentBodyText));
        OnPropertyChanged(nameof(CharacterCountText));
        OnPropertyChanged(nameof(HasDocumentBlocks));
        QuickAiCommand.NotifyCanExecuteChanged();
        if (!_suppressDirtyTracking)
        {
            ClearPendingQuickEdit();
        }
    }

    private void OnDocumentBlockTextChanged(DocumentBlockViewModel block, string oldText, string newText)
    {
        if (_suppressDocumentBlockChanges)
        {
            return;
        }

        _documentCharacterCount += newText.Length - oldText.Length;
        _documentContent = string.Empty;
        OnPropertyChanged(nameof(DocumentContent));
        OnPropertyChanged(nameof(DocumentBodyText));
        OnPropertyChanged(nameof(CharacterCountText));
        QuickAiCommand.NotifyCanExecuteChanged();
        if (!_suppressDirtyTracking)
        {
            ClearPendingQuickEdit();
            MarkDocumentDirty();
        }
    }

    private void MarkDocumentDirty()
    {
        _documentDirty = true;
        HasUnsavedChanges = true;
    }

    private string AssembleDocumentContent()
    {
        if (DocumentBlocks.Count == 0)
        {
            return string.Empty;
        }

        var builder = new StringBuilder(_documentCharacterCount);
        foreach (var block in DocumentBlocks.OrderBy(block => block.Index))
        {
            builder.Append(block.Text);
        }
        _documentContent = builder.ToString();
        return _documentContent;
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
        const int targetBlockSize = 4_000;
        const int hardBlockSize = 6_000;
        if (string.IsNullOrEmpty(content))
        {
            yield break;
        }

        var start = 0;
        while (start < content.Length)
        {
            var remaining = content.Length - start;
            if (remaining <= hardBlockSize)
            {
                yield return content[start..];
                yield break;
            }

            var limit = Math.Min(content.Length, start + hardBlockSize);
            var preferredStart = Math.Min(content.Length, start + targetBlockSize);
            var split = content.LastIndexOf("\n\n", limit - 1, limit - start, StringComparison.Ordinal);
            if (split < preferredStart)
            {
                split = content.LastIndexOf('\n', limit - 1, limit - start);
            }
            if (split < preferredStart)
            {
                split = start + targetBlockSize;
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
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
        finally
        {
            OnPropertyChanged(nameof(IsWorksTreeEmpty));
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

            _suppressDirtyTracking = true;
            try
            {
                var document = await _backend.GetDocumentContentDetailsByPathAsync(item.Path).ConfigureAwait(true);
                DocumentContent = document.Content;
                _currentDocumentId = nextDocumentId;
                _currentDocumentVersion = document.Metadata.Version;
                OnCurrentDocumentChanged();
                ClearPendingQuickEdit();
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
            _currentDocumentVersion = report.Metadata.Version;
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
            StatusText = _displayNames.Format("ui.works.export_done", new Dictionary<string, string>
            {
                ["format"] = report.Format,
                ["path"] = report.StorageUri,
            });
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
            var result = await _backend.ProjectAiChatAsync(ProjectAiMessage).ConfigureAwait(true);
            ProjectAiAnswer = result.Answer;
            StatusText = _displayNames.Text("ui.common.configured");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

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
        try
        {
            var selection = RequestEditorSelection?.Invoke();
            var hasSelection = selection is { } currentSelection
                               && currentSelection.End > currentSelection.Start
                               && !string.IsNullOrWhiteSpace(currentSelection.Text);
            var selectedText = hasSelection && selection is not null
                ? selection.Text
                : AssembleDocumentContent();
            if (string.IsNullOrWhiteSpace(selectedText) || string.IsNullOrWhiteSpace(QuickEditInstruction))
            {
                StatusText = QuickAiHint;
                return;
            }
            var documentContent = AssembleDocumentContent();
            _pendingQuickEditRange = hasSelection && selection is not null
                ? Utf8Range(documentContent, selection.Start, selection.End)
                : Utf8Range(documentContent, 0, documentContent.Length);
            var result = await _backend.QuickEditAsync(new QuickEditRequest(
                selectedText,
                QuickEditInstruction,
                string.IsNullOrWhiteSpace(_currentDocumentId) ? null : _currentDocumentId)).ConfigureAwait(true);
            _pendingQuickEdit = result;
            _pendingQuickEditBaseVersion = _currentDocumentVersion;
            ApplyQuickEditCommand.NotifyCanExecuteChanged();
            QuickEditDiff = result.Diff;
            StatusText = QuickEditDiffText;
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private QuickEditResult? _pendingQuickEdit;

    private async Task ApplyQuickEditAsync()
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
        try
        {
            var documentContent = AssembleDocumentContent();
            var report = await _backend.ApplyQuickEditAsync(
                _currentDocumentId,
                _pendingQuickEditBaseVersion,
                documentContent,
                _pendingQuickEditRange ?? Utf8Range(documentContent, 0, documentContent.Length),
                _pendingQuickEdit).ConfigureAwait(true);
            _suppressDirtyTracking = true;
            try
            {
                var document = await _backend.GetDocumentContentDetailsAsync(_currentDocumentId).ConfigureAwait(true);
                DocumentContent = document.Content;
                _currentDocumentVersion = document.Metadata.Version;
            }
            finally
            {
                _suppressDirtyTracking = false;
            }
            if (report.Metadata is not null)
            {
                _currentDocumentVersion = report.Metadata.Version;
            }
            CaptureSnapshot();
            _pendingQuickEdit = null;
            _pendingQuickEditRange = null;
            _pendingQuickEditBaseVersion = null;
            ApplyQuickEditCommand.NotifyCanExecuteChanged();
            StatusText = _displayNames.Text("ui.common.configured");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
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
            ClearPendingQuickEdit();
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
        yield return new WorksTreeItemViewModel(
            node.NodeId,
            node.Title,
            node.Path,
            new string(' ', Math.Max(0, depth) * 2),
            () => _ = LoadDocumentAsync(node.Path, node.Title));
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

    private static string ProjectRelativePath(string path)
    {
        var normalized = path.Replace('\\', '/');
        foreach (var marker in new[] { "/documents/", "/planning/", "/workflows/" })
        {
            var index = normalized.IndexOf(marker, StringComparison.OrdinalIgnoreCase);
            if (index >= 0)
            {
                return normalized[(index + 1)..];
            }
        }
        return normalized.TrimStart('/');
    }

    private static TextRange Utf8Range(string document, int start, int end)
    {
        var length = document.Length;
        var startIndex = Math.Clamp(Math.Min(start, end), 0, length);
        var endIndex = Math.Clamp(Math.Max(start, end), 0, length);
        return new TextRange(
            Encoding.UTF8.GetByteCount(document[..startIndex]),
            Encoding.UTF8.GetByteCount(document[..endIndex]));
    }
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
    public RelayCommand OpenCommand { get; }
}
