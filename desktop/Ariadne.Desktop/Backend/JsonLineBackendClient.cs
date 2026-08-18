using System.Diagnostics;
using System.Text;
using System.Text.Json;

namespace Ariadne.Desktop.Backend;

public sealed class JsonLineBackendClient : IAriadneBackendClient, IDisposable
{
    /// <summary>
    /// 无 BOM 的 UTF-8。JSON-line 协议逐行解析，BOM 会让后端把第一行判为非法 JSON。
    /// </summary>
    private static readonly Encoding Utf8NoBom = new UTF8Encoding(encoderShouldEmitUTF8Identifier: false);

    private readonly string? _backendCommand;
    private readonly string _appStateRoot;
    private readonly JsonSerializerOptions _jsonOptions = new(JsonSerializerDefaults.Web);
    private readonly SemaphoreSlim _ipcWriteLock = new(1, 1);
    private readonly object _processLock = new();
    private readonly IpcResponseRouter _responseRouter = new();
    private readonly BoundedTextBuffer _stderrBuffer = new(32 * 1024);
    private Process? _backendProcess;
    private StreamWriter? _backendInput;
    private StreamReader? _backendOutput;
    private Task? _stderrPump;
    private Task? _stdoutPump;
    private string? _projectRoot;
    private long _nextRequestId;
    private int _processGeneration;
    private bool _disposed;

