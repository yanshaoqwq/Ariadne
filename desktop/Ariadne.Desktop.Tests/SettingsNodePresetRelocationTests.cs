using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

// 复用既有的权限/预设双读取假后端：它已经实现了这两条读取的排队与保存记录，
// 再写一份只会漂移。
using PermissionPresetBackend =
    Ariadne.Desktop.Tests.SettingsPermissionPresetCompositionTests.PermissionPresetBackend;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U164-A：「节点类型预设」章节从预设页搬到权限控制页。
///
/// ## 这批用例真正要守的东西
///
/// 搬迁本身（改 catalog 一行 + 挪一段 XAML）是低风险的，页内索引会自动跟随。
/// **危险的是它不会自动跟随的那部分**：页签 → 脏页 section 的映射是
/// `CanRestoreSelectedTab` 与 `SaveSelectedTabAsync` 两处**手写的 switch**，
/// 与 <see cref="SettingsNavigationCatalog"/> 毫无关联。
///
/// 只改 catalog 的失效形态是最坏的一种：用户在权限页改了某节点的联网权限 →
/// 保存钮不亮（脏判定不含 presets）→ 切页后改动被丢弃，**全程无报错**。
/// 所以判据不取「UI 上能看到那个控件」（那在只改 catalog 时也照样绿），
/// 而取「改 → 存 → 重新加载 → 值还在」这条端到端链路。
/// </summary>
public sealed class SettingsNodePresetRelocationTests
{
    [Fact]
    public void NodePresetsSection_BelongsToPermissionsTabBetweenToolControlsAndPaths()
    {
        var sections = SettingsNavigationCatalog.Sections;
        var nodePresets = Assert.Single(sections, item => item.Id == "node_presets");

        Assert.Equal("permissions", nodePresets.TabId);

        // 顺序也是判据的一部分：全局工具 → 按节点类型的工具覆盖 → 路径。
        var permissionSections = sections
            .Where(item => item.TabId == "permissions")
            .Select(item => item.Id)
            .ToArray();
        Assert.Equal(
            new[] { "capabilities", "tool_controls", "node_presets", "paths" },
            permissionSections);

        // 预设页搬走后剩下的三节仍成立（不必合并页签）。
        Assert.Equal(
            new[] { "model_aliases", "defaults", "templates" },
            sections.Where(item => item.TabId == "presets").Select(item => item.Id).ToArray());
    }

