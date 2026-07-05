namespace Ariadne.Desktop.Backend;

public interface IAriadneBackendClient
{
    Task<T?> InvokeAsync<T>(
        string method,
        object? parameters = null,
        CancellationToken cancellationToken = default);

    Task<IReadOnlyList<RecentProjectEntry>> ListRecentProjectsAsync(CancellationToken cancellationToken = default);

    Task<AppStatus?> GetAppStatusAsync(CancellationToken cancellationToken = default);

    Task<CurrentProjectStatus?> GetCurrentProjectAsync(CancellationToken cancellationToken = default);

    Task<ProjectInitReport> CreateProjectAsync(string projectRoot, string? name = null, CancellationToken cancellationToken = default);

    Task<IReadOnlyList<RecentProjectEntry>> OpenProjectAsync(string projectRoot, string? name = null, CancellationToken cancellationToken = default);

    Task<AppSettings> GetAppSettingsAsync(CancellationToken cancellationToken = default);

    Task<AppSettings> SaveAppSettingsAsync(AppSettings settings, CancellationToken cancellationToken = default);

    Task<ProviderConfigStatus> GetProviderConfigAsync(CancellationToken cancellationToken = default);

    Task<ProviderConfigStatus> SaveProviderSettingsAsync(ProviderSettingsUpdate update, CancellationToken cancellationToken = default);

    Task<ProviderModelsResult> FetchProviderModelsAsync(string? providerId = null, CancellationToken cancellationToken = default);

    Task SaveProviderKeyAsync(string provider, string key, CancellationToken cancellationToken = default);

    Task<NodePresetSettings> GetNodePresetSettingsAsync(CancellationToken cancellationToken = default);

    Task<NodePresetSettings> SaveNodePresetSettingsAsync(NodePresetSettings settings, CancellationToken cancellationToken = default);

    Task<AutomationSettings> GetAutomationSettingsAsync(CancellationToken cancellationToken = default);

    Task<AutomationSettings> SaveAutomationSettingsAsync(AutomationSettings settings, CancellationToken cancellationToken = default);

    Task<PermissionsSettings> GetPermissionsSettingsAsync(CancellationToken cancellationToken = default);

    Task<PermissionsSettings> SavePermissionsSettingsAsync(PermissionsSettings settings, CancellationToken cancellationToken = default);

    Task<UiPreferences> GetUiPreferencesAsync(CancellationToken cancellationToken = default);

    Task SaveUiPreferencesAsync(UiPreferences preferences, CancellationToken cancellationToken = default);

    Task<TemplateRepositorySettings> GetTemplateRepositorySettingsAsync(CancellationToken cancellationToken = default);

    Task<TemplateRepositorySettings> SaveTemplateRepositorySettingsAsync(TemplateRepositorySettings settings, CancellationToken cancellationToken = default);

    Task<WorkflowSettings> GetWorkflowSettingsAsync(CancellationToken cancellationToken = default);

    Task<WorkflowSettings> SaveWorkflowSettingsAsync(WorkflowSettings settings, CancellationToken cancellationToken = default);

    Task<GitSettings> GetGitSettingsAsync(CancellationToken cancellationToken = default);

    Task<GitSettings> SaveGitSettingsAsync(GitSettings settings, CancellationToken cancellationToken = default);

    Task<RagSettings> GetRagSettingsAsync(CancellationToken cancellationToken = default);

    Task<RagSettings> SaveRagSettingsAsync(RagSettings settings, CancellationToken cancellationToken = default);

    Task<IReadOnlyList<TemplateSummary>> SearchTemplatesAsync(string baseUrl, string query, int page = 0, CancellationToken cancellationToken = default);

    Task<TemplateInstallReport> InstallTemplateAsync(string baseUrl, string id, CancellationToken cancellationToken = default);

    Task<WorkflowRunStarted> RunWorkflowAsync(string workflowId, string? startNodeId = null, CancellationToken cancellationToken = default);

    Task<WorkflowGraphData> LoadWorkflowGraphAsync(string? workflowId = null, CancellationToken cancellationToken = default);

    Task SaveWorkflowGraphAsync(WorkflowGraphData graphData, CancellationToken cancellationToken = default);

    Task ValidateWorkflowGraphAsync(WorkflowGraphData graphData, CancellationToken cancellationToken = default);

    Task<WorkflowGraphData> ExportWorkflowSelectionAsync(string workflowId, IReadOnlyList<string> selectedNodeIds, CancellationToken cancellationToken = default);

    Task SaveDocumentContentAsync(string documentId, string content, CancellationToken cancellationToken = default);

    Task<string> GetDocumentContentAsync(string documentId, CancellationToken cancellationToken = default);

    Task<ArchivePoint> CreateCheckpointAsync(string message, CancellationToken cancellationToken = default);

    Task<IReadOnlyList<UiRunLogEntry>> QueryRunLogsAsync(string? level = null, string? query = null, CancellationToken cancellationToken = default);
}
