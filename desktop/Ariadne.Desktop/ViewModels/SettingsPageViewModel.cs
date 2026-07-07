using System.Collections.ObjectModel;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class SettingsPageViewModel : ViewModelBase, IUnsavedChangesGuard
{
    private const char SnapshotSeparator = '\u001f';
    private const int SnapshotLocaleIndex = 1;
    private const int SnapshotOnboardingSeenIndex = 44;
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
        nameof(SavePresetsText),
        nameof(SaveTemplateRepositoryText),
        nameof(BudgetLabel),
        nameof(PreauthorizedBudgetLabel),
        nameof(AutoModeLabel),
        nameof(SpentLabel),
        nameof(NormalModeLabel),
        nameof(AutoModePolicyLabel),
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
        nameof(ReadableRootsLabel),
        nameof(WritableRootsLabel),
        nameof(PathPlaceholder),
        nameof(SavePermissionsText),
        nameof(ThemeLabel),
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
    private SettingsTabViewModel _selectedTab;
    private SettingsSectionIndexItemViewModel? _selectedSection;
    private string _selectedSectionId = string.Empty;
    private string _selectedLanguage;
    private string _statusText;
    private bool _isLoading;
    private bool _hasUnsavedChanges;
    private bool _suppressDirtyTracking;
    private string _savedSnapshot = string.Empty;

    private int _schemaVersion = 1;
    private string _projectName = string.Empty;
    private string _locale = string.Empty;
    private string _documentsDir = string.Empty;
    private string _workflowsDir = string.Empty;
    private string _skillsDir = string.Empty;
    private string _exportsDir = string.Empty;

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

    public SettingsPageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        _selectedLanguage = displayNames.CurrentLanguage;
        _statusText = displayNames.Text("ui.common.loading");

        LanguageOptions = new ObservableCollection<LanguageOption>
        {
            new("zh", displayNames.Text("ui.settings.misc.language.zh")),
            new("en", displayNames.Text("ui.settings.misc.language.en")),
            new("ja", displayNames.Text("ui.settings.misc.language.ja")),
        };

        ProviderTypeOptions = new ObservableCollection<string>
        {
            "open_ai", "anthropic", "gemini", "open_ai_compatible", "local", "other",
        };

        ThemeOptions = new ObservableCollection<string> { "system", "light", "dark" };
        ConfirmationPolicies = new ObservableCollection<ConfirmationPolicyViewModel>();
        NodePresets = new ObservableCollection<NodeTypePresetViewModel>();
        AvailableModels = new ObservableCollection<ModelOptionViewModel>();
        ToolControlGroups = new ObservableCollection<ToolControlGroupViewModel>();
        SectionIndexItems = new ObservableCollection<SettingsSectionIndexItemViewModel>();

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
        SavePresetsCommand = new RelayCommand(() => _ = SavePresetsAsync());
        SaveTemplateRepositoryCommand = new RelayCommand(() => _ = SaveTemplateRepositoryAsync());
        SaveAutomationCommand = new RelayCommand(() => _ = SaveAutomationAsync());
        SavePermissionsCommand = new RelayCommand(() => _ = SavePermissionsAsync());
        SavePersonalizationCommand = new RelayCommand(() => _ = SavePersonalizationAsync());
        SaveMiscCommand = new RelayCommand(() => _ = SaveMiscAsync());
        ResetOnboardingCommand = new RelayCommand(() => _ = ResetOnboardingAsync());

        _ = LoadAsync();
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
    public ObservableCollection<string> ThemeOptions { get; }
    public ObservableCollection<ConfirmationPolicyViewModel> ConfirmationPolicies { get; }
    public ObservableCollection<NodeTypePresetViewModel> NodePresets { get; }
    public ObservableCollection<ModelOptionViewModel> AvailableModels { get; }
    public ObservableCollection<ToolControlGroupViewModel> ToolControlGroups { get; }
    public ObservableCollection<SettingsSectionIndexItemViewModel> SectionIndexItems { get; }

    public RelayCommand SaveGeneralCommand { get; }
    public RelayCommand RefreshModelsCommand { get; }
    public RelayCommand SaveModelCommand { get; }
    public RelayCommand SaveProviderKeyCommand { get; }
    public RelayCommand SavePresetsCommand { get; }
    public RelayCommand SaveTemplateRepositoryCommand { get; }
    public RelayCommand SaveAutomationCommand { get; }
    public RelayCommand SavePermissionsCommand { get; }
    public RelayCommand SavePersonalizationCommand { get; }
    public RelayCommand SaveMiscCommand { get; }
    public RelayCommand ResetOnboardingCommand { get; }

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

    public string PresetNodeTypeLabel => _displayNames.Text("ui.settings.presets.node_type");
    public string PresetNodeModelLabel => _displayNames.Text("ui.settings.presets.node_model");
    public string PresetNodeTimeoutLabel => _displayNames.Text("ui.settings.presets.node_timeout_ms");
    public string PresetNodeBudgetLabel => _displayNames.Text("ui.settings.presets.node_budget_usd");
    public string DefaultModelLabel => _displayNames.Text("ui.settings.presets.default_model");
    public string DefaultTimeoutLabel => _displayNames.Text("ui.settings.presets.default_timeout_ms");
    public string DefaultBudgetLabel => _displayNames.Text("ui.settings.presets.default_budget_usd");
    public string TemplateRepositoryLabel => _displayNames.Text("ui.settings.presets.template_repository");
    public string SavePresetsText => _displayNames.Text("ui.settings.presets.save");
    public string SaveTemplateRepositoryText => _displayNames.Text("ui.settings.presets.save_template_repository");

    public string BudgetLabel => _displayNames.Text("ui.settings.automation.global_budget");
    public string PreauthorizedBudgetLabel => _displayNames.Text("ui.settings.automation.preauthorized_budget");
    public string AutoModeLabel => _displayNames.Text("ui.settings.automation.auto_mode");
    public string SpentLabel => _displayNames.Text("ui.settings.automation.spent").Replace("{spent}", SpentText);
    public string NormalModeLabel => _displayNames.Text("ui.settings.automation.confirmation.normal_mode");
    public string AutoModePolicyLabel => _displayNames.Text("ui.settings.automation.confirmation.auto_mode_policy");
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
    public string ReadableRootsLabel => _displayNames.Text("ui.settings.permissions.read_roots");
    public string WritableRootsLabel => _displayNames.Text("ui.settings.permissions.write_roots");
    public string PathPlaceholder => _displayNames.Text("ui.settings.permissions.path_placeholder");
    public string SavePermissionsText => _displayNames.Text("ui.settings.permissions.save");

    public string ThemeLabel => _displayNames.Text("ui.settings.personalization.theme");
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

    public string Theme { get => _theme; set => SetProperty(ref _theme, value); }
    public string GitAutoColor { get => _gitAutoColor; set => SetProperty(ref _gitAutoColor, value); }
    public string GitManualColor { get => _gitManualColor; set => SetProperty(ref _gitManualColor, value); }
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
            if (SetProperty(ref _selectedLanguage, value))
            {
                _displayNames.SwitchLanguage(value);
                RefreshLocalizedText();
                _ = PersistLanguageAsync(value);
            }
        }
    }

    private SettingsTabViewModel CreateTab(string id, string key) => new(id, _displayNames.Text(key), SelectTab);

    private void ApplySavedLanguage(string locale)
    {
        var language = DisplayNameService.NormalizeLanguageCode(locale);
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
            },
            "models" => new[]
            {
                ("provider", "ui.settings.section.provider"),
                ("available", "ui.settings.section.available_models"),
                ("embedding", "ui.settings.section.embedding"),
                ("manual", "ui.settings.section.manual_fallback"),
                ("secret", "ui.settings.section.secret"),
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

    private void SelectSection(SettingsSectionIndexItemViewModel section)
    {
        foreach (var item in SectionIndexItems)
        {
            item.IsSelected = item == section;
        }
        _selectedSection = section;
        _selectedSectionId = section.Id;
        OnSelectedSectionChanged();
    }

    private bool IsSectionSelected(string id) => _selectedSectionId == id;

    private void OnSelectedSectionChanged()
    {
        OnPropertyChanged(nameof(IsSectionProjectSelected));
        OnPropertyChanged(nameof(IsSectionDirectoriesSelected));
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
            Theme = _uiPreferences.Theme;
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
            var selected = _providerConfig.Providers.FirstOrDefault(p => p.Provider == ProviderId)
                ?? _providerConfig.Providers.FirstOrDefault();
            if (selected is not null)
            {
                ProviderId = selected.Provider;
                ProviderType = selected.ProviderType;
                ProviderDisplayName = selected.DisplayName;
                ProviderBaseUrl = selected.BaseUrl ?? string.Empty;
                ProviderEnabled = selected.Enabled;
                MakeDefaultLlm = _providerConfig.DefaultLlmProviderId == selected.Provider;
                MakeDefaultEmbedding = _providerConfig.DefaultEmbeddingProviderId == selected.Provider;
                MakeDefaultReranker = _providerConfig.DefaultRerankerProviderId == selected.Provider;
                ModelsText = string.Join(Environment.NewLine, selected.Models.Select(ModelLine));
                EmbeddingModelId = selected.Models.FirstOrDefault(IsEmbeddingModel)?.ModelId ?? string.Empty;
                AvailableModels.Clear();
                foreach (var model in selected.Models)
                {
                    AvailableModels.Add(new ModelOptionViewModel(model.ModelId, model.Capability));
                }
            }
            ProviderStatus = _providerConfig.Providers.Count == 0
                ? _displayNames.Text("ui.settings.models.no_provider_status")
                : string.Join(" / ", _providerConfig.Providers.Select(p => $"{p.DisplayName}:{(p.HasKey ? _displayNames.Text("ui.common.configured") : _displayNames.Text("ui.common.not_configured"))}"));
        }
        catch (Exception ex)
        {
            ProviderStatus = ex.Message;
        }
    }

    private async Task FetchModelsAsync()
    {
        IsLoading = true;
        try
        {
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
        });
    }

    private async Task SaveModelAsync()
    {
        await RunWithStatusAsync(async () =>
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
            await _backend.SaveProviderSettingsAsync(update).ConfigureAwait(true);
            if (!string.IsNullOrWhiteSpace(ApiKey))
            {
                await _backend.SaveProviderKeyAsync(ProviderId, ApiKey).ConfigureAwait(true);
                ApiKey = string.Empty;
            }
            await LoadProviderConfigAsync().ConfigureAwait(true);
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

    private async Task SaveAutomationAsync()
    {
        await RunWithStatusAsync(async () =>
        {
            var automation = new AutomationSettings(
                new BudgetStatus(ParseDouble(BudgetUsd, 0), ParseDouble(SpentText.TrimStart('$'), 0), ParseDouble(PreauthorizedUsd, 0), AutoModeEnabled),
                ConfirmationPolicies.Select(item => new ConfirmationPolicySetting(
                    item.Kind,
                    item.NormalPolicy,
                    item.AutoModePolicy,
                    string.Empty)).ToArray());
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
            var preferences = new UiPreferences(
                Theme,
                GitAutoColor,
                GitManualColor,
                ProjectPanelVisible,
                _uiPreferences?.ProjectPanelPosition,
                _uiPreferences?.PanelStates ?? new Dictionary<string, bool>(),
                OnboardingSeen);
            await _backend.SaveUiPreferencesAsync(preferences).ConfigureAwait(true);
            _uiPreferences = preferences;
        });
    }

    private async Task ResetOnboardingAsync()
    {
        IsLoading = true;
        var wasDirty = HasUnsavedChanges;
        try
        {
            OnboardingSeen = false;
            var preferences = new UiPreferences(
                Theme,
                GitAutoColor,
                GitManualColor,
                ProjectPanelVisible,
                _uiPreferences?.ProjectPanelPosition,
                _uiPreferences?.PanelStates ?? new Dictionary<string, bool>(),
                OnboardingSeen);
            await _backend.SaveUiPreferencesAsync(preferences).ConfigureAwait(true);
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
                    () => HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot));
            }
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

    private static int ParseInt(string text, int fallback) => int.TryParse(text, out var value) ? value : fallback;
    private static long ParseLong(string text, long fallback) => long.TryParse(text, out var value) ? value : fallback;
    private static double ParseDouble(string text, double fallback) => double.TryParse(text, out var value) ? value : fallback;

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
        var savedParts = _savedSnapshot.Split(SnapshotSeparator);
        var currentParts = CurrentSnapshot().Split(SnapshotSeparator);
        if (savedParts.Length == currentParts.Length && currentParts.Length > snapshotIndex)
        {
            savedParts[snapshotIndex] = currentParts[snapshotIndex];
            _savedSnapshot = string.Join(SnapshotSeparator, savedParts);
        }
        HasUnsavedChanges = CurrentSnapshot() != _savedSnapshot;
    }

    private void RefreshLocalizedText()
    {
        foreach (var propertyName in LocalizedPropertyNames)
        {
            OnPropertyChanged(propertyName);
        }

        foreach (var option in LanguageOptions)
        {
            option.Label = option.Code switch
            {
                "zh" => _displayNames.Text("ui.settings.misc.language.zh"),
                "en" => _displayNames.Text("ui.settings.misc.language.en"),
                "ja" => _displayNames.Text("ui.settings.misc.language.ja"),
                _ => option.Label,
            };
        }

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
            or nameof(SkillsDir) or nameof(ExportsDir) or nameof(ProviderId) or nameof(ProviderType)
            or nameof(ProviderDisplayName) or nameof(ProviderBaseUrl) or nameof(ProviderEnabled)
            or nameof(MakeDefaultLlm) or nameof(MakeDefaultEmbedding) or nameof(MakeDefaultReranker)
            or nameof(ModelsText) or nameof(EmbeddingModelId) or nameof(ManualModelsVisible) or nameof(ApiKey) or nameof(DefaultModelId)
            or nameof(DefaultTimeoutMs) or nameof(DefaultBudgetUsd) or nameof(TemplateRepositoryBaseUrl)
            or nameof(BudgetUsd) or nameof(PreauthorizedUsd) or nameof(AutoModeEnabled)
            or nameof(WorkflowDefaultTimeoutMs) or nameof(MaxLoopIterations) or nameof(MaxToolRounds)
            or nameof(CheckpointEnabled) or nameof(RuntimeAutosaveMs) or nameof(AllowNetwork)
            or nameof(AllowWebSearch) or nameof(AllowHttpSkill) or nameof(AllowWasmNetwork)
            or nameof(AllowSecretRead) or nameof(ReadableRootsText) or nameof(WritableRootsText)
            or nameof(Theme) or nameof(GitAutoColor) or nameof(GitManualColor)
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

public sealed class ToolControlGroupViewModel : ViewModelBase
{
    private string _displayName;

    public ToolControlGroupViewModel(string scope, string displayName)
    {
        Scope = scope;
        _displayName = displayName;
        Controls = new ObservableCollection<ToolControlItemViewModel>();
    }

    public string Scope { get; }
    public string DisplayName { get => _displayName; set => SetProperty(ref _displayName, value); }
    public ObservableCollection<ToolControlItemViewModel> Controls { get; }
}

public sealed class ToolControlItemViewModel : ViewModelBase
{
    private readonly Action _markDirty;
    private string _displayName;
    private bool _isEnabled;

    public ToolControlItemViewModel(string toolId, string displayName, bool isEnabled, Action markDirty)
    {
        ToolId = toolId;
        _displayName = displayName;
        _isEnabled = isEnabled;
        _markDirty = markDirty;
    }

    public string ToolId { get; }
    public string DisplayName { get => _displayName; set => SetProperty(ref _displayName, value); }

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
        _autoModeAutoApproval = string.IsNullOrWhiteSpace(autoModePolicy) || autoModePolicy == "auto_approval";
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
