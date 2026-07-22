namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 设置页页签与章节导航的单一目录。
/// </summary>
public static class SettingsNavigationCatalog
{
    public static readonly IReadOnlyList<SettingsTabDefinition> Tabs =
    [
        new("general", "ui.settings.tab.general"),
        new("models", "ui.settings.tab.models"),
        new("presets", "ui.settings.tab.presets"),
        new("automation", "ui.settings.tab.automation"),
        new("permissions", "ui.settings.tab.permissions"),
        new("personalization", "ui.settings.tab.personalization"),
        new("misc", "ui.settings.tab.misc"),
    ];

    public static readonly IReadOnlyList<SettingsSectionDefinition> Sections =
    [
        new("project", "general", "ProjectSectionAnchor", "ui.settings.section.project"),
        new("directories", "general", "DirectoriesSectionAnchor", "ui.settings.section.directories"),
        new("project_memory", "general", "ProjectMemorySectionAnchor", "ui.settings.section.project_memory"),
        new("provider", "models", "ProviderSectionAnchor", "ui.settings.section.provider"),
        new("available_models", "models", "AvailableModelsSectionAnchor", "ui.settings.section.available_models"),
        new("embedding", "models", "EmbeddingSectionAnchor", "ui.settings.section.embedding"),
        new("manual_models", "models", "ManualModelsSectionAnchor", "ui.settings.section.manual_fallback"),
        new("node_presets", "presets", "NodePresetsSectionAnchor", "ui.settings.section.node_presets"),
        new("defaults", "presets", "DefaultsSectionAnchor", "ui.settings.section.defaults"),
        new("templates", "presets", "TemplatesSectionAnchor", "ui.settings.section.templates"),
        new("budget", "automation", "BudgetSectionAnchor", "ui.settings.section.budget"),
        new("confirmations", "automation", "ConfirmationsSectionAnchor", "ui.settings.section.confirmations"),
        new("runtime", "automation", "RuntimeSectionAnchor", "ui.settings.section.runtime"),
        new("capabilities", "permissions", "CapabilitiesSectionAnchor", "ui.settings.section.capabilities"),
        new("tool_controls", "permissions", "ToolControlsSectionAnchor", "ui.settings.section.tool_controls"),
        new("paths", "permissions", "PathsSectionAnchor", "ui.settings.section.paths"),
        new("language", "personalization", "LanguageSectionAnchor", "ui.settings.section.language"),
        new("theme", "personalization", "ThemeSectionAnchor", "ui.settings.section.theme"),
        new("workspace", "personalization", "WorkspaceSectionAnchor", "ui.settings.section.workspace"),
        new("tutorial", "misc", "TutorialSectionAnchor", "ui.settings.index.tutorial"),
        new("app_runtime", "misc", "AppRuntimeSectionAnchor", "ui.settings.section.app_runtime"),
        new("retrieval", "misc", "RetrievalSectionAnchor", "ui.settings.section.retrieval"),
        new("git", "misc", "GitSectionAnchor", "ui.settings.section.git"),
        new("diagnostics", "misc", "DiagnosticsSectionAnchor", "ui.settings.section.diagnostics"),
    ];

}

public sealed record SettingsTabDefinition(string Id, string DisplayNameKey);

public sealed record SettingsSectionDefinition(
    string Id,
    string TabId,
    string AnchorName,
    string DisplayNameKey);
