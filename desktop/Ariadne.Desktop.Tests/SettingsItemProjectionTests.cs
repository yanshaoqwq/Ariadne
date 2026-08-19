using System.Reflection;
using System.Text.RegularExpressions;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U178-B 后半（设置页）：per-item 模板不得再绑 <c>$parent[UserControl]</c>。
///
/// 每处祖先绑定在每个 item 上建一个 <c>ControlTracker</c>、订阅 attach/detach 两个事件、
/// 并跑 10 层 LINQ 祖先遍历；订阅的正是 attach/detach ⇒ 成本落在「切回本页」的
/// 重挂载路径上（U159 认定的同一条路径）。设置页原有 40 处，全部在 per-item 模板内。
///
/// ⚠️ **结构判据单独不够**：若投影属性从不被写，结构用例照样全绿而所有 per-item
/// 文案变成空白。所以本文件必须配两条行为判据（投影真的发生 + 切语言仍生效）。
/// 画布页那轮变异实测确认过这一点：摘掉广播后结构用例仍绿，只有行为用例转红。
/// </summary>
public sealed class SettingsItemProjectionTests
{
    /// <summary>
    /// 结构判据：所有 per-item <c>DataTemplate</c> 区间内祖先绑定命中数为 0。
    ///
    /// ⚠️ **先剥注释再匹配**：本仓库已踩过——注释里的示例文本会让守卫假绿
    /// （反过来也成立：注释里留一句「原为 $parent[...]」会让守卫假红）。
    /// ⚠️ **带自检阈值**：正则一失效就会解析出 0 个模板区间、于是「0 处违规」全绿。
    /// 阈值取 6 是因为本页至少有 6 个 per-item 模板（模型行、LLM 目标、确认策略、
    /// 权限档、安全工具、节点预设），实际更多（chip、下拉项模板等）。
    /// </summary>
    [Fact]
    public void PerItemTemplates_HaveNoAncestorBindings()
    {
        var view = StripXmlComments(File.ReadAllText(ResolveDesktopView("SettingsPageView.axaml")));
        var templates = ExtractTopLevelDataTemplates(view);

        Assert.True(
            templates.Count >= 6,
            $"只解析出 {templates.Count} 个 DataTemplate 区间——本页至少有 6 个 per-item 模板。"
            + "解析失效时本用例会以「0 处违规」假绿，故此处先失败。");

        var offenders = templates
            .SelectMany(template => Regex
                .Matches(template, @"\$parent\[UserControl\]\.DataContext\.(\w+)")
                .Select(match => match.Groups[1].Value))
            .Distinct(StringComparer.Ordinal)
            .OrderBy(name => name, StringComparer.Ordinal)
            .ToArray();

        Assert.True(
            offenders.Length == 0,
            "设置页 per-item 模板里又出现了祖先绑定：" + string.Join("、", offenders)
            + "。每处都会在每个 item 上建 ControlTracker + 订阅 attach/detach + 跑 10 层祖先遍历，"
            + "成本落在切回本页的重挂载路径上（U178-B/U159）。"
            + "⚠️ 本页**不能**用 {loc:Text}——设置页是语言切换的现场，"
            + "静态取值切语言后不刷新。请投影到 item VM 自身属性，"
            + "并在 SettingsPageViewModel.BroadcastSharedItemProjections 里下推"
            + "（含 RefreshLocalizedText 里那一次）。");
    }

