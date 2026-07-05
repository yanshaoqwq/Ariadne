using System.Diagnostics;
using System.Text;
using System.Text.Json;

namespace Ariadne.Desktop.Backend;

public sealed class JsonLineBackendClient : IAriadneBackendClient
{
    private readonly string? _backendCommand;
    private readonly JsonSerializerOptions _jsonOptions = new(JsonSerializerDefaults.Web);

    private JsonLineBackendClient(string? backendCommand)
    {
        _backendCommand = backendCommand;
    }

    public static JsonLineBackendClient CreateDefault()
    {
        return new JsonLineBackendClient(Environment.GetEnvironmentVariable("ARIADNE_BACKEND_IPC") ?? DiscoverBackendCommand());
    }

    public Task<IReadOnlyList<RecentProjectEntry>> ListRecentProjectsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeOrEmptyListAsync<RecentProjectEntry>("list_recent_projects", null, cancellationToken);
    }

    public Task<AppStatus?> GetAppStatusAsync(CancellationToken cancellationToken = default)
    {
        return InvokeAsync<AppStatus>("get_app_status", null, cancellationToken);
    }

    public Task<CurrentProjectStatus?> GetCurrentProjectAsync(CancellationToken cancellationToken = default)
    {
        return InvokeAsync<CurrentProjectStatus>("get_current_project", null, cancellationToken);
    }

    public Task<ProjectInitReport> CreateProjectAsync(string projectRoot, string? name = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProjectInitReport>("create_project", new { project_root = projectRoot, name }, cancellationToken);
    }

    public Task<CurrentProjectStatus> OpenProjectAsync(string projectRoot, string? name = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<CurrentProjectStatus>("open_project", new { project_root = projectRoot, name }, cancellationToken);
    }

    public Task SetProjectRootAsync(string projectRoot, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("set_project_root", new { project_root = projectRoot }, cancellationToken);
    }

    public Task<AppSettings> GetAppSettingsAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<AppSettings>("get_app_settings", null, cancellationToken);
    }

    public Task<AppSettings> SaveAppSettingsAsync(AppSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<AppSettings>("save_app_settings", new { settings }, cancellationToken);
    }

    public Task<ProviderConfigStatus> GetProviderConfigAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderConfigStatus>("get_provider_config", null, cancellationToken);
    }

    public Task<ProviderConfigStatus> SaveProviderSettingsAsync(ProviderSettingsUpdate update, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderConfigStatus>("save_provider_settings", new { update }, cancellationToken);
    }

    public Task<ProviderModelsResult> FetchProviderModelsAsync(string? providerId = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProviderModelsResult>("fetch_provider_models", new { provider_id = providerId }, cancellationToken);
    }

    public Task SaveProviderKeyAsync(string provider, string key, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("save_provider_key", new { provider, key }, cancellationToken);
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

    public Task<RagSettings> SaveRagSettingsAsync(RagSettings settings, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<RagSettings>("save_rag_settings", new { settings }, cancellationToken);
    }

    public Task<IReadOnlyList<TemplateSummary>> SearchTemplatesAsync(string baseUrl, string query, int page = 0, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredListAsync<TemplateSummary>("search_templates", new
        {
            request = new { base_url = string.IsNullOrWhiteSpace(baseUrl) ? null : baseUrl },
            query,
            tags = Array.Empty<string>(),
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

    public Task<TemplateInstallReport> InstallTemplateAsync(string baseUrl, string id, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<TemplateInstallReport>("install_template", new
        {
            request = new { base_url = string.IsNullOrWhiteSpace(baseUrl) ? null : baseUrl },
            id,
        }, cancellationToken);
    }

    public Task<WorkflowRunStarted> RunWorkflowAsync(string workflowId, string? startNodeId = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowRunStarted>("run_workflow", new { workflow_id = workflowId, start_node_id = startNodeId }, cancellationToken);
    }

    public Task<WorkflowRunStarted> PauseWorkflowAsync(string workflowId, string runId, string? reason = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowRunStarted>("pause_workflow", new { workflow_id = workflowId, run_id = runId, reason }, cancellationToken);
    }

    public Task<WorkflowRunStarted> StopWorkflowAsync(string workflowId, string runId, string? reason = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowRunStarted>("stop_workflow", new { workflow_id = workflowId, run_id = runId, reason }, cancellationToken);
    }

    public Task<WorkflowRunStarted> ResumeWorkflowAsync(string workflowId, string runId, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowRunStarted>("resume_workflow", new { workflow_id = workflowId, run_id = runId }, cancellationToken);
    }

    public Task<WorkflowRunState> GetWorkflowRunStateAsync(string workflowId, string runId, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowRunState>("get_workflow_run_state", new { workflow_id = workflowId, run_id = runId }, cancellationToken);
    }

    public Task<ProjectAiResponse> ProjectAiChatAsync(string message, string? workflowIdToRun = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ProjectAiResponse>("project_ai_chat", new
        {
            request = new
            {
                message,
                chat_history = Array.Empty<object>(),
                references = Array.Empty<string>(),
                workflow_id_to_run = workflowIdToRun,
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

    public Task<WorkflowGraphData> LoadWorkflowGraphAsync(string? workflowId = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowGraphData>("load_workflow_graph", new { workflow_id = workflowId }, cancellationToken);
    }

    public Task SaveWorkflowGraphAsync(WorkflowGraphData graphData, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("save_workflow_graph", new { graph_data = graphData }, cancellationToken);
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

    public Task<WorkflowGraphData> ExportWorkflowSelectionAsync(string workflowId, IReadOnlyList<string> selectedNodeIds, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowGraphData>("export_workflow_selection", new
        {
            workflow_id = workflowId,
            selected_node_ids = selectedNodeIds,
        }, cancellationToken);
    }

    public Task<WorkflowGraphData> PackWorkflowSelectionAsync(string workflowId, IReadOnlyList<string> selectedNodeIds, string? subworkflowNodeId = null, string? title = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorkflowGraphData>("pack_workflow_selection", new
        {
            workflow_id = workflowId,
            selected_node_ids = selectedNodeIds,
            subworkflow_node_id = subworkflowNodeId,
            title,
        }, cancellationToken);
    }

    public Task<WorksTreeNode> GetWorksTreeAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<WorksTreeNode>("get_works_tree", null, cancellationToken);
    }

    public Task<DocumentTreeNode> GetDocumentTreeAsync(string? projectId = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<DocumentTreeNode>("get_document_tree", new { project_id = projectId }, cancellationToken);
    }

    public Task<ChapterImportReport> ImportChapterAsync(ChapterImportRequest request, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ChapterImportReport>("import_chapter", new { request }, cancellationToken);
    }

    public Task<CombinedExportReport> ExportChaptersAsync(IReadOnlyList<string> selectedChapterIds, string artifactId, string format = "markdown", CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<CombinedExportReport>("export_chapters", new
        {
            selected_chapter_ids = selectedChapterIds,
            artifact_id = artifactId,
            format,
        }, cancellationToken);
    }

    public Task SaveDocumentContentAsync(string documentId, string content, CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("save_document_content", new { document_id = documentId, content }, cancellationToken);
    }

    public Task<string> GetDocumentContentAsync(string documentId, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<string>("get_document_content", new { document_id = documentId }, cancellationToken);
    }

    public Task<string> GetDocumentContentByPathAsync(string path, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<string>("get_document_content", new { path }, cancellationToken);
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

    public Task<ConfirmationLogEntry> ResolveConfirmationAsync(string workflowId, string runId, string confirmationId, string decision, string? reviewReason = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<ConfirmationLogEntry>("resolve_confirmation", new
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

    public Task<IReadOnlyList<UiRunLogEntry>> QueryRunLogsAsync(string? level = null, string? query = null, CancellationToken cancellationToken = default)
    {
        return InvokeRequiredListAsync<UiRunLogEntry>("query_run_logs", new
        {
            filter = new
            {
                level = string.IsNullOrWhiteSpace(level) ? null : level,
                query = string.IsNullOrWhiteSpace(query) ? null : query,
            },
        }, cancellationToken);
    }

    public Task MarkRunLogsReadAsync(CancellationToken cancellationToken = default)
    {
        return InvokeCommandAsync("mark_run_logs_read", null, cancellationToken);
    }

    public Task<BudgetStatus> GetBudgetStatusAsync(CancellationToken cancellationToken = default)
    {
        return InvokeRequiredAsync<BudgetStatus>("get_budget_status", null, cancellationToken);
    }

    public Task<BudgetStatus> UpdateBudgetConfigAsync(double budgetUsd, double preauthorizedUsd, CancellationToken cancellationToken = default)
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
        if (string.IsNullOrWhiteSpace(_backendCommand))
        {
            return default;
        }

        var startInfo = new ProcessStartInfo
        {
            FileName = ResolveCommandFileName(_backendCommand),
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            StandardInputEncoding = Encoding.UTF8,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
        };
        foreach (var argument in ResolveCommandArguments(_backendCommand))
        {
            startInfo.ArgumentList.Add(argument);
        }

        using var process = Process.Start(startInfo);
        if (process is null)
        {
            return default;
        }

        var request = JsonSerializer.Serialize(new { method, @params = parameters ?? new { } }, _jsonOptions);
        await process.StandardInput.WriteLineAsync(request.AsMemory(), cancellationToken).ConfigureAwait(false);
        process.StandardInput.Close();

        var output = await process.StandardOutput.ReadToEndAsync(cancellationToken).ConfigureAwait(false);
        await process.WaitForExitAsync(cancellationToken).ConfigureAwait(false);

        if (process.ExitCode != 0 || string.IsNullOrWhiteSpace(output))
        {
            return default;
        }

        var result = JsonSerializer.Deserialize<BackendResult<T>>(output, _jsonOptions);
        if (result?.Ok != true)
        {
            return default;
        }

        return result.Data;
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
            throw new InvalidOperationException("backend ipc command not found");
        }

        var startInfo = new ProcessStartInfo
        {
            FileName = ResolveCommandFileName(_backendCommand),
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            StandardInputEncoding = Encoding.UTF8,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
        };
        foreach (var argument in ResolveCommandArguments(_backendCommand))
        {
            startInfo.ArgumentList.Add(argument);
        }

        using var process = Process.Start(startInfo);
        if (process is null)
        {
            throw new InvalidOperationException("failed to start backend ipc process");
        }

        var request = JsonSerializer.Serialize(new { method, @params = parameters ?? new { } }, _jsonOptions);
        await process.StandardInput.WriteLineAsync(request.AsMemory(), cancellationToken).ConfigureAwait(false);
        process.StandardInput.Close();

        var output = await process.StandardOutput.ReadToEndAsync(cancellationToken).ConfigureAwait(false);
        var stderr = await process.StandardError.ReadToEndAsync(cancellationToken).ConfigureAwait(false);
        await process.WaitForExitAsync(cancellationToken).ConfigureAwait(false);

        if (process.ExitCode != 0 || string.IsNullOrWhiteSpace(output))
        {
            throw new InvalidOperationException(string.IsNullOrWhiteSpace(stderr) ? "backend ipc returned no response" : stderr.Trim());
        }

        var result = JsonSerializer.Deserialize<BackendResult<T>>(output, _jsonOptions)
            ?? throw new InvalidOperationException("backend ipc returned invalid json");
        if (!result.Ok)
        {
            throw new InvalidOperationException(result.Error ?? "backend command failed");
        }
        return result.Data is null
            ? throw new InvalidOperationException("backend command returned empty data")
            : result.Data;
    }

    private async Task InvokeCommandAsync(
        string method,
        object? parameters,
        CancellationToken cancellationToken)
    {
        if (string.IsNullOrWhiteSpace(_backendCommand))
        {
            throw new InvalidOperationException("backend ipc command not found");
        }

        var startInfo = new ProcessStartInfo
        {
            FileName = ResolveCommandFileName(_backendCommand),
            RedirectStandardInput = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            UseShellExecute = false,
            StandardInputEncoding = Encoding.UTF8,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
        };
        foreach (var argument in ResolveCommandArguments(_backendCommand))
        {
            startInfo.ArgumentList.Add(argument);
        }

        using var process = Process.Start(startInfo);
        if (process is null)
        {
            throw new InvalidOperationException("failed to start backend ipc process");
        }

        var request = JsonSerializer.Serialize(new { method, @params = parameters ?? new { } }, _jsonOptions);
        await process.StandardInput.WriteLineAsync(request.AsMemory(), cancellationToken).ConfigureAwait(false);
        process.StandardInput.Close();

        var output = await process.StandardOutput.ReadToEndAsync(cancellationToken).ConfigureAwait(false);
        var stderr = await process.StandardError.ReadToEndAsync(cancellationToken).ConfigureAwait(false);
        await process.WaitForExitAsync(cancellationToken).ConfigureAwait(false);

        if (process.ExitCode != 0 || string.IsNullOrWhiteSpace(output))
        {
            throw new InvalidOperationException(string.IsNullOrWhiteSpace(stderr) ? "backend ipc returned no response" : stderr.Trim());
        }

        var result = JsonSerializer.Deserialize<BackendResult<object>>(output, _jsonOptions)
            ?? throw new InvalidOperationException("backend ipc returned invalid json");
        if (!result.Ok)
        {
            throw new InvalidOperationException(result.Error ?? "backend command failed");
        }
    }

    private static string? DiscoverBackendCommand()
    {
        var candidates = new[]
        {
            Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "..", "..", "..", "..", "core", "target", "debug", "ariadne-ipc")),
            Path.GetFullPath(Path.Combine(Environment.CurrentDirectory, "core", "target", "debug", "ariadne-ipc")),
            Path.GetFullPath(Path.Combine(Environment.CurrentDirectory, "target", "debug", "ariadne-ipc")),
        };

        return candidates.FirstOrDefault(File.Exists);
    }

    private static string ResolveCommandFileName(string command)
    {
        return command.Split(' ', StringSplitOptions.RemoveEmptyEntries).FirstOrDefault() ?? command;
    }

    private static IEnumerable<string> ResolveCommandArguments(string command)
    {
        return command.Split(' ', StringSplitOptions.RemoveEmptyEntries).Skip(1);
    }
}
