using System.Text.Json;
using System.Text.Json.Serialization;

namespace Ariadne.Desktop.Backend;

public sealed record RecentProjectEntry(
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("project_root")] string ProjectRoot,
    [property: JsonPropertyName("last_opened_at")] string? LastOpenedAt);

public sealed record CurrentProjectStatus(
    [property: JsonPropertyName("project_root")] string ProjectRoot,
    [property: JsonPropertyName("project_name")] string ProjectName);

public sealed record SidebarBadgeCounts(
    [property: JsonPropertyName("confirmations")] int Confirmations,
    [property: JsonPropertyName("run_logs")] int RunLogs,
    [property: JsonPropertyName("diagnostics")] int Diagnostics);

/// <summary>D3：项目维护门禁状态（git restore / 全量重建）。</summary>
public sealed record ProjectMaintenanceState(
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("phase")] string Phase,
    [property: JsonPropertyName("error")] string? Error);

public sealed record UiPreferences(
    [property: JsonPropertyName("theme")] string Theme,
    [property: JsonPropertyName("git_auto_color")] string GitAutoColor,
    [property: JsonPropertyName("git_manual_color")] string GitManualColor,
    [property: JsonPropertyName("project_panel_visible")] bool ProjectPanelVisible,
    [property: JsonPropertyName("project_panel_position")] int[]? ProjectPanelPosition,
    [property: JsonPropertyName("panel_states")] Dictionary<string, bool> PanelStates,
    [property: JsonPropertyName("onboarding_seen")] bool OnboardingSeen,
    /// 主题主底色（昼）；空/缺省 = 用主题预设。
    [property: JsonPropertyName("theme_main_color")] string? ThemeMainColor = null,
    /// 主题表面色（昼）。
    [property: JsonPropertyName("theme_surface_color")] string? ThemeSurfaceColor = null,
    /// 主题强调色（昼）。
    [property: JsonPropertyName("theme_brand_color")] string? ThemeBrandColor = null,
    /// 夜·主底（跟随系统时使用）。
    [property: JsonPropertyName("theme_main_color_dark")] string? ThemeMainColorDark = null,
    /// 夜·表面。
    [property: JsonPropertyName("theme_surface_color_dark")] string? ThemeSurfaceColorDark = null,
    /// 夜·强调。
    [property: JsonPropertyName("theme_brand_color_dark")] string? ThemeBrandColorDark = null,
    /// 自定义三色是否按系统明暗分别应用昼/夜。
    [property: JsonPropertyName("theme_follow_system_colors")] bool ThemeFollowSystemColors = true);

public sealed record AppStatus(
    [property: JsonPropertyName("current_project")] CurrentProjectStatus CurrentProject,
    [property: JsonPropertyName("badges")] SidebarBadgeCounts Badges,
    [property: JsonPropertyName("preferences")] UiPreferences Preferences);

public sealed record BackendResult<T>(
    [property: JsonPropertyName("ok")] bool Ok,
    [property: JsonPropertyName("data")] T? Data,
    [property: JsonPropertyName("error")] string? Error,
    [property: JsonPropertyName("error_code")] string? ErrorCode = null,
    [property: JsonPropertyName("error_key")] string? ErrorKey = null);

public sealed record ProjectInitReport(
    [property: JsonPropertyName("project_root")] string ProjectRoot,
    [property: JsonPropertyName("created_dirs")] IReadOnlyList<string> CreatedDirs,
    [property: JsonPropertyName("git_initialized")] bool GitInitialized);

public sealed record AppSettings(
    [property: JsonPropertyName("app")] AppConfig App);

public sealed record AppConfig(
    [property: JsonPropertyName("schema_version")] int SchemaVersion,
    [property: JsonPropertyName("project_name")] string ProjectName,
    [property: JsonPropertyName("locale")] string Locale,
    [property: JsonPropertyName("documents_dir")] string DocumentsDir,
    [property: JsonPropertyName("workflows_dir")] string WorkflowsDir,
    [property: JsonPropertyName("skills_dir")] string SkillsDir,
    [property: JsonPropertyName("exports_dir")] string ExportsDir);

