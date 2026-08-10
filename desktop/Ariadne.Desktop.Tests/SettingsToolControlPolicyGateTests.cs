using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U110：工具开关的有效状态 = tool_controls &amp;&amp; PermissionPolicy。
/// 出厂状态下 tool_controls 把 web-search 设为 true，而 PermissionPolicy 默认
/// allow_network / allow_web_search 全为 false，两层自相矛盾；UI 若只渲染
/// tool_controls 原始值，用户会看到「开关是开的」但功能不可用。
/// 权威判定公式见后端 <c>workflow_web_search_tool_enabled</c>（core/src/commands.rs）。
/// </summary>
public sealed class SettingsToolControlPolicyGateTests
{
    [Fact]
    public async Task WebSearchToolSwitch_IsMarkedBlocked_WhenPolicyDeniesNetwork()
    {
        var vm = await LoadAsync(allowNetwork: false, allowWebSearch: false);

        var globalWebSearch = FindTool(vm, "global", "web-search");
        Assert.True(globalWebSearch.IsEnabled);
        Assert.True(
            globalWebSearch.IsBlockedByPolicy,
            "出厂 tool_controls 为 true 但硬权限全拒，必须标记为被策略否决");
        Assert.False(string.IsNullOrWhiteSpace(globalWebSearch.PolicyBlockedHint));
    }

    [Fact]
    public async Task WebSearchToolSwitch_ClearsBlockedState_WhenBothPolicyBitsAreEnabled()
    {
        var vm = await LoadAsync(allowNetwork: false, allowWebSearch: false);
        var globalWebSearch = FindTool(vm, "global", "web-search");
        Assert.True(globalWebSearch.IsBlockedByPolicy);

        // 用户在权限页打开两个硬权限位后，开关必须立刻不再显示为被否决。
        vm.AllowNetwork = true;
        vm.AllowWebSearch = true;

        Assert.False(globalWebSearch.IsBlockedByPolicy);
        Assert.Null(globalWebSearch.PolicyBlockedHint);
    }

    [Fact]
    public async Task WebSearchToolSwitch_ReturnsToBlocked_WhenNetworkIsTurnedOffAgain()
    {
        var vm = await LoadAsync(allowNetwork: true, allowWebSearch: true);
        var globalWebSearch = FindTool(vm, "global", "web-search");
        Assert.False(globalWebSearch.IsBlockedByPolicy);

        // allow_network 是子权限的总闸：关掉它，web search 立即失效。
        vm.AllowNetwork = false;

        Assert.True(globalWebSearch.IsBlockedByPolicy);
    }

    [Fact]
    public async Task NonWebSearchTools_AreNeverMarkedBlockedByPolicy()
    {
        // find / search / register / write 在 policy 层没有并联的布尔门
        // （FileRead/FileWrite 需要具体路径），不得叠加第二状态源。
        var vm = await LoadAsync(allowNetwork: false, allowWebSearch: false);

        foreach (var group in vm.ToolControlGroups)
        {
            foreach (var control in group.Controls)
            {
                if (IsWebSearchTool(control.ToolId))
                {
                    continue;
                }
                Assert.False(
                    control.IsBlockedByPolicy,
                    $"{group.Scope}/{control.ToolId} 不应被 policy 门标记");
            }
        }
    }

    [Fact]
    public async Task NodeScopedWebSearchTool_FollowsWorkflowNodesScopedPolicy()
    {
        // 节点类作用域（writer 等）的生效 policy 是 scoped_policies["workflow_nodes"]，
        // 与后端 permission_policy_for_node 的解析顺序一致。
        var vm = await LoadAsync(
            allowNetwork: true,
            allowWebSearch: true,
            workflowNodesPolicy: Policy(allowNetwork: false, allowWebSearch: false));

        var writerWebSearch = FindTool(vm, "writer", "writer-web-search");
        Assert.True(
            writerWebSearch.IsBlockedByPolicy,
            "全局放行但 workflow_nodes 作用域否决时，节点工具必须显示为被否决");

        var globalWebSearch = FindTool(vm, "global", "web-search");
        Assert.False(globalWebSearch.IsBlockedByPolicy);
    }

    [Fact]
    public async Task PolicyGateDoesNotMakeThePageDirty()
    {
        // 有效状态是派生显示，不是用户编辑，不能让「打开设置页」就变成未保存。
        var vm = await LoadAsync(allowNetwork: false, allowWebSearch: false);

        Assert.False(vm.HasUnsavedChanges);
    }

    private static bool IsWebSearchTool(string toolId) =>
        string.Equals(toolId, "web-search", StringComparison.Ordinal)
        || toolId.EndsWith("-web-search", StringComparison.Ordinal);

    private static ToolControlItemViewModel FindTool(
        SettingsPageViewModel vm,
        string scope,
        string toolId)
    {
        var group = Assert.Single(vm.ToolControlGroups, item => item.Scope == scope);
        return Assert.Single(group.Controls, item => item.ToolId == toolId);
    }

    private static async Task<SettingsPageViewModel> LoadAsync(
        bool allowNetwork,
        bool allowWebSearch,
        PermissionPolicy? workflowNodesPolicy = null)
    {
        var backend = SettingsPermissionPresetCompositionTests.PermissionPresetBackend.Create(
            out var proxy);
        proxy.Enqueue(
            Task.FromResult(Presets()),
            Task.FromResult(Permissions(allowNetwork, allowWebSearch, workflowNodesPolicy)));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);
        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        return vm;
    }

    private static NodePresetSettings Presets() => new(
        new[]
        {
            new NodeTypePreset(
                "llm",
                "node.type.llm",
                "model-a",
                30_000,
                1,
                null,
                new Dictionary<string, bool?>()),
        },
        "model-a",
        30_000,
        1);

    /// <summary>复刻后端 <c>default_permission_tool_controls</c> 中与本用例相关的出厂值。</summary>
    private static PermissionsSettings Permissions(
        bool allowNetwork,
        bool allowWebSearch,
        PermissionPolicy? workflowNodesPolicy) => new(
        Policy(allowNetwork, allowWebSearch),
        new Dictionary<string, PermissionPolicy?>
        {
            ["workflow_nodes"] = workflowNodesPolicy,
            ["project_ai"] = null,
        },
        new Dictionary<string, IReadOnlyDictionary<string, bool?>>
        {
            ["global"] = new Dictionary<string, bool?>
            {
                ["find"] = true,
                ["search"] = true,
                ["web-search"] = true,
                ["register"] = false,
                ["write"] = false,
            },
            ["writer"] = new Dictionary<string, bool?>
            {
                ["writer-find"] = null,
                ["writer-search"] = null,
                ["writer-web-search"] = null,
            },
        });

    private static PermissionPolicy Policy(bool allowNetwork, bool allowWebSearch) => new(
        allowNetwork,
        allowWebSearch,
        false,
        false,
        false,
        Array.Empty<string>(),
        Array.Empty<string>());
}
