namespace Ariadne.Desktop.Backend;

public interface IAriadneBackendClient
{
    Task<T?> InvokeAsync<T>(
        string method,
        object? parameters = null,
        CancellationToken cancellationToken = default);

    Task<IReadOnlyList<RecentProjectEntry>> ListRecentProjectsAsync(CancellationToken cancellationToken = default);

    Task<IReadOnlyList<RecentProjectEntry>> ForgetRecentProjectAsync(string projectRoot, CancellationToken cancellationToken = default);

    Task<CurrentProjectStatus> RelocateRecentProjectAsync(
        string previousProjectRoot,
        string projectRoot,
        CancellationToken cancellationToken = default);

    Task<AppStatus?> GetAppStatusAsync(CancellationToken cancellationToken = default);

    Task<SidebarBadgeCounts> GetSidebarBadgesAsync(CancellationToken cancellationToken = default);

    /// <summary>D3：查询项目维护状态；无维护时返回 null。</summary>
    Task<ProjectMaintenanceState?> GetProjectMaintenanceAsync(CancellationToken cancellationToken = default);

    /// <summary>
    /// U196-B：解除「维护失败」态，让项目重新可写。
    ///
    /// 只对 <c>status == "failed"</c> 生效（<c>active</c> 表示确实有维护在跑，
    /// 清掉它等于绕过 checkout 期间的保护），并连带入队一次全量索引重建 ——
    /// 回档中断后索引与正文的对应关系不可信。
    /// </summary>
    Task<MaintenanceRecoveryReport> RecoverProjectMaintenanceAsync(CancellationToken cancellationToken = default);

    Task<CurrentProjectStatus?> GetCurrentProjectAsync(CancellationToken cancellationToken = default);

    Task<ProjectInitReport> CreateProjectAsync(string projectRoot, string? name = null, CancellationToken cancellationToken = default);

    Task<CurrentProjectStatus> OpenProjectAsync(string projectRoot, string? name = null, CancellationToken cancellationToken = default);

    Task SetProjectRootAsync(string projectRoot, CancellationToken cancellationToken = default);

    Task CloseProjectAsync(CancellationToken cancellationToken = default);

    /// <summary>桌面侧是否已打开项目根（未打开时项目页应走空态，勿把 cwd 当项目）。</summary>
    bool HasProjectRoot { get; }

    Task<AppSettings> GetAppSettingsAsync(CancellationToken cancellationToken = default);

    Task<AppSettings> SaveAppSettingsAsync(AppSettings settings, CancellationToken cancellationToken = default);

    Task<GeneralSectionSettings> SaveGeneralSectionSettingsAsync(GeneralSectionSettings settings, CancellationToken cancellationToken = default);

    Task<ProviderConfigStatus> GetProviderConfigAsync(CancellationToken cancellationToken = default);

    Task<ProviderConfigStatus> SaveProviderSettingsAsync(ProviderSettingsUpdate update, CancellationToken cancellationToken = default);

    Task<ProviderConfigStatus> SaveProviderSectionSettingsAsync(ProviderSectionSettings settings, CancellationToken cancellationToken = default);

    Task<ProviderRemovalPreview> PreviewProviderRemovalAsync(string provider, CancellationToken cancellationToken = default);

    Task<ProviderConfigStatus> RemoveProviderAsync(string provider, string expectedRevision, CancellationToken cancellationToken = default);

    Task<ProviderModelsResult> FetchProviderModelsAsync(string? providerId = null, CancellationToken cancellationToken = default);

    Task<ProviderModelsResult> TestProviderDraftAsync(ProviderDraftProbe probe, CancellationToken cancellationToken = default);

    Task<ProviderConfigStatus> SaveProviderKeyAsync(string provider, string key, CancellationToken cancellationToken = default);

    Task<ProviderConfigStatus> RevokeProviderKeyAsync(string provider, CancellationToken cancellationToken = default);

    Task<NodePresetSettings> GetNodePresetSettingsAsync(CancellationToken cancellationToken = default);

    Task<NodePresetSettings> SaveNodePresetSettingsAsync(NodePresetSettings settings, CancellationToken cancellationToken = default);

