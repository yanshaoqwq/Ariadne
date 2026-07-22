using System.Collections.ObjectModel;
using System.Globalization;
using Avalonia;
using Avalonia.Media;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;


namespace Ariadne.Desktop.ViewModels;

public sealed class SettingsPageViewModel : ViewModelBase, IUnsavedChangesGuard, IProjectDataReloadable, IUiPreferencesAware, ILocalizedUiAware
{
    private const string GeneralSection = "general";
    private const string ModelsSection = "models";
    private const string PresetsSection = "presets";
    private const string TemplateRepositorySection = "template_repository";
    private const string AutomationSection = "automation";
    private const string PermissionsSection = "permissions";
    private const string PersonalizationSection = "personalization";
    private const string AppRuntimeSection = "app_runtime";
    private const string MiscSection = "misc";
    private static readonly string[] LocalizedPropertyNames =
    {
        nameof(Title),
        nameof(GeneralTitle),
        nameof(GeneralScopeHelpText),
        nameof(ModelsTitle),
        nameof(PresetsTitle),
        nameof(AutomationTitle),
        nameof(AutomationScopeHelpText),
        nameof(PermissionsTitle),
        nameof(PersonalizationTitle),
        nameof(PersonalizationScopeHelpText),
        nameof(MiscTitle),
        nameof(AppRuntimeScopeHelpText),
        nameof(RetrievalScopeHelpText),
        nameof(ProjectNameLabel),
        nameof(DocumentsDirLabel),
        nameof(WorkflowsDirLabel),
        nameof(SkillsDirLabel),
        nameof(ExportsDirLabel),
        nameof(ProjectMemoryLabel),
        nameof(ProjectMemoryPlaceholder),
        nameof(SaveGeneralText),
        nameof(ProviderIdLabel),
        nameof(ProviderTypeLabel),
        nameof(ProviderDisplayNameLabel),
        nameof(BaseUrlLabel),
        nameof(BaseUrlPlaceholder),
        nameof(ProviderEnabledText),
        nameof(MakeDefaultLlmText),
        nameof(MakeDefaultEmbeddingText),
        nameof(MakeDefaultRerankerText),
        nameof(MakeDefaultSearchText),
        nameof(AvailableModelsText),
        nameof(ManualModelsText),
        nameof(ModelsTextLabel),
        nameof(ModelsPlaceholder),
        nameof(EmbeddingModelLabel),
        nameof(EmbeddingModelPlaceholder),
        nameof(ApiKeyLabel),
        nameof(ApiKeyPlaceholder),
        nameof(SaveModelText),
        nameof(SaveKeyText),
        nameof(RemoveProviderText),
        nameof(RefreshText),
        nameof(ProviderStatusLabel),
        nameof(AddProviderText),
        nameof(ProviderListTitle),
        nameof(ProviderEditorTitle),
        nameof(ProviderScopeHelpText),
        nameof(PresetNodeTypeLabel),
        nameof(PresetNodeModelLabel),
        nameof(PresetNodeTimeoutLabel),
        nameof(PresetNodeBudgetLabel),
        nameof(PresetAccessTitle),
        nameof(PresetToolsTitle),
        nameof(PresetScopeHelpText),
        nameof(InheritNodePermissionsText),
        nameof(DefaultModelLabel),
        nameof(DefaultTimeoutLabel),
        nameof(DefaultBudgetLabel),
        nameof(TemplateRepositoryLabel),
        nameof(OpenTemplateMarketText),
        nameof(SavePresetsText),
        nameof(SaveTemplateRepositoryText),
        nameof(BudgetLabel),
        nameof(BudgetHelpText),
        nameof(PreauthorizedBudgetLabel),
        nameof(PreauthorizedHelpText),
        nameof(SpentLabel),
        nameof(NormalModeLabel),
        nameof(AutoModePolicyLabel),
        nameof(ApprovalPromptLabel),
        nameof(ApprovalPromptPlaceholder),
        nameof(ConfirmationPolicyHelpText),
        nameof(PolicyAllowText),
        nameof(PolicyReviewText),
        nameof(PolicyAutoOnText),
        nameof(PolicyAutoOffText),
        nameof(BrowseFolderText),
        nameof(WorkflowLimitLabel),
        nameof(WorkflowDefaultTimeoutLabel),
        nameof(MaxLoopIterationsLabel),
        nameof(MaxToolRoundsLabel),
        nameof(CheckpointEnabledLabel),
        nameof(RuntimeAutosaveLabel),
        nameof(SaveAutomationText),
        nameof(AllowNetworkText),
        nameof(AllowWebSearchText),
        nameof(AllowHttpSkillText),
        nameof(AllowWasmNetworkText),
        nameof(AllowSecretReadText),
        nameof(GlobalDefaultsHelpText),
        nameof(PermissionsScopeHelpText),
        nameof(InheritGlobalText),
        nameof(ToolControlsLabel),
        nameof(DangerToolsTitle),
        nameof(DangerToolsHelp),
        nameof(SafeToolsTitle),
        nameof(ReadableRootsLabel),
        nameof(WritableRootsLabel),
        nameof(PathPlaceholder),
        nameof(SavePermissionsText),
        nameof(ThemeLabel),
        nameof(ThemePaletteHelpText),
        nameof(ThemeCustomThreeLabel),
        nameof(ThemeCustomThreeHint),
        nameof(ThemeMainColorLabel),
        nameof(ThemeSurfaceColorLabel),
        nameof(ThemeBrandColorLabel),
        nameof(ActiveThemeColorLabel),
        nameof(ThemeFollowSystemColorsText),
        nameof(ThemeEditDayText),
        nameof(ThemeEditNightText),
        nameof(ColorMapHintText),
        nameof(ProjectSectionTitle),
        nameof(DirectoriesSectionTitle),
        nameof(ProjectMemorySectionTitle),
        nameof(ProviderSectionTitle),
        nameof(AvailableModelsSectionTitle),
        nameof(EmbeddingSectionTitle),
        nameof(ManualModelsSectionTitle),
        nameof(NodePresetsSectionTitle),
        nameof(DefaultsSectionTitle),
        nameof(TemplatesSectionTitle),
        nameof(BudgetSectionTitle),
        nameof(ConfirmationsSectionTitle),
        nameof(RuntimeSectionTitle),
        nameof(CapabilitiesSectionTitle),
        nameof(ToolControlsSectionTitle),
        nameof(PathsSectionTitle),
        nameof(ThemeSectionTitle),
        nameof(WorkspaceSectionTitle),
        nameof(RetrievalSectionTitle),
        nameof(AppRuntimeSectionTitle),
        nameof(GitSectionTitle),
        nameof(LanguageSectionTitle),
        nameof(DiagnosticsSectionTitle),
        nameof(GitAutoColorLabel),
        nameof(GitManualColorLabel),
        nameof(ProjectPanelVisibleText),
        nameof(ReduceMotionText),
        nameof(ReduceMotionHintText),
        nameof(SavePersonalizationText),
        nameof(RagLabel),
        nameof(VectorEnabledText),
        nameof(VectorBackendLabel),
        nameof(VectorCollectionLabel),
        nameof(VectorDimensionsLabel),
        nameof(QdrantHostLabel),
        nameof(QdrantPortLabel),
        nameof(QdrantDataDirLabel),
        nameof(QdrantBinaryPathLabel),
        nameof(QdrantStartupTimeoutLabel),
        nameof(SaveAppRuntimeText),
        nameof(RerankerEnabledText),
        nameof(ChunkSizeLabel),
        nameof(ChunkOverlapLabel),
        nameof(GitLabel),
        nameof(TrackDocumentsText),
        nameof(TrackWorkflowsText),
        nameof(TrackSkillsText),
        nameof(TrackConfigText),
        nameof(IgnoredPathsLabel),
        nameof(IgnoredPathsPlaceholder),
        nameof(SaveMiscText),
        nameof(LanguageLabel),
        nameof(TutorialText),
        nameof(OpenTutorialText),
        nameof(DiagnosticsLabel),
        nameof(DiagnosticsStatusText),
    };

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private readonly Func<Task>? _openTemplateMarket;
    private readonly Func<UiPreferences, Task> _saveUiPreferences;
    private readonly SettingsDraftState _draftState = new();
    private SettingsTabViewModel _selectedTab;
    private SettingsSectionNavigationItemViewModel _selectedSectionNavigationItem;
    private string _selectedLanguage;
    private string _statusText;
    private bool _isLoading;
    private bool _hasUnsavedChanges;
    private PendingSettingsNavigation? _pendingNavigation;
    private Task _navigationSelectionTask = Task.CompletedTask;
    private bool _suppressDirtyTracking;
    private bool _suppressProviderSelectionChange;
    private bool _providerRemovalInProgress;
    private string? _pendingProviderSelectionId;
    private Task _providerSelectionTask = Task.CompletedTask;
    private readonly RequestGenerationSession _providerModelRefreshSession = new();

    private readonly record struct PendingSettingsNavigation(
        SettingsTabViewModel Tab,
        SettingsSectionNavigationItemViewModel? Section);

    private sealed record PreparedSettingsCommit(string Section, Func<Task<bool>> Save);

    private int _schemaVersion = 1;
    private string _projectRoot = string.Empty;
    private string _projectName = string.Empty;
    private string _locale = string.Empty;
    private string _documentsDir = string.Empty;
    private string _workflowsDir = string.Empty;
    private string _skillsDir = string.Empty;
    private string _exportsDir = string.Empty;
    private string _projectMemory = string.Empty;

    private string _providerId = "openai";
    private string _providerType = "open_ai";
    private string _providerDisplayName = "OpenAI";
    private string _providerBaseUrl = string.Empty;
    private bool _providerEnabled = true;
    private bool _makeDefaultLlm = true;
    private bool _makeDefaultEmbedding;
    private bool _makeDefaultReranker;
    private bool _makeDefaultSearch;
    private string _apiKey = string.Empty;
    private string _modelsText = "gpt-4.1-mini,llm,,,,";
    private string _embeddingModelId = string.Empty;
    private bool _manualModelsVisible;
    private string _providerStatus = string.Empty;
    private ProviderConfigStatus? _providerConfig;
    private ProviderOptionViewModel? _selectedProviderOption;

    private string _defaultProviderId = string.Empty;
    private string _defaultModelId = "gpt-4.1-mini";
    private WorkflowModelOption? _selectedDefaultModelOption;
    // Author-facing timeout fields hold **seconds** (same unit as Workspace); convert to ms at save.
    private string _defaultTimeoutMs = "300";
    private string _defaultBudgetUsd = "0";
    private string _templateRepositoryBaseUrl = string.Empty;

    private string _budgetUsd = "0";
    private string _preauthorizedUsd = "0";
    private string _spentText = "$0.00";
    private double _spentUsd;
    private string _workflowDefaultTimeoutMs = "300";
    private string _maxLoopIterations = "5";
    private string _maxToolRounds = "8";
    private bool _checkpointEnabled = true;
    private string _runtimeAutosaveMs = "5000";

    private bool _allowNetwork;
    private bool _allowWebSearch;
    private bool _allowHttpSkill;
    private bool _allowWasmNetwork;
    private bool _allowSecretRead;
    private string _readableRootsText = string.Empty;
    private string _writableRootsText = string.Empty;

    private string _theme = "system";
    private string _themeMainLight = "#F6F7F6";
    private string _themeSurfaceLight = "#FFFFFF";
    private string _themeBrandLight = "#356F68";
    private string _themeMainDark = "#121417";
    private string _themeSurfaceDark = "#1B1F23";
    private string _themeBrandDark = "#70B8AC";
    private bool _themeFollowSystemColors = true;
    private bool _editingNightThemeColors;
    private string _gitAutoColor = "#8a8f98";
    private string _gitManualColor = "#f59e0b";
    private bool _projectPanelVisible = true;
    private bool _reduceMotion;
    private UiPreferences? _uiPreferences;

    private string _vectorBackend = "qdrant_sidecar";
    private bool _vectorEnabled;
    private string _vectorCollection = "ariadne_chunks";
    private string _vectorDimensions = "1536";
    private string _qdrantHost = "127.0.0.1";
    private string _qdrantPort = "6333";
    private string _qdrantDataDir = ".indexes/qdrant";
    private string _qdrantBinaryPath = "qdrant";
    private string _qdrantStartupTimeoutMs = "10000";
    private string _fullTextBackend = "tantivy";
    private string _fullTextIndexDir = ".indexes/tantivy";
    private bool _rerankerEnabled;
    private string _chunkSizeChars = "2000";
    private string _chunkOverlapChars = "200";
    private int _ragSchemaVersion = 1;
    private int _workflowSchemaVersion = 1;
    private int _gitSchemaVersion = 1;
    private bool _trackDocuments = true;
    private bool _trackWorkflows = true;
    private bool _trackSkills = true;
    private bool _trackNonSensitiveConfig = true;
    private string _ignoredPathsText = string.Empty;
    private string _diagnosticsStatus = string.Empty;
    private readonly ProjectAutomationState _projectAutomation;

    public SettingsPageViewModel(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        Func<Task>? openTemplateMarket = null,
        Func<UiPreferences, Task>? saveUiPreferences = null,
        ProjectAutomationState? projectAutomation = null)
    {
        _displayNames = displayNames;
        _backend = backend;
        _projectAutomation = projectAutomation ?? new ProjectAutomationState(displayNames, backend);
        _openTemplateMarket = openTemplateMarket;
        _saveUiPreferences = saveUiPreferences ?? (preferences => _backend.SaveUiPreferencesAsync(preferences));
        _selectedLanguage = _displayNames.NormalizeAvailableLanguage(displayNames.CurrentLanguage);
        _statusText = displayNames.Text("ui.common.loading");

        LanguageOptions = new ObservableCollection<LanguageOption>(
            displayNames.AvailableLanguages.Select(code => new LanguageOption(code, displayNames.LanguageLabel(code))));

        VectorBackendOptions = new ObservableCollection<SettingsValueOption>
        {
            new("qdrant_sidecar", displayNames.Text("ui.settings.misc.vector_backend.sidecar")),
            new("external_qdrant", displayNames.Text("ui.settings.misc.vector_backend.external")),
        };

        ProviderTypeOptions = new ObservableCollection<SettingsValueOption>
        {
            new("open_ai", displayNames.Text("ui.settings.models.provider_type.open_ai")),
            new("anthropic", displayNames.Text("ui.settings.models.provider_type.anthropic")),
            new("gemini", displayNames.Text("ui.settings.models.provider_type.gemini")),
            new("open_ai_compatible", displayNames.Text("ui.settings.models.provider_type.open_ai_compatible")),
            new("local", displayNames.Text("ui.settings.models.provider_type.local")),
        };

        ThemeOptions = new ObservableCollection<ThemeOption>(
            ThemeCatalog.All.Select(palette => CreateThemeOption(palette, displayNames)));
        ThemeGroups = new ObservableCollection<ThemeGroupViewModel>(
            ThemeOptions.GroupBy(o => o.GroupTitle)
                .Select(g => new ThemeGroupViewModel(g.Key, g)));
        ConfirmationPolicies = new ObservableCollection<ConfirmationPolicyViewModel>();
        ConfirmationPolicyGroups = new ObservableCollection<ConfirmationPolicyGroupViewModel>();
        NodePresets = new ObservableCollection<NodeTypePresetViewModel>();
        ProviderOptions = new ObservableCollection<ProviderOptionViewModel>();
        AvailableModels = new ObservableCollection<ModelOptionViewModel>();
        AvailableLlmModelOptions = new ObservableCollection<WorkflowModelOption>();
        ToolControlGroups = new ObservableCollection<ToolControlGroupViewModel>();
        ScopedPermissionProfiles = new ObservableCollection<PermissionScopeProfileViewModel>();
        // 先建色图集合，再挂编辑器回调（回调里会同步选中态）
        GitAutoColorSwatches = new ObservableCollection<ColorSwatchItemViewModel>();
        GitManualColorSwatches = new ObservableCollection<ColorSwatchItemViewModel>();
        ColorChannelEditor gitAutoEditor = null!;
        ColorChannelEditor gitManualEditor = null!;
        gitAutoEditor = new ColorChannelEditor(() =>
        {
            OnPropertyChanged(nameof(GitAutoColor));
            SyncGitColorSwatchSelection(GitAutoColorSwatches, gitAutoEditor.ToHexValue());
            if (!_suppressDirtyTracking)
            {
                UpdateDirtyState();
            }
        });
        gitManualEditor = new ColorChannelEditor(() =>
        {
            OnPropertyChanged(nameof(GitManualColor));
            SyncGitColorSwatchSelection(GitManualColorSwatches, gitManualEditor.ToHexValue());
            if (!_suppressDirtyTracking)
            {
                UpdateDirtyState();
            }
        });
        GitAutoColorEditor = gitAutoEditor;
        GitManualColorEditor = gitManualEditor;
        // 个性化色图：色相×深浅点选（非 RGB 滑条）
        foreach (var item in BuildColorSwatchCollection(hex => GitAutoColor = hex))
        {
            GitAutoColorSwatches.Add(item);
        }
        foreach (var item in BuildColorSwatchCollection(hex => GitManualColor = hex))
        {
            GitManualColorSwatches.Add(item);
        }
        SyncGitColorSwatchSelection(GitAutoColorSwatches, gitAutoEditor.ToHexValue());
        SyncGitColorSwatchSelection(GitManualColorSwatches, gitManualEditor.ToHexValue());

        // 主题三色：主底 / 表面 / 强调 + 共享色图
        ThemeMainColorEditor = new ColorChannelEditor(() => OnThemeCustomColorChanged(ThemeColorChannel.Main));
        ThemeSurfaceColorEditor = new ColorChannelEditor(() => OnThemeCustomColorChanged(ThemeColorChannel.Surface));
        ThemeBrandColorEditor = new ColorChannelEditor(() => OnThemeCustomColorChanged(ThemeColorChannel.Brand));
        ThemeColorSwatches = new ObservableCollection<ColorSwatchItemViewModel>();
        foreach (var item in BuildColorSwatchCollection(OnThemeColorSwatchPicked))
        {
            ThemeColorSwatches.Add(item);
        }
        SeedThemeColorsFromPalette(ThemeCatalog.Resolve(Theme), force: true);

        Tabs = new ObservableCollection<SettingsTabViewModel>(
            SettingsNavigationCatalog.Tabs.Select(definition =>
                CreateTab(definition.Id, definition.DisplayNameKey)));
        _selectedTab = Tabs[0];
        _selectedTab.IsSelected = true;
        SectionIndexItems = new ObservableCollection<SettingsSectionNavigationItemViewModel>(
            SettingsNavigationCatalog.Sections.Select(definition =>
                new SettingsSectionNavigationItemViewModel(
                    definition.Id,
                    definition.TabId,
                    definition.AnchorName,
                    _displayNames.Text(definition.DisplayNameKey))));
        _selectedSectionNavigationItem = SectionIndexItems[0];

        SaveGeneralCommand = new RelayCommand(() => _ = SaveGeneralAsync(), () => CanSave(GeneralSection));
        RefreshModelsCommand = new RelayCommand(() => _ = FetchModelsAsync(), CanUsePersistedProvider);
        SaveModelCommand = new RelayCommand(() => _ = SaveModelAsync(), () => CanSave(ModelsSection));
        SaveProviderKeyCommand = new RelayCommand(() => _ = SaveProviderKeyAsync(), CanUsePersistedProvider);
        RemoveProviderCommand = new RelayCommand(() => _ = RemoveProviderAsync(), CanUsePersistedProvider);
        AddProviderCommand = new RelayCommand(() => _ = AddProviderDraftAsync(), () => CanSave(ModelsSection));
        SavePresetsCommand = new RelayCommand(() => _ = SavePresetsAsync(), () => CanSave(PresetsSection));
        SaveTemplateRepositoryCommand = new RelayCommand(
            () => _ = SaveTemplateRepositoryAsync(),
            () => CanSave(TemplateRepositorySection));
        OpenTemplateMarketCommand = new RelayCommand(() => _ = OpenTemplateMarketAsync());
        SaveAutomationCommand = new RelayCommand(() => _ = SaveAutomationAsync(), () => CanSave(AutomationSection));
        SavePermissionsCommand = new RelayCommand(() => _ = SavePermissionsAsync(), () => CanSave(PermissionsSection));
        SavePersonalizationCommand = new RelayCommand(() => _ = SavePersonalizationAsync(), () => CanSave(PersonalizationSection));
        SaveAppRuntimeCommand = new RelayCommand(() => _ = SaveAppRuntimeAsync(), () => CanSave(AppRuntimeSection));
        SaveMiscCommand = new RelayCommand(() => _ = SaveMiscAsync(), () => CanSave(MiscSection));
        ShowTutorialCommand = new RelayCommand(() => _ = ShowTutorialAsync());
        BrowseDocumentsDirCommand = new RelayCommand(() => _ = BrowseProjectDirectoryAsync(value => DocumentsDir = value));
        BrowseWorkflowsDirCommand = new RelayCommand(() => _ = BrowseProjectDirectoryAsync(value => WorkflowsDir = value));
        BrowseSkillsDirCommand = new RelayCommand(() => _ = BrowseProjectDirectoryAsync(value => SkillsDir = value));
        BrowseExportsDirCommand = new RelayCommand(() => _ = BrowseProjectDirectoryAsync(value => ExportsDir = value));
        BrowseReadableRootsCommand = new RelayCommand(() => _ = BrowseIntoAsync(AppendReadableRoot));
        BrowseWritableRootsCommand = new RelayCommand(() => _ = BrowseIntoAsync(AppendWritableRoot));
        SelectThemeMainChannelCommand = new RelayCommand(() => SetActiveThemeColorChannel(ThemeColorChannel.Main));
        SelectThemeSurfaceChannelCommand = new RelayCommand(() => SetActiveThemeColorChannel(ThemeColorChannel.Surface));
        SelectThemeBrandChannelCommand = new RelayCommand(() => SetActiveThemeColorChannel(ThemeColorChannel.Brand));
        SelectThemeEditDayCommand = new RelayCommand(() => SetEditingNightThemeColors(false));
        SelectThemeEditNightCommand = new RelayCommand(() => SetEditingNightThemeColors(true));

    }

    private void AppendReadableRoot(string path) =>
        ReadableRootsText = AppendPathLine(ReadableRootsText, path);

    private void AppendWritableRoot(string path) =>
        WritableRootsText = AppendPathLine(WritableRootsText, path);

    /// <summary>权限根列表：浏览后追加一行，避免作者手敲绝对路径。</summary>
    public static string AppendPathLine(string existing, string path)
    {
        var line = (path ?? string.Empty).Trim();
        if (string.IsNullOrWhiteSpace(line))
        {
            return existing ?? string.Empty;
        }

        var current = existing ?? string.Empty;
        var lines = current
            .Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries)
            .Select(l => l.Trim())
            .Where(l => !string.IsNullOrWhiteSpace(l))
            .ToList();
        if (lines.Any(l => SettingsInputValidation.PathComparer.Equals(l, line)))
        {
            return string.Join(Environment.NewLine, lines);
        }

