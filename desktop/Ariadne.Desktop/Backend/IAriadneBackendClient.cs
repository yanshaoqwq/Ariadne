namespace Ariadne.Desktop.Backend;

public interface IAriadneBackendClient
{
    Task<T?> InvokeAsync<T>(
        string method,
        object? parameters = null,
        CancellationToken cancellationToken = default);

    Task<IReadOnlyList<RecentProjectEntry>> ListRecentProjectsAsync(CancellationToken cancellationToken = default);

    Task<AppStatus?> GetAppStatusAsync(CancellationToken cancellationToken = default);

    Task<SidebarBadgeCounts> GetSidebarBadgesAsync(CancellationToken cancellationToken = default);

    /// <summary>D3：查询项目维护状态；无维护时返回 null。</summary>
    Task<ProjectMaintenanceState?> GetProjectMaintenanceAsync(CancellationToken cancellationToken = default);

    Task<CurrentProjectStatus?> GetCurrentProjectAsync(CancellationToken cancellationToken = default);

    Task<ProjectInitReport> CreateProjectAsync(string projectRoot, string? name = null, CancellationToken cancellationToken = default);

    Task<CurrentProjectStatus> OpenProjectAsync(string projectRoot, string? name = null, CancellationToken cancellationToken = default);

    Task SetProjectRootAsync(string projectRoot, CancellationToken cancellationToken = default);

    void ClearProjectRoot();

    /// <summary>桌面侧是否已打开项目根（未打开时项目页应走空态，勿把 cwd 当项目）。</summary>
    bool HasProjectRoot { get; }

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

    Task<TemplateDetail> GetTemplateDetailAsync(string baseUrl, string id, CancellationToken cancellationToken = default);

    Task<TemplateInstallReport> InstallTemplateAsync(string baseUrl, string id, CancellationToken cancellationToken = default);

    Task<WorkflowRunStarted> RunWorkflowAsync(string workflowId, string? startNodeId = null, CancellationToken cancellationToken = default);

    Task<WorkflowActionResult> PauseWorkflowAsync(string workflowId, string runId, string? reason = null, CancellationToken cancellationToken = default);

    Task<WorkflowActionResult> StopWorkflowAsync(string workflowId, string runId, string? reason = null, CancellationToken cancellationToken = default);

    Task<WorkflowActionResult> ResumeWorkflowAsync(string workflowId, string runId, CancellationToken cancellationToken = default);

    Task<WorkflowRunState> GetWorkflowRunStateAsync(string workflowId, string runId, CancellationToken cancellationToken = default);

    Task<WorkflowEventsResult> GetWorkflowEventsAsync(string workflowId, string runId, long afterSequence = 0, int? limit = null, CancellationToken cancellationToken = default);

    Task<IReadOnlyList<WorkflowOperation>> ListInDoubtOperationsAsync(string workflowId, string runId, CancellationToken cancellationToken = default);

    Task<ResolveInDoubtOperationResult> ResolveInDoubtOperationAsync(string operationId, string decision, object? response = null, string? reason = null, CancellationToken cancellationToken = default);

    Task<ProjectAiResponse> ProjectAiChatAsync(string message, string? workflowIdToRun = null, CancellationToken cancellationToken = default);

    Task<ProjectAiResponse> ProjectAiChatAsync(
        string message,
        IReadOnlyList<ProjectAiChatMessage> chatHistory,
        string? workflowIdToRun = null,
        CancellationToken cancellationToken = default);

    Task<string> ReadProjectMemoryAsync(CancellationToken cancellationToken = default);

    Task<string> AppendProjectMemoryAsync(string content, CancellationToken cancellationToken = default);

    Task WriteProjectMemoryAsync(string content, CancellationToken cancellationToken = default);

    Task<ProjectReference> ResolveProjectReferenceAsync(string reference, CancellationToken cancellationToken = default);

    Task<IReadOnlyList<WorkflowSummary>> ListWorkflowGraphsAsync(CancellationToken cancellationToken = default);

    Task<WorkflowGraphData> LoadWorkflowGraphAsync(string? workflowId = null, CancellationToken cancellationToken = default);

    Task<WorkflowGraphData> SaveWorkflowGraphAsync(WorkflowGraphData graphData, CancellationToken cancellationToken = default);

    Task ValidateWorkflowGraphAsync(WorkflowGraphData graphData, CancellationToken cancellationToken = default);

