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
    private const string RetrievalSection = "retrieval";
    private const string GitSection = "git";
    private const string OfficialTemplateRepositoryUrl = "ariadne://official-templates/v1";
    private static readonly string[] RecommendedGitIgnoredPaths =
    {
        ".cache/", ".runtime/", ".indexes/", ".knowledge/", "costs.db", "runtime.db",
    };
    private static readonly (string Id, string DisplayNameKey)[] ModelAliasDefinitions =
    {
        ("planning", "ui.settings.presets.alias.planning"),
        ("writing", "ui.settings.presets.alias.writing"),
        ("review", "ui.settings.presets.alias.review"),
    };
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
        nameof(RetrievalTitle),
        nameof(VersionControlTitle),
        nameof(SupportTitle),
        nameof(AdvancedSettingsText),
        nameof(AppRuntimeScopeHelpText),
        nameof(RetrievalScopeHelpText),
        nameof(ProjectNameLabel),
        nameof(ProjectRootLabel),
        nameof(DirectorySwitchWarningText),
        nameof(DocumentsDirLabel),
        nameof(WorkflowsDirLabel),
        nameof(SkillsDirLabel),
        nameof(ExportsDirLabel),
        nameof(ProjectMemoryLabel),
        nameof(ProjectMemoryPlaceholder),
        nameof(SaveGeneralText),
        nameof(ProviderIdLabel),
        nameof(CopyProviderIdText),
        nameof(ProviderTypeLabel),
        nameof(ProviderDisplayNameLabel),
        nameof(BaseUrlLabel),
        nameof(BaseUrlPlaceholder),
        nameof(ProviderEnabledText),
        nameof(MakeDefaultLlmText),
        nameof(MakeDefaultEmbeddingText),
        nameof(MakeDefaultRerankerText),
        nameof(MakeDefaultSearchText),
        nameof(DefaultLlmRouteLabel),
        nameof(DefaultEmbeddingRouteLabel),
        nameof(DefaultRerankerRouteLabel),
        nameof(DefaultSearchRouteLabel),
        nameof(AvailableModelsText),
        nameof(ManualModelsText),
        nameof(ModelsTextLabel),
        nameof(ModelsPlaceholder),
        nameof(ModelIdColumnLabel),
        nameof(ModelCapabilityColumnLabel),
        nameof(ModelContextColumnLabel),
        nameof(ModelInputCostColumnLabel),
        nameof(ModelOutputCostColumnLabel),
        nameof(AddModelText),
        nameof(RemoveModelText),
        nameof(EmbeddingModelLabel),
        nameof(EmbeddingModelPlaceholder),
        nameof(ApiKeyLabel),
        nameof(ApiKeyPlaceholder),
        nameof(SaveModelText),
        nameof(SaveKeyText),
        nameof(RevokeKeyText),
        nameof(RemoveProviderText),
        nameof(RefreshText),
        nameof(TestProviderDraftText),
        nameof(LegacyOtherProviderMessage),
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
        nameof(ModelAliasesTitle),
        nameof(ModelAliasTargetLabel),
        nameof(ModelAliasesHelpText),
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
        nameof(ConfirmationProfileLabel),
        nameof(ConfirmationProfileHelpText),
        nameof(AdvancedConfirmationPoliciesText),
        nameof(BrowseFolderText),
        nameof(BrowseFileText),
        nameof(WorkflowLimitLabel),
        nameof(WorkflowDefaultTimeoutLabel),
        nameof(MaxLoopIterationsLabel),
        nameof(MaxToolRoundsLabel),
        nameof(CheckpointEnabledLabel),
        nameof(RunEventRetentionLabel),
        nameof(SaveAutomationText),
        nameof(AllowNetworkText),
        nameof(AllowWebSearchText),
        nameof(AllowHttpSkillText),
        nameof(AllowWasmNetworkText),
        nameof(AllowSecretReadText),
        nameof(PermissionProfileLabel),
        nameof(PermissionProfileHelpText),
        nameof(AdvancedPermissionsText),
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
        nameof(QdrantTlsText),
        nameof(QdrantAuthModeLabel),
        nameof(QdrantApiKeyLabel),
        nameof(QdrantApiKeyPlaceholder),
        nameof(QdrantApiKeyStatusText),
        nameof(QdrantApiKeyErrorText),
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
        nameof(SaveRetrievalText),
        nameof(SaveGitText),
        nameof(LanguageLabel),
        nameof(TutorialText),
        nameof(OpenTutorialText),
        nameof(DiagnosticsLabel),
        nameof(DiagnosticsStatusText),
        nameof(DiagnosticsEmptyText),
        nameof(RefreshDiagnosticsText),
        nameof(CopyDiagnosticsText),
        nameof(SectionLoadFailureRetryText),
        nameof(CompatibilityPermissionScopesText),
        nameof(RestoreCurrentTabText),
        nameof(RestoreRecommendedDefaultsText),
        nameof(TemplateRepositorySourceLabel),
        nameof(TemplateRepositorySourceText),
        nameof(AdvancedTemplateRepositoryText),
        nameof(RestoreOfficialTemplateRepositoryText),
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
    private readonly RequestGenerationSession _diagnosticsRefreshSession = new();
    private readonly Dictionary<string, Func<Task<bool>>> _failedSectionRetries = new(StringComparer.Ordinal);
    private readonly Dictionary<string, PermissionPolicy?> _compatibilityScopedPolicies = new(StringComparer.Ordinal);

    private readonly record struct PendingSettingsNavigation(
        SettingsTabViewModel Tab,
        SettingsSectionNavigationItemViewModel? Section);

    private sealed record PreparedSettingsCommit(string Section, Func<Task<bool>> Save);

    private sealed record DirectorySwitchChange(string Label, string OldPath, string NewPath, string TargetStatus);

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
    private ProviderModelRouteOption? _selectedDefaultLlmRoute;
    private ProviderModelRouteOption? _selectedDefaultEmbeddingRoute;
    private ProviderModelRouteOption? _selectedDefaultRerankerRoute;
    private ProviderModelRouteOption? _selectedDefaultSearchRoute;

    private string _defaultProviderId = string.Empty;
    private string _defaultModelId = "gpt-4.1-mini";
    private string? _defaultModelAlias;
    private WorkflowModelOption? _selectedDefaultModelOption;
    // Author-facing timeout fields hold **seconds** (same unit as Workspace); convert to ms at save.
    private string _defaultTimeoutMs = "300";
    private string _defaultBudgetUsd = "0";
    private string _templateRepositoryBaseUrl = string.Empty;

    private string _budgetUsd = "0";
    private string _preauthorizedUsd = "0";
    private string _spentText = "$0.00";
    private double _spentUsd;
    private SettingsValueOption? _selectedConfirmationProfile;
    private string _workflowDefaultTimeoutMs = "300";
    private string _maxLoopIterations = "5";
    private string _maxToolRounds = "8";
    private bool _checkpointEnabled = true;
    private string _runEventRetentionDays = "30";

    private bool _allowNetwork;
    private bool _allowWebSearch;
    private bool _allowHttpSkill;
    private bool _allowWasmNetwork;
    private bool _allowSecretRead;
    private SettingsValueOption? _selectedPermissionProfile;
    private bool _applyingPermissionProfile;
    private bool _areAdvancedPermissionsExpanded;
    private bool _areAdvancedConfirmationPoliciesExpanded;
    private bool _areAdvancedRetrievalSettingsExpanded;
    private bool _areAdvancedAppRuntimeSettingsExpanded;
    private string _recoveryText = string.Empty;
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
    private bool _qdrantUseTls;
    private string _qdrantAuthMode = "none";
    private string _qdrantApiKey = string.Empty;
    private bool _hasQdrantApiKey;
    private bool _hasQdrantApiKeyError;
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
    private bool _isDiagnosticsRefreshing;
    private BackendDiagnosticsReport? _diagnosticsReport;
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
        QdrantAuthModeOptions = new ObservableCollection<SettingsValueOption>
        {
            new("none", displayNames.Text("ui.settings.misc.qdrant_auth.none")),
            new("api_key", displayNames.Text("ui.settings.misc.qdrant_auth.api_key")),
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
        ConfirmationProfileOptions = new ObservableCollection<SettingsValueOption>
        {
            new("conservative", displayNames.Text("ui.settings.automation.confirmation.profile.conservative")),
            new("recommended", displayNames.Text("ui.settings.automation.confirmation.profile.recommended")),
            new("automated", displayNames.Text("ui.settings.automation.confirmation.profile.automated")),
            new("custom", displayNames.Text("ui.settings.automation.confirmation.profile.custom")),
        };
        ConfirmationNormalPolicyOptions = new ObservableCollection<SettingsValueOption>
        {
            new("manual_review", displayNames.Text("ui.settings.automation.confirmation.review")),
            new("allow_by_default", displayNames.Text("ui.settings.automation.confirmation.allow")),
        };
        ConfirmationAutoModePolicyOptions = new ObservableCollection<SettingsValueOption>
        {
            new("allow_by_default", displayNames.Text("ui.settings.automation.confirmation.auto_off")),
            new("auto_approval", displayNames.Text("ui.settings.automation.confirmation.auto_on")),
        };
        PermissionProfileOptions = new ObservableCollection<SettingsValueOption>
        {
            new("restricted", displayNames.Text("ui.settings.permissions.profile.restricted")),
            new("recommended", displayNames.Text("ui.settings.permissions.profile.recommended")),
            new("custom", displayNames.Text("ui.settings.permissions.profile.custom")),
        };
        NodePresets = new ObservableCollection<NodeTypePresetViewModel>();
        ModelAliases = new ObservableCollection<ModelAliasViewModel>();
        ProviderOptions = new ObservableCollection<ProviderOptionViewModel>();
        AvailableModels = new ObservableCollection<ModelOptionViewModel>();
        ProviderModels = new ObservableCollection<ProviderModelEditorRow>();
        // U145：模型 ID 候选。来源是「测试连接 / 刷新模型」从服务商真实拉回的模型目录——
        // 产品已经知道这批 id，此前却让用户对着文档手抄（抄错只会在真正调用时才报错）。
        FetchedModelIdCandidates = new ObservableCollection<string>();
        ProviderCapabilityOptions = new ObservableCollection<SettingsValueOption>
        {
            new("llm", displayNames.Text("ui.settings.models.capability.llm")),
            new("tool_use", displayNames.Text("ui.settings.models.capability.tool_use")),
            new("embedding", displayNames.Text("ui.settings.models.capability.embedding")),
            new("reranker", displayNames.Text("ui.settings.models.capability.reranker")),
            new("search", displayNames.Text("ui.settings.models.capability.search")),
        };
        DefaultLlmRouteOptions = new ObservableCollection<ProviderModelRouteOption>();
        DefaultEmbeddingRouteOptions = new ObservableCollection<ProviderModelRouteOption>();
        DefaultRerankerRouteOptions = new ObservableCollection<ProviderModelRouteOption>();
        DefaultSearchRouteOptions = new ObservableCollection<ProviderModelRouteOption>();
        AvailableLlmModelOptions = new ObservableCollection<WorkflowModelOption>();
        AvailableLlmModelTargetOptions = new ObservableCollection<WorkflowModelOption>();
        ToolControlGroups = new ObservableCollection<ToolControlGroupViewModel>();
        ScopedPermissionProfiles = new ObservableCollection<PermissionScopeProfileViewModel>();
        DiagnosticsItems = new ObservableCollection<SettingsDiagnosticItemViewModel>();
        SectionLoadFailures = new ObservableCollection<SettingsSectionLoadFailureViewModel>();
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
        TestProviderDraftCommand = new RelayCommand(() => _ = TestProviderDraftAsync(), CanTestProviderDraft);
        SaveModelCommand = new RelayCommand(() => _ = SaveModelAsync(), () => CanSave(ModelsSection) && !IsLegacyOtherProvider);
        SaveProviderKeyCommand = new RelayCommand(() => _ = SaveProviderKeyAsync(), CanUsePersistedProvider);
        RevokeProviderKeyCommand = new RelayCommand(() => _ = RevokeProviderKeyAsync(), CanRevokeProviderKey);
        RemoveProviderCommand = new RelayCommand(() => _ = RemoveProviderAsync(), CanUsePersistedProvider);
        AddProviderCommand = new RelayCommand(() => _ = AddProviderDraftAsync(), () => CanSave(ModelsSection));
        AddProviderModelCommand = new RelayCommand(AddProviderModelRow, () => CanSave(ModelsSection));
        SavePresetsCommand = new RelayCommand(() => _ = SavePresetsAsync(), () => CanSave(PresetsSection));
        SaveTemplateRepositoryCommand = new RelayCommand(
            () => _ = SaveTemplateRepositoryAsync(),
            () => CanSave(TemplateRepositorySection));
        OpenTemplateMarketCommand = new RelayCommand(() => _ = OpenTemplateMarketAsync());
        SaveAutomationCommand = new RelayCommand(() => _ = SaveAutomationAsync(), () => CanSave(AutomationSection));
        SavePermissionsCommand = new RelayCommand(() => _ = SavePermissionsAsync(), () => CanSave(PermissionsSection));
        SavePersonalizationCommand = new RelayCommand(() => _ = SavePersonalizationAsync(), () => CanSave(PersonalizationSection));
        SaveAppRuntimeCommand = new RelayCommand(() => _ = SaveAppRuntimeAsync(), () => CanSave(AppRuntimeSection));
        SaveRetrievalCommand = new RelayCommand(() => _ = SaveRetrievalAsync(), () => CanSave(RetrievalSection));
        SaveGitCommand = new RelayCommand(() => _ = SaveGitAsync(), () => CanSave(GitSection));
        RestoreCurrentTabCommand = new RelayCommand(
            () => _ = RestoreSelectedTabAsync(),
            CanRestoreSelectedTab);
        SaveCurrentTabCommand = new RelayCommand(
            () => _ = SaveSelectedTabAsync(),
            CanSaveSelectedTab);
        RestoreRecommendedDefaultsCommand = new RelayCommand(
            () => _ = RestoreRecommendedDefaultsAsync(),
            CanRestoreRecommendedDefaults);
        RestoreOfficialTemplateRepositoryCommand = new RelayCommand(
            () => TemplateRepositoryBaseUrl = OfficialTemplateRepositoryUrl,
            () => CanSave(TemplateRepositorySection));
        RefreshDiagnosticsCommand = new RelayCommand(
            () => _ = RefreshDiagnosticsAsync(),
            () => !IsDiagnosticsRefreshing);
        ShowTutorialCommand = new RelayCommand(() => _ = ShowTutorialAsync());
        BrowseDocumentsDirCommand = new RelayCommand(() => _ = BrowseProjectDirectoryAsync(value => DocumentsDir = value));
        BrowseWorkflowsDirCommand = new RelayCommand(() => _ = BrowseProjectDirectoryAsync(value => WorkflowsDir = value));
        BrowseSkillsDirCommand = new RelayCommand(() => _ = BrowseProjectDirectoryAsync(value => SkillsDir = value));
        BrowseExportsDirCommand = new RelayCommand(() => _ = BrowseProjectDirectoryAsync(value => ExportsDir = value));
        BrowseReadableRootsCommand = new RelayCommand(() => _ = BrowseIntoAsync(AppendReadableRoot));
        BrowseWritableRootsCommand = new RelayCommand(() => _ = BrowseIntoAsync(AppendWritableRoot));
        BrowseQdrantBinaryCommand = new RelayCommand(() => _ = BrowseFileIntoAsync(value => QdrantBinaryPath = value));
        SelectThemeMainChannelCommand = new RelayCommand(() => SetActiveThemeColorChannel(ThemeColorChannel.Main));
        SelectThemeSurfaceChannelCommand = new RelayCommand(() => SetActiveThemeColorChannel(ThemeColorChannel.Surface));
        SelectThemeBrandChannelCommand = new RelayCommand(() => SetActiveThemeColorChannel(ThemeColorChannel.Brand));
        SelectThemeEditDayCommand = new RelayCommand(() => SetEditingNightThemeColors(false));
        SelectThemeEditNightCommand = new RelayCommand(() => SetEditingNightThemeColors(true));

        // U146：三处路径列表的 chip 投影。必须建在构造器末尾——
        // PathChipListViewModel 的构造会立刻 Sync 一次（读宿主字符串），
        // 建得太早读到的是尚未初始化的字段。
        ReadableRootChips = new PathChipListViewModel(
            _displayNames,
            () => ReadableRootsText,
            value => ReadableRootsText = value,
            requireAbsolute: true,
            probeExistence: true,
            assign => BrowseIntoAsync(assign));
        WritableRootChips = new PathChipListViewModel(
            _displayNames,
            () => WritableRootsText,
            value => WritableRootsText = value,
            requireAbsolute: true,
            probeExistence: true,
            assign => BrowseIntoAsync(assign));
        // 忽略路径是**项目内相对路径**（Git 忽略项），所以 requireAbsolute=false；
        // 体检要先拼上项目根才知道存不存在，故给 resolveForProbe。
        // 注意这里的选择器走 BrowseProjectDirectoryAsync——它会把绝对路径折回
        // 项目内相对路径，并在选到项目外时给出提示，与该字段的取值域一致。
        IgnoredPathChips = new PathChipListViewModel(
            _displayNames,
            () => IgnoredPathsText,
            value => IgnoredPathsText = value,
            requireAbsolute: false,
            probeExistence: true,
            assign => BrowseProjectDirectoryAsync(assign),
            relative => string.IsNullOrWhiteSpace(ProjectRoot)
                ? string.Empty
                : Path.Combine(ProjectRoot, relative));
    }

    /// <summary>U146：全局可读根的 chip 投影。</summary>
    public PathChipListViewModel ReadableRootChips { get; }

    /// <summary>U146：全局可写根的 chip 投影。</summary>
    public PathChipListViewModel WritableRootChips { get; }

    /// <summary>U146：Git 忽略路径的 chip 投影（相对路径）。</summary>
    public PathChipListViewModel IgnoredPathChips { get; }

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
    private Func<string?, Task<string?>>? _filePickerWithTitle;

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

    public void SetFilePicker(Func<string?, Task<string?>> picker) =>
        _filePickerWithTitle = picker;

    public void ClearFilePicker(Func<string?, Task<string?>> picker)
    {
        if (_filePickerWithTitle == picker)
        {
            _filePickerWithTitle = null;
        }
    }

    private async Task BrowseFileIntoAsync(Action<string> assign)
    {
        if (_filePickerWithTitle is null)
        {
            StatusText = _displayNames.Text("ui.settings.browse_unavailable");
            return;
        }
        try
        {
            var path = await _filePickerWithTitle(
                _displayNames.Text("ui.settings.browse_file_title")).ConfigureAwait(true);
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
    public string RecoveryText
    {
        get => _recoveryText;
        private set
        {
            if (SetProperty(ref _recoveryText, value))
            {
                OnPropertyChanged(nameof(HasRecoveryText));
            }
        }
    }
    public bool HasRecoveryText => !string.IsNullOrWhiteSpace(RecoveryText);
    public bool IsLoading { get => _isLoading; private set => SetProperty(ref _isLoading, value); }
    public bool HasUnsavedChanges
    {
        get => _hasUnsavedChanges;
        private set => SetProperty(ref _hasUnsavedChanges, value);
    }

    public ObservableCollection<SettingsTabViewModel> Tabs { get; }
    public ObservableCollection<SettingsSectionNavigationItemViewModel> SectionIndexItems { get; }

    /// <summary>只列当前页签的节锚点（悬浮索引不再罗列全部页签的小节，跟随当前页）。</summary>
    public IEnumerable<SettingsSectionNavigationItemViewModel> CurrentTabSectionIndexItems =>
        SectionIndexItems.Where(item => string.Equals(item.TabId, SelectedTab.Id, StringComparison.Ordinal));
    public event EventHandler<SettingsSectionNavigationRequest>? ScrollToSectionRequested;
    public event EventHandler<SettingsFieldFocusRequest>? FocusValidationFieldRequested;

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
                OnPropertyChanged(nameof(IsRetrievalSelected));
                OnPropertyChanged(nameof(IsVersionControlSelected));
                OnPropertyChanged(nameof(IsSupportSelected));
                OnPropertyChanged(nameof(NavigationSelection));
                OnPropertyChanged(nameof(CurrentTabSectionIndexItems));
                RestoreCurrentTabCommand.NotifyCanExecuteChanged();
                SaveCurrentTabCommand.NotifyCanExecuteChanged();
                OnPropertyChanged(nameof(SaveCurrentTabText));
                RestoreRecommendedDefaultsCommand.NotifyCanExecuteChanged();
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

        if (string.Equals(target.Tab.Id, PermissionsSection, StringComparison.Ordinal)
            && target.Section is not null
            && !string.Equals(target.Section.AnchorName, "CapabilitiesSectionAnchor", StringComparison.Ordinal))
        {
            AreAdvancedPermissionsExpanded = true;
        }

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
    public bool IsRetrievalSelected => SelectedTab.Id == "retrieval";
    public bool IsVersionControlSelected => SelectedTab.Id == "version_control";
    public bool IsSupportSelected => SelectedTab.Id == "support";
    public bool IsGeneralEditable => CanSave(GeneralSection);
    public bool IsModelsEditable => CanSave(ModelsSection);
    public bool IsPresetsEditable => CanSave(PresetsSection);
    public bool IsTemplateRepositoryEditable => CanSave(TemplateRepositorySection);
    public bool IsAutomationEditable => CanSave(AutomationSection);
    public bool IsPermissionsEditable => CanSave(PermissionsSection);
    public bool IsPersonalizationEditable => CanSave(PersonalizationSection);
    public bool IsAppRuntimeEditable => CanSave(AppRuntimeSection);
    public bool IsRetrievalEditable => CanSave(RetrievalSection);
    public bool IsGitEditable => CanSave(GitSection);
    public ObservableCollection<LanguageOption> LanguageOptions { get; }
    public ObservableCollection<SettingsValueOption> VectorBackendOptions { get; }
    public ObservableCollection<SettingsValueOption> QdrantAuthModeOptions { get; }
    public ObservableCollection<SettingsValueOption> ProviderTypeOptions { get; }
    public ObservableCollection<ThemeOption> ThemeOptions { get; }
    public ObservableCollection<ThemeGroupViewModel> ThemeGroups { get; }
    public ObservableCollection<ConfirmationPolicyViewModel> ConfirmationPolicies { get; }
    /// <summary>确认项按总结机制分组。</summary>
    public ObservableCollection<ConfirmationPolicyGroupViewModel> ConfirmationPolicyGroups { get; }
    public ObservableCollection<SettingsValueOption> ConfirmationProfileOptions { get; }
    public ObservableCollection<SettingsValueOption> ConfirmationNormalPolicyOptions { get; }
    public ObservableCollection<SettingsValueOption> ConfirmationAutoModePolicyOptions { get; }
    public ObservableCollection<SettingsValueOption> PermissionProfileOptions { get; }
    public ObservableCollection<NodeTypePresetViewModel> NodePresets { get; }
    public ObservableCollection<ModelAliasViewModel> ModelAliases { get; }
    public ObservableCollection<ProviderOptionViewModel> ProviderOptions { get; }
    public ObservableCollection<ModelOptionViewModel> AvailableModels { get; }
    public ObservableCollection<ProviderModelEditorRow> ProviderModels { get; }
    /// <summary>
    /// U145：模型 ID 候选，来自「刷新模型 / 测试连接」向服务商真实拉回的目录。
    ///
    /// 仍允许手打列表外的值：新发布的模型常常还没进 /models 接口，
    /// 而用户往往正是为了用它才来改这一行。
    /// </summary>
    public ObservableCollection<string> FetchedModelIdCandidates { get; }
    public ObservableCollection<SettingsValueOption> ProviderCapabilityOptions { get; }
    public ObservableCollection<ProviderModelRouteOption> DefaultLlmRouteOptions { get; }
    public ObservableCollection<ProviderModelRouteOption> DefaultEmbeddingRouteOptions { get; }
    public ObservableCollection<ProviderModelRouteOption> DefaultRerankerRouteOptions { get; }
    public ObservableCollection<ProviderModelRouteOption> DefaultSearchRouteOptions { get; }
    /// <summary>全局节点默认和节点类型预设使用 Provider/Model 成对身份。</summary>
    public ObservableCollection<WorkflowModelOption> AvailableLlmModelOptions { get; }
    public ObservableCollection<WorkflowModelOption> AvailableLlmModelTargetOptions { get; }
    public ObservableCollection<ToolControlGroupViewModel> ToolControlGroups { get; }
    public ObservableCollection<PermissionScopeProfileViewModel> ScopedPermissionProfiles { get; }
    public ObservableCollection<SettingsDiagnosticItemViewModel> DiagnosticsItems { get; }
    public ObservableCollection<SettingsSectionLoadFailureViewModel> SectionLoadFailures { get; }
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
    public RelayCommand TestProviderDraftCommand { get; }
    public RelayCommand SaveModelCommand { get; }
    public RelayCommand SaveProviderKeyCommand { get; }
    public RelayCommand RevokeProviderKeyCommand { get; }
    public RelayCommand RemoveProviderCommand { get; }
    public RelayCommand AddProviderCommand { get; }
    public RelayCommand AddProviderModelCommand { get; }
    public RelayCommand SavePresetsCommand { get; }
    public RelayCommand SaveTemplateRepositoryCommand { get; }
    public RelayCommand OpenTemplateMarketCommand { get; }
    public RelayCommand SaveAutomationCommand { get; }
    public RelayCommand SavePermissionsCommand { get; }
    public RelayCommand SavePersonalizationCommand { get; }
    public RelayCommand SaveAppRuntimeCommand { get; }
    public RelayCommand SaveRetrievalCommand { get; }
    public RelayCommand SaveGitCommand { get; }
    public RelayCommand RestoreCurrentTabCommand { get; }

    /// <summary>右下角悬浮保存钮：保存当前分页。</summary>
    public RelayCommand SaveCurrentTabCommand { get; }

    public string SaveCurrentTabText => _displayNames.Text("ui.settings.save_current_tab");
    public RelayCommand RestoreRecommendedDefaultsCommand { get; }
    public RelayCommand RestoreOfficialTemplateRepositoryCommand { get; }
    public RelayCommand RefreshDiagnosticsCommand { get; }
    public RelayCommand ShowTutorialCommand { get; }
    public RelayCommand BrowseDocumentsDirCommand { get; }
    public RelayCommand BrowseWorkflowsDirCommand { get; }
    public RelayCommand BrowseSkillsDirCommand { get; }
    public RelayCommand BrowseExportsDirCommand { get; }

    // U146：chip 化后 XAML 不再绑这两个命令（浏览按钮收进了 chip 列表模板，
    // 走 PathChipListViewModel.BrowseCommand，那条路径会对选到的目录做逐条校验）。
    // 保留是因为它们仍是「往路径列表追加一条」的编程入口，且现有用例在用；
    // 两者最终都落到同一个 AppendPathLine + 字符串 setter，不会与 chip 投影分叉。
    public RelayCommand BrowseReadableRootsCommand { get; }
    public RelayCommand BrowseWritableRootsCommand { get; }
    public RelayCommand BrowseQdrantBinaryCommand { get; }

    public string GeneralTitle => _displayNames.Text("ui.settings.general.title");
    public string GeneralScopeHelpText => _displayNames.Text("ui.settings.general.scope_help");
    public string ModelsTitle => _displayNames.Text("ui.settings.models.title");
    public string PresetsTitle => _displayNames.Text("ui.settings.presets.title");
    public string AutomationTitle => _displayNames.Text("ui.settings.automation.title");
    public string AutomationScopeHelpText => _displayNames.Text("ui.settings.automation.scope_help");
    public string PermissionsTitle => _displayNames.Text("ui.settings.permissions.title");
    public string PersonalizationTitle => _displayNames.Text("ui.settings.personalization.title");
    public string PersonalizationScopeHelpText => _displayNames.Text("ui.settings.personalization.scope_help");
    public string RetrievalTitle => _displayNames.Text("ui.settings.tab.retrieval");
    public string VersionControlTitle => _displayNames.Text("ui.settings.tab.version_control");
    public string SupportTitle => _displayNames.Text("ui.settings.tab.support");
    public string AdvancedSettingsText => _displayNames.Text("ui.settings.advanced");
    public string AppRuntimeScopeHelpText => _displayNames.Text("ui.settings.misc.app_runtime_scope_help");
    public string RetrievalScopeHelpText => _displayNames.Text("ui.settings.misc.retrieval_scope_help");

    public string ProjectNameLabel => _displayNames.Text("ui.settings.general.project_name");
    public string ProjectRootLabel => _displayNames.Text("ui.settings.general.project_root");
    public string DirectorySwitchWarningText => _displayNames.Text("ui.settings.general.directory_switch_warning");
    public string DocumentsDirLabel => _displayNames.Text("ui.settings.general.documents_dir");
    public string WorkflowsDirLabel => _displayNames.Text("ui.settings.general.workflows_dir");
    public string SkillsDirLabel => _displayNames.Text("ui.settings.general.skills_dir");
    public string ExportsDirLabel => _displayNames.Text("ui.settings.general.exports_dir");
    public string ProjectMemoryLabel => _displayNames.Text("ui.works.project_memory");
    public string ProjectMemoryPlaceholder => _displayNames.Text("ui.works.project_memory.placeholder");
    public string SaveGeneralText => _displayNames.Text("ui.settings.general.save");

    public string ProviderIdLabel => _displayNames.Text("ui.settings.models.provider_id");
    /// <summary>U135：服务 ID 改为只读展示后，「取值」由显式复制动作承接。</summary>
    public string CopyProviderIdText => _displayNames.Text("ui.settings.models.provider_id.copy");
    public string ProviderTypeLabel => _displayNames.Text("ui.settings.models.provider_type");
    public string ProviderDisplayNameLabel => _displayNames.Text("ui.settings.models.display_name");
    public string BaseUrlLabel => _displayNames.Text("ui.settings.models.base_url");
    public string BaseUrlPlaceholder => _displayNames.Text("ui.settings.models.base_url.placeholder");
    public string ProviderEnabledText => _displayNames.Text("ui.settings.models.enabled");
    public string MakeDefaultLlmText => _displayNames.Text("ui.settings.models.make_default_llm");
    public string MakeDefaultEmbeddingText => _displayNames.Text("ui.settings.models.make_default_embedding");
    public string MakeDefaultRerankerText => _displayNames.Text("ui.settings.models.make_default_reranker");
    public string MakeDefaultSearchText => _displayNames.Text("ui.settings.models.make_default_search");
    public string DefaultLlmRouteLabel => _displayNames.Text("ui.settings.models.default_llm_route");
    public string DefaultEmbeddingRouteLabel => _displayNames.Text("ui.settings.models.default_embedding_route");
    public string DefaultRerankerRouteLabel => _displayNames.Text("ui.settings.models.default_reranker_route");
    public string DefaultSearchRouteLabel => _displayNames.Text("ui.settings.models.default_search_route");
    public string AvailableModelsText => _displayNames.Text("ui.settings.models.available_models");
    public string ManualModelsText => _displayNames.Text("ui.settings.models.manual_models");
    public string ModelsTextLabel => _displayNames.Text("ui.settings.models.models");
    public string ModelsPlaceholder => _displayNames.Text("ui.settings.models.models.placeholder");
    public string ModelIdColumnLabel => _displayNames.Text("ui.settings.models.column.id");
    public string ModelCapabilityColumnLabel => _displayNames.Text("ui.settings.models.column.capability");
    public string ModelContextColumnLabel => _displayNames.Text("ui.settings.models.column.context");
    public string ModelInputCostColumnLabel => _displayNames.Text("ui.settings.models.column.input_cost");
    public string ModelOutputCostColumnLabel => _displayNames.Text("ui.settings.models.column.output_cost");
    public string AddModelText => _displayNames.Text("ui.settings.models.add_model");
    public string RemoveModelText => _displayNames.Text("ui.settings.models.remove_model");
    public string EmbeddingModelLabel => _displayNames.Text("ui.settings.models.embedding_model");
    public string EmbeddingModelPlaceholder => _displayNames.Text("ui.settings.models.embedding_model.placeholder");
    public string ApiKeyLabel => _displayNames.Text("ui.settings.models.api_key");
    public string ApiKeyPlaceholder => _displayNames.Text("ui.settings.models.api_key.placeholder");
    public string SaveModelText => _displayNames.Text("ui.settings.models.save");
    public string SaveKeyText => _displayNames.Text("ui.settings.models.save_key");
    public string RevokeKeyText => _displayNames.Text("ui.settings.models.revoke_key");
    public string RemoveProviderText => _displayNames.Text("ui.settings.models.remove");
    public string RefreshText => _displayNames.Text("ui.common.refresh");
    public string TestProviderDraftText => _displayNames.Text("ui.settings.models.test_connection");
    public string LegacyOtherProviderMessage => _displayNames.Text("ui.settings.models.provider_type.other.migration");
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
    public string ModelAliasesTitle => _displayNames.Text("ui.settings.presets.model_aliases");
    public string ModelAliasTargetLabel => _displayNames.Text("ui.settings.presets.model_alias_target");
    public string ModelAliasesHelpText => _displayNames.Text("ui.settings.presets.model_aliases_help");
    public string InheritNodePermissionsText => _displayNames.Text("ui.settings.presets.inherit_node_permissions");
    public string DefaultModelLabel => _displayNames.Text("ui.settings.presets.default_model");
    public string DefaultTimeoutLabel => _displayNames.Text("ui.settings.presets.default_timeout_ms");
    public string DefaultBudgetLabel => _displayNames.Text("ui.settings.presets.default_budget_usd");
    public string TemplateRepositoryLabel => _displayNames.Text("ui.settings.presets.template_repository");
    public string TemplateRepositorySourceLabel => _displayNames.Text("ui.settings.presets.template_repository_source");
    public string TemplateRepositorySourceText => _displayNames.Text(
        string.Equals(TemplateRepositoryBaseUrl, OfficialTemplateRepositoryUrl, StringComparison.Ordinal)
            ? "ui.settings.presets.template_repository_source.official"
            : "ui.settings.presets.template_repository_source.custom");
    public string AdvancedTemplateRepositoryText => _displayNames.Text("ui.settings.presets.template_repository_advanced");
    public string RestoreOfficialTemplateRepositoryText => _displayNames.Text("ui.settings.presets.template_repository_restore_official");
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
    public string ConfirmationProfileLabel => _displayNames.Text("ui.settings.automation.confirmation.profile");
    public string ConfirmationProfileHelpText => _displayNames.Text("ui.settings.automation.confirmation.profile.help");
    public string AdvancedConfirmationPoliciesText => _displayNames.Text("ui.settings.automation.confirmation.advanced");
    private string DefaultAutoApprovalPrompt => _displayNames.Text("ui.settings.automation.confirmation.default_approval_prompt");
    public string BrowseFolderText => _displayNames.Text("ui.settings.browse_folder");
    public string BrowseFileText => _displayNames.Text("ui.settings.browse_file");
    public string WorkflowLimitLabel => _displayNames.Text("ui.settings.automation.workflow");
    public string WorkflowDefaultTimeoutLabel => _displayNames.Text("ui.settings.automation.default_timeout_ms");
    public string MaxLoopIterationsLabel => _displayNames.Text("ui.settings.automation.max_loop_iterations");
    public string MaxToolRoundsLabel => _displayNames.Text("ui.settings.automation.max_tool_rounds");
    public string CheckpointEnabledLabel => _displayNames.Text("ui.settings.automation.checkpoint_enabled");
    public string RunEventRetentionLabel => _displayNames.Text("ui.settings.automation.run_event_retention_days");
    public string SaveAutomationText => _displayNames.Text("ui.settings.automation.save");

    public string AllowNetworkText => _displayNames.Text("ui.settings.permissions.allow_network");
    public string AllowWebSearchText => _displayNames.Text("ui.settings.permissions.allow_web_search");
    public string AllowHttpSkillText => _displayNames.Text("ui.settings.permissions.allow_http_skill");
    public string AllowWasmNetworkText => _displayNames.Text("ui.settings.permissions.allow_wasm_network");
    public string AllowSecretReadText => _displayNames.Text("ui.settings.permissions.allow_secret_read");
    public string PermissionProfileLabel => _displayNames.Text("ui.settings.permissions.profile");
    public string PermissionProfileHelpText => _displayNames.Text("ui.settings.permissions.profile.help");
    public string AdvancedPermissionsText => _displayNames.Text("ui.settings.permissions.advanced");
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
    public string QdrantTlsText => _displayNames.Text("ui.settings.misc.qdrant_tls");
    public string QdrantAuthModeLabel => _displayNames.Text("ui.settings.misc.qdrant_auth");
    public string QdrantApiKeyLabel => _displayNames.Text("ui.settings.misc.qdrant_api_key");
    public string QdrantApiKeyPlaceholder => _displayNames.Text("ui.settings.misc.qdrant_api_key.placeholder");
    public string QdrantApiKeyStatusText => _displayNames.Text(
        HasQdrantApiKeyForCurrentEndpoint() ? "ui.common.configured" : "ui.common.not_configured");
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
    public string SaveRetrievalText => _displayNames.Text("ui.settings.retrieval.save");
    public string SaveGitText => _displayNames.Text("ui.settings.git.save");
    public string LanguageLabel => _displayNames.Text("ui.settings.misc.language");
    public string TutorialText => _displayNames.Text("ui.settings.index.tutorial");
    public string OpenTutorialText => _displayNames.Text("ui.settings.misc.open_tutorial");
    public string DiagnosticsLabel => _displayNames.Text("ui.settings.misc.diagnostics");
    public string DiagnosticsStatusText => _displayNames.Format("ui.settings.misc.diagnostics.status", new Dictionary<string, string>
    {
        ["status"] = DiagnosticStatusLabel(DiagnosticsStatus),
    });
    public string DiagnosticsEmptyText => _displayNames.Text("ui.settings.misc.diagnostics.no_items");
    public string RefreshDiagnosticsText => _displayNames.Text("ui.settings.misc.diagnostics.refresh");
    public string CopyDiagnosticsText => _displayNames.Text("ui.settings.misc.diagnostics.copy");
    public string RestoreCurrentTabText => _displayNames.Text("ui.settings.restore_current_tab");
    public string RestoreRecommendedDefaultsText => _displayNames.Text("ui.settings.restore_recommended_defaults");
    public bool HasDiagnosticsItems => DiagnosticsItems.Count > 0;
    public bool HasSectionLoadFailures => SectionLoadFailures.Count > 0;
    public string SectionLoadFailureRetryText => _displayNames.Text("ui.settings.retry");
    public bool HasCompatibilityPermissionScopes => _compatibilityScopedPolicies.Count > 0;
    public string CompatibilityPermissionScopesText => _displayNames.Format(
        "ui.settings.permissions.compatibility_scopes",
        new Dictionary<string, string>
        {
            ["count"] = _compatibilityScopedPolicies.Count.ToString(CultureInfo.InvariantCulture),
        });
    public bool IsDiagnosticsRefreshing
    {
        get => _isDiagnosticsRefreshing;
        private set
        {
            if (SetProperty(ref _isDiagnosticsRefreshing, value))
            {
                RefreshDiagnosticsCommand.NotifyCanExecuteChanged();
            }
        }
    }
    public string DiagnosticsCopyText => string.Join(
        Environment.NewLine,
        DiagnosticsItems.Select(item => $"{item.Component}: {item.Status} - {item.Reason} - {item.RecoveryAction}"));

    public string ProjectName { get => _projectName; set => SetProperty(ref _projectName, value); }
    public string ProjectRoot
    {
        get => _projectRoot;
        private set => SetProperty(ref _projectRoot, value ?? string.Empty);
    }
    public string Locale { get => _locale; set => SetProperty(ref _locale, value); }
    public string DocumentsDir { get => _documentsDir; set => SetProperty(ref _documentsDir, value); }
    public string WorkflowsDir { get => _workflowsDir; set => SetProperty(ref _workflowsDir, value); }
    public string SkillsDir { get => _skillsDir; set => SetProperty(ref _skillsDir, value); }
    public string ExportsDir { get => _exportsDir; set => SetProperty(ref _exportsDir, value); }
    public string ProjectMemory { get => _projectMemory; set => SetProperty(ref _projectMemory, value); }

    public string ProviderId { get => _providerId; set => SetProperty(ref _providerId, value); }
    public string ProviderType
    {
        get => _providerType;
        set
        {
            if (SetProperty(ref _providerType, value))
            {
                EnsureLegacyOtherProviderTypeOption(value);
                OnPropertyChanged(nameof(IsLegacyOtherProvider));
                OnPropertyChanged(nameof(IsProviderEditorEditable));
                NotifyProviderCommands();
                NotifySaveCommands();
            }
        }
    }
    public string ProviderDisplayName { get => _providerDisplayName; set => SetProperty(ref _providerDisplayName, value); }
    public string ProviderBaseUrl { get => _providerBaseUrl; set => SetProperty(ref _providerBaseUrl, value); }
    public bool ProviderEnabled
    {
        get => _providerEnabled;
        set
        {
            if (SetProperty(ref _providerEnabled, value) && !value)
            {
                ClearDefaultRoutesForProvider(ProviderId);
            }
            RebuildProviderDefaultModelRoutes();
        }
    }
    public bool MakeDefaultLlm { get => _makeDefaultLlm; set => SetProperty(ref _makeDefaultLlm, value); }
    public bool MakeDefaultEmbedding { get => _makeDefaultEmbedding; set => SetProperty(ref _makeDefaultEmbedding, value); }
    public bool MakeDefaultReranker { get => _makeDefaultReranker; set => SetProperty(ref _makeDefaultReranker, value); }
    public bool MakeDefaultSearch { get => _makeDefaultSearch; set => SetProperty(ref _makeDefaultSearch, value); }
    public string ApiKey { get => _apiKey; set => SetProperty(ref _apiKey, value); }
    public string ModelsText { get => _modelsText; set => SetProperty(ref _modelsText, value); }
    public string EmbeddingModelId { get => _embeddingModelId; set => SetProperty(ref _embeddingModelId, value); }
    public bool IsLegacyOtherProvider => string.Equals(ProviderType, "other", StringComparison.Ordinal);
    public bool IsProviderEditorEditable => IsModelsEditable && !IsLegacyOtherProvider;
    public bool ManualModelsVisible { get => _manualModelsVisible; set => SetProperty(ref _manualModelsVisible, value); }
    public ProviderModelRouteOption? SelectedDefaultLlmRoute
    {
        get => _selectedDefaultLlmRoute;
        set => SetProperty(ref _selectedDefaultLlmRoute, value);
    }
    public ProviderModelRouteOption? SelectedDefaultEmbeddingRoute
    {
        get => _selectedDefaultEmbeddingRoute;
        set => SetProperty(ref _selectedDefaultEmbeddingRoute, value);
    }
    public ProviderModelRouteOption? SelectedDefaultRerankerRoute
    {
        get => _selectedDefaultRerankerRoute;
        set => SetProperty(ref _selectedDefaultRerankerRoute, value);
    }
    public ProviderModelRouteOption? SelectedDefaultSearchRoute
    {
        get => _selectedDefaultSearchRoute;
        set => SetProperty(ref _selectedDefaultSearchRoute, value);
    }
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
    public string? DefaultModelAlias => _defaultModelAlias;
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

            ApplyDefaultModelIdentity(value.AliasId, value.ProviderId, value.ModelId);
        }
    }
    public string DefaultTimeoutMs { get => _defaultTimeoutMs; set => SetProperty(ref _defaultTimeoutMs, value); }
    public string DefaultBudgetUsd { get => _defaultBudgetUsd; set => SetProperty(ref _defaultBudgetUsd, value); }
    public string TemplateRepositoryBaseUrl
    {
        get => _templateRepositoryBaseUrl;
        set
        {
            if (SetProperty(ref _templateRepositoryBaseUrl, value))
            {
                OnPropertyChanged(nameof(TemplateRepositorySourceText));
            }
        }
    }

    public string BudgetUsd { get => _budgetUsd; set => SetProperty(ref _budgetUsd, value); }
    public string PreauthorizedUsd { get => _preauthorizedUsd; set => SetProperty(ref _preauthorizedUsd, value); }
    public string SpentText { get => _spentText; set => SetProperty(ref _spentText, value); }
    public SettingsValueOption? SelectedConfirmationProfile
    {
        get => _selectedConfirmationProfile;
        set
        {
            if (SetProperty(ref _selectedConfirmationProfile, value))
            {
                ApplyConfirmationProfile(value?.Value);
            }
        }
    }
    public string WorkflowDefaultTimeoutMs { get => _workflowDefaultTimeoutMs; set => SetProperty(ref _workflowDefaultTimeoutMs, value); }
    public string MaxLoopIterations { get => _maxLoopIterations; set => SetProperty(ref _maxLoopIterations, value); }
    public string MaxToolRounds { get => _maxToolRounds; set => SetProperty(ref _maxToolRounds, value); }
    public bool CheckpointEnabled { get => _checkpointEnabled; set => SetProperty(ref _checkpointEnabled, value); }
    public string RunEventRetentionDays { get => _runEventRetentionDays; set => SetProperty(ref _runEventRetentionDays, value); }

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
            RefreshToolControlPolicyGates();
        }
    }
    public bool AllowWebSearch
    {
        get => _allowWebSearch;
        set
        {
            SetProperty(ref _allowWebSearch, value);
            RefreshToolControlPolicyGates();
        }
    }
    public bool AllowHttpSkill { get => _allowHttpSkill; set => SetProperty(ref _allowHttpSkill, value); }
    public bool AllowWasmNetwork { get => _allowWasmNetwork; set => SetProperty(ref _allowWasmNetwork, value); }
    public bool AllowSecretRead { get => _allowSecretRead; set => SetProperty(ref _allowSecretRead, value); }
    public SettingsValueOption? SelectedPermissionProfile
    {
        get => _selectedPermissionProfile;
        set
        {
            if (SetProperty(ref _selectedPermissionProfile, value) && !_applyingPermissionProfile)
            {
                ApplyPermissionProfile(value?.Value);
            }
        }
    }
    public bool AreAdvancedPermissionsExpanded
    {
        get => _areAdvancedPermissionsExpanded;
        set => SetProperty(ref _areAdvancedPermissionsExpanded, value);
    }
    public bool AreAdvancedConfirmationPoliciesExpanded
    {
        get => _areAdvancedConfirmationPoliciesExpanded;
        set => SetProperty(ref _areAdvancedConfirmationPoliciesExpanded, value);
    }
    public bool AreAdvancedRetrievalSettingsExpanded
    {
        get => _areAdvancedRetrievalSettingsExpanded;
        set => SetProperty(ref _areAdvancedRetrievalSettingsExpanded, value);
    }
    public bool AreAdvancedAppRuntimeSettingsExpanded
    {
        get => _areAdvancedAppRuntimeSettingsExpanded;
        set => SetProperty(ref _areAdvancedAppRuntimeSettingsExpanded, value);
    }
    // U146：全局可读/可写根同样 chip 化。字符串仍是真源（脏状态、快照、
    // MatchesPermissionProfile、ApplyRecommendedDefaults 全部依赖它），
    // chip 增删经由 setter 落地，故 OnPropertyChanged → UpdateDirtyState 链路不变。
    public string ReadableRootsText
    {
        get => _readableRootsText;
        set
        {
            if (SetProperty(ref _readableRootsText, value))
            {
                ReadableRootChips.Sync();
            }
        }
    }

    public string WritableRootsText
    {
        get => _writableRootsText;
        set
        {
            if (SetProperty(ref _writableRootsText, value))
            {
                WritableRootChips.Sync();
            }
        }
    }

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

    public bool VectorEnabled
    {
        get => _vectorEnabled;
        set
        {
            if (SetProperty(ref _vectorEnabled, value))
            {
                OnPropertyChanged(nameof(IsVectorConfigurationVisible));
            }
        }
    }
    public string VectorBackend
    {
        get => _vectorBackend;
        set
        {
            if (SetProperty(ref _vectorBackend, value))
            {
                OnPropertyChanged(nameof(IsQdrantSidecarBackend));
                OnPropertyChanged(nameof(IsExternalQdrantBackend));
                OnPropertyChanged(nameof(IsQdrantApiKeyAuth));
            }
        }
    }
    public bool IsQdrantSidecarBackend => string.Equals(VectorBackend, "qdrant_sidecar", StringComparison.Ordinal);
    public bool IsExternalQdrantBackend => string.Equals(VectorBackend, "external_qdrant", StringComparison.Ordinal);
    public bool IsVectorConfigurationVisible => VectorEnabled;
    public string VectorCollection { get => _vectorCollection; set => SetProperty(ref _vectorCollection, value); }
    public string VectorDimensions { get => _vectorDimensions; set => SetProperty(ref _vectorDimensions, value); }
    public string QdrantHost
    {
        get => _qdrantHost;
        set { if (SetProperty(ref _qdrantHost, value)) OnPropertyChanged(nameof(QdrantApiKeyStatusText)); }
    }
    public string QdrantPort
    {
        get => _qdrantPort;
        set { if (SetProperty(ref _qdrantPort, value)) OnPropertyChanged(nameof(QdrantApiKeyStatusText)); }
    }
    public bool QdrantUseTls
    {
        get => _qdrantUseTls;
        set { if (SetProperty(ref _qdrantUseTls, value)) OnPropertyChanged(nameof(QdrantApiKeyStatusText)); }
    }
    public string QdrantAuthMode
    {
        get => _qdrantAuthMode;
        set
        {
            if (SetProperty(ref _qdrantAuthMode, value))
            {
                OnPropertyChanged(nameof(IsQdrantApiKeyAuth));
                OnPropertyChanged(nameof(QdrantApiKeyStatusText));
            }
        }
    }
    public bool IsQdrantApiKeyAuth => IsExternalQdrantBackend
        && string.Equals(QdrantAuthMode, "api_key", StringComparison.Ordinal);
    public string QdrantApiKey
    {
        get => _qdrantApiKey;
        set
        {
            if (SetProperty(ref _qdrantApiKey, value) && _hasQdrantApiKeyError)
            {
                HasQdrantApiKeyError = false;
            }
        }
    }
    public string QdrantApiKeyErrorText => HasQdrantApiKeyError
        ? ValidationMessage("ui.settings.validation.required", QdrantApiKeyLabel)
        : string.Empty;
    public bool HasQdrantApiKeyError
    {
        get => _hasQdrantApiKeyError;
        private set
        {
            if (SetProperty(ref _hasQdrantApiKeyError, value))
            {
                OnPropertyChanged(nameof(QdrantApiKeyErrorText));
            }
        }
    }
    public bool HasQdrantApiKey
    {
        get => _hasQdrantApiKey;
        private set
        {
            if (SetProperty(ref _hasQdrantApiKey, value))
            {
                OnPropertyChanged(nameof(QdrantApiKeyStatusText));
            }
        }
    }

    private bool HasQdrantApiKeyForCurrentEndpoint()
    {
        if (!HasQdrantApiKey)
        {
            return false;
        }
        return SavedValueMatches(nameof(QdrantHost), QdrantHost)
            && SavedValueMatches(nameof(QdrantPort), QdrantPort)
            && SavedValueMatches(nameof(QdrantUseTls), QdrantUseTls.ToString());
    }

    private bool SavedValueMatches(string field, string current) =>
        !_draftState.TryGetSavedValue(RetrievalSection, field, out var saved)
        || string.Equals(saved, current, StringComparison.Ordinal);
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
    public string IgnoredPathsText
    {
        get => _ignoredPathsText;
        set
        {
            if (SetProperty(ref _ignoredPathsText, value))
            {
                IgnoredPathChips.Sync();
            }
        }
    }
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
        _failedSectionRetries.Clear();
        SectionLoadFailures.Clear();
        OnPropertyChanged(nameof(HasSectionLoadFailures));
        var failed = false;
        IsLoading = true;
        NotifySectionStateChanged();
        try
        {
            // 整页加载：所有 section 的读取一次性发出，让 IPC 往返重叠。
            // 后端是 8 worker 线程池，前端按 request_id 路由响应，两侧都支持并发在途请求；
            // 此前逐个 await 把 14 次往返排成一条队，实测空项目就要 186–240ms。
            // 顺序仍与原实现一致（apply 续体在 UI 线程串行执行），
            // 只是不再让第 N 次往返等第 N-1 次回来。
            //
            // 各 section 的装配只写在 LoadSingleSectionAsync 一处：整页加载与
            // 「取消后只重载脏 section」共用它，避免两处清单漂移——
            // 漂移的后果是取消后拿到过期值，且不会有任何报错。
            var sectionTasks = AllLoadableSections
                .Select(section => BeginLoadSection(generation, section, cancellationToken))
                .ToList();

            // 项目身份不属于任何 section，单独发；它失败不能拖垮整页加载。
            var currentProjectTask = _backend.GetCurrentProjectAsync(cancellationToken);
            var diagnosticsTask = RefreshDiagnosticsAsync(generation, cancellationToken);

            try
            {
                var currentProject = await currentProjectTask.ConfigureAwait(true);
                cancellationToken.ThrowIfCancellationRequested();
                if (_draftState.IsCurrentLoad(generation))
                {
                    ProjectRoot = currentProject?.ProjectRoot ?? string.Empty;
                }
            }
            catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
            {
                throw;
            }
            catch
            {
                // 项目身份只服务目录浏览边界，失败不能拖垮项目配置和项目记忆的加载事务。
                ProjectRoot = string.Empty;
            }

            foreach (var apply in sectionTasks)
            {
                failed |= !await apply().ConfigureAwait(true);
            }
            failed |= !await diagnosticsTask.ConfigureAwait(true);

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

    /// <summary>
    /// 「取消/放弃改动」的回退路径：只重载**真正脏了的** section。
    ///
    /// 此前这里直接调 <see cref="LoadAsync"/>，那会重新拉全部 14 个 section
    /// （实测空项目 debug 构建就要 186–240ms，真实项目更慢），而用户只是点了
    /// 「不保存」——绝大多数情况下只有当前这一页脏。
    ///
    /// 为什么不做「纯本地回滚」：<see cref="SettingsDraftState"/> 虽然按 section
    /// 存了 baseline，但要把 ~40 个字段逐个映射回属性，漏一个就静默留下脏值，
    /// 而「取消之后还留着改动」比慢更糟。重载脏 section 既拿到权威值，
    /// 又把代价压到 1–2 次往返。
    /// </summary>
    private async Task ReloadDirtySectionsAsync(CancellationToken cancellationToken = default)
    {
        var dirty = DirtySections();
        if (dirty.Count == 0)
        {
            return;
        }

        IsLoading = true;
        NotifySectionStateChanged();
        try
        {
            // 复用当前 generation：这不是一次新的整页加载，未涉及的 section
            // 必须保留自己的 baseline。走 BeginLoad() 会把它们一并清空，
            // 之后 TryBeginSave 因 IsLoaded=false 直接拒绝保存。
            var generation = _draftState.CurrentLoadGeneration;
            await LoadSectionsAsync(generation, dirty, cancellationToken).ConfigureAwait(true);
        }
        finally
        {
            _suppressDirtyTracking = false;
            IsLoading = false;
            NotifySectionStateChanged();
            UpdateDirtyState(updateStatus: false);
        }
    }

    /// <summary>
    /// 全部可加载 section，也是整页加载的顺序。
    ///
    /// 注意不含 <c>PresetsSection</c>：它与 permissions 由同一个后端读取一并产出
    /// （见 <see cref="LoadSingleSectionAsync"/> 的合并分支），单列会多打一次同样的往返。
    /// </summary>
    private static readonly string[] AllLoadableSections =
    {
        GeneralSection, ModelsSection, PermissionsSection, TemplateRepositorySection,
        AutomationSection, PersonalizationSection, AppRuntimeSection,
        RetrievalSection, GitSection,
    };

    /// <summary>
    /// 当前有未保存改动的 section 清单。
    ///
    /// 从 <see cref="AllLoadableSections"/> 派生并额外并入 presets——
    /// presets 脏时要走 permissions 那条合并读取，故映射到 permissions。
    /// </summary>
    private List<string> DirtySections()
    {
        var dirty = AllLoadableSections
            .Where(section => _draftState.IsSectionDirty(section, CurrentSectionValues(section)))
            .ToList();
        if (!dirty.Contains(PermissionsSection)
            && _draftState.IsSectionDirty(PresetsSection, CurrentSectionValues(PresetsSection)))
        {
            dirty.Add(PermissionsSection);
        }
        return dirty;
    }

    /// <summary>
    /// 按名字重载指定 section。
    ///
    /// **读并发、写串行**：每个 section 的 read 是独立的 IPC 往返，可以重叠；
    /// 但 apply 改的是共享 ViewModel 状态（还要嵌套翻 <c>_suppressDirtyTracking</c>），
    /// 并发 apply 会互相踩。所以先把读取一次性发出去让往返重叠，
    /// 再按**原顺序**逐个 await——await 的续体回到 UI 线程，天然串行执行 apply。
    ///
    /// 后端 IPC 是 8 worker 线程池（<c>MAX_CONCURRENT_IPC_REQUESTS</c>），
    /// 客户端按 request_id 路由响应，两侧都支持并发在途请求。
    /// 实测同样 8 个读：串行 165.8ms → 并发 63.5ms。
    ///
    /// 注意 <see cref="LoadSingleSectionAsync"/> 返回的 Task 内部是「读完立刻 apply」，
    /// 因此这里的并发严格来说是「读并发 + apply 顺序不保证」。之所以仍然安全：
    /// 所有续体都由 UI 线程的同一个调度队列执行，不会真正并行；
    /// 而各 section 的 apply 写的是互不相交的属性集合，顺序无关。
    /// </summary>
    private async Task<bool> LoadSectionsAsync(
        long generation,
        IReadOnlyList<string> sections,
        CancellationToken cancellationToken)
    {
        // 读取先全部发出（BeginLoadSection 内部已把请求打出去），让 IPC 往返重叠；
        // 返回的续作只负责「等结果 + apply」，由下面按顺序逐个执行，绝不并行。
        var deferred = sections
            .Select(section => BeginLoadSection(generation, section, cancellationToken))
            .ToList();

        var failed = false;
        foreach (var apply in deferred)
        {
            failed |= !await apply().ConfigureAwait(true);
        }
        return !failed;
    }

    /// <summary>
    /// 读取指定 section（并发发起），返回一个「把结果 apply 到 ViewModel」的续作。
    ///
    /// **读并发、apply 串行**是本页并发化的硬约束：
    /// <see cref="LoadSectionAsync"/> 里的 apply 会保存/恢复共享的
    /// <c>_suppressDirtyTracking</c>，两个 apply 若交错执行，后一个的 finally
    /// 会把标志恢复成前一个的中间值，脏标记随之错乱。
    ///
    /// 因此这里把「读」和「应用」拆开：读全部先发出去让 IPC 往返重叠，
    /// 应用则由调用方按固定顺序逐个 await，绝不并行。**不能**依赖
    /// 「续体都在 UI 线程所以自然串行」——headless 测试宿主没有
    /// <c>SynchronizationContext</c>，续体会落到线程池上真正并行。
    /// </summary>
    private Func<Task<bool>> BeginLoadSection(
        long generation,
        string section,
        CancellationToken cancellationToken)
    {
        return section switch
        {
            GeneralSection => Deferred(
                (
                    _backend.GetAppSettingsAsync(cancellationToken),
                    _backend.ReadProjectMemoryAsync(cancellationToken)),
                GeneralSection,
                static async pair => (
                    await pair.Item1.ConfigureAwait(true),
                    await pair.Item2.ConfigureAwait(true)),
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
                generation,
                cancellationToken),
            ModelsSection => Deferred(
                _backend.GetProviderConfigAsync(cancellationToken),
                ModelsSection,
                static task => task,
                value =>
                {
                    _providerConfig = value;
                    RebuildProviderOptionsFromConfig(preferProviderId: ProviderId);
                },
                generation,
                cancellationToken),
            // permissions 与 presets 由同一个装配函数产出（共用一次后端读取），
            // 因此任一脏都走它，不拆开——拆开会多打一次同样的往返。
            // 它内部自带读取+应用，无法拆分，故整体延后执行（不参与读并发）。
            PermissionsSection or PresetsSection =>
                () => LoadPermissionPresetSectionsAsync(generation, cancellationToken),
            TemplateRepositorySection => Deferred(
                _backend.GetTemplateRepositorySettingsAsync(cancellationToken),
                TemplateRepositorySection,
                static task => task,
                value => TemplateRepositoryBaseUrl = value.BaseUrl,
                generation,
                cancellationToken),
            AutomationSection => Deferred(
                (
                    _backend.GetAutomationSettingsAsync(cancellationToken),
                    _backend.GetWorkflowSettingsAsync(cancellationToken)),
                AutomationSection,
                static async pair => (
                    await pair.Item1.ConfigureAwait(true),
                    await pair.Item2.ConfigureAwait(true)),
                value =>
                {
                    ApplyAutomation(value.Item1);
                    _workflowSchemaVersion = value.Item2.Workflow.SchemaVersion;
                    WorkflowDefaultTimeoutMs = SecondsFromStoredMs(value.Item2.Workflow.DefaultTimeoutMs);
                    MaxLoopIterations = value.Item2.Workflow.MaxLoopIterations.ToString();
                    MaxToolRounds = value.Item2.Workflow.MaxToolRounds.ToString();
                    CheckpointEnabled = value.Item2.Workflow.CheckpointEnabled;
                    RunEventRetentionDays = value.Item2.Workflow.RunEventRetentionDays.ToString(CultureInfo.InvariantCulture);
                },
                generation,
                cancellationToken),
            PersonalizationSection => Deferred(
                _backend.GetUiPreferencesAsync(cancellationToken),
                PersonalizationSection,
                static task => task,
                ApplyLoadedUiPreferences,
                generation,
                cancellationToken),
            AppRuntimeSection => Deferred(
                _backend.GetAppRuntimeSettingsAsync(cancellationToken),
                AppRuntimeSection,
                static task => task,
                ApplyAppRuntime,
                generation,
                cancellationToken),
            RetrievalSection => Deferred(
                _backend.GetRagSettingsAsync(cancellationToken),
                RetrievalSection,
                static task => task,
                ApplyRag,
                generation,
                cancellationToken),
            GitSection => Deferred(
                _backend.GetGitSettingsAsync(cancellationToken),
                GitSection,
                static task => task,
                ApplyGit,
                generation,
                cancellationToken),
            // 未知 section 名视为「没这一页要重载」，不是错误——
            // 但也不能谎报成功，否则调用方会以为已回退。
            _ => () => Task.FromResult(false),
        };

        // 把「已发出的读取」包成延后应用的续作：读取此刻已在飞行中，
        // apply 要等调用方按顺序 await 时才执行。
        Func<Task<bool>> Deferred<TRaw, TValue>(
            TRaw inflight,
            string sectionName,
            Func<TRaw, Task<TValue>> await_,
            Action<TValue> apply,
            long gen,
            CancellationToken token)
            => () => LoadSectionAsync(gen, sectionName, () => await_(inflight), apply, token);
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

            var accepted = _draftState.AcceptLoaded(generation, section, CurrentSectionValues(section));
            if (accepted)
            {
                ClearSectionLoadFailure(section);
                if (string.Equals(section, RetrievalSection, StringComparison.Ordinal))
                {
                    OnPropertyChanged(nameof(QdrantApiKeyStatusText));
                }
            }
            return accepted;
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            throw;
        }
        catch
        {
            if (_draftState.IsCurrentLoad(generation))
            {
                RegisterSectionLoadFailure(
                    section,
                    () => LoadSectionAsync(
                        _draftState.CurrentLoadGeneration,
                        section,
                        read,
                        apply,
                        CancellationToken.None));
            }
            return false;
        }
        finally
        {
            NotifySectionStateChanged();
        }
    }

    private void RegisterSectionLoadFailure(string section, Func<Task<bool>> retry)
    {
        _failedSectionRetries[section] = retry;
        for (var index = SectionLoadFailures.Count - 1; index >= 0; index--)
        {
            if (string.Equals(SectionLoadFailures[index].Section, section, StringComparison.Ordinal))
            {
                SectionLoadFailures.RemoveAt(index);
            }
        }

        SectionLoadFailures.Add(new SettingsSectionLoadFailureViewModel(
            section,
            SectionTitle(section),
            _displayNames.Format("ui.settings.load_failure", new Dictionary<string, string>
            {
                ["section"] = SectionTitle(section),
            }),
            SectionLoadFailureRetryText,
            () => _ = RetryFailedSectionAsync(section)));
        OnPropertyChanged(nameof(HasSectionLoadFailures));
    }

    private void ClearSectionLoadFailure(string section)
    {
        _failedSectionRetries.Remove(section);
        for (var index = SectionLoadFailures.Count - 1; index >= 0; index--)
        {
            if (string.Equals(SectionLoadFailures[index].Section, section, StringComparison.Ordinal))
            {
                SectionLoadFailures.RemoveAt(index);
            }
        }
        OnPropertyChanged(nameof(HasSectionLoadFailures));
    }

    private async Task RetryFailedSectionAsync(string section)
    {
        if (!_failedSectionRetries.TryGetValue(section, out var retry))
        {
            return;
        }

        await retry().ConfigureAwait(true);
        NotifySectionStateChanged();
        UpdateDirtyState(updateStatus: false);
    }

    private bool CanRestoreSelectedTab()
    {
        return SelectedTab.Id switch
        {
            "general" => _draftState.IsSectionDirty(GeneralSection, CurrentSectionValues(GeneralSection)),
            "models" => _draftState.IsSectionDirty(ModelsSection, CurrentSectionValues(ModelsSection)),
            "presets" => _draftState.IsSectionDirty(PresetsSection, CurrentSectionValues(PresetsSection))
                         || _draftState.IsSectionDirty(TemplateRepositorySection, CurrentSectionValues(TemplateRepositorySection)),
            "automation" => _draftState.IsSectionDirty(AutomationSection, CurrentSectionValues(AutomationSection)),
            "permissions" => _draftState.IsSectionDirty(PermissionsSection, CurrentSectionValues(PermissionsSection)),
            "personalization" => _draftState.IsSectionDirty(PersonalizationSection, CurrentSectionValues(PersonalizationSection)),
            "retrieval" => _draftState.IsSectionDirty(AppRuntimeSection, CurrentSectionValues(AppRuntimeSection))
                           || _draftState.IsSectionDirty(RetrievalSection, CurrentSectionValues(RetrievalSection)),
            "version_control" => _draftState.IsSectionDirty(GitSection, CurrentSectionValues(GitSection)),
            _ => false,
        };
    }

    /// <summary>
    /// 右下角悬浮保存钮是否可用。
    ///
    /// 判据直接复用 <see cref="CanRestoreSelectedTab"/> ——「本页有未保存改动」
    /// 与「本页可还原」是同一件事。另开一套脏判定必然与它漂移。
    /// </summary>
    private bool CanSaveSelectedTab() => CanRestoreSelectedTab();

    /// <summary>
    /// 保存当前分页。
    ///
    /// 各分页原本各自散着一个「保存」按钮（有的还两个），作者要在长页里找它。
    /// 悬浮钮把这件事收成一个固定落点：改哪一页就保存哪一页，
    /// retrieval 页跨两个 section，两个都要写回。
    /// </summary>
    private async Task SaveSelectedTabAsync()
    {
        switch (SelectedTab.Id)
        {
            case "general":
                await SaveGeneralAsync().ConfigureAwait(true);
                break;
            case "models":
                await SaveModelAsync().ConfigureAwait(true);
                break;
            case "presets":
                // 预设页也横跨两个 section：模板仓库地址与预设本身分开写回。
                // 只存前者会让「有未保存改动」一直亮着却存不下去。
                await SavePresetsAsync().ConfigureAwait(true);
                await SaveTemplateRepositoryAsync().ConfigureAwait(true);
                break;
            case "automation":
                await SaveAutomationAsync().ConfigureAwait(true);
                break;
            case "permissions":
                await SavePermissionsAsync().ConfigureAwait(true);
                break;
            case "personalization":
                await SavePersonalizationAsync().ConfigureAwait(true);
                break;
            case "retrieval":
                // 检索页横跨 app_runtime 与 retrieval 两个 section，逐个写回。
                await SaveAppRuntimeAsync().ConfigureAwait(true);
                await SaveRetrievalAsync().ConfigureAwait(true);
                break;
            case "version_control":
                await SaveGitAsync().ConfigureAwait(true);
                break;
        }
    }

    private bool CanRestoreRecommendedDefaults() => SelectedTab.Id switch
    {
        "automation" => CanSave(AutomationSection),
        "permissions" => CanSave(PermissionsSection),
        "personalization" => CanSave(PersonalizationSection),
        "retrieval" => CanSave(AppRuntimeSection) && CanSave(RetrievalSection),
        "version_control" => CanSave(GitSection),
        _ => false,
    };

    private async Task RestoreRecommendedDefaultsAsync()
    {
        if (!CanRestoreRecommendedDefaults())
        {
            return;
        }

        var confirmed = await DialogService.Current.ConfirmAsync(new ConfirmDialogViewModel(
            _displayNames.Text("ui.dialog.settings.restore_defaults.title"),
            _displayNames.Format(
                "ui.dialog.settings.restore_defaults.message",
                new Dictionary<string, string> { ["section"] = SelectedTab.Title }),
            new[]
            {
                new DialogButton(RestoreRecommendedDefaultsText, DialogButtonVariant.Primary, 0),
                new DialogButton(_displayNames.Text("ui.common.cancel"), DialogButtonVariant.Subtle, 1),
            })
        {
            Severity = DialogSeverity.Warning,
            ConfirmResultIndex = 0,
            CancelResultIndex = 1,
        }.SealKeyboardRoles()).ConfigureAwait(true);
        if (confirmed != 0)
        {
            return;
        }

        ApplyRecommendedDefaults(SelectedTab.Id);
        StatusText = _displayNames.Text("ui.settings.restore_recommended_defaults.pending");
    }

    private void ApplyRecommendedDefaults(string tabId)
    {
        switch (tabId)
        {
            case "automation":
                BudgetUsd = "0";
                // U112：推荐默认是「预授权未设置」（留空），不是零额度。
                PreauthorizedUsd = string.Empty;
                SelectedConfirmationProfile = ConfirmationProfileOptions.First(item => item.Value == "recommended");
                WorkflowDefaultTimeoutMs = "300";
                MaxLoopIterations = "5";
                MaxToolRounds = "8";
                CheckpointEnabled = true;
                RunEventRetentionDays = "30";
                break;
            case "permissions":
                SelectedPermissionProfile = PermissionProfileOptions.First(item => item.Value == "recommended");
                break;
            case "personalization":
                Theme = "system";
                ThemeFollowSystemColors = true;
                GitAutoColor = "#8a8f98";
                GitManualColor = "#f59e0b";
                ProjectPanelVisible = true;
                ReduceMotion = false;
                break;
            case "retrieval":
                VectorEnabled = false;
                VectorBackend = "qdrant_sidecar";
                VectorCollection = "ariadne_chunks";
                VectorDimensions = "1536";
                QdrantHost = "127.0.0.1";
                QdrantPort = "6333";
                QdrantUseTls = false;
                QdrantAuthMode = "none";
                QdrantApiKey = string.Empty;
                QdrantDataDir = ".indexes/qdrant";
                QdrantBinaryPath = "qdrant";
                QdrantStartupTimeoutMs = "10000";
                RerankerEnabled = false;
                ChunkSizeChars = "2000";
                ChunkOverlapChars = "200";
                break;
            case "version_control":
                TrackDocuments = true;
                TrackWorkflows = true;
                TrackSkills = true;
                TrackNonSensitiveConfig = true;
                IgnoredPathsText = string.Join(Environment.NewLine, RecommendedGitIgnoredPaths);
                break;
        }
        UpdateDirtyState();
    }

    internal void ApplyRecommendedDefaultsForTests(string tabId) => ApplyRecommendedDefaults(tabId);

    private async Task RestoreSelectedTabAsync()
    {
        var generation = _draftState.CurrentLoadGeneration;
        var restored = SelectedTab.Id switch
        {
            "general" => await LoadSectionAsync(
                generation,
                GeneralSection,
                async () => (
                    await _backend.GetAppSettingsAsync().ConfigureAwait(true),
                    await _backend.ReadProjectMemoryAsync().ConfigureAwait(true)),
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
                }).ConfigureAwait(true),
            "models" => await LoadSectionAsync(
                generation,
                ModelsSection,
                () => _backend.GetProviderConfigAsync(),
                value =>
                {
                    _providerConfig = value;
                    RebuildProviderOptionsFromConfig(preferProviderId: ProviderId);
                }).ConfigureAwait(true),
            "presets" => await RestorePresetsTabAsync(generation).ConfigureAwait(true),
            "automation" => await LoadSectionAsync(
                generation,
                AutomationSection,
                async () => (
                    await _backend.GetAutomationSettingsAsync().ConfigureAwait(true),
                    await _backend.GetWorkflowSettingsAsync().ConfigureAwait(true)),
                value =>
                {
                    ApplyAutomation(value.Item1);
                    _workflowSchemaVersion = value.Item2.Workflow.SchemaVersion;
                    WorkflowDefaultTimeoutMs = SecondsFromStoredMs(value.Item2.Workflow.DefaultTimeoutMs);
                    MaxLoopIterations = value.Item2.Workflow.MaxLoopIterations.ToString(CultureInfo.InvariantCulture);
                    MaxToolRounds = value.Item2.Workflow.MaxToolRounds.ToString(CultureInfo.InvariantCulture);
                    CheckpointEnabled = value.Item2.Workflow.CheckpointEnabled;
                    RunEventRetentionDays = value.Item2.Workflow.RunEventRetentionDays.ToString(CultureInfo.InvariantCulture);
                }).ConfigureAwait(true),
            "permissions" => await LoadSectionAsync(
                generation,
                PermissionsSection,
                () => _backend.GetPermissionsSettingsAsync(),
                ApplyPermissions).ConfigureAwait(true),
            "personalization" => await LoadSectionAsync(
                generation,
                PersonalizationSection,
                () => _backend.GetUiPreferencesAsync(),
                ApplyLoadedUiPreferences).ConfigureAwait(true),
            "retrieval" => await RestoreRetrievalTabAsync(generation).ConfigureAwait(true),
            "version_control" => await LoadSectionAsync(
                generation,
                GitSection,
                () => _backend.GetGitSettingsAsync(),
                ApplyGit).ConfigureAwait(true),
            _ => false,
        };

        if (restored)
        {
            StatusText = _displayNames.Text("ui.common.configured");
        }
        NotifySectionStateChanged();
        UpdateDirtyState(updateStatus: false);
    }

    private async Task<bool> RestorePresetsTabAsync(long generation)
    {
        var presets = await LoadSectionAsync(
            generation,
            PresetsSection,
            () => _backend.GetNodePresetSettingsAsync(),
            value =>
            {
                ApplyModelAliases(value.ModelAliases);
                ApplyDefaultModelIdentity(value.DefaultModelAlias, value.DefaultProviderId, value.DefaultModelId);
                DefaultTimeoutMs = SecondsFromStoredMs(value.DefaultTimeoutMs);
                DefaultBudgetUsd = value.DefaultBudgetUsd.ToString("0.####", CultureInfo.InvariantCulture);
                ApplyNodePresets(value, BuildEffectiveWorkflowNodePermissionPolicy());
            }).ConfigureAwait(true);
        var repository = await LoadSectionAsync(
            generation,
            TemplateRepositorySection,
            () => _backend.GetTemplateRepositorySettingsAsync(),
            value => TemplateRepositoryBaseUrl = value.BaseUrl).ConfigureAwait(true);
        return presets && repository;
    }

    private async Task<bool> RestoreRetrievalTabAsync(long generation)
    {
        var runtime = await LoadSectionAsync(
            generation,
            AppRuntimeSection,
            () => _backend.GetAppRuntimeSettingsAsync(),
            ApplyAppRuntime).ConfigureAwait(true);
        var retrieval = await LoadSectionAsync(
            generation,
            RetrievalSection,
            () => _backend.GetRagSettingsAsync(),
            ApplyRag).ConfigureAwait(true);
        return runtime && retrieval;
    }

    private string SectionTitle(string section) => section switch
    {
        GeneralSection => GeneralTitle,
        ModelsSection => ModelsTitle,
        PresetsSection => PresetsTitle,
        TemplateRepositorySection => TemplatesSectionTitle,
        AutomationSection => AutomationTitle,
        PermissionsSection => PermissionsTitle,
        PersonalizationSection => PersonalizationTitle,
        AppRuntimeSection => AppRuntimeSectionTitle,
        RetrievalSection => RetrievalSectionTitle,
        GitSection => GitSectionTitle,
        _ => section,
    };

    private Task<bool> RefreshDiagnosticsAsync() => RefreshDiagnosticsAsync(null, CancellationToken.None);

    private async Task<bool> RefreshDiagnosticsAsync(
        long? settingsLoadGeneration,
        CancellationToken cancellationToken)
    {
        var ownerGeneration = settingsLoadGeneration ?? 0;
        var request = _diagnosticsRefreshSession.Begin(ownerGeneration);
        using var linked = CancellationTokenSource.CreateLinkedTokenSource(
            cancellationToken,
            request.CancellationToken);
        IsDiagnosticsRefreshing = true;
        try
        {
            var diagnostics = await _backend.GetBackendDiagnosticsAsync(linked.Token).ConfigureAwait(true);
            linked.Token.ThrowIfCancellationRequested();
            if (!_diagnosticsRefreshSession.IsCurrent(request, ownerGeneration)
                || (settingsLoadGeneration is { } loadGeneration
                    && !_draftState.IsCurrentLoad(loadGeneration)))
            {
                return false;
            }

            ApplyDiagnostics(diagnostics);
            return true;
        }
        catch (OperationCanceledException) when (linked.IsCancellationRequested)
        {
            if (cancellationToken.IsCancellationRequested)
            {
                throw;
            }
            return false;
        }
        catch (Exception ex)
        {
            if (_diagnosticsRefreshSession.IsCurrent(request, ownerGeneration))
            {
                StatusText = UserFacingError.Format(ex, _displayNames);
            }
            return false;
        }
        finally
        {
            if (_diagnosticsRefreshSession.IsCurrent(request, ownerGeneration))
            {
                IsDiagnosticsRefreshing = false;
            }
        }
    }

    private void ApplyDiagnostics(BackendDiagnosticsReport report)
    {
        _diagnosticsReport = report;
        DiagnosticsStatus = report.Status;
        DiagnosticsItems.Clear();
        foreach (var item in report.Items)
        {
            DiagnosticsItems.Add(new SettingsDiagnosticItemViewModel(
                DiagnosticComponentLabel(item.Component),
                DiagnosticStatusLabel(item.Status),
                DiagnosticReasonLabel(item.Status, item.Reason),
                DiagnosticRecoveryLabel(item.Component, item.Status)));
        }
        OnPropertyChanged(nameof(HasDiagnosticsItems));
        OnPropertyChanged(nameof(DiagnosticsCopyText));
    }

    private string DiagnosticStatusLabel(string? status) => status switch
    {
        "healthy" => _displayNames.Text("ui.settings.misc.diagnostics.status.healthy"),
        "degraded" => _displayNames.Text("ui.settings.misc.diagnostics.status.degraded"),
        "unavailable" => _displayNames.Text("ui.settings.misc.diagnostics.status.unavailable"),
        _ => _displayNames.Text("ui.settings.misc.diagnostics.status.unknown"),
    };

    private string DiagnosticComponentLabel(string component)
    {
        if (component.StartsWith("provider.", StringComparison.Ordinal))
        {
            var providerId = component.Split('.').LastOrDefault() ?? string.Empty;
            var provider = _providerConfig?.Providers.FirstOrDefault(item =>
                string.Equals(item.Provider, providerId, StringComparison.Ordinal));
            return _displayNames.Format(
                "ui.settings.misc.diagnostics.component.provider",
                new Dictionary<string, string>
                {
                    ["provider"] = provider?.DisplayName ?? _displayNames.Text("ui.settings.misc.diagnostics.component.provider_unknown"),
                });
        }

        if (component.StartsWith("retrieval", StringComparison.Ordinal)
            || component.Contains("qdrant", StringComparison.OrdinalIgnoreCase)
            || component.Contains("tantivy", StringComparison.OrdinalIgnoreCase))
        {
            return _displayNames.Text("ui.settings.misc.diagnostics.component.retrieval");
        }

        return component switch
        {
            "runtime.db" => _displayNames.Text("ui.settings.misc.diagnostics.component.runtime_store"),
            "workflow_runtime_recovery" => _displayNames.Text("ui.settings.misc.diagnostics.component.runtime_recovery"),
            "project.config" => _displayNames.Text("ui.settings.misc.diagnostics.component.project_config"),
            "providers.config" => _displayNames.Text("ui.settings.misc.diagnostics.component.provider_config"),
            "providers.llm.default" => _displayNames.Text("ui.settings.misc.diagnostics.component.default_llm"),
            "providers.embedding.default" => _displayNames.Text("ui.settings.misc.diagnostics.component.default_embedding"),
            "providers.reranker.default" => _displayNames.Text("ui.settings.misc.diagnostics.component.default_reranker"),
            // U118：凭据保护是常驻诊断项——用户当时同意了明文，几个月后未必记得。
            "secrets.protection" => _displayNames.Text("ui.settings.misc.diagnostics.component.secrets_protection"),
            _ => _displayNames.Text("ui.settings.misc.diagnostics.component.other"),
        };
    }

    private string DiagnosticReasonLabel(string status, string? reason)
    {
        if (!string.IsNullOrWhiteSpace(reason)
            && reason.StartsWith("diagnostics.", StringComparison.Ordinal))
        {
            var localized = _displayNames.Text(reason);
            if (!string.Equals(localized, $"[{reason}]", StringComparison.Ordinal))
            {
                return localized;
            }
        }

        return status switch
        {
            "healthy" => _displayNames.Text("ui.settings.misc.diagnostics.reason.healthy"),
            "degraded" => _displayNames.Text("ui.settings.misc.diagnostics.reason.degraded"),
            "unavailable" => _displayNames.Text("ui.settings.misc.diagnostics.reason.unavailable"),
            _ => _displayNames.Text("ui.settings.misc.diagnostics.reason.unknown"),
        };
    }

    private string DiagnosticRecoveryLabel(string component, string status)
    {
        if (string.Equals(status, "healthy", StringComparison.Ordinal))
        {
            return _displayNames.Text("ui.settings.misc.diagnostics.recovery.none");
        }
        if (component.StartsWith("provider", StringComparison.Ordinal))
        {
            return _displayNames.Text("ui.settings.misc.diagnostics.recovery.provider");
        }
        if (component.StartsWith("retrieval", StringComparison.Ordinal)
            || component.Contains("qdrant", StringComparison.OrdinalIgnoreCase)
            || component.Contains("tantivy", StringComparison.OrdinalIgnoreCase))
        {
            return _displayNames.Text("ui.settings.misc.diagnostics.recovery.retrieval");
        }
        if (component == "project.config")
        {
            return _displayNames.Text("ui.settings.misc.diagnostics.recovery.project_config");
        }
        // U118：凭据保护的补救动作是「设主密码 / 确认明文风险」，都在设置页内，
        // 落到 runtime 那条「重启应用」的兜底文案上会把用户指向一个完全无效的操作。
        if (component == "secrets.protection")
        {
            return _displayNames.Text("ui.settings.misc.diagnostics.recovery.secrets");
        }
        return _displayNames.Text("ui.settings.misc.diagnostics.recovery.runtime");
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
                    if (permissionsAccepted)
                    {
                        ClearSectionLoadFailure(PermissionsSection);
                    }
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
                        ApplyModelAliases(presets.ModelAliases);
                        ApplyDefaultModelIdentity(
                            presets.DefaultModelAlias,
                            presets.DefaultProviderId,
                            presets.DefaultModelId);
                        DefaultTimeoutMs = SecondsFromStoredMs(presets.DefaultTimeoutMs);
                        DefaultBudgetUsd = StableNumber(presets.DefaultBudgetUsd);
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
                    if (presetsAccepted)
                    {
                        ClearSectionLoadFailure(PresetsSection);
                    }
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

            if (_draftState.IsCurrentLoad(generation))
            {
                if (!permissionsAccepted)
                {
                    RegisterSectionLoadFailure(
                        PermissionsSection,
                        () => LoadPermissionPresetSectionsAsync(
                            _draftState.CurrentLoadGeneration,
                            CancellationToken.None));
                }
                if (!presetsAccepted)
                {
                    RegisterSectionLoadFailure(
                        PresetsSection,
                        () => LoadPermissionPresetSectionsAsync(
                            _draftState.CurrentLoadGeneration,
                            CancellationToken.None));
                }
            }

            return presetsAccepted && permissionsAccepted;
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            throw;
        }
        catch
        {
            if (_draftState.IsCurrentLoad(generation))
            {
                RegisterSectionLoadFailure(
                    PermissionsSection,
                    () => LoadPermissionPresetSectionsAsync(
                        _draftState.CurrentLoadGeneration,
                        CancellationToken.None));
                RegisterSectionLoadFailure(
                    PresetsSection,
                    () => LoadPermissionPresetSectionsAsync(
                        _draftState.CurrentLoadGeneration,
                        CancellationToken.None));
            }
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

    internal void ApplyDiagnosticsForTests(BackendDiagnosticsReport report) => ApplyDiagnostics(report);

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

    /// <summary>仅供 ARIADNE_UI_START_SECTION 视觉验收使用，不加载或伪造项目配置。</summary>
    internal void OpenPreviewSection(string sectionId)
    {
        if (string.Equals(sectionId, "model_aliases", StringComparison.Ordinal)
            && ModelAliases.Count == 0)
        {
            ApplyModelAliases(null);
        }

        var section = SectionIndexItems.First(item =>
            string.Equals(item.Id, sectionId, StringComparison.Ordinal));
        var tab = Tabs.First(item =>
            string.Equals(item.Id, section.TabId, StringComparison.Ordinal));
        CommitNavigation(new PendingSettingsNavigation(tab, section));
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

    private void ApplyRag(RagSettings rag)
    {
        _ragSchemaVersion = rag.Rag.SchemaVersion;
        VectorEnabled = rag.Rag.VectorStore.Enabled;
        VectorBackend = rag.Rag.VectorStore.Backend;
        VectorCollection = rag.Rag.VectorStore.Collection;
        VectorDimensions = rag.Rag.VectorStore.VectorDimensions.ToString();
        QdrantHost = rag.Rag.VectorStore.Sidecar.Host;
        QdrantPort = rag.Rag.VectorStore.Sidecar.Port.ToString();
        QdrantUseTls = rag.Rag.VectorStore.Sidecar.UseTls;
        QdrantAuthMode = rag.Rag.VectorStore.Sidecar.AuthMode;
        QdrantApiKey = string.Empty;
        HasQdrantApiKey = rag.HasQdrantApiKey;
        QdrantDataDir = rag.Rag.VectorStore.Sidecar.DataDir;
        _fullTextBackend = rag.Rag.FullTextStore.Backend;
        _fullTextIndexDir = rag.Rag.FullTextStore.IndexDir;
        RerankerEnabled = rag.Rag.RerankerEnabled;
        ChunkSizeChars = rag.Rag.ChunkSizeChars.ToString();
        ChunkOverlapChars = rag.Rag.ChunkOverlapChars.ToString();
    }

    private void ApplyGit(GitSettings git)
    {
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
        RebuildProviderDefaultModelRoutes();
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
            ApplyProviderModels(ParseModelsForDisplay(ModelsText));
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
        ApplyProviderModels(selected.Models);
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
            ApplyProviderModels(result.Models);
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

    private async Task TestProviderDraftAsync()
    {
        if (!CanTestProviderDraft())
        {
            return;
        }

        ProviderSettingsUpdate update;
        try
        {
            update = BuildProviderSettingsUpdate();
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return;
        }

        var submittedProvider = SelectedProviderOption?.ProviderId;
        var request = _providerModelRefreshSession.Begin();
        try
        {
            var result = await _backend
                .TestProviderDraftAsync(
                    new ProviderDraftProbe(update, string.IsNullOrWhiteSpace(ApiKey) ? null : ApiKey),
                    request.CancellationToken)
                .ConfigureAwait(true);
            if (!_providerModelRefreshSession.IsCurrent(request)
                || !string.Equals(SelectedProviderOption?.ProviderId, submittedProvider, StringComparison.Ordinal))
            {
                return;
            }

            ProviderId = result.ProviderId;
            ModelsText = string.Join(Environment.NewLine, result.Models.Select(ModelLine));
            EmbeddingModelId = result.Models.FirstOrDefault(IsEmbeddingModel)?.ModelId ?? string.Empty;
            ManualModelsVisible = false;
            ApplyProviderModels(result.Models);
            StatusText = _displayNames.Text("ui.settings.models.test_connection.succeeded");
            UpdateDirtyState();
        }
        catch (OperationCanceledException) when (!_providerModelRefreshSession.IsCurrent(request))
        {
            // 仅忽略被新请求替代的临时探测，避免旧结果写回当前草稿。
        }
        catch (Exception ex)
        {
            if (!_providerModelRefreshSession.IsCurrent(request)
                || !string.Equals(SelectedProviderOption?.ProviderId, submittedProvider, StringComparison.Ordinal))
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

    private async Task<bool> SaveGeneralAsync(
        GeneralSectionSettings settings,
        IReadOnlyDictionary<string, string> submitted)
    {
        if (!await ConfirmDirectorySwitchIfNeededAsync(submitted).ConfigureAwait(true))
        {
            return false;
        }

        return await RunSectionSaveAsync(GeneralSection, submitted, async () =>
        {
            await _backend.SaveGeneralSectionSettingsAsync(settings).ConfigureAwait(true);
        }).ConfigureAwait(true);
    }

    private async Task<bool> ConfirmDirectorySwitchIfNeededAsync(
        IReadOnlyDictionary<string, string> submitted)
    {
        var changes = DirectorySwitchChanges(submitted);
        if (changes.Count == 0)
        {
            return true;
        }

        var details = string.Join(Environment.NewLine, changes.Select(change =>
            _displayNames.Format(
                "ui.dialog.settings.directory_switch.change",
                new Dictionary<string, string>
                {
                    ["label"] = change.Label,
                    ["old"] = change.OldPath,
                    ["new"] = change.NewPath,
                    ["status"] = change.TargetStatus,
                })));
        var result = await DialogService.Current.ConfirmAsync(new ConfirmDialogViewModel(
            _displayNames.Text("ui.dialog.settings.directory_switch.title"),
            _displayNames.Format(
                "ui.dialog.settings.directory_switch.message",
                new Dictionary<string, string> { ["changes"] = details }),
            new[]
            {
                new DialogButton(
                    _displayNames.Text("ui.dialog.settings.directory_switch.confirm"),
                    DialogButtonVariant.Primary,
                    0),
                new DialogButton(_displayNames.Text("ui.common.cancel"), DialogButtonVariant.Subtle, 1),
            })
        {
            Severity = DialogSeverity.Warning,
            ConfirmResultIndex = 0,
            CancelResultIndex = 1,
        }.SealKeyboardRoles()).ConfigureAwait(true);
        return result == 0;
    }

    private IReadOnlyList<DirectorySwitchChange> DirectorySwitchChanges(
        IReadOnlyDictionary<string, string> submitted)
    {
        var fields = new[]
        {
            (nameof(DocumentsDir), DocumentsDirLabel),
            (nameof(WorkflowsDir), WorkflowsDirLabel),
            (nameof(SkillsDir), SkillsDirLabel),
            (nameof(ExportsDir), ExportsDirLabel),
        };
        var changes = new List<DirectorySwitchChange>();
        foreach (var (field, label) in fields)
        {
            if (!submitted.TryGetValue(field, out var current)
                || !_draftState.TryGetSavedValue(GeneralSection, field, out var saved)
                || string.Equals(current.Trim(), saved.Trim(), StringComparison.Ordinal))
            {
                continue;
            }

            changes.Add(new DirectorySwitchChange(
                label,
                DisplayProjectDirectory(saved),
                DisplayProjectDirectory(current),
                DirectoryTargetStatus(current)));
        }
        return changes;
    }

    private string DisplayProjectDirectory(string path)
    {
        try
        {
            return Path.GetFullPath(Path.Combine(ProjectRoot, path.Trim()));
        }
        catch
        {
            return path.Trim();
        }
    }

    private string DirectoryTargetStatus(string path)
    {
        try
        {
            var root = Path.GetFullPath(ProjectRoot);
            var target = Path.GetFullPath(Path.Combine(root, path.Trim()));
            var relative = Path.GetRelativePath(root, target);
            if (Path.IsPathFullyQualified(path)
                || relative == ".."
                || relative.StartsWith($"..{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
                || relative.StartsWith($"..{Path.AltDirectorySeparatorChar}", StringComparison.Ordinal))
            {
                return _displayNames.Text("ui.dialog.settings.directory_switch.status.invalid");
            }
            if (!Directory.Exists(target))
            {
                return _displayNames.Text("ui.dialog.settings.directory_switch.status.missing");
            }
            return Directory.EnumerateFileSystemEntries(target).Any()
                ? _displayNames.Text("ui.dialog.settings.directory_switch.status.not_empty")
                : _displayNames.Text("ui.dialog.settings.directory_switch.status.empty");
        }
        catch
        {
            return _displayNames.Text("ui.dialog.settings.directory_switch.status.unavailable");
        }
    }

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
            var defaultModels = BuildProviderDefaultModelRoutes();
            var apiKey = ApiKey;
            var submitted = CurrentSectionValues(ModelsSection);
            return SaveModelAsync(update, defaultModels, apiKey, submitted);
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
        ProviderDefaultModelRoutes defaultModels,
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
                        string.IsNullOrWhiteSpace(apiKey) ? null : apiKey,
                        defaultModels)).ConfigureAwait(true);
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
        if (!ValidateProviderModelRows())
        {
            var firstInvalid = ProviderModels.First(row =>
                row.HasModelIdError || row.HasCapabilityError || row.HasMaxContextTokensError
                || row.HasInputCostError || row.HasOutputCostError);
            throw new SettingsInputException(SettingsInputFailure.ModelLine, "ui.settings.models.models")
                .WithFocusItem(firstInvalid);
        }
        return new ProviderSettingsUpdate(
            ProviderId,
            ProviderType,
            ProviderDisplayName,
            ProviderEnabled,
            string.IsNullOrWhiteSpace(ProviderBaseUrl) ? null : ProviderBaseUrl,
            MergeEmbeddingModel(
                SettingsInputValidation.Models(ProviderModelsText(), "ui.settings.models.models"),
                EmbeddingModelId),
            string.Equals(SelectedDefaultLlmRoute?.ProviderId, ProviderId, StringComparison.Ordinal),
            string.Equals(SelectedDefaultEmbeddingRoute?.ProviderId, ProviderId, StringComparison.Ordinal),
            string.Equals(SelectedDefaultRerankerRoute?.ProviderId, ProviderId, StringComparison.Ordinal),
            string.Equals(SelectedDefaultSearchRoute?.ProviderId, ProviderId, StringComparison.Ordinal));
    }

    private void ApplyProviderModels(IEnumerable<ModelConfig> models)
    {
        ProviderModels.Clear();
        AvailableModels.Clear();
        foreach (var model in models)
        {
            ProviderModels.Add(new ProviderModelEditorRow(
                model.ModelId,
                model.Capability,
                model.MaxContextTokens?.ToString(CultureInfo.InvariantCulture) ?? string.Empty,
                model.InputCostPerMillionTokens is { } input ? StableNumber(input) : string.Empty,
                model.OutputCostPerMillionTokens is { } output ? StableNumber(output) : string.Empty,
                OnProviderModelsChanged,
                RemoveProviderModelRow));
            AvailableModels.Add(CreateModelOption(model));
        }
        // U145：候选取自这批「后端交给我们的」模型（已落库配置 + 刷新/测试连接拉回的目录）。
        // 刻意**不**把用户正在手打的行回收进候选：那等于把打错的 id 洗成「看起来是官方的选项」，
        // 下一次就再也分不清哪个才是真的。
        IdentifierCandidates.Sync(
            FetchedModelIdCandidates,
            IdentifierCandidates.Compose(ProviderModels.Select(row => row.ModelId)));
        ValidateProviderModelRows();
        ModelsText = ProviderModelsText();
        RebuildProviderDefaultModelRoutes();
    }

    private void AddProviderModelRow()
    {
        ProviderModels.Add(new ProviderModelEditorRow(
            string.Empty,
            "llm",
            string.Empty,
            string.Empty,
            string.Empty,
            OnProviderModelsChanged,
            RemoveProviderModelRow));
        OnProviderModelsChanged();
    }

    private void RemoveProviderModelRow(ProviderModelEditorRow row)
    {
        if (ProviderModels.Remove(row))
        {
            OnProviderModelsChanged();
        }
    }

    private void OnProviderModelsChanged()
    {
        ValidateProviderModelRows();
        ModelsText = ProviderModelsText();
        RebuildProviderDefaultModelRoutes();
        UpdateDirtyState();
    }

    private bool ValidateProviderModelRows()
    {
        var duplicateIds = ProviderModels
            .Select(row => row.ModelId.Trim())
            .Where(id => !string.IsNullOrWhiteSpace(id))
            .GroupBy(id => id, StringComparer.Ordinal)
            .Where(group => group.Count() > 1)
            .Select(group => group.Key)
            .ToHashSet(StringComparer.Ordinal);
        var messages = new ProviderModelValidationMessages(
            ValidationMessage("ui.settings.validation.required", ModelIdColumnLabel),
            ValidationMessage("ui.settings.validation.duplicate", ModelIdColumnLabel),
            ValidationMessage("ui.settings.validation.required", ModelCapabilityColumnLabel),
            ValidationMessage("ui.settings.validation.positive", ModelContextColumnLabel),
            ValidationMessage("ui.settings.validation.non_negative", ModelInputCostColumnLabel),
            ValidationMessage("ui.settings.validation.non_negative", ModelOutputCostColumnLabel));
        // Validate 会写入每行错误文案，必须逐行执行，不能用 All 短路。
        var valid = true;
        foreach (var row in ProviderModels)
        {
            valid &= row.Validate(duplicateIds, messages);
        }

        return valid;
    }

    private string ValidationMessage(string key, string field) => _displayNames.Format(
        key,
        new Dictionary<string, string> { ["field"] = field });

    private string ProviderModelsText() => string.Join(
        Environment.NewLine,
        ProviderModels.Select(row => string.Join(",", new[]
        {
            row.ModelId.Trim(),
            row.Capability,
            row.MaxContextTokens.Trim(),
            row.InputCost.Trim(),
            row.OutputCost.Trim(),
        })));

    private ProviderDefaultModelRoutes BuildProviderDefaultModelRoutes() => new(
        RouteTarget(SelectedDefaultLlmRoute),
        RouteTarget(SelectedDefaultEmbeddingRoute),
        RouteTarget(SelectedDefaultRerankerRoute),
        RouteTarget(SelectedDefaultSearchRoute));

    private static ModelAliasTarget? RouteTarget(ProviderModelRouteOption? option) =>
        option is null || string.IsNullOrWhiteSpace(option.ProviderId) || string.IsNullOrWhiteSpace(option.ModelId)
            ? null
            : new ModelAliasTarget(option.ProviderId, option.ModelId);

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
        RebuildProviderDefaultModelRoutes();
    }

    private void RebuildProviderDefaultModelRoutes()
    {
        // 留存选项本身而非 RouteTarget()：占位「无」项经 RouteTarget 会变成 null，
        // 随后回落到已保存的默认路由，用户清空路由的选择就被还原了。
        var previousLlm = SelectedDefaultLlmRoute;
        var previousEmbedding = SelectedDefaultEmbeddingRoute;
        var previousReranker = SelectedDefaultRerankerRoute;
        var previousSearch = SelectedDefaultSearchRoute;
        var none = _displayNames.Text("ui.common.none");
        ResetRouteOptions(DefaultLlmRouteOptions, "llm", none);
        ResetRouteOptions(DefaultEmbeddingRouteOptions, "embedding", none);
        ResetRouteOptions(DefaultRerankerRouteOptions, "reranker", none);
        ResetRouteOptions(DefaultSearchRouteOptions, "search", none);
        if (_providerConfig is not null)
        {
            foreach (var provider in _providerConfig.Providers
                         .Where(item => item.Configured)
                         .OrderBy(item => item.DisplayName, StringComparer.Ordinal))
            {
                var isCurrent = string.Equals(provider.Provider, ProviderId, StringComparison.Ordinal);
                if (isCurrent ? !ProviderEnabled : !provider.Enabled)
                {
                    continue;
                }
                var models = isCurrent ? ProviderModelsForRouting() : provider.Models;
                foreach (var model in models.OrderBy(item => item.ModelId, StringComparer.Ordinal))
                {
                    var option = new ProviderModelRouteOption(
                        provider.Provider,
                        model.ModelId,
                        $"{provider.DisplayName} / {model.ModelId}",
                        model.Capability);
                    if (string.Equals(model.Capability, "llm", StringComparison.OrdinalIgnoreCase)
                        || string.Equals(model.Capability, "tool_use", StringComparison.OrdinalIgnoreCase))
                    {
                        DefaultLlmRouteOptions.Add(option);
                    }
                    if (string.Equals(model.Capability, "embedding", StringComparison.OrdinalIgnoreCase))
                    {
                        DefaultEmbeddingRouteOptions.Add(option);
                    }
                    if (string.Equals(model.Capability, "reranker", StringComparison.OrdinalIgnoreCase))
                    {
                        DefaultRerankerRouteOptions.Add(option);
                    }
                    if (string.Equals(model.Capability, "search", StringComparison.OrdinalIgnoreCase))
                    {
                        DefaultSearchRouteOptions.Add(option);
                    }
                }
            }
        }

        _selectedDefaultLlmRoute = ResolveRoute(
            DefaultLlmRouteOptions,
            previousLlm?.ProviderId ?? _providerConfig?.DefaultLlmProviderId,
            previousLlm?.ModelId ?? _providerConfig?.DefaultLlmModelId);
        _selectedDefaultEmbeddingRoute = ResolveRoute(
            DefaultEmbeddingRouteOptions,
            previousEmbedding?.ProviderId ?? _providerConfig?.DefaultEmbeddingProviderId,
            previousEmbedding?.ModelId ?? _providerConfig?.DefaultEmbeddingModelId);
        _selectedDefaultRerankerRoute = ResolveRoute(
            DefaultRerankerRouteOptions,
            previousReranker?.ProviderId ?? _providerConfig?.DefaultRerankerProviderId,
            previousReranker?.ModelId ?? _providerConfig?.DefaultRerankerModelId);
        _selectedDefaultSearchRoute = ResolveRoute(
            DefaultSearchRouteOptions,
            previousSearch?.ProviderId ?? _providerConfig?.DefaultSearchProviderId,
            previousSearch?.ModelId ?? _providerConfig?.DefaultSearchModelId);
        OnPropertyChanged(nameof(SelectedDefaultLlmRoute));
        OnPropertyChanged(nameof(SelectedDefaultEmbeddingRoute));
        OnPropertyChanged(nameof(SelectedDefaultRerankerRoute));
        OnPropertyChanged(nameof(SelectedDefaultSearchRoute));
    }

    // 能力集合需与下方路由分派及后端 validate_default_provider 一致：tool_use 同样是合法的默认 LLM 路由。
    private IReadOnlyList<ModelConfig> ProviderModelsForRouting() => ProviderModels
        .Where(row => !string.IsNullOrWhiteSpace(row.ModelId))
        .Where(row => row.Capability is "llm" or "tool_use" or "embedding" or "reranker" or "search")
        .Select(row => new ModelConfig(row.ModelId.Trim(), row.Capability, null, null, null))
        .ToArray();

    private static void ResetRouteOptions(
        ObservableCollection<ProviderModelRouteOption> options,
        string capability,
        string none)
    {
        options.Clear();
        options.Add(new ProviderModelRouteOption(string.Empty, string.Empty, none, capability));
    }

    private static ProviderModelRouteOption ResolveRoute(
        IEnumerable<ProviderModelRouteOption> options,
        string? providerId,
        string? modelId) =>
        options.FirstOrDefault(option =>
            string.Equals(option.ProviderId, providerId, StringComparison.Ordinal)
            && (string.IsNullOrWhiteSpace(modelId)
                || string.Equals(option.ModelId, modelId, StringComparison.Ordinal)))
        ?? options.First();

    private void ClearDefaultRoutesForProvider(string providerId)
    {
        if (string.IsNullOrWhiteSpace(providerId))
        {
            return;
        }
        if (string.Equals(SelectedDefaultLlmRoute?.ProviderId, providerId, StringComparison.Ordinal))
        {
            SelectedDefaultLlmRoute = DefaultLlmRouteOptions.FirstOrDefault();
        }
        if (string.Equals(SelectedDefaultEmbeddingRoute?.ProviderId, providerId, StringComparison.Ordinal))
        {
            SelectedDefaultEmbeddingRoute = DefaultEmbeddingRouteOptions.FirstOrDefault();
        }
        if (string.Equals(SelectedDefaultRerankerRoute?.ProviderId, providerId, StringComparison.Ordinal))
        {
            SelectedDefaultRerankerRoute = DefaultRerankerRouteOptions.FirstOrDefault();
        }
        if (string.Equals(SelectedDefaultSearchRoute?.ProviderId, providerId, StringComparison.Ordinal))
        {
            SelectedDefaultSearchRoute = DefaultSearchRouteOptions.FirstOrDefault();
        }
        MakeDefaultLlm = false;
        MakeDefaultEmbedding = false;
        MakeDefaultReranker = false;
        MakeDefaultSearch = false;
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

    private bool CanRevokeProviderKey()
    {
        var providerId = SelectedProviderOption?.ProviderId;
        return CanUsePersistedProvider()
            && _providerConfig?.Providers.Any(provider =>
                string.Equals(provider.Provider, providerId, StringComparison.Ordinal)
                && provider.HasKey) == true;
    }

    private async Task RevokeProviderKeyAsync()
    {
        var providerId = SelectedProviderOption?.ProviderId;
        if (string.IsNullOrWhiteSpace(providerId) || !CanRevokeProviderKey())
        {
            return;
        }
        var providerName = _providerConfig?.Providers
            .FirstOrDefault(provider => string.Equals(provider.Provider, providerId, StringComparison.Ordinal))
            ?.DisplayName ?? ProviderDisplayName;
        var confirmed = await DialogService.Current.ConfirmAsync(new ConfirmDialogViewModel(
            _displayNames.Text("ui.dialog.settings.revoke_key.title"),
            _displayNames.Format("ui.dialog.settings.revoke_key.message", new Dictionary<string, string>
            {
                ["provider"] = providerName,
            }),
            new[]
            {
                new DialogButton(_displayNames.Text("ui.settings.models.revoke_key"), DialogButtonVariant.Danger, 0),
                new DialogButton(_displayNames.Text("ui.common.cancel"), DialogButtonVariant.Subtle, 1),
            })
        {
            Severity = DialogSeverity.Danger,
            ConfirmResultIndex = 0,
            CancelResultIndex = 1,
        }.SealKeyboardRoles()).ConfigureAwait(true);
        if (confirmed != 0)
        {
            return;
        }

        try
        {
            var status = await _backend.RevokeProviderKeyAsync(providerId).ConfigureAwait(true);
            MergeProviderConfigCache(status, preserveFormSnapshots: true);
            StatusText = _displayNames.Text("ui.settings.models.key_revoked");
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
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
            "model_alias" => "ui.dialog.settings.remove_provider.reference.model_alias",
            "workflow" => "ui.dialog.settings.remove_provider.reference.workflow",
            "active_run" => "ui.dialog.settings.remove_provider.reference.active_run",
            _ => "ui.dialog.settings.remove_provider.reference.unknown",
        };
        return _displayNames.Format(key, new Dictionary<string, string>
        {
            ["owner"] = reference.ReferenceType == "model_alias"
                ? _displayNames.Text($"ui.settings.presets.alias.{reference.OwnerId}")
                : reference.OwnerId,
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
                nameof(DefaultModelAlias),
                nameof(ModelAliases),
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
                ApplyCanonicalText(submitted, persisted, nameof(DefaultModelAlias), saved.DefaultModelAlias ?? string.Empty,
                    value => ApplyDefaultModelIdentity(saved.DefaultModelAlias, saved.DefaultProviderId, saved.DefaultModelId));
                ApplyCanonicalText(submitted, persisted, nameof(DefaultProviderId), saved.DefaultProviderId,
                    value => ApplyDefaultModelIdentity(saved.DefaultModelAlias, value, saved.DefaultModelId));
                ApplyCanonicalText(submitted, persisted, nameof(DefaultModelId), saved.DefaultModelId,
                    value => ApplyDefaultModelIdentity(saved.DefaultModelAlias, saved.DefaultProviderId, value));
                ApplyCanonicalText(submitted, persisted, nameof(DefaultTimeoutMs),
                    SecondsFromStoredMs(saved.DefaultTimeoutMs), value => DefaultTimeoutMs = value);
                ApplyCanonicalText(submitted, persisted, nameof(DefaultBudgetUsd),
                    StableNumber(saved.DefaultBudgetUsd), value => DefaultBudgetUsd = value);
                if (CurrentSectionValues(PresetsSection).TryGetValue(nameof(ModelAliases), out var currentAliases)
                    && submitted.TryGetValue(nameof(ModelAliases), out var submittedAliases)
                    && string.Equals(currentAliases, submittedAliases, StringComparison.Ordinal))
                {
                    ApplyModelAliases(saved.ModelAliases);
                    persisted[nameof(ModelAliases)] = CurrentSectionValues(PresetsSection)[nameof(ModelAliases)];
                }
                else if (submitted.TryGetValue(nameof(ModelAliases), out var submittedAliasSnapshot))
                {
                    persisted[nameof(ModelAliases)] = submittedAliasSnapshot;
                }
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

    private NodePresetSettings BuildNodePresetSettings()
    {
        var aliases = ModelAliases
            .Where(alias => alias.IsConfigured)
            .ToDictionary(
                alias => alias.AliasId,
                alias => new ModelAliasTarget(alias.TargetProviderId, alias.TargetModelId),
                StringComparer.Ordinal);
        foreach (var alias in ModelAliases.Where(alias => alias.IsConfigured && alias.SelectedTargetOption is null))
        {
            throw new SettingsInputException(SettingsInputFailure.Required, alias.DisplayNameKey)
                .WithFocusItem(alias);
        }
        foreach (var referencedAlias in NodePresets.Select(item => item.ModelAlias)
                     .Append(DefaultModelAlias)
                     .Where(alias => !string.IsNullOrWhiteSpace(alias))
                     .Distinct(StringComparer.Ordinal))
        {
            if (!aliases.ContainsKey(referencedAlias!))
            {
                var exception = new SettingsInputException(
                    SettingsInputFailure.Required,
                    $"ui.settings.presets.alias.{referencedAlias}");
                var alias = ModelAliases.FirstOrDefault(item =>
                    string.Equals(item.AliasId, referencedAlias, StringComparison.Ordinal));
                throw alias is null ? exception : exception.WithFocusItem(alias);
            }
        }

        return new NodePresetSettings(
            NodePresets.Select(BuildNodeTypePreset).ToArray(),
            DefaultModelId,
            SettingsInputValidation.PositiveLong(
                SecondsUiToMsString(DefaultTimeoutMs, "ui.settings.presets.default_timeout_ms"),
                "ui.settings.presets.default_timeout_ms"),
            SettingsInputValidation.NonNegativeDouble(
                DefaultBudgetUsd,
                "ui.settings.presets.default_budget_usd"),
            DefaultProviderId,
            aliases,
            DefaultModelAlias);
    }

    private static NodeTypePreset BuildNodeTypePreset(NodeTypePresetViewModel item)
    {
        try
        {
            return new NodeTypePreset(
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
                item.ProviderId,
                item.ModelAlias);
        }
        catch (SettingsInputException exception)
        {
            throw exception.WithFocusItem(item);
        }
    }

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
                ApplyCanonicalText(submitted, persisted, nameof(RunEventRetentionDays),
                    saved.Workflow.Workflow.RunEventRetentionDays.ToString(CultureInfo.InvariantCulture), value => RunEventRetentionDays = value);
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
        foreach (var item in ConfirmationPolicies)
        {
            item.SetApprovalPromptError(string.Empty);
        }
        var automation = new AutomationSettings(
            new BudgetStatus(
                SettingsInputValidation.NonNegativeDouble(
                    BudgetUsd,
                    "ui.settings.automation.global_budget"),
                _spentUsd,
                SettingsInputValidation.OptionalNonNegativeDouble(
                    PreauthorizedUsd,
                    "ui.settings.automation.preauthorized_budget"),
                _projectAutomation.IsEnabled),
            ConfirmationPolicies.Select(item =>
            {
                if (item.AutoModeAutoApproval && string.IsNullOrWhiteSpace(item.ApprovalPrompt))
                {
                    item.SetApprovalPromptError(ValidationMessage(
                        "ui.settings.validation.required",
                        ApprovalPromptLabel));
                    throw new SettingsInputException(
                        SettingsInputFailure.Required,
                        "ui.settings.automation.confirmation.approval_prompt")
                        .WithFocusItem(item);
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
            // 0 合法：表示不清理历史事件，故用 NonNegativeInt 而非 PositiveInt。
            SettingsInputValidation.NonNegativeInt(
                RunEventRetentionDays,
                "ui.settings.automation.run_event_retention_days")));
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

    private PermissionsSettings BuildPermissionsSettings()
    {
        var scopedPolicies = new Dictionary<string, PermissionPolicy?>(
            _compatibilityScopedPolicies,
            StringComparer.Ordinal);
        foreach (var profile in ScopedPermissionProfiles)
        {
            scopedPolicies[profile.Scope] = profile.InheritGlobal ? null : profile.ToPolicy();
        }

        return new PermissionsSettings(
            new PermissionPolicy(
            AllowNetwork,
            AllowWebSearch,
            AllowHttpSkill,
            AllowWasmNetwork,
            AllowSecretRead,
            SettingsInputValidation.AbsolutePaths(
                WritableRootsText,
                "ui.settings.permissions.write_roots"),
            SettingsInputValidation.AbsolutePaths(
                ReadableRootsText,
                "ui.settings.permissions.read_roots")),
            scopedPolicies,
            ToToolControls());
    }

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

    private Task<bool> SaveRetrievalAsync()
    {
        try
        {
            var request = BuildRagSettings();
            var submitted = CurrentSectionValues(RetrievalSection);
            return SaveRetrievalAsync(request, submitted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private Task<bool> SaveRetrievalAsync(
        RagSettings request,
        IReadOnlyDictionary<string, string> submitted)
    {
        try
        {
            var persisted = new Dictionary<string, string>(submitted, StringComparer.Ordinal);
            return RunSectionSaveAsync(RetrievalSection, submitted, async () =>
            {
                var saved = await _backend.SaveRagSettingsAsync(
                    request).ConfigureAwait(true);
                var savedRag = saved.Rag;
                ApplyCanonicalText(submitted, persisted, nameof(VectorDimensions),
                    savedRag.VectorStore.VectorDimensions.ToString(CultureInfo.InvariantCulture), value => VectorDimensions = value);
                ApplyCanonicalText(submitted, persisted, nameof(QdrantPort),
                    savedRag.VectorStore.Sidecar.Port.ToString(CultureInfo.InvariantCulture), value => QdrantPort = value);
                ApplyCanonicalText(submitted, persisted, nameof(QdrantUseTls),
                    savedRag.VectorStore.Sidecar.UseTls.ToString(), value => QdrantUseTls = bool.Parse(value));
                ApplyCanonicalText(submitted, persisted, nameof(QdrantAuthMode),
                    savedRag.VectorStore.Sidecar.AuthMode, value => QdrantAuthMode = value);
                ApplyCanonicalText(submitted, persisted, nameof(QdrantApiKey),
                    string.Empty, value => QdrantApiKey = value);
                persisted[nameof(HasQdrantApiKey)] = saved.HasQdrantApiKey.ToString();
                HasQdrantApiKey = saved.HasQdrantApiKey;
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

    private RagSettings BuildRagSettings()
    {
        var vectorDimensions = VectorEnabled
            ? SettingsInputValidation.PositiveInt(VectorDimensions, "ui.settings.misc.vector_dimensions")
            : ParseInactiveInt(VectorDimensions, 1536);
        var qdrantPort = VectorEnabled && IsExternalQdrantBackend
            ? SettingsInputValidation.PositiveInt(QdrantPort, "ui.settings.misc.qdrant_port")
            : ParseInactiveInt(QdrantPort, 0);
        if (VectorEnabled && IsExternalQdrantBackend && qdrantPort > ushort.MaxValue)
        {
            throw new SettingsInputException(
                SettingsInputFailure.Positive,
                "ui.settings.misc.qdrant_port");
        }
        if (VectorEnabled && IsExternalQdrantBackend && string.IsNullOrWhiteSpace(QdrantHost))
        {
            throw new SettingsInputException(
                SettingsInputFailure.Required,
                "ui.settings.misc.qdrant_host");
        }
        if (VectorEnabled && IsQdrantApiKeyAuth
            && string.IsNullOrWhiteSpace(QdrantApiKey)
            && !HasQdrantApiKeyForCurrentEndpoint())
        {
            throw new SettingsInputException(
                SettingsInputFailure.Required,
                "ui.settings.misc.qdrant_api_key");
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
        // 密钥仅在 API key 认证下发送；切回其它认证方式时改为请求撤销端点上的遗留凭据。
        // 两者互斥，不会触发后端「同一请求既替换又删除」的校验。
        var qdrantApiKey = IsQdrantApiKeyAuth && !string.IsNullOrWhiteSpace(QdrantApiKey)
            ? QdrantApiKey.Trim()
            : null;
        var clearQdrantApiKey = IsExternalQdrantBackend
            && !IsQdrantApiKeyAuth
            && HasQdrantApiKeyForCurrentEndpoint();
        return new RagSettings(new RagConfig(
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
                    30_000,
                    QdrantUseTls,
                    QdrantAuthMode)),
            new FullTextStoreConfig(_fullTextBackend, _fullTextIndexDir),
            RerankerEnabled,
            chunkSize,
            chunkOverlap),
            qdrantApiKey,
            clearQdrantApiKey,
            false);
    }

    internal RagSettings BuildRagSettingsForTests() => BuildRagSettings();

    internal AutomationSectionSettings BuildAutomationSectionSettingsForTests() =>
        BuildAutomationSectionSettings();

    private Task<bool> SaveGitAsync()
    {
        try
        {
            var request = BuildGitSettings();
            var submitted = CurrentSectionValues(GitSection);
            return SaveGitAsync(request, submitted);
        }
        catch (SettingsInputException ex)
        {
            SetValidationStatus(ex);
            return Task.FromResult(false);
        }
    }

    private Task<bool> SaveGitAsync(
        GitSettings request,
        IReadOnlyDictionary<string, string> submitted) =>
        RunSectionSaveAsync(GitSection, submitted, async () =>
        {
            await _backend.SaveGitSettingsAsync(request).ConfigureAwait(true);
        });

    private GitSettings BuildGitSettings() => new(new GitConfig(
            _gitSchemaVersion,
            TrackDocuments,
            TrackWorkflows,
            TrackSkills,
            TrackNonSensitiveConfig,
            SettingsInputValidation.RelativePaths(
                IgnoredPathsText,
                "ui.settings.misc.ignored_paths")));

    private static int ParseInactiveInt(string value, int fallback) =>
        int.TryParse(value, NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsed)
            ? parsed
            : fallback;

    private void ApplyAutomation(AutomationSettings automation)
    {
        BudgetUsd = automation.Budget.BudgetUsd.ToString("0.####");
        // U112：未设置就保持空串，不折叠成 "0"——否则原样保存会把「不限制」
        // 写成「零额度、全部暂停」。空串在保存侧回到 null。
        PreauthorizedUsd = automation.Budget.PreauthorizedUsd?.ToString("0.####") ?? string.Empty;
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
                OnConfirmationPolicyChanged));
        }

        RebuildConfirmationGroups();
        RefreshConfirmationProfile();
    }

    private void OnConfirmationPolicyChanged()
    {
        RefreshConfirmationProfile();
        UpdateDirtyState();
    }

    private void ApplyConfirmationProfile(string? profile)
    {
        if (string.Equals(profile, "custom", StringComparison.Ordinal) || string.IsNullOrWhiteSpace(profile))
        {
            return;
        }

        var (normalAllow, autoApprove) = profile switch
        {
            "conservative" => ((bool?)false, (bool?)false),
            "recommended" => ((bool?)false, (bool?)true),
            "automated" => ((bool?)true, (bool?)true),
            _ => (null, null),
        };
        if (normalAllow is null || autoApprove is null)
        {
            return;
        }

        foreach (var policy in ConfirmationPolicies)
        {
            policy.NormalAllowByDefault = normalAllow.Value;
            policy.AutoModeAutoApproval = autoApprove.Value;
            if (autoApprove.Value && string.IsNullOrWhiteSpace(policy.ApprovalPrompt))
            {
                policy.ApprovalPrompt = DefaultAutoApprovalPrompt;
            }
        }
        RefreshConfirmationProfile();
        UpdateDirtyState();
    }

    private void RefreshConfirmationProfile()
    {
        var profile = ConfirmationPolicies.Count == 0
            ? "custom"
            : ConfirmationPolicies.All(item => !item.NormalAllowByDefault && !item.AutoModeAutoApproval)
                ? "conservative"
                : ConfirmationPolicies.All(item => !item.NormalAllowByDefault && item.AutoModeAutoApproval)
                    ? "recommended"
                    : ConfirmationPolicies.All(item => item.NormalAllowByDefault && item.AutoModeAutoApproval)
                        ? "automated"
                        : "custom";
        var option = ConfirmationProfileOptions.FirstOrDefault(item => item.Value == profile)
            ?? ConfirmationProfileOptions.Last();
        if (!ReferenceEquals(_selectedConfirmationProfile, option))
        {
            _selectedConfirmationProfile = option;
            OnPropertyChanged(nameof(SelectedConfirmationProfile));
        }
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
        RefreshToolControlPolicyGates();
        RefreshPermissionProfile();
    }

    private void ApplyScopedPermissionProfiles(PermissionsSettings settings)
    {
        _compatibilityScopedPolicies.Clear();
        ScopedPermissionProfiles.Clear();
        var supportedScopes = new[] { "workflow_nodes", "project_ai" };
        foreach (var (scope, policy) in settings.ScopedPolicies)
        {
            if (!supportedScopes.Contains(scope, StringComparer.Ordinal))
            {
                _compatibilityScopedPolicies[scope] = policy;
            }
        }
        foreach (var scope in supportedScopes)
        {
            settings.ScopedPolicies.TryGetValue(scope, out var policy);
            ScopedPermissionProfiles.Add(new PermissionScopeProfileViewModel(
                scope,
                PermissionScopeLabel(scope),
                policy,
                settings.Policy,
                () => OnScopedPermissionProfileChanged(scope),
                assign => BrowseIntoAsync(assign),
                _displayNames));
        }
        OnPropertyChanged(nameof(HasCompatibilityPermissionScopes));
        OnPropertyChanged(nameof(CompatibilityPermissionScopesText));
    }

    private void OnScopedPermissionProfileChanged(string scope)
    {
        if (string.Equals(scope, "workflow_nodes", StringComparison.Ordinal))
        {
            RebindNodePresetPermissionParents();
        }
        RefreshToolControlPolicyGates();
        if (_applyingPermissionProfile)
        {
            return;
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
                    markDirty: OnPermissionDetailChanged));
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

    private void OnPermissionDetailChanged()
    {
        if (!_applyingPermissionProfile)
        {
            UpdateDirtyState();
        }
    }

    private void ApplyPermissionProfile(string? profile)
    {
        if (string.IsNullOrWhiteSpace(profile) || string.Equals(profile, "custom", StringComparison.Ordinal))
        {
            return;
        }

        var recommended = string.Equals(profile, "recommended", StringComparison.Ordinal);
        if (!recommended && !string.Equals(profile, "restricted", StringComparison.Ordinal))
        {
            return;
        }

        _applyingPermissionProfile = true;
        try
        {
            AllowNetwork = recommended;
            AllowWebSearch = recommended;
            AllowHttpSkill = false;
            AllowWasmNetwork = false;
            AllowSecretRead = false;

            var projectRoot = PermissionProfileProjectRoot();
            ReadableRootsText = projectRoot;
            WritableRootsText = recommended ? projectRoot : string.Empty;

            foreach (var scoped in ScopedPermissionProfiles)
            {
                scoped.InheritGlobal = true;
            }
            foreach (var group in ToolControlGroups)
            {
                foreach (var control in group.Controls)
                {
                    control.IsEnabled = control.CanInherit ? null : !control.IsDangerous;
                }
            }
        }
        finally
        {
            _applyingPermissionProfile = false;
        }

        RebindPermissionInheritance();
        RefreshPermissionProfile();
        UpdateDirtyState();
    }

    private void RefreshPermissionProfile()
    {
        if (_applyingPermissionProfile)
        {
            return;
        }

        var profile = MatchesPermissionProfile(recommended: false)
            ? "restricted"
            : MatchesPermissionProfile(recommended: true)
                ? "recommended"
                : "custom";
        var option = PermissionProfileOptions.FirstOrDefault(item => item.Value == profile)
            ?? PermissionProfileOptions.Last();
        if (!ReferenceEquals(_selectedPermissionProfile, option))
        {
            _selectedPermissionProfile = option;
            OnPropertyChanged(nameof(SelectedPermissionProfile));
        }
    }

    private bool MatchesPermissionProfile(bool recommended)
    {
        if (AllowNetwork != recommended
            || AllowWebSearch != recommended
            || AllowHttpSkill
            || AllowWasmNetwork
            || AllowSecretRead
            || ScopedPermissionProfiles.Any(item => !item.InheritGlobal))
        {
            return false;
        }

        var projectRoot = PermissionProfileProjectRoot();
        if (!PathLinesEqual(ReadableRootsText, projectRoot)
            || !PathLinesEqual(WritableRootsText, recommended ? projectRoot : string.Empty))
        {
            return false;
        }

        return ToolControlGroups
            .SelectMany(group => group.Controls)
            .All(control => control.IsEnabled == (control.CanInherit ? null : !control.IsDangerous));
    }

    private string PermissionProfileProjectRoot() =>
        Path.IsPathFullyQualified(ProjectRoot) ? Path.GetFullPath(ProjectRoot) : string.Empty;

    private static bool PathLinesEqual(string left, string right)
    {
        static string[] Lines(string value) => (value ?? string.Empty)
            .Replace("\r\n", "\n", StringComparison.Ordinal)
            .Split('\n', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);

        return Lines(left).SequenceEqual(Lines(right), SettingsInputValidation.PathComparer);
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
                UpdateDirtyState,
                preset.ModelAlias));
        }
        RebindPresetModelOptions();
    }

    private void ApplyModelAliases(IReadOnlyDictionary<string, ModelAliasTarget>? aliases)
    {
        ModelAliases.Clear();
        var configured = aliases ?? new Dictionary<string, ModelAliasTarget>(StringComparer.Ordinal);
        foreach (var (aliasId, displayNameKey) in ModelAliasDefinitions)
        {
            configured.TryGetValue(aliasId, out var target);
            ModelAliases.Add(new ModelAliasViewModel(
                aliasId,
                displayNameKey,
                _displayNames.Text(displayNameKey),
                target?.ProviderId ?? string.Empty,
                target?.ModelId ?? string.Empty,
                OnModelAliasChanged));
        }
        RebuildAvailableLlmModelOptions();
    }

    private void OnModelAliasChanged()
    {
        RebuildAvailableLlmModelOptions();
        if (!_suppressDirtyTracking)
        {
            UpdateDirtyState();
        }
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

    /// <summary>
    /// U110：把工具开关分组的 Scope key 映射到生效的 <see cref="PermissionPolicy"/>，
    /// 与后端 `permission_policy_for_node` / `permission_policy_for_scope`
    /// （`core/src/commands.rs`）的映射规则一致：
    /// project_ai 用自己的作用域覆盖，其余节点类作用域（含 global 近似）落到 workflow_nodes。
    /// </summary>
    private PermissionPolicy BuildEffectivePolicyForToolScope(string scope)
    {
        if (string.Equals(scope, "project_ai", StringComparison.Ordinal))
        {
            var projectAiProfile = ScopedPermissionProfiles.FirstOrDefault(profile =>
                string.Equals(profile.Scope, "project_ai", StringComparison.Ordinal));
            return projectAiProfile?.ToPolicy() ?? BuildGlobalPermissionPolicy();
        }
        if (string.Equals(scope, "global", StringComparison.Ordinal))
        {
            return BuildGlobalPermissionPolicy();
        }
        return BuildEffectiveWorkflowNodePermissionPolicy();
    }

    /// <summary>
    /// U110：工具开关的有效状态 = tool_controls &amp;&amp; policy，与后端
    /// `workflow_web_search_tool_enabled`（`core/src/commands.rs:6864`）判定 web-search
    /// 的公式一致。开关本身仍然可交互，只叠加「已开但被策略否决」的提示。
    /// </summary>
    private void RefreshToolControlPolicyGates()
    {
        var hint = _displayNames.Text("ui.settings.permissions.tool.web_search_blocked_by_policy");
        foreach (var group in ToolControlGroups)
        {
            var policy = BuildEffectivePolicyForToolScope(group.Scope);
            var allowed = policy.AllowNetwork && policy.AllowWebSearch;
            group.ApplyPolicyGate(allowed, hint);
        }
    }

    private void RebindPermissionInheritance()
    {
        var global = BuildGlobalPermissionPolicy();
        foreach (var profile in ScopedPermissionProfiles)
        {
            profile.RebindInheritedPolicy(global);
        }
        RebindNodePresetPermissionParents();
        RefreshToolControlPolicyGates();
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
        RecoveryText = string.Empty;
        NotifySectionStateChanged();
        try
        {
            await action().ConfigureAwait(true);
            _draftState.CompleteSave(attempt, persistedValues);
            if (string.Equals(section, RetrievalSection, StringComparison.Ordinal))
            {
                OnPropertyChanged(nameof(QdrantApiKeyStatusText));
            }
            UpdateDirtyState();
            return true;
        }
        catch (Exception ex)
        {
            _draftState.FailSave(attempt);
            HandleSettingsFailure(ex, section);
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
            _ => _displayNames.Format("ui.settings.permissions.unknown_scope", new Dictionary<string, string>
            {
                ["scope"] = scope,
            }),
        };
    }

    private string PermissionScopeLabel(string scope) => scope switch
    {
        "workflow_nodes" => _displayNames.Text("ui.settings.permissions.scope.workflow_nodes"),
        "project_ai" => _displayNames.Text("ui.settings.permissions.scope.project_ai"),
        _ => _displayNames.Format("ui.settings.permissions.unknown_scope", new Dictionary<string, string>
        {
            ["scope"] = scope,
        }),
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
            _ => _displayNames.Format("ui.settings.permissions.unknown_tool", new Dictionary<string, string>
            {
                ["tool"] = tool,
            }),
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

    /// <summary>
    /// U112：可空金额的规范文本。「未设置」渲染成空串而非 <c>0</c>——
    /// 折叠成 0 会在下一次保存时把「不限制」静默写成「零额度、全部暂停」。
    /// </summary>
    private static string StableNumber(double? value) =>
        value.HasValue ? StableNumber(value.Value) : string.Empty;

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
        var concreteOptions = new List<WorkflowModelOption>();
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
                    concreteOptions.Add(new WorkflowModelOption(
                        provider.Provider,
                        model.ModelId,
                        string.IsNullOrWhiteSpace(provider.DisplayName)
                            ? provider.Provider
                            : provider.DisplayName));
                }
            }
        }

        AvailableLlmModelTargetOptions.Clear();
        AvailableLlmModelTargetOptions.Add(
            WorkflowModelOption.Unconfigured(_displayNames.Text("ui.settings.presets.model_alias_unconfigured")));
        foreach (var option in concreteOptions)
        {
            AvailableLlmModelTargetOptions.Add(option);
        }

        AvailableLlmModelOptions.Clear();
        foreach (var alias in ModelAliases)
        {
            var target = alias.IsConfigured
                ? $"{alias.TargetProviderId} · {alias.TargetModelId}"
                : _displayNames.Text("ui.settings.presets.model_alias_unconfigured");
            AvailableLlmModelOptions.Add(WorkflowModelOption.Alias(
                alias.AliasId,
                _displayNames.Format("ui.settings.presets.model_alias_option", new Dictionary<string, string>
                {
                    ["alias"] = alias.DisplayName,
                    ["target"] = target,
                })));
        }
        foreach (var option in concreteOptions)
        {
            AvailableLlmModelOptions.Add(option);
        }

        RebindModelAliasTargetOptions();
        RebindDefaultModelOption();
        RebindPresetModelOptions();
    }

    private void RebindModelAliasTargetOptions()
    {
        var options = AvailableLlmModelTargetOptions;
        foreach (var alias in ModelAliases)
        {
            alias.RebindTargetOptions(options);
        }
    }

    private void ApplyDefaultModelIdentity(string? modelAlias, string providerId, string modelId)
    {
        SetProperty(ref _defaultModelAlias, string.IsNullOrWhiteSpace(modelAlias) ? null : modelAlias.Trim(), nameof(DefaultModelAlias));
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
        var selected = !string.IsNullOrWhiteSpace(_defaultModelAlias)
            ? AvailableLlmModelOptions.FirstOrDefault(option =>
                string.Equals(option.AliasId, _defaultModelAlias, StringComparison.Ordinal))
            : string.IsNullOrWhiteSpace(_defaultProviderId)
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
            // 写回保持**串行**，与读取的并发处理不同。
            //
            // 实测过并发写：4 个 section 并发 vs 串行，三轮比值在 0.7x–1.5x 之间
            // 剧烈摆动——因为各 save 命令都要抢同一把项目互斥锁与 SQLite 写锁，
            // 真正的写入本来就排队，并发只是把排队从后端挪到前端。
            // 收益既不稳定又不显著，却要换来「部分成功」这一类更难处理的语义
            // （前一个 section 已落盘、后一个失败），不划算。
            //
            // 读取那边并发有效（实测 165.8ms → 63.5ms），是因为读不互斥。
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
            // 只重载脏 section，不再整页重拉——见 ReloadDirtySectionsAsync 的说明。
            await ReloadDirtySectionsAsync().ConfigureAwait(true);
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
            var defaultModels = BuildProviderDefaultModelRoutes();
            var apiKey = ApiKey;
            var submitted = CurrentSectionValues(ModelsSection);
            commits.Add(new(ModelsSection, () => SaveModelAsync(request, defaultModels, apiKey, submitted)));
        }
        if (_draftState.IsSectionDirty(PresetsSection, CurrentSectionValues(PresetsSection)))
        {
            var request = BuildNodePresetSettings();
            var submitted = PickValues(
                PresetsSection,
                nameof(DefaultModelAlias),
                nameof(ModelAliases),
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
        if (_draftState.IsSectionDirty(RetrievalSection, CurrentSectionValues(RetrievalSection)))
        {
            var request = BuildRagSettings();
            var submitted = CurrentSectionValues(RetrievalSection);
            commits.Add(new(RetrievalSection, () => SaveRetrievalAsync(request, submitted)));
        }
        if (_draftState.IsSectionDirty(GitSection, CurrentSectionValues(GitSection)))
        {
            var request = BuildGitSettings();
            var submitted = CurrentSectionValues(GitSection);
            commits.Add(new(GitSection, () => SaveGitAsync(request, submitted)));
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
                     RetrievalSection,
                     GitSection,
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

        // U146：chip 的失效说明、占位符、按钮文案都走 display_name，切语言要跟着刷。
        // 漏掉这一处的表现是「切成英文后只有 chip 区还是中文」。
        ReadableRootChips.RefreshLocalizedText();
        WritableRootChips.RefreshLocalizedText();
        IgnoredPathChips.RefreshLocalizedText();
        foreach (var profile in ScopedPermissionProfiles)
        {
            profile.ReadableRootChips.RefreshLocalizedText();
            profile.WritableRootChips.RefreshLocalizedText();
        }
        foreach (var preset in NodePresets)
        {
            preset.Permissions.ReadableRootChips.RefreshLocalizedText();
            preset.Permissions.WritableRootChips.RefreshLocalizedText();
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

        foreach (var option in QdrantAuthModeOptions)
        {
            option.Label = _displayNames.Text($"ui.settings.misc.qdrant_auth.{option.Value}");
        }

        foreach (var option in ProviderTypeOptions)
        {
            option.Label = _displayNames.Text($"ui.settings.models.provider_type.{option.Value}");
        }
        foreach (var option in ConfirmationProfileOptions)
        {
            option.Label = _displayNames.Text($"ui.settings.automation.confirmation.profile.{option.Value}");
        }
        foreach (var option in PermissionProfileOptions)
        {
            option.Label = _displayNames.Text($"ui.settings.permissions.profile.{option.Value}");
        }
        ConfirmationNormalPolicyOptions[0].Label = PolicyReviewText;
        ConfirmationNormalPolicyOptions[1].Label = PolicyAllowText;
        ConfirmationAutoModePolicyOptions[0].Label = PolicyAutoOffText;
        ConfirmationAutoModePolicyOptions[1].Label = PolicyAutoOnText;
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

        foreach (var alias in ModelAliases)
        {
            alias.DisplayName = _displayNames.Text(alias.DisplayNameKey);
        }
        RebuildAvailableLlmModelOptions();

        foreach (var group in ToolControlGroups)
        {
            group.DisplayName = ToolScopeLabel(group.Scope);
            foreach (var control in group.Controls)
            {
                control.DisplayName = ToolLabel(group.Scope, control.ToolId);
            }
        }
        RefreshToolControlPolicyGates();
        foreach (var profile in ScopedPermissionProfiles)
        {
            profile.DisplayName = PermissionScopeLabel(profile.Scope);
        }

        if (_diagnosticsReport is not null)
        {
            ApplyDiagnostics(_diagnosticsReport);
        }

        foreach (var section in _failedSectionRetries.Keys.ToArray())
        {
            RegisterSectionLoadFailure(section, _failedSectionRetries[section]);
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
            [nameof(SelectedDefaultLlmRoute)] = RouteSnapshot(SelectedDefaultLlmRoute),
            [nameof(SelectedDefaultEmbeddingRoute)] = RouteSnapshot(SelectedDefaultEmbeddingRoute),
            [nameof(SelectedDefaultRerankerRoute)] = RouteSnapshot(SelectedDefaultRerankerRoute),
            [nameof(SelectedDefaultSearchRoute)] = RouteSnapshot(SelectedDefaultSearchRoute),
            [nameof(DefaultModelAlias)] = DefaultModelAlias ?? string.Empty,
            [nameof(DefaultProviderId)] = DefaultProviderId,
            [nameof(DefaultModelId)] = DefaultModelId,
            [nameof(DefaultTimeoutMs)] = DefaultTimeoutMs,
            [nameof(DefaultBudgetUsd)] = DefaultBudgetUsd,
            [nameof(ModelAliases)] = string.Join("|", ModelAliases.Select(alias => alias.Snapshot)),
            [nameof(NodePresets)] = string.Join("|", NodePresets.Select(preset => preset.Snapshot)),
            [nameof(TemplateRepositoryBaseUrl)] = TemplateRepositoryBaseUrl,
            [nameof(BudgetUsd)] = BudgetUsd,
            [nameof(PreauthorizedUsd)] = PreauthorizedUsd,
            [nameof(WorkflowDefaultTimeoutMs)] = WorkflowDefaultTimeoutMs,
            [nameof(MaxLoopIterations)] = MaxLoopIterations,
            [nameof(MaxToolRounds)] = MaxToolRounds,
            [nameof(CheckpointEnabled)] = CheckpointEnabled.ToString(),
            [nameof(RunEventRetentionDays)] = RunEventRetentionDays,
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
            [nameof(QdrantUseTls)] = QdrantUseTls.ToString(),
            [nameof(QdrantAuthMode)] = QdrantAuthMode,
            [nameof(QdrantApiKey)] = QdrantApiKey,
            [nameof(HasQdrantApiKey)] = HasQdrantApiKey.ToString(),
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

    private static string RouteSnapshot(ProviderModelRouteOption? option) =>
        option is null
            ? string.Empty
            : $"{SnapshotPart(option.ProviderId)}{SnapshotPart(option.ModelId)}";

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
                nameof(SelectedDefaultLlmRoute), nameof(SelectedDefaultEmbeddingRoute),
                nameof(SelectedDefaultRerankerRoute), nameof(SelectedDefaultSearchRoute),
            },
            PresetsSection => new[]
            {
                nameof(DefaultModelAlias), nameof(DefaultProviderId), nameof(DefaultModelId), nameof(DefaultTimeoutMs), nameof(DefaultBudgetUsd),
                nameof(ModelAliases),
                nameof(NodePresets),
            },
            TemplateRepositorySection => new[] { nameof(TemplateRepositoryBaseUrl) },
            AutomationSection => new[]
            {
                nameof(BudgetUsd), nameof(PreauthorizedUsd),
                nameof(WorkflowDefaultTimeoutMs), nameof(MaxLoopIterations), nameof(MaxToolRounds),
                nameof(CheckpointEnabled), nameof(RunEventRetentionDays), nameof(ConfirmationPolicies),
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
            RetrievalSection => new[]
            {
                nameof(VectorEnabled), nameof(VectorBackend), nameof(VectorCollection),
                nameof(VectorDimensions), nameof(QdrantHost), nameof(QdrantPort),
                nameof(QdrantUseTls), nameof(QdrantAuthMode), nameof(QdrantApiKey),
                nameof(HasQdrantApiKey), nameof(QdrantDataDir),
                nameof(RerankerEnabled), nameof(ChunkSizeChars), nameof(ChunkOverlapChars),
            },
            GitSection => new[]
            {
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

    private bool CanTestProviderDraft() =>
        CanSave(ModelsSection)
        && SelectedProviderOption is not null
        && !IsLegacyOtherProvider;

    private void EnsureLegacyOtherProviderTypeOption(string providerType)
    {
        if (!string.Equals(providerType, "other", StringComparison.Ordinal)
            || ProviderTypeOptions.Any(option => string.Equals(option.Value, "other", StringComparison.Ordinal)))
        {
            return;
        }
        ProviderTypeOptions.Add(new SettingsValueOption(
            "other",
            _displayNames.Text("ui.settings.models.provider_type.other")));
    }

    private void NotifyProviderCommands()
    {
        RefreshModelsCommand?.NotifyCanExecuteChanged();
        TestProviderDraftCommand?.NotifyCanExecuteChanged();
        SaveProviderKeyCommand?.NotifyCanExecuteChanged();
        RevokeProviderKeyCommand?.NotifyCanExecuteChanged();
        RemoveProviderCommand?.NotifyCanExecuteChanged();
    }

    private void UpdateDirtyState() => UpdateDirtyState(updateStatus: true);

    private void UpdateDirtyState(bool updateStatus)
    {
        RefreshPermissionProfile();
        var current = CurrentValues();
        HasUnsavedChanges = _draftState.IsDirty(current);
        RestoreCurrentTabCommand?.NotifyCanExecuteChanged();
        SaveCurrentTabCommand?.NotifyCanExecuteChanged();
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
        AddIfDirty(RetrievalSection, RetrievalTitle);
        AddIfDirty(GitSection, VersionControlTitle);
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
        RecoveryText = _displayNames.Text("ui.settings.recovery.edit_field");
        if (exception.FieldKey == "ui.settings.misc.qdrant_api_key")
        {
            HasQdrantApiKeyError = true;
        }
        RequestValidationFieldFocus(exception.FieldKey, exception.FocusItem, null);
    }

    internal void ReportValidationForTests(SettingsInputException exception) =>
        SetValidationStatus(exception);

    internal void ReportBackendFailureForTests(BackendException exception, string section) =>
        HandleSettingsFailure(exception, section);

    private void HandleSettingsFailure(Exception exception, string fallbackSection)
    {
        StatusText = UserFacingError.Format(exception, _displayNames);
        RecoveryText = string.Empty;
        if (exception is not BackendException backend)
        {
            return;
        }
        if (!string.IsNullOrWhiteSpace(backend.RecoveryAction))
        {
            var recoveryKey = $"ui.settings.recovery.{backend.RecoveryAction}";
            var localized = _displayNames.Text(recoveryKey);
            RecoveryText = localized.StartsWith('[') ? string.Empty : localized;
        }
        if (!string.IsNullOrWhiteSpace(backend.Field))
        {
            if (backend.Field == "qdrant_api_key")
            {
                HasQdrantApiKeyError = true;
            }
            RequestValidationFieldFocus(
                BackendFieldDisplayKey(backend.Field),
                null,
                backend.Section ?? fallbackSection);
        }
    }

    private string BackendFieldDisplayKey(string field) => field switch
    {
        "vector_backend" => "ui.settings.misc.vector_backend",
        "qdrant_host" => "ui.settings.misc.qdrant_host",
        "qdrant_port" => "ui.settings.misc.qdrant_port",
        "qdrant_auth_mode" => "ui.settings.misc.qdrant_auth",
        "qdrant_api_key" => "ui.settings.misc.qdrant_api_key",
        _ => field.StartsWith("ui.", StringComparison.Ordinal)
            ? field
            : $"ui.settings.field.{field}",
    };

    private void RequestValidationFieldFocus(
        string fieldKey,
        object? focusItem,
        string? sectionHint)
    {
        var sectionId = ValidationSectionId(fieldKey, sectionHint);
        var section = SectionIndexItems.FirstOrDefault(item =>
            string.Equals(item.Id, sectionId, StringComparison.Ordinal));
        if (section is not null)
        {
            var tab = Tabs.First(item => string.Equals(item.Id, section.TabId, StringComparison.Ordinal));
            CommitNavigation(new PendingSettingsNavigation(tab, section));
        }

        if (sectionId == "confirmations")
        {
            AreAdvancedConfirmationPoliciesExpanded = true;
            if (focusItem is ConfirmationPolicyViewModel confirmation)
            {
                confirmation.IsApprovalPromptExpanded = true;
            }
        }
        else if (sectionId == "paths")
        {
            AreAdvancedPermissionsExpanded = true;
        }
        else if (sectionId == "retrieval"
                 && fieldKey is "ui.settings.misc.vector_dimensions"
                     or "ui.settings.misc.vector_collection"
                     or "ui.settings.misc.chunk_size"
                     or "ui.settings.misc.chunk_overlap"
                     or "ui.settings.misc.qdrant_data_dir")
        {
            AreAdvancedRetrievalSettingsExpanded = true;
        }
        else if (sectionId == "app_runtime")
        {
            AreAdvancedAppRuntimeSettingsExpanded = true;
        }

        var accessibleName = focusItem is ProviderModelEditorRow row
            ? row.HasModelIdError
                ? ModelIdColumnLabel
                : row.HasCapabilityError
                    ? ModelCapabilityColumnLabel
                    : row.HasMaxContextTokensError
                        ? ModelContextColumnLabel
                        : row.HasInputCostError
                            ? ModelInputCostColumnLabel
                            : ModelOutputCostColumnLabel
            : _displayNames.Text(fieldKey);
        FocusValidationFieldRequested?.Invoke(
            this,
            new SettingsFieldFocusRequest(accessibleName, focusItem));
    }

    private static string ValidationSectionId(string fieldKey, string? sectionHint)
    {
        if (fieldKey.StartsWith("ui.settings.models", StringComparison.Ordinal)) return "available_models";
        if (fieldKey.StartsWith("ui.settings.presets.alias", StringComparison.Ordinal)) return "model_aliases";
        if (fieldKey.Contains("node_", StringComparison.Ordinal)) return "node_presets";
        if (fieldKey.StartsWith("ui.settings.presets.default", StringComparison.Ordinal)) return "defaults";
        if (fieldKey is "ui.settings.automation.global_budget" or "ui.settings.automation.preauthorized_budget") return "budget";
        if (fieldKey.StartsWith("ui.settings.automation.confirmation", StringComparison.Ordinal)) return "confirmations";
        if (fieldKey.StartsWith("ui.settings.automation", StringComparison.Ordinal)) return "runtime";
        if (fieldKey.StartsWith("ui.settings.permissions", StringComparison.Ordinal)) return "paths";
        if (fieldKey is "ui.settings.misc.ignored_paths") return "git";
        if (fieldKey is "ui.settings.misc.qdrant_binary_path" or "ui.settings.misc.qdrant_startup_timeout") return "app_runtime";
        if (fieldKey.StartsWith("ui.settings.misc.", StringComparison.Ordinal)) return "retrieval";
        return sectionHint switch
        {
            ModelsSection => "provider",
            PresetsSection or TemplateRepositorySection => "defaults",
            AutomationSection => "runtime",
            PermissionsSection => "paths",
            AppRuntimeSection => "app_runtime",
            RetrievalSection => "retrieval",
            GitSection => "git",
            _ => "project",
        };
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
        OnPropertyChanged(nameof(IsRetrievalEditable));
        OnPropertyChanged(nameof(IsGitEditable));
        NotifySaveCommands();
    }

    private void NotifySaveCommands()
    {
        SaveGeneralCommand?.NotifyCanExecuteChanged();
        RefreshModelsCommand?.NotifyCanExecuteChanged();
        TestProviderDraftCommand?.NotifyCanExecuteChanged();
        SaveModelCommand?.NotifyCanExecuteChanged();
        SaveProviderKeyCommand?.NotifyCanExecuteChanged();
        RevokeProviderKeyCommand?.NotifyCanExecuteChanged();
        RemoveProviderCommand?.NotifyCanExecuteChanged();
        AddProviderCommand?.NotifyCanExecuteChanged();
        AddProviderModelCommand?.NotifyCanExecuteChanged();
        SavePresetsCommand?.NotifyCanExecuteChanged();
        SaveTemplateRepositoryCommand?.NotifyCanExecuteChanged();
        RestoreRecommendedDefaultsCommand?.NotifyCanExecuteChanged();
        RestoreOfficialTemplateRepositoryCommand?.NotifyCanExecuteChanged();
        SaveAutomationCommand?.NotifyCanExecuteChanged();
        SavePermissionsCommand?.NotifyCanExecuteChanged();
        SavePersonalizationCommand?.NotifyCanExecuteChanged();
        SaveAppRuntimeCommand?.NotifyCanExecuteChanged();
        SaveRetrievalCommand?.NotifyCanExecuteChanged();
        SaveGitCommand?.NotifyCanExecuteChanged();
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
            or nameof(SelectedDefaultLlmRoute) or nameof(SelectedDefaultEmbeddingRoute)
            or nameof(SelectedDefaultRerankerRoute) or nameof(SelectedDefaultSearchRoute)
            or nameof(DefaultModelAlias) or nameof(DefaultProviderId) or nameof(DefaultModelId)
            or nameof(DefaultTimeoutMs) or nameof(DefaultBudgetUsd) or nameof(TemplateRepositoryBaseUrl)
            or nameof(BudgetUsd) or nameof(PreauthorizedUsd)
            or nameof(WorkflowDefaultTimeoutMs) or nameof(MaxLoopIterations) or nameof(MaxToolRounds)
            or nameof(CheckpointEnabled) or nameof(RunEventRetentionDays) or nameof(AllowNetwork)
            or nameof(AllowWebSearch) or nameof(AllowHttpSkill) or nameof(AllowWasmNetwork)
            or nameof(AllowSecretRead) or nameof(ReadableRootsText) or nameof(WritableRootsText)
            or nameof(Theme) or nameof(ThemeMainColor) or nameof(ThemeSurfaceColor) or nameof(ThemeBrandColor)
            or nameof(ThemeMainColorDark) or nameof(ThemeSurfaceColorDark) or nameof(ThemeBrandColorDark)
            or nameof(ThemeFollowSystemColors)
            or nameof(GitAutoColor) or nameof(GitManualColor)
            or nameof(ProjectPanelVisible) or nameof(ReduceMotion) or nameof(SelectedLanguage)
            or nameof(VectorEnabled)
            or nameof(VectorBackend) or nameof(VectorCollection) or nameof(VectorDimensions)
            or nameof(QdrantHost) or nameof(QdrantPort) or nameof(QdrantUseTls)
            or nameof(QdrantAuthMode) or nameof(QdrantApiKey) or nameof(QdrantDataDir)
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

    /// <summary>
    /// U110：有效状态 = tool_controls &amp;&amp; policy（与后端 `workflow_web_search_tool_enabled`
    /// 的判定公式一致）。当前只有 web-search 工具在 policy 层有对应的布尔门，
    /// 其余工具动作（find/search/register/write）没有并联的 policy 判定，不叠加。
    /// </summary>
    public void ApplyPolicyGate(bool allowedByPolicy, string? hintWhenBlocked)
    {
        foreach (var item in Controls)
        {
            if (IsWebSearchToolId(item.ToolId))
            {
                item.ApplyPolicyGate(allowedByPolicy, hintWhenBlocked);
            }
        }
    }

    private static bool IsWebSearchToolId(string toolId) =>
        string.Equals(toolId, "web-search", StringComparison.Ordinal)
        || toolId.EndsWith("-web-search", StringComparison.Ordinal);
}

public sealed class ToolControlItemViewModel : ViewModelBase
{
    private readonly Action _markDirty;
    private string _displayName;
    private bool? _isEnabled;
    private bool _isBlockedByPolicy;
    private string? _policyBlockedHint;

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

    /// <summary>
    /// U110：该工具在 tool_controls 里可能显示为「开」，但硬权限 policy（如 web-search
    /// 需要 allow_network &amp;&amp; allow_web_search）否决了它，实际不生效。当前只对
    /// web-search 工具计算这一层（唯一有对应布尔 policy 门的工具动作），见
    /// <see cref="ToolControlGroupViewModel.ApplyPolicyGate"/>。
    /// </summary>
    public bool IsBlockedByPolicy { get => _isBlockedByPolicy; private set => SetProperty(ref _isBlockedByPolicy, value); }

    /// <summary>被否决时展示给用户的提示；未被否决时为 null，ToolTip 不显示。</summary>
    public string? PolicyBlockedHint { get => _policyBlockedHint; private set => SetProperty(ref _policyBlockedHint, value); }

    /// <summary>由所属分组按 policy 判定结果调用；开关本身仍可交互，只叠加提示。</summary>
    public void ApplyPolicyGate(bool allowedByPolicy, string? hintWhenBlocked)
    {
        IsBlockedByPolicy = !allowedByPolicy;
        PolicyBlockedHint = IsBlockedByPolicy ? hintWhenBlocked : null;
    }

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
    private readonly Func<Action<string>, Task>? _browse;
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
        Action markDirty,
        Func<Action<string>, Task>? browse = null,
        DisplayNameService? displayNames = null)
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
        _browse = browse;
        BrowseReadableRootsCommand = new RelayCommand(
            () => _ = BrowseRootAsync(writable: false),
            () => _browse is not null);
        BrowseWritableRootsCommand = new RelayCommand(
            () => _ = BrowseRootAsync(writable: true),
            () => _browse is not null);
        // U146：chip 列表是**投影**，读写都经由上面那两个字符串属性，
        // 所以 SetAndMark 的脏标记、Snapshot、ToPolicy 全部照旧生效——
        // 这是选投影而非换数据结构的核心收益。
        // displayNames 为 null 时退回全局实例：这个 VM 有多个既有构造点
        // （NodeTypePresetViewModel 里也建），不该为了 chip 化去改它们的签名。
        var names = displayNames ?? DisplayNameService.Current;
        ReadableRootChips = new PathChipListViewModel(
            names,
            () => ReadableRootsText,
            value => ReadableRootsText = value,
            requireAbsolute: true,
            probeExistence: true,
            browse);
        WritableRootChips = new PathChipListViewModel(
            names,
            () => WritableRootsText,
            value => WritableRootsText = value,
            requireAbsolute: true,
            probeExistence: true,
            browse);
    }

    public string Scope { get; }
    public string DisplayName { get => _displayName; set => SetProperty(ref _displayName, value); }
    public RelayCommand BrowseReadableRootsCommand { get; }
    public RelayCommand BrowseWritableRootsCommand { get; }

    /// <summary>U146：可读根的 chip 投影。</summary>
    public PathChipListViewModel ReadableRootChips { get; }

    /// <summary>U146：可写根的 chip 投影。</summary>
    public PathChipListViewModel WritableRootChips { get; }
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

    // U146：字符串仍是唯一真源，chip 只是投影。任何写入这两个属性的旧路径
    // （继承投影、推荐默认值、目录选择器、测试直接赋值）都会把 chip 拉回一致；
    // 反过来 chip 增删也是经由这里落地，故脏标记只需在这一处维持。
    // Sync 内部做「投影结果相同就早退」，所以 chip 触发的回写不会递归重建集合。
    public string ReadableRootsText
    {
        get => _readableRootsText;
        set
        {
            if (SetAndMark(ref _readableRootsText, value))
            {
                ReadableRootChips.Sync();
            }
        }
    }

    public string WritableRootsText
    {
        get => _writableRootsText;
        set
        {
            if (SetAndMark(ref _writableRootsText, value))
            {
                WritableRootChips.Sync();
            }
        }
    }

    public PermissionPolicy ToPolicy() => new(
        AllowNetwork,
        AllowWebSearch,
        AllowHttpSkill,
        AllowWasmNetwork,
        AllowSecretRead,
        SettingsInputValidation.AbsolutePaths(
            WritableRootsText,
            "ui.settings.permissions.write_roots"),
        SettingsInputValidation.AbsolutePaths(
            ReadableRootsText,
            "ui.settings.permissions.read_roots"));

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
        // U146：这里刻意走 backing field（避免与 InheritGlobal setter 抢），
        // 于是绕过了属性 setter 里的 Sync——必须在此手工补一次，
        // 否则「继承父级」改了字符串而 chip 还显示旧路径，界面与将要保存的值不一致。
        ReadableRootChips.Sync();
        WritableRootChips.Sync();
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

    private async Task BrowseRootAsync(bool writable)
    {
        if (_browse is null)
        {
            return;
        }

        await _browse(path =>
        {
            if (writable)
            {
                WritableRootsText = SettingsPageViewModel.AppendPathLine(WritableRootsText, path);
            }
            else
            {
                ReadableRootsText = SettingsPageViewModel.AppendPathLine(ReadableRootsText, path);
            }
        }).ConfigureAwait(true);
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
    private string _approvalPromptError = string.Empty;
    private bool _isApprovalPromptExpanded;

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
    public string NormalPolicySelection
    {
        get => NormalPolicy;
        set => NormalAllowByDefault = string.Equals(value, "allow_by_default", StringComparison.Ordinal);
    }
    public string AutoModePolicySelection
    {
        get => AutoModePolicy;
        set => AutoModeAutoApproval = string.Equals(value, "auto_approval", StringComparison.Ordinal);
    }
    public string ApprovalPrompt
    {
        get => _approvalPrompt;
        set
        {
            if (SetProperty(ref _approvalPrompt, value ?? string.Empty))
            {
                SetApprovalPromptError(string.Empty);
                _markDirty();
            }
        }
    }
    public string ApprovalPromptError
    {
        get => _approvalPromptError;
        private set
        {
            if (SetProperty(ref _approvalPromptError, value))
            {
                OnPropertyChanged(nameof(HasApprovalPromptError));
            }
        }
    }
    public bool HasApprovalPromptError => !string.IsNullOrWhiteSpace(ApprovalPromptError);

    internal void SetApprovalPromptError(string value) =>
        ApprovalPromptError = value ?? string.Empty;
    public bool IsApprovalPromptExpanded
    {
        get => _isApprovalPromptExpanded;
        set => SetProperty(ref _isApprovalPromptExpanded, value);
    }

    public bool NormalAllowByDefault
    {
        get => _normalAllowByDefault;
        set
        {
            if (SetProperty(ref _normalAllowByDefault, value))
            {
                OnPropertyChanged(nameof(NormalPolicy));
                OnPropertyChanged(nameof(NormalPolicySelection));
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
                OnPropertyChanged(nameof(AutoModePolicySelection));
                _markDirty();
            }
        }
    }
}

public sealed class SettingsDiagnosticItemViewModel
{
    public SettingsDiagnosticItemViewModel(
        string component,
        string status,
        string reason,
        string recoveryAction)
    {
        Component = component;
        Status = status;
        Reason = reason;
        RecoveryAction = recoveryAction;
    }

    public string Component { get; }
    public string Status { get; }
    public string Reason { get; }
    public string RecoveryAction { get; }
}

public sealed class SettingsSectionLoadFailureViewModel
{
    public SettingsSectionLoadFailureViewModel(
        string section,
        string title,
        string message,
        string retryText,
        Action retry)
    {
        Section = section;
        Title = title;
        Message = message;
        RetryText = retryText;
        RetryCommand = new RelayCommand(retry);
    }

    public string Section { get; }
    public string Title { get; }
    public string Message { get; }
    public string RetryText { get; }
    public RelayCommand RetryCommand { get; }
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

public sealed class SettingsFieldFocusRequest : EventArgs
{
    public SettingsFieldFocusRequest(string accessibleName, object? item)
    {
        AccessibleName = accessibleName;
        Item = item;
    }

    public string AccessibleName { get; }
    public object? Item { get; }
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

/// <summary>路径存在性体检结论。与 U143 的 <c>RecentProjectHealth</c> 同族语义。</summary>
public enum PathChipHealth
{
    /// <summary>尚未体检。体检是磁盘 IO，不能在 UI 线程同步跑，所以必然有这个中间态。</summary>
    Unknown,

    /// <summary>目录存在。</summary>
    Healthy,

    /// <summary>目录不存在（被删或被移走）。</summary>
    Missing,

    /// <summary>路径存在但是文件而非目录——「可读根」必须是目录，指到文件上等于配错。</summary>
    NotADirectory,
}

/// <summary>
/// U146：路径列表里的单条 chip。
///
/// 原先整个列表塞在一个 <c>AcceptsReturn</c> 的 <c>TextBox</c> 里，三条后果：
/// 单条无法校验（只能整体接受或拒绝，用户不知道哪行错）、看不出哪条已失效、
/// 首尾空格静默出错（<c>" /home/x"</c> 与 <c>"/home/x"</c> 在文本里长得一样，
/// 作为路径却是两个值）。这些是**权限边界配置**，静默出错等于权限判定与用户意图不符。
///
/// 每条独立成 chip 后，失效状态才有地方显示——与 U143「路径类数据必须能显示自身健康度」同源。
/// </summary>
public sealed class PathChipViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private PathChipHealth _health = PathChipHealth.Unknown;

    public PathChipViewModel(
        string path,
        DisplayNameService displayNames,
        Action<PathChipViewModel> remove)
    {
        // Path 一律是已规范化的值：调用方在加入前必须过 TryNormalizePath，
        // 所以这里不再 Trim——若还需要 Trim，说明有旁路绕过了规范化收口。
        PathText = path;
        _displayNames = displayNames;
        RemoveCommand = new RelayCommand(() => remove(this));
    }

    /// <summary>
    /// 规范化后的路径值。这就是序列化出去的内容，chip 上显示的也是它。
    /// 叫 PathText 而不是 Path：<c>{Binding Path}</c> 在 XAML 里与
    /// <c>Binding.Path</c> 以及 <c>&lt;Path&gt;</c> 图形元素同名，是编译绑定的歧义源。
    /// </summary>
    public string PathText { get; }

    /// <summary>移除本条。chip 自己持有删除命令，列表模板里不必再靠索引回查。</summary>
    public RelayCommand RemoveCommand { get; }

    /// <summary>
    /// 体检结论。<see cref="PathChipHealth.Unknown"/> 期间**不显示失效标记**——
    /// 体检未完成就先标红会让每次打开设置页都闪一片告警色，用户会学会忽略它。
    /// </summary>
    public PathChipHealth Health
    {
        get => _health;
        private set
        {
            if (SetProperty(ref _health, value))
            {
                OnPropertyChanged(nameof(IsUnavailable));
                OnPropertyChanged(nameof(UnavailableText));
                OnPropertyChanged(nameof(HasUnavailableText));
            }
        }
    }

    /// <summary>是否该置灰/标告警色。</summary>
    public bool IsUnavailable =>
        Health is PathChipHealth.Missing or PathChipHealth.NotADirectory;

    /// <summary>
    /// 失效说明文字。**只灰掉不给字是不够的**——用户看到一条灰 chip 无从知道
    /// 是「目录被删了」还是「我指到文件上了」，两者的出路完全不同。
    /// </summary>
    public string UnavailableText => Health switch
    {
        PathChipHealth.Missing => _displayNames.Text("ui.settings.permissions.path_chip.missing"),
        PathChipHealth.NotADirectory => _displayNames.Text("ui.settings.permissions.path_chip.not_a_directory"),
        _ => string.Empty,
    };

    public bool HasUnavailableText => !string.IsNullOrEmpty(UnavailableText);

    /// <summary>无障碍名：路径 + 失效说明，读屏用户不靠颜色也能知道这条坏了。</summary>
    public string AccessibleName => HasUnavailableText
        ? $"{PathText}（{UnavailableText}）"
        : PathText;

    /// <summary>切语言后刷新失效说明。</summary>
    internal void RefreshLocalizedText()
    {
        OnPropertyChanged(nameof(UnavailableText));
        OnPropertyChanged(nameof(HasUnavailableText));
        OnPropertyChanged(nameof(AccessibleName));
    }

    internal void ApplyHealth(PathChipHealth health)
    {
        Health = health;
        OnPropertyChanged(nameof(AccessibleName));
    }
}

/// <summary>
/// U146：路径列表的 chip 投影层。
///
/// **为什么是投影而不是彻底换掉数据结构**：底层仍以「换行分隔的字符串」作为序列化形态，
/// 原因是这个字符串同时被 7 类逻辑消费——保存序列化（<c>AbsolutePaths</c>/<c>RelativePaths</c>）、
/// 脏状态快照（<c>CurrentValues</c> 的字典值）、快照对比（<c>IsTrackedDirtyProperty</c>）、
/// 预设匹配（<c>PathLinesEqual</c>）、继承投影（<c>ApplyPolicy</c>）、
/// 推荐默认值（<c>ApplyRecommendedDefaults</c>）、权限档位判定（<c>MatchesPermissionProfile</c>）。
/// 换成集合要同时改这 7 处，且脏状态快照必须自己实现一套稳定序列化——
/// 漏一处就是「改了不算脏」或「保存丢内容」。
///
/// 投影方案把风险收在一处：chip 的任何改动都**经由宿主原有的字符串 setter** 落地，
/// 于是脏状态、快照、保存链路一行都不用动；宿主字符串被任何旧路径改写后，
/// <see cref="Sync"/> 把 chip 拉回一致。双向同步的唯一真源始终是那个字符串。
/// </summary>
public sealed class PathChipListViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly Func<string> _read;
    private readonly Action<string> _write;
    private readonly Func<Action<string>, Task>? _browse;
    private readonly bool _requireAbsolute;
    private readonly bool _probeExistence;
    private readonly Func<string, string>? _resolveForProbe;
    private readonly RequestGenerationSession _probeSession = new();
    private string _draftPath = string.Empty;
    private string _draftError = string.Empty;

    public PathChipListViewModel(
        DisplayNameService displayNames,
        Func<string> read,
        Action<string> write,
        bool requireAbsolute,
        bool probeExistence,
        Func<Action<string>, Task>? browse = null,
        Func<string, string>? resolveForProbe = null)
    {
        _displayNames = displayNames;
        _read = read;
        _write = write;
        _requireAbsolute = requireAbsolute;
        _probeExistence = probeExistence;
        _browse = browse;
        _resolveForProbe = resolveForProbe;
        Chips = new ObservableCollection<PathChipViewModel>();
        AddCommand = new RelayCommand(() => TryCommitDraft(), () => !string.IsNullOrWhiteSpace(DraftPath));
        BrowseCommand = new RelayCommand(() => _ = BrowseAsync(), () => _browse is not null);
        Sync();
    }

    public ObservableCollection<PathChipViewModel> Chips { get; }
    public RelayCommand AddCommand { get; }
    public RelayCommand BrowseCommand { get; }

    public bool HasChips => Chips.Count > 0;

    /// <summary>空列表要给字而不是留白：留白让人以为界面没加载完。</summary>
    public string EmptyText => _displayNames.Text("ui.settings.permissions.path_chip.empty");

    public string AddText => _displayNames.Text("ui.settings.permissions.path_chip.add");
    public string RemoveHintText => _displayNames.Text("ui.settings.permissions.path_chip.remove");

    /// <summary>目录选择器按钮文案。复用既有的「浏览…」，不新造一个同义词。</summary>
    public string BrowseText => _displayNames.Text("ui.settings.browse_folder");

    public string DraftPlaceholder => _requireAbsolute
        ? _displayNames.Text("ui.settings.permissions.path_chip.draft_placeholder_absolute")
        : _displayNames.Text("ui.settings.permissions.path_chip.draft_placeholder_relative");

    /// <summary>待添加的输入。留一个单行输入框，键盘用户不必依赖目录选择器。</summary>
    public string DraftPath
    {
        get => _draftPath;
        set
        {
            if (SetProperty(ref _draftPath, value))
            {
                // 用户开始重新输入就清掉上一条错误：错误常驻会盖住当前这次输入的真实状态。
                DraftError = string.Empty;
                AddCommand.NotifyCanExecuteChanged();
            }
        }
    }

    /// <summary>
    /// 逐条校验的错误文字。**这就是 U146 第一条后果的正解**——
    /// 原先整个文本框只能整体接受或拒绝，用户不知道是哪一行错；
    /// 现在错的那一条根本进不了列表，且当场说明原因。
    /// </summary>
    public string DraftError
    {
        get => _draftError;
        private set
        {
            if (SetProperty(ref _draftError, value))
            {
                OnPropertyChanged(nameof(HasDraftError));
            }
        }
    }

    public bool HasDraftError => !string.IsNullOrEmpty(DraftError);

    /// <summary>
    /// 把宿主字符串拉进 chip 集合。
    ///
    /// 只在**投影结果真的不同**时重建：宿主 setter 会因为无关改动反复触发 PropertyChanged，
    /// 每次都重建集合会让正在体检的结论全部作废、并让 UI 无谓地重排一遍 chip。
    /// </summary>
    public void Sync()
    {
        var desired = SplitStoredLines(_read());
        if (desired.Count == Chips.Count
            && desired.SequenceEqual(Chips.Select(chip => chip.PathText), StringComparer.Ordinal))
        {
            return;
        }

        Chips.Clear();
        foreach (var path in desired)
        {
            Chips.Add(new PathChipViewModel(path, _displayNames, Remove));
        }
        OnPropertyChanged(nameof(HasChips));
        _ = ProbeHealthAsync();
    }

    /// <summary>切语言后刷新自身与全部 chip 的文案。</summary>
    public void RefreshLocalizedText()
    {
        OnPropertyChanged(nameof(EmptyText));
        OnPropertyChanged(nameof(AddText));
        OnPropertyChanged(nameof(RemoveHintText));
        OnPropertyChanged(nameof(BrowseText));
        OnPropertyChanged(nameof(DraftPlaceholder));
        foreach (var chip in Chips)
        {
            chip.RefreshLocalizedText();
        }
    }

    /// <summary>提交输入框里那条。成功则清空输入框，失败则留着原文并给出原因。</summary>
    internal bool TryCommitDraft()
    {
        if (!TryAdd(DraftPath, out var failure))
        {
            DraftError = failure;
            return false;
        }

        DraftPath = string.Empty;
        DraftError = string.Empty;
        return true;
    }

    /// <summary>
    /// 加入一条路径。首尾空格在 <see cref="SettingsInputValidation.TryNormalizePath"/> 里就被吃掉——
    /// U146 的第三条后果（<c>" /home/x"</c> 与 <c>"/home/x"</c> 肉眼不可辨却是两个值）由此消除。
    /// </summary>
    internal bool TryAdd(string? raw, out string failure)
    {
        failure = string.Empty;
        if (string.IsNullOrWhiteSpace(raw))
        {
            // 空行不入列表：原先文本框里的空行会在保存时被静默丢掉，
            // 用户看到「我输的东西不见了」却查不出原因。
            failure = _displayNames.Text("ui.settings.permissions.path_chip.error_empty");
            return false;
        }

        if (!SettingsInputValidation.TryNormalizePath(raw, _requireAbsolute, out var normalized))
        {
            failure = _requireAbsolute
                ? _displayNames.Text("ui.settings.permissions.path_chip.error_absolute")
                : _displayNames.Text("ui.settings.permissions.path_chip.error_relative");
            return false;
        }

        // 重复项当场拒绝并说明。若放进去，保存时 Paths() 会因重复抛 PathLine 异常，
        // 那时错误信息只有行号——而 chip 列表里根本没有「行号」这个概念可指。
        if (Chips.Any(chip => SettingsInputValidation.PathComparer.Equals(chip.PathText, normalized)))
        {
            failure = _displayNames.Text("ui.settings.permissions.path_chip.error_duplicate");
            return false;
        }

        Chips.Add(new PathChipViewModel(normalized, _displayNames, Remove));
        Commit();
        return true;
    }

    internal void Remove(PathChipViewModel chip)
    {
        if (Chips.Remove(chip))
        {
            Commit();
        }
    }

    /// <summary>chip 集合 → 宿主字符串。经由宿主原有 setter，脏状态与快照链路照旧生效。</summary>
    private void Commit()
    {
        _write(string.Join(Environment.NewLine, Chips.Select(chip => chip.PathText)));
        OnPropertyChanged(nameof(HasChips));
        _ = ProbeHealthAsync();
    }

    private async Task BrowseAsync()
    {
        if (_browse is null)
        {
            return;
        }

        await _browse(path =>
        {
            if (!TryAdd(path, out var failure))
            {
                // 选择器给回的路径也可能不合法（例如相对根要求的字段选到了项目外），
                // 同样要给字——只把它悄悄丢掉会让用户以为选择器坏了。
                DraftError = failure;
            }
        }).ConfigureAwait(true);
    }

    /// <summary>
    /// 逐条体检存在性。
    ///
    /// **不在 UI 线程同步跑**：失效条目往往落在已卸载的外部盘或网络路径上，
    /// 那里的 <c>Directory.Exists</c> 可能阻塞到超时，一列路径顺序探完足以卡住设置页。
    /// 整批丢进 <c>Task.Run</c>，只把结论切回 UI 线程回填（<c>ConfigureAwait(true)</c>——
    /// <c>Health</c> 会触发 PropertyChanged，绑定必须在 UI 线程收到）。
    ///
    /// 代次守卫：体检期间用户可能已经增删过 chip，过期结论不能盖到新集合上。
    /// </summary>
    private async Task ProbeHealthAsync()
    {
        if (!_probeExistence)
        {
            return;
        }

        var request = _probeSession.Begin();
        var targets = Chips.ToArray();
        if (targets.Length == 0)
        {
            return;
        }

        var probePaths = targets
            .Select(chip => _resolveForProbe is null ? chip.PathText : _resolveForProbe(chip.PathText))
            .ToArray();
        PathChipHealth[] results;
        try
        {
            results = await Task.Run(
                () => probePaths.Select(Inspect).ToArray(),
                request.CancellationToken).ConfigureAwait(true);
        }
        catch (OperationCanceledException)
        {
            return;
        }

        if (!_probeSession.IsCurrent(request))
        {
            return;
        }

        for (var i = 0; i < targets.Length && i < results.Length; i++)
        {
            // 集合可能已被替换（Sync 重建过）；只回填仍在列表里的那些实例。
            if (Chips.Contains(targets[i]))
            {
                targets[i].ApplyHealth(results[i]);
            }
        }
    }

    /// <summary>
    /// 单条体检。区分「不存在」与「存在但是文件」——两者出路不同：
    /// 前者要重新指路，后者是配错了层级。
    /// </summary>
    private static PathChipHealth Inspect(string path)
    {
        if (string.IsNullOrWhiteSpace(path))
        {
            return PathChipHealth.Missing;
        }

        try
        {
            if (Directory.Exists(path))
            {
                return PathChipHealth.Healthy;
            }

            return File.Exists(path) ? PathChipHealth.NotADirectory : PathChipHealth.Missing;
        }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or ArgumentException)
        {
            // 权限不足、路径非法一律按不可用：让用户看到失效标记，
            // 好过给一个看着正常、实际权限判定用不上的条目。
            return PathChipHealth.Missing;
        }
    }

    /// <summary>
    /// 宿主字符串 → 去空行、去首尾空格、去重的有序列表。
    ///
    /// 刻意**不做合法性过滤**：历史配置里可能存着非法值（旧版本写进去的相对路径等），
    /// 过滤掉会让它在界面上消失、却仍在保存时报错，用户永远找不到那条。
    /// 让它作为 chip 显示出来、体检标失效，才有机会被删掉。
    /// </summary>
    private static List<string> SplitStoredLines(string? text)
    {
        var result = new List<string>();
        if (string.IsNullOrWhiteSpace(text))
        {
            return result;
        }

        var seen = new HashSet<string>(SettingsInputValidation.PathComparer);
        foreach (var line in text.Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n'))
        {
            var trimmed = line.Trim();
            if (!string.IsNullOrWhiteSpace(trimmed) && seen.Add(trimmed))
            {
                result.Add(trimmed);
            }
        }
        return result;
    }
}