        lines.Add(line);
        return string.Join(Environment.NewLine, lines);
    }

    private Func<string?, Task<string?>>? _folderPickerWithTitle;

    public void SetFolderPicker(Func<Task<string?>> picker) =>
        _folderPickerWithTitle = _ => picker();

    public void SetFolderPicker(Func<string?, Task<string?>> picker) =>
        _folderPickerWithTitle = picker;

    public void ClearFolderPicker(Func<string?, Task<string?>> picker)
    {
        if (_folderPickerWithTitle == picker)
        {
            _folderPickerWithTitle = null;
        }
    }

    private async Task BrowseIntoAsync(Action<string> assign)
    {
        if (_folderPickerWithTitle is null)
        {
            StatusText = _displayNames.Text("ui.settings.browse_unavailable");
            return;
        }
        try
        {
            var path = await _folderPickerWithTitle(
                _displayNames.Text("ui.settings.browse_folder_title")).ConfigureAwait(true);
            if (!string.IsNullOrWhiteSpace(path))
            {
                assign(path);
            }
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private async Task BrowseProjectDirectoryAsync(Action<string> assign)
    {
        if (_folderPickerWithTitle is null)
        {
            StatusText = _displayNames.Text("ui.settings.browse_unavailable");
            return;
        }
        try
        {
            var path = await _folderPickerWithTitle(
                _displayNames.Text("ui.settings.browse_folder_title")).ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(path))
            {
                return;
            }
            if (!ProjectPathHelper.TryMakeRelativeToProjectRoot(path, _projectRoot, out var relative)
                || string.Equals(relative, ".", StringComparison.Ordinal))
            {
                StatusText = _displayNames.Format(
                    "ui.settings.directory_outside_project",
                    new Dictionary<string, string>
                    {
                        ["path"] = path,
                        ["root"] = _projectRoot,
                    });
                return;
            }
            assign(relative);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    public string Title => _displayNames.Text("ui.settings.title");
    public string StatusText { get => _statusText; set => SetProperty(ref _statusText, value); }
    public bool IsLoading { get => _isLoading; private set => SetProperty(ref _isLoading, value); }
    public bool HasUnsavedChanges
    {
        get => _hasUnsavedChanges;
        private set => SetProperty(ref _hasUnsavedChanges, value);
    }

    public ObservableCollection<SettingsTabViewModel> Tabs { get; }
    public ObservableCollection<SettingsSectionNavigationItemViewModel> SectionIndexItems { get; }
    public event EventHandler<SettingsSectionNavigationRequest>? ScrollToSectionRequested;

    internal int SectionNavigationSubscriberCountForTests =>
        ScrollToSectionRequested?.GetInvocationList().Length ?? 0;

    internal bool HasFolderPickerForTests => _folderPickerWithTitle is not null;

    public SettingsTabViewModel SelectedTab
    {
        get => _selectedTab;
        private set
        {
            if (SetProperty(ref _selectedTab, value))
            {
                OnPropertyChanged(nameof(IsGeneralSelected));
                OnPropertyChanged(nameof(IsModelsSelected));
                OnPropertyChanged(nameof(IsPresetsSelected));
                OnPropertyChanged(nameof(IsAutomationSelected));
                OnPropertyChanged(nameof(IsPermissionsSelected));
                OnPropertyChanged(nameof(IsPersonalizationSelected));
                OnPropertyChanged(nameof(IsMiscSelected));
                OnPropertyChanged(nameof(NavigationSelection));
            }
        }
    }

    public SettingsTabViewModel NavigationSelection
    {
        get => SelectedTab;
        set
        {
            if (value is null)
            {
                return;
            }
            if (ReferenceEquals(value, SelectedTab))
            {
                if (!_navigationSelectionTask.IsCompleted)
                {
                    _pendingNavigation = new PendingSettingsNavigation(
                        SelectedTab,
                        _selectedSectionNavigationItem);
                }
                OnPropertyChanged(nameof(NavigationSelection));
                return;
            }
            _ = QueueNavigationAsync(value, null);
        }
    }

    public SettingsSectionNavigationItemViewModel SectionNavigationSelection
    {
        get => _selectedSectionNavigationItem;
        set
        {
            if (value is null)
            {
                return;
            }
            if (ReferenceEquals(value, _selectedSectionNavigationItem))
            {
                if (!_navigationSelectionTask.IsCompleted)
                {
                    _pendingNavigation = new PendingSettingsNavigation(SelectedTab, value);
                }
                OnPropertyChanged(nameof(SectionNavigationSelection));
                return;
            }
            var tab = Tabs.FirstOrDefault(candidate =>
                string.Equals(candidate.Id, value.TabId, StringComparison.Ordinal));
            if (tab is not null)
            {
                _ = QueueNavigationAsync(tab, value);
            }
            else
            {
                OnPropertyChanged(nameof(SectionNavigationSelection));
            }
        }
    }

    private Task QueueNavigationAsync(
        SettingsTabViewModel tab,
        SettingsSectionNavigationItemViewModel? section)
    {
        _pendingNavigation = new PendingSettingsNavigation(tab, section);
        if (_navigationSelectionTask.IsCompleted)
        {
            _navigationSelectionTask = ProcessNavigationQueueAsync();
        }
        return _navigationSelectionTask;
    }

    private async Task ProcessNavigationQueueAsync()
    {
        while (_pendingNavigation is { } requested)
        {
            _pendingNavigation = null;
            if (!ReferenceEquals(requested.Tab, SelectedTab)
                && !await ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
            {
                _pendingNavigation = null;
                OnPropertyChanged(nameof(NavigationSelection));
                OnPropertyChanged(nameof(SectionNavigationSelection));
                return;
            }

            var target = _pendingNavigation ?? requested;
            _pendingNavigation = null;
            CommitNavigation(target);
        }
    }

    private void CommitNavigation(PendingSettingsNavigation target)
    {
        foreach (var item in Tabs)
        {
            item.IsSelected = ReferenceEquals(item, target.Tab);
        }
        SelectedTab = target.Tab;

        if (target.Section is null)
        {
            SelectFirstSectionForTab(target.Tab.Id);
            return;
        }

        if (SetProperty(
            ref _selectedSectionNavigationItem,
            target.Section,
            nameof(SectionNavigationSelection)))
        {
            ScrollToSectionRequested?.Invoke(
                this,
                new SettingsSectionNavigationRequest(target.Section.AnchorName, target.Section.Title));
        }
    }

    private void SelectFirstSectionForTab(string tabId)
    {
        var first = SectionIndexItems.FirstOrDefault(item =>
            string.Equals(item.TabId, tabId, StringComparison.Ordinal));
        if (first is not null)
        {
            SetProperty(ref _selectedSectionNavigationItem, first, nameof(SectionNavigationSelection));
        }
    }

    public bool IsGeneralSelected => SelectedTab.Id == "general";
    public bool IsModelsSelected => SelectedTab.Id == "models";
    public bool IsPresetsSelected => SelectedTab.Id == "presets";
    public bool IsAutomationSelected => SelectedTab.Id == "automation";
    public bool IsPermissionsSelected => SelectedTab.Id == "permissions";
    public bool IsPersonalizationSelected => SelectedTab.Id == "personalization";
    public bool IsMiscSelected => SelectedTab.Id == "misc";
    public bool IsGeneralEditable => CanSave(GeneralSection);
    public bool IsModelsEditable => CanSave(ModelsSection);
    public bool IsPresetsEditable => CanSave(PresetsSection);
    public bool IsTemplateRepositoryEditable => CanSave(TemplateRepositorySection);
    public bool IsAutomationEditable => CanSave(AutomationSection);
    public bool IsPermissionsEditable => CanSave(PermissionsSection);
    public bool IsPersonalizationEditable => CanSave(PersonalizationSection);
    public bool IsAppRuntimeEditable => CanSave(AppRuntimeSection);
    public bool IsMiscEditable => CanSave(MiscSection);
    public ObservableCollection<LanguageOption> LanguageOptions { get; }
    public ObservableCollection<SettingsValueOption> VectorBackendOptions { get; }
    public ObservableCollection<SettingsValueOption> ProviderTypeOptions { get; }
    public ObservableCollection<ThemeOption> ThemeOptions { get; }
    public ObservableCollection<ThemeGroupViewModel> ThemeGroups { get; }
    public ObservableCollection<ConfirmationPolicyViewModel> ConfirmationPolicies { get; }
    /// <summary>确认项按总结机制分组。</summary>
    public ObservableCollection<ConfirmationPolicyGroupViewModel> ConfirmationPolicyGroups { get; }
    public ObservableCollection<NodeTypePresetViewModel> NodePresets { get; }
    public ObservableCollection<ProviderOptionViewModel> ProviderOptions { get; }
    public ObservableCollection<ModelOptionViewModel> AvailableModels { get; }
    /// <summary>全局节点默认和节点类型预设使用 Provider/Model 成对身份。</summary>
    public ObservableCollection<WorkflowModelOption> AvailableLlmModelOptions { get; }
    public ObservableCollection<ToolControlGroupViewModel> ToolControlGroups { get; }
    public ObservableCollection<PermissionScopeProfileViewModel> ScopedPermissionProfiles { get; }
    public ColorChannelEditor GitAutoColorEditor { get; }
    public ColorChannelEditor GitManualColorEditor { get; }
    public ColorChannelEditor ThemeMainColorEditor { get; }
    public ColorChannelEditor ThemeSurfaceColorEditor { get; }
    public ColorChannelEditor ThemeBrandColorEditor { get; }

    /// <summary>Git 自动色色图（色相×深浅）。</summary>
    public ObservableCollection<ColorSwatchItemViewModel> GitAutoColorSwatches { get; }

    /// <summary>Git 手动色色图。</summary>
    public ObservableCollection<ColorSwatchItemViewModel> GitManualColorSwatches { get; }

    /// <summary>主题自定义三色共用色图。</summary>
    public ObservableCollection<ColorSwatchItemViewModel> ThemeColorSwatches { get; }

    /// <summary>色图列数（色相 + 中性灰列）。</summary>
    public int ColorMapColumns => ColorPaletteMap.Columns();

    private ThemeColorChannel _activeThemeColorChannel = ThemeColorChannel.Brand;

    public bool IsThemeMainChannelActive => _activeThemeColorChannel == ThemeColorChannel.Main;
    public bool IsThemeSurfaceChannelActive => _activeThemeColorChannel == ThemeColorChannel.Surface;
    public bool IsThemeBrandChannelActive => _activeThemeColorChannel == ThemeColorChannel.Brand;
    public bool IsEditingDayThemeColors => !_editingNightThemeColors;
    public bool IsEditingNightThemeColors => _editingNightThemeColors;

    /// <summary>当前激活的主题色槽（供 PS 式调色板双向绑定）。</summary>
    public string ActiveThemeColorLabel => _activeThemeColorChannel switch
    {
        ThemeColorChannel.Main => ThemeMainColorLabel,
        ThemeColorChannel.Surface => ThemeSurfaceColorLabel,
        _ => ThemeBrandColorLabel,
    };

    public string ActiveThemeColorHex
    {
        get => _activeThemeColorChannel switch
        {
            ThemeColorChannel.Main => ThemeMainColorEditor.ToHexValue(),
            ThemeColorChannel.Surface => ThemeSurfaceColorEditor.ToHexValue(),
            _ => ThemeBrandColorEditor.ToHexValue(),
        };
        set
        {
            if (string.IsNullOrWhiteSpace(value))
            {
                return;
            }

            OnThemeColorSwatchPicked(value);
            OnPropertyChanged();
        }
    }

    public RelayCommand SelectThemeMainChannelCommand { get; }
    public RelayCommand SelectThemeSurfaceChannelCommand { get; }
    public RelayCommand SelectThemeBrandChannelCommand { get; }
    public RelayCommand SelectThemeEditDayCommand { get; }
    public RelayCommand SelectThemeEditNightCommand { get; }

    public RelayCommand SaveGeneralCommand { get; }
    public RelayCommand RefreshModelsCommand { get; }
    public RelayCommand SaveModelCommand { get; }
    public RelayCommand SaveProviderKeyCommand { get; }
    public RelayCommand RemoveProviderCommand { get; }
    public RelayCommand AddProviderCommand { get; }
    public RelayCommand SavePresetsCommand { get; }
    public RelayCommand SaveTemplateRepositoryCommand { get; }
    public RelayCommand OpenTemplateMarketCommand { get; }
    public RelayCommand SaveAutomationCommand { get; }
    public RelayCommand SavePermissionsCommand { get; }
    public RelayCommand SavePersonalizationCommand { get; }
    public RelayCommand SaveAppRuntimeCommand { get; }
    public RelayCommand SaveMiscCommand { get; }
    public RelayCommand ShowTutorialCommand { get; }
    public RelayCommand BrowseDocumentsDirCommand { get; }
    public RelayCommand BrowseWorkflowsDirCommand { get; }
    public RelayCommand BrowseSkillsDirCommand { get; }
    public RelayCommand BrowseExportsDirCommand { get; }
    public RelayCommand BrowseReadableRootsCommand { get; }
    public RelayCommand BrowseWritableRootsCommand { get; }

    public string GeneralTitle => _displayNames.Text("ui.settings.general.title");
    public string GeneralScopeHelpText => _displayNames.Text("ui.settings.general.scope_help");
    public string ModelsTitle => _displayNames.Text("ui.settings.models.title");
    public string PresetsTitle => _displayNames.Text("ui.settings.presets.title");
    public string AutomationTitle => _displayNames.Text("ui.settings.automation.title");
    public string AutomationScopeHelpText => _displayNames.Text("ui.settings.automation.scope_help");
    public string PermissionsTitle => _displayNames.Text("ui.settings.permissions.title");
    public string PersonalizationTitle => _displayNames.Text("ui.settings.personalization.title");
    public string PersonalizationScopeHelpText => _displayNames.Text("ui.settings.personalization.scope_help");
    public string MiscTitle => _displayNames.Text("ui.settings.misc.title");
    public string AppRuntimeScopeHelpText => _displayNames.Text("ui.settings.misc.app_runtime_scope_help");
    public string RetrievalScopeHelpText => _displayNames.Text("ui.settings.misc.retrieval_scope_help");

    public string ProjectNameLabel => _displayNames.Text("ui.settings.general.project_name");
    public string DocumentsDirLabel => _displayNames.Text("ui.settings.general.documents_dir");
    public string WorkflowsDirLabel => _displayNames.Text("ui.settings.general.workflows_dir");
    public string SkillsDirLabel => _displayNames.Text("ui.settings.general.skills_dir");
    public string ExportsDirLabel => _displayNames.Text("ui.settings.general.exports_dir");
    public string ProjectMemoryLabel => _displayNames.Text("ui.works.project_memory");
    public string ProjectMemoryPlaceholder => _displayNames.Text("ui.works.project_memory.placeholder");
    public string SaveGeneralText => _displayNames.Text("ui.settings.general.save");

    public string ProviderIdLabel => _displayNames.Text("ui.settings.models.provider_id");
    public string ProviderTypeLabel => _displayNames.Text("ui.settings.models.provider_type");
    public string ProviderDisplayNameLabel => _displayNames.Text("ui.settings.models.display_name");
    public string BaseUrlLabel => _displayNames.Text("ui.settings.models.base_url");
    public string BaseUrlPlaceholder => _displayNames.Text("ui.settings.models.base_url.placeholder");
    public string ProviderEnabledText => _displayNames.Text("ui.settings.models.enabled");
    public string MakeDefaultLlmText => _displayNames.Text("ui.settings.models.make_default_llm");
    public string MakeDefaultEmbeddingText => _displayNames.Text("ui.settings.models.make_default_embedding");
    public string MakeDefaultRerankerText => _displayNames.Text("ui.settings.models.make_default_reranker");
    public string MakeDefaultSearchText => _displayNames.Text("ui.settings.models.make_default_search");
    public string AvailableModelsText => _displayNames.Text("ui.settings.models.available_models");
    public string ManualModelsText => _displayNames.Text("ui.settings.models.manual_models");
    public string ModelsTextLabel => _displayNames.Text("ui.settings.models.models");
    public string ModelsPlaceholder => _displayNames.Text("ui.settings.models.models.placeholder");
    public string EmbeddingModelLabel => _displayNames.Text("ui.settings.models.embedding_model");
    public string EmbeddingModelPlaceholder => _displayNames.Text("ui.settings.models.embedding_model.placeholder");
    public string ApiKeyLabel => _displayNames.Text("ui.settings.models.api_key");
    public string ApiKeyPlaceholder => _displayNames.Text("ui.settings.models.api_key.placeholder");
    public string SaveModelText => _displayNames.Text("ui.settings.models.save");
    public string SaveKeyText => _displayNames.Text("ui.settings.models.save_key");
    public string RemoveProviderText => _displayNames.Text("ui.settings.models.remove");
    public string RefreshText => _displayNames.Text("ui.common.refresh");
    public string ProviderStatusLabel => _displayNames.Text("ui.settings.models.status");
    public string AddProviderText => _displayNames.Text("ui.settings.models.add_provider");
    public string ProviderListTitle => _displayNames.Text("ui.settings.models.provider_list");
    public string ProviderEditorTitle => _displayNames.Text("ui.settings.models.provider_editor");
    public string ProviderScopeHelpText => _displayNames.Text("ui.settings.models.scope_help");
    public string ColorRgbHintText => _displayNames.Text("ui.settings.personalization.color_rgb_hint");
    public string ColorMapHintText => _displayNames.Text("ui.settings.personalization.color_map_hint");
    public string ColorHexSecondaryText => _displayNames.Text("ui.settings.personalization.color_hex_secondary");

    public string PresetNodeTypeLabel => _displayNames.Text("ui.settings.presets.node_type");
    public string PresetNodeModelLabel => _displayNames.Text("ui.settings.presets.node_model");
    public string PresetNodeTimeoutLabel => _displayNames.Text("ui.settings.presets.node_timeout_ms");
    public string PresetNodeBudgetLabel => _displayNames.Text("ui.settings.presets.node_budget_usd");
    public string PresetAccessTitle => _displayNames.Text("ui.settings.presets.access_title");
    public string PresetToolsTitle => _displayNames.Text("ui.settings.presets.tools_title");
    public string PresetScopeHelpText => _displayNames.Text("ui.settings.presets.scope_help");
    public string InheritNodePermissionsText => _displayNames.Text("ui.settings.presets.inherit_node_permissions");
    public string DefaultModelLabel => _displayNames.Text("ui.settings.presets.default_model");
    public string DefaultTimeoutLabel => _displayNames.Text("ui.settings.presets.default_timeout_ms");
    public string DefaultBudgetLabel => _displayNames.Text("ui.settings.presets.default_budget_usd");
    public string TemplateRepositoryLabel => _displayNames.Text("ui.settings.presets.template_repository");
    public string OpenTemplateMarketText => _displayNames.Text("ui.settings.presets.open_market");
    public string SavePresetsText => _displayNames.Text("ui.settings.presets.save");
    public string SaveTemplateRepositoryText => _displayNames.Text("ui.settings.presets.save_template_repository");

    public string BudgetLabel => _displayNames.Text("ui.settings.automation.global_budget");
    public string BudgetHelpText => _displayNames.Text("ui.settings.automation.budget_help");
    public string PreauthorizedBudgetLabel => _displayNames.Text("ui.settings.automation.preauthorized_budget");
    public string PreauthorizedHelpText => _displayNames.Text("ui.settings.automation.preauthorized_help");
    public string SpentLabel => _displayNames.Text("ui.settings.automation.spent");
    public string NormalModeLabel => _displayNames.Text("ui.settings.automation.confirmation.normal_mode");
    public string AutoModePolicyLabel => _displayNames.Text("ui.settings.automation.confirmation.auto_mode_policy");
    public string ApprovalPromptLabel => _displayNames.Text("ui.settings.automation.confirmation.approval_prompt");
    public string ApprovalPromptPlaceholder => _displayNames.Text("ui.settings.automation.confirmation.approval_prompt_placeholder");
    public string ConfirmationPolicyHelpText => _displayNames.Text("ui.settings.automation.confirmation.help");
    public string PolicyAllowText => _displayNames.Text("ui.settings.automation.confirmation.allow");
    public string PolicyReviewText => _displayNames.Text("ui.settings.automation.confirmation.review");
    /// <summary>Auto Mode 列：开 = auto_approval（自动审计），与普通模式「放行」语义不同。</summary>
    public string PolicyAutoOnText => _displayNames.Text("ui.settings.automation.confirmation.auto_on");
    /// <summary>Auto Mode 列：关 = allow_by_default（默认跳过确认），不是人工审核。</summary>
    public string PolicyAutoOffText => _displayNames.Text("ui.settings.automation.confirmation.auto_off");
    public string BrowseFolderText => _displayNames.Text("ui.settings.browse_folder");
    public string WorkflowLimitLabel => _displayNames.Text("ui.settings.automation.workflow");
    public string WorkflowDefaultTimeoutLabel => _displayNames.Text("ui.settings.automation.default_timeout_ms");
    public string MaxLoopIterationsLabel => _displayNames.Text("ui.settings.automation.max_loop_iterations");
    public string MaxToolRoundsLabel => _displayNames.Text("ui.settings.automation.max_tool_rounds");
    public string CheckpointEnabledLabel => _displayNames.Text("ui.settings.automation.checkpoint_enabled");
    public string RuntimeAutosaveLabel => _displayNames.Text("ui.settings.automation.runtime_autosave_ms");
    public string SaveAutomationText => _displayNames.Text("ui.settings.automation.save");

    public string AllowNetworkText => _displayNames.Text("ui.settings.permissions.allow_network");
    public string AllowWebSearchText => _displayNames.Text("ui.settings.permissions.allow_web_search");
    public string AllowHttpSkillText => _displayNames.Text("ui.settings.permissions.allow_http_skill");
    public string AllowWasmNetworkText => _displayNames.Text("ui.settings.permissions.allow_wasm_network");
    public string AllowSecretReadText => _displayNames.Text("ui.settings.permissions.allow_secret_read");
    public string GlobalDefaultsHelpText => _displayNames.Text("ui.settings.permissions.global_defaults_help");
    public string PermissionsScopeHelpText => _displayNames.Text("ui.settings.permissions.scope_help");
    public string InheritGlobalText => _displayNames.Text("ui.settings.permissions.inherit_global");
    public string ToolControlsLabel => _displayNames.Text("ui.settings.permissions.tool_controls");
    public string DangerToolsTitle => _displayNames.Text("ui.settings.permissions.danger_tools.title");
    public string DangerToolsHelp => _displayNames.Text("ui.settings.permissions.danger_tools.help");
    public string SafeToolsTitle => _displayNames.Text("ui.settings.permissions.safe_tools.title");
    public string ReadableRootsLabel => _displayNames.Text("ui.settings.permissions.read_roots");
    public string WritableRootsLabel => _displayNames.Text("ui.settings.permissions.write_roots");
    public string PathPlaceholder => _displayNames.Text("ui.settings.permissions.path_placeholder");
    public string SavePermissionsText => _displayNames.Text("ui.settings.permissions.save");

    public string ThemeLabel => _displayNames.Text("ui.settings.personalization.theme");
    public string ThemePaletteHelpText => _displayNames.Text("ui.settings.personalization.theme.palette_help");
    public string ThemeCustomThreeLabel => _displayNames.Text("ui.settings.personalization.theme.custom_three");
    public string ThemeCustomThreeHint => _displayNames.Text("ui.settings.personalization.theme.custom_three_hint");
    public string ThemeMainColorLabel => _displayNames.Text("ui.settings.personalization.theme.color_main");
    public string ThemeFollowSystemColorsText => _displayNames.Text("ui.settings.personalization.theme.follow_system_colors");
    public string ThemeEditDayText => _displayNames.Text("ui.settings.personalization.theme.edit_day");
    public string ThemeEditNightText => _displayNames.Text("ui.settings.personalization.theme.edit_night");
    public string ThemeSurfaceColorLabel => _displayNames.Text("ui.settings.personalization.theme.color_surface");
    public string ThemeBrandColorLabel => _displayNames.Text("ui.settings.personalization.theme.color_brand");
    public string GitAutoColorLabel => _displayNames.Text("ui.settings.personalization.git_auto_color");
    public string GitManualColorLabel => _displayNames.Text("ui.settings.personalization.git_manual_color");
    public string ProjectPanelVisibleText => _displayNames.Text("ui.settings.personalization.project_panel");
    public string ReduceMotionText => _displayNames.Text("ui.settings.personalization.reduce_motion");
    public string ReduceMotionHintText => _displayNames.Text("ui.settings.personalization.reduce_motion.desc");
    public string SavePersonalizationText => _displayNames.Text("ui.settings.personalization.save");

    public string RagLabel => _displayNames.Text("ui.settings.misc.rag");
    public string VectorEnabledText => _displayNames.Text("ui.settings.misc.vector_enabled");
    public string VectorBackendLabel => _displayNames.Text("ui.settings.misc.vector_backend");
    public string VectorCollectionLabel => _displayNames.Text("ui.settings.misc.vector_collection");
    public string VectorDimensionsLabel => _displayNames.Text("ui.settings.misc.vector_dimensions");
    public string QdrantHostLabel => _displayNames.Text("ui.settings.misc.qdrant_host");
    public string QdrantPortLabel => _displayNames.Text("ui.settings.misc.qdrant_port");
    public string QdrantDataDirLabel => _displayNames.Text("ui.settings.misc.qdrant_data_dir");
    public string QdrantBinaryPathLabel => _displayNames.Text("ui.settings.misc.qdrant_binary_path");
    public string QdrantStartupTimeoutLabel => _displayNames.Text("ui.settings.misc.qdrant_startup_timeout");
    public string SaveAppRuntimeText => _displayNames.Text("ui.settings.misc.save_app_runtime");
    public string RerankerEnabledText => _displayNames.Text("ui.settings.misc.reranker_enabled");
    public string ChunkSizeLabel => _displayNames.Text("ui.settings.misc.chunk_size");
    public string ChunkOverlapLabel => _displayNames.Text("ui.settings.misc.chunk_overlap");
    public string GitLabel => _displayNames.Text("ui.settings.misc.git");
    public string TrackDocumentsText => _displayNames.Text("ui.settings.misc.track_documents");
    public string TrackWorkflowsText => _displayNames.Text("ui.settings.misc.track_workflows");
    public string TrackSkillsText => _displayNames.Text("ui.settings.misc.track_skills");
    public string TrackConfigText => _displayNames.Text("ui.settings.misc.track_config");
    public string IgnoredPathsLabel => _displayNames.Text("ui.settings.misc.ignored_paths");
    public string IgnoredPathsPlaceholder => _displayNames.Text("ui.settings.misc.ignored_paths.placeholder");
    public string SaveMiscText => _displayNames.Text("ui.settings.misc.save");
    public string LanguageLabel => _displayNames.Text("ui.settings.misc.language");
    public string TutorialText => _displayNames.Text("ui.settings.index.tutorial");
    public string OpenTutorialText => _displayNames.Text("ui.settings.misc.open_tutorial");
    public string DiagnosticsLabel => _displayNames.Text("ui.settings.misc.diagnostics");
    public string DiagnosticsStatusText => _displayNames.Format("ui.settings.misc.diagnostics.status", new Dictionary<string, string>
    {
        ["status"] = DiagnosticsStatus,
    });

    public string ProjectName { get => _projectName; set => SetProperty(ref _projectName, value); }
    public string Locale { get => _locale; set => SetProperty(ref _locale, value); }
    public string DocumentsDir { get => _documentsDir; set => SetProperty(ref _documentsDir, value); }
    public string WorkflowsDir { get => _workflowsDir; set => SetProperty(ref _workflowsDir, value); }
    public string SkillsDir { get => _skillsDir; set => SetProperty(ref _skillsDir, value); }
    public string ExportsDir { get => _exportsDir; set => SetProperty(ref _exportsDir, value); }
    public string ProjectMemory { get => _projectMemory; set => SetProperty(ref _projectMemory, value); }

    public string ProviderId { get => _providerId; set => SetProperty(ref _providerId, value); }
    public string ProviderType { get => _providerType; set => SetProperty(ref _providerType, value); }
    public string ProviderDisplayName { get => _providerDisplayName; set => SetProperty(ref _providerDisplayName, value); }
    public string ProviderBaseUrl { get => _providerBaseUrl; set => SetProperty(ref _providerBaseUrl, value); }
    public bool ProviderEnabled { get => _providerEnabled; set => SetProperty(ref _providerEnabled, value); }
    public bool MakeDefaultLlm { get => _makeDefaultLlm; set => SetProperty(ref _makeDefaultLlm, value); }
    public bool MakeDefaultEmbedding { get => _makeDefaultEmbedding; set => SetProperty(ref _makeDefaultEmbedding, value); }
    public bool MakeDefaultReranker { get => _makeDefaultReranker; set => SetProperty(ref _makeDefaultReranker, value); }
    public bool MakeDefaultSearch { get => _makeDefaultSearch; set => SetProperty(ref _makeDefaultSearch, value); }
    public string ApiKey { get => _apiKey; set => SetProperty(ref _apiKey, value); }
    public string ModelsText { get => _modelsText; set => SetProperty(ref _modelsText, value); }
    public string EmbeddingModelId { get => _embeddingModelId; set => SetProperty(ref _embeddingModelId, value); }
    public bool ManualModelsVisible { get => _manualModelsVisible; set => SetProperty(ref _manualModelsVisible, value); }
    public string ProviderStatus { get => _providerStatus; set => SetProperty(ref _providerStatus, value); }
    public ProviderOptionViewModel? SelectedProviderOption
    {
        get => _selectedProviderOption;
        set
        {
            // 抑制路径（SetSelected/Restore）直接写字段；用户改选走单飞选择队列，
            // 仅在离开成功后才提交列表选中，避免取消时列表与表单脱节。
            if (_suppressProviderSelectionChange)
            {
                if (SetProperty(ref _selectedProviderOption, value))
                {
                    OnPropertyChanged(nameof(IsSelectedProviderDraft));
                    NotifyProviderCommands();
                }
                return;
            }

            if (value is null)
            {
                if (SetProperty(ref _selectedProviderOption, null))
                {
                    OnPropertyChanged(nameof(IsSelectedProviderDraft));
                    NotifyProviderCommands();
                }
                return;
            }

            if (ReferenceEquals(_selectedProviderOption, value)
                || string.Equals(_selectedProviderOption?.ProviderId, value.ProviderId, StringComparison.Ordinal))
            {
                return;
            }

            _ = QueueProviderSelectionAsync(value);
        }
    }

    /// <summary>当前选中供应商是否为未落库草稿（仅草稿可改 ProviderId）。</summary>
    public bool IsSelectedProviderDraft => SelectedProviderOption?.IsDraft == true;

    public string DefaultProviderId => _defaultProviderId;
    public string DefaultModelId { get => _defaultModelId; set => SetProperty(ref _defaultModelId, value); }
    public WorkflowModelOption? SelectedDefaultModelOption
    {
        get => _selectedDefaultModelOption;
        set
        {
            if (!SetProperty(ref _selectedDefaultModelOption, value) || value is null)
            {
                return;
            }

            ApplyDefaultModelIdentity(value.ProviderId, value.ModelId);
        }
    }
    public string DefaultTimeoutMs { get => _defaultTimeoutMs; set => SetProperty(ref _defaultTimeoutMs, value); }
    public string DefaultBudgetUsd { get => _defaultBudgetUsd; set => SetProperty(ref _defaultBudgetUsd, value); }
    public string TemplateRepositoryBaseUrl { get => _templateRepositoryBaseUrl; set => SetProperty(ref _templateRepositoryBaseUrl, value); }

    public string BudgetUsd { get => _budgetUsd; set => SetProperty(ref _budgetUsd, value); }
    public string PreauthorizedUsd { get => _preauthorizedUsd; set => SetProperty(ref _preauthorizedUsd, value); }
    public string SpentText { get => _spentText; set => SetProperty(ref _spentText, value); }
    public string WorkflowDefaultTimeoutMs { get => _workflowDefaultTimeoutMs; set => SetProperty(ref _workflowDefaultTimeoutMs, value); }
    public string MaxLoopIterations { get => _maxLoopIterations; set => SetProperty(ref _maxLoopIterations, value); }
    public string MaxToolRounds { get => _maxToolRounds; set => SetProperty(ref _maxToolRounds, value); }
    public bool CheckpointEnabled { get => _checkpointEnabled; set => SetProperty(ref _checkpointEnabled, value); }
    public string RuntimeAutosaveMs { get => _runtimeAutosaveMs; set => SetProperty(ref _runtimeAutosaveMs, value); }

    public bool AllowNetwork
    {
        get => _allowNetwork;
        set
        {
            if (SetProperty(ref _allowNetwork, value) && !value)
            {
                AllowWebSearch = false;
                AllowHttpSkill = false;
                AllowWasmNetwork = false;
            }
        }
    }
    public bool AllowWebSearch { get => _allowWebSearch; set => SetProperty(ref _allowWebSearch, value); }
    public bool AllowHttpSkill { get => _allowHttpSkill; set => SetProperty(ref _allowHttpSkill, value); }
    public bool AllowWasmNetwork { get => _allowWasmNetwork; set => SetProperty(ref _allowWasmNetwork, value); }
    public bool AllowSecretRead { get => _allowSecretRead; set => SetProperty(ref _allowSecretRead, value); }
    public string ReadableRootsText { get => _readableRootsText; set => SetProperty(ref _readableRootsText, value); }
    public string WritableRootsText { get => _writableRootsText; set => SetProperty(ref _writableRootsText, value); }

    public string Theme
    {
        get => _theme;
        set
        {
            var normalized = ThemeCatalog.Normalize(value);
            if (SetProperty(ref _theme, normalized))
            {
                SyncThemeOptionSelection();
                OnPropertyChanged(nameof(SelectedThemeOption));
                // 选预设时同步三色到该主题 swatch，再应用
                SeedThemeColorsFromPalette(ThemeCatalog.Resolve(normalized), force: true);
                ApplyCurrentThemeColors();
            }
        }
    }

    /// <summary>昼·主底（快照/持久化）。</summary>
    public string ThemeMainColor
    {
        get => SettingsDirtyHelper.NormalizeHexForSnapshot(_themeMainLight);
        set
        {
            var n = SettingsDirtyHelper.NormalizeHexForSnapshot(value);
            if (string.Equals(_themeMainLight, n, StringComparison.OrdinalIgnoreCase) && ThemeApplication.HasHex(n))
            {
                return;
            }

            _themeMainLight = n;
            if (!_editingNightThemeColors)
            {
                ThemeMainColorEditor.SetFromHex(n);
            }

            OnPropertyChanged();
            SyncActiveThemeSwatchSelection();
        }
    }

    public string ThemeSurfaceColor
    {
        get => SettingsDirtyHelper.NormalizeHexForSnapshot(_themeSurfaceLight);
        set
        {
            var n = SettingsDirtyHelper.NormalizeHexForSnapshot(value);
            if (string.Equals(_themeSurfaceLight, n, StringComparison.OrdinalIgnoreCase) && ThemeApplication.HasHex(n))
            {
                return;
            }

            _themeSurfaceLight = n;
            if (!_editingNightThemeColors)
            {
                ThemeSurfaceColorEditor.SetFromHex(n);
            }

            OnPropertyChanged();
            SyncActiveThemeSwatchSelection();
        }
    }

    public string ThemeBrandColor
    {
        get => SettingsDirtyHelper.NormalizeHexForSnapshot(_themeBrandLight);
        set
        {
            var n = SettingsDirtyHelper.NormalizeHexForSnapshot(value);
            if (string.Equals(_themeBrandLight, n, StringComparison.OrdinalIgnoreCase) && ThemeApplication.HasHex(n))
            {
                return;
            }

            _themeBrandLight = n;
            if (!_editingNightThemeColors)
            {
                ThemeBrandColorEditor.SetFromHex(n);
            }

            OnPropertyChanged();
            SyncActiveThemeSwatchSelection();
        }
    }

    public string ThemeMainColorDark
    {
        get => SettingsDirtyHelper.NormalizeHexForSnapshot(_themeMainDark);
        set
        {
            var n = SettingsDirtyHelper.NormalizeHexForSnapshot(value);
            if (string.Equals(_themeMainDark, n, StringComparison.OrdinalIgnoreCase) && ThemeApplication.HasHex(n))
            {
                return;
            }

            _themeMainDark = n;
            if (_editingNightThemeColors)
            {
                ThemeMainColorEditor.SetFromHex(n);
            }

            OnPropertyChanged();
            SyncActiveThemeSwatchSelection();
        }
    }

    public string ThemeSurfaceColorDark
    {
        get => SettingsDirtyHelper.NormalizeHexForSnapshot(_themeSurfaceDark);
        set
        {
            var n = SettingsDirtyHelper.NormalizeHexForSnapshot(value);
            if (string.Equals(_themeSurfaceDark, n, StringComparison.OrdinalIgnoreCase) && ThemeApplication.HasHex(n))
            {
                return;
            }

            _themeSurfaceDark = n;
            if (_editingNightThemeColors)
            {
                ThemeSurfaceColorEditor.SetFromHex(n);
            }

            OnPropertyChanged();
            SyncActiveThemeSwatchSelection();
        }
    }

    public string ThemeBrandColorDark
    {
        get => SettingsDirtyHelper.NormalizeHexForSnapshot(_themeBrandDark);
        set
        {
            var n = SettingsDirtyHelper.NormalizeHexForSnapshot(value);
            if (string.Equals(_themeBrandDark, n, StringComparison.OrdinalIgnoreCase) && ThemeApplication.HasHex(n))
            {
                return;
            }

            _themeBrandDark = n;
            if (_editingNightThemeColors)
            {
                ThemeBrandColorEditor.SetFromHex(n);
            }

            OnPropertyChanged();
            SyncActiveThemeSwatchSelection();
        }
    }

    public bool ThemeFollowSystemColors
    {
        get => _themeFollowSystemColors;
        set
        {
            if (SetProperty(ref _themeFollowSystemColors, value))
            {
                ApplyCurrentThemeColors();
            }
        }
    }
    public ThemeOption? SelectedThemeOption
    {
        get => ThemeOptions.FirstOrDefault(option => option.Code == Theme);
        set
        {
            if (value is not null)
            {
                Theme = value.Code;
            }
        }
    }

    public IEnumerable<IGrouping<string, ThemeOption>> ThemeOptionGroups =>
        ThemeOptions.GroupBy(option => option.Group);

    public string GitAutoColor
    {
        get => GitAutoColorEditor.ToHexValue();
        set
        {
            GitAutoColorEditor.SetFromHex(value);
            _gitAutoColor = GitAutoColorEditor.ToHexValue();
            SyncGitColorSwatchSelection(GitAutoColorSwatches, _gitAutoColor);
            OnPropertyChanged();
        }
    }

    public string GitManualColor
    {
        get => GitManualColorEditor.ToHexValue();
        set
        {
            GitManualColorEditor.SetFromHex(value);
            _gitManualColor = GitManualColorEditor.ToHexValue();
            SyncGitColorSwatchSelection(GitManualColorSwatches, _gitManualColor);
            OnPropertyChanged();
        }
    }

    private static ObservableCollection<ColorSwatchItemViewModel> BuildColorSwatchCollection(Action<string> select)
    {
        var items = ColorPaletteMap.BuildHexMap()
            .Select(hex => new ColorSwatchItemViewModel(hex, select));
        return new ObservableCollection<ColorSwatchItemViewModel>(items);
    }

    private static void SyncGitColorSwatchSelection(
        ObservableCollection<ColorSwatchItemViewModel> swatches,
        string? selectedHex)
    {
        var normalized = ColorSwatchItemViewModel.NormalizeHex(selectedHex ?? string.Empty);
        foreach (var swatch in swatches)
        {
            swatch.IsSelected = string.Equals(swatch.Hex, normalized, StringComparison.OrdinalIgnoreCase);
        }
    }

    private void SetActiveThemeColorChannel(ThemeColorChannel channel)
    {
        if (_activeThemeColorChannel == channel)
        {
            return;
        }

        _activeThemeColorChannel = channel;
        OnPropertyChanged(nameof(IsThemeMainChannelActive));
        OnPropertyChanged(nameof(IsThemeSurfaceChannelActive));
        OnPropertyChanged(nameof(IsThemeBrandChannelActive));
        OnPropertyChanged(nameof(ActiveThemeColorLabel));
        OnPropertyChanged(nameof(ActiveThemeColorHex));
        SyncActiveThemeSwatchSelection();
    }

    private void OnThemeColorSwatchPicked(string hex)
    {
        switch (_activeThemeColorChannel)
        {
            case ThemeColorChannel.Main:
                ThemeMainColorEditor.SetFromHex(hex);
                break;
            case ThemeColorChannel.Surface:
                ThemeSurfaceColorEditor.SetFromHex(hex);
                break;
            default:
                ThemeBrandColorEditor.SetFromHex(hex);
                break;
        }

        PersistActiveEditorsToScheme();
        OnPropertyChanged(nameof(ThemeMainColor));
        OnPropertyChanged(nameof(ThemeSurfaceColor));
        OnPropertyChanged(nameof(ThemeBrandColor));
        OnPropertyChanged(nameof(ThemeMainColorDark));
        OnPropertyChanged(nameof(ThemeSurfaceColorDark));
        OnPropertyChanged(nameof(ThemeBrandColorDark));
        ApplyCurrentThemeColors();
        if (!_suppressDirtyTracking)
        {
            UpdateDirtyState();
        }
    }

    private void OnThemeCustomColorChanged(ThemeColorChannel channel)
    {
        PersistActiveEditorsToScheme();
        OnPropertyChanged(channel switch
        {
            ThemeColorChannel.Main => _editingNightThemeColors ? nameof(ThemeMainColorDark) : nameof(ThemeMainColor),
            ThemeColorChannel.Surface => _editingNightThemeColors ? nameof(ThemeSurfaceColorDark) : nameof(ThemeSurfaceColor),
            _ => _editingNightThemeColors ? nameof(ThemeBrandColorDark) : nameof(ThemeBrandColor),
        });
        if (channel == _activeThemeColorChannel)
        {
            SyncActiveThemeSwatchSelection();
            OnPropertyChanged(nameof(ActiveThemeColorHex));
        }

        ApplyCurrentThemeColors();
        if (!_suppressDirtyTracking)
        {
            UpdateDirtyState();
        }
    }

    private void SetEditingNightThemeColors(bool night)
    {
        if (_editingNightThemeColors == night)
        {
            return;
        }

        PersistActiveEditorsToScheme();
        _editingNightThemeColors = night;
        LoadSchemeIntoEditors();
        OnPropertyChanged(nameof(IsEditingDayThemeColors));
        OnPropertyChanged(nameof(IsEditingNightThemeColors));
        OnPropertyChanged(nameof(ActiveThemeColorHex));
        SyncActiveThemeSwatchSelection();
    }

    private void PersistActiveEditorsToScheme()
    {
        if (_editingNightThemeColors)
        {
            _themeMainDark = ThemeMainColorEditor.ToHexValue();
            _themeSurfaceDark = ThemeSurfaceColorEditor.ToHexValue();
            _themeBrandDark = ThemeBrandColorEditor.ToHexValue();
        }
        else
        {
            _themeMainLight = ThemeMainColorEditor.ToHexValue();
            _themeSurfaceLight = ThemeSurfaceColorEditor.ToHexValue();
            _themeBrandLight = ThemeBrandColorEditor.ToHexValue();
        }
    }

    private void LoadSchemeIntoEditors()
    {
        var suppress = _suppressDirtyTracking;
        _suppressDirtyTracking = true;
        try
        {
            if (_editingNightThemeColors)
            {
                ThemeMainColorEditor.SetFromHex(_themeMainDark);
                ThemeSurfaceColorEditor.SetFromHex(_themeSurfaceDark);
                ThemeBrandColorEditor.SetFromHex(_themeBrandDark);
            }
            else
            {
                ThemeMainColorEditor.SetFromHex(_themeMainLight);
                ThemeSurfaceColorEditor.SetFromHex(_themeSurfaceLight);
                ThemeBrandColorEditor.SetFromHex(_themeBrandLight);
            }
        }
        finally
        {
            _suppressDirtyTracking = suppress;
        }
    }

    private void SyncActiveThemeSwatchSelection()
    {
        var hex = _activeThemeColorChannel switch
        {
            ThemeColorChannel.Main => ThemeMainColorEditor.ToHexValue(),
            ThemeColorChannel.Surface => ThemeSurfaceColorEditor.ToHexValue(),
            _ => ThemeBrandColorEditor.ToHexValue(),
        };
        SyncGitColorSwatchSelection(ThemeColorSwatches, hex);
    }

    private void SeedThemeColorsFromPalette(ThemePalette palette, bool force)
    {
        if (!force
            && ThemeApplication.HasHex(ThemeMainColor)
            && ThemeApplication.HasHex(ThemeSurfaceColor)
            && ThemeApplication.HasHex(ThemeBrandColor))
        {
            return;
        }

        var suppress = _suppressDirtyTracking;
        _suppressDirtyTracking = true;
        try
        {
            var light = palette.IsDark ? ThemeCatalog.Resolve("light") : palette;
            var dark = palette.IsDark ? palette : ThemeCatalog.Resolve("dark");
            _themeMainLight = ThemeApplication.ToHex(light.SwatchMain);
            _themeSurfaceLight = ThemeApplication.ToHex(light.SwatchSurface);
            _themeBrandLight = ThemeApplication.ToHex(light.SwatchBrand);
            _themeMainDark = ThemeApplication.ToHex(dark.SwatchMain);
            _themeSurfaceDark = ThemeApplication.ToHex(dark.SwatchSurface);
            _themeBrandDark = ThemeApplication.ToHex(dark.SwatchBrand);
            // system 演示禁止近黑 surface
            if (palette.Id == "system")
            {
                var demo = ThemeCatalog.SystemDemoSwatches();
                _themeMainLight = ThemeApplication.ToHex(demo.Main);
                _themeSurfaceLight = ThemeApplication.ToHex(demo.Surface);
                _themeBrandLight = ThemeApplication.ToHex(demo.Brand);
            }

            LoadSchemeIntoEditors();
            OnPropertyChanged(nameof(ThemeMainColor));
            OnPropertyChanged(nameof(ThemeSurfaceColor));
            OnPropertyChanged(nameof(ThemeBrandColor));
            OnPropertyChanged(nameof(ThemeMainColorDark));
            OnPropertyChanged(nameof(ThemeSurfaceColorDark));
            OnPropertyChanged(nameof(ThemeBrandColorDark));
            SyncActiveThemeSwatchSelection();
        }
        finally
        {
            _suppressDirtyTracking = suppress;
        }
    }

    private void ApplyCurrentThemeColors()
    {
        PersistActiveEditorsToScheme();
        // 仅勾选「跟随系统明暗」时用昼/夜两套；未勾选始终用昼侧三色
        ThemeApplication.Apply(
            Theme,
            _themeMainLight,
            _themeSurfaceLight,
            _themeBrandLight,
            _themeMainDark,
            _themeSurfaceDark,
            _themeBrandDark,
            ThemeFollowSystemColors);
    }

    private void LoadThemeColorsFromPreferences(UiPreferences prefs)
    {
        var palette = ThemeCatalog.Resolve(prefs.Theme);
        var suppress = _suppressDirtyTracking;
        _suppressDirtyTracking = true;
        try
        {
            var light = palette.IsDark ? ThemeCatalog.Resolve("light") : palette;
            var dark = palette.IsDark ? palette : ThemeCatalog.Resolve("dark");
            var defaultMain = palette.Id == "system"
                ? ThemeApplication.ToHex(ThemeCatalog.SystemDemoSwatches().Main)
                : ThemeApplication.ToHex(light.SwatchMain);
            var defaultSurface = palette.Id == "system"
                ? ThemeApplication.ToHex(ThemeCatalog.SystemDemoSwatches().Surface)
                : ThemeApplication.ToHex(light.SwatchSurface);
            var defaultBrand = palette.Id == "system"
                ? ThemeApplication.ToHex(ThemeCatalog.SystemDemoSwatches().Brand)
                : ThemeApplication.ToHex(light.SwatchBrand);

            _themeMainLight = ThemeApplication.HasHex(prefs.ThemeMainColor)
                ? SettingsDirtyHelper.NormalizeHexForSnapshot(prefs.ThemeMainColor)
                : defaultMain;
            _themeSurfaceLight = ThemeApplication.HasHex(prefs.ThemeSurfaceColor)
                ? SettingsDirtyHelper.NormalizeHexForSnapshot(prefs.ThemeSurfaceColor)
                : defaultSurface;
            _themeBrandLight = ThemeApplication.HasHex(prefs.ThemeBrandColor)
                ? SettingsDirtyHelper.NormalizeHexForSnapshot(prefs.ThemeBrandColor)
                : defaultBrand;
            _themeMainDark = ThemeApplication.HasHex(prefs.ThemeMainColorDark)
                ? SettingsDirtyHelper.NormalizeHexForSnapshot(prefs.ThemeMainColorDark)
                : ThemeApplication.ToHex(dark.SwatchMain);
            _themeSurfaceDark = ThemeApplication.HasHex(prefs.ThemeSurfaceColorDark)
                ? SettingsDirtyHelper.NormalizeHexForSnapshot(prefs.ThemeSurfaceColorDark)
                : ThemeApplication.ToHex(dark.SwatchSurface);
            _themeBrandDark = ThemeApplication.HasHex(prefs.ThemeBrandColorDark)
                ? SettingsDirtyHelper.NormalizeHexForSnapshot(prefs.ThemeBrandColorDark)
                : ThemeApplication.ToHex(dark.SwatchBrand);
            _themeFollowSystemColors = prefs.ThemeFollowSystemColors;
            LoadSchemeIntoEditors();
            OnPropertyChanged(nameof(ThemeMainColor));
            OnPropertyChanged(nameof(ThemeSurfaceColor));
            OnPropertyChanged(nameof(ThemeBrandColor));
            OnPropertyChanged(nameof(ThemeMainColorDark));
            OnPropertyChanged(nameof(ThemeSurfaceColorDark));
            OnPropertyChanged(nameof(ThemeBrandColorDark));
            OnPropertyChanged(nameof(ThemeFollowSystemColors));
            SyncActiveThemeSwatchSelection();
            ApplyCurrentThemeColors();
        }
        finally
        {
            _suppressDirtyTracking = suppress;
        }
    }
    public bool ProjectPanelVisible { get => _projectPanelVisible; set => SetProperty(ref _projectPanelVisible, value); }
    public bool ReduceMotion { get => _reduceMotion; set => SetProperty(ref _reduceMotion, value); }

    public bool VectorEnabled { get => _vectorEnabled; set => SetProperty(ref _vectorEnabled, value); }
    public string VectorBackend
    {
        get => _vectorBackend;
        set
        {
            if (SetProperty(ref _vectorBackend, value))
            {
                OnPropertyChanged(nameof(IsQdrantSidecarBackend));
            }
        }
    }
    public bool IsQdrantSidecarBackend => string.Equals(VectorBackend, "qdrant_sidecar", StringComparison.Ordinal);
    public string VectorCollection { get => _vectorCollection; set => SetProperty(ref _vectorCollection, value); }
    public string VectorDimensions { get => _vectorDimensions; set => SetProperty(ref _vectorDimensions, value); }
    public string QdrantHost { get => _qdrantHost; set => SetProperty(ref _qdrantHost, value); }
    public string QdrantPort { get => _qdrantPort; set => SetProperty(ref _qdrantPort, value); }
    public string QdrantDataDir { get => _qdrantDataDir; set => SetProperty(ref _qdrantDataDir, value); }
    public string QdrantBinaryPath { get => _qdrantBinaryPath; set => SetProperty(ref _qdrantBinaryPath, value); }
    public string QdrantStartupTimeoutMs { get => _qdrantStartupTimeoutMs; set => SetProperty(ref _qdrantStartupTimeoutMs, value); }
    public bool RerankerEnabled { get => _rerankerEnabled; set => SetProperty(ref _rerankerEnabled, value); }
    public string ChunkSizeChars { get => _chunkSizeChars; set => SetProperty(ref _chunkSizeChars, value); }
    public string ChunkOverlapChars { get => _chunkOverlapChars; set => SetProperty(ref _chunkOverlapChars, value); }
    public bool TrackDocuments { get => _trackDocuments; set => SetProperty(ref _trackDocuments, value); }
    public bool TrackWorkflows { get => _trackWorkflows; set => SetProperty(ref _trackWorkflows, value); }
    public bool TrackSkills { get => _trackSkills; set => SetProperty(ref _trackSkills, value); }
    public bool TrackNonSensitiveConfig { get => _trackNonSensitiveConfig; set => SetProperty(ref _trackNonSensitiveConfig, value); }
    public string IgnoredPathsText { get => _ignoredPathsText; set => SetProperty(ref _ignoredPathsText, value); }
    public string DiagnosticsStatus { get => _diagnosticsStatus; set { if (SetProperty(ref _diagnosticsStatus, value)) OnPropertyChanged(nameof(DiagnosticsStatusText)); } }

    public string SelectedLanguage
    {
        get => _selectedLanguage;
        set
        {
            var language = _displayNames.NormalizeAvailableLanguage(value);
            if (SetProperty(ref _selectedLanguage, language))
            {
                _displayNames.SwitchLanguage(language);
                RefreshLocalizedText();
            }
        }
    }

    private SettingsTabViewModel CreateTab(string id, string key) => new(id, _displayNames.Text(key), SelectTab);

    private void ApplySavedLanguage(string locale)
    {
        var language = _displayNames.NormalizeAvailableLanguage(locale);
        if (_displayNames.CurrentLanguage != language)
        {
            _displayNames.SwitchLanguage(language);
        }
        RefreshLocalizedText();
        _selectedLanguage = language;
        OnPropertyChanged(nameof(SelectedLanguage));
    }

    private void SelectTab(SettingsTabViewModel tab)
    {
        _ = QueueNavigationAsync(tab, null);
    }

    public string ProjectSectionTitle => _displayNames.Text("ui.settings.section.project");
    public string DirectoriesSectionTitle => _displayNames.Text("ui.settings.section.directories");
    public string ProjectMemorySectionTitle => _displayNames.Text("ui.settings.section.project_memory");
    public string ProviderSectionTitle => _displayNames.Text("ui.settings.section.provider");
    public string AvailableModelsSectionTitle => _displayNames.Text("ui.settings.section.available_models");
    public string EmbeddingSectionTitle => _displayNames.Text("ui.settings.section.embedding");
    public string ManualModelsSectionTitle => _displayNames.Text("ui.settings.section.manual_fallback");
    public string NodePresetsSectionTitle => _displayNames.Text("ui.settings.section.node_presets");
    public string DefaultsSectionTitle => _displayNames.Text("ui.settings.section.defaults");
    public string TemplatesSectionTitle => _displayNames.Text("ui.settings.section.templates");
    public string BudgetSectionTitle => _displayNames.Text("ui.settings.section.budget");
    public string ConfirmationsSectionTitle => _displayNames.Text("ui.settings.section.confirmations");
    public string RuntimeSectionTitle => _displayNames.Text("ui.settings.section.runtime");
    public string CapabilitiesSectionTitle => _displayNames.Text("ui.settings.section.capabilities");
    public string ToolControlsSectionTitle => _displayNames.Text("ui.settings.section.tool_controls");
    public string PathsSectionTitle => _displayNames.Text("ui.settings.section.paths");
    public string ThemeSectionTitle => _displayNames.Text("ui.settings.section.theme");
    public string WorkspaceSectionTitle => _displayNames.Text("ui.settings.section.workspace");
    public string RetrievalSectionTitle => _displayNames.Text("ui.settings.section.retrieval");
    public string AppRuntimeSectionTitle => _displayNames.Text("ui.settings.section.app_runtime");
    public string GitSectionTitle => _displayNames.Text("ui.settings.section.git");
    public string LanguageSectionTitle => _displayNames.Text("ui.settings.section.language");
    public string DiagnosticsSectionTitle => _displayNames.Text("ui.settings.section.diagnostics");

    private async Task LoadAsync(CancellationToken cancellationToken = default)
    {
        _providerModelRefreshSession.Invalidate();
        var generation = _draftState.BeginLoad();
        var failed = false;
        IsLoading = true;
        NotifySectionStateChanged();
        try
        {
            failed |= !await LoadSectionAsync(
                generation,
                GeneralSection,
                async () => (
                    await _backend.GetAppSettingsAsync(cancellationToken).ConfigureAwait(true),
                    await _backend.ReadProjectMemoryAsync(cancellationToken).ConfigureAwait(true)),
                value =>
                {
                    _schemaVersion = value.Item1.App.SchemaVersion;
                    ProjectName = value.Item1.App.ProjectName;
                    Locale = value.Item1.App.Locale;
                    DocumentsDir = value.Item1.App.DocumentsDir;
                    WorkflowsDir = value.Item1.App.WorkflowsDir;
                    SkillsDir = value.Item1.App.SkillsDir;
                    ExportsDir = value.Item1.App.ExportsDir;
                    ProjectMemory = value.Item2;
                },
                cancellationToken).ConfigureAwait(true);

            try
            {
                var currentProject = await _backend.GetCurrentProjectAsync(cancellationToken).ConfigureAwait(true);
                cancellationToken.ThrowIfCancellationRequested();
                if (_draftState.IsCurrentLoad(generation))
                {
                    _projectRoot = currentProject?.ProjectRoot ?? string.Empty;
                }
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                throw;
            }
            catch
            {
                // 项目身份只服务目录浏览边界，失败不能拖垮项目配置和项目记忆的加载事务。
                _projectRoot = string.Empty;
            }

            failed |= !await LoadSectionAsync(
                generation,
                ModelsSection,
                () => _backend.GetProviderConfigAsync(cancellationToken),
                value =>
                {
                    _providerConfig = value;
                    RebuildProviderOptionsFromConfig(preferProviderId: ProviderId);
                },
                cancellationToken).ConfigureAwait(true);

            failed |= !await LoadPermissionPresetSectionsAsync(
                generation,
                cancellationToken).ConfigureAwait(true);

            failed |= !await LoadSectionAsync(
                generation,
                TemplateRepositorySection,
                () => _backend.GetTemplateRepositorySettingsAsync(cancellationToken),
                value => TemplateRepositoryBaseUrl = value.BaseUrl,
                cancellationToken).ConfigureAwait(true);

            failed |= !await LoadSectionAsync(
                generation,
                AutomationSection,
                async () => (
                    await _backend.GetAutomationSettingsAsync(cancellationToken).ConfigureAwait(true),
                    await _backend.GetWorkflowSettingsAsync(cancellationToken).ConfigureAwait(true)),
                value =>
                {
                    ApplyAutomation(value.Item1);
                    _workflowSchemaVersion = value.Item2.Workflow.SchemaVersion;
                    WorkflowDefaultTimeoutMs = SecondsFromStoredMs(value.Item2.Workflow.DefaultTimeoutMs);
                    MaxLoopIterations = value.Item2.Workflow.MaxLoopIterations.ToString();
                    MaxToolRounds = value.Item2.Workflow.MaxToolRounds.ToString();
                    CheckpointEnabled = value.Item2.Workflow.CheckpointEnabled;
                    RuntimeAutosaveMs = value.Item2.Workflow.RuntimeAutosaveMs.ToString();
                },
                cancellationToken).ConfigureAwait(true);

            failed |= !await LoadSectionAsync(
                generation,
                PersonalizationSection,
                () => _backend.GetUiPreferencesAsync(cancellationToken),
                ApplyLoadedUiPreferences,
                cancellationToken).ConfigureAwait(true);

            failed |= !await LoadSectionAsync(
                generation,
                AppRuntimeSection,
                () => _backend.GetAppRuntimeSettingsAsync(cancellationToken),
                ApplyAppRuntime,
                cancellationToken).ConfigureAwait(true);

            failed |= !await LoadSectionAsync(
                generation,
                MiscSection,
                async () => (
                    await _backend.GetRagSettingsAsync(cancellationToken).ConfigureAwait(true),
                    await _backend.GetGitSettingsAsync(cancellationToken).ConfigureAwait(true)),
                value => ApplyMisc(value.Item1, value.Item2),
                cancellationToken).ConfigureAwait(true);

            try
            {
                var diagnostics = await _backend.GetBackendDiagnosticsAsync(cancellationToken).ConfigureAwait(true);
                cancellationToken.ThrowIfCancellationRequested();
                if (_draftState.IsCurrentLoad(generation))
                {
                    DiagnosticsStatus = diagnostics.Status;
                }
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                throw;
            }
            catch
            {
                failed = true;
            }

            EnsureDefaultConfirmationPoliciesIfEmpty();
            StatusText = failed
                ? _displayNames.Text("ui.settings.status.section_load_failed")
                : _displayNames.Text("ui.common.configured");
        }
        finally
        {
            _suppressDirtyTracking = false;
            IsLoading = false;
            NotifySectionStateChanged();
            UpdateDirtyState(updateStatus: false);
        }
    }

    private async Task<bool> LoadSectionAsync<T>(
        long generation,
        string section,
        Func<Task<T>> read,
        Action<T> apply,
        CancellationToken cancellationToken = default)
    {
        try
        {
            cancellationToken.ThrowIfCancellationRequested();
            var value = await read().ConfigureAwait(true);
            cancellationToken.ThrowIfCancellationRequested();
            if (!_draftState.IsCurrentLoad(generation))
            {
                return false;
            }

            var wasSuppressing = _suppressDirtyTracking;
            _suppressDirtyTracking = true;
            try
            {
                apply(value);
            }
            finally
            {
                _suppressDirtyTracking = wasSuppressing;
            }

            return _draftState.AcceptLoaded(generation, section, CurrentSectionValues(section));
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            throw;
        }
        catch
        {
            return false;
        }
        finally
        {
            NotifySectionStateChanged();
        }
    }

    /// <summary>
    /// 节点预设的权限投影依赖全局权限配置，因此预设只能在同代权限成功后提交。
    /// 全局权限本身可独立成功，避免无项目或项目预设损坏时阻断应用级安全设置。
    /// </summary>
    private async Task<bool> LoadPermissionPresetSectionsAsync(
        long generation,
        CancellationToken cancellationToken = default)
    {
        var permissionsAccepted = false;
        var presetsAccepted = false;
        try
        {
            cancellationToken.ThrowIfCancellationRequested();
            var presetsTask = _backend.GetNodePresetSettingsAsync(cancellationToken);
            var permissionsTask = _backend.GetPermissionsSettingsAsync(cancellationToken);

            PermissionsSettings? permissions = null;
            try
            {
                permissions = await permissionsTask.ConfigureAwait(true);
                cancellationToken.ThrowIfCancellationRequested();
                if (_draftState.IsCurrentLoad(generation))
                {
                    var wasSuppressing = _suppressDirtyTracking;
                    _suppressDirtyTracking = true;
                    try
                    {
                        ApplyPermissions(permissions);
                    }
                    finally
                    {
                        _suppressDirtyTracking = wasSuppressing;
                    }

                    permissionsAccepted = _draftState.AcceptLoaded(
                        generation,
                        PermissionsSection,
                        CurrentSectionValues(PermissionsSection));
                }
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                throw;
            }
            catch
            {
                permissions = null;
            }

            try
            {
                var presets = await presetsTask.ConfigureAwait(true);
                cancellationToken.ThrowIfCancellationRequested();
                if (permissions is not null && _draftState.IsCurrentLoad(generation))
                {
                    var wasSuppressing = _suppressDirtyTracking;
                    _suppressDirtyTracking = true;
                    try
                    {
                        ApplyDefaultModelIdentity(presets.DefaultProviderId, presets.DefaultModelId);
                        DefaultTimeoutMs = SecondsFromStoredMs(presets.DefaultTimeoutMs);
                        DefaultBudgetUsd = presets.DefaultBudgetUsd.ToString("0.####");
                        ApplyNodePresets(presets, ResolveWorkflowNodePermissionPolicy(permissions));
                    }
                    finally
                    {
                        _suppressDirtyTracking = wasSuppressing;
                    }

                    presetsAccepted = _draftState.AcceptLoaded(
                        generation,
                        PresetsSection,
                        CurrentSectionValues(PresetsSection));
                }
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                throw;
            }
            catch
            {
                presetsAccepted = false;
            }

            return presetsAccepted && permissionsAccepted;
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            throw;
        }
        catch
        {
            return false;
        }
        finally
        {
            NotifySectionStateChanged();
        }
    }

    internal Task<bool> ReloadPermissionPresetProjectionForTestsAsync(
        CancellationToken cancellationToken = default)
    {
        var generation = _draftState.BeginLoad();
        return LoadPermissionPresetSectionsAsync(generation, cancellationToken);
    }

    internal Task<bool> ReloadAppRuntimeForTestsAsync(
        CancellationToken cancellationToken = default)
    {
        var generation = _draftState.BeginLoad();
        return LoadSectionAsync(
            generation,
            AppRuntimeSection,
            () => _backend.GetAppRuntimeSettingsAsync(cancellationToken),
            ApplyAppRuntime,
            cancellationToken);
    }

    internal Task<bool> SaveAppRuntimeForTestsAsync() => SaveAppRuntimeAsync();

    internal void ApplyProviderConfigForTests(ProviderConfigStatus status)
    {
        _providerModelRefreshSession.Invalidate();
        _providerConfig = status;
        RebuildProviderOptionsFromConfig(status.DefaultLlmProviderId);
        SetSectionBaseline(ModelsSection);
    }

    internal Task RefreshProviderModelsForTestsAsync() => FetchModelsAsync();

    internal void SelectProviderForTests(string providerId)
    {
        _providerModelRefreshSession.Invalidate();
        SelectProviderForEditing(providerId);
    }

    internal Task AddProviderDraftForTestsAsync()
    {
        SelectTabForTests("models");
        return AddProviderDraftAsync();
    }

    internal Task SelectProviderOptionForTestsAsync(string providerId)
    {
        var option = ProviderOptions.First(item =>
            string.Equals(item.ProviderId, providerId, StringComparison.Ordinal));
        return QueueProviderSelectionAsync(option);
    }

    internal void ConfigureProjectDirectoryPickerForTests(
        string projectRoot,
        Func<string?, Task<string?>> picker)
    {
        _projectRoot = projectRoot;
        SetFolderPicker(picker);
    }

    internal Task BrowseDocumentsDirectoryForTestsAsync() =>
        BrowseProjectDirectoryAsync(value => DocumentsDir = value);

    internal Task SelectSectionForTestsAsync(string sectionId)
    {
        var section = SectionIndexItems.First(item =>
            string.Equals(item.Id, sectionId, StringComparison.Ordinal));
        var tab = Tabs.First(item =>
            string.Equals(item.Id, section.TabId, StringComparison.Ordinal));
        return QueueNavigationAsync(tab, section);
    }

    internal Task SelectNavigationTabForTestsAsync(string tabId)
    {
        var tab = Tabs.First(item =>
            string.Equals(item.Id, tabId, StringComparison.Ordinal));
        return QueueNavigationAsync(tab, null);
    }

    internal void ReportSectionNavigationFailure(string sectionTitle)
    {
        StatusText = _displayNames.Format(
            "ui.settings.section_navigation_failed",
            new Dictionary<string, string> { ["section"] = sectionTitle });
    }

    private void SelectTabForTests(string tabId)
    {
        var tab = Tabs.First(item => string.Equals(item.Id, tabId, StringComparison.Ordinal));
        foreach (var item in Tabs)
        {
            item.IsSelected = ReferenceEquals(item, tab);
        }
        SelectedTab = tab;
    }

    public void ApplyUiPreferences(UiPreferences preferences)
    {
        // 外部页面只会改变 panel_states 等非表单元数据。持续合并最新全局快照，
        // 避免个性化表单稍后保存时用旧副本覆盖画布/作品/Git 的面板状态。
        _uiPreferences = preferences;
    }

    private void ApplyLoadedUiPreferences(UiPreferences preferences)
    {
        _uiPreferences = preferences;
        ApplySavedLanguage(preferences.Locale);
        _theme = ThemeCatalog.Normalize(preferences.Theme);
        OnPropertyChanged(nameof(Theme));
        SyncThemeOptionSelection();
        OnPropertyChanged(nameof(SelectedThemeOption));
        LoadThemeColorsFromPreferences(preferences);
        GitAutoColor = preferences.GitAutoColor;
        GitManualColor = preferences.GitManualColor;
        ProjectPanelVisible = preferences.ProjectPanelVisible;
        ReduceMotion = preferences.ReduceMotion;
        MotionPreferences.Apply(preferences.ReduceMotion);
    }

    private void ApplyMisc(RagSettings rag, GitSettings git)
    {
        _ragSchemaVersion = rag.Rag.SchemaVersion;
        VectorEnabled = rag.Rag.VectorStore.Enabled;
        VectorBackend = rag.Rag.VectorStore.Backend;
        VectorCollection = rag.Rag.VectorStore.Collection;
        VectorDimensions = rag.Rag.VectorStore.VectorDimensions.ToString();
        QdrantHost = rag.Rag.VectorStore.Sidecar.Host;
        QdrantPort = rag.Rag.VectorStore.Sidecar.Port.ToString();
        QdrantDataDir = rag.Rag.VectorStore.Sidecar.DataDir;
        _fullTextBackend = rag.Rag.FullTextStore.Backend;
        _fullTextIndexDir = rag.Rag.FullTextStore.IndexDir;
        RerankerEnabled = rag.Rag.RerankerEnabled;
        ChunkSizeChars = rag.Rag.ChunkSizeChars.ToString();
        ChunkOverlapChars = rag.Rag.ChunkOverlapChars.ToString();
        _gitSchemaVersion = git.Git.SchemaVersion;
        TrackDocuments = git.Git.TrackDocuments;
        TrackWorkflows = git.Git.TrackWorkflows;
        TrackSkills = git.Git.TrackSkills;
        TrackNonSensitiveConfig = git.Git.TrackNonSensitiveConfig;
        IgnoredPathsText = string.Join(Environment.NewLine, git.Git.IgnoredPaths);
    }

    private void ApplyAppRuntime(AppRuntimeSettings settings)
    {
        QdrantBinaryPath = settings.QdrantBinaryPath;
        QdrantStartupTimeoutMs = settings.QdrantStartupTimeoutMs.ToString(CultureInfo.InvariantCulture);
    }

    private void RebuildProviderOptionsFromConfig(string? preferProviderId)
    {
        ProviderOptions.Clear();
        if (_providerConfig is null)
        {
            RebuildAvailableLlmModelOptions();
            ProviderStatus = _displayNames.Text("ui.settings.models.no_provider_status");
            return;
        }

        foreach (var provider in _providerConfig.Providers)
        {
            ProviderOptions.Add(CreateProviderOption(
                provider.Provider,
                provider.DisplayName,
                provider.HasKey
                    ? _displayNames.Text("ui.common.configured")
                    : _displayNames.Text("ui.common.not_configured"),
                isDraft: !provider.Configured));
        }

        var selected = _providerConfig.Providers.FirstOrDefault(p => p.Provider == preferProviderId)
            ?? _providerConfig.Providers.FirstOrDefault();
        if (selected is not null)
        {
            ApplyProviderForEditing(selected);
            SetSelectedProviderOption(selected.Provider);
        }
        else
        {
            _selectedProviderOption = null;
            OnPropertyChanged(nameof(SelectedProviderOption));
        }

        ProviderStatus = _providerConfig.Providers.Count == 0
            ? _displayNames.Text("ui.settings.models.no_provider_status")
            : _displayNames.Format("ui.settings.models.provider_count", new Dictionary<string, string>
            {
                ["count"] = _providerConfig.Providers.Count.ToString(),
            });
        RebuildAvailableLlmModelOptions();
    }

    private ProviderOptionViewModel CreateProviderOption(
        string providerId,
        string displayName,
        string keyStatus,
        bool isDraft)
    {
        return new ProviderOptionViewModel(
            providerId,
            displayName,
            keyStatus,
            option => _ = QueueProviderSelectionAsync(option),
            isDraft);
    }

    private void SetSelectedProviderOption(string providerId)
    {
        foreach (var option in ProviderOptions)
        {
            option.IsSelected = string.Equals(option.ProviderId, providerId, StringComparison.Ordinal);
        }

        _suppressProviderSelectionChange = true;
        try
        {
            _selectedProviderOption = ProviderOptions.FirstOrDefault(option => option.ProviderId == providerId);
            OnPropertyChanged(nameof(SelectedProviderOption));
            OnPropertyChanged(nameof(IsSelectedProviderDraft));
            NotifyProviderCommands();
        }
        finally
        {
            _suppressProviderSelectionChange = false;
        }
    }

    private Task QueueProviderSelectionAsync(ProviderOptionViewModel option)
    {
        _pendingProviderSelectionId = option.ProviderId;
        if (_providerSelectionTask.IsCompleted)
        {
            _providerSelectionTask = ProcessProviderSelectionQueueAsync();
        }
        return _providerSelectionTask;
    }

    private async Task ProcessProviderSelectionQueueAsync()
    {
        while (_pendingProviderSelectionId is { } requestedProviderId)
        {
            _pendingProviderSelectionId = null;
            if (string.Equals(
                requestedProviderId,
                SelectedProviderOption?.ProviderId,
                StringComparison.Ordinal))
            {
                continue;
            }

            _providerModelRefreshSession.Invalidate();
            var previous = SelectedProviderOption;
            if (!await TryLeaveCurrentProviderAsync(stashOnSuccess: true).ConfigureAwait(true))
            {
                _pendingProviderSelectionId = null;
                RestoreSelectedProviderOption(previous);
                return;
            }

            // 等待确认期间的后续点击覆盖旧目标；一次确认只提交最后一次有效选择。
            var targetProviderId = _pendingProviderSelectionId ?? requestedProviderId;
            _pendingProviderSelectionId = null;
            if (ProviderOptions.Any(option => string.Equals(
                option.ProviderId,
                targetProviderId,
                StringComparison.Ordinal)))
            {
                SelectProviderForEditing(targetProviderId);
            }
            else
            {
                RestoreSelectedProviderOption(previous);
            }
        }
    }

    private async Task AddProviderDraftAsync()
    {
        _providerModelRefreshSession.Invalidate();
        var previous = SelectedProviderOption;
        // 与切换供应商同一套未保存确认，避免静默冲掉正在编辑的表单。
        if (!await TryLeaveCurrentProviderAsync(stashOnSuccess: true).ConfigureAwait(true))
        {
            RestoreSelectedProviderOption(previous);
            return;
        }

        var id = ProviderIdAllocator.Allocate(ProviderOptions.Select(p => p.ProviderId), "provider");
        var draftLabel = _displayNames.Text("ui.settings.models.new_provider_name");
        var draft = CreateProviderOption(
            id,
            draftLabel,
            _displayNames.Text("ui.common.not_configured"),
            isDraft: true);
        var blank = CreateBlankDraftSnapshot(id, draftLabel);
        draft.CaptureForm(blank);
        ProviderOptions.Add(draft);

        ApplyFormSnapshot(blank);
        SetSelectedProviderOption(id);
        HasUnsavedChanges = true;
        StatusText = _displayNames.Format("ui.settings.models.provider_added", new Dictionary<string, string>
        {
            ["id"] = id,
        });
    }

    private void SelectProviderForEditing(string providerId)
    {
        var option = ProviderOptions.FirstOrDefault(p =>
            string.Equals(p.ProviderId, providerId, StringComparison.Ordinal));
        var fromConfig = _providerConfig?.Providers.FirstOrDefault(p =>
            string.Equals(p.Provider, providerId, StringComparison.Ordinal));

        // leave-save 后快照是最新表单；切勿用过期 _providerConfig 盖掉再写回快照。
        if (ProviderFormResolver.PreferFormSnapshotOverConfig(option?.HasFormSnapshot == true)
            && option?.PeekForm() is { } snap)
        {
            ApplyFormSnapshot(snap);
            SetSelectedProviderOption(providerId);
            SetSectionBaseline(ModelsSection);
            return;
        }

        if (fromConfig is not null)
        {
            var wasSuppressingDirty = _suppressDirtyTracking;
            _suppressDirtyTracking = true;
            try
            {
                ApplyProviderForEditing(fromConfig);
            }
            finally
            {
                _suppressDirtyTracking = wasSuppressingDirty;
            }
            // 全局目录中的服务在本项目保存前仍是未授权草稿，不能提前开放密钥/删除操作。
            CaptureCurrentFormToOption(providerId, markDraft: !fromConfig.Configured);
            SetSelectedProviderOption(providerId);
            SetSectionBaseline(ModelsSection);
            return;
        }

        if (option is null)
        {
            return;
        }

        var blank = CreateBlankDraftSnapshot(option.ProviderId, option.DisplayName);
        ApplyFormSnapshot(blank);
        option.CaptureForm(blank);
        SetSelectedProviderOption(providerId);
        if (option.IsDraft)
        {
            UpdateDirtyState();
        }
        else
        {
            SetSectionBaseline(ModelsSection);
        }
    }

    /// <summary>
    /// 处理未保存离开：Save / Discard / Cancel。
    /// stashOnSuccess：成功离开且应保留当前表单到选项快照时（非 Discard 脏数据）写入。
    /// </summary>
    private async Task<bool> TryLeaveCurrentProviderAsync(bool stashOnSuccess)
    {
        var previousId = ProviderId;
        if (string.IsNullOrWhiteSpace(previousId))
        {
            return true;
        }

        if (HasUnsavedChanges && IsModelsSelected)
        {
            var choice = await DialogService.Current.ConfirmUnsavedLeaveAsync().ConfigureAwait(true);
            switch (choice)
            {
                case UnsavedLeaveChoice.Save:
                    try
                    {
                        if (!await SaveModelAsync().ConfigureAwait(true))
                        {
                            return false;
                        }
                        if (stashOnSuccess)
                        {
                            CaptureCurrentFormToOption(previousId, markDraft: false);
                        }
                        return true;
                    }
                    catch (Exception ex)
                    {
                        StatusText = UserFacingError.Format(ex, _displayNames);
                        return false;
                    }
                case UnsavedLeaveChoice.Discard:
                    var option = ProviderOptions.FirstOrDefault(item =>
                        string.Equals(item.ProviderId, previousId, StringComparison.Ordinal));
                    if (option?.IsDraft == true)
                    {
                        ProviderOptions.Remove(option);
                        _providerModelRefreshSession.Invalidate();
                        _selectedProviderOption = null;
                        OnPropertyChanged(nameof(SelectedProviderOption));
                        OnPropertyChanged(nameof(IsSelectedProviderDraft));
                    }
                    else if (option?.PeekForm() is { } cleanSnapshot)
                    {
                        ApplyFormSnapshot(cleanSnapshot);
                    }
                    return true;
                default:
                    return false;
            }
        }

        if (stashOnSuccess)
        {
            CaptureCurrentFormToOption(previousId, markDraft: null);
        }
        return true;
    }

    private void RestoreSelectedProviderOption(ProviderOptionViewModel? option)
    {
        if (option is not null)
        {
            SetSelectedProviderOption(option.ProviderId);
            return;
        }

        foreach (var item in ProviderOptions)
        {
            item.IsSelected = false;
        }
        _suppressProviderSelectionChange = true;
        try
        {
            _selectedProviderOption = null;
            OnPropertyChanged(nameof(SelectedProviderOption));
        }
        finally
        {
            _suppressProviderSelectionChange = false;
        }
    }

    private void CaptureCurrentFormToOption(string providerId, bool? markDraft)
    {
        var option = ProviderOptions.FirstOrDefault(p =>
            string.Equals(p.ProviderId, providerId, StringComparison.Ordinal));
        if (option is null)
        {
            return;
        }

        option.CaptureForm(new ProviderFormSnapshot
        {
            ProviderId = ProviderId,
            ProviderType = ProviderType,
            DisplayName = ProviderDisplayName,
            BaseUrl = ProviderBaseUrl,
            Enabled = ProviderEnabled,
            MakeDefaultLlm = MakeDefaultLlm,
            MakeDefaultEmbedding = MakeDefaultEmbedding,
            MakeDefaultReranker = MakeDefaultReranker,
            MakeDefaultSearch = MakeDefaultSearch,
            ModelsText = ModelsText,
            EmbeddingModelId = EmbeddingModelId,
        });
        if (markDraft is bool draftFlag)
        {
            option.IsDraft = draftFlag;
        }
    }

    private void ApplyFormSnapshot(ProviderFormSnapshot snapshot)
    {
        var wasSuppressing = _suppressDirtyTracking;
        _suppressDirtyTracking = true;
        try
        {
            ProviderId = snapshot.ProviderId;
            ProviderType = snapshot.ProviderType;
            ProviderDisplayName = snapshot.DisplayName;
            ProviderBaseUrl = snapshot.BaseUrl;
            ProviderEnabled = snapshot.Enabled;
            MakeDefaultLlm = snapshot.MakeDefaultLlm;
            MakeDefaultEmbedding = snapshot.MakeDefaultEmbedding;
            MakeDefaultReranker = snapshot.MakeDefaultReranker;
            MakeDefaultSearch = snapshot.MakeDefaultSearch;
            ApiKey = string.Empty;
            ModelsText = snapshot.ModelsText;
            EmbeddingModelId = snapshot.EmbeddingModelId;
            ManualModelsVisible = false;
            AvailableModels.Clear();
            foreach (var line in ParseModelsForDisplay(ModelsText))
            {
                AvailableModels.Add(CreateModelOption(line));
            }
        }
        finally
        {
            _suppressDirtyTracking = wasSuppressing;
        }
    }

    private static ProviderFormSnapshot CreateBlankDraftSnapshot(string id, string displayName) =>
        new()
        {
            ProviderId = id,
            ProviderType = "open_ai_compatible",
            DisplayName = displayName,
            BaseUrl = string.Empty,
            Enabled = true,
            MakeDefaultLlm = false,
            MakeDefaultEmbedding = false,
            MakeDefaultReranker = false,
            MakeDefaultSearch = false,
            ModelsText = string.Empty,
            EmbeddingModelId = string.Empty,
        };

    private void ApplyProviderForEditing(ProviderKeyStatus selected)
    {
        ProviderId = selected.Provider;
        ProviderType = selected.ProviderType;
        ProviderDisplayName = selected.DisplayName;
        ProviderBaseUrl = selected.BaseUrl ?? string.Empty;
        ProviderEnabled = selected.Enabled;
        MakeDefaultLlm = _providerConfig?.DefaultLlmProviderId == selected.Provider;
        MakeDefaultEmbedding = _providerConfig?.DefaultEmbeddingProviderId == selected.Provider;
        MakeDefaultReranker = _providerConfig?.DefaultRerankerProviderId == selected.Provider;
        MakeDefaultSearch = _providerConfig?.DefaultSearchProviderId == selected.Provider;
        ApiKey = string.Empty;
        ModelsText = string.Join(Environment.NewLine, selected.Models.Select(ModelLine));
        EmbeddingModelId = selected.Models.FirstOrDefault(IsEmbeddingModel)?.ModelId ?? string.Empty;
        ManualModelsVisible = false;
        AvailableModels.Clear();
        foreach (var model in selected.Models)
        {
            AvailableModels.Add(CreateModelOption(model));
        }
    }

    private async Task FetchModelsAsync()
    {
        var submittedProvider = SelectedProviderOption?.ProviderId;
        if (string.IsNullOrWhiteSpace(submittedProvider) || !CanUsePersistedProvider())
        {
            return;
        }
        var request = _providerModelRefreshSession.Begin();
        try
        {
            var result = await _backend
                .FetchProviderModelsAsync(submittedProvider, request.CancellationToken)
                .ConfigureAwait(true);
            if (!_providerModelRefreshSession.IsCurrent(request)
                || !string.Equals(
                    SelectedProviderOption?.ProviderId,
                    submittedProvider,
                    StringComparison.Ordinal))
            {
                return;
            }
            ProviderId = result.ProviderId;
            ModelsText = string.Join(Environment.NewLine, result.Models.Select(ModelLine));
            EmbeddingModelId = result.Models.FirstOrDefault(IsEmbeddingModel)?.ModelId ?? string.Empty;
            ManualModelsVisible = false;
            AvailableModels.Clear();
            foreach (var model in result.Models)
            {
                AvailableModels.Add(CreateModelOption(model));
            }
            UpdateDirtyState();
        }
        catch (OperationCanceledException) when (!_providerModelRefreshSession.IsCurrent(request))
        {
            // Provider 已切换、删除、重载或有更新的刷新请求；旧请求不得提交状态。
        }
        catch (Exception ex)
        {
            if (!_providerModelRefreshSession.IsCurrent(request)
                || !string.Equals(
                    SelectedProviderOption?.ProviderId,
                    submittedProvider,
                    StringComparison.Ordinal))
            {
                return;
            }
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private Task<bool> SaveGeneralAsync()
    {
        var settings = BuildGeneralSectionSettings();
        var submitted = CurrentSectionValues(GeneralSection);
        return SaveGeneralAsync(settings, submitted);
    }

    private Task<bool> SaveGeneralAsync(
        GeneralSectionSettings settings,
        IReadOnlyDictionary<string, string> submitted) =>
        RunSectionSaveAsync(GeneralSection, submitted, async () =>
        {
            await _backend.SaveGeneralSectionSettingsAsync(settings).ConfigureAwait(true);
        });

    private GeneralSectionSettings BuildGeneralSectionSettings() => new(
        new AppSettings(new AppConfig(
            _schemaVersion,
            ProjectName,
            Locale,
            DocumentsDir,
            WorkflowsDir,
            SkillsDir,
            ExportsDir)),
        ProjectMemory);

    private Task<bool> SaveModelAsync()
    {
        try
        {
            var update = BuildProviderSettingsUpdate();
            var apiKey = ApiKey;
            var submitted = CurrentSectionValues(ModelsSection);
            return SaveModelAsync(update, apiKey, submitted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
            return Task.FromResult(false);
        }
    }

    private Task<bool> SaveModelAsync(
        ProviderSettingsUpdate update,
        string apiKey,
        IReadOnlyDictionary<string, string> submitted)
    {
        try
        {
            var persisted = new Dictionary<string, string>(submitted, StringComparer.Ordinal)
            {
                [nameof(ApiKey)] = string.Empty,
            };
            return RunSectionSaveAsync(ModelsSection, submitted, async () =>
            {
                var status = await _backend.SaveProviderSectionSettingsAsync(
                    new ProviderSectionSettings(
                        update,
                        string.IsNullOrWhiteSpace(apiKey) ? null : apiKey)).ConfigureAwait(true);
                var canonicalProviderId = NormalizeProviderId(update.ProviderId);
                var saved = status.Providers.First(provider =>
                    string.Equals(provider.Provider, canonicalProviderId, StringComparison.Ordinal));
                var selectedDraft = SelectedProviderOption?.IsDraft == true
                    ? SelectedProviderOption
                    : null;
                MergeProviderConfigCache(status, preserveFormSnapshots: true);
                if (selectedDraft is not null
                    && !string.Equals(selectedDraft.ProviderId, saved.Provider, StringComparison.Ordinal)
                    && !status.Providers.Any(provider =>
                        string.Equals(provider.Provider, selectedDraft.ProviderId, StringComparison.Ordinal)))
                {
                    ProviderOptions.Remove(selectedDraft);
                }
                SetSelectedProviderOption(saved.Provider);
                ApplyCanonicalText(submitted, persisted, nameof(ProviderId), saved.Provider, value => ProviderId = value);
                ApplyCanonicalText(submitted, persisted, nameof(ProviderType), saved.ProviderType, value => ProviderType = value);
                ApplyCanonicalText(submitted, persisted, nameof(ProviderDisplayName), saved.DisplayName, value => ProviderDisplayName = value);
                ApplyCanonicalText(submitted, persisted, nameof(ProviderBaseUrl), saved.BaseUrl ?? string.Empty, value => ProviderBaseUrl = value);
                ApplyCanonicalText(submitted, persisted, nameof(ProviderEnabled), saved.Enabled.ToString(), value => ProviderEnabled = bool.Parse(value));
                ApplyCanonicalText(submitted, persisted, nameof(MakeDefaultLlm),
                    (status.DefaultLlmProviderId == saved.Provider).ToString(), value => MakeDefaultLlm = bool.Parse(value));
                ApplyCanonicalText(submitted, persisted, nameof(MakeDefaultEmbedding),
                    (status.DefaultEmbeddingProviderId == saved.Provider).ToString(), value => MakeDefaultEmbedding = bool.Parse(value));
                ApplyCanonicalText(submitted, persisted, nameof(MakeDefaultReranker),
                    (status.DefaultRerankerProviderId == saved.Provider).ToString(), value => MakeDefaultReranker = bool.Parse(value));
                ApplyCanonicalText(submitted, persisted, nameof(MakeDefaultSearch),
                    (status.DefaultSearchProviderId == saved.Provider).ToString(), value => MakeDefaultSearch = bool.Parse(value));
                ApplyCanonicalText(submitted, persisted, nameof(ModelsText),
                    string.Join(Environment.NewLine, saved.Models.Select(ModelLine)), value => ModelsText = value);
                ApplyCanonicalText(submitted, persisted, nameof(EmbeddingModelId),
                    saved.Models.FirstOrDefault(IsEmbeddingModel)?.ModelId ?? string.Empty, value => EmbeddingModelId = value);
                if (string.Equals(ApiKey, apiKey, StringComparison.Ordinal))
                {
                    ApiKey = string.Empty;
                }
            }, persisted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
            return Task.FromResult(false);
        }
    }

    private ProviderSettingsUpdate BuildProviderSettingsUpdate()
    {
        return new ProviderSettingsUpdate(
            ProviderId,
            ProviderType,
            ProviderDisplayName,
            ProviderEnabled,
            string.IsNullOrWhiteSpace(ProviderBaseUrl) ? null : ProviderBaseUrl,
            MergeEmbeddingModel(
                SettingsInputValidation.Models(ModelsText, "ui.settings.models.models"),
                EmbeddingModelId),
            MakeDefaultLlm,
            MakeDefaultEmbedding,
            MakeDefaultReranker,
            MakeDefaultSearch);
    }

    /// <summary>
    /// 用服务端状态更新 _providerConfig 与列表元数据；不重载当前编辑表单，不抹掉草稿快照。
    /// </summary>
    private void MergeProviderConfigCache(ProviderConfigStatus status, bool preserveFormSnapshots)
    {
        _providerConfig = status;
        var savedIds = new HashSet<string>(
            status.Providers.Select(p => p.Provider),
            StringComparer.Ordinal);

        foreach (var provider in status.Providers)
        {
            var existing = ProviderOptions.FirstOrDefault(o =>
                string.Equals(o.ProviderId, provider.Provider, StringComparison.Ordinal));
            var keyStatus = provider.HasKey
                ? _displayNames.Text("ui.common.configured")
                : _displayNames.Text("ui.common.not_configured");
            if (existing is not null)
            {
                existing.DisplayName = provider.DisplayName;
                existing.KeyStatus = keyStatus;
                existing.IsDraft = !provider.Configured;
                if (!preserveFormSnapshots)
                {
                    existing.ClearFormSnapshot();
                }
            }
            else
            {
                ProviderOptions.Add(CreateProviderOption(
                    provider.Provider,
                    provider.DisplayName,
                    keyStatus,
                    isDraft: !provider.Configured));
            }
        }

        // 移除已不在服务端、且非草稿的幽灵项
        for (var i = ProviderOptions.Count - 1; i >= 0; i--)
        {
            var option = ProviderOptions[i];
            if (!option.IsDraft && !savedIds.Contains(option.ProviderId))
            {
                ProviderOptions.RemoveAt(i);
            }
        }

        ProviderStatus = status.Providers.Count == 0
            ? _displayNames.Text("ui.settings.models.no_provider_status")
            : _displayNames.Format("ui.settings.models.provider_count", new Dictionary<string, string>
            {
                ["count"] = status.Providers.Count.ToString(),
            });
        RebuildAvailableLlmModelOptions();
    }

    private Task<bool> SaveProviderKeyAsync()
    {
        var providerId = ProviderId;
        var apiKey = ApiKey;
        return RunSectionSaveAsync(
            ModelsSection,
        PickValues(ModelsSection, nameof(ApiKey)),
        async () =>
        {
            var status = await _backend.SaveProviderKeyAsync(providerId, apiKey).ConfigureAwait(true);
            MergeProviderConfigCache(status, preserveFormSnapshots: true);
            if (string.Equals(ApiKey, apiKey, StringComparison.Ordinal))
            {
                ApiKey = string.Empty;
            }
        }, new Dictionary<string, string>(StringComparer.Ordinal)
        {
            [nameof(ApiKey)] = string.Empty,
        });
    }

    private async Task RemoveProviderAsync()
    {
        var providerId = SelectedProviderOption?.ProviderId;
        if (string.IsNullOrWhiteSpace(providerId) || !CanUsePersistedProvider())
        {
            return;
        }

        _providerModelRefreshSession.Invalidate();
        _providerRemovalInProgress = true;
        NotifySectionStateChanged();
        try
        {
            var preview = await _backend.PreviewProviderRemovalAsync(providerId).ConfigureAwait(true);
            if (preview.BlockingReferences.Count > 0)
            {
                await DialogService.Current.ConfirmAsync(BuildProviderRemovalBlockedDialog(preview)).ConfigureAwait(true);
                StatusText = _displayNames.Text("ui.settings.models.remove_blocked");
                return;
            }

            var confirmed = await DialogService.Current
                .ConfirmAsync(BuildProviderRemovalConfirmationDialog(preview))
                .ConfigureAwait(true);
            if (confirmed != 0)
            {
                return;
            }

            var status = await _backend
                .RemoveProviderAsync(providerId, preview.Revision)
                .ConfigureAwait(true);
            var preferredProvider = status.Providers
                .FirstOrDefault(provider => provider.Configured
                    && !string.Equals(provider.Provider, providerId, StringComparison.Ordinal))
                ?.Provider
                ?? status.Providers.FirstOrDefault(provider =>
                    !string.Equals(provider.Provider, providerId, StringComparison.Ordinal))?.Provider
                ?? status.Providers.FirstOrDefault()?.Provider;
            var wasSuppressing = _suppressDirtyTracking;
            _suppressDirtyTracking = true;
            try
            {
                MergeProviderConfigCache(status, preserveFormSnapshots: true);
                if (preferredProvider is not null)
                {
                    SelectProviderForEditing(preferredProvider);
                }
                else
                {
                    _selectedProviderOption = null;
                    OnPropertyChanged(nameof(SelectedProviderOption));
                    OnPropertyChanged(nameof(IsSelectedProviderDraft));
                }
            }
            finally
            {
                _suppressDirtyTracking = wasSuppressing;
            }
            SetSectionBaseline(ModelsSection);
            StatusText = _displayNames.Format("ui.settings.models.removed", new Dictionary<string, string>
            {
                ["provider"] = preview.DisplayName,
            });
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
        finally
        {
            _providerRemovalInProgress = false;
            NotifySectionStateChanged();
        }
    }

    private ConfirmDialogViewModel BuildProviderRemovalConfirmationDialog(ProviderRemovalPreview preview)
    {
        var roles = preview.DefaultRoles.Count == 0
            ? _displayNames.Text("ui.common.none")
            : string.Join("、", preview.DefaultRoles.Select(ProviderDefaultRoleText));
        var keyImpact = _displayNames.Text(preview.HasKey
            ? "ui.dialog.settings.remove_provider.key_present"
            : "ui.dialog.settings.remove_provider.key_absent");
        var message = _displayNames.Format("ui.dialog.settings.remove_provider.message", new Dictionary<string, string>
        {
            ["provider"] = preview.DisplayName,
            ["id"] = preview.ProviderId,
            ["roles"] = roles,
            ["key"] = keyImpact,
        });
        return new ConfirmDialogViewModel(
            _displayNames.Text("ui.dialog.settings.remove_provider.title"),
            message,
            new[]
            {
                new DialogButton(
                    _displayNames.Text("ui.dialog.settings.remove_provider.confirm"),
                    DialogButtonVariant.Danger,
                    0),
                new DialogButton(_displayNames.Text("ui.common.cancel"), DialogButtonVariant.Subtle, 1),
            })
        {
            Severity = DialogSeverity.Danger,
            ConfirmResultIndex = 0,
            CancelResultIndex = 1,
        }.SealKeyboardRoles();
    }

    private ConfirmDialogViewModel BuildProviderRemovalBlockedDialog(ProviderRemovalPreview preview)
    {
        var references = string.Join(Environment.NewLine, preview.BlockingReferences.Select(reference =>
            $"· {ProviderRemovalReferenceText(reference)}"));
        var message = _displayNames.Format("ui.dialog.settings.remove_provider.blocked_message", new Dictionary<string, string>
        {
            ["provider"] = preview.DisplayName,
            ["id"] = preview.ProviderId,
            ["references"] = references,
        });
        return new ConfirmDialogViewModel(
            _displayNames.Text("ui.dialog.settings.remove_provider.blocked_title"),
            message,
            new[]
            {
                new DialogButton(_displayNames.Text("ui.common.dismiss"), DialogButtonVariant.Subtle, 0),
            })
        {
            Severity = DialogSeverity.Warning,
            ConfirmResultIndex = -1,
            CancelResultIndex = 0,
        }.SealKeyboardRoles();
    }

    private string ProviderDefaultRoleText(string role) => role switch
    {
        "llm" => _displayNames.Text("ui.settings.models.default_role.llm"),
        "embedding" => _displayNames.Text("ui.settings.models.default_role.embedding"),
        "reranker" => _displayNames.Text("ui.settings.models.default_role.reranker"),
        "search" => _displayNames.Text("ui.settings.models.default_role.search"),
        _ => role,
    };

    private string ProviderRemovalReferenceText(ProviderRemovalReference reference)
    {
        var key = reference.ReferenceType switch
        {
            "node_preset" => "ui.dialog.settings.remove_provider.reference.node_preset",
            "workflow" => "ui.dialog.settings.remove_provider.reference.workflow",
            "active_run" => "ui.dialog.settings.remove_provider.reference.active_run",
            _ => "ui.dialog.settings.remove_provider.reference.unknown",
        };
        return _displayNames.Format(key, new Dictionary<string, string>
        {
            ["owner"] = reference.OwnerId,
            ["node"] = reference.NodeId ?? _displayNames.Text("ui.common.none"),
            ["model"] = reference.ModelId ?? _displayNames.Text("ui.common.none"),
        });
    }

    private Task<bool> SavePresetsAsync()
    {
        try
        {
            var request = BuildNodePresetSettings();
            var submitted = PickValues(
                PresetsSection,
                nameof(DefaultProviderId),
                nameof(DefaultModelId),
                nameof(DefaultTimeoutMs),
                nameof(DefaultBudgetUsd),
                nameof(NodePresets));
            return SavePresetsAsync(request, submitted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private Task<bool> SavePresetsAsync(
        NodePresetSettings request,
        IReadOnlyDictionary<string, string> submitted)
    {
        try
        {
            var persisted = new Dictionary<string, string>(submitted, StringComparer.Ordinal);
            return RunSectionSaveAsync(PresetsSection, submitted, async () =>
            {
                var saved = await _backend.SaveNodePresetSettingsAsync(request).ConfigureAwait(true);
                ApplyCanonicalText(submitted, persisted, nameof(DefaultProviderId), saved.DefaultProviderId,
                    value => ApplyDefaultModelIdentity(value, saved.DefaultModelId));
                ApplyCanonicalText(submitted, persisted, nameof(DefaultModelId), saved.DefaultModelId,
                    value => ApplyDefaultModelIdentity(saved.DefaultProviderId, value));
                ApplyCanonicalText(submitted, persisted, nameof(DefaultTimeoutMs),
                    SecondsFromStoredMs(saved.DefaultTimeoutMs), value => DefaultTimeoutMs = value);
                ApplyCanonicalText(submitted, persisted, nameof(DefaultBudgetUsd),
                    StableNumber(saved.DefaultBudgetUsd), value => DefaultBudgetUsd = value);
                if (CurrentSectionValues(PresetsSection).TryGetValue(nameof(NodePresets), out var current)
                    && submitted.TryGetValue(nameof(NodePresets), out var submittedPresets)
                    && string.Equals(current, submittedPresets, StringComparison.Ordinal))
                {
                    ApplyNodePresets(saved);
                    persisted[nameof(NodePresets)] = CurrentSectionValues(PresetsSection)[nameof(NodePresets)];
                }
                else if (submitted.TryGetValue(nameof(NodePresets), out var submittedSnapshot))
                {
                    // 保存期间继续编辑时，后端确认的是提交快照；不能用当前编辑值推进基线。
                    persisted[nameof(NodePresets)] = submittedSnapshot;
                }
            }, persisted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private NodePresetSettings BuildNodePresetSettings() => new(
        NodePresets.Select(item => new NodeTypePreset(
            item.NodeType,
            item.DisplayNameKey,
            item.ModelId,
            SettingsInputValidation.PositiveLong(
                SecondsUiToMsString(item.TimeoutMs, "ui.settings.presets.node_timeout_ms"),
                "ui.settings.presets.node_timeout_ms"),
            SettingsInputValidation.NonNegativeDouble(
                item.BudgetUsd,
                "ui.settings.presets.node_budget_usd"),
            item.Permissions.InheritGlobal ? null : item.Permissions.ToPolicy(),
            item.ToolControls.ToDictionary(
                tool => tool.ToolId,
                tool => tool.IsEnabled,
                StringComparer.Ordinal),
            item.ProviderId)).ToArray(),
        DefaultModelId,
        SettingsInputValidation.PositiveLong(
            SecondsUiToMsString(DefaultTimeoutMs, "ui.settings.presets.default_timeout_ms"),
            "ui.settings.presets.default_timeout_ms"),
        SettingsInputValidation.NonNegativeDouble(
            DefaultBudgetUsd,
            "ui.settings.presets.default_budget_usd"),
        DefaultProviderId);

    private Task<bool> SaveTemplateRepositoryAsync()
    {
        var request = new TemplateRepositorySettings(TemplateRepositoryBaseUrl);
        var submitted = CurrentSectionValues(TemplateRepositorySection);
        return SaveTemplateRepositoryAsync(request, submitted);
    }

    private Task<bool> SaveTemplateRepositoryAsync(
        TemplateRepositorySettings request,
        IReadOnlyDictionary<string, string> submitted)
    {
        return RunSectionSaveAsync(
            TemplateRepositorySection,
            submitted,
            async () =>
        {
            await _backend.SaveTemplateRepositorySettingsAsync(request).ConfigureAwait(true);
        });
    }

    private async Task OpenTemplateMarketAsync()
    {
        if (_openTemplateMarket is null)
        {
            StatusText = _displayNames.Text("ui.nav.templates");
            return;
        }

        await _openTemplateMarket().ConfigureAwait(true);
    }

    private Task<bool> SaveAutomationAsync()
    {
        try
        {
            var request = BuildAutomationSectionSettings();
            var submitted = CurrentSectionValues(AutomationSection);
            return SaveAutomationAsync(request, submitted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private Task<bool> SaveAutomationAsync(
        AutomationSectionSettings request,
        IReadOnlyDictionary<string, string> submitted)
    {
        try
        {
            var persisted = new Dictionary<string, string>(submitted, StringComparer.Ordinal);
            return RunSectionSaveAsync(AutomationSection, submitted, async () =>
            {
                var saved = await _backend.SaveAutomationSectionSettingsAsync(
                    request).ConfigureAwait(true);
                _projectAutomation.ApplyBackendValue(saved.Automation.Budget.AutoModeEnabled);
                ApplyCanonicalText(submitted, persisted, nameof(BudgetUsd),
                    StableNumber(saved.Automation.Budget.BudgetUsd), value => BudgetUsd = value);
                ApplyCanonicalText(submitted, persisted, nameof(PreauthorizedUsd),
                    StableNumber(saved.Automation.Budget.PreauthorizedUsd), value => PreauthorizedUsd = value);
                ApplyCanonicalText(submitted, persisted, nameof(WorkflowDefaultTimeoutMs),
                    SecondsFromStoredMs(saved.Workflow.Workflow.DefaultTimeoutMs), value => WorkflowDefaultTimeoutMs = value);
                ApplyCanonicalText(submitted, persisted, nameof(MaxLoopIterations),
                    saved.Workflow.Workflow.MaxLoopIterations.ToString(CultureInfo.InvariantCulture), value => MaxLoopIterations = value);
                ApplyCanonicalText(submitted, persisted, nameof(MaxToolRounds),
                    saved.Workflow.Workflow.MaxToolRounds.ToString(CultureInfo.InvariantCulture), value => MaxToolRounds = value);
                ApplyCanonicalText(submitted, persisted, nameof(RuntimeAutosaveMs),
                    saved.Workflow.Workflow.RuntimeAutosaveMs.ToString(CultureInfo.InvariantCulture), value => RuntimeAutosaveMs = value);
            }, persisted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private AutomationSectionSettings BuildAutomationSectionSettings()
    {
        var automation = new AutomationSettings(
            new BudgetStatus(
                SettingsInputValidation.NonNegativeDouble(
                    BudgetUsd,
                    "ui.settings.automation.global_budget"),
                _spentUsd,
                SettingsInputValidation.NonNegativeDouble(
                    PreauthorizedUsd,
                    "ui.settings.automation.preauthorized_budget"),
                _projectAutomation.IsEnabled),
            ConfirmationPolicies.Select(item =>
            {
                if (item.AutoModeAutoApproval && string.IsNullOrWhiteSpace(item.ApprovalPrompt))
                {
                    throw new SettingsInputException(
                        SettingsInputFailure.Required,
                        "ui.settings.automation.confirmation.approval_prompt");
                }

                return new ConfirmationPolicySetting(
                    item.Kind,
                    item.NormalPolicy,
                    item.AutoModePolicy,
                    item.ApprovalPrompt.Trim());
            }).ToArray());
        var workflow = new WorkflowSettings(new WorkflowConfig(
            _workflowSchemaVersion,
            SettingsInputValidation.PositiveLong(
                SecondsUiToMsString(WorkflowDefaultTimeoutMs, "ui.settings.automation.default_timeout_ms"),
                "ui.settings.automation.default_timeout_ms"),
            SettingsInputValidation.PositiveInt(
                MaxLoopIterations,
                "ui.settings.automation.max_loop_iterations"),
            SettingsInputValidation.PositiveInt(
                MaxToolRounds,
                "ui.settings.automation.max_tool_rounds"),
            CheckpointEnabled,
            SettingsInputValidation.PositiveLong(
                RuntimeAutosaveMs,
                "ui.settings.automation.runtime_autosave_ms")));
        return new AutomationSectionSettings(automation, workflow);
    }

    private Task<bool> SavePermissionsAsync()
    {
        try
        {
            var request = BuildPermissionsSettings();
            var submitted = CurrentSectionValues(PermissionsSection);
            return SavePermissionsAsync(request, submitted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private Task<bool> SavePermissionsAsync(
        PermissionsSettings request,
        IReadOnlyDictionary<string, string> submitted)
    {
        try
        {
            return RunSectionSaveAsync(PermissionsSection, submitted, async () =>
            {
                await _backend.SavePermissionsSettingsAsync(request).ConfigureAwait(true);
            });
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private PermissionsSettings BuildPermissionsSettings() => new(
        new PermissionPolicy(
            AllowNetwork,
            AllowWebSearch,
            AllowHttpSkill,
            AllowWasmNetwork,
            AllowSecretRead,
            SettingsInputValidation.AbsolutePaths(
                WritableRootsText,
                "ui.settings.permissions.writable_roots"),
            SettingsInputValidation.AbsolutePaths(
                ReadableRootsText,
                "ui.settings.permissions.readable_roots")),
        ScopedPermissionProfiles.ToDictionary(
            profile => profile.Scope,
            profile => profile.InheritGlobal ? null : profile.ToPolicy(),
            StringComparer.Ordinal),
        ToToolControls());

    private Task<bool> SavePersonalizationAsync()
    {
        var preferences = BuildUiPreferences();
        var submitted = CurrentSectionValues(PersonalizationSection);
        return SavePersonalizationAsync(preferences, submitted);
    }

    private Task<bool> SavePersonalizationAsync(
        UiPreferences preferences,
        IReadOnlyDictionary<string, string> submitted) =>
        RunSectionSaveAsync(PersonalizationSection, submitted, async () =>
        {
            await _saveUiPreferences(preferences).ConfigureAwait(true);
            ApplyCurrentThemeColors();
            MotionPreferences.Apply(preferences.ReduceMotion);
            _uiPreferences = preferences;
        });

    private UiPreferences BuildUiPreferences()
    {
        PersistActiveEditorsToScheme();
        return new(
            Theme,
            GitAutoColor,
            GitManualColor,
            ProjectPanelVisible,
            _uiPreferences?.ProjectPanelPosition,
            _uiPreferences?.PanelStates ?? new Dictionary<string, bool>(),
            _uiPreferences?.OnboardingSeen ?? false,
            ThemeMainColor,
            ThemeSurfaceColor,
            ThemeBrandColor,
            ThemeMainColorDark,
            ThemeSurfaceColorDark,
            ThemeBrandColorDark,
            ThemeFollowSystemColors,
            ReduceMotion,
            SelectedLanguage);
    }

    internal async Task ShowTutorialAsync()
    {
        await DialogService.Current
            .ConfirmAsync(HelpDialogFactory.CreateTutorialDialog(_displayNames))
            .ConfigureAwait(true);
    }

    private Task<bool> SaveAppRuntimeAsync()
    {
        try
        {
            var request = BuildAppRuntimeSettings();
            var submitted = CurrentSectionValues(AppRuntimeSection);
            return SaveAppRuntimeAsync(request, submitted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private Task<bool> SaveAppRuntimeAsync(
        AppRuntimeSettings request,
        IReadOnlyDictionary<string, string> submitted)
    {
        try
        {
            var persisted = new Dictionary<string, string>(submitted, StringComparer.Ordinal);
            return RunSectionSaveAsync(AppRuntimeSection, submitted, async () =>
            {
                var saved = await _backend.SaveAppRuntimeSettingsAsync(request).ConfigureAwait(true);
                ApplyCanonicalText(
                    submitted,
                    persisted,
                    nameof(QdrantStartupTimeoutMs),
                    saved.QdrantStartupTimeoutMs.ToString(CultureInfo.InvariantCulture),
                    value => QdrantStartupTimeoutMs = value);
            }, persisted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private AppRuntimeSettings BuildAppRuntimeSettings() => new(
        QdrantBinaryPath,
        SettingsInputValidation.PositiveLong(
            QdrantStartupTimeoutMs,
            "ui.settings.misc.qdrant_startup_timeout"));

    private Task<bool> SaveMiscAsync()
    {
        try
        {
            var request = BuildMiscSectionSettings();
            var submitted = CurrentSectionValues(MiscSection);
            return SaveMiscAsync(request, submitted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private Task<bool> SaveMiscAsync(
        MiscSectionSettings request,
        IReadOnlyDictionary<string, string> submitted)
    {
        try
        {
            var persisted = new Dictionary<string, string>(submitted, StringComparer.Ordinal);
            return RunSectionSaveAsync(MiscSection, submitted, async () =>
            {
                var saved = await _backend.SaveMiscSectionSettingsAsync(
                    request).ConfigureAwait(true);
                var savedRag = saved.Rag.Rag;
                ApplyCanonicalText(submitted, persisted, nameof(VectorDimensions),
                    savedRag.VectorStore.VectorDimensions.ToString(CultureInfo.InvariantCulture), value => VectorDimensions = value);
                ApplyCanonicalText(submitted, persisted, nameof(QdrantPort),
                    savedRag.VectorStore.Sidecar.Port.ToString(CultureInfo.InvariantCulture), value => QdrantPort = value);
                ApplyCanonicalText(submitted, persisted, nameof(ChunkSizeChars),
                    savedRag.ChunkSizeChars.ToString(CultureInfo.InvariantCulture), value => ChunkSizeChars = value);
                ApplyCanonicalText(submitted, persisted, nameof(ChunkOverlapChars),
                    savedRag.ChunkOverlapChars.ToString(CultureInfo.InvariantCulture), value => ChunkOverlapChars = value);
            }, persisted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private MiscSectionSettings BuildMiscSectionSettings()
    {
        var vectorDimensions = SettingsInputValidation.PositiveInt(
            VectorDimensions,
            "ui.settings.misc.vector_dimensions");
        var qdrantPort = SettingsInputValidation.PositiveInt(
            QdrantPort,
            "ui.settings.misc.qdrant_port");
        if (qdrantPort > ushort.MaxValue)
        {
            throw new SettingsInputException(
                SettingsInputFailure.Positive,
                "ui.settings.misc.qdrant_port");
        }
        var chunkSize = SettingsInputValidation.PositiveInt(
            ChunkSizeChars,
            "ui.settings.misc.chunk_size");
        var chunkOverlap = SettingsInputValidation.NonNegativeInt(
            ChunkOverlapChars,
            "ui.settings.misc.chunk_overlap");
        if (chunkOverlap >= chunkSize)
        {
            throw new SettingsInputException(
                SettingsInputFailure.Number,
                "ui.settings.misc.chunk_overlap");
        }
        var rag = new RagSettings(new RagConfig(
            _ragSchemaVersion,
            new VectorStoreConfig(
                VectorEnabled,
                VectorBackend,
                VectorCollection,
                vectorDimensions,
                new SidecarConfig(
                    QdrantHost,
                    qdrantPort,
                    QdrantDataDir,
                    "qdrant",
                    30_000)),
            new FullTextStoreConfig(_fullTextBackend, _fullTextIndexDir),
            RerankerEnabled,
            chunkSize,
            chunkOverlap));
        var git = new GitSettings(new GitConfig(
            _gitSchemaVersion,
            TrackDocuments,
            TrackWorkflows,
            TrackSkills,
            TrackNonSensitiveConfig,
            SettingsInputValidation.RelativePaths(
                IgnoredPathsText,
                "ui.settings.misc.ignored_paths")));
        return new MiscSectionSettings(rag, git);
    }

    private void ApplyAutomation(AutomationSettings automation)
    {
        BudgetUsd = automation.Budget.BudgetUsd.ToString("0.####");
        PreauthorizedUsd = automation.Budget.PreauthorizedUsd.ToString("0.####");
        _projectAutomation.ApplyBackendValue(automation.Budget.AutoModeEnabled);
        _spentUsd = automation.Budget.SpentUsd;
        SpentText = $"${automation.Budget.SpentUsd:0.####}";
        var policies = SettingsDirtyHelper.EnsureConfirmationPolicies(
                (automation.ConfirmationPolicies ?? Array.Empty<ConfirmationPolicySetting>())
                .Select(item => (item.ConfirmationKind, item.NormalPolicy, item.AutoModePolicy, item.ApprovalPrompt)));
        ApplyConfirmationPolicies(policies);
    }

    private void ApplyConfirmationPolicies(
        IReadOnlyList<(string Kind, string NormalPolicy, string AutoModePolicy, string ApprovalPrompt)> policies)
    {
        ConfirmationPolicies.Clear();
        foreach (var item in policies)
        {
            ConfirmationPolicies.Add(new ConfirmationPolicyViewModel(
                item.Kind,
                ConfirmationLabel(item.Kind),
                item.NormalPolicy,
                item.AutoModePolicy,
                item.ApprovalPrompt,
                UpdateDirtyState));
        }

        RebuildConfirmationGroups();
    }

    private void RebuildConfirmationGroups()
    {
        ConfirmationPolicyGroups.Clear();
        var order = SettingsDirtyHelper.ConfirmationSubIndexGroups.Select(g => g.Id).ToList();
        var buckets = order.ToDictionary(
            id => id,
            id => new List<ConfirmationPolicyViewModel>(),
            StringComparer.Ordinal);

        foreach (var policy in ConfirmationPolicies)
        {
            var groupId = SettingsDirtyHelper.ConfirmationGroupIdForKind(policy.Kind);
            if (!buckets.ContainsKey(groupId))
            {
                buckets[groupId] = new List<ConfirmationPolicyViewModel>();
                if (!order.Contains(groupId))
                {
                    order.Add(groupId);
                }
            }

            buckets[groupId].Add(policy);
        }

        foreach (var groupId in order)
        {
            if (!buckets.TryGetValue(groupId, out var items) || items.Count == 0)
            {
                continue;
            }

            var titleKey = SettingsDirtyHelper.ConfirmationSubIndexGroups
                .FirstOrDefault(g => g.Id == groupId).DisplayKey;
            var title = string.IsNullOrWhiteSpace(titleKey)
                ? groupId
                : _displayNames.Text(titleKey);
            ConfirmationPolicyGroups.Add(new ConfirmationPolicyGroupViewModel(groupId, title, items));
        }

        OnPropertyChanged(nameof(ConfirmationPolicyGroups));
    }

    private void EnsureDefaultConfirmationPoliciesIfEmpty()
    {
        // 不足全集时强制补齐（旧后端只回 4 项 / 空列表时）
        if (ConfirmationPolicies.Count >= SettingsDirtyHelper.DefaultConfirmationKinds.Length)
        {
            return;
        }

        var existing = ConfirmationPolicies
            .Select(p => (p.Kind, p.NormalPolicy, p.AutoModePolicy, p.ApprovalPrompt))
            .ToArray();
        ApplyConfirmationPolicies(SettingsDirtyHelper.EnsureConfirmationPolicies(existing));
    }

    private void ApplyPermissions(PermissionsSettings settings)
    {
        AllowNetwork = settings.Policy.AllowNetwork;
        AllowWebSearch = settings.Policy.AllowWebSearch;
        AllowHttpSkill = settings.Policy.AllowHttpSkill;
        AllowWasmNetwork = settings.Policy.AllowWasmNetwork;
        AllowSecretRead = settings.Policy.AllowSecretRead;
        ReadableRootsText = string.Join(Environment.NewLine, settings.Policy.ReadableFileRoots);
        WritableRootsText = string.Join(Environment.NewLine, settings.Policy.WritableFileRoots);
        ApplyScopedPermissionProfiles(settings);
        ApplyToolControls(settings.ToolControls);
    }

    private void ApplyScopedPermissionProfiles(PermissionsSettings settings)
    {
        ScopedPermissionProfiles.Clear();
        foreach (var scope in new[] { "workflow_nodes", "project_ai" })
        {
            settings.ScopedPolicies.TryGetValue(scope, out var policy);
            ScopedPermissionProfiles.Add(new PermissionScopeProfileViewModel(
                scope,
                PermissionScopeLabel(scope),
                policy,
                settings.Policy,
                () => OnScopedPermissionProfileChanged(scope)));
        }
    }

    private void OnScopedPermissionProfileChanged(string scope)
    {
        if (string.Equals(scope, "workflow_nodes", StringComparison.Ordinal))
        {
            RebindNodePresetPermissionParents();
        }
        UpdateDirtyState();
    }

    private void ApplyToolControls(IReadOnlyDictionary<string, IReadOnlyDictionary<string, bool?>>? toolControls)
    {
        ToolControlGroups.Clear();
        foreach (var (scope, controls) in (toolControls ?? new Dictionary<string, IReadOnlyDictionary<string, bool?>>()).OrderBy(item => item.Key, StringComparer.Ordinal))
        {
            var group = new ToolControlGroupViewModel(scope, ToolScopeLabel(scope));
            foreach (var (tool, enabled) in controls.OrderBy(item => item.Key, StringComparer.Ordinal))
            {
                group.Controls.Add(new ToolControlItemViewModel(
                    tool,
                    ToolLabel(scope, tool),
                    enabled,
                    ToolControlItemViewModel.IsDangerToolId(tool),
                    canInherit: scope != "global",
                    markDirty: UpdateDirtyState));
            }
            group.RefreshPartitions();
            ToolControlGroups.Add(group);
        }
    }

    private IReadOnlyDictionary<string, IReadOnlyDictionary<string, bool?>> ToToolControls()
    {
        return ToolControlGroups.ToDictionary(
            group => group.Scope,
            group => (IReadOnlyDictionary<string, bool?>)group.Controls.ToDictionary(
                item => item.ToolId,
                item => item.IsEnabled,
                StringComparer.Ordinal),
            StringComparer.Ordinal);
    }

    private void ApplyNodePresets(
        NodePresetSettings settings,
        PermissionPolicy? workflowNodePermissionPolicy = null)
    {
        var inheritedPermissionPolicy = workflowNodePermissionPolicy
                                        ?? BuildEffectiveWorkflowNodePermissionPolicy();
        NodePresets.Clear();
        foreach (var preset in settings.Presets)
        {
            NodePresets.Add(new NodeTypePresetViewModel(
                preset.NodeType,
                preset.DisplayNameKey,
                _displayNames.Text(preset.DisplayNameKey),
                preset.ProviderId,
                preset.ModelId,
                SecondsFromStoredMs(preset.TimeoutMs),
                preset.BudgetUsd.ToString("0.####"),
                preset.PermissionPolicy,
                inheritedPermissionPolicy,
                preset.ToolControls,
                tool => ToolLabel("global", tool),
                UpdateDirtyState));
        }
        RebindPresetModelOptions();
    }

    private static PermissionPolicy ResolveWorkflowNodePermissionPolicy(PermissionsSettings settings)
    {
        return settings.ScopedPolicies.TryGetValue("workflow_nodes", out var scoped)
               && scoped is not null
            ? scoped
            : settings.Policy;
    }

    private PermissionPolicy BuildEffectiveWorkflowNodePermissionPolicy()
    {
        var workflowProfile = ScopedPermissionProfiles.FirstOrDefault(profile =>
            string.Equals(profile.Scope, "workflow_nodes", StringComparison.Ordinal));
        return workflowProfile?.ToPolicy() ?? BuildGlobalPermissionPolicy();
    }

    private void RebindPermissionInheritance()
    {
        var global = BuildGlobalPermissionPolicy();
        foreach (var profile in ScopedPermissionProfiles)
        {
            profile.RebindInheritedPolicy(global);
        }
        RebindNodePresetPermissionParents();
    }

    private void RebindNodePresetPermissionParents()
    {
        var workflowNodes = BuildEffectiveWorkflowNodePermissionPolicy();
        foreach (var preset in NodePresets)
        {
            preset.Permissions.RebindInheritedPolicy(workflowNodes);
        }
    }

    /// <summary>Backend ms → author-facing seconds string (matches Workspace).</summary>
    private static string SecondsFromStoredMs(long timeoutMs) =>
        NodeTimeoutHelper.FormatSecondsFromMs(timeoutMs.ToString(CultureInfo.InvariantCulture));

    /// <summary>Author-facing seconds → ms string for PositiveLong validation / backend.</summary>
    private static string SecondsUiToMsString(string? secondsUi, string fieldKey)
    {
        var msText = NodeTimeoutHelper.ParseSecondsToMs(secondsUi);
        if (string.IsNullOrWhiteSpace(msText)
            || !long.TryParse(msText, NumberStyles.Integer, CultureInfo.InvariantCulture, out var ms)
            || ms <= 0)
        {
            throw new SettingsInputException(SettingsInputFailure.Positive, fieldKey);
        }

        return ms.ToString(CultureInfo.InvariantCulture);
    }

    private PermissionPolicy BuildGlobalPermissionPolicy() => new(
        AllowNetwork,
        AllowWebSearch,
        AllowHttpSkill,
        AllowWasmNetwork,
        AllowSecretRead,
        Lines(WritableRootsText),
        Lines(ReadableRootsText));

    private async Task<bool> RunSectionSaveAsync(
        string section,
        IReadOnlyDictionary<string, string> submittedValues,
        Func<Task> action,
        IReadOnlyDictionary<string, string>? persistedValues = null)
    {
        var attempt = _draftState.TryBeginSave(section, submittedValues);
        if (attempt is null)
        {
            StatusText = _draftState.IsLoaded(section)
                ? _displayNames.Text("ui.settings.status.saving")
                : _displayNames.Text("ui.settings.status.section_load_failed");
            return false;
        }

        StatusText = _displayNames.Text("ui.settings.status.saving");
        NotifySectionStateChanged();
        try
        {
            await action().ConfigureAwait(true);
            _draftState.CompleteSave(attempt, persistedValues);
            UpdateDirtyState();
            return true;
        }
        catch (Exception ex)
        {
            _draftState.FailSave(attempt);
            StatusText = UserFacingError.Format(ex, _displayNames);
            UpdateDirtyState(updateStatus: false);
            return false;
        }
        finally
        {
            NotifySectionStateChanged();
        }
    }

    private string ConfirmationLabel(string kind)
    {
        // 对齐 创作总结机制 / 配置项与确认项清单 的用户可见名称
        return kind switch
        {
            "chapter_write" => _displayNames.Text("ui.settings.automation.confirmation.chapter_write"),
            "summary_write" => _displayNames.Text("ui.settings.automation.confirmation.summary_write"),
            "high_risk_permission" => _displayNames.Text("ui.settings.automation.confirmation.high_risk_permission"),
            "budget_exceeded" => _displayNames.Text("ui.settings.automation.confirmation.budget_exceeded"),
            "outliner_output" => _displayNames.Text("confirmation.outliner.output"),
            "designer_output" => _displayNames.Text("confirmation.designer.output"),
            "planner_output" => _displayNames.Text("confirmation.planner.output"),
            "planner_register" => _displayNames.Text("ui.settings.automation.confirmation.planner_register_all"),
            "critic_review" => _displayNames.Text("confirmation.critic.review"),
            "prudent_review" => _displayNames.Text("confirmation.prudent.review"),
            "segment_summary" => _displayNames.Text("confirmation.summarizer.segment"),
            "event_summary" => _displayNames.Text("confirmation.summarizer.event"),
            "chapter_summary" => _displayNames.Text("confirmation.summarizer.chapter"),
            "stage_summary" => _displayNames.Text("confirmation.summarizer.stage"),
            "writer_correction_patch" => _displayNames.Text("confirmation.writer.correction_patch"),
            "polisher_correction_patch" => _displayNames.Text("confirmation.polisher.correction_patch"),
            // register 子功能：{agent}_register_{function}
            _ when kind.Contains("_register_", StringComparison.Ordinal) =>
                RegisterConfirmationLabel(kind),
            _ => kind,
        };
    }

    private string RegisterConfirmationLabel(string kind)
    {
        // outliner_register_character_trait → agent=outliner, func=character_trait
        var idx = kind.IndexOf("_register_", StringComparison.Ordinal);
        if (idx <= 0)
        {
            return kind;
        }

        var agent = kind[..idx];
        var func = kind[(idx + "_register_".Length)..];
        var agentLabel = agent switch
        {
            "outliner" => _displayNames.Text("agent.outliner"),
            "designer" => _displayNames.Text("agent.designer"),
            "planner" => _displayNames.Text("agent.planner"),
            _ => agent,
        };
        var funcKey = func switch
        {
            "character_trait" => "confirmation.planner.register.character_trait",
            "relationship" => "confirmation.planner.register.relationship",
            "foreshadowing" => "confirmation.planner.register.foreshadowing",
            "character_profile" => "confirmation.register.character_profile",
            "character_plan" => "confirmation.register.character_plan",
            "theme_anchor" => "confirmation.register.theme_anchor",
            _ => null,
        };
        var funcLabel = funcKey is null ? func : _displayNames.Text(funcKey);
        // 人物性格注册确认 → 总览者 · 人物性格注册
        var shortFunc = funcLabel
            .Replace("确认", string.Empty, StringComparison.Ordinal)
            .Trim();
        return $"{agentLabel} · {shortFunc}";
    }

    private string ToolScopeLabel(string scope)
    {
        return scope switch
        {
            "global" => _displayNames.Text("ui.settings.permissions.scope.global"),
            "project_ai" => _displayNames.Text("ui.settings.permissions.tool_scope.project_ai"),
            "llm" => _displayNames.Text("ui.settings.permissions.tool_scope.llm"),
            "executor_adapter" => _displayNames.Text("ui.settings.permissions.tool_scope.executor_adapter"),
            "outliner" => _displayNames.Text("agent.outliner"),
            "designer" => _displayNames.Text("agent.designer"),
            "planner" => _displayNames.Text("agent.planner"),
            "detail" => _displayNames.Text("agent.detail"),
            "writer" => _displayNames.Text("agent.writer"),
            "critic" => _displayNames.Text("agent.critic"),
            "prudent" => _displayNames.Text("agent.prudent"),
            "polisher" => _displayNames.Text("agent.polisher"),
            "summarizer" => _displayNames.Text("agent.summarizer"),
            _ => scope,
        };
    }

    private string PermissionScopeLabel(string scope) => scope switch
    {
        "workflow_nodes" => _displayNames.Text("ui.settings.permissions.scope.workflow_nodes"),
        "project_ai" => _displayNames.Text("ui.settings.permissions.scope.project_ai"),
        _ => scope,
    };

    private string ToolLabel(string scope, string tool)
    {
        if (tool == "project-ai-workflow-tools")
        {
            return _displayNames.Text("ui.settings.permissions.tool.project_ai_workflow_tools");
        }

        var prefix = scope.Replace("_", "-", StringComparison.Ordinal) + "-";
        var action = tool.StartsWith(prefix, StringComparison.Ordinal) ? tool[prefix.Length..] : tool;
        return action switch
        {
            "write" => _displayNames.Text("ui.settings.permissions.tool.write"),
            "workflow-tools" => _displayNames.Text("ui.settings.permissions.tool.project_ai_workflow_tools"),
            "register" => _displayNames.Text("ui.settings.permissions.tool.register"),
            "find" => _displayNames.Text("ui.settings.permissions.tool.find"),
            "search" => _displayNames.Text("ui.settings.permissions.tool.search"),
            "web-search" => _displayNames.Text("ui.settings.permissions.tool.web_search"),
            "insert-lines" => _displayNames.Text("ui.settings.permissions.tool.insert_lines"),
            "replace-lines" => _displayNames.Text("ui.settings.permissions.tool.replace_lines"),
            "rewrite-file" => _displayNames.Text("ui.settings.permissions.tool.rewrite_file"),
            _ => tool,
        };
    }

    private static string ModelLine(ModelConfig model)
    {
        return string.Join(",", new[]
        {
            model.ModelId,
            model.Capability,
            model.MaxContextTokens?.ToString(CultureInfo.InvariantCulture) ?? string.Empty,
            model.InputCostPerMillionTokens is { } input ? StableNumber(input) : string.Empty,
            model.OutputCostPerMillionTokens is { } output ? StableNumber(output) : string.Empty,
        });
    }

    private static string NormalizeProviderId(string providerId) =>
        providerId.Trim().ToLowerInvariant().Replace('-', '_');

    private static string StableNumber(double value) =>
        value.ToString("0.####", CultureInfo.InvariantCulture);

    private void ApplyCanonicalText(
        IReadOnlyDictionary<string, string> submitted,
        IDictionary<string, string> persisted,
        string field,
        string canonical,
        Action<string> apply)
    {
        persisted[field] = canonical;
        var current = CurrentValues();
        if (submitted.TryGetValue(field, out var submittedValue)
            && current.TryGetValue(field, out var currentValue)
            && string.Equals(currentValue, submittedValue, StringComparison.Ordinal))
        {
            apply(canonical);
        }
    }

    private static IReadOnlyList<ModelConfig> ParseModelsForDisplay(string text)
    {
        try
        {
            return SettingsInputValidation.Models(text, "ui.settings.models.models");
        }
        catch (SettingsInputException)
        {
            return Array.Empty<ModelConfig>();
        }
    }

    private void RebuildAvailableLlmModelOptions()
    {
        AvailableLlmModelOptions.Clear();
        if (_providerConfig is not null)
        {
            foreach (var provider in _providerConfig.Providers
                         .Where(provider => provider.Configured && provider.Enabled)
                         .OrderBy(provider => provider.DisplayName, StringComparer.Ordinal)
                         .ThenBy(provider => provider.Provider, StringComparer.Ordinal))
            {
                foreach (var model in provider.Models
                             .Where(model => string.Equals(model.Capability, "llm", StringComparison.OrdinalIgnoreCase)
                                             || string.Equals(model.Capability, "tool_use", StringComparison.OrdinalIgnoreCase))
                             .Where(model => !string.IsNullOrWhiteSpace(model.ModelId))
                             .OrderBy(model => model.ModelId, StringComparer.Ordinal))
                {
                    AvailableLlmModelOptions.Add(new WorkflowModelOption(
                        provider.Provider,
                        model.ModelId,
                        string.IsNullOrWhiteSpace(provider.DisplayName)
                            ? provider.Provider
                            : provider.DisplayName));
                }
            }
        }

        RebindDefaultModelOption();
        RebindPresetModelOptions();
    }

    private void ApplyDefaultModelIdentity(string providerId, string modelId)
    {
        SetProperty(ref _defaultProviderId, providerId?.Trim() ?? string.Empty, nameof(DefaultProviderId));
        SetProperty(ref _defaultModelId, modelId?.Trim() ?? string.Empty, nameof(DefaultModelId));
        RebindDefaultModelOption();
    }

    private void RebindDefaultModelOption()
    {
        var candidates = AvailableLlmModelOptions
            .Where(option => string.Equals(option.ModelId, _defaultModelId, StringComparison.Ordinal))
            .Take(2)
            .ToArray();
        var selected = string.IsNullOrWhiteSpace(_defaultProviderId)
            ? (candidates.Length == 1 ? candidates[0] : null)
            : candidates.FirstOrDefault(option =>
                string.Equals(option.ProviderId, _defaultProviderId, StringComparison.Ordinal));
        SetProperty(ref _selectedDefaultModelOption, selected, nameof(SelectedDefaultModelOption));
    }

    private void RebindPresetModelOptions()
    {
        foreach (var preset in NodePresets)
        {
            preset.RebindModelOptions(AvailableLlmModelOptions);
        }
    }

    private static IReadOnlyList<ModelConfig> MergeEmbeddingModel(IReadOnlyList<ModelConfig> models, string embeddingModelId)
    {
        var merged = models
            .Where(model => !string.IsNullOrWhiteSpace(model.ModelId))
            .ToList();
        var trimmed = embeddingModelId.Trim();
        if (string.IsNullOrWhiteSpace(trimmed))
        {
            return merged;
        }

        var existing = merged.FindIndex(model => string.Equals(model.ModelId, trimmed, StringComparison.Ordinal));
        if (existing >= 0)
        {
            var model = merged[existing];
            if (!IsEmbeddingModel(model))
            {
                throw new SettingsInputException(
                    SettingsInputFailure.ModelLine,
                    "ui.settings.models.embedding_model");
            }
        }
        else
        {
            merged.Add(new ModelConfig(trimmed, "embedding", null, null, null));
        }

        return merged;
    }

    private ModelOptionViewModel CreateModelOption(ModelConfig model) =>
        new(model.ModelId, model.Capability, ModelCapabilityLabel(model.Capability));

    private string ModelCapabilityLabel(string capability) =>
        _displayNames.Text($"ui.settings.models.capability.{capability}");

    private static bool IsEmbeddingModel(ModelConfig model)
    {
        return string.Equals(model.Capability, "embedding", StringComparison.OrdinalIgnoreCase);
    }

    private static IReadOnlyList<string> Lines(string text)
    {
        return text
            .Split(new[] { "\r\n", "\n" }, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
            .Where(line => !string.IsNullOrWhiteSpace(line))
            .ToArray();
    }

    public string UnsavedChangesPageTitle => Title;
    public string UnsavedChangesPageId => "settings";
    public string? PreparedUnsavedChangesPayloadIdentity => CreatePreparedPayloadIdentity();

    public async Task<bool> ConfirmLeaveIfNeededAsync()
    {
        if (!HasUnsavedChanges)
        {
            return true;
        }

        var choice = await DialogService.Current.ConfirmUnsavedLeaveAsync(UnsavedChangesPageTitle).ConfigureAwait(true);
        switch (choice)
        {
            case UnsavedLeaveChoice.Save:
                return await SaveUnsavedChangesAsync().ConfigureAwait(true);
            case UnsavedLeaveChoice.Discard:
                await DiscardUnsavedChangesAsync().ConfigureAwait(true);
                return true;
            default:
                return false;
        }
    }

    private bool _leavePrepared;
    private Dictionary<string, IReadOnlyDictionary<string, string>>? _preparedSettingsSections;
    private IReadOnlyList<PreparedSettingsCommit>? _preparedSettingsCommits;

    private string? CreatePreparedPayloadIdentity()
    {
        if (_preparedSettingsSections is null)
        {
            return null;
        }

        var redacted = _preparedSettingsSections.ToDictionary(
            section => section.Key,
            section => section.Value.ToDictionary(
                value => value.Key,
                value => string.Equals(value.Key, nameof(ApiKey), StringComparison.Ordinal)
                    ? "<redacted>"
                    : value.Value,
                StringComparer.Ordinal),
            StringComparer.Ordinal);
        return BatchLeaveSaveCoordinator.CreatePayloadIdentity(
            System.Text.Json.JsonSerializer.Serialize(redacted));
    }

    public Task<bool> PrepareUnsavedChangesAsync()
    {
        _leavePrepared = false;
        _preparedSettingsSections = null;
        _preparedSettingsCommits = null;
        if (!HasUnsavedChanges)
        {
            _leavePrepared = true;
            _preparedSettingsSections = new(StringComparer.Ordinal);
            _preparedSettingsCommits = Array.Empty<PreparedSettingsCommit>();
            return Task.FromResult(true);
        }

        try
        {
            _preparedSettingsCommits = BuildPreparedSettingsCommits();
            _preparedSettingsSections = CaptureDirtySettingsSections();
            _leavePrepared = true;
            return Task.FromResult(true);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
            return Task.FromResult(false);
        }
    }

    public async Task<bool> CommitPreparedUnsavedChangesAsync()
    {
        if (!_leavePrepared)
        {
            return false;
        }

        if (!HasUnsavedChanges)
        {
            _leavePrepared = false;
            _preparedSettingsSections = null;
            _preparedSettingsCommits = null;
            return true;
        }

        try
        {
            if (_preparedSettingsSections is null
                || _preparedSettingsCommits is null
                || !PreparedSettingsSectionsStillMatch())
            {
                return false;
            }
            var saved = true;
            foreach (var prepared in _preparedSettingsCommits)
            {
                if (!await prepared.Save().ConfigureAwait(true))
                {
                    saved = false;
                    break;
                }
            }
            var ok = saved && SettingsDirtyHelper.CanNavigateAfterLeaveSave(HasUnsavedChanges);
            if (ok)
            {
                _leavePrepared = false;
            }

            return ok;
        }
        catch
        {
            return false;
        }
        finally
        {
            _leavePrepared = false;
            _preparedSettingsSections = null;
            _preparedSettingsCommits = null;
        }
    }

    public Task AbortPreparedUnsavedChangesAsync()
    {
        _leavePrepared = false;
        _preparedSettingsSections = null;
        _preparedSettingsCommits = null;
        return Task.CompletedTask;
    }

    public async Task<bool> SaveUnsavedChangesAsync()
    {
        if (!await PrepareUnsavedChangesAsync().ConfigureAwait(true))
        {
            return false;
        }

        return await CommitPreparedUnsavedChangesAsync().ConfigureAwait(true);
    }

    public async Task DiscardUnsavedChangesAsync()
    {
        await AbortPreparedUnsavedChangesAsync().ConfigureAwait(true);
        if (HasUnsavedChanges)
        {
            await LoadAsync().ConfigureAwait(true);
        }
    }

    public Task ReloadProjectDataAsync(CancellationToken cancellationToken = default) => LoadAsync(cancellationToken);

    public void DeactivateProjectData()
    {
        _draftState.BeginLoad();
    }

    private IReadOnlyList<PreparedSettingsCommit> BuildPreparedSettingsCommits()
    {
        var commits = new List<PreparedSettingsCommit>();
        if (_draftState.IsSectionDirty(GeneralSection, CurrentSectionValues(GeneralSection)))
        {
            var request = BuildGeneralSectionSettings();
            var submitted = CurrentSectionValues(GeneralSection);
            commits.Add(new(GeneralSection, () => SaveGeneralAsync(request, submitted)));
        }
        if (_draftState.IsSectionDirty(ModelsSection, CurrentSectionValues(ModelsSection)))
        {
            var request = BuildProviderSettingsUpdate();
            var apiKey = ApiKey;
            var submitted = CurrentSectionValues(ModelsSection);
            commits.Add(new(ModelsSection, () => SaveModelAsync(request, apiKey, submitted)));
        }
        if (_draftState.IsSectionDirty(PresetsSection, CurrentSectionValues(PresetsSection)))
        {
            var request = BuildNodePresetSettings();
            var submitted = PickValues(
                PresetsSection,
                nameof(DefaultProviderId),
                nameof(DefaultModelId),
                nameof(DefaultTimeoutMs),
                nameof(DefaultBudgetUsd),
                nameof(NodePresets));
            commits.Add(new(PresetsSection, () => SavePresetsAsync(request, submitted)));
        }
        if (_draftState.IsSectionDirty(
                TemplateRepositorySection,
                CurrentSectionValues(TemplateRepositorySection)))
        {
            var request = new TemplateRepositorySettings(TemplateRepositoryBaseUrl);
            var submitted = CurrentSectionValues(TemplateRepositorySection);
            commits.Add(new(
                TemplateRepositorySection,
                () => SaveTemplateRepositoryAsync(request, submitted)));
        }
        if (_draftState.IsSectionDirty(AutomationSection, CurrentSectionValues(AutomationSection)))
        {
            var request = BuildAutomationSectionSettings();
            var submitted = CurrentSectionValues(AutomationSection);
            commits.Add(new(AutomationSection, () => SaveAutomationAsync(request, submitted)));
        }
        if (_draftState.IsSectionDirty(PermissionsSection, CurrentSectionValues(PermissionsSection)))
        {
            var request = BuildPermissionsSettings();
            var submitted = CurrentSectionValues(PermissionsSection);
            commits.Add(new(PermissionsSection, () => SavePermissionsAsync(request, submitted)));
        }
        if (_draftState.IsSectionDirty(PersonalizationSection, CurrentSectionValues(PersonalizationSection)))
        {
            var request = BuildUiPreferences();
            var submitted = CurrentSectionValues(PersonalizationSection);
            commits.Add(new(PersonalizationSection, () => SavePersonalizationAsync(request, submitted)));
        }
        if (_draftState.IsSectionDirty(AppRuntimeSection, CurrentSectionValues(AppRuntimeSection)))
        {
            var request = BuildAppRuntimeSettings();
            var submitted = CurrentSectionValues(AppRuntimeSection);
            commits.Add(new(AppRuntimeSection, () => SaveAppRuntimeAsync(request, submitted)));
        }
        if (_draftState.IsSectionDirty(MiscSection, CurrentSectionValues(MiscSection)))
        {
            var request = BuildMiscSectionSettings();
            var submitted = CurrentSectionValues(MiscSection);
            commits.Add(new(MiscSection, () => SaveMiscAsync(request, submitted)));
        }
        return commits;
    }

    private Dictionary<string, IReadOnlyDictionary<string, string>> CaptureDirtySettingsSections()
    {
        var result = new Dictionary<string, IReadOnlyDictionary<string, string>>(StringComparer.Ordinal);
        foreach (var section in new[]
                 {
                     GeneralSection,
                     ModelsSection,
                     PresetsSection,
                     TemplateRepositorySection,
                     AutomationSection,
                     PermissionsSection,
                     PersonalizationSection,
                     AppRuntimeSection,
                     MiscSection,
                 })
        {
            var values = CurrentSectionValues(section);
            if (_draftState.IsSectionDirty(section, values))
            {
                result[section] = new Dictionary<string, string>(values, StringComparer.Ordinal);
            }
        }
        return result;
    }

    private bool PreparedSettingsSectionsStillMatch()
    {
        var current = CaptureDirtySettingsSections();
        if (_preparedSettingsSections is null
            || current.Count != _preparedSettingsSections.Count)
        {
            return false;
        }

        return current.All(pair => _preparedSettingsSections.TryGetValue(pair.Key, out var prepared)
            && prepared.Count == pair.Value.Count
            && pair.Value.All(value => prepared.TryGetValue(value.Key, out var expected)
                && string.Equals(expected, value.Value, StringComparison.Ordinal)));
    }

    protected override void OnPropertyChanged(string? propertyName = null)
    {
        base.OnPropertyChanged(propertyName);
        if (!_suppressDirtyTracking && IsGlobalPermissionProperty(propertyName))
        {
            RebindPermissionInheritance();
        }
        if (!_suppressDirtyTracking && IsTrackedDirtyProperty(propertyName))
        {
            UpdateDirtyState();
        }
    }

    private static bool IsGlobalPermissionProperty(string? propertyName) => propertyName is
        nameof(AllowNetwork) or nameof(AllowWebSearch) or nameof(AllowHttpSkill)
        or nameof(AllowWasmNetwork) or nameof(AllowSecretRead)
        or nameof(ReadableRootsText) or nameof(WritableRootsText);

    private ThemeOption CreateThemeOption(ThemePalette palette, DisplayNameService displayNames)
    {
        return new ThemeOption(
            palette.Id,
            palette.Group,
            ThemeGroupTitleFor(palette.Group, displayNames),
            ThemeLabelFor(palette.Id, displayNames),
            ThemeDescriptionFor(palette.Id, displayNames),
            new SolidColorBrush(palette.SwatchMain),
            new SolidColorBrush(palette.SwatchSurface),
            new SolidColorBrush(palette.SwatchBrand),
            option => SelectedThemeOption = option);
    }

    private void SyncThemeOptionSelection()
    {
        foreach (var option in ThemeOptions)
        {
            option.IsSelected = string.Equals(option.Code, Theme, StringComparison.OrdinalIgnoreCase);
        }
    }

    private static string ThemeLabelFor(string code, DisplayNameService displayNames)
    {
        var key = $"ui.theme.{code}";
        var text = displayNames.Text(key);
        return text.StartsWith('[') ? displayNames.Text($"ui.settings.personalization.theme.{code}") : text;
    }

    private static string ThemeDescriptionFor(string code, DisplayNameService displayNames) =>
        displayNames.Text($"ui.theme.{code}.desc");

    private static string ThemeGroupTitleFor(string group, DisplayNameService displayNames) => group switch
    {
        "light_accent" => displayNames.Text("ui.settings.personalization.theme.group.light_accent"),
        "dark_accent" => displayNames.Text("ui.settings.personalization.theme.group.dark_accent"),
        _ => displayNames.Text("ui.settings.personalization.theme.group.base"),
    };

    private void RefreshLocalizedText()
    {
        foreach (var propertyName in LocalizedPropertyNames)
        {
            OnPropertyChanged(propertyName);
        }

        foreach (var option in LanguageOptions)
        {
            option.Label = _displayNames.LanguageLabel(option.Code);
        }

        foreach (var option in VectorBackendOptions)
        {
            option.Label = _displayNames.Text(option.Value switch
            {
                "external_qdrant" => "ui.settings.misc.vector_backend.external",
                _ => "ui.settings.misc.vector_backend.sidecar",
            });
        }

        foreach (var option in ProviderTypeOptions)
        {
            option.Label = _displayNames.Text($"ui.settings.models.provider_type.{option.Value}");
        }
        foreach (var model in AvailableModels)
        {
            model.CapabilityLabel = ModelCapabilityLabel(model.Capability);
        }

        foreach (var option in ThemeOptions)
        {
            option.Label = ThemeLabelFor(option.Code, _displayNames);
            option.Description = ThemeDescriptionFor(option.Code, _displayNames);
            option.GroupTitle = ThemeGroupTitleFor(option.Group, _displayNames);
        }
        OnPropertyChanged(nameof(ThemeOptionGroups));

        foreach (var tab in Tabs)
        {
            var definition = SettingsNavigationCatalog.Tabs.FirstOrDefault(candidate =>
                string.Equals(candidate.Id, tab.Id, StringComparison.Ordinal));
            if (definition is not null)
            {
                tab.Title = _displayNames.Text(definition.DisplayNameKey);
            }
        }
        foreach (var section in SectionIndexItems)
        {
            var definition = SettingsNavigationCatalog.Sections.FirstOrDefault(candidate =>
                string.Equals(candidate.Id, section.Id, StringComparison.Ordinal));
            if (definition is not null)
            {
                section.Title = _displayNames.Text(definition.DisplayNameKey);
            }
        }
        foreach (var policy in ConfirmationPolicies)
        {
            policy.Label = ConfirmationLabel(policy.Kind);
        }

        foreach (var preset in NodePresets)
        {
            preset.DisplayName = _displayNames.Text(preset.DisplayNameKey);
        }

        foreach (var group in ToolControlGroups)
        {
            group.DisplayName = ToolScopeLabel(group.Scope);
            foreach (var control in group.Controls)
            {
                control.DisplayName = ToolLabel(group.Scope, control.ToolId);
            }
        }
        foreach (var profile in ScopedPermissionProfiles)
        {
            profile.DisplayName = PermissionScopeLabel(profile.Scope);
        }

    }

    void ILocalizedUiAware.RefreshLocalizedUi() => RefreshLocalizedText();

    private IReadOnlyDictionary<string, string> CurrentValues()
    {
        var confirmationSnapshot = string.Join("|", ConfirmationPolicies.Select(policy =>
            $"{SnapshotPart(policy.Kind)}:{SnapshotPart(policy.NormalPolicy)}:{SnapshotPart(policy.AutoModePolicy)}:{SnapshotPart(policy.ApprovalPrompt)}"));
        var toolControlSnapshot = string.Join("|", ToolControlGroups.SelectMany(group =>
            group.Controls.Select(item => $"{group.Scope}:{item.ToolId}:{item.IsEnabled?.ToString() ?? "inherit"}")));
        var scopedPermissionSnapshot = string.Join("|", ScopedPermissionProfiles.Select(profile => profile.Snapshot));
        return new Dictionary<string, string>(StringComparer.Ordinal)
        {
            [nameof(ProjectName)] = ProjectName,
            [nameof(Locale)] = Locale,
            [nameof(DocumentsDir)] = DocumentsDir,
            [nameof(WorkflowsDir)] = WorkflowsDir,
            [nameof(SkillsDir)] = SkillsDir,
            [nameof(ExportsDir)] = ExportsDir,
            [nameof(ProjectMemory)] = ProjectMemory,
            [nameof(ProviderId)] = ProviderId,
            [nameof(ProviderType)] = ProviderType,
            [nameof(ProviderDisplayName)] = ProviderDisplayName,
            [nameof(ProviderBaseUrl)] = ProviderBaseUrl,
            [nameof(ProviderEnabled)] = ProviderEnabled.ToString(),
            [nameof(MakeDefaultLlm)] = MakeDefaultLlm.ToString(),
            [nameof(MakeDefaultEmbedding)] = MakeDefaultEmbedding.ToString(),
            [nameof(MakeDefaultReranker)] = MakeDefaultReranker.ToString(),
            [nameof(MakeDefaultSearch)] = MakeDefaultSearch.ToString(),
            [nameof(ApiKey)] = ApiKey,
            [nameof(ModelsText)] = ModelsText,
            [nameof(EmbeddingModelId)] = EmbeddingModelId,
            [nameof(DefaultProviderId)] = DefaultProviderId,
            [nameof(DefaultModelId)] = DefaultModelId,
            [nameof(DefaultTimeoutMs)] = DefaultTimeoutMs,
            [nameof(DefaultBudgetUsd)] = DefaultBudgetUsd,
            [nameof(NodePresets)] = string.Join("|", NodePresets.Select(preset => preset.Snapshot)),
            [nameof(TemplateRepositoryBaseUrl)] = TemplateRepositoryBaseUrl,
            [nameof(BudgetUsd)] = BudgetUsd,
            [nameof(PreauthorizedUsd)] = PreauthorizedUsd,
            [nameof(WorkflowDefaultTimeoutMs)] = WorkflowDefaultTimeoutMs,
            [nameof(MaxLoopIterations)] = MaxLoopIterations,
            [nameof(MaxToolRounds)] = MaxToolRounds,
            [nameof(CheckpointEnabled)] = CheckpointEnabled.ToString(),
            [nameof(RuntimeAutosaveMs)] = RuntimeAutosaveMs,
            [nameof(ConfirmationPolicies)] = confirmationSnapshot,
            [nameof(AllowNetwork)] = AllowNetwork.ToString(),
            [nameof(AllowWebSearch)] = AllowWebSearch.ToString(),
            [nameof(AllowHttpSkill)] = AllowHttpSkill.ToString(),
            [nameof(AllowWasmNetwork)] = AllowWasmNetwork.ToString(),
            [nameof(AllowSecretRead)] = AllowSecretRead.ToString(),
            [nameof(ReadableRootsText)] = ReadableRootsText,
            [nameof(WritableRootsText)] = WritableRootsText,
            [nameof(ToolControlGroups)] = toolControlSnapshot,
            [nameof(ScopedPermissionProfiles)] = scopedPermissionSnapshot,
            [nameof(Theme)] = Theme,
            [nameof(ThemeMainColor)] = ThemeMainColor,
            [nameof(ThemeSurfaceColor)] = ThemeSurfaceColor,
            [nameof(ThemeBrandColor)] = ThemeBrandColor,
            [nameof(ThemeMainColorDark)] = ThemeMainColorDark,
            [nameof(ThemeSurfaceColorDark)] = ThemeSurfaceColorDark,
            [nameof(ThemeBrandColorDark)] = ThemeBrandColorDark,
            [nameof(ThemeFollowSystemColors)] = ThemeFollowSystemColors.ToString(),
            [nameof(GitAutoColor)] = GitAutoColor,
            [nameof(GitManualColor)] = GitManualColor,
            [nameof(ProjectPanelVisible)] = ProjectPanelVisible.ToString(),
            [nameof(ReduceMotion)] = ReduceMotion.ToString(),
            [nameof(SelectedLanguage)] = SelectedLanguage,
            [nameof(VectorEnabled)] = VectorEnabled.ToString(),
            [nameof(VectorBackend)] = VectorBackend,
            [nameof(VectorCollection)] = VectorCollection,
            [nameof(VectorDimensions)] = VectorDimensions,
            [nameof(QdrantHost)] = QdrantHost,
            [nameof(QdrantPort)] = QdrantPort,
            [nameof(QdrantDataDir)] = QdrantDataDir,
            [nameof(QdrantBinaryPath)] = QdrantBinaryPath,
            [nameof(QdrantStartupTimeoutMs)] = QdrantStartupTimeoutMs,
            [nameof(RerankerEnabled)] = RerankerEnabled.ToString(),
            [nameof(ChunkSizeChars)] = ChunkSizeChars,
            [nameof(ChunkOverlapChars)] = ChunkOverlapChars,
            [nameof(TrackDocuments)] = TrackDocuments.ToString(),
            [nameof(TrackWorkflows)] = TrackWorkflows.ToString(),
            [nameof(TrackSkills)] = TrackSkills.ToString(),
            [nameof(TrackNonSensitiveConfig)] = TrackNonSensitiveConfig.ToString(),
            [nameof(IgnoredPathsText)] = IgnoredPathsText,
        };
    }

    private static string SnapshotPart(string value)
    {
        value ??= string.Empty;
        return $"{value.Length}:{value}";
    }

    private IReadOnlyDictionary<string, string> CurrentSectionValues(string section)
    {
        var fields = section switch
        {
            GeneralSection => new[]
            {
                nameof(ProjectName), nameof(DocumentsDir), nameof(WorkflowsDir),
                nameof(SkillsDir), nameof(ExportsDir), nameof(ProjectMemory),
            },
            ModelsSection => new[]
            {
                nameof(ProviderId), nameof(ProviderType), nameof(ProviderDisplayName),
                nameof(ProviderBaseUrl), nameof(ProviderEnabled), nameof(MakeDefaultLlm),
                nameof(MakeDefaultEmbedding), nameof(MakeDefaultReranker), nameof(MakeDefaultSearch), nameof(ApiKey),
                nameof(ModelsText), nameof(EmbeddingModelId),
            },
            PresetsSection => new[]
            {
                nameof(DefaultProviderId), nameof(DefaultModelId), nameof(DefaultTimeoutMs), nameof(DefaultBudgetUsd),
                nameof(NodePresets),
            },
            TemplateRepositorySection => new[] { nameof(TemplateRepositoryBaseUrl) },
            AutomationSection => new[]
            {
                nameof(BudgetUsd), nameof(PreauthorizedUsd),
                nameof(WorkflowDefaultTimeoutMs), nameof(MaxLoopIterations), nameof(MaxToolRounds),
                nameof(CheckpointEnabled), nameof(RuntimeAutosaveMs), nameof(ConfirmationPolicies),
            },
            PermissionsSection => new[]
            {
                nameof(AllowNetwork), nameof(AllowWebSearch), nameof(AllowHttpSkill),
                nameof(AllowWasmNetwork), nameof(AllowSecretRead), nameof(ReadableRootsText),
                nameof(WritableRootsText), nameof(ToolControlGroups),
                nameof(ScopedPermissionProfiles),
            },
            PersonalizationSection => new[]
            {
                nameof(Theme), nameof(ThemeMainColor), nameof(ThemeSurfaceColor),
                nameof(ThemeBrandColor), nameof(ThemeMainColorDark), nameof(ThemeSurfaceColorDark),
                nameof(ThemeBrandColorDark), nameof(ThemeFollowSystemColors), nameof(GitAutoColor),
                nameof(GitManualColor), nameof(ProjectPanelVisible), nameof(ReduceMotion),
                nameof(SelectedLanguage),
            },
            AppRuntimeSection => new[]
            {
                nameof(QdrantBinaryPath), nameof(QdrantStartupTimeoutMs),
            },
            MiscSection => new[]
            {
                nameof(VectorEnabled), nameof(VectorBackend), nameof(VectorCollection),
                nameof(VectorDimensions), nameof(QdrantHost), nameof(QdrantPort),
                nameof(QdrantDataDir),
                nameof(RerankerEnabled), nameof(ChunkSizeChars), nameof(ChunkOverlapChars),
                nameof(TrackDocuments), nameof(TrackWorkflows), nameof(TrackSkills),
                nameof(TrackNonSensitiveConfig), nameof(IgnoredPathsText),
            },
            _ => Array.Empty<string>(),
        };
        var current = CurrentValues();
        return fields.ToDictionary(field => field, field => current[field], StringComparer.Ordinal);
    }

    private IReadOnlyDictionary<string, string> PickValues(string section, params string[] fields)
    {
        var current = CurrentSectionValues(section);
        return fields.ToDictionary(field => field, field => current[field], StringComparer.Ordinal);
    }

    private void SetSectionBaseline(string section)
    {
        _draftState.SetBaseline(section, CurrentSectionValues(section));
        NotifySectionStateChanged();
        UpdateDirtyState();
    }

    private bool CanSave(string section) =>
        _draftState.IsLoaded(section)
        && !_draftState.IsSaving(section)
        && !(section == ModelsSection && _providerRemovalInProgress);

    private bool CanUsePersistedProvider()
    {
        var selected = SelectedProviderOption;
        return CanSave(ModelsSection)
            && selected is not null
            && !selected.IsDraft
            && _providerConfig?.Providers.Any(provider =>
                provider.Configured
                && string.Equals(provider.Provider, selected.ProviderId, StringComparison.Ordinal)) == true;
    }

    private void NotifyProviderCommands()
    {
        RefreshModelsCommand?.NotifyCanExecuteChanged();
        SaveProviderKeyCommand?.NotifyCanExecuteChanged();
        RemoveProviderCommand?.NotifyCanExecuteChanged();
    }

    private void UpdateDirtyState() => UpdateDirtyState(updateStatus: true);

    private void UpdateDirtyState(bool updateStatus)
    {
        var current = CurrentValues();
        HasUnsavedChanges = _draftState.IsDirty(current);
        if (!updateStatus)
        {
            return;
        }

        if (_draftState.IsAnySaving)
        {
            StatusText = _displayNames.Text("ui.settings.status.saving");
            return;
        }

        if (_draftState.HasUnsubmittedChanges(current))
        {
            var dirtyTitles = DirtySectionTitles();
            StatusText = dirtyTitles.Count == 0
                ? _displayNames.Text("ui.settings.status.unsaved")
                : _displayNames.Format(
                    "ui.settings.status.unsaved_sections",
                    new Dictionary<string, string> { ["sections"] = string.Join("、", dirtyTitles) });
            return;
        }

        StatusText = _displayNames.Text("ui.common.configured");
    }

    private List<string> DirtySectionTitles()
    {
        var titles = new List<string>();
        void AddIfDirty(string section, string title)
        {
            if (_draftState.IsSectionDirty(section, CurrentSectionValues(section)))
            {
                titles.Add(title);
            }
        }

        AddIfDirty(GeneralSection, GeneralTitle);
        AddIfDirty(ModelsSection, ModelsTitle);
        if (_draftState.IsSectionDirty(PresetsSection, CurrentSectionValues(PresetsSection))
            || _draftState.IsSectionDirty(TemplateRepositorySection, CurrentSectionValues(TemplateRepositorySection)))
        {
            titles.Add(PresetsTitle);
        }
        AddIfDirty(AutomationSection, AutomationTitle);
        AddIfDirty(PermissionsSection, PermissionsTitle);
        AddIfDirty(PersonalizationSection, PersonalizationTitle);
        AddIfDirty(AppRuntimeSection, AppRuntimeSectionTitle);
        AddIfDirty(MiscSection, MiscTitle);
        return titles;
    }

    private void SetValidationStatus(SettingsInputException exception)
    {
        var field = _displayNames.Text(exception.FieldKey);
        var key = exception.Failure switch
        {
            SettingsInputFailure.Positive => "ui.settings.validation.positive",
            SettingsInputFailure.NonNegative => "ui.settings.validation.non_negative",
            SettingsInputFailure.ModelLine => "ui.settings.validation.model_line",
            SettingsInputFailure.PathLine => "ui.settings.validation.path_line",
            SettingsInputFailure.Required => "ui.settings.validation.required",
            _ => "ui.settings.validation.number",
        };
        StatusText = _displayNames.Format(key, new Dictionary<string, string>
        {
            ["field"] = field,
            ["line"] = exception.Line?.ToString() ?? string.Empty,
        });
    }

    private void NotifySectionStateChanged()
    {
        OnPropertyChanged(nameof(IsGeneralEditable));
        OnPropertyChanged(nameof(IsModelsEditable));
        OnPropertyChanged(nameof(IsPresetsEditable));
        OnPropertyChanged(nameof(IsTemplateRepositoryEditable));
        OnPropertyChanged(nameof(IsAutomationEditable));
        OnPropertyChanged(nameof(IsPermissionsEditable));
        OnPropertyChanged(nameof(IsPersonalizationEditable));
        OnPropertyChanged(nameof(IsAppRuntimeEditable));
        OnPropertyChanged(nameof(IsMiscEditable));
        NotifySaveCommands();
    }

    private void NotifySaveCommands()
    {
        SaveGeneralCommand?.NotifyCanExecuteChanged();
        RefreshModelsCommand?.NotifyCanExecuteChanged();
        SaveModelCommand?.NotifyCanExecuteChanged();
        SaveProviderKeyCommand?.NotifyCanExecuteChanged();
        RemoveProviderCommand?.NotifyCanExecuteChanged();
        AddProviderCommand?.NotifyCanExecuteChanged();
        SavePresetsCommand?.NotifyCanExecuteChanged();
        SaveTemplateRepositoryCommand?.NotifyCanExecuteChanged();
        SaveAutomationCommand?.NotifyCanExecuteChanged();
        SavePermissionsCommand?.NotifyCanExecuteChanged();
        SavePersonalizationCommand?.NotifyCanExecuteChanged();
        SaveAppRuntimeCommand?.NotifyCanExecuteChanged();
        SaveMiscCommand?.NotifyCanExecuteChanged();
    }

    private static bool IsTrackedDirtyProperty(string? propertyName)
    {
        return propertyName is
            nameof(ProjectName) or nameof(DocumentsDir) or nameof(WorkflowsDir)
            or nameof(SkillsDir) or nameof(ExportsDir) or nameof(ProjectMemory) or nameof(ProviderId) or nameof(ProviderType)
            or nameof(ProviderDisplayName) or nameof(ProviderBaseUrl) or nameof(ProviderEnabled)
            or nameof(MakeDefaultLlm) or nameof(MakeDefaultEmbedding) or nameof(MakeDefaultReranker)
            or nameof(MakeDefaultSearch)
            or nameof(ModelsText) or nameof(EmbeddingModelId) or nameof(ApiKey)
            or nameof(DefaultProviderId) or nameof(DefaultModelId)
            or nameof(DefaultTimeoutMs) or nameof(DefaultBudgetUsd) or nameof(TemplateRepositoryBaseUrl)
            or nameof(BudgetUsd) or nameof(PreauthorizedUsd)
            or nameof(WorkflowDefaultTimeoutMs) or nameof(MaxLoopIterations) or nameof(MaxToolRounds)
            or nameof(CheckpointEnabled) or nameof(RuntimeAutosaveMs) or nameof(AllowNetwork)
            or nameof(AllowWebSearch) or nameof(AllowHttpSkill) or nameof(AllowWasmNetwork)
            or nameof(AllowSecretRead) or nameof(ReadableRootsText) or nameof(WritableRootsText)
            or nameof(Theme) or nameof(ThemeMainColor) or nameof(ThemeSurfaceColor) or nameof(ThemeBrandColor)
            or nameof(ThemeMainColorDark) or nameof(ThemeSurfaceColorDark) or nameof(ThemeBrandColorDark)
            or nameof(ThemeFollowSystemColors)
            or nameof(GitAutoColor) or nameof(GitManualColor)
            or nameof(ProjectPanelVisible) or nameof(ReduceMotion) or nameof(SelectedLanguage)
            or nameof(VectorEnabled)
            or nameof(VectorBackend) or nameof(VectorCollection) or nameof(VectorDimensions)
            or nameof(QdrantHost) or nameof(QdrantPort) or nameof(QdrantDataDir)
            or nameof(QdrantBinaryPath) or nameof(QdrantStartupTimeoutMs) or nameof(RerankerEnabled)
            or nameof(ChunkSizeChars) or nameof(ChunkOverlapChars) or nameof(TrackDocuments)
            or nameof(TrackWorkflows) or nameof(TrackSkills) or nameof(TrackNonSensitiveConfig)
            or nameof(IgnoredPathsText);
    }
}

public sealed class LanguageOption : ViewModelBase
{
    private string _label;

    public LanguageOption(string code, string label)
    {
        Code = code;
        _label = label;
    }

    public string Code { get; }
    public string Label { get => _label; set => SetProperty(ref _label, value); }
}

public sealed class SettingsValueOption : ViewModelBase
{
    private string _label;

    public SettingsValueOption(string value, string label)
    {
        Value = value;
        _label = label;
    }

    public string Value { get; }
    public string Label { get => _label; set => SetProperty(ref _label, value); }
}

public sealed class ThemeOption : ViewModelBase
{
    private string _label;
    private string _description;
    private string _groupTitle;
    private bool _isSelected;
    private readonly Action<ThemeOption> _select;

    public ThemeOption(
        string code,
        string group,
        string groupTitle,
        string label,
        string description,
        IBrush swatchMain,
        IBrush swatchSurface,
        IBrush swatchBrand,
        Action<ThemeOption> select)
    {
        Code = code;
        Group = group;
        _groupTitle = groupTitle;
        _label = label;
        _description = description;
        SwatchMain = swatchMain;
        SwatchSurface = swatchSurface;
        SwatchBrand = swatchBrand;
        _select = select;
        SelectCommand = new RelayCommand(() => _select(this));
    }

    public string Code { get; }
    public string Group { get; }
    public string GroupTitle { get => _groupTitle; set => SetProperty(ref _groupTitle, value); }
    public string Label { get => _label; set => SetProperty(ref _label, value); }
    public string Description { get => _description; set => SetProperty(ref _description, value); }
    public IBrush SwatchMain { get; }
    public IBrush SwatchSurface { get; }
    public IBrush SwatchBrand { get; }
    public RelayCommand SelectCommand { get; }
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }
}

public sealed class ToolControlGroupViewModel : ViewModelBase
{
    private string _displayName;

    public ToolControlGroupViewModel(string scope, string displayName)
    {
        Scope = scope;
        _displayName = displayName;
        Controls = new ObservableCollection<ToolControlItemViewModel>();
        SafeControls = new ObservableCollection<ToolControlItemViewModel>();
        DangerControls = new ObservableCollection<ToolControlItemViewModel>();
    }

    public string Scope { get; }
    public string DisplayName { get => _displayName; set => SetProperty(ref _displayName, value); }
    public ObservableCollection<ToolControlItemViewModel> Controls { get; }
    public ObservableCollection<ToolControlItemViewModel> SafeControls { get; }
    public ObservableCollection<ToolControlItemViewModel> DangerControls { get; }
    public bool HasSafeControls => SafeControls.Count > 0;
    public bool HasDangerControls => DangerControls.Count > 0;

    public void RefreshPartitions()
    {
        SafeControls.Clear();
        DangerControls.Clear();
        foreach (var item in Controls)
        {
            if (item.IsDangerous)
            {
                DangerControls.Add(item);
            }
            else
            {
                SafeControls.Add(item);
            }
        }
        OnPropertyChanged(nameof(HasSafeControls));
        OnPropertyChanged(nameof(HasDangerControls));
    }
}

public sealed class ToolControlItemViewModel : ViewModelBase
{
    private readonly Action _markDirty;
    private string _displayName;
    private bool? _isEnabled;

    public ToolControlItemViewModel(
        string toolId,
        string displayName,
        bool? isEnabled,
        bool isDangerous,
        bool canInherit,
        Action markDirty)
    {
        ToolId = toolId;
        _displayName = displayName;
        _isEnabled = isEnabled;
        IsDangerous = isDangerous;
        CanInherit = canInherit;
        _markDirty = markDirty;
    }

    public string ToolId { get; }
    public string DisplayName { get => _displayName; set => SetProperty(ref _displayName, value); }
    public bool IsDangerous { get; }
    public bool CanInherit { get; }

    /// <summary>写盘/重写类工具视为危险，与权限页 warning 分组共用。</summary>
    public static bool IsDangerToolId(string toolId)
    {
        var id = (toolId ?? string.Empty).Trim().ToLowerInvariant();
        if (string.IsNullOrEmpty(id))
        {
            return false;
        }
        return id.Contains("rewrite-file", StringComparison.Ordinal)
               || id == "write"
               || id.Contains("replace-lines", StringComparison.Ordinal)
               || id.Contains("insert-lines", StringComparison.Ordinal)
               || id.Contains("secret", StringComparison.Ordinal)
               || id.EndsWith("-delete", StringComparison.Ordinal)
               || id.Contains("delete-file", StringComparison.Ordinal);
    }

    public bool? IsEnabled
    {
        get => _isEnabled;
        set
        {
            if (SetProperty(ref _isEnabled, value))
            {
                _markDirty();
            }
        }
    }
}

public sealed class PermissionScopeProfileViewModel : ViewModelBase
{
    private readonly Action _markDirty;
    private PermissionPolicy _inheritedPolicy;
    private string _displayName;
    private bool _inheritGlobal;
    private bool _allowNetwork;
    private bool _allowWebSearch;
    private bool _allowHttpSkill;
    private bool _allowWasmNetwork;
    private bool _allowSecretRead;
    private string _readableRootsText;
    private string _writableRootsText;

    public PermissionScopeProfileViewModel(
        string scope,
        string displayName,
        PermissionPolicy? policy,
        PermissionPolicy fallback,
        Action markDirty)
    {
        Scope = scope;
        _displayName = displayName;
        _inheritGlobal = policy is null;
        _inheritedPolicy = fallback;
        var resolved = policy ?? fallback;
        _allowNetwork = resolved.AllowNetwork;
        _allowWebSearch = resolved.AllowWebSearch;
        _allowHttpSkill = resolved.AllowHttpSkill;
        _allowWasmNetwork = resolved.AllowWasmNetwork;
        _allowSecretRead = resolved.AllowSecretRead;
        _readableRootsText = string.Join(Environment.NewLine, resolved.ReadableFileRoots);
        _writableRootsText = string.Join(Environment.NewLine, resolved.WritableFileRoots);
        _markDirty = markDirty;
    }

    public string Scope { get; }
    public string DisplayName { get => _displayName; set => SetProperty(ref _displayName, value); }
    public bool IsOverrideEnabled => !InheritGlobal;
    public bool InheritGlobal
    {
        get => _inheritGlobal;
        set
        {
            if (SetProperty(ref _inheritGlobal, value))
            {
                if (value)
                {
                    // Re-join parent projection.
                    ApplyPolicy(_inheritedPolicy);
                }
                else
                {
                    // Leaving inherit: freeze the currently displayed parent projection as an
                    // explicit override so later parent rebinds cannot rewrite these fields.
                    ApplyPolicy(_inheritedPolicy);
                }
                OnPropertyChanged(nameof(IsOverrideEnabled));
                _markDirty();
            }
        }
    }
    public bool AllowNetwork
    {
        get => _allowNetwork;
        set
        {
            if (SetAndMark(ref _allowNetwork, value) && !value)
            {
                AllowWebSearch = false;
                AllowHttpSkill = false;
                AllowWasmNetwork = false;
            }
        }
    }
    public bool AllowWebSearch { get => _allowWebSearch; set => SetAndMark(ref _allowWebSearch, value); }
    public bool AllowHttpSkill { get => _allowHttpSkill; set => SetAndMark(ref _allowHttpSkill, value); }
    public bool AllowWasmNetwork { get => _allowWasmNetwork; set => SetAndMark(ref _allowWasmNetwork, value); }
    public bool AllowSecretRead { get => _allowSecretRead; set => SetAndMark(ref _allowSecretRead, value); }
    public string ReadableRootsText { get => _readableRootsText; set => SetAndMark(ref _readableRootsText, value); }
    public string WritableRootsText { get => _writableRootsText; set => SetAndMark(ref _writableRootsText, value); }

    public PermissionPolicy ToPolicy() => new(
        AllowNetwork,
        AllowWebSearch,
        AllowHttpSkill,
        AllowWasmNetwork,
        AllowSecretRead,
        SettingsInputValidation.AbsolutePaths(
            WritableRootsText,
            "ui.settings.permissions.writable_roots"),
        SettingsInputValidation.AbsolutePaths(
            ReadableRootsText,
            "ui.settings.permissions.readable_roots"));

    public string Snapshot => InheritGlobal
        ? $"{Scope}:inherit"
        : string.Join(":", new[]
        {
            Scope,
            "override",
            AllowNetwork.ToString(),
            AllowWebSearch.ToString(),
            AllowHttpSkill.ToString(),
            AllowWasmNetwork.ToString(),
            AllowSecretRead.ToString(),
            ReadableRootsText,
            WritableRootsText,
        });

    /// <summary>刷新继承父级；显式覆盖保持原值，父级投影变化不改写覆盖字段、不触发 dirty。</summary>
    public void RebindInheritedPolicy(PermissionPolicy inheritedPolicy)
    {
        _inheritedPolicy = inheritedPolicy;
        // Use the backing field so a parent rebind cannot race with the InheritGlobal setter.
        if (_inheritGlobal)
        {
            ApplyPolicy(inheritedPolicy);
        }
    }

    private void ApplyPolicy(PermissionPolicy policy)
    {
        SetProperty(ref _allowNetwork, policy.AllowNetwork, nameof(AllowNetwork));
        SetProperty(ref _allowWebSearch, policy.AllowWebSearch, nameof(AllowWebSearch));
        SetProperty(ref _allowHttpSkill, policy.AllowHttpSkill, nameof(AllowHttpSkill));
        SetProperty(ref _allowWasmNetwork, policy.AllowWasmNetwork, nameof(AllowWasmNetwork));
        SetProperty(ref _allowSecretRead, policy.AllowSecretRead, nameof(AllowSecretRead));
        SetProperty(
            ref _readableRootsText,
            string.Join(Environment.NewLine, policy.ReadableFileRoots),
            nameof(ReadableRootsText));
        SetProperty(
            ref _writableRootsText,
            string.Join(Environment.NewLine, policy.WritableFileRoots),
            nameof(WritableRootsText));
    }

    private bool SetAndMark<T>(ref T field, T value)
    {
        if (SetProperty(ref field, value))
        {
            _markDirty();
            return true;
        }
        return false;
    }

    private static IReadOnlyList<string> Lines(string text) => text
        .Split(new[] { "\r\n", "\n" }, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
        .Where(line => !string.IsNullOrWhiteSpace(line))
        .ToArray();
}

public sealed class ConfirmationPolicyViewModel : ViewModelBase
{
    private string _label;
    private bool _normalAllowByDefault;
    private bool _autoModeAutoApproval;
    private string _approvalPrompt;

    private readonly Action _markDirty;

    public ConfirmationPolicyViewModel(string kind, string label, string normalPolicy, string autoModePolicy, string approvalPrompt, Action markDirty)
    {
        Kind = kind;
        _label = label;
        _markDirty = markDirty;
        _normalAllowByDefault = normalPolicy == "allow_by_default";
        _autoModeAutoApproval = autoModePolicy == "auto_approval";
        _approvalPrompt = approvalPrompt ?? string.Empty;
    }

    public string Kind { get; }
    public string Label { get => _label; set => SetProperty(ref _label, value); }
    public string NormalPolicy => NormalAllowByDefault ? "allow_by_default" : "manual_review";
    public string AutoModePolicy => AutoModeAutoApproval ? "auto_approval" : "allow_by_default";
    public string ApprovalPrompt
    {
        get => _approvalPrompt;
        set
        {
            if (SetProperty(ref _approvalPrompt, value ?? string.Empty))
            {
                _markDirty();
            }
        }
    }

    public bool NormalAllowByDefault
    {
        get => _normalAllowByDefault;
        set
        {
            if (SetProperty(ref _normalAllowByDefault, value))
            {
                OnPropertyChanged(nameof(NormalPolicy));
                _markDirty();
            }
        }
    }

    public bool AutoModeAutoApproval
    {
        get => _autoModeAutoApproval;
        set
        {
            if (SetProperty(ref _autoModeAutoApproval, value))
            {
                OnPropertyChanged(nameof(AutoModePolicy));
                _markDirty();
            }
        }
    }
}

public sealed class SettingsTabViewModel : ViewModelBase
{
    private bool _isSelected;
    private string _title;

    public SettingsTabViewModel(string id, string title, Action<SettingsTabViewModel> select)
    {
        Id = id;
        _title = title;
        SelectCommand = new RelayCommand(() => select(this));
    }

    public string Id { get; }
    public string Title { get => _title; set => SetProperty(ref _title, value); }
    public RelayCommand SelectCommand { get; }
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }
}

public sealed class SettingsSectionNavigationItemViewModel : ViewModelBase
{
    private string _title;

    public SettingsSectionNavigationItemViewModel(
        string id,
        string tabId,
        string anchorName,
        string title)
    {
        Id = id;
        TabId = tabId;
        AnchorName = anchorName;
        _title = title;
    }

    public string Id { get; }
    public string TabId { get; }
    public string AnchorName { get; }
    public string Title { get => _title; set => SetProperty(ref _title, value); }
}

public sealed class SettingsSectionNavigationRequest : EventArgs
{
    public SettingsSectionNavigationRequest(string anchorName, string sectionTitle)
    {
        AnchorName = anchorName;
        SectionTitle = sectionTitle;
    }

    public string AnchorName { get; }
    public string SectionTitle { get; }
}

/// <summary>确认项策略分组。</summary>
public sealed class ConfirmationPolicyGroupViewModel : ViewModelBase
{
    public ConfirmationPolicyGroupViewModel(
        string groupId,
        string title,
        IEnumerable<ConfirmationPolicyViewModel> items)
    {
        GroupId = groupId;
        Title = title;
        Items = new ObservableCollection<ConfirmationPolicyViewModel>(items);
    }

    public string GroupId { get; }
    public string Title { get; }
    public ObservableCollection<ConfirmationPolicyViewModel> Items { get; }
}
