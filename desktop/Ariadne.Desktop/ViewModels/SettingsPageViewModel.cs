using System.Collections.ObjectModel;
using Avalonia.Media;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;


namespace Ariadne.Desktop.ViewModels;

public sealed class SettingsPageViewModel : ViewModelBase, IUnsavedChangesGuard
{
    private const char SnapshotSeparator = '\u001f';
    private const int SnapshotLocaleIndex = 1;
    private const int SnapshotModelStartIndex = 7;
    private const int SnapshotModelEndIndex = 18;
    // Theme + 三色自定义 + Git 双色 + panel + onboarding：onboarding 在 snapshot 中的下标
    private const int SnapshotOnboardingSeenIndex = 48;
    private static readonly string[] LocalizedPropertyNames =
    {
        nameof(Title),
        nameof(GeneralTitle),
        nameof(ModelsTitle),
        nameof(PresetsTitle),
        nameof(AutomationTitle),
        nameof(PermissionsTitle),
        nameof(PersonalizationTitle),
        nameof(MiscTitle),
        nameof(ProjectNameLabel),
        nameof(LocaleLabel),
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
        nameof(RefreshText),
        nameof(ProviderStatusLabel),
        nameof(PresetNodeTypeLabel),
        nameof(PresetNodeModelLabel),
        nameof(PresetNodeTimeoutLabel),
        nameof(PresetNodeBudgetLabel),
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
        nameof(AutoModeLabel),
        nameof(SpentLabel),
        nameof(NormalModeLabel),
        nameof(AutoModePolicyLabel),
        nameof(ConfirmationPolicyHelpText),
        nameof(PolicyAllowText),
        nameof(PolicyReviewText),
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
        nameof(ColorMapHintText),
        nameof(ShowAllSectionsText),
        nameof(SectionIndexHintText),
        nameof(GitAutoColorLabel),
        nameof(GitManualColorLabel),
        nameof(ProjectPanelVisibleText),
        nameof(OnboardingSeenText),
        nameof(SavePersonalizationText),
        nameof(RagLabel),
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
        nameof(LanguageDescText),
        nameof(TutorialText),
        nameof(DiagnosticsLabel),
        nameof(DiagnosticsStatusText),
    };

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private readonly Func<Task>? _openTemplateMarket;
    private SettingsTabViewModel _selectedTab;
    private SettingsSectionIndexItemViewModel? _selectedSection;
    private string _selectedSectionId = string.Empty;
    private string _selectedLanguage;
    private string _statusText;
    private bool _isLoading;
    private bool _hasUnsavedChanges;
    private bool _suppressDirtyTracking;
    private bool _suppressProviderSelectionChange;

    private string _savedSnapshot = string.Empty;

    private int _schemaVersion = 1;
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
    private string _apiKey = string.Empty;
    private string _modelsText = "gpt-4.1-mini,llm,,,,";
    private string _embeddingModelId = string.Empty;
    private bool _manualModelsVisible;
    private string _providerStatus = string.Empty;
    private ProviderConfigStatus? _providerConfig;
    private ProviderOptionViewModel? _selectedProviderOption;

    private string _defaultModelId = "gpt-4.1-mini";
    private string _defaultTimeoutMs = "300000";
    private string _defaultBudgetUsd = "0";
    private string _templateRepositoryBaseUrl = string.Empty;

    private string _budgetUsd = "0";
    private string _preauthorizedUsd = "0";
    private bool _autoModeEnabled;
    private string _spentText = "$0.00";
    private string _workflowDefaultTimeoutMs = "300000";
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
    private string _gitAutoColor = "#8a8f98";
    private string _gitManualColor = "#f59e0b";
    private bool _projectPanelVisible = true;
    private bool _onboardingSeen;
    private UiPreferences? _uiPreferences;

    private string _vectorBackend = "qdrant_sidecar";
    private string _qdrantHost = "127.0.0.1";
    private string _qdrantPort = "6333";
    private string _qdrantDataDir = ".indexes/qdrant";
    private string _fullTextBackend = "tantivy";
    private string _fullTextIndexDir = ".indexes/tantivy";
    private bool _rerankerEnabled = true;
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