public sealed record ProviderConfigStatus(
    [property: JsonPropertyName("has_openai_key")] bool HasOpenAiKey,
    [property: JsonPropertyName("has_anthropic_key")] bool HasAnthropicKey,
    [property: JsonPropertyName("has_gemini_key")] bool HasGeminiKey,
    [property: JsonPropertyName("default_llm_provider_id")] string? DefaultLlmProviderId,
    [property: JsonPropertyName("default_embedding_provider_id")] string? DefaultEmbeddingProviderId,
    [property: JsonPropertyName("default_reranker_provider_id")] string? DefaultRerankerProviderId,
    [property: JsonPropertyName("providers")] IReadOnlyList<ProviderKeyStatus> Providers);

public sealed record ProviderKeyStatus(
    [property: JsonPropertyName("provider")] string Provider,
    [property: JsonPropertyName("display_name")] string DisplayName,
    [property: JsonPropertyName("provider_type")] string ProviderType,
    [property: JsonPropertyName("enabled")] bool Enabled,
    [property: JsonPropertyName("base_url")] string? BaseUrl,
    [property: JsonPropertyName("models")] IReadOnlyList<ModelConfig> Models,
    [property: JsonPropertyName("has_key")] bool HasKey);

public sealed record ProviderSettingsUpdate(
    [property: JsonPropertyName("provider_id")] string ProviderId,
    [property: JsonPropertyName("provider_type")] string ProviderType,
    [property: JsonPropertyName("display_name")] string DisplayName,
    [property: JsonPropertyName("enabled")] bool Enabled,
    [property: JsonPropertyName("base_url")] string? BaseUrl,
    [property: JsonPropertyName("models")] IReadOnlyList<ModelConfig> Models,
    [property: JsonPropertyName("make_default_llm")] bool MakeDefaultLlm,
    [property: JsonPropertyName("make_default_embedding")] bool MakeDefaultEmbedding,
    [property: JsonPropertyName("make_default_reranker")] bool MakeDefaultReranker);

public sealed record ModelConfig(
    [property: JsonPropertyName("model_id")] string ModelId,
    [property: JsonPropertyName("capability")] string Capability,
    [property: JsonPropertyName("max_context_tokens")] int? MaxContextTokens,
    [property: JsonPropertyName("input_cost_per_million_tokens")] double? InputCostPerMillionTokens,
    [property: JsonPropertyName("output_cost_per_million_tokens")] double? OutputCostPerMillionTokens);

public sealed record ProviderModelsResult(
    [property: JsonPropertyName("provider_id")] string ProviderId,
    [property: JsonPropertyName("models")] IReadOnlyList<ModelConfig> Models);

public sealed record BudgetStatus(
    [property: JsonPropertyName("budget_usd")] double BudgetUsd,
    [property: JsonPropertyName("spent_usd")] double SpentUsd,
    [property: JsonPropertyName("preauthorized_usd")] double PreauthorizedUsd,
    [property: JsonPropertyName("auto_mode_enabled")] bool AutoModeEnabled);

public sealed record AutomationSettings(
    [property: JsonPropertyName("budget")] BudgetStatus Budget,
    [property: JsonPropertyName("confirmation_policies")] IReadOnlyList<ConfirmationPolicySetting> ConfirmationPolicies);

public sealed record ConfirmationPolicySetting(
    [property: JsonPropertyName("confirmation_kind")] string ConfirmationKind,
    [property: JsonPropertyName("normal_policy")] string NormalPolicy,
    [property: JsonPropertyName("auto_mode_policy")] string AutoModePolicy);

public sealed record PermissionsSettings(
    [property: JsonPropertyName("policy")] PermissionPolicy Policy,
    [property: JsonPropertyName("tool_controls")] IReadOnlyDictionary<string, IReadOnlyDictionary<string, bool>> ToolControls);