    /// <summary>
    /// 行为判据 1：投影**真的发生**——item 上的文案属性非空，且等于
    /// <c>display_name.json</c> 里对应 key 的值。
    ///
    /// 这一条拦的是「模板改完了但没人下推」：那种状态下上一条结构用例全绿，
    /// 而用户看到的是一片空白标签。所以断言取的是**真实 display_name 值**
    /// 而不是「非空」——只断非空的话，下推一个占位串也能骗过去。
    /// </summary>
    [Fact]
    public async Task ItemProjections_CarryRealDisplayNameValues()
    {
        var displayNames = DisplayNameService.LoadDefault();
        var vm = await BuildLoadedSettingsAsync(displayNames);

        var preset = Assert.Single(vm.NodePresets);
        Assert.Equal(displayNames.Text("ui.settings.presets.node_model"), preset.PresetNodeModelLabel);
        Assert.Equal(displayNames.Text("ui.settings.presets.node_timeout_ms"), preset.PresetNodeTimeoutLabel);
        Assert.Equal(displayNames.Text("ui.settings.presets.node_budget_usd"), preset.PresetNodeBudgetLabel);
        Assert.Equal(displayNames.Text("ui.settings.presets.access_title"), preset.PresetAccessTitle);
        Assert.Equal(displayNames.Text("ui.settings.presets.tools_title"), preset.PresetToolsTitle);
        Assert.Equal(
            displayNames.Text("ui.settings.presets.inherit_node_permissions"),
            preset.Permissions.InheritNodePermissionsText);
        Assert.Equal(
            displayNames.Text("ui.settings.permissions.allow_network"),
            preset.Permissions.AllowNetworkText);

        var profile = Assert.Single(vm.ScopedPermissionProfiles, item => item.Scope == "workflow_nodes");
        Assert.Equal(displayNames.Text("ui.settings.permissions.inherit_global"), profile.InheritGlobalText);
        Assert.Equal(displayNames.Text("ui.settings.permissions.read_roots"), profile.ReadableRootsLabel);
        Assert.Equal(displayNames.Text("ui.settings.permissions.write_roots"), profile.WritableRootsLabel);

        // 共享选项列表：必须是**同一个实例**，不是逐 item 的拷贝。
        // 拷贝会白吃内存，且 RebuildAvailableLlmModelOptions 原地重建后各行选项漂移。
        Assert.Same(vm.AvailableLlmModelOptions, preset.AvailableLlmModelOptions);

        // 用户点「新增模型行」这条路径（不经后端投影）也必须继承——
        // 继承挂在 CollectionChanged 上，正是为了覆盖所有增项路径。
        vm.AddProviderModelCommand.Execute(null);
        var row = Assert.Single(vm.ProviderModels);
        Assert.Equal(displayNames.Text("ui.settings.models.column.id"), row.ModelIdColumnLabel);
        Assert.Equal(displayNames.Text("ui.settings.models.remove_model"), row.RemoveModelText);
        Assert.Same(vm.FetchedModelIdCandidates, row.FetchedModelIdCandidates);
        Assert.Same(vm.ProviderCapabilityOptions, row.ProviderCapabilityOptions);
    }

    /// <summary>
    /// 行为判据 2（**本任务的正确性底线**）：切一次语言后，投影下来的文案必须**真的变了**。
    ///
    /// 这一条存在的理由是本页**不能**用画布页那轮的 <c>{loc:Text}</c>：
    /// <c>TextExtension</c> 只在「页面不订阅 LanguageChanged、没有 RefreshLocalizedText」
    /// 时安全，而 <c>SettingsPageViewModel</c> 恰恰有 <c>RefreshLocalizedText</c>，
    /// 设置页就是语言切换的现场。若投影只在初次下推、不在 RefreshLocalizedText 里重推，
    /// 缺陷只是换了个形式：「切成英文后这些 per-item 文案还是中文」。
    /// </summary>
    [Fact]
    public async Task LanguageSwitch_RepushesItemProjections()
    {
        var displayNames = DisplayNameService.LoadDefault();
        var vm = await BuildLoadedSettingsAsync(displayNames);
        vm.SelectedLanguage = "zh";

        var preset = Assert.Single(vm.NodePresets);
        var profile = Assert.Single(vm.ScopedPermissionProfiles, item => item.Scope == "workflow_nodes");
        var policy = vm.ConfirmationPolicies.First();
        var group = vm.ToolControlGroups.FirstOrDefault();
        var beforeAccess = preset.PresetAccessTitle;
        var beforeNetwork = profile.AllowNetworkText;
        var beforeNormalMode = policy.NormalModeLabel;

        vm.SelectedLanguage = "en";

        Assert.Equal("en", displayNames.CurrentLanguage);
        Assert.Equal(displayNames.Text("ui.settings.presets.access_title"), preset.PresetAccessTitle);
        Assert.NotEqual(beforeAccess, preset.PresetAccessTitle);
        Assert.Equal(displayNames.Text("ui.settings.permissions.allow_network"), profile.AllowNetworkText);
        Assert.NotEqual(beforeNetwork, profile.AllowNetworkText);
        Assert.Equal(
            displayNames.Text("ui.settings.automation.confirmation.normal_mode"),
            policy.NormalModeLabel);
        Assert.NotEqual(beforeNormalMode, policy.NormalModeLabel);
        if (group is not null)
        {
            Assert.Equal(displayNames.Text("ui.settings.permissions.safe_tools.title"), group.SafeToolsTitle);
        }
    }

    /// <summary>
    /// 剥掉 XML 注释。注释里的示例绑定既能让守卫假绿也能让它假红，两种都是坏的。
    /// </summary>
    private static string StripXmlComments(string xaml) =>
        Regex.Replace(xaml, "<!--.*?-->", string.Empty, RegexOptions.Singleline);