    Task<AutomationSettings> GetAutomationSettingsAsync(CancellationToken cancellationToken = default);

    Task<AutomationSettings> SaveAutomationSettingsAsync(AutomationSettings settings, CancellationToken cancellationToken = default);

    Task<AutomationSectionSettings> SaveAutomationSectionSettingsAsync(AutomationSectionSettings settings, CancellationToken cancellationToken = default);

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

    Task<AppRuntimeSettings> GetAppRuntimeSettingsAsync(CancellationToken cancellationToken = default);

    Task<AppRuntimeSettings> SaveAppRuntimeSettingsAsync(AppRuntimeSettings settings, CancellationToken cancellationToken = default);

    Task<RagSettings> SaveRagSettingsAsync(RagSettings settings, CancellationToken cancellationToken = default);

    Task<MiscSectionSettings> SaveMiscSectionSettingsAsync(MiscSectionSettings settings, CancellationToken cancellationToken = default);

    Task<IReadOnlyList<TemplateSummary>> SearchTemplatesAsync(
        string baseUrl,
        string query,
        IReadOnlyList<string> tags,
        int page = 0,
        CancellationToken cancellationToken = default);

    Task<TemplateDetail> GetTemplateDetailAsync(string baseUrl, string id, CancellationToken cancellationToken = default);

    Task<TemplateInstallReport> InstallTemplateAsync(
        string baseUrl,
        string id,
        string expectedProjectRoot,
        CancellationToken cancellationToken = default);

    Task<WorkflowRunStarted> RunWorkflowAsync(string workflowId, string? startNodeId = null, IReadOnlyDictionary<string, object?>? variables = null, CancellationToken cancellationToken = default);

    Task<WorkflowActionResult> PauseWorkflowAsync(string workflowId, string runId, string? reason = null, CancellationToken cancellationToken = default);

    Task<WorkflowActionResult> StopWorkflowAsync(string workflowId, string runId, string? reason = null, CancellationToken cancellationToken = default);

    Task<WorkflowActionResult> ResumeWorkflowAsync(string workflowId, string runId, CancellationToken cancellationToken = default);

    Task<WorkflowRunState> GetWorkflowRunStateAsync(string workflowId, string runId, CancellationToken cancellationToken = default);

    Task<WorkflowEventsResult> GetWorkflowEventsAsync(string workflowId, string runId, long afterSequence = 0, int? limit = null, CancellationToken cancellationToken = default);

    Task<IReadOnlyList<WorkflowOperation>> ListInDoubtOperationsAsync(string workflowId, string runId, CancellationToken cancellationToken = default);

    Task<ResolveInDoubtOperationResult> ResolveInDoubtOperationAsync(string operationId, string decision, object? response = null, string? reason = null, CancellationToken cancellationToken = default);

    /// <summary>
    /// 项目 AI 问答。<paramref name="references"/> 走后端 `ProjectAiRequest.references`，
    /// 支持 `@确认项:&lt;id&gt;` / `@文档:…` 等前缀形态——后端会把它们展开成真实内容
    /// 再送进 LLM（`resolve_project_references_with_context`），所以引用必须带前缀，
    /// 裸 id 在顶层解析器那里会因缺少 `:`/`/` 被拒。
    /// </summary>
    Task<ProjectAiResponse> ProjectAiChatAsync(
        string message,
        string? workflowIdToRun = null,
        string? referenceWorkflowId = null,
        string? referenceRunId = null,
        string? conversationId = null,
        long? conversationRevision = null,
        IReadOnlyList<string>? references = null,
        CancellationToken cancellationToken = default);

    Task<ProjectAiResponse> ProjectAiChatAsync(
        string message,
        IReadOnlyList<ProjectAiChatMessage> chatHistory,
        string? workflowIdToRun = null,
        string? referenceWorkflowId = null,
        string? referenceRunId = null,
        string? conversationId = null,
        long? conversationRevision = null,
        IReadOnlyList<string>? references = null,
        CancellationToken cancellationToken = default);

    Task<string> ReadProjectMemoryAsync(CancellationToken cancellationToken = default);