public sealed record PermissionPolicy(
    [property: JsonPropertyName("allow_network")] bool AllowNetwork,
    [property: JsonPropertyName("allow_web_search")] bool AllowWebSearch,
    [property: JsonPropertyName("allow_http_skill")] bool AllowHttpSkill,
    [property: JsonPropertyName("allow_wasm_network")] bool AllowWasmNetwork,
    [property: JsonPropertyName("allow_secret_read")] bool AllowSecretRead,
    [property: JsonPropertyName("writable_file_roots")] IReadOnlyList<string> WritableFileRoots,
    [property: JsonPropertyName("readable_file_roots")] IReadOnlyList<string> ReadableFileRoots);

public sealed record NodePresetSettings(
    [property: JsonPropertyName("presets")] IReadOnlyList<NodeTypePreset> Presets,
    [property: JsonPropertyName("default_model_id")] string DefaultModelId,
    [property: JsonPropertyName("default_timeout_ms")] long DefaultTimeoutMs,
    [property: JsonPropertyName("default_budget_usd")] double DefaultBudgetUsd);

public sealed record NodeTypePreset(
    [property: JsonPropertyName("node_type")] string NodeType,
    [property: JsonPropertyName("display_name_key")] string DisplayNameKey,
    [property: JsonPropertyName("model_id")] string ModelId,
    [property: JsonPropertyName("timeout_ms")] long TimeoutMs,
    [property: JsonPropertyName("budget_usd")] double BudgetUsd);

public sealed record TemplateRepositorySettings(
    [property: JsonPropertyName("base_url")] string BaseUrl);

public sealed record WorkflowSettings(
    [property: JsonPropertyName("workflow")] WorkflowConfig Workflow);

public sealed record WorkflowConfig(
    [property: JsonPropertyName("schema_version")] int SchemaVersion,
    [property: JsonPropertyName("default_timeout_ms")] long DefaultTimeoutMs,
    [property: JsonPropertyName("max_loop_iterations")] int MaxLoopIterations,
    [property: JsonPropertyName("max_tool_rounds")] int MaxToolRounds,
    [property: JsonPropertyName("checkpoint_enabled")] bool CheckpointEnabled,
    [property: JsonPropertyName("runtime_autosave_ms")] long RuntimeAutosaveMs);

public sealed record GitSettings(
    [property: JsonPropertyName("git")] GitConfig Git);

public sealed record GitConfig(
    [property: JsonPropertyName("schema_version")] int SchemaVersion,
    [property: JsonPropertyName("track_documents")] bool TrackDocuments,
    [property: JsonPropertyName("track_workflows")] bool TrackWorkflows,
    [property: JsonPropertyName("track_skills")] bool TrackSkills,
    [property: JsonPropertyName("track_non_sensitive_config")] bool TrackNonSensitiveConfig,
    [property: JsonPropertyName("ignored_paths")] IReadOnlyList<string> IgnoredPaths);

public sealed record RagSettings(
    [property: JsonPropertyName("rag")] RagConfig Rag);

public sealed record RagConfig(
    [property: JsonPropertyName("schema_version")] int SchemaVersion,
    [property: JsonPropertyName("vector_store")] VectorStoreConfig VectorStore,
    [property: JsonPropertyName("full_text_store")] FullTextStoreConfig FullTextStore,
    [property: JsonPropertyName("reranker_enabled")] bool RerankerEnabled,
    [property: JsonPropertyName("chunk_size_chars")] int ChunkSizeChars,
    [property: JsonPropertyName("chunk_overlap_chars")] int ChunkOverlapChars);

public sealed record VectorStoreConfig(
    [property: JsonPropertyName("enabled")] bool Enabled,
    [property: JsonPropertyName("backend")] string Backend,
    [property: JsonPropertyName("collection")] string Collection,
    [property: JsonPropertyName("vector_dimensions")] int VectorDimensions,
    [property: JsonPropertyName("sidecar")] SidecarConfig Sidecar);

public sealed record SidecarConfig(
    [property: JsonPropertyName("host")] string Host,
    [property: JsonPropertyName("port")] int Port,
    [property: JsonPropertyName("data_dir")] string DataDir,
    [property: JsonPropertyName("binary_path")] string BinaryPath,
    [property: JsonPropertyName("startup_timeout_ms")] long StartupTimeoutMs);

public sealed record FullTextStoreConfig(
    [property: JsonPropertyName("backend")] string Backend,
    [property: JsonPropertyName("index_dir")] string IndexDir);

