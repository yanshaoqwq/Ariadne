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
        new("retrieval", "ui.settings.tab.retrieval"),
        new("version_control", "ui.settings.tab.version_control"),
        new("support", "ui.settings.tab.support"),
    ];

    public static readonly IReadOnlyList<SettingsSectionDefinition> Sections =
    [
        new("project", "general", "ProjectSectionAnchor", "ui.settings.section.project"),
        new("directories", "general", "DirectoriesSectionAnchor", "ui.settings.section.directories"),
        new("project_memory", "general", "ProjectMemorySectionAnchor", "ui.settings.section.project_memory"),
        new("provider", "models", "ProviderSectionAnchor", "ui.settings.section.provider"),
        new("available_models", "models", "AvailableModelsSectionAnchor", "ui.settings.section.available_models"),
        new("embedding", "models", "EmbeddingSectionAnchor", "ui.settings.section.embedding"),
        new("model_aliases", "presets", "ModelAliasesSectionAnchor", "ui.settings.section.model_aliases"),
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
        new("retrieval", "retrieval", "RetrievalSectionAnchor", "ui.settings.section.retrieval"),
        new("app_runtime", "retrieval", "AppRuntimeSectionAnchor", "ui.settings.section.app_runtime"),
        new("git", "version_control", "GitSectionAnchor", "ui.settings.section.git"),
        new("diagnostics", "support", "DiagnosticsSectionAnchor", "ui.settings.section.diagnostics"),
        new("tutorial", "support", "TutorialSectionAnchor", "ui.settings.index.tutorial"),
    ];

}

public sealed record SettingsTabDefinition(string Id, string DisplayNameKey);

public sealed record SettingsSectionDefinition(
    string Id,
    string TabId,
    string AnchorName,
    string DisplayNameKey);