    Task<string> AppendProjectMemoryAsync(string content, CancellationToken cancellationToken = default);

    Task WriteProjectMemoryAsync(string content, CancellationToken cancellationToken = default);

    Task<ProjectReference> ResolveProjectReferenceAsync(string reference, CancellationToken cancellationToken = default);

    Task<IReadOnlyList<WorkflowSummary>> ListWorkflowGraphsAsync(CancellationToken cancellationToken = default);

    Task<WorkflowGraphData> LoadWorkflowGraphAsync(string? workflowId = null, CancellationToken cancellationToken = default);

    Task<WorkflowGraphData> LoadProjectCanvasAsync(CancellationToken cancellationToken = default);

    Task<WorkflowGraphData> SaveWorkflowGraphAsync(WorkflowGraphData graphData, CancellationToken cancellationToken = default);

    Task<WorkflowGraphData> SaveProjectCanvasAsync(WorkflowGraphData graphData, CancellationToken cancellationToken = default);

    Task ValidateWorkflowGraphAsync(WorkflowGraphData graphData, CancellationToken cancellationToken = default);

    Task ApplyNodeDetailPatchAsync(string workflowId, NodeDetailPatch patch, CancellationToken cancellationToken = default);

    Task UpsertCanvasAnnotationAsync(string workflowId, CanvasAnnotation annotation, CancellationToken cancellationToken = default);

    Task SetNodeBreakpointAsync(string workflowId, string nodeId, bool enabled, CancellationToken cancellationToken = default);

    Task<WorkflowSelectionExportData> ExportWorkflowSelectionAsync(string workflowId, IReadOnlyList<string> selectedNodeIds, CancellationToken cancellationToken = default);

    Task<WorkflowPackReport> PackWorkflowSelectionAsync(string workflowId, IReadOnlyList<string> selectedNodeIds, string? subworkflowNodeId = null, string? title = null, string? expectedRevision = null, string? operationId = null, CancellationToken cancellationToken = default);

    Task<WorkflowPackReport> GetPackOperationAsync(string operationId, CancellationToken cancellationToken = default);

    Task<WorksTreeNode> GetWorksTreeAsync(CancellationToken cancellationToken = default);

    /// <summary>
    /// U184-A：项目正文全文检索（Tantivy，走 <c>search_project_documents</c>）。
    ///
    /// <para>此前作品页搜索只比 <c>Title.Contains</c>，而全文检索后端（<c>commands.rs</c>
    /// 的 <c>search_project_documents_impl</c>）与 IPC 分发（<c>ipc.rs</c>）**早就就绪**，
    /// 缺的只是这一根线——百万字项目靠标题找内容是不可能的。</para>
    ///
    /// <para>⚠️ 这条路会因「索引还没追上刚才的保存」而**正常失败**：后端
    /// <c>ensure_search_not_blocked_by_pending_index</c> 在 outbox 有未完成失效项时
    /// 直接返回 <c>validation</c> + 诊断含 <c>indexing_not_ready</c>。
    /// 调用方必须把它当**可重试的暂态**而不是错误（见
    /// <see cref="ViewModels.WorksPageViewModel"/> 的 body-search 分支）——
    /// 弹红色报错会让作者以为搜索功能坏了。</para>
    ///
    /// <para>不做凭据前置校验：后端 <c>embedder</c> 是 <c>Option</c>，没配 Provider 时
    /// 退化为纯 Tantivy 全文，「没配模型也能搜正文」是成立的。</para>
    /// </summary>
    Task<IReadOnlyList<RetrievalHit>> SearchProjectDocumentsAsync(string query, int limit = 20, CancellationToken cancellationToken = default);

    Task<ChapterSummaryView> GetChapterSummaryViewAsync(string chapterId, CancellationToken cancellationToken = default);

    Task<DocumentTreeNode> GetDocumentTreeAsync(string? projectId = null, CancellationToken cancellationToken = default);

    Task<ChapterImportReport> ImportChapterAsync(ChapterImportRequest request, CancellationToken cancellationToken = default);

