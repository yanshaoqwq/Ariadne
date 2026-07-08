using System.Collections.ObjectModel;
using System.Text;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class WorksPageViewModel : ViewModelBase, IUnsavedChangesGuard
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private bool _isRightPanelOpen = true;
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
    private string _documentTitle;
    private string _importChapterId = string.Empty;
    private string _importChapterTitle = string.Empty;
    private string _importOrder = "0";
    private string _importSourcePath = string.Empty;
    private string _importTargetPath = string.Empty;
    private string _savedSnapshot = string.Empty;
    private bool _hasUnsavedChanges;
    private bool _suppressDirtyTracking;
    private bool _isEditMode;
    private TextRange? _pendingQuickEditRange;

    public WorksPageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        _projectAiAnswer = displayNames.Text("ui.works.project_ai.empty");
        _documentTitle = displayNames.Text("ui.works.no_document_selected");
        WorksTreeNodes = new ObservableCollection<WorksTreeItemViewModel>();
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
        set => SetProperty(ref _isRightPanelOpen, value);
    }

    public RelayCommand ToggleRightPanelCommand { get; }

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

    public ObservableCollection<ExportFormatOption> ExportFormats { get; }

    public bool IsWorksTreeEmpty => WorksTreeNodes.Count == 0;

    public bool IsEditMode
    {
        get => _isEditMode;
        set => SetProperty(ref _isEditMode, value);
    }

    public string DocumentContent
    {
        get => _documentContent;
        set
        {
            if (SetProperty(ref _documentContent, value))
            {
                OnPropertyChanged(nameof(DocumentBodyText));
                OnPropertyChanged(nameof(CharacterCountText));
                QuickAiCommand.NotifyCanExecuteChanged();
                if (!_suppressDirtyTracking)
                {
                    ClearPendingQuickEdit();
                }
            }
        }
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

    public string DocumentBodyText => string.IsNullOrWhiteSpace(DocumentContent)
        ? (string.IsNullOrWhiteSpace(_currentDocumentId)
            ? NoDocumentText
            : _displayNames.Text("ui.works.empty_document"))
        : DocumentContent;

    public string CharacterCountText => _displayNames.Format("ui.works.characters_count", new Dictionary<string, string>
    {
        ["count"] = DocumentContent.Length.ToString(),
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
               && !string.IsNullOrWhiteSpace(DocumentContent)
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
        if (_pendingQuickEdit is null && _pendingQuickEditRange is null && string.IsNullOrEmpty(QuickEditDiff))
        {
            return;
        }

        _pendingQuickEdit = null;
        _pendingQuickEditRange = null;
        QuickEditDiff = string.Empty;
        ApplyQuickEditCommand.NotifyCanExecuteChanged();
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
                DocumentContent = await _backend.GetDocumentContentByPathAsync(item.Path).ConfigureAwait(true);
                _currentDocumentId = nextDocumentId;
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
            await _backend.SaveDocumentContentAsync(_currentDocumentId, DocumentContent).ConfigureAwait(true);
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
        DocumentContent += Environment.NewLine + "@planning/outline.md";
        StatusText = OutlineText;
    }

    private void OpenImportPanel()
    {
        IsRightPanelOpen = true;
        IsNavTreeTab = true;
        IsImportPanelOpen = true;
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
                : DocumentContent;
            if (string.IsNullOrWhiteSpace(selectedText) || string.IsNullOrWhiteSpace(QuickEditInstruction))
            {
                StatusText = QuickAiHint;
                return;
            }
            _pendingQuickEditRange = hasSelection && selection is not null
                ? Utf8Range(DocumentContent, selection.Start, selection.End)
                : Utf8Range(DocumentContent, 0, DocumentContent.Length);
            var result = await _backend.QuickEditAsync(new QuickEditRequest(
                selectedText,
                QuickEditInstruction,
                string.IsNullOrWhiteSpace(_currentDocumentId) ? null : _currentDocumentId)).ConfigureAwait(true);
            _pendingQuickEdit = result;
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
            await _backend.ApplyQuickEditAsync(
                _currentDocumentId,
                null,
                DocumentContent,
                _pendingQuickEditRange ?? Utf8Range(DocumentContent, 0, DocumentContent.Length),
                _pendingQuickEdit).ConfigureAwait(true);
            _suppressDirtyTracking = true;
            try
            {
                DocumentContent = await _backend.GetDocumentContentAsync(_currentDocumentId).ConfigureAwait(true);
            }
            finally
            {
                _suppressDirtyTracking = false;
            }
            CaptureSnapshot();
            _pendingQuickEdit = null;
            _pendingQuickEditRange = null;
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
        _savedSnapshot = DocumentContent;
        RefreshDirtyState();
    }

    private void RestoreSnapshot()
    {
        _suppressDirtyTracking = true;
        try
        {
            DocumentContent = _savedSnapshot;
            ClearPendingQuickEdit();
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

    private bool HasUnsavedDocumentChanges => DocumentContent != _savedSnapshot;

    protected override void OnPropertyChanged(string? propertyName = null)
    {
        base.OnPropertyChanged(propertyName);
        if (!_suppressDirtyTracking
            && propertyName is nameof(DocumentContent))
        {
            RefreshDirtyState();
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