public sealed record TemplateSummary(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("tags")] IReadOnlyList<string> Tags,
    [property: JsonPropertyName("requires_permissions")] bool RequiresPermissions);

public sealed record TemplateDetail(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("version")] string Version,
    [property: JsonPropertyName("manifest")] object? Manifest,
    [property: JsonPropertyName("requires_permissions")] bool RequiresPermissions);

public sealed record TemplateInstallReport(
    [property: JsonPropertyName("workflow_id")] string WorkflowId,
    [property: JsonPropertyName("version")] string Version,
    [property: JsonPropertyName("manifest_path")] string ManifestPath,
    [property: JsonPropertyName("requires_permissions")] bool RequiresPermissions,
    [property: JsonPropertyName("required_permissions")] IReadOnlyList<string> RequiredPermissions);

public sealed record WorkflowRunStarted(
    [property: JsonPropertyName("run_id")] string RunId,
    [property: JsonPropertyName("status")] string Status);

public sealed record WorkflowActionResult(
    [property: JsonPropertyName("workflow_id")] string WorkflowId,
    [property: JsonPropertyName("run_id")] string RunId,
    [property: JsonPropertyName("status")] string Status);

public sealed record WorkflowRunFailure(
    [property: JsonPropertyName("code")] string Code,
    [property: JsonPropertyName("stage")] string Stage,
    [property: JsonPropertyName("message")] string Message,
    [property: JsonPropertyName("recovery_suggestion")] string RecoverySuggestion);

public sealed record WorkflowRunState(
    [property: JsonPropertyName("workflow_id")] string WorkflowId,
    [property: JsonPropertyName("run_id")] string RunId,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("pause_reason")] string? PauseReason,
    [property: JsonPropertyName("stop_reason")] string? StopReason,
    [property: JsonPropertyName("failure")] WorkflowRunFailure? Failure,
    [property: JsonPropertyName("events")] IReadOnlyList<string> Events);

public sealed record WorkflowRuntimeEvent(
    [property: JsonPropertyName("sequence")] long Sequence,
    [property: JsonPropertyName("event_type")] string EventType,
    [property: JsonPropertyName("node_id")] string? NodeId,
    [property: JsonPropertyName("message")] string Message,
    [property: JsonPropertyName("metadata")] object? Metadata);

public sealed record WorkflowEventsResult(
    [property: JsonPropertyName("workflow_id")] string WorkflowId,
    [property: JsonPropertyName("run_id")] string RunId,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("next_sequence")] long NextSequence,
    [property: JsonPropertyName("events")] IReadOnlyList<WorkflowRuntimeEvent> Events);

public sealed record WorkflowOperation(
    [property: JsonPropertyName("operation_id")] string OperationId,
    [property: JsonPropertyName("workflow_id")] string WorkflowId,
    [property: JsonPropertyName("run_id")] string RunId,
    [property: JsonPropertyName("node_id")] string NodeId,
    [property: JsonPropertyName("attempt")] int Attempt,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("provider")] string Provider,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("in_doubt_at_ms")] long? InDoubtAtMs);

public sealed record ResolveInDoubtOperationResult(
    [property: JsonPropertyName("operation_id")] string OperationId,
    [property: JsonPropertyName("decision")] string Decision,
    [property: JsonPropertyName("workflow")] WorkflowActionResult Workflow);

public sealed record ProjectAiResponse(
    [property: JsonPropertyName("answer")] string Answer,
    [property: JsonPropertyName("chat_history")] IReadOnlyList<ProjectAiChatMessage> ChatHistory,
    [property: JsonPropertyName("workflow_run")] WorkflowRunStarted? WorkflowRun,
    [property: JsonPropertyName("project_memory")] string ProjectMemory);

public sealed record ProjectAiChatMessage(
    [property: JsonPropertyName("role")] string Role,
    [property: JsonPropertyName("content")] string Content);

public sealed record WorksTreeNode(
    [property: JsonPropertyName("node_id")] string NodeId,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("path")] string Path,
    [property: JsonPropertyName("children")] IReadOnlyList<WorksTreeNode> Children,
    [property: JsonPropertyName("chapter_id")] string? ChapterId = null,
    [property: JsonPropertyName("stage_id")] string? StageId = null);

