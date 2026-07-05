using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class WorksPageViewModel : ViewModelBase, IUnsavedChangesGuard
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private bool _isRightPanelOpen = true;
    private bool _isNavTreeTab = true;
    private string _documentContent = string.Empty;
    private string _statusText = string.Empty;
    private string _projectAiMessage = string.Empty;
    private string _projectAiAnswer;
    private string _savedSnapshot = string.Empty;
    private bool _hasUnsavedChanges;
    private bool _suppressDirtyTracking;
    private bool _isEditMode;

    public WorksPageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        _projectAiAnswer = displayNames.Text("ui.works.project_ai.empty");
        ToggleRightPanelCommand = new RelayCommand(() => IsRightPanelOpen = !IsRightPanelOpen);
        ShowNavTreeCommand = new RelayCommand(() => IsNavTreeTab = true);
        ShowProjectAiCommand = new RelayCommand(() => IsNavTreeTab = false);
        ImportCommand = new RelayCommand(() => _ = ImportAsync());
        ExportCommand = new RelayCommand(() => _ = ExportAsync());
        SaveCommand = new RelayCommand(() => _ = SaveAsync());
        ReadModeCommand = new RelayCommand(() => IsEditMode = false);
        EditModeCommand = new RelayCommand(() => IsEditMode = true);
        CopyCommand = new RelayCommand(() => StatusText = CtxCopyText);
        SelectAllCommand = new RelayCommand(() => StatusText = CtxSelectAllText);
        QuickAiCommand = new RelayCommand(() => StatusText = QuickAiHint);
        InsertOutlineCommand = new RelayCommand(() => StatusText = OutlineText);
        ToggleEditCommand = new RelayCommand(() => IsEditMode = !IsEditMode);
        SendProjectAiCommand = new RelayCommand(() => _ = SendProjectAiAsync());
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

    public RelayCommand ShowNavTreeCommand { get; }

    public RelayCommand ShowProjectAiCommand { get; }

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

    public bool IsEditMode
    {
        get => _isEditMode;
        set => SetProperty(ref _isEditMode, value);
    }

    public string DocumentContent
    {
        get => _documentContent;
        set => SetProperty(ref _documentContent, value);
    }

    public bool HasUnsavedChanges
    {
        get => _hasUnsavedChanges;
        private set => SetProperty(ref _hasUnsavedChanges, value);
    }

    public string StatusText
    {
        get => _statusText;
        set => SetProperty(ref _statusText, value);
    }

    public string ProjectAiMessage
    {
        get => _projectAiMessage;
        set => SetProperty(ref _projectAiMessage, value);
    }

    public string ProjectAiAnswer
    {
        get => _projectAiAnswer;
        set => SetProperty(ref _projectAiAnswer, value);
    }

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

    public string EmptyIndexText => _displayNames.Text("ui.works.empty_index");

    public string QuickAiHint => _displayNames.Text("ui.works.quick_ai_hint");

    public string ProjectAiPlaceholder => _displayNames.Text("ui.works.project_ai.placeholder");

    // 右键菜单文案（阅读/修改器）
    public string CtxCopyText => _displayNames.Text("ui.works.context.copy");
    public string CtxSelectAllText => _displayNames.Text("ui.works.context.select_all");
    public string CtxQuickAiText => _displayNames.Text("ui.works.context.quick_ai");
    public string CtxInsertOutlineText => _displayNames.Text("ui.works.context.insert_outline");
    public string CtxToggleEditText => _displayNames.Text("ui.works.context.toggle_edit");

    private async Task ImportAsync()
    {
        try
        {
            _suppressDirtyTracking = true;
            try
            {
                DocumentContent = await _backend.GetDocumentContentAsync("documents/sample.md").ConfigureAwait(true);
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

    private async Task SaveAsync()
    {
        try
        {
            await _backend.SaveDocumentContentAsync("documents/sample.md", DocumentContent).ConfigureAwait(true);
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
            var report = await _backend.ExportChaptersAsync(Array.Empty<string>(), "combined-markdown", "markdown").ConfigureAwait(true);
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
            var result = await _backend.ProjectAiChatAsync(ProjectAiMessage).ConfigureAwait(true);
            ProjectAiAnswer = result.Answer;
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
                await SaveAsync().ConfigureAwait(true);
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
            HasUnsavedChanges = false;
        }
        finally
        {
            _suppressDirtyTracking = false;
        }
    }

    private void RefreshDirtyState()
    {
        HasUnsavedChanges = DocumentContent != _savedSnapshot;
    }

    protected override void OnPropertyChanged(string? propertyName = null)
    {
        base.OnPropertyChanged(propertyName);
        if (!_suppressDirtyTracking && propertyName == nameof(DocumentContent))
        {
            RefreshDirtyState();
        }
    }
}
