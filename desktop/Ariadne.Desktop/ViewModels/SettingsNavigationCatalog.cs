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
        new("defaults", "presets", "DefaultsSectionAnchor", "ui.settings.section.defaults"),
        new("templates", "presets", "TemplatesSectionAnchor", "ui.settings.section.templates"),
        new("budget", "automation", "BudgetSectionAnchor", "ui.settings.section.budget"),
        new("confirmations", "automation", "ConfirmationsSectionAnchor", "ui.settings.section.confirmations"),
        new("runtime", "automation", "RuntimeSectionAnchor", "ui.settings.section.runtime"),
        new("capabilities", "permissions", "CapabilitiesSectionAnchor", "ui.settings.section.capabilities"),
        new("tool_controls", "permissions", "ToolControlsSectionAnchor", "ui.settings.section.tool_controls"),
        // U164-A：node_presets 归到 permissions 页，不留在 presets 页。
        // 它的主体是「按节点类型的权限覆盖 + 工具覆盖」，与 capabilities /
        // tool_controls / paths 是同一组概念的不同作用域（按节点类型 vs 全局）；
        // 分在两个页签时，用户要弄清「这个节点能不能联网」得来回切页。
        // 后端本来就把 permissions 与 presets 作为一次读取产出
        // （SettingsPageViewModel 的 `PermissionsSection or PresetsSection` 合并分支），
        // 数据层从未分开过，只有 UI 拆开了。
        // 顺序放在 tool_controls 之后、paths 之前：全局工具 → 按节点类型的工具覆盖 → 路径。
        // ⚠️ 页签归属改了不等于脏页 section 改了：node_presets 的字段仍属 presets section
        // （见 SettingsPageViewModel.IsPresetsEditable），页签→section 的映射是手写的两处
        // switch（CanRestoreSelectedTab / SaveSelectedTabAsync），**不会跟随本表**。
        new("node_presets", "permissions", "NodePresetsSectionAnchor", "ui.settings.section.node_presets"),
        new("paths", "permissions", "PathsSectionAnchor", "ui.settings.section.paths"),
        // U176：凭据保护（设本地主密码 / 显式接受明文）落在权限页。
        // 它与 capabilities / tool_controls / paths 是同一组概念——「谁能读到什么」，
        // 而 API Key 摊不摊在磁盘上正是这组问题里最直接的一条。
        // 放在 paths 之后（本页最后一节）：它是**一次性处置**而非日常调节的配置，
        // 不该占据用户每次进权限页都先看到的位置。
        // ⚠️ 它**刻意不进** CanRestoreSelectedTab / SaveSelectedTabAsync 那两处
        // 手写 switch：两个动作是即时命令（点一下就调后端），没有草稿态，
        // 因此不存在「改了没存」。详见 SettingsPageViewModel 里
        // _secretMasterPassword 字段处的注释。
        new("secret_protection", "permissions", "SecretProtectionSectionAnchor", "ui.settings.section.secret_protection"),
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