    /// <summary>
    /// U174：新建一章（空白或带初始正文）。
    ///
    /// 与 <see cref="ImportChapterAsync"/> 并列，区别只在正文来源：导入取自项目外的
    /// 稿件文件，新建取自 <c>InitialContent</c>。此前后端**只有导入**这一条路能让章节
    /// 出现在作品树里，于是「新建一章」要求作者先去项目外手工造一个文件——
    /// 用户报的「一些东西还不能创建」就是这件事。
    ///
    /// ⚠️ 不要改用 <see cref="SaveDocumentContentAsync"/> 来「新建」章节：
    /// 那条路只写文件、**不登记章节索引**，而作品树读的是索引 ⇒ 文件落盘了但
    /// 用户看不见（U174-A 的原始形态）。
    /// </summary>
    Task<ChapterDocumentIndexResult> CreateChapterAsync(ChapterCreateRequest request, CancellationToken cancellationToken = default);

    /// <summary>
    /// 合并导出章节。
    ///
    /// U134：<paramref name="artifactId"/> 传 null 时由**后端**命名
    /// （导出目录 + 作品名 + 本地时间戳 + 真实扩展名）。前端不再自己拼——
    /// 命名依赖后端的路径重定向与写入语义（只认 exports/ 前缀、无回执即直写覆盖），
    /// 前端无从知晓，拼错的表现是文件静默落到 .runtime/artifacts 且互相覆盖。
    /// </summary>
    Task<CombinedExportReport> ExportChaptersAsync(IReadOnlyList<string> selectedChapterIds, string? artifactId = null, string format = "markdown", CancellationToken cancellationToken = default);

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

    Task<IReadOnlyList<UiRunLogEntry>> QueryRunLogsAsync(RunLogQuery query, CancellationToken cancellationToken = default);

    Task<int> MarkRunLogsReadAsync(RunLogQuery filter, CancellationToken cancellationToken = default);

    Task<BudgetStatus> GetBudgetStatusAsync(CancellationToken cancellationToken = default);

    Task<BudgetStatus> UpdateBudgetConfigAsync(double budgetUsd, double? preauthorizedUsd, CancellationToken cancellationToken = default);

    Task SetAutoModeAsync(bool enabled, CancellationToken cancellationToken = default);

    Task<BackendDiagnosticsReport> GetBackendDiagnosticsAsync(CancellationToken cancellationToken = default);

    /// <summary>
    /// U176：读取当前凭据保护状态（后端 <c>get_secret_protection</c>，ipc.rs:923）。
    ///
    /// 设置页据此决定「设主密码 / 接受明文」这两个入口显不显、以及提示哪一种。
    /// </summary>
    Task<SecretProtectionReport> GetSecretProtectionAsync(CancellationToken cancellationToken = default);

    /// <summary>
    /// U176：设置本地主密码，凭据自此以密文落盘（后端 <c>set_local_secret_master_password</c>，ipc.rs:924）。
    ///
    /// 此前后端命令齐备而前端三层全无，于是诊断文案 <c>diagnostics.secrets.locked</c>
    /// 让用户「配置本地主密码」，而配置页根本没有那个入口——用户会在设置页
    /// 反复找一个不存在的开关。新项目**默认**就是 <c>Locked</c>（secrets.rs:584-588），
    /// 所以那是一条装完应用就撞上的死胡同。
    /// </summary>
    Task<SecretProtectionReport> SetLocalSecretMasterPasswordAsync(
        string masterPassword,
        CancellationToken cancellationToken = default);

    /// <summary>
    /// U176：显式接受明文保存（后端 <c>allow_unprotected_local_secrets</c>，ipc.rs:931）。
    ///
    /// **必须是显式动作、且调用点必须带警告**：<c>Unprotected</c> 与 <c>Locked</c>
    /// 刻意分成两个状态（secrets.rs:87-103 写明理由——用户当时同意了明文，
    /// 三个月后未必记得，诊断要能持续把这件事说出来）。做成一个不带警告的
    /// 普通开关，等于让用户在不知情的情况下把 API Key 摊在磁盘上。
    /// </summary>
    Task<SecretProtectionReport> AllowUnprotectedLocalSecretsAsync(
        CancellationToken cancellationToken = default);
}