    /// <summary>
    /// 抽出所有**最外层** <c>DataTemplate</c> 区间（嵌套模板包含在外层区间内，
    /// 因此不会漏掉 ComboBox.ItemTemplate 这类内层模板里的祖先绑定）。
    ///
    /// 用深度计数而非正则配对：<c>&lt;DataTemplate&gt;</c> 在本页有多层嵌套，
    /// 贪婪/懒惰正则都会配错边界。
    /// </summary>
    private static List<string> ExtractTopLevelDataTemplates(string xaml)
    {
        var regions = new List<string>();
        var depth = 0;
        var start = -1;
        // ⚠️ 必须 Cast<Match>()：MatchCollection 的 foreach 走非泛型 IEnumerator，
        // 元素静态类型是 object，直接取 .Value/.Index 编译不过。
        foreach (var token in Regex
                     .Matches(xaml, @"<DataTemplate\b|</DataTemplate>")
                     .Cast<Match>())
        {
            var text = token.Value;
            if (text.StartsWith("</", StringComparison.Ordinal))
            {
                depth--;
                if (depth == 0 && start >= 0)
                {
                    regions.Add(xaml[start..(token.Index + token.Length)]);
                    start = -1;
                }
                continue;
            }

            if (depth == 0)
            {
                start = token.Index;
            }
            depth++;
        }

        return regions;
    }

    private static string ResolveDesktopView(string fileName)
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null)
        {
            var candidate = Path.Combine(dir.FullName, "desktop", "Ariadne.Desktop", "Views", fileName);
            if (File.Exists(candidate))
            {
                return candidate;
            }
            dir = dir.Parent;
        }
        throw new FileNotFoundException(
            $"从 {AppContext.BaseDirectory} 向上找不到 desktop/Ariadne.Desktop/Views/{fileName}");
    }

    /// <summary>
    /// 构造一个**装入了真实投影数据**的设置页 VM：节点预设、作用域权限档、
    /// 工具组、确认策略都得有至少一项，否则行为判据会在空集合上空转、假绿。
    /// </summary>
    private static async Task<SettingsPageViewModel> BuildLoadedSettingsAsync(DisplayNameService displayNames)
    {
        var backend = ProjectionBackend.Create(out var proxy);
        proxy.Enqueue(Presets(), Permissions());
        var vm = new SettingsPageViewModel(displayNames, backend);
        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());

        // 确认策略只由整页加载（automation section）填充，本文件不接那条后端；
        // 而生产路径 ApplyConfirmationPolicies 也正是对同一个集合 Clear + Add，
        // 所以直接 Add 一条走的是**同一个** CollectionChanged 继承路径。
        vm.ConfirmationPolicies.Add(new ConfirmationPolicyViewModel(
            "writer_write",
            "writer",
            "manual_review",
            "allow_by_default",
            string.Empty,
            () => { }));

        Assert.NotEmpty(vm.NodePresets);
        Assert.NotEmpty(vm.ScopedPermissionProfiles);
        Assert.NotEmpty(vm.ConfirmationPolicies);
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

    private static PermissionsSettings Permissions() => new(
        Policy(),
        new Dictionary<string, PermissionPolicy?>
        {
            ["workflow_nodes"] = Policy(),
            ["project_ai"] = null,
        },
        new Dictionary<string, IReadOnlyDictionary<string, bool?>>
        {
            ["global"] = new Dictionary<string, bool?> { ["find"] = true, ["write"] = true },
        });

    private static PermissionPolicy Policy() => new(
        true,
        true,
        true,
        true,
        false,
        new[] { "/root" },
        new[] { "/root" });

    /// <summary>
    /// ⚠️ <c>DispatchProxy</c> 运行时派生宿主类型，所以**不能 sealed**
    /// （否则 <c>ArgumentException: The base type cannot be sealed</c>）。
    /// 未列出的调用抛异常：本文件只关心投影，页面进错误分支不影响判据，
    /// 反而能避免在没接后端的调用上无限等待。
    /// </summary>
    internal class ProjectionBackend : DispatchProxy
    {
        private readonly Queue<Task<NodePresetSettings>> _presets = new();
        private readonly Queue<Task<PermissionsSettings>> _permissions = new();

        public static IAriadneBackendClient Create(out ProjectionBackend proxy)
        {
            var client = Create<IAriadneBackendClient, ProjectionBackend>();
            proxy = (ProjectionBackend)(object)client;
            return client;
        }

        public void Enqueue(NodePresetSettings presets, PermissionsSettings permissions)
        {
            _presets.Enqueue(Task.FromResult(presets));
            _permissions.Enqueue(Task.FromResult(permissions));
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args) => targetMethod?.Name switch
        {
            nameof(IAriadneBackendClient.GetNodePresetSettingsAsync) => _presets.Dequeue(),
            nameof(IAriadneBackendClient.GetPermissionsSettingsAsync) => _permissions.Dequeue(),
            $"get_{nameof(IAriadneBackendClient.HasProjectRoot)}" => true,
            _ => throw new NotSupportedException(targetMethod?.Name),
        };
    }
}
