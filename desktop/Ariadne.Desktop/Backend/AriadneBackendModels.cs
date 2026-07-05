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

public sealed record UiPreferences(
    [property: JsonPropertyName("theme")] string Theme,
    [property: JsonPropertyName("git_auto_color")] string GitAutoColor,
    [property: JsonPropertyName("git_manual_color")] string GitManualColor,
    [property: JsonPropertyName("project_panel_visible")] bool ProjectPanelVisible,
    [property: JsonPropertyName("project_panel_position")] int[]? ProjectPanelPosition,
    [property: JsonPropertyName("panel_states")] Dictionary<string, bool> PanelStates,
    [property: JsonPropertyName("onboarding_seen")] bool OnboardingSeen);

public sealed record AppStatus(
    [property: JsonPropertyName("current_project")] CurrentProjectStatus CurrentProject,
    [property: JsonPropertyName("badges")] SidebarBadgeCounts Badges,
    [property: JsonPropertyName("preferences")] UiPreferences Preferences);

public sealed record BackendResult<T>(
    [property: JsonPropertyName("ok")] bool Ok,
    [property: JsonPropertyName("data")] T? Data,
    [property: JsonPropertyName("error")] string? Error);