    /// <summary>
    /// **本批最重要的一条**：在权限控制页改节点级权限 → 点保存 → 值真的落到后端。
    ///
    /// 判据刻意取「后端收到的 payload」而不是「保存钮亮不亮」：
    /// 后者在 `SaveSelectedTabAsync` 漏改时仍会亮（脏判定已修），
    /// 但 `SavePresetsAsync` 没被调用，改动照样丢——两处 switch 要分别被钉住。
    /// </summary>
    [Fact]
    public async Task PermissionsTab_SavesNodeLevelPermissionEditsToBackend()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        proxy.Enqueue(
            Task.FromResult(Presets(permissionPolicy: null)),
            Task.FromResult(Permissions(globalNetwork: false, workflowNetwork: false)));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);
        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());

        // 用户站在权限控制页（搬迁后节点预设就在这一页）。
        vm.SelectTabForTests("permissions");
        var preset = Assert.Single(vm.NodePresets);
        preset.Permissions.InheritGlobal = false;
        preset.Permissions.AllowNetwork = true;

        // 保存钮必须亮：脏判定要认得 presets section 属于这一页。
        Assert.True(vm.SaveCurrentTabCommand.CanExecute(null));

        Assert.True(vm.SaveCurrentTabCommand.TryExecute());
        await DrainAsync(() => proxy.SavePresetsCalls > 0);

        // 真实出站产物：后端确实收到了节点级权限覆盖。
        Assert.Equal(1, proxy.SavePresetsCalls);
        Assert.NotNull(proxy.SavedPresets);
        var savedPolicy = Assert.Single(proxy.SavedPresets!.Presets).PermissionPolicy;
        Assert.NotNull(savedPolicy);
        Assert.True(savedPolicy!.AllowNetwork);
    }

    /// <summary>
    /// 保存后「有未保存改动」必须落回 false。
    ///
    /// 单独一条是因为它守的是另一个环节：`SaveSelectedTabAsync` 即使调了
    /// `SavePresetsAsync`，若脏判定那侧没修，`HasUnsavedChanges` 会一直挂着，
    /// 用户切页时被反复追问「要保存吗」。
    /// </summary>
    [Fact]
    public async Task PermissionsTab_ClearsUnsavedFlagAfterSavingNodePresets()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        proxy.Enqueue(
            Task.FromResult(Presets(permissionPolicy: null)),
            Task.FromResult(Permissions(globalNetwork: false, workflowNetwork: false)));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);
        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        vm.SelectTabForTests("permissions");

        Assert.Single(vm.NodePresets).TimeoutMs = "600";
        Assert.True(vm.HasUnsavedChanges);

        Assert.True(vm.SaveCurrentTabCommand.TryExecute());
        await DrainAsync(() => !vm.HasUnsavedChanges);

        Assert.False(vm.HasUnsavedChanges);
        Assert.Equal(1, proxy.SavePresetsCalls);
    }

    /// <summary>
    /// 镜像方向：预设页**仍然**要能保存 presets section。
    ///
    /// U164 文档建议「把 `presets` 那两处的 `PresetsSection` 去掉（只留模板仓库）」——
    /// **这条不成立，已推翻**。`PresetsSection` 这个数据 section 同时承载
    /// `ModelAliases` / `Default*`（留在预设页）与 `NodePresets`（搬到权限页）三组字段，
    /// 去掉后用户改模型别名会保存钮不亮，正是搬迁前那条缺陷换了个受害者。
    /// 一个 section 被两个页签共同承载是**允许的**（retrieval 页早有先例），
    /// 只是两页都要在 switch 里认领它。
    /// </summary>
    [Fact]
    public async Task PresetsTab_StillSavesModelAliasEditsAfterRelocation()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        proxy.Enqueue(
            Task.FromResult(Presets(permissionPolicy: null)),
            Task.FromResult(Permissions(globalNetwork: false, workflowNetwork: false)));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);
        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());

        vm.SelectTabForTests("presets");
        Assert.Single(vm.NodePresets).BudgetUsd = "3";

        Assert.True(vm.SaveCurrentTabCommand.CanExecute(null));
    }

    /// <summary>
    /// 索引项点击能跨页把用户带到权限页的节点预设锚点。
    ///
    /// 文档第 6 点担心「锚点查找限定在当前页签的可视树内会找不到」——
    /// 这里断言 ViewModel 侧确实切了页并发出了正确锚点请求。
    /// （真实滚动落点属视觉，由 SettingsViewLifecycleTests 那条 headless 用例覆盖机制。）
    /// </summary>
    [Fact]
    public async Task NodePresetsIndexEntry_NavigatesToPermissionsTabAnchor()
    {
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), NoopBackend.Create());
        SettingsSectionNavigationRequest? request = null;
        vm.ScrollToSectionRequested += (_, current) => request = current;

        await vm.SelectSectionForTestsAsync("node_presets");

        Assert.Equal("permissions", vm.SelectedTab.Id);
        Assert.Equal("node_presets", vm.SectionNavigationSelection.Id);
        Assert.NotNull(request);
        Assert.Equal("NodePresetsSectionAnchor", request!.AnchorName);
        // 章节在「高级权限控制」Expander 内，跨页跳转必须顺手展开，否则滚过去是一片空白。
        Assert.True(vm.AreAdvancedPermissionsExpanded);
    }

    /// <summary>
    /// 那段 XAML 真的在权限页的可见性分组里，而不只是 catalog 改了。
    ///
    /// 源码文本断言在这里是合适的：判据是「哪个 IsVisible 分组包住它」，
    /// 这是 XAML 结构事实，而 headless 下要证明同一件事得先把整页实体化。
    /// </summary>
    [Fact]
    public void NodePresetsMarkup_LivesUnderPermissionsVisibilityGroup()
    {
        var view = File.ReadAllText(ResolveDesktopSource("Views", "SettingsPageView.axaml"));
        var anchor = view.IndexOf("x:Name=\"NodePresetsSectionAnchor\"", StringComparison.Ordinal);
        var permissionsGroup = view.IndexOf("IsVisible=\"{Binding IsPermissionsSelected}\"", StringComparison.Ordinal);
        var personalizationGroup = view.IndexOf("IsVisible=\"{Binding IsPersonalizationSelected}\"", StringComparison.Ordinal);

        Assert.True(anchor > 0 && permissionsGroup > 0 && personalizationGroup > permissionsGroup);
        Assert.InRange(anchor, permissionsGroup, personalizationGroup);

        // 顺序：tool_controls 之后、paths 之前，与 catalog 一致。
        var toolControls = view.IndexOf("x:Name=\"ToolControlsSectionAnchor\"", StringComparison.Ordinal);
        var paths = view.IndexOf("x:Name=\"PathsSectionAnchor\"", StringComparison.Ordinal);
        Assert.InRange(anchor, toolControls, paths);

        // 该 Border 仍绑 IsPresetsEditable：页签归属是 UI 概念，脏页 section 是数据概念。
        var borderStart = view.LastIndexOf("<Border", anchor, StringComparison.Ordinal);
        Assert.Contains(
            "IsEnabled=\"{Binding IsPresetsEditable}\"",
            view[borderStart..anchor],
            StringComparison.Ordinal);
    }

    private static async Task DrainAsync(Func<bool> until)    {
        for (var attempt = 0; attempt < 200 && !until(); attempt++)
        {
            await Task.Delay(1);
        }
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        for (var depth = 0; directory is not null && depth < 12; depth++)
        {
            var candidate = Path.Combine(
                new[] { directory.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            directory = directory.Parent;
        }
        throw new FileNotFoundException($"找不到桌面源文件：{string.Join('/', parts)}");
    }

    private static NodePresetSettings Presets(PermissionPolicy? permissionPolicy) => new(
        new[]
        {
            new NodeTypePreset(
                "llm",
                "node.type.llm",
                "model-a",
                30_000,
                1,
                permissionPolicy,
                new Dictionary<string, bool?>()),
        },
        "model-a",
        30_000,
        1);

    private static PermissionsSettings Permissions(bool globalNetwork, bool workflowNetwork) => new(
        Policy(globalNetwork, "/global"),
        new Dictionary<string, PermissionPolicy?>
        {
            ["workflow_nodes"] = Policy(workflowNetwork, "/workflow"),
            ["project_ai"] = null,
        },
        new Dictionary<string, IReadOnlyDictionary<string, bool?>>());

    private static PermissionPolicy Policy(bool network, string root) => new(
        network,
        network,
        network,
        network,
        false,
        new[] { root },
        new[] { root });

    // DispatchProxy 的 TProxy 不能是 sealed（运行时要继承它生成代理类型）。
    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args) =>
            throw new NotSupportedException(targetMethod?.Name);
    }
}