public sealed record ChapterSummaryView(
    [property: JsonPropertyName("chapter_id")] string ChapterId,
    [property: JsonPropertyName("chapter_summary")] string? ChapterSummary,
    [property: JsonPropertyName("stage")] ChapterStageSummaryView? Stage,
    [property: JsonPropertyName("segments")] IReadOnlyList<StorySegmentView> Segments,
    [property: JsonPropertyName("events")] IReadOnlyList<StoryEventView> Events,
    [property: JsonPropertyName("realized_changes")] IReadOnlyList<RegisteredChangeView> RealizedChanges,
    [property: JsonPropertyName("foreshadowing")] IReadOnlyList<ForeshadowingView> Foreshadowing,
    [property: JsonPropertyName("confirmations")] IReadOnlyList<ChapterSummaryConfirmationView> Confirmations);

public sealed record ChapterStageSummaryView(
    [property: JsonPropertyName("stage_id")] string StageId,
    [property: JsonPropertyName("summary")] string? Summary,
    [property: JsonPropertyName("chapter_ids")] IReadOnlyList<string> ChapterIds);

public sealed record StorySegmentView(
    [property: JsonPropertyName("segment_id")] string SegmentId,
    [property: JsonPropertyName("number")] string Number,
    [property: JsonPropertyName("chapter_id")] string ChapterId,
    [property: JsonPropertyName("summary")] string Summary,
    [property: JsonPropertyName("source")] WritingSourceSpan Source);

public sealed record WritingSourceSpan(
    [property: JsonPropertyName("document_id")] string DocumentId,
    [property: JsonPropertyName("range")] TextRange Range,
    [property: JsonPropertyName("version")] string? Version);

public sealed record StoryEventView(
    [property: JsonPropertyName("event_id")] string EventId,
    [property: JsonPropertyName("summary")] string Summary,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("segment_ids")] IReadOnlyList<string> SegmentIds,
    [property: JsonPropertyName("chapter_ids")] IReadOnlyList<string> ChapterIds);

public sealed record RegisteredChangeView(
    [property: JsonPropertyName("change_id")] string ChangeId,
    [property: JsonPropertyName("function")] string Function,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("content")] JsonElement Content,
    [property: JsonPropertyName("linked_segment_ids")] IReadOnlyList<string> LinkedSegmentIds);

public sealed record ForeshadowingView(
    [property: JsonPropertyName("foreshadowing_id")] string ForeshadowingId,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("description")] string Description,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("planted_segment_ids")] IReadOnlyList<string> PlantedSegmentIds,
    [property: JsonPropertyName("recovered_segment_ids")] IReadOnlyList<string> RecoveredSegmentIds);

public sealed record ChapterSummaryConfirmationView(
    [property: JsonPropertyName("confirmation_id")] string ConfirmationId,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("state")] string State,
    [property: JsonPropertyName("revision_id")] string? RevisionId);

public sealed record DocumentTreeNode(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("path")] string Path,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("children")] IReadOnlyList<DocumentTreeNode> Children);

public sealed record DocumentMetadata(
    [property: JsonPropertyName("document_id")] string DocumentId,
    [property: JsonPropertyName("path")] string Path,
    [property: JsonPropertyName("format")] string Format,
    [property: JsonPropertyName("media_type")] string MediaType,
    [property: JsonPropertyName("size_bytes")] long SizeBytes,
    [property: JsonPropertyName("version")] string Version);

public sealed record DocumentContentResult(
    [property: JsonPropertyName("metadata")] DocumentMetadata Metadata,
    [property: JsonPropertyName("content")] string Content);

public sealed record DocumentWriteReport(
    [property: JsonPropertyName("metadata")] DocumentMetadata Metadata,
    [property: JsonPropertyName("index_invalidation")] object? IndexInvalidation);

public sealed record ChapterImportRequest(
    [property: JsonPropertyName("chapter_id")] string ChapterId,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("order")] long Order,
    [property: JsonPropertyName("source_path")] string SourcePath,
    [property: JsonPropertyName("target_path")] string TargetPath);