    Task ApplyNodeDetailPatchAsync(string workflowId, NodeDetailPatch patch, CancellationToken cancellationToken = default);

    Task UpsertCanvasAnnotationAsync(string workflowId, CanvasAnnotation annotation, CancellationToken cancellationToken = default);

    Task SetNodeBreakpointAsync(string workflowId, string nodeId, bool enabled, CancellationToken cancellationToken = default);

    Task<WorkflowGraphData> ExportWorkflowSelectionAsync(string workflowId, IReadOnlyList<string> selectedNodeIds, CancellationToken cancellationToken = default);

    Task<WorkflowPackReport> PackWorkflowSelectionAsync(string workflowId, IReadOnlyList<string> selectedNodeIds, string? subworkflowNodeId = null, string? title = null, string? expectedRevision = null, string? operationId = null, CancellationToken cancellationToken = default);

    Task<WorkflowPackReport> GetPackOperationAsync(string operationId, CancellationToken cancellationToken = default);

    Task<WorksTreeNode> GetWorksTreeAsync(CancellationToken cancellationToken = default);

    Task<ChapterSummaryView> GetChapterSummaryViewAsync(string chapterId, CancellationToken cancellationToken = default);

    Task<DocumentTreeNode> GetDocumentTreeAsync(string? projectId = null, CancellationToken cancellationToken = default);

    Task<ChapterImportReport> ImportChapterAsync(ChapterImportRequest request, CancellationToken cancellationToken = default);

    Task<CombinedExportReport> ExportChaptersAsync(IReadOnlyList<string> selectedChapterIds, string artifactId, string format = "markdown", CancellationToken cancellationToken = default);

    Task<DocumentWriteReport> SaveDocumentContentAsync(string documentId, string content, string? baseVersion = null, CancellationToken cancellationToken = default);

    Task<string> GetDocumentContentAsync(string documentId, CancellationToken cancellationToken = default);

    Task<string> GetDocumentContentByPathAsync(string path, CancellationToken cancellationToken = default);

    Task<DocumentContentResult> GetDocumentContentDetailsAsync(string documentId, CancellationToken cancellationToken = default);

    Task<DocumentContentResult> GetDocumentContentDetailsByPathAsync(string path, CancellationToken cancellationToken = default);

    Task<QuickEditResult> QuickEditAsync(QuickEditRequest request, CancellationToken cancellationToken = default);

    Task<PatchApplyReport> ApplyQuickEditAsync(string documentId, string? baseVersion, string text, TextRange range, QuickEditResult result, CancellationToken cancellationToken = default);

    Task<ArchivePoint> CreateCheckpointAsync(string message, CancellationToken cancellationToken = default);

    Task<IReadOnlyList<GitCommitSummary>> GetGitHistoryAsync(CancellationToken cancellationToken = default);

    Task<GitRepositoryStatus> GetGitRepositoryStatusAsync(CancellationToken cancellationToken = default);

    Task<IReadOnlyList<BranchGraphNode>> GetGitBranchGraphAsync(int limit = 200, CancellationToken cancellationToken = default);

    Task<RestoreReport> RestoreToNewBranchAsync(string commitId, string newBranch, CancellationToken cancellationToken = default);

    Task<IReadOnlyList<ConfirmationLogEntry>> ListConfirmationsAsync(CancellationToken cancellationToken = default);

    Task<ConfirmationLogEntry> GetConfirmationAsync(string confirmationId, CancellationToken cancellationToken = default);

    Task<ResolveConfirmationResult> ResolveConfirmationAsync(string workflowId, string runId, string confirmationId, string decision, string? reviewReason = null, CancellationToken cancellationToken = default);

    Task<IReadOnlyList<UiRunLogEntry>> QueryRunLogsAsync(string? level = null, string? query = null, CancellationToken cancellationToken = default);

    Task MarkRunLogsReadAsync(CancellationToken cancellationToken = default);

    Task<BudgetStatus> GetBudgetStatusAsync(CancellationToken cancellationToken = default);

    Task<BudgetStatus> UpdateBudgetConfigAsync(double budgetUsd, double preauthorizedUsd, CancellationToken cancellationToken = default);

    Task SetAutoModeAsync(bool enabled, CancellationToken cancellationToken = default);

    Task<BackendDiagnosticsReport> GetBackendDiagnosticsAsync(CancellationToken cancellationToken = default);
}