    public SettingsPageViewModel(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        Func<Task>? openTemplateMarket = null)
    {
        _displayNames = displayNames;
        _backend = backend;
        _openTemplateMarket = openTemplateMarket;
        _selectedLanguage = _displayNames.NormalizeAvailableLanguage(displayNames.CurrentLanguage);
        _statusText = displayNames.Text("ui.common.loading");

        LanguageOptions = new ObservableCollection<LanguageOption>(
            displayNames.AvailableLanguages.Select(code => new LanguageOption(code, displayNames.LanguageLabel(code))));

        ProviderTypeOptions = new ObservableCollection<string>
        {
            "open_ai", "anthropic", "gemini", "open_ai_compatible", "local", "other",
        };

        ThemeOptions = new ObservableCollection<ThemeOption>(
            ThemeCatalog.All.Select(palette => CreateThemeOption(palette, displayNames)));
        ThemeGroups = new ObservableCollection<ThemeGroupViewModel>(
            ThemeOptions.GroupBy(o => o.GroupTitle)
                .Select(g => new ThemeGroupViewModel(g.Key, g)));
        ConfirmationPolicies = new ObservableCollection<ConfirmationPolicyViewModel>();
        NodePresets = new ObservableCollection<NodeTypePresetViewModel>();
        ProviderOptions = new ObservableCollection<ProviderOptionViewModel>();
        AvailableModels = new ObservableCollection<ModelOptionViewModel>();
        AvailableModelIds = new ObservableCollection<string>();
        ToolControlGroups = new ObservableCollection<ToolControlGroupViewModel>();
        SectionIndexItems = new ObservableCollection<SettingsSectionIndexItemViewModel>();
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
                HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot;
            }
        });
        gitManualEditor = new ColorChannelEditor(() =>
        {
            OnPropertyChanged(nameof(GitManualColor));
            SyncGitColorSwatchSelection(GitManualColorSwatches, gitManualEditor.ToHexValue());
            if (!_suppressDirtyTracking)
            {
                HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot;
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

        Tabs = new ObservableCollection<SettingsTabViewModel>
        {
            CreateTab("general", "ui.settings.tab.general"),
            CreateTab("models", "ui.settings.tab.models"),
            CreateTab("presets", "ui.settings.tab.presets"),
            CreateTab("automation", "ui.settings.tab.automation"),
            CreateTab("permissions", "ui.settings.tab.permissions"),
            CreateTab("personalization", "ui.settings.tab.personalization"),
            CreateTab("misc", "ui.settings.tab.misc"),
        };
        _selectedTab = Tabs[0];
        _selectedTab.IsSelected = true;
        RebuildSectionIndex();

        SaveGeneralCommand = new RelayCommand(() => _ = SaveGeneralAsync());
        RefreshModelsCommand = new RelayCommand(() => _ = FetchModelsAsync());
        SaveModelCommand = new RelayCommand(() => _ = SaveModelAsync());
        SaveProviderKeyCommand = new RelayCommand(() => _ = SaveProviderKeyAsync());
        AddProviderCommand = new RelayCommand(AddProviderDraft);
        SavePresetsCommand = new RelayCommand(() => _ = SavePresetsAsync());
        SaveTemplateRepositoryCommand = new RelayCommand(() => _ = SaveTemplateRepositoryAsync());
        OpenTemplateMarketCommand = new RelayCommand(() => _ = OpenTemplateMarketAsync());
        SaveAutomationCommand = new RelayCommand(() => _ = SaveAutomationAsync());
        SavePermissionsCommand = new RelayCommand(() => _ = SavePermissionsAsync());
        SavePersonalizationCommand = new RelayCommand(() => _ = SavePersonalizationAsync());
        SaveMiscCommand = new RelayCommand(() => _ = SaveMiscAsync());
        ResetOnboardingCommand = new RelayCommand(() => _ = ResetOnboardingAsync());
        BrowseDocumentsDirCommand = new RelayCommand(() => _ = BrowseIntoAsync(value => DocumentsDir = value));
        BrowseWorkflowsDirCommand = new RelayCommand(() => _ = BrowseIntoAsync(value => WorkflowsDir = value));
        BrowseSkillsDirCommand = new RelayCommand(() => _ = BrowseIntoAsync(value => SkillsDir = value));
        BrowseExportsDirCommand = new RelayCommand(() => _ = BrowseIntoAsync(value => ExportsDir = value));
        BrowseReadableRootsCommand = new RelayCommand(() => _ = BrowseIntoAsync(AppendReadableRoot));
        BrowseWritableRootsCommand = new RelayCommand(() => _ = BrowseIntoAsync(AppendWritableRoot));

        _ = LoadAsync();
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
        if (lines.Any(l => string.Equals(l, line, StringComparison.OrdinalIgnoreCase)))
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
            StatusText = ex.Message;
        }
    }

    public string Title => _displayNames.Text("ui.settings.title");
    public string StatusText { get => _statusText; set => SetProperty(ref _statusText, value); }
    public bool IsLoading { get => _isLoading; set => SetProperty(ref _isLoading, value); }
    public bool HasUnsavedChanges
    {
        get => _hasUnsavedChanges;
        private set => SetProperty(ref _hasUnsavedChanges, value);
    }

    public ObservableCollection<SettingsTabViewModel> Tabs { get; }
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
                OnSelectedSectionChanged();
            }
        }
    }

    public bool IsGeneralSelected => SelectedTab.Id == "general";
    public bool IsModelsSelected => SelectedTab.Id == "models";
    public bool IsPresetsSelected => SelectedTab.Id == "presets";
    public bool IsAutomationSelected => SelectedTab.Id == "automation";
    public bool IsPermissionsSelected => SelectedTab.Id == "permissions";
    public bool IsPersonalizationSelected => SelectedTab.Id == "personalization";
    public bool IsMiscSelected => SelectedTab.Id == "misc";
    public bool IsSectionProjectSelected => IsSectionSelected("project");
    public bool IsSectionDirectoriesSelected => IsSectionSelected("directories");
    public bool IsSectionProjectMemorySelected => IsSectionSelected("project_memory");
    public bool IsSectionProviderSelected => IsSectionSelected("provider");
    public bool IsSectionAvailableModelsSelected => IsSectionSelected("available");
    public bool IsSectionEmbeddingSelected => IsSectionSelected("embedding");
    public bool IsSectionManualModelsSelected => IsSectionSelected("manual");
    public bool IsSectionSecretSelected => IsSectionSelected("secret");
    public bool IsSectionNodePresetsSelected => IsSectionSelected("node_presets");
    public bool IsSectionDefaultsSelected => IsSectionSelected("defaults");
    public bool IsSectionTemplatesSelected => IsSectionSelected("templates");
    public bool IsSectionBudgetSelected => IsSectionSelected("budget");
    public bool IsSectionConfirmationsSelected => IsSectionSelected("confirmations");
    public bool IsSectionRuntimeSelected => IsSectionSelected("runtime");
    public bool IsSectionCapabilitiesSelected => IsSectionSelected("capabilities");
    public bool IsSectionToolControlsSelected => IsSectionSelected("tool_controls");
    public bool IsSectionPathsSelected => IsSectionSelected("paths");
    public bool IsSectionThemeSelected => IsSectionSelected("theme");
    public bool IsSectionWorkspaceSelected => IsSectionSelected("workspace");
    public bool IsSectionRetrievalSelected => IsSectionSelected("retrieval");
    public bool IsSectionGitSelected => IsSectionSelected("git");
    public bool IsSectionLanguageSelected => IsSectionSelected("language");
    public bool IsSectionDiagnosticsSelected => IsSectionSelected("diagnostics");
    public ObservableCollection<LanguageOption> LanguageOptions { get; }
    public ObservableCollection<string> ProviderTypeOptions { get; }
    public ObservableCollection<ThemeOption> ThemeOptions { get; }
    public ObservableCollection<ThemeGroupViewModel> ThemeGroups { get; }
    public ObservableCollection<ConfirmationPolicyViewModel> ConfirmationPolicies { get; }
    public ObservableCollection<NodeTypePresetViewModel> NodePresets { get; }
    public ObservableCollection<ProviderOptionViewModel> ProviderOptions { get; }
    public ObservableCollection<ModelOptionViewModel> AvailableModels { get; }
    /// <summary>模型 ID 列表，供预设/默认模型 ComboBox 选择；可手填时列表为空。</summary>
    public ObservableCollection<string> AvailableModelIds { get; }
    public bool HasAvailableModelChoices => AvailableModelIds.Count > 0;
    public ObservableCollection<ToolControlGroupViewModel> ToolControlGroups { get; }
    public ObservableCollection<SettingsSectionIndexItemViewModel> SectionIndexItems { get; }
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

    /// <summary>当前激活的主题色槽（供 PS 式调色板双向绑定）。</summary>
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

    public RelayCommand SelectThemeMainChannelCommand => new(() => SetActiveThemeColorChannel(ThemeColorChannel.Main));
    public RelayCommand SelectThemeSurfaceChannelCommand => new(() => SetActiveThemeColorChannel(ThemeColorChannel.Surface));
    public RelayCommand SelectThemeBrandChannelCommand => new(() => SetActiveThemeColorChannel(ThemeColorChannel.Brand));

    public RelayCommand SaveGeneralCommand { get; }
    public RelayCommand RefreshModelsCommand { get; }
    public RelayCommand SaveModelCommand { get; }
    public RelayCommand SaveProviderKeyCommand { get; }
    public RelayCommand AddProviderCommand { get; }
    public RelayCommand SavePresetsCommand { get; }
    public RelayCommand SaveTemplateRepositoryCommand { get; }
    public RelayCommand OpenTemplateMarketCommand { get; }
    public RelayCommand SaveAutomationCommand { get; }
    public RelayCommand SavePermissionsCommand { get; }
    public RelayCommand SavePersonalizationCommand { get; }
    public RelayCommand SaveMiscCommand { get; }
    public RelayCommand ResetOnboardingCommand { get; }
    public RelayCommand BrowseDocumentsDirCommand { get; }
    public RelayCommand BrowseWorkflowsDirCommand { get; }
    public RelayCommand BrowseSkillsDirCommand { get; }
    public RelayCommand BrowseExportsDirCommand { get; }
    public RelayCommand BrowseReadableRootsCommand { get; }
    public RelayCommand BrowseWritableRootsCommand { get; }

    public string GeneralTitle => _displayNames.Text("ui.settings.general.title");
    public string ModelsTitle => _displayNames.Text("ui.settings.models.title");
    public string PresetsTitle => _displayNames.Text("ui.settings.presets.title");
    public string AutomationTitle => _displayNames.Text("ui.settings.automation.title");
    public string PermissionsTitle => _displayNames.Text("ui.settings.permissions.title");
    public string PersonalizationTitle => _displayNames.Text("ui.settings.personalization.title");
    public string MiscTitle => _displayNames.Text("ui.settings.misc.title");

    public string ProjectNameLabel => _displayNames.Text("ui.settings.general.project_name");
    public string LocaleLabel => _displayNames.Text("ui.settings.general.locale");
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
    public string RefreshText => _displayNames.Text("ui.common.refresh");
    public string ProviderStatusLabel => _displayNames.Text("ui.settings.models.status");
    public string AddProviderText => _displayNames.Text("ui.settings.models.add_provider");
    public string ProviderListTitle => _displayNames.Text("ui.settings.models.provider_list");
    public string ProviderEditorTitle => _displayNames.Text("ui.settings.models.provider_editor");
    public string ColorRgbHintText => _displayNames.Text("ui.settings.personalization.color_rgb_hint");
    public string ColorMapHintText => _displayNames.Text("ui.settings.personalization.color_map_hint");
    public string ColorHexSecondaryText => _displayNames.Text("ui.settings.personalization.color_hex_secondary");

    public string PresetNodeTypeLabel => _displayNames.Text("ui.settings.presets.node_type");
    public string PresetNodeModelLabel => _displayNames.Text("ui.settings.presets.node_model");
    public string PresetNodeTimeoutLabel => _displayNames.Text("ui.settings.presets.node_timeout_ms");
    public string PresetNodeBudgetLabel => _displayNames.Text("ui.settings.presets.node_budget_usd");
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
    public string AutoModeLabel => _displayNames.Text("ui.settings.automation.auto_mode");
    public string SpentLabel => _displayNames.Text("ui.settings.automation.spent").Replace("{spent}", SpentText);
    public string NormalModeLabel => _displayNames.Text("ui.settings.automation.confirmation.normal_mode");
    public string AutoModePolicyLabel => _displayNames.Text("ui.settings.automation.confirmation.auto_mode_policy");
    public string ConfirmationPolicyHelpText => _displayNames.Text("ui.settings.automation.confirmation.help");
    public string PolicyAllowText => _displayNames.Text("ui.settings.automation.confirmation.allow");
    public string PolicyReviewText => _displayNames.Text("ui.settings.automation.confirmation.review");
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
    public string ThemeSurfaceColorLabel => _displayNames.Text("ui.settings.personalization.theme.color_surface");
    public string ThemeBrandColorLabel => _displayNames.Text("ui.settings.personalization.theme.color_brand");
    public string GitAutoColorLabel => _displayNames.Text("ui.settings.personalization.git_auto_color");
    public string GitManualColorLabel => _displayNames.Text("ui.settings.personalization.git_manual_color");
    public string ProjectPanelVisibleText => _displayNames.Text("ui.settings.personalization.project_panel");
    public string OnboardingSeenText => _displayNames.Text("ui.settings.personalization.onboarding_seen");
    public string SavePersonalizationText => _displayNames.Text("ui.settings.personalization.save");

    public string RagLabel => _displayNames.Text("ui.settings.misc.rag");
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
    public string LanguageDescText => _displayNames.Text("ui.settings.misc.language.desc");
    public string TutorialText => _displayNames.Text("ui.settings.index.tutorial");
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
            // 抑制路径（SetSelected/Restore）直接写字段；用户改选走 SelectProviderOptionAsync，
            // 仅在离开成功后才提交列表选中，避免取消时列表与表单脱节。
            if (_suppressProviderSelectionChange)
            {
                if (SetProperty(ref _selectedProviderOption, value))
                {
                    OnPropertyChanged(nameof(IsSelectedProviderDraft));
                }
                return;
            }

            if (value is null)
            {
                if (SetProperty(ref _selectedProviderOption, null))
                {
                    OnPropertyChanged(nameof(IsSelectedProviderDraft));
                }
                return;
            }

            if (ReferenceEquals(_selectedProviderOption, value)
                || string.Equals(_selectedProviderOption?.ProviderId, value.ProviderId, StringComparison.Ordinal))
            {
                return;
            }

            _ = SelectProviderOptionAsync(value);
        }
    }

    /// <summary>当前选中供应商是否为未落库草稿（仅草稿可改 ProviderId）。</summary>
    public bool IsSelectedProviderDraft => SelectedProviderOption?.IsDraft == true;

    public string DefaultModelId { get => _defaultModelId; set => SetProperty(ref _defaultModelId, value); }
    public string DefaultTimeoutMs { get => _defaultTimeoutMs; set => SetProperty(ref _defaultTimeoutMs, value); }
    public string DefaultBudgetUsd { get => _defaultBudgetUsd; set => SetProperty(ref _defaultBudgetUsd, value); }
    public string TemplateRepositoryBaseUrl { get => _templateRepositoryBaseUrl; set => SetProperty(ref _templateRepositoryBaseUrl, value); }

    public string BudgetUsd { get => _budgetUsd; set => SetProperty(ref _budgetUsd, value); }
    public string PreauthorizedUsd { get => _preauthorizedUsd; set => SetProperty(ref _preauthorizedUsd, value); }
    public bool AutoModeEnabled { get => _autoModeEnabled; set => SetProperty(ref _autoModeEnabled, value); }
    public string SpentText { get => _spentText; set { if (SetProperty(ref _spentText, value)) OnPropertyChanged(nameof(SpentLabel)); } }
    public string WorkflowDefaultTimeoutMs { get => _workflowDefaultTimeoutMs; set => SetProperty(ref _workflowDefaultTimeoutMs, value); }
    public string MaxLoopIterations { get => _maxLoopIterations; set => SetProperty(ref _maxLoopIterations, value); }
    public string MaxToolRounds { get => _maxToolRounds; set => SetProperty(ref _maxToolRounds, value); }
    public bool CheckpointEnabled { get => _checkpointEnabled; set => SetProperty(ref _checkpointEnabled, value); }
    public string RuntimeAutosaveMs { get => _runtimeAutosaveMs; set => SetProperty(ref _runtimeAutosaveMs, value); }

    public bool AllowNetwork { get => _allowNetwork; set => SetProperty(ref _allowNetwork, value); }
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

    public string ThemeMainColor
    {
        get => ThemeMainColorEditor.ToHexValue();
        set
        {
            ThemeMainColorEditor.SetFromHex(value);
            OnPropertyChanged();
            SyncActiveThemeSwatchSelection();
        }
    }

    public string ThemeSurfaceColor
    {
        get => ThemeSurfaceColorEditor.ToHexValue();
        set
        {
            ThemeSurfaceColorEditor.SetFromHex(value);
            OnPropertyChanged();
            SyncActiveThemeSwatchSelection();
        }
    }

    public string ThemeBrandColor
    {
        get => ThemeBrandColorEditor.ToHexValue();
        set
        {
            ThemeBrandColorEditor.SetFromHex(value);
            OnPropertyChanged();
            SyncActiveThemeSwatchSelection();
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
        OnPropertyChanged(nameof(ActiveThemeColorHex));
        SyncActiveThemeSwatchSelection();
    }

    private void OnThemeColorSwatchPicked(string hex)
    {
        switch (_activeThemeColorChannel)
        {
            case ThemeColorChannel.Main:
                ThemeMainColor = hex;
                break;
            case ThemeColorChannel.Surface:
                ThemeSurfaceColor = hex;
                break;
            default:
                ThemeBrandColor = hex;
                break;
        }

        ApplyCurrentThemeColors();
        if (!_suppressDirtyTracking)
        {
            HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot;
        }
    }

    private void OnThemeCustomColorChanged(ThemeColorChannel channel)
    {
        OnPropertyChanged(channel switch
        {
            ThemeColorChannel.Main => nameof(ThemeMainColor),
            ThemeColorChannel.Surface => nameof(ThemeSurfaceColor),
            _ => nameof(ThemeBrandColor),
        });
        if (channel == _activeThemeColorChannel)
        {
            SyncActiveThemeSwatchSelection();
            OnPropertyChanged(nameof(ActiveThemeColorHex));
        }

        ApplyCurrentThemeColors();
        if (!_suppressDirtyTracking)
        {
            HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot;
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
            ThemeMainColorEditor.SetFromHex(ThemeApplication.ToHex(palette.SwatchMain));
            ThemeSurfaceColorEditor.SetFromHex(ThemeApplication.ToHex(palette.SwatchSurface));
            ThemeBrandColorEditor.SetFromHex(ThemeApplication.ToHex(palette.SwatchBrand));
            OnPropertyChanged(nameof(ThemeMainColor));
            OnPropertyChanged(nameof(ThemeSurfaceColor));
            OnPropertyChanged(nameof(ThemeBrandColor));
            SyncActiveThemeSwatchSelection();
        }
        finally
        {
            _suppressDirtyTracking = suppress;
        }
    }

    private void ApplyCurrentThemeColors()
    {
        ThemeApplication.Apply(
            Theme,
            ThemeMainColorEditor.ToHexValue(),
            ThemeSurfaceColorEditor.ToHexValue(),
            ThemeBrandColorEditor.ToHexValue());
    }

    private void LoadThemeColorsFromPreferences(UiPreferences prefs)
    {
        var palette = ThemeCatalog.Resolve(prefs.Theme);
        var suppress = _suppressDirtyTracking;
        _suppressDirtyTracking = true;
        try
        {
            ThemeMainColorEditor.SetFromHex(
                ThemeApplication.HasHex(prefs.ThemeMainColor)
                    ? prefs.ThemeMainColor
                    : ThemeApplication.ToHex(palette.SwatchMain));
            ThemeSurfaceColorEditor.SetFromHex(
                ThemeApplication.HasHex(prefs.ThemeSurfaceColor)
                    ? prefs.ThemeSurfaceColor
                    : ThemeApplication.ToHex(palette.SwatchSurface));
            ThemeBrandColorEditor.SetFromHex(
                ThemeApplication.HasHex(prefs.ThemeBrandColor)
                    ? prefs.ThemeBrandColor
                    : ThemeApplication.ToHex(palette.SwatchBrand));
            OnPropertyChanged(nameof(ThemeMainColor));
            OnPropertyChanged(nameof(ThemeSurfaceColor));
            OnPropertyChanged(nameof(ThemeBrandColor));
            SyncActiveThemeSwatchSelection();
            ApplyCurrentThemeColors();
        }
        finally
        {
            _suppressDirtyTracking = suppress;
        }
    }
    public bool ProjectPanelVisible { get => _projectPanelVisible; set => SetProperty(ref _projectPanelVisible, value); }
    public bool OnboardingSeen { get => _onboardingSeen; set => SetProperty(ref _onboardingSeen, value); }

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
                _ = PersistLanguageAsync(language);
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
        if (!string.Equals(locale, language, StringComparison.OrdinalIgnoreCase))
        {
            _ = PersistLanguageAsync(language);
        }
    }

    private void SelectTab(SettingsTabViewModel tab)
    {
        _ = SelectTabAsync(tab);
    }

    private async Task SelectTabAsync(SettingsTabViewModel tab)
    {
        if (tab == SelectedTab)
        {
            return;
        }
        if (!await ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
        {
            return;
        }
        foreach (var item in Tabs)
        {
            item.IsSelected = item == tab;
        }
        SelectedTab = tab;
        RebuildSectionIndex();
    }

    private void RebuildSectionIndex()
    {
        var previousSectionId = string.IsNullOrWhiteSpace(_selectedSectionId)
            ? null
            : _selectedSectionId;
        SectionIndexItems.Clear();
        var items = SelectedTab.Id switch
        {
            "general" => new[]
            {
                ("project", "ui.settings.section.project"),
                ("directories", "ui.settings.section.directories"),
                ("project_memory", "ui.settings.section.project_memory"),
            },
            "models" => new[]
            {
                ("provider", "ui.settings.section.provider"),
                ("available", "ui.settings.section.available_models"),
                ("embedding", "ui.settings.section.embedding"),
                ("manual", "ui.settings.section.manual_fallback"),
            },
            "presets" => new[]
            {
                ("node_presets", "ui.settings.section.node_presets"),
                ("defaults", "ui.settings.section.defaults"),
                ("templates", "ui.settings.section.templates"),
            },
            "automation" => new[]
            {
                ("budget", "ui.settings.section.budget"),
                ("confirmations", "ui.settings.section.confirmations"),
                ("runtime", "ui.settings.section.runtime"),
            },
            "permissions" => new[]
            {
                ("capabilities", "ui.settings.section.capabilities"),
                ("tool_controls", "ui.settings.section.tool_controls"),
                ("paths", "ui.settings.section.paths"),
            },
            "personalization" => new[]
            {
                ("theme", "ui.settings.section.theme"),
                ("workspace", "ui.settings.section.workspace"),
            },
            _ => new[]
            {
                ("retrieval", "ui.settings.section.retrieval"),
                ("git", "ui.settings.section.git"),
                ("language", "ui.settings.section.language"),
                ("diagnostics", "ui.settings.section.diagnostics"),
            },
        };

        foreach (var (id, key) in items)
        {
            SectionIndexItems.Add(new SettingsSectionIndexItemViewModel(id, _displayNames.Text(key), SelectSection));
        }
        _selectedSection = previousSectionId is null
            ? SectionIndexItems.FirstOrDefault()
            : SectionIndexItems.FirstOrDefault(item => item.Id == previousSectionId)
              ?? SectionIndexItems.FirstOrDefault();
        if (_selectedSection is not null)
        {
            _selectedSection.IsSelected = true;
            _selectedSectionId = _selectedSection.Id;
        }
        OnSelectedSectionChanged();
    }

    /// <summary>侧栏点目录时请求内容区滚到对应区块（由 View 处理 BringIntoView）。</summary>
    public Action<string>? RequestScrollToSection { get; set; }

    private void SelectSection(SettingsSectionIndexItemViewModel section)
    {
        foreach (var item in SectionIndexItems)
        {
            item.IsSelected = item == section;
        }
        _selectedSection = section;
        _selectedSectionId = section.Id;
        OnSelectedSectionChanged();
        // 各区块默认始终展示；目录只负责跳转滚动，不再做「只显示一项」切换。
        RequestScrollToSection?.Invoke(section.Id);
    }

    public string ShowAllSectionsText => _displayNames.Text("ui.settings.show_all_sections");
    public string SectionIndexHintText => _displayNames.Text("ui.settings.section_index.hint");

    /// <summary>始终展示当前 Tab 下全部区块；侧栏用于跳转而非显隐开关。</summary>
    private bool IsSectionSelected(string id) => true;

    private void OnSelectedSectionChanged()
    {
        OnPropertyChanged(nameof(IsSectionProjectSelected));
        OnPropertyChanged(nameof(IsSectionDirectoriesSelected));
        OnPropertyChanged(nameof(IsSectionProjectMemorySelected));
        OnPropertyChanged(nameof(IsSectionProviderSelected));
        OnPropertyChanged(nameof(IsSectionAvailableModelsSelected));
        OnPropertyChanged(nameof(IsSectionEmbeddingSelected));
        OnPropertyChanged(nameof(IsSectionManualModelsSelected));
        OnPropertyChanged(nameof(IsSectionSecretSelected));
        OnPropertyChanged(nameof(IsSectionNodePresetsSelected));
        OnPropertyChanged(nameof(IsSectionDefaultsSelected));
        OnPropertyChanged(nameof(IsSectionTemplatesSelected));
        OnPropertyChanged(nameof(IsSectionBudgetSelected));
        OnPropertyChanged(nameof(IsSectionConfirmationsSelected));
        OnPropertyChanged(nameof(IsSectionRuntimeSelected));
        OnPropertyChanged(nameof(IsSectionCapabilitiesSelected));
        OnPropertyChanged(nameof(IsSectionToolControlsSelected));
        OnPropertyChanged(nameof(IsSectionPathsSelected));
        OnPropertyChanged(nameof(IsSectionThemeSelected));
        OnPropertyChanged(nameof(IsSectionWorkspaceSelected));
        OnPropertyChanged(nameof(IsSectionRetrievalSelected));
        OnPropertyChanged(nameof(IsSectionGitSelected));
        OnPropertyChanged(nameof(IsSectionLanguageSelected));
        OnPropertyChanged(nameof(IsSectionDiagnosticsSelected));
    }

    private async Task LoadAsync()
    {
        IsLoading = true;
        try
        {
            _suppressDirtyTracking = true;
            var app = await _backend.GetAppSettingsAsync().ConfigureAwait(true);
            _schemaVersion = app.App.SchemaVersion;
            ProjectName = app.App.ProjectName;
            Locale = app.App.Locale;
            ApplySavedLanguage(app.App.Locale);
            DocumentsDir = app.App.DocumentsDir;
            WorkflowsDir = app.App.WorkflowsDir;
            SkillsDir = app.App.SkillsDir;
            ExportsDir = app.App.ExportsDir;
            ProjectMemory = await _backend.ReadProjectMemoryAsync().ConfigureAwait(true);

            await LoadProviderConfigAsync().ConfigureAwait(true);

            var presets = await _backend.GetNodePresetSettingsAsync().ConfigureAwait(true);
            DefaultModelId = presets.DefaultModelId;
            DefaultTimeoutMs = presets.DefaultTimeoutMs.ToString();
            DefaultBudgetUsd = presets.DefaultBudgetUsd.ToString("0.####");
            ApplyNodePresets(presets);

            var template = await _backend.GetTemplateRepositorySettingsAsync().ConfigureAwait(true);
            TemplateRepositoryBaseUrl = template.BaseUrl;

            var automation = await _backend.GetAutomationSettingsAsync().ConfigureAwait(true);
            ApplyAutomation(automation);

            var workflow = await _backend.GetWorkflowSettingsAsync().ConfigureAwait(true);
            _workflowSchemaVersion = workflow.Workflow.SchemaVersion;
            WorkflowDefaultTimeoutMs = workflow.Workflow.DefaultTimeoutMs.ToString();
            MaxLoopIterations = workflow.Workflow.MaxLoopIterations.ToString();
            MaxToolRounds = workflow.Workflow.MaxToolRounds.ToString();
            CheckpointEnabled = workflow.Workflow.CheckpointEnabled;
            RuntimeAutosaveMs = workflow.Workflow.RuntimeAutosaveMs.ToString();

            var permissions = await _backend.GetPermissionsSettingsAsync().ConfigureAwait(true);
            ApplyPermissions(permissions);

            _uiPreferences = await _backend.GetUiPreferencesAsync().ConfigureAwait(true);
            // 不经 Theme setter：避免强制覆盖已保存的三色自定义
            _theme = ThemeCatalog.Normalize(_uiPreferences.Theme);
            OnPropertyChanged(nameof(Theme));
            SyncThemeOptionSelection();
            OnPropertyChanged(nameof(SelectedThemeOption));
            LoadThemeColorsFromPreferences(_uiPreferences);
            GitAutoColor = _uiPreferences.GitAutoColor;
            GitManualColor = _uiPreferences.GitManualColor;
            ProjectPanelVisible = _uiPreferences.ProjectPanelVisible;
            OnboardingSeen = _uiPreferences.OnboardingSeen;

            var rag = await _backend.GetRagSettingsAsync().ConfigureAwait(true);
            _ragSchemaVersion = rag.Rag.SchemaVersion;
            _vectorBackend = rag.Rag.VectorStore.Backend;
            _qdrantHost = rag.Rag.VectorStore.Sidecar.Host;
            _qdrantPort = rag.Rag.VectorStore.Sidecar.Port.ToString();
            _qdrantDataDir = rag.Rag.VectorStore.Sidecar.DataDir;
            _fullTextBackend = rag.Rag.FullTextStore.Backend;
            _fullTextIndexDir = rag.Rag.FullTextStore.IndexDir;
            RerankerEnabled = rag.Rag.RerankerEnabled;
            ChunkSizeChars = rag.Rag.ChunkSizeChars.ToString();
            ChunkOverlapChars = rag.Rag.ChunkOverlapChars.ToString();

            var git = await _backend.GetGitSettingsAsync().ConfigureAwait(true);
            _gitSchemaVersion = git.Git.SchemaVersion;
            TrackDocuments = git.Git.TrackDocuments;
            TrackWorkflows = git.Git.TrackWorkflows;
            TrackSkills = git.Git.TrackSkills;
            TrackNonSensitiveConfig = git.Git.TrackNonSensitiveConfig;
            IgnoredPathsText = string.Join(Environment.NewLine, git.Git.IgnoredPaths);

            var diagnostics = await _backend.GetBackendDiagnosticsAsync().ConfigureAwait(true);
            DiagnosticsStatus = diagnostics.Status;

            HasUnsavedChanges = false;
            CaptureSnapshot();
            StatusText = _displayNames.Text("ui.common.configured");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
        finally
        {
            _suppressDirtyTracking = false;
            IsLoading = false;
        }
    }

    private async Task LoadProviderConfigAsync()
    {
        try
        {
            _providerConfig = await _backend.GetProviderConfigAsync().ConfigureAwait(true);
            RebuildProviderOptionsFromConfig(preferProviderId: ProviderId);
        }
        catch (Exception ex)
        {
            ProviderStatus = ex.Message;
        }
    }

    private void RebuildProviderOptionsFromConfig(string? preferProviderId)
    {
        ProviderOptions.Clear();
        if (_providerConfig is null)
        {
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
                isDraft: false));
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
            option => _ = SelectProviderOptionAsync(option),
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
        }
        finally
        {
            _suppressProviderSelectionChange = false;
        }
    }

    private async Task SelectProviderOptionAsync(ProviderOptionViewModel option)
    {
        // 仅在离开成功时改列表选中；取消/保存失败时保持与编辑器一致。
        var switched = await SwitchProviderForEditingAsync(option, SelectedProviderOption).ConfigureAwait(true);
        if (switched)
        {
            SetSelectedProviderOption(option.ProviderId);
        }
    }

    private async void AddProviderDraft()
    {
        await AddProviderDraftAsync().ConfigureAwait(true);
    }

    private async Task AddProviderDraftAsync()
    {
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
            CaptureSnapshot();
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
            CaptureCurrentFormToOption(providerId, markDraft: false);
            SetSelectedProviderOption(providerId);
            CaptureSnapshot();
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
        CaptureSnapshot();
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
                        await SaveProviderConnectionAsync().ConfigureAwait(true);
                        // Save 已刷新 _providerConfig；再把当前表单写入快照，供再选中时优先于缓存。
                        MarkSavedSnapshotRange(SnapshotModelStartIndex, SnapshotModelEndIndex);
                        if (stashOnSuccess)
                        {
                            CaptureCurrentFormToOption(previousId, markDraft: false);
                        }
                        return true;
                    }
                    catch (Exception ex)
                    {
                        StatusText = ex.Message;
                        return false;
                    }
                case UnsavedLeaveChoice.Discard:
                    // 丢弃脏编辑：不覆盖草稿快照（保留上次干净快照）。
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

    private async Task<bool> SwitchProviderForEditingAsync(
        ProviderOptionViewModel target,
        ProviderOptionViewModel? previous)
    {
        if (string.Equals(target.ProviderId, ProviderId, StringComparison.Ordinal))
        {
            return true;
        }

        if (!await TryLeaveCurrentProviderAsync(stashOnSuccess: true).ConfigureAwait(true))
        {
            RestoreSelectedProviderOption(previous);
            return false;
        }

        SelectProviderForEditing(target.ProviderId);
        MarkSavedSnapshotRange(SnapshotModelStartIndex, SnapshotModelEndIndex);
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
            ApiKey = string.Empty;
            ModelsText = snapshot.ModelsText;
            EmbeddingModelId = snapshot.EmbeddingModelId;
            AvailableModels.Clear();
            foreach (var line in ParseModels(ModelsText))
            {
                AvailableModels.Add(new ModelOptionViewModel(line.ModelId, line.Capability));
            }
            RebuildAvailableModelIds();
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
        ApiKey = string.Empty;
        ModelsText = string.Join(Environment.NewLine, selected.Models.Select(ModelLine));
        EmbeddingModelId = selected.Models.FirstOrDefault(IsEmbeddingModel)?.ModelId ?? string.Empty;
        AvailableModels.Clear();
        foreach (var model in selected.Models)
        {
            AvailableModels.Add(new ModelOptionViewModel(model.ModelId, model.Capability));
        }
        RebuildAvailableModelIds();
    }

    private async Task FetchModelsAsync()
    {
        IsLoading = true;
        try
        {
            await SaveProviderConnectionAsync().ConfigureAwait(true);
            var result = await _backend.FetchProviderModelsAsync(ProviderId).ConfigureAwait(true);
            ProviderId = result.ProviderId;
            ModelsText = string.Join(Environment.NewLine, result.Models.Select(ModelLine));
            EmbeddingModelId = result.Models.FirstOrDefault(IsEmbeddingModel)?.ModelId ?? string.Empty;
            ManualModelsVisible = false;
            AvailableModels.Clear();
            foreach (var model in result.Models)
            {
                AvailableModels.Add(new ModelOptionViewModel(model.ModelId, model.Capability));
            }
            RebuildAvailableModelIds();
            await SaveProviderConnectionAsync().ConfigureAwait(true);
            await LoadProviderConfigAsync().ConfigureAwait(true);
            CaptureSnapshot();
            StatusText = _displayNames.Text("ui.common.configured");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
        finally
        {
            IsLoading = false;
        }
    }

    private async Task SaveGeneralAsync()
    {
        await RunWithStatusAsync(async () =>
        {
            await _backend.SaveAppSettingsAsync(new AppSettings(new AppConfig(
                _schemaVersion, ProjectName, Locale, DocumentsDir, WorkflowsDir, SkillsDir, ExportsDir))).ConfigureAwait(true);
            await _backend.WriteProjectMemoryAsync(ProjectMemory).ConfigureAwait(true);
        });
    }

    private async Task SaveModelAsync()
    {
        await RunWithStatusAsync(async () =>
        {
            await SaveProviderConnectionAsync().ConfigureAwait(true);
            await LoadProviderConfigAsync().ConfigureAwait(true);
        });
    }

    private async Task SaveProviderConnectionAsync()
    {
        var update = new ProviderSettingsUpdate(
            ProviderId,
            ProviderType,
            ProviderDisplayName,
            ProviderEnabled,
            string.IsNullOrWhiteSpace(ProviderBaseUrl) ? null : ProviderBaseUrl,
            MergeEmbeddingModel(ParseModels(ModelsText), EmbeddingModelId),
            MakeDefaultLlm,
            MakeDefaultEmbedding,
            MakeDefaultReranker);
        // 使用返回值刷新本地缓存，避免 leave-save 后 _providerConfig 仍是旧列表/旧 BaseUrl。
        var status = await _backend.SaveProviderSettingsAsync(update).ConfigureAwait(true);
        if (!string.IsNullOrWhiteSpace(ApiKey))
        {
            await _backend.SaveProviderKeyAsync(ProviderId, ApiKey).ConfigureAwait(true);
            ApiKey = string.Empty;
            status = await _backend.GetProviderConfigAsync().ConfigureAwait(true);
        }

        MergeProviderConfigCache(status, preserveFormSnapshots: true);
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
                existing.IsDraft = false;
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
                    isDraft: false));
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
    }

    private async Task SaveProviderKeyAsync()
    {
        await RunWithStatusAsync(async () =>
        {
            await _backend.SaveProviderKeyAsync(ProviderId, ApiKey).ConfigureAwait(true);
            ApiKey = string.Empty;
            await LoadProviderConfigAsync().ConfigureAwait(true);
        });
    }

    private async Task SavePresetsAsync()
    {
        await RunWithStatusAsync(async () =>
        {
            await _backend.SaveNodePresetSettingsAsync(new NodePresetSettings(
                NodePresets.Select(item => new NodeTypePreset(
                    item.NodeType,
                    item.DisplayNameKey,
                    item.ModelId,
                    ParseLong(item.TimeoutMs, 300000),
                    ParseDouble(item.BudgetUsd, 0))).ToArray(),
                DefaultModelId,
                ParseLong(DefaultTimeoutMs, 300000),
                ParseDouble(DefaultBudgetUsd, 0))).ConfigureAwait(true);
        });
    }

    private async Task SaveTemplateRepositoryAsync()
    {
        await RunWithStatusAsync(async () =>
        {
            var saved = await _backend.SaveTemplateRepositorySettingsAsync(new TemplateRepositorySettings(TemplateRepositoryBaseUrl)).ConfigureAwait(true);
            TemplateRepositoryBaseUrl = saved.BaseUrl;
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

    private async Task SaveAutomationAsync()
    {
        await RunWithStatusAsync(async () =>
        {
            var automation = new AutomationSettings(
                new BudgetStatus(ParseDouble(BudgetUsd, 0), ParseDouble(SpentText.TrimStart('$'), 0), ParseDouble(PreauthorizedUsd, 0), AutoModeEnabled),
                ConfirmationPolicies.Select(item => new ConfirmationPolicySetting(
                    item.Kind,
                    item.NormalPolicy,
                    item.AutoModePolicy)).ToArray());
            var budget = await _backend.UpdateBudgetConfigAsync(ParseDouble(BudgetUsd, 0), ParseDouble(PreauthorizedUsd, 0)).ConfigureAwait(true);
            await _backend.SetAutoModeAsync(AutoModeEnabled).ConfigureAwait(true);
            SpentText = $"${budget.SpentUsd:0.####}";
            await _backend.SaveAutomationSettingsAsync(automation).ConfigureAwait(true);
            await _backend.SaveWorkflowSettingsAsync(new WorkflowSettings(new WorkflowConfig(
                _workflowSchemaVersion,
                ParseLong(WorkflowDefaultTimeoutMs, 300000),
                ParseInt(MaxLoopIterations, 5),
                ParseInt(MaxToolRounds, 8),
                CheckpointEnabled,
                ParseLong(RuntimeAutosaveMs, 5000)))).ConfigureAwait(true);
        });
    }

    private async Task SavePermissionsAsync()
    {
        await RunWithStatusAsync(async () =>
        {
            await _backend.SavePermissionsSettingsAsync(new PermissionsSettings(new PermissionPolicy(
                AllowNetwork,
                AllowWebSearch,
                AllowHttpSkill,
                AllowWasmNetwork,
                AllowSecretRead,
                Lines(WritableRootsText),
                Lines(ReadableRootsText)),
                ToToolControls())).ConfigureAwait(true);
        });
    }

    private async Task SavePersonalizationAsync()
    {
        await RunWithStatusAsync(async () =>
        {
            var preferences = BuildUiPreferences();
            await _backend.SaveUiPreferencesAsync(preferences).ConfigureAwait(true);
            ApplyCurrentThemeColors();
            _uiPreferences = preferences;
        });
    }

    private UiPreferences BuildUiPreferences() =>
        new(
            Theme,
            GitAutoColor,
            GitManualColor,
            ProjectPanelVisible,
            _uiPreferences?.ProjectPanelPosition,
            _uiPreferences?.PanelStates ?? new Dictionary<string, bool>(),
            OnboardingSeen,
            ThemeMainColor,
            ThemeSurfaceColor,
            ThemeBrandColor);

    private async Task ResetOnboardingAsync()
    {
        IsLoading = true;
        var wasDirty = HasUnsavedChanges;
        try
        {
            OnboardingSeen = false;
            var preferences = BuildUiPreferences();
            await _backend.SaveUiPreferencesAsync(preferences).ConfigureAwait(true);
            ApplyCurrentThemeColors();
            _uiPreferences = preferences;
            StatusText = _displayNames.Text("ui.common.configured");
            if (wasDirty)
            {
                MarkSavedSnapshotValue(SnapshotOnboardingSeenIndex);
            }
            else
            {
                CaptureSnapshot();
            }
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
        finally
        {
            IsLoading = false;
        }
    }

    private async Task SaveMiscAsync()
    {
        await RunWithStatusAsync(async () =>
        {
            await _backend.SaveRagSettingsAsync(new RagSettings(new RagConfig(
                _ragSchemaVersion,
                new VectorStoreConfig(_vectorBackend, new SidecarConfig(_qdrantHost, ParseInt(_qdrantPort, 6333), _qdrantDataDir)),
                new FullTextStoreConfig(_fullTextBackend, _fullTextIndexDir),
                RerankerEnabled,
                ParseInt(ChunkSizeChars, 2000),
                ParseInt(ChunkOverlapChars, 200)))).ConfigureAwait(true);
            await _backend.SaveGitSettingsAsync(new GitSettings(new GitConfig(
                _gitSchemaVersion,
                TrackDocuments,
                TrackWorkflows,
                TrackSkills,
                TrackNonSensitiveConfig,
                Lines(IgnoredPathsText)))).ConfigureAwait(true);
        });
    }

    private void ApplyAutomation(AutomationSettings automation)
    {
        BudgetUsd = automation.Budget.BudgetUsd.ToString("0.####");
        PreauthorizedUsd = automation.Budget.PreauthorizedUsd.ToString("0.####");
        AutoModeEnabled = automation.Budget.AutoModeEnabled;
        SpentText = $"${automation.Budget.SpentUsd:0.####}";
        ConfirmationPolicies.Clear();
        foreach (var item in automation.ConfirmationPolicies)
        {
            ConfirmationPolicies.Add(new ConfirmationPolicyViewModel(
                item.ConfirmationKind,
                ConfirmationLabel(item.ConfirmationKind),
                item.NormalPolicy,
                item.AutoModePolicy,
                () => HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot));
        }
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
        ApplyToolControls(settings.ToolControls);
    }

    private void ApplyToolControls(IReadOnlyDictionary<string, IReadOnlyDictionary<string, bool>>? toolControls)
    {
        ToolControlGroups.Clear();
        foreach (var (scope, controls) in (toolControls ?? new Dictionary<string, IReadOnlyDictionary<string, bool>>()).OrderBy(item => item.Key, StringComparer.Ordinal))
        {
            var group = new ToolControlGroupViewModel(scope, ToolScopeLabel(scope));
            foreach (var (tool, enabled) in controls.OrderBy(item => item.Key, StringComparer.Ordinal))
            {
                group.Controls.Add(new ToolControlItemViewModel(
                    tool,
                    ToolLabel(scope, tool),
                    enabled,
                    ToolControlItemViewModel.IsDangerToolId(tool),
                    () => HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot));
            }
            group.RefreshPartitions();
            ToolControlGroups.Add(group);
        }
    }

    private IReadOnlyDictionary<string, IReadOnlyDictionary<string, bool>> ToToolControls()
    {
        return ToolControlGroups.ToDictionary(
            group => group.Scope,
            group => (IReadOnlyDictionary<string, bool>)group.Controls.ToDictionary(
                item => item.ToolId,
                item => item.IsEnabled,
                StringComparer.Ordinal),
            StringComparer.Ordinal);
    }

    private void ApplyNodePresets(NodePresetSettings settings)
    {
        NodePresets.Clear();
        foreach (var preset in settings.Presets)
        {
            NodePresets.Add(new NodeTypePresetViewModel(
                preset.NodeType,
                preset.DisplayNameKey,
                _displayNames.Text(preset.DisplayNameKey),
                preset.ModelId,
                preset.TimeoutMs.ToString(),
                preset.BudgetUsd.ToString("0.####"),
                () => HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot));
        }
    }

    private async Task RunWithStatusAsync(Func<Task> action)
    {
        IsLoading = true;
        try
        {
            await action().ConfigureAwait(true);
            CaptureSnapshot();
            StatusText = _displayNames.Text("ui.common.configured");
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
        finally
        {
            IsLoading = false;
        }
    }

    private string ConfirmationLabel(string kind)
    {
        return kind switch
        {
            "chapter_write" => _displayNames.Text("ui.settings.automation.confirmation.chapter_write"),
            "summary_write" => _displayNames.Text("ui.settings.automation.confirmation.summary_write"),
            "high_risk_permission" => _displayNames.Text("ui.settings.automation.confirmation.high_risk_permission"),
            "budget_exceeded" => _displayNames.Text("ui.settings.automation.confirmation.budget_exceeded"),
            _ => kind,
        };
    }

    private string ToolScopeLabel(string scope)
    {
        return scope switch
        {
            "project_ai" => _displayNames.Text("ui.settings.permissions.tool_scope.project_ai"),
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
            "register" => _displayNames.Text("ui.settings.permissions.tool.register"),
            "find" => _displayNames.Text("ui.settings.permissions.tool.find"),
            "search" => _displayNames.Text("ui.settings.permissions.tool.search"),
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
            model.MaxContextTokens?.ToString() ?? string.Empty,
            model.InputCostPerMillionTokens?.ToString("0.####") ?? string.Empty,
            model.OutputCostPerMillionTokens?.ToString("0.####") ?? string.Empty,
        });
    }

    private static IReadOnlyList<ModelConfig> ParseModels(string text)
    {
        return Lines(text)
            .Select(line => line.Split(',', StringSplitOptions.TrimEntries))
            .Where(parts => parts.Length >= 2 && !string.IsNullOrWhiteSpace(parts[0]))
            .Select(parts => new ModelConfig(
                parts[0],
                parts[1],
                parts.Length > 2 && int.TryParse(parts[2], out var context) ? context : null,
                parts.Length > 3 && double.TryParse(parts[3], out var input) ? input : null,
                parts.Length > 4 && double.TryParse(parts[4], out var output) ? output : null))
            .ToArray();
    }

    private void RebuildAvailableModelIds()
    {
        AvailableModelIds.Clear();
        foreach (var model in AvailableModels
                     .Select(item => item.ModelId)
                     .Where(id => !string.IsNullOrWhiteSpace(id))
                     .Distinct(StringComparer.Ordinal)
                     .OrderBy(id => id, StringComparer.Ordinal))
        {
            AvailableModelIds.Add(model);
        }
        OnPropertyChanged(nameof(HasAvailableModelChoices));
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
            merged[existing] = model with { Capability = "embedding" };
        }
        else
        {
            merged.Add(new ModelConfig(trimmed, "embedding", null, null, null));
        }

        return merged;
    }

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

    private static int ParseInt(string text, int fallback) => CultureNumberParse.ParseInt(text, fallback);
    private static long ParseLong(string text, long fallback) => CultureNumberParse.ParseLong(text, fallback);
    private static double ParseDouble(string text, double fallback) => CultureNumberParse.ParseDouble(text, fallback);

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
                await SaveCurrentSectionAsync().ConfigureAwait(true);
                return !HasUnsavedChanges;
            case UnsavedLeaveChoice.Discard:
                await LoadAsync().ConfigureAwait(true);
                return true;
            default:
                return false;
        }
    }

    private Task SaveCurrentSectionAsync()
    {
        return SelectedTab.Id switch
        {
            "general" => SaveGeneralAsync(),
            "models" => SaveModelSectionAsync(),
            "presets" => SavePresetSectionAsync(),
            "automation" => SaveAutomationAsync(),
            "permissions" => SavePermissionsAsync(),
            "personalization" => SavePersonalizationAsync(),
            "misc" => SaveMiscAsync(),
            _ => Task.CompletedTask,
        };
    }

    private async Task SaveModelSectionAsync()
    {
        await SaveModelAsync().ConfigureAwait(true);
        if (!string.IsNullOrWhiteSpace(ApiKey))
        {
            await SaveProviderKeyAsync().ConfigureAwait(true);
        }
    }

    private async Task SavePresetSectionAsync()
    {
        await SavePresetsAsync().ConfigureAwait(true);
        await SaveTemplateRepositoryAsync().ConfigureAwait(true);
    }

    protected override void OnPropertyChanged(string? propertyName = null)
    {
        base.OnPropertyChanged(propertyName);
        if (!_suppressDirtyTracking && IsTrackedDirtyProperty(propertyName))
        {
            HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot;
        }
    }

    private void CaptureSnapshot()
    {
        _savedSnapshot = CurrentSnapshot();
        HasUnsavedChanges = false;
    }

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

    private async Task PersistLanguageAsync(string language)
    {
        IsLoading = true;
        var wasDirty = HasUnsavedChanges;
        try
        {
            var savedSettings = await _backend.GetAppSettingsAsync().ConfigureAwait(true);
            var savedApp = savedSettings.App with { Locale = language };
            await _backend.SaveAppSettingsAsync(new AppSettings(savedApp)).ConfigureAwait(true);
            _schemaVersion = savedApp.SchemaVersion;
            Locale = language;
            StatusText = _displayNames.Text("ui.common.configured");
            if (wasDirty)
            {
                MarkSavedSnapshotValue(SnapshotLocaleIndex);
            }
            else
            {
                CaptureSnapshot();
            }
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
        finally
        {
            IsLoading = false;
        }
    }

    private void MarkSavedSnapshotValue(int snapshotIndex)
    {
        MarkSavedSnapshotRange(snapshotIndex, snapshotIndex);
    }

    private void MarkSavedSnapshotRange(int startIndex, int endIndex)
    {
        var currentSnapshot = CurrentSnapshot();
        var savedParts = _savedSnapshot.Split(SnapshotSeparator);
        var currentParts = currentSnapshot.Split(SnapshotSeparator);
        if (savedParts.Length == currentParts.Length
            && startIndex >= 0
            && endIndex >= startIndex
            && currentParts.Length > endIndex)
        {
            for (var index = startIndex; index <= endIndex; index++)
            {
                savedParts[index] = currentParts[index];
            }
            _savedSnapshot = string.Join(SnapshotSeparator, savedParts);
        }
        HasUnsavedChanges = currentSnapshot != _savedSnapshot;
    }

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

        foreach (var option in ThemeOptions)
        {
            option.Label = ThemeLabelFor(option.Code, _displayNames);
            option.Description = ThemeDescriptionFor(option.Code, _displayNames);
            option.GroupTitle = ThemeGroupTitleFor(option.Group, _displayNames);
        }
        OnPropertyChanged(nameof(ThemeOptionGroups));

        foreach (var tab in Tabs)
        {
            tab.Title = tab.Id switch
            {
                "general" => _displayNames.Text("ui.settings.tab.general"),
                "models" => _displayNames.Text("ui.settings.tab.models"),
                "presets" => _displayNames.Text("ui.settings.tab.presets"),
                "automation" => _displayNames.Text("ui.settings.tab.automation"),
                "permissions" => _displayNames.Text("ui.settings.tab.permissions"),
                "personalization" => _displayNames.Text("ui.settings.tab.personalization"),
                "misc" => _displayNames.Text("ui.settings.tab.misc"),
                _ => tab.Title,
            };
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

        RebuildSectionIndex();
    }

    private string CurrentSnapshot()
    {
        var confirmationSnapshot = string.Join("|", ConfirmationPolicies.Select(policy =>
            $"{policy.Kind}:{policy.NormalPolicy}:{policy.AutoModePolicy}"));
        var toolControlSnapshot = string.Join("|", ToolControlGroups.SelectMany(group =>
            group.Controls.Select(item => $"{group.Scope}:{item.ToolId}:{item.IsEnabled}")));
        return string.Join(SnapshotSeparator, new[]
        {
            ProjectName,
            Locale,
            DocumentsDir,
            WorkflowsDir,
            SkillsDir,
            ExportsDir,
            ProjectMemory,
            ProviderId,
            ProviderType,
            ProviderDisplayName,
            ProviderBaseUrl,
            ProviderEnabled.ToString(),
            MakeDefaultLlm.ToString(),
            MakeDefaultEmbedding.ToString(),
            MakeDefaultReranker.ToString(),
            ApiKey,
            ModelsText,
            EmbeddingModelId,
            ManualModelsVisible.ToString(),
            DefaultModelId,
            DefaultTimeoutMs,
            DefaultBudgetUsd,
            string.Join("|", NodePresets.Select(preset => preset.Snapshot)),
            TemplateRepositoryBaseUrl,
            BudgetUsd,
            PreauthorizedUsd,
            AutoModeEnabled.ToString(),
            WorkflowDefaultTimeoutMs,
            MaxLoopIterations,
            MaxToolRounds,
            CheckpointEnabled.ToString(),
            RuntimeAutosaveMs,
            confirmationSnapshot,
            AllowNetwork.ToString(),
            AllowWebSearch.ToString(),
            AllowHttpSkill.ToString(),
            AllowWasmNetwork.ToString(),
            AllowSecretRead.ToString(),
            ReadableRootsText,
            WritableRootsText,
            toolControlSnapshot,
            Theme,
            ThemeMainColor,
            ThemeSurfaceColor,
            ThemeBrandColor,
            GitAutoColor,
            GitManualColor,
            ProjectPanelVisible.ToString(),
            OnboardingSeen.ToString(),
            RerankerEnabled.ToString(),
            ChunkSizeChars,
            ChunkOverlapChars,
            TrackDocuments.ToString(),
            TrackWorkflows.ToString(),
            TrackSkills.ToString(),
            TrackNonSensitiveConfig.ToString(),
            IgnoredPathsText,
        });
    }

    private static bool IsTrackedDirtyProperty(string? propertyName)
    {
        return propertyName is
            nameof(ProjectName) or nameof(Locale) or nameof(DocumentsDir) or nameof(WorkflowsDir)
            or nameof(SkillsDir) or nameof(ExportsDir) or nameof(ProjectMemory) or nameof(ProviderId) or nameof(ProviderType)
            or nameof(ProviderDisplayName) or nameof(ProviderBaseUrl) or nameof(ProviderEnabled)
            or nameof(MakeDefaultLlm) or nameof(MakeDefaultEmbedding) or nameof(MakeDefaultReranker)
            or nameof(ModelsText) or nameof(EmbeddingModelId) or nameof(ManualModelsVisible) or nameof(ApiKey) or nameof(DefaultModelId)
            or nameof(DefaultTimeoutMs) or nameof(DefaultBudgetUsd) or nameof(TemplateRepositoryBaseUrl)
            or nameof(BudgetUsd) or nameof(PreauthorizedUsd) or nameof(AutoModeEnabled)
            or nameof(WorkflowDefaultTimeoutMs) or nameof(MaxLoopIterations) or nameof(MaxToolRounds)
            or nameof(CheckpointEnabled) or nameof(RuntimeAutosaveMs) or nameof(AllowNetwork)
            or nameof(AllowWebSearch) or nameof(AllowHttpSkill) or nameof(AllowWasmNetwork)
            or nameof(AllowSecretRead) or nameof(ReadableRootsText) or nameof(WritableRootsText)
            or nameof(Theme) or nameof(ThemeMainColor) or nameof(ThemeSurfaceColor) or nameof(ThemeBrandColor)
            or nameof(GitAutoColor) or nameof(GitManualColor)
            or nameof(ProjectPanelVisible) or nameof(OnboardingSeen) or nameof(RerankerEnabled)
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
    private bool _isEnabled;

    public ToolControlItemViewModel(
        string toolId,
        string displayName,
        bool isEnabled,
        bool isDangerous,
        Action markDirty)
    {
        ToolId = toolId;
        _displayName = displayName;
        _isEnabled = isEnabled;
        IsDangerous = isDangerous;
        _markDirty = markDirty;
    }

    public string ToolId { get; }
    public string DisplayName { get => _displayName; set => SetProperty(ref _displayName, value); }
    public bool IsDangerous { get; }

    /// <summary>写盘/重写类工具视为危险，与权限页 warning 分组共用。</summary>
    public static bool IsDangerToolId(string toolId)
    {
        var id = (toolId ?? string.Empty).Trim().ToLowerInvariant();
        if (string.IsNullOrEmpty(id))
        {
            return false;
        }
        return id.Contains("rewrite-file", StringComparison.Ordinal)
               || id.Contains("replace-lines", StringComparison.Ordinal)
               || id.Contains("insert-lines", StringComparison.Ordinal)
               || id.Contains("secret", StringComparison.Ordinal)
               || id.EndsWith("-delete", StringComparison.Ordinal)
               || id.Contains("delete-file", StringComparison.Ordinal);
    }

    public bool IsEnabled
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

public sealed class ConfirmationPolicyViewModel : ViewModelBase
{
    private string _label;
    private bool _normalAllowByDefault;
    private bool _autoModeAutoApproval;

    private readonly Action _markDirty;

    public ConfirmationPolicyViewModel(string kind, string label, string normalPolicy, string autoModePolicy, Action markDirty)
    {
        Kind = kind;
        _label = label;
        _markDirty = markDirty;
        _normalAllowByDefault = normalPolicy == "allow_by_default";
        _autoModeAutoApproval = autoModePolicy == "auto_approval";
    }

    public string Kind { get; }
    public string Label { get => _label; set => SetProperty(ref _label, value); }
    public string NormalPolicy => NormalAllowByDefault ? "allow_by_default" : "manual_review";
    public string AutoModePolicy => AutoModeAutoApproval ? "auto_approval" : "allow_by_default";

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

public sealed class SettingsSectionIndexItemViewModel : ViewModelBase
{
    private bool _isSelected;

    public SettingsSectionIndexItemViewModel(string id, string title, Action<SettingsSectionIndexItemViewModel> select)
    {
        Id = id;
        Title = title;
        SelectCommand = new RelayCommand(() => select(this));
    }

    public string Id { get; }
    public string Title { get; }
    public RelayCommand SelectCommand { get; }
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }
}