public sealed record ChapterImportReport(
    [property: JsonPropertyName("entry")] object? Entry,
    [property: JsonPropertyName("index_invalidation")] object? IndexInvalidation);

public sealed record ProjectReference(
    [property: JsonPropertyName("reference")] string Reference,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("summary")] string Summary,
    [property: JsonPropertyName("payload")] object? Payload);

public sealed record QuickEditRequest(
    [property: JsonPropertyName("selected_text")] string SelectedText,
    [property: JsonPropertyName("instruction")] string Instruction,
    [property: JsonPropertyName("context_ref")] string? ContextRef);

public sealed record QuickEditResult(
    [property: JsonPropertyName("original")] string Original,
    [property: JsonPropertyName("suggested")] string Suggested,
    [property: JsonPropertyName("diff")] string Diff);

public sealed record TextRange(
    [property: JsonPropertyName("start")] long Start,
    [property: JsonPropertyName("end")] long End);

public sealed record PatchApplyReport(
    [property: JsonPropertyName("preview")] object? Preview,
    [property: JsonPropertyName("metadata")] DocumentMetadata? Metadata,
    [property: JsonPropertyName("index_invalidation")] object? IndexInvalidation);

public sealed record WorkflowGraphData(
    [property: JsonPropertyName("workflow_id")] string WorkflowId,
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("nodes")] IReadOnlyList<CanvasNode> Nodes,
    [property: JsonPropertyName("edges")] IReadOnlyList<CanvasEdge> Edges,
    [property: JsonPropertyName("metadata")] Dictionary<string, object?> Metadata,
    [property: JsonPropertyName("content_revision")] string? ContentRevision = null,
    [property: JsonPropertyName("expected_revision")] string? ExpectedRevision = null);

public sealed record WorkflowPortEndpoint(
    [property: JsonPropertyName("node_id")] string NodeId,
    [property: JsonPropertyName("port_name")] string PortName);

/// <summary>与 Rust WorkflowPackReport 保持同构，避免把顶层报告误当成工作流图。</summary>
public sealed record WorkflowPackReport(
    [property: JsonPropertyName("workflow")] WorkflowGraphData Workflow,
    [property: JsonPropertyName("subworkflow_node_id")] string SubworkflowNodeId,
    [property: JsonPropertyName("embedded_workflow")] WorkflowGraphData EmbeddedWorkflow,
    [property: JsonPropertyName("boundary_inputs")] IReadOnlyList<WorkflowPortEndpoint> BoundaryInputs,
    [property: JsonPropertyName("boundary_outputs")] IReadOnlyList<WorkflowPortEndpoint> BoundaryOutputs,
    [property: JsonPropertyName("operation_id")] string? OperationId = null);

public sealed record WorkflowSummary(
    [property: JsonPropertyName("workflow_id")] string WorkflowId,
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("path")] string Path,
    [property: JsonPropertyName("node_count")] int NodeCount,
    [property: JsonPropertyName("edge_count")] int EdgeCount);

public sealed record CanvasNode(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("type")] string Type,
    [property: JsonPropertyName("label")] string? Label,
    [property: JsonPropertyName("data")] Dictionary<string, object?> Data,
    [property: JsonPropertyName("position")] CanvasPosition? Position);

public sealed record CanvasPosition(
    [property: JsonPropertyName("x")] double X,
    [property: JsonPropertyName("y")] double Y);

public sealed record CanvasEdge(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("source")] string Source,
    [property: JsonPropertyName("target")] string Target,
    [property: JsonPropertyName("source_handle")] string SourceHandle,
    [property: JsonPropertyName("target_handle")] string TargetHandle,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("label")] string? Label,
    [property: JsonPropertyName("data")] object? Data);