    internal JsonLineBackendClient(string? backendCommand)
    {
        _backendCommand = backendCommand;
        _appStateRoot = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "Ariadne");
    }

    public static JsonLineBackendClient CreateDefault()
    {
        return new JsonLineBackendClient(Environment.GetEnvironmentVariable("ARIADNE_BACKEND_IPC") ?? DiscoverBackendCommand());
    }

    public Task<IReadOnlyList<RecentProjectEntry>> ListRecentProjectsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeOrEmptyListAsync<RecentProjectEntry>("list_recent_projects", null, cancellationToken);
    }

    public Task<IReadOnlyList<RecentProjectEntry>> ForgetRecentProjectAsync(
        string projectRoot,
        CancellationToken cancellationToken = default)
    {
        return InvokeRequiredListAsync<RecentProjectEntry>(
            "forget_recent_project",
            new { project_root = projectRoot },
            cancellationToken);
    }

    public Task<CurrentProjectStatus> RelocateRecentProjectAsync(
        string previousProjectRoot,
        string projectRoot,
        CancellationToken cancellationToken = default)
    {
        return InvokeAndRememberProjectAsync<CurrentProjectStatus>(
            "relocate_recent_project",
            projectRoot,
            new
            {
                previous_project_root = previousProjectRoot,
                project_root = projectRoot,
            },
            cancellationToken);
    }

    public Task<AppStatus?> GetAppStatusAsync(CancellationToken cancellationToken = default)
    {
        return InvokeAsync<AppStatus>("get_app_status", null, cancellationToken);
    }

    public Task<SidebarBadgeCounts> GetSidebarBadgesAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<SidebarBadgeCounts>("get_sidebar_badges", null, cancellationToken);
    }

    public Task<ProjectMaintenanceState?> GetProjectMaintenanceAsync(CancellationToken cancellationToken = default)
    {
        return InvokeAsync<ProjectMaintenanceState>("get_project_maintenance", null, cancellationToken);
    }

    public Task<CurrentProjectStatus?> GetCurrentProjectAsync(CancellationToken cancellationToken = default)
    {
        return InvokeAsync<CurrentProjectStatus>("get_current_project", null, cancellationToken);
    }

    public Task<ProjectInitReport> CreateProjectAsync(string projectRoot, string? name = null, CancellationToken cancellationToken = default)
    {
        return InvokeAndRememberProjectAsync<ProjectInitReport>("create_project", projectRoot, new { project_root = projectRoot, name }, cancellationToken);
    }

    public Task<CurrentProjectStatus> OpenProjectAsync(string projectRoot, string? name = null, CancellationToken cancellationToken = default)
    {
        return InvokeAndRememberProjectAsync<CurrentProjectStatus>("open_project", projectRoot, new { project_root = projectRoot, name }, cancellationToken);
    }

    public Task SetProjectRootAsync(string projectRoot, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAndRememberProjectAsync("set_project_root", projectRoot, new { project_root = projectRoot }, cancellationToken);
    }

    public async Task CloseProjectAsync(CancellationToken cancellationToken = default)
    {
        await InvokeCommandAsync("close_project", null, cancellationToken).ConfigureAwait(false);
        _projectRoot = null;
    }

    public bool HasProjectRoot => !string.IsNullOrWhiteSpace(_projectRoot);

    public Task<AppSettings> GetAppSettingsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<AppSettings>("get_app_settings", null, cancellationToken);
    }

    public Task<AppSettings> SaveAppSettingsAsync(AppSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<AppSettings>("save_app_settings", new { settings }, cancellationToken);
    }

    public Task<GeneralSectionSettings> SaveGeneralSectionSettingsAsync(GeneralSectionSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<GeneralSectionSettings>("save_general_section_settings", new { settings }, cancellationToken);
    }

    public Task<ProviderConfigStatus> GetProviderConfigAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderConfigStatus>("get_provider_config", null, cancellationToken);
    }

    public Task<ProviderConfigStatus> SaveProviderSettingsAsync(ProviderSettingsUpdate update, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderConfigStatus>("save_provider_settings", new { update }, cancellationToken);
    }

    public Task<ProviderConfigStatus> SaveProviderSectionSettingsAsync(ProviderSectionSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderConfigStatus>("save_provider_section_settings", new { settings }, cancellationToken);
    }

    public Task<ProviderRemovalPreview> PreviewProviderRemovalAsync(string provider, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderRemovalPreview>("preview_provider_removal", new { provider }, cancellationToken);
    }

    public Task<ProviderConfigStatus> RemoveProviderAsync(string provider, string expectedRevision, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderConfigStatus>(
            "remove_provider",
            new { provider, expected_revision = expectedRevision },
            cancellationToken);
    }

    public Task<ProviderModelsResult> FetchProviderModelsAsync(string? providerId = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderModelsResult>("fetch_provider_models", new { provider_id = providerId }, cancellationToken);
    }

    public Task<ProviderModelsResult> TestProviderDraftAsync(ProviderDraftProbe probe, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderModelsResult>("test_provider_draft", new { probe }, cancellationToken);
    }

    public Task<ProviderConfigStatus> SaveProviderKeyAsync(string provider, string key, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderConfigStatus>("save_provider_key", new { provider, key }, cancellationToken);
    }

    public Task<ProviderConfigStatus> RevokeProviderKeyAsync(string provider, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderConfigStatus>("revoke_provider_key", new { provider }, cancellationToken);
    }

    public Task<NodePresetSettings> GetNodePresetSettingsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<NodePresetSettings>("get_node_preset_settings", null, cancellationToken);
    }

    public Task<NodePresetSettings> SaveNodePresetSettingsAsync(NodePresetSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<NodePresetSettings>("save_node_preset_settings", new { settings }, cancellationToken);
    }

    public Task<AutomationSettings> GetAutomationSettingsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<AutomationSettings>("get_automation_settings", null, cancellationToken);
    }

    public Task<AutomationSettings> SaveAutomationSettingsAsync(AutomationSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<AutomationSettings>("save_automation_settings", new { settings }, cancellationToken);
    }

    public Task<AutomationSectionSettings> SaveAutomationSectionSettingsAsync(AutomationSectionSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<AutomationSectionSettings>("save_automation_section_settings", new { settings }, cancellationToken);
    }

    public Task<PermissionsSettings> GetPermissionsSettingsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<PermissionsSettings>("get_permissions_settings", null, cancellationToken);
    }

    public Task<PermissionsSettings> SavePermissionsSettingsAsync(PermissionsSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<PermissionsSettings>("save_permissions_settings", new { settings }, cancellationToken);
    }

    public Task<UiPreferences> GetUiPreferencesAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<UiPreferences>("get_ui_preferences", null, cancellationToken);
    }

    public Task SaveUiPreferencesAsync(UiPreferences preferences, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("save_ui_preferences", new { preferences }, cancellationToken);
    }

    public Task<TemplateRepositorySettings> GetTemplateRepositorySettingsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<TemplateRepositorySettings>("get_template_repository_settings", null, cancellationToken);
    }

    public Task<TemplateRepositorySettings> SaveTemplateRepositorySettingsAsync(TemplateRepositorySettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<TemplateRepositorySettings>("save_template_repository_settings", new { settings }, cancellationToken);
    }

    public Task<WorkflowSettings> GetWorkflowSettingsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowSettings>("get_workflow_settings", null, cancellationToken);
    }

    public Task<WorkflowSettings> SaveWorkflowSettingsAsync(WorkflowSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowSettings>("save_workflow_settings", new { settings }, cancellationToken);
    }

    public Task<GitSettings> GetGitSettingsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<GitSettings>("get_git_settings", null, cancellationToken);
    }

    public Task<GitSettings> SaveGitSettingsAsync(GitSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<GitSettings>("save_git_settings", new { settings }, cancellationToken);
    }

    public Task<RagSettings> GetRagSettingsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<RagSettings>("get_rag_settings", null, cancellationToken);
    }

    public Task<AppRuntimeSettings> GetAppRuntimeSettingsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<AppRuntimeSettings>("get_app_runtime_settings", null, cancellationToken);
    }

    public Task<AppRuntimeSettings> SaveAppRuntimeSettingsAsync(AppRuntimeSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<AppRuntimeSettings>("save_app_runtime_settings", new { settings }, cancellationToken);
    }

    public Task<RagSettings> SaveRagSettingsAsync(RagSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<RagSettings>("save_rag_settings", new { settings }, cancellationToken);
    }

    public Task<MiscSectionSettings> SaveMiscSectionSettingsAsync(MiscSectionSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<MiscSectionSettings>("save_misc_section_settings", new { settings }, cancellationToken);
    }

    public Task<IReadOnlyList<TemplateSummary>> SearchTemplatesAsync(
        string baseUrl,
        string query,
        IReadOnlyList<string> tags,
        int page = 0,
        CancellationToken cancellationToken = default)
    {
        return InvokeRequiredListAsync<TemplateSummary>("search_templates", new
        {
            request = new { base_url = string.IsNullOrWhiteSpace(baseUrl) ? null : baseUrl },
            query,
            tags,
            page,
        }, cancellationToken);
    }

    public Task<TemplateDetail> GetTemplateDetailAsync(string baseUrl, string id, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<TemplateDetail>("get_template_detail", new
        {
            request = new { base_url = string.IsNullOrWhiteSpace(baseUrl) ? null : baseUrl },
            id,
        }, cancellationToken);
    }

    public Task<TemplateInstallReport> InstallTemplateAsync(
        string baseUrl,
        string id,
        string expectedProjectRoot,
        CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<TemplateInstallReport>("install_template", new
        {
            request = new { base_url = string.IsNullOrWhiteSpace(baseUrl) ? null : baseUrl },
            id,
            expected_project_root = expectedProjectRoot,
        }, cancellationToken);
    }

    public Task<WorkflowRunStarted> RunWorkflowAsync(string workflowId, string? startNodeId = null, IReadOnlyDictionary<string, object?>? variables = null, CancellationToken cancellationToken = default)
    {
        // U156（P0 回归）：**匿名对象压根做不到「按需加键」**。
        //
        // 原写法是 `variables = variables is { Count: > 0 } ? variables : null`，
        // 上面还配着「variables 为空时不发该键」的注释——**注释的意图对，代码做的正好相反**：
        // 匿名对象的属性集是**编译期固定**的，三元表达式只能改值、改不了「这个键在不在」，
        // 而 `System.Text.Json` 默认会序列化 null 属性，于是 `"variables":null` 如实发出。
        // 后端 `RunWorkflowParams.variables`（`core/src/ipc.rs`）是**非 `Option`** 的
        // `BTreeMap` + `#[serde(default)]`，而 **`default` 只对「键缺失」生效、不接受显式 null**
        // ⇒ 整条请求被拒：`invalid ipc params: invalid type: null, expected a map`。
        // 后果是产品主功能全废——**点运行什么都不会发生**。
        //
        // ⚠️ 旁边的 `start_node_id` 同样可能传 null 却没事，因为它是 `Option<String>`。
        // **两个字段并排、写法看着一样、一个能吃 null 一个不能**，这是本条最容易看漏的地方。
        //
        // ⚠️ **不要**改用 `_jsonOptions` 全局加 `DefaultIgnoreCondition = WhenWritingNull` 来「一劳永逸」：
        // 全仓有大量参数**依赖显式 null 表达「清空」**（`expected_revision`、
        // U112 的 `preauthorized_budget_usd` 那个 `Some(0.0)` vs `None` 双重语义），
        // 全局忽略会静默改变这些命令的语义，换来一批更难查的缺陷。
        // 所以只在这一处用字典**真正做到按需加键**。
        var payload = new Dictionary<string, object?>(StringComparer.Ordinal)
        {
            ["workflow_id"] = workflowId,
        };
        if (startNodeId is not null)
        {
            payload["start_node_id"] = startNodeId;
        }
        // 空字典也不发键：`BuildStartVariables()` 在「无变量组」时返回**空字典**而非 null，
        // 发个空 map 虽然后端能吃，但与「没有变量」语义相同，少一个键更省。
        if (variables is { Count: > 0 })
        {
            payload["variables"] = variables;
        }

        return InvokeRequiredAsync<WorkflowRunStarted>("start_workflow", payload, cancellationToken);
    }

    public Task<WorkflowActionResult> PauseWorkflowAsync(string workflowId, string runId, string? reason = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowActionResult>("pause_workflow", new { workflow_id = workflowId, run_id = runId, reason }, cancellationToken);
    }

    public Task<WorkflowActionResult> StopWorkflowAsync(string workflowId, string runId, string? reason = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowActionResult>("stop_workflow", new { workflow_id = workflowId, run_id = runId, reason }, cancellationToken);
    }

    public Task<WorkflowActionResult> ResumeWorkflowAsync(string workflowId, string runId, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowActionResult>("resume_workflow", new { workflow_id = workflowId, run_id = runId }, cancellationToken);
    }

    public Task<WorkflowRunState> GetWorkflowRunStateAsync(string workflowId, string runId, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowRunState>("get_workflow_run_state", new { workflow_id = workflowId, run_id = runId }, cancellationToken);
    }

    public Task<WorkflowEventsResult> GetWorkflowEventsAsync(string workflowId, string runId, long afterSequence = 0, int? limit = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowEventsResult>("get_workflow_events", new
        {
            workflow_id = workflowId,
            run_id = runId,
            after_sequence = afterSequence,
            limit,
        }, cancellationToken);
    }

    public Task<IReadOnlyList<WorkflowOperation>> ListInDoubtOperationsAsync(string workflowId, string runId, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredListAsync<WorkflowOperation>("list_in_doubt_operations", new
        {
            workflow_id = workflowId,
            run_id = runId,
        }, cancellationToken);
    }

    public Task<ResolveInDoubtOperationResult> ResolveInDoubtOperationAsync(string operationId, string decision, object? response = null, string? reason = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ResolveInDoubtOperationResult>("resolve_workflow_operation_in_doubt", new
        {
            operation_id = operationId,
            decision,
            response,
            reason,
        }, cancellationToken);
    }

    public Task<ProjectAiResponse> ProjectAiChatAsync(
        string message,
        string? workflowIdToRun = null,
        string? referenceWorkflowId = null,
        string? referenceRunId = null,
        string? conversationId = null,
        long? conversationRevision = null,
        IReadOnlyList<string>? references = null,
        CancellationToken cancellationToken = default)
    {
        return ProjectAiChatAsync(
            message,
            Array.Empty<ProjectAiChatMessage>(),
            workflowIdToRun,
            referenceWorkflowId,
            referenceRunId,
            conversationId,
            conversationRevision,
            references,
            cancellationToken);
    }

    public Task<ProjectAiResponse> ProjectAiChatAsync(
        string message,
        IReadOnlyList<ProjectAiChatMessage> chatHistory,
        string? workflowIdToRun = null,
        string? referenceWorkflowId = null,
        string? referenceRunId = null,
        string? conversationId = null,
        long? conversationRevision = null,
        IReadOnlyList<string>? references = null,
        CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProjectAiResponse>("project_ai_chat", new
        {
            request = new
            {
                message,
                chat_history = chatHistory,
                // 原先写死 Array.Empty<string>()：后端 references 链路（`@确认项:<id>` 展开）
                // 从头到尾是通的，唯独前端永远只发空数组，等于把入口焊死（U139）。
                // 传 null 时仍发空数组——后端字段是 Vec<String> 而非 Option，缺字段虽有
                // serde default 兜底，但显式空数组语义更清楚。
                references = references ?? (IReadOnlyList<string>)Array.Empty<string>(),
                workflow_id_to_run = workflowIdToRun,
                reference_workflow_id = referenceWorkflowId,
                reference_run_id = referenceRunId,
                conversation_id = conversationId,
                conversation_revision = conversationRevision,
                append_memory = (string?)null,
            },
        }, cancellationToken);
    }

    public Task<string> ReadProjectMemoryAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<string>("read_project_memory", null, cancellationToken);
    }

    public Task<string> AppendProjectMemoryAsync(string content, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<string>("append_project_memory", new { content }, cancellationToken);
    }

    public Task WriteProjectMemoryAsync(string content, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("write_project_memory", new { content }, cancellationToken);
    }

    public Task<ProjectReference> ResolveProjectReferenceAsync(string reference, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProjectReference>("resolve_project_reference", new { reference }, cancellationToken);
    }

    public Task<IReadOnlyList<WorkflowSummary>> ListWorkflowGraphsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredListAsync<WorkflowSummary>("list_workflow_graphs", null, cancellationToken);
    }

    public Task<WorkflowGraphData> LoadWorkflowGraphAsync(string? workflowId = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowGraphData>("load_workflow_graph", new { workflow_id = workflowId }, cancellationToken);
    }

    public Task<WorkflowGraphData> LoadProjectCanvasAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowGraphData>("load_project_canvas", null, cancellationToken);
    }

    public Task<WorkflowGraphData> SaveWorkflowGraphAsync(WorkflowGraphData graphData, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowGraphData>("save_workflow_graph", new { graph_data = graphData }, cancellationToken);
    }

    public Task<WorkflowGraphData> SaveProjectCanvasAsync(WorkflowGraphData graphData, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowGraphData>("save_project_canvas", new { graph_data = graphData }, cancellationToken);
    }

    public Task ValidateWorkflowGraphAsync(WorkflowGraphData graphData, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("validate_workflow_graph", new { graph_data = graphData }, cancellationToken);
    }

    public Task ApplyNodeDetailPatchAsync(string workflowId, NodeDetailPatch patch, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("apply_node_detail_patch", new { workflow_id = workflowId, patch }, cancellationToken);
    }

    public Task UpsertCanvasAnnotationAsync(string workflowId, CanvasAnnotation annotation, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("upsert_canvas_annotation", new { workflow_id = workflowId, annotation }, cancellationToken);
    }

    public Task SetNodeBreakpointAsync(string workflowId, string nodeId, bool enabled, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("set_node_breakpoint", new { workflow_id = workflowId, node_id = nodeId, enabled }, cancellationToken);
    }

    public Task<WorkflowSelectionExportData> ExportWorkflowSelectionAsync(string workflowId, IReadOnlyList<string> selectedNodeIds, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowSelectionExportData>("export_workflow_selection", new
        {
            workflow_id = workflowId,
            selected_node_ids = selectedNodeIds,
        }, cancellationToken);
    }

    public Task<WorkflowPackReport> PackWorkflowSelectionAsync(string workflowId, IReadOnlyList<string> selectedNodeIds, string? subworkflowNodeId = null, string? title = null, string? expectedRevision = null, string? operationId = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowPackReport>("pack_workflow_selection", new
        {
            workflow_id = workflowId,
            selected_node_ids = selectedNodeIds,
            subworkflow_node_id = subworkflowNodeId,
            title,
            expected_revision = expectedRevision,
            operation_id = operationId,
        }, cancellationToken);
    }

    public Task<WorkflowPackReport> GetPackOperationAsync(string operationId, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowPackReport>("get_pack_operation", new
        {
            operation_id = operationId,
        }, cancellationToken);
    }

    public Task<WorksTreeNode> GetWorksTreeAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorksTreeNode>("get_works_tree", null, cancellationToken);
    }

    public Task<ChapterSummaryView> GetChapterSummaryViewAsync(string chapterId, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ChapterSummaryView>("get_chapter_summary_view", new { chapter_id = chapterId }, cancellationToken);
    }

    public Task<DocumentTreeNode> GetDocumentTreeAsync(string? projectId = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<DocumentTreeNode>("get_document_tree", new { project_id = projectId }, cancellationToken);
    }

    public Task<ChapterImportReport> ImportChapterAsync(ChapterImportRequest request, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ChapterImportReport>("import_chapter", new { request }, cancellationToken);
    }

    /// <summary>U174：新建章节。返回更新后的章节索引（与后端 create_chapter 一致）。</summary>
    public Task<ChapterDocumentIndexResult> CreateChapterAsync(ChapterCreateRequest request, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ChapterDocumentIndexResult>("create_chapter", new { request }, cancellationToken);
    }

    public Task<CombinedExportReport> ExportChaptersAsync(IReadOnlyList<string> selectedChapterIds, string? artifactId = null, string format = "markdown", CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<CombinedExportReport>("export_chapters", new
        {
            selected_chapter_ids = selectedChapterIds,
            artifact_id = artifactId,
            format,
        }, cancellationToken);
    }

    public Task<DocumentWriteReport> SaveDocumentContentAsync(string documentId, string content, string? baseVersion = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<DocumentWriteReport>("save_document_content", new { document_id = documentId, content, base_version = baseVersion }, cancellationToken);
    }

    public Task<string> GetDocumentContentAsync(string documentId, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<string>("get_document_content", new { document_id = documentId }, cancellationToken);
    }

    public Task<string> GetDocumentContentByPathAsync(string path, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<string>("get_document_content", new { path }, cancellationToken);
    }

    public Task<DocumentContentResult> GetDocumentContentDetailsAsync(string documentId, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<DocumentContentResult>("get_document_content_details", new { document_id = documentId }, cancellationToken);
    }

    public Task<DocumentContentResult> GetDocumentContentDetailsByPathAsync(string path, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<DocumentContentResult>("get_document_content_details", new { path }, cancellationToken);
    }

    public Task<QuickEditResult> QuickEditAsync(QuickEditRequest request, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<QuickEditResult>("quick_edit", new { request }, cancellationToken);
    }

    public Task<PatchApplyReport> ApplyQuickEditAsync(string documentId, string? baseVersion, string text, TextRange range, QuickEditResult result, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<PatchApplyReport>("apply_quick_edit", new
        {
            document_id = documentId,
            base_version = baseVersion,
            text,
            range,
            result,
        }, cancellationToken);
    }

    public Task<ArchivePoint> CreateCheckpointAsync(string message, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ArchivePoint>("create_checkpoint", new { message }, cancellationToken);
    }

    public Task<IReadOnlyList<GitCommitSummary>> GetGitHistoryAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredListAsync<GitCommitSummary>("get_git_history", null, cancellationToken);
    }

    public Task<GitRepositoryStatus> GetGitRepositoryStatusAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<GitRepositoryStatus>("get_git_repository_status", null, cancellationToken);
    }

    public Task<IReadOnlyList<BranchGraphNode>> GetGitBranchGraphAsync(int limit = 200, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredListAsync<BranchGraphNode>("get_git_branch_graph", new { limit }, cancellationToken);
    }

    public Task<RestoreReport> RestoreToNewBranchAsync(string commitId, string newBranch, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<RestoreReport>("restore_to_new_branch", new
        {
            commit_id = commitId,
            new_branch = newBranch,
        }, cancellationToken);
    }

    public Task<IReadOnlyList<ConfirmationLogEntry>> ListConfirmationsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredListAsync<ConfirmationLogEntry>("list_confirmations", null, cancellationToken);
    }

    public Task<ConfirmationLogEntry> GetConfirmationAsync(string confirmationId, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ConfirmationLogEntry>("get_confirmation", new { confirmation_id = confirmationId }, cancellationToken);
    }

    public Task<ResolveConfirmationResult> ResolveConfirmationAsync(string workflowId, string runId, string confirmationId, string decision, string? reviewReason = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ResolveConfirmationResult>("resolve_confirmation", new
        {
            request = new
            {
                workflow_id = workflowId,
                run_id = runId,
                confirmation_id = confirmationId,
                decision,
                review_reason = reviewReason,
            },
        }, cancellationToken);
    }

    public Task<IReadOnlyList<UiRunLogEntry>> QueryRunLogsAsync(RunLogQuery query, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredListAsync<UiRunLogEntry>("query_run_logs", new
        {
            filter = query,
        }, cancellationToken);
    }

    public Task<int> MarkRunLogsReadAsync(RunLogQuery filter, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<int>("mark_run_logs_read", new { filter }, cancellationToken);
    }

    public Task<BudgetStatus> GetBudgetStatusAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<BudgetStatus>("get_budget_status", null, cancellationToken);
    }

    public Task<BudgetStatus> UpdateBudgetConfigAsync(double budgetUsd, double? preauthorizedUsd, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<BudgetStatus>("update_budget_config", new { budget_usd = budgetUsd, preauthorized_usd = preauthorizedUsd }, cancellationToken);
    }

    public Task SetAutoModeAsync(bool enabled, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("set_auto_mode", new { enabled }, cancellationToken);
    }

    public Task<BackendDiagnosticsReport> GetBackendDiagnosticsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<BackendDiagnosticsReport>("get_backend_diagnostics", null, cancellationToken);
    }

    /// <summary>U176：读取凭据保护状态。</summary>
    public Task<SecretProtectionReport> GetSecretProtectionAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<SecretProtectionReport>("get_secret_protection", null, cancellationToken);
    }

    /// <summary>
    /// U176：设置本地主密码。
    ///
    /// 参数名必须是 <c>master_password</c>：后端 <c>SetMasterPasswordParams</c> 按该字段名
    /// 反序列化，写错会得到一条「missing field」而不是任何界面提示。
    /// </summary>
    public Task<SecretProtectionReport> SetLocalSecretMasterPasswordAsync(
        string masterPassword,
        CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<SecretProtectionReport>(
            "set_local_secret_master_password",
            new { master_password = masterPassword },
            cancellationToken);
    }

    /// <summary>U176：显式接受明文保存；无参数。</summary>
    public Task<SecretProtectionReport> AllowUnprotectedLocalSecretsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<SecretProtectionReport>(
            "allow_unprotected_local_secrets",
            null,
            cancellationToken);
    }

    public async Task<T?> InvokeAsync<T>(
        string method,
        object? parameters = null,
        CancellationToken cancellationToken = default)
    {
        return await InvokeOrDefaultAsync<T>(method, parameters, cancellationToken).ConfigureAwait(false);
    }

    private async Task<IReadOnlyList<T>> InvokeOrEmptyListAsync<T>(
        string method,
        object? parameters,
        CancellationToken cancellationToken)
    {
        var result = await InvokeOrDefaultAsync<List<T>>(method, parameters, cancellationToken).ConfigureAwait(false);
        return result is null ? Array.Empty<T>() : result;
    }

    private async Task<T?> InvokeOrDefaultAsync<T>(
        string method,
        object? parameters,
        CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        if (string.IsNullOrWhiteSpace(_backendCommand))
        {
            return default;
        }
        try
        {
            return await SendRequestAsync<T>(method, parameters, cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            throw;
        }
        catch
        {
            return default;
        }
    }

    private async Task<IReadOnlyList<T>> InvokeRequiredListAsync<T>(
        string method,
        object? parameters,
        CancellationToken cancellationToken)
    {
        var result = await InvokeRequiredAsync<List<T>>(method, parameters, cancellationToken).ConfigureAwait(false);
        return result;
    }

    private async Task<T> InvokeRequiredAsync<T>(
        string method,
        object? parameters,
        CancellationToken cancellationToken)
    {
        if (string.IsNullOrWhiteSpace(_backendCommand))
        {
            throw BackendException.Transport("ipc", BackendCommandMissingDiagnostic());
        }
        var data = await SendRequestAsync<T>(method, parameters, cancellationToken).ConfigureAwait(false);
        return data is null
            ? throw BackendException.Transport("ipc", "backend command returned empty data")
            : data;
    }

    private async Task InvokeCommandAsync(
        string method,
        object? parameters,
        CancellationToken cancellationToken)
    {
        if (string.IsNullOrWhiteSpace(_backendCommand))
        {
            throw BackendException.Transport("ipc", BackendCommandMissingDiagnostic());
        }
        await SendRequestAsync<object>(method, parameters, cancellationToken).ConfigureAwait(false);
    }

    private async Task<T?> SendRequestAsync<T>(
        string method,
        object? parameters,
        CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();
        EnsureBackendProcess();
        var requestId = Interlocked.Increment(ref _nextRequestId).ToString(System.Globalization.CultureInfo.InvariantCulture);
        if (!_responseRouter.TryRegister(requestId, out var response))
        {
            throw BackendException.Transport("ipc", "duplicate backend request id");
        }

        try
        {
            using var registration = cancellationToken.Register(
                () => CancelPendingRequest(requestId, cancellationToken));
            var request = JsonSerializer.Serialize(
                new { request_id = requestId, method, @params = parameters ?? new { } },
                _jsonOptions);
            await WriteRequestLineAsync(request, cancellationToken).ConfigureAwait(false);
            var output = await response.ConfigureAwait(false);
            var result = JsonSerializer.Deserialize<BackendResult<T>>(output, _jsonOptions)
                ?? throw BackendException.Transport("ipc", "backend ipc returned invalid json");
            if (!string.Equals(result.RequestId, requestId, StringComparison.Ordinal))
            {
                throw BackendException.Transport("ipc", "backend ipc response id mismatch");
            }
            if (!result.Ok)
            {
                throw BackendException.FromIpcPayload(
                    result.ErrorCode,
                    result.Error ?? "backend command failed",
                    result.ErrorKey,
                    result.ErrorParams,
                    result.ErrorField,
                    result.ErrorSection,
                    result.RecoveryAction,
                    result.CorrelationId);
            }
            return result.Data;
        }
        catch
        {
            _responseRouter.Remove(requestId);
            if (_backendProcess?.HasExited == true)
            {
                ResetBackendProcess();
            }
            throw;
        }
    }

    private async Task WriteRequestLineAsync(string request, CancellationToken cancellationToken)
    {
        await _ipcWriteLock.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            StreamWriter input;
            lock (_processLock)
            {
                input = _backendInput
                    ?? throw BackendException.Transport("ipc", "backend ipc process is not connected");
            }
            await input.WriteLineAsync(request.AsMemory(), cancellationToken).ConfigureAwait(false);
            await input.FlushAsync(cancellationToken).ConfigureAwait(false);
        }
        finally
        {
            _ipcWriteLock.Release();
        }
    }

    private void CancelPendingRequest(string requestId, CancellationToken cancellationToken)
    {
        if (_responseRouter.TryCancel(requestId, cancellationToken))
        {
            _ = SendCancellationRequestAsync(requestId);
        }
    }

    private async Task SendCancellationRequestAsync(string targetRequestId)
    {
        try
        {
            var cancelRequestId = $"cancel-{targetRequestId}";
            var request = JsonSerializer.Serialize(
                new
                {
                    request_id = cancelRequestId,
                    method = "cancel_request",
                    @params = new { target_request_id = targetRequestId },
                },
                _jsonOptions);
            await WriteRequestLineAsync(request, CancellationToken.None).ConfigureAwait(false);
        }
        catch
        {
            // The caller is already cancelled; process failure is reported to remaining requests.
        }
    }

    private void EnsureBackendProcess()
    {
        lock (_processLock)
        {
            if (_disposed)
            {
                throw new ObjectDisposedException(nameof(JsonLineBackendClient));
            }
            if (_backendProcess is { HasExited: false }
                && _backendInput is not null
                && _backendOutput is not null)
            {
                return;
            }
            ResetBackendProcessLocked();
            if (string.IsNullOrWhiteSpace(_backendCommand))
            {
                throw BackendException.Transport("ipc", BackendCommandMissingDiagnostic());
            }

            var startInfo = new ProcessStartInfo
            {
                // ARIADNE_BACKEND_IPC 与发布目录中的 sidecar 都是可执行文件路径。
                // 路径可能包含空格，禁止再按命令行字符串拆分。
                FileName = _backendCommand,
                RedirectStandardInput = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                // P0 根因：静态 Encoding.UTF8 带 BOM 前导码，管道不可 seek 时
                // StreamWriter 会在第一次写入前发出 EF BB BF——后端 serde_json
                // 对第一条请求报 "expected value at line 1 column 1"，回一个
                // 无 request_id 的错误，旧版 stdout pump 因此杀死整条连接，
                // 之后所有按钮都报「无法连接本地后端服务」。必须用无 BOM 编码。
                StandardInputEncoding = Utf8NoBom,
                StandardOutputEncoding = Utf8NoBom,
                StandardErrorEncoding = Utf8NoBom,
            };
            ApplyProjectEnvironment(startInfo);

            var process = Process.Start(startInfo)
                ?? throw BackendException.Transport("ipc", "failed to start backend ipc process");
            var generation = ++_processGeneration;
            _backendProcess = process;
            _backendInput = process.StandardInput;
            _backendOutput = process.StandardOutput;
            _stderrBuffer.Clear();
            _stdoutPump = PumpStdoutAsync(process.StandardOutput, generation);
            _stderrPump = Task.Run(async () =>
            {
                try
                {
                    while (await process.StandardError.ReadLineAsync().ConfigureAwait(false) is { } line)
                    {
                        _stderrBuffer.AppendLine(line);
                    }
                }
                catch
                {
                    // stderr is diagnostic only; request/response errors are handled by stdout.
                }
            });
        }
    }

    private async Task PumpStdoutAsync(StreamReader output, int generation)
    {
        Exception? failure = null;
        try
        {
            while (await output.ReadLineAsync().ConfigureAwait(false) is { } line)
            {
                if (string.IsNullOrWhiteSpace(line))
                {
                    continue;
                }
                BackendResult<JsonElement>? envelope;
                try
                {
                    envelope = JsonSerializer.Deserialize<BackendResult<JsonElement>>(line, _jsonOptions);
                }
                catch (JsonException)
                {
                    // 非 JSON 行（sidecar 误把日志写到 stdout、外层包装脚本回显等）
                    // 不得毒死整条连接：过去这里抛异常会终止 pump，之后**所有**
                    // 请求都失败并报「无法连接本地后端服务」，整个应用不可用。
                    _stderrBuffer.AppendLine($"[stdout non-json] {Truncate(line)}");
                    continue;
                }
                if (envelope is null || string.IsNullOrWhiteSpace(envelope.RequestId))
                {
                    // 缺 request_id 的响应无法归属到任何等待中的请求。
                    // 它通常是后端在解析请求失败时回的全局错误——记录下来供诊断，
                    // 但同样不能终止 pump，否则一次坏输入就让整个会话报废。
                    _stderrBuffer.AppendLine($"[unattributed response] {Truncate(line)}");
                    continue;
                }
                _responseRouter.TryComplete(envelope.RequestId, line);
            }
            failure = BackendException.Transport(
                "ipc",
                string.IsNullOrWhiteSpace(CurrentBackendStderr())
                    ? "backend ipc returned no response"
                    : CurrentBackendStderr());
        }
        catch (Exception error)
        {
            failure = error;
        }
        finally
        {
            HandleBackendPumpEnded(generation, failure);
        }
    }

    private void HandleBackendPumpEnded(int generation, Exception? failure)
    {
        lock (_processLock)
        {
            if (generation != _processGeneration)
            {
                return;
            }
            ResetBackendProcessLocked();
        }
        FailPendingRequests(failure ?? BackendException.Transport("ipc", "backend ipc disconnected"));
    }

    private string CurrentBackendStderr()
    {
        return _stderrBuffer.Read().Trim();
    }

    /// <summary>诊断行截断，避免异常长的坏行撑爆 stderr 环形缓冲。</summary>
    private static string Truncate(string line) =>
        line.Length <= 500 ? line : line[..500] + "…";

    private void ResetBackendProcess()
    {
        lock (_processLock)
        {
            ResetBackendProcessLocked();
        }
        FailPendingRequests(BackendException.Transport("ipc", "backend ipc process reset"));
    }

    private void ResetBackendProcessLocked()
    {
        try
        {
            _backendInput?.Dispose();
            _backendOutput?.Dispose();
            if (_backendProcess is { HasExited: false })
            {
                _backendProcess.Kill(entireProcessTree: true);
            }
            _backendProcess?.Dispose();
        }
        catch
        {
            // Best-effort cleanup before reconnecting.
        }
        finally
        {
            _backendInput = null;
            _backendOutput = null;
            _backendProcess = null;
            _stderrPump = null;
            _stdoutPump = null;
        }
    }

    private void FailPendingRequests(Exception error)
    {
        _responseRouter.FailAll(error);
    }

    public void Dispose()
    {
        _disposed = true;
        ResetBackendProcess();
        _ipcWriteLock.Dispose();
    }

    private async Task<T> InvokeAndRememberProjectAsync<T>(
        string method,
        string projectRoot,
        object? parameters,
        CancellationToken cancellationToken)
    {
        var result = await InvokeRequiredAsync<T>(method, parameters, cancellationToken).ConfigureAwait(false);
        _projectRoot = projectRoot;
        return result;
    }

    private async Task InvokeCommandAndRememberProjectAsync(
        string method,
        string projectRoot,
        object? parameters,
        CancellationToken cancellationToken)
    {
        await InvokeCommandAsync(method, parameters, cancellationToken).ConfigureAwait(false);
        _projectRoot = projectRoot;
    }

    private void ApplyProjectEnvironment(ProcessStartInfo startInfo)
    {
        startInfo.Environment["ARIADNE_APP_STATE_ROOT"] = _appStateRoot;
        if (!string.IsNullOrWhiteSpace(_projectRoot))
        {
            startInfo.Environment["ARIADNE_PROJECT_ROOT"] = _projectRoot;
        }
    }

    private static string? DiscoverBackendCommand()
    {
        return DiscoverBackendCommand(AppContext.BaseDirectory, Environment.CurrentDirectory);
    }

    /// <summary>
    /// sidecar 未找到时的可诊断说明。
    /// 「无法连接本地后端服务」这一句对排查毫无帮助——用户分不清是没编译 sidecar、
    /// 启动目录不在源码树内，还是 Release 构建缺少打包。这里把搜索轨迹带出来。
    /// </summary>
    private static string BackendCommandMissingDiagnostic()
    {
        return LastDiscoveryReport ?? "backend ipc command not found";
    }

    /// <summary>
    /// 记录最近一次 sidecar 查找过程，供「后端连不上」时给出可诊断原因。
    /// 只保存路径，不含任何凭据。
    /// </summary>
    internal static string? LastDiscoveryReport { get; private set; }

    internal static string? DiscoverBackendCommand(string appBaseDirectory, string currentDirectory)
    {
        var attempted = new List<string>();
        var packaged = FindPackagedBackendCommand(appBaseDirectory);
        if (packaged is not null)
        {
            LastDiscoveryReport = null;
            return packaged;
        }
        attempted.Add($"packaged under {appBaseDirectory}");

        var executableNames = OperatingSystem.IsWindows()
            ? new[] { "ariadne-ipc.exe", "ariadne-ipc" }
            : new[] { "ariadne-ipc" };
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var root in CandidateBackendRoots(appBaseDirectory, currentDirectory))
        {
            foreach (var executableName in executableNames)
            {
                // 同时搜 debug 与 release：只搜 debug 会让 release 构建必然找不到
                // sidecar，表现为「所有按钮都连不上后端」。
                foreach (var profile in new[] { "debug", "release" })
                {
                    foreach (var relativePath in new[]
                             {
                                 Path.Combine("target", profile, executableName),
                                 Path.Combine("core", "target", profile, executableName),
                             })
                    {
                        var candidate = Path.GetFullPath(Path.Combine(root, relativePath));
                        if (!seen.Add(candidate))
                        {
                            continue;
                        }
                        if (File.Exists(candidate))
                        {
                            LastDiscoveryReport = null;
                            return candidate;
                        }
                        attempted.Add(candidate);
                    }
                }
            }
        }

        // 找不到时保留完整搜索轨迹：否则用户只看到「无法连接本地后端服务」，
        // 无从判断是没编译 sidecar、还是启动目录不在源码树内。
        LastDiscoveryReport =
            $"backend ipc executable not found. Set ARIADNE_BACKEND_IPC to its path, "
            + $"or build it with `cargo build -p ariadne --bin ariadne-ipc`. "
            + $"Searched {attempted.Count} location(s): {string.Join("; ", attempted.Take(12))}";
        return null;
    }

    internal static string? FindPackagedBackendCommand(string appBaseDirectory)
    {
        var executableNames = OperatingSystem.IsWindows()
            ? new[] { "ariadne-ipc.exe", "ariadne-ipc" }
            : new[] { "ariadne-ipc" };
        foreach (var executableName in executableNames)
        {
            foreach (var relativePath in new[]
                     {
                         Path.Combine("Backend", executableName),
                         executableName,
                     })
            {
                var candidate = Path.GetFullPath(Path.Combine(appBaseDirectory, relativePath));
                if (File.Exists(candidate))
                {
                    return candidate;
                }
            }
        }

        return null;
    }

    private static IEnumerable<string> CandidateBackendRoots(params string[] starts)
    {
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var start in starts)
        {
            var directory = new DirectoryInfo(Path.GetFullPath(start));
            for (var depth = 0; directory is not null && depth < 8; depth++)
            {
                if (seen.Add(directory.FullName))
                {
                    yield return directory.FullName;
                }
                directory = directory.Parent;
            }
        }
    }
}