public sealed record NodeDetailPatch(
    [property: JsonPropertyName("node_id")] string NodeId,
    [property: JsonPropertyName("prompt_template")] string? PromptTemplate,
    [property: JsonPropertyName("input_aliases")] Dictionary<string, string> InputAliases,
    [property: JsonPropertyName("tool_enabled")] Dictionary<string, bool> ToolEnabled,
    [property: JsonPropertyName("approval_policy")] Dictionary<string, string> ApprovalPolicy,
    [property: JsonPropertyName("model_id")] string? ModelId,
    [property: JsonPropertyName("budget_usd")] double? BudgetUsd,
    [property: JsonPropertyName("timeout_ms")] long? TimeoutMs);

public sealed record CanvasAnnotation(
    [property: JsonPropertyName("annotation_id")] string AnnotationId,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("node_ids")] IReadOnlyList<string> NodeIds,
    [property: JsonPropertyName("metadata")] Dictionary<string, object?> Metadata);

public sealed record CombinedExportReport(
    [property: JsonPropertyName("artifact_id")] string ArtifactId,
    [property: JsonPropertyName("format")] string Format,
    [property: JsonPropertyName("exported_chapter_ids")] IReadOnlyList<string> ExportedChapterIds,
    [property: JsonPropertyName("document_ids")] IReadOnlyList<string> DocumentIds,
    [property: JsonPropertyName("storage_uri")] string StorageUri,
    [property: JsonPropertyName("size_bytes")] long? SizeBytes);

public sealed record ArchivePoint(
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("commit_id")] string CommitId,
    [property: JsonPropertyName("message")] string Message,
    [property: JsonPropertyName("checkpoint_kind")] string CheckpointKind);

public sealed record GitCommitSummary(
    [property: JsonPropertyName("commit_id")] string CommitId,
    [property: JsonPropertyName("summary")] string Summary,
    [property: JsonPropertyName("checkpoint_kind")] string? CheckpointKind);

public sealed record GitRepositoryStatus(
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("branch")] string? Branch,
    [property: JsonPropertyName("head")] string? Head,
    [property: JsonPropertyName("dirty")] bool Dirty,
    [property: JsonPropertyName("reason")] string? Reason,
    [property: JsonPropertyName("diff_line_count")] int DiffLineCount,
    [property: JsonPropertyName("diff_preview")] string DiffPreview);

public sealed record BranchGraphNode(
    [property: JsonPropertyName("commit_id")] string CommitId,
    [property: JsonPropertyName("parents")] IReadOnlyList<string> Parents,
    [property: JsonPropertyName("refs")] IReadOnlyList<string> Refs,
    [property: JsonPropertyName("summary")] string Summary);

public sealed record RestoreReport(
    [property: JsonPropertyName("new_branch")] string NewBranch,
    [property: JsonPropertyName("base_commit")] string BaseCommit,
    [property: JsonPropertyName("index_rebuild_required")] bool IndexRebuildRequired,
    [property: JsonPropertyName("runtime_rebind_required")] bool RuntimeRebindRequired);

public sealed record UiRunLogEntry(
    [property: JsonPropertyName("log_id")] string LogId,
    [property: JsonPropertyName("timestamp_ms")] long TimestampMs,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("level")] string Level,
    [property: JsonPropertyName("message")] string Message);

public sealed record ConfirmationLogEntry(
    [property: JsonPropertyName("confirmation_id")] string ConfirmationId,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("node_id")] string NodeId,
    [property: JsonPropertyName("timestamp_ms")] long TimestampMs,
    [property: JsonPropertyName("state")] string State,
    [property: JsonPropertyName("handling_method")] string HandlingMethod,
    [property: JsonPropertyName("summary")] string Summary,
    [property: JsonPropertyName("diff")] string Diff,
    [property: JsonPropertyName("workflow_id")] string? WorkflowId = null,
    [property: JsonPropertyName("run_id")] string? RunId = null);

public sealed record ResolveConfirmationResult(
    [property: JsonPropertyName("workflow")] WorkflowActionResult Workflow,
    [property: JsonPropertyName("confirmation")] ConfirmationLogEntry Confirmation,
    [property: JsonPropertyName("badges")] SidebarBadgeCounts Badges);

public sealed record DiagnosticItem(
    [property: JsonPropertyName("component")] string Component,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("reason")] string? Reason);

public sealed record BackendDiagnosticsReport(
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("items")] IReadOnlyList<DiagnosticItem> Items);
