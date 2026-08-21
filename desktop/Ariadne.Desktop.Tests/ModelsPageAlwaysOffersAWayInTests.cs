using System.Reflection;
using System.Xml;
using System.Xml.Linq;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Threading;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U211-A：**「配置 → 模型」在读取失败时是一条死路** —— 新用户在产品第一步 100% 被阻断。
///
/// # 缺陷形态（Xvfb 真窗口 + xdotool 实机取证）
///
/// 全新项目打开「配置 → 模型」，屏上**一个能点的控件都没有**：供应商列表整片空白、
/// Base URL 输入框打不进字、「保存密钥 / 测试连接 / 刷新 / 移除」全灰、
/// 点「添加供应商」零反应（截图字节数与前一张完全相同）。
///
/// # 两条根因，各自都足以造成整页不可用
///
/// 1. **列表空**：`_providerConfig` 停在 null。models 分区走 `Deferred(...)`，
///    它的 apply 回调（`_providerConfig = value; RebuildProviderOptionsFromConfig(...)`）
///    **只在读取成功时执行**；失败时 `_providerConfig` 保持 null，而
///    `RebuildProviderOptionsFromConfig` 在 `ProviderOptions.Clear()` **之后**才撞上
///    `if (_providerConfig is null) return` ⇒ 清空了却什么也没填回去。
///    openai / anthropic / gemini 是**内置目录**（后端 `default_provider_status_configs`）
///    而不是用户数据，读不到项目配置并不构成隐藏它们的理由。
///
/// 2. **全部灰掉**：`SettingsPageView.axaml` 里「模型」页根 StackPanel 绑着
///    `IsEnabled="{Binding IsModelsEditable}"`，而 `IsModelsEditable = CanSave("models")`
///    的第一个条件就是 `_draftState.IsLoaded("models")` ⇒ 读取失败即整棵子树变灰。
///    这与权限页 U213-A 是**同一个形状**：把出路挡在故障后面。
///
/// # 判据为什么必须落在 `IsEffectivelyEnabled`
///
/// 缺陷版本里「添加供应商」按钮**自己的** `IsEnabled` 就是 `true`（它谁都没绑），
/// 断言 `IsEnabled` 会恒绿。真正决定「手指落下去点不点得动」的是继承后的
/// `IsEffectivelyEnabled`（U213-A 已在权限页付过一次学费）。
///
/// 本文件三层判据，各自不可省：
/// - **纯 VM 层**：内置目录可见 + 出路命令可执行（任务给定的两条硬判据）；
/// - **结构层**：解析 axaml，钉住出路的祖先链上没有禁用门，且门**仍然存在**于该受约束的小节；
/// - **运行层**：headless 实体化整页、走真实读取失败路径，断言 `IsEffectivelyEnabled`。
/// </summary>
[Collection("AvaloniaHeadless")]
public sealed class ModelsPageAlwaysOffersAWayInTests
{
    /// <summary>
    /// 主判据（任务给定的两条）：**模型分区读取失败之后**，
    /// 内置目录仍然可见，且「添加供应商」这条出路仍然可执行。
    ///
    /// 前提先核实：读取真的失败了、失败范围正是 models。少了这一步，
    /// 用一个健康后端跑同样两条断言会全绿——那就什么都没测。
    /// </summary>
    [Fact]
    public async Task ModelsSectionFailedToLoad_StillShowsBuiltInCatalogAndKeepsTheWayIn()
    {
        var names = DisplayNameService.LoadDefault();
        var client = ModelsLoadFailureBackend.Create(out var backend);
        var vm = new SettingsPageViewModel(names, client);

        await vm.ReloadProjectDataAsync();

        // 前提核实：失败的确实是 models，而且它真的没落基线。
        var failure = Assert.Single(vm.SectionLoadFailures);
        Assert.Equal("models", failure.Section);
        Assert.False(vm.IsModelsEditable);
        Assert.True(backend.CallCounts.ContainsKey(nameof(IAriadneBackendClient.GetProviderConfigAsync)));

        // 判据 1：内置目录必须可见。
        Assert.NotEmpty(vm.ProviderOptions);

        // 判据 2：出路必须存在（含加载失败后）。
        Assert.True(vm.AddProviderCommand.CanExecute(null));
    }

    /// <summary>
    /// 判据 2 单独一条，**刻意不与判据 1 同一个用例**：两条修复（内置目录 / 放开出路的门）
    /// 是相互独立的，摘掉任一条都必须有用例单独变红。写在一起的话摘掉内置目录会让
    /// `NotEmpty` 先炸、后面的断言根本跑不到，独立性就验不出来了。
    /// </summary>
    [Fact]
    public async Task ModelsSectionFailedToLoad_KeepsAddProviderExecutable()
    {
        var vm = new SettingsPageViewModel(
            DisplayNameService.LoadDefault(),
            ModelsLoadFailureBackend.Create(out _));

        await vm.ReloadProjectDataAsync();

        Assert.False(vm.IsModelsEditable);
        Assert.True(
            vm.AddProviderCommand.CanExecute(null),
            "「添加供应商」在模型分区读取失败时不可执行——它只打开一张空表单、"
            + "不读任何已加载数据，把它挡在读取成功之后等于让新用户在产品第一步无路可走"
            + "（U211-A；这是 U208-B「门禁粒度错了」的镜像方向）。");
    }

    /// <summary>
    /// 内置目录的**内容**判据：三项 id 必须与后端 `default_provider_status_configs` 对齐，
    /// 且全部标成草稿。
    ///
    /// 为什么不能只有上一条的 `NotEmpty`：往列表里塞一项「（无）」占位也能让它全绿，
    /// 而那对「我该添加什么」这个问题一个字都没回答。这条钉的是「目录里到底有什么」。
    ///
    /// 「全部草稿」这半句同样不可省：标成非草稿会让 `CanUsePersistedProvider` 放开
    /// 「保存密钥 / 移除 / 刷新」——拿一份没读到的后端状态去发写请求，
    /// 那是把空列表这个缺陷换成一个更糟的。
    /// </summary>
    [Fact]
    public async Task BuiltInCatalog_MatchesBackendDefaultsAndStaysDraftOnly()
    {
        var vm = new SettingsPageViewModel(
            DisplayNameService.LoadDefault(),
            ModelsLoadFailureBackend.Create(out _));

        await vm.ReloadProjectDataAsync();

        Assert.Equal(
            new[] { "openai", "anthropic", "gemini" },
            vm.ProviderOptions.Select(option => option.ProviderId).ToArray());
        Assert.All(vm.ProviderOptions, option => Assert.True(
            option.IsDraft,
            $"内置目录项 `{option.ProviderId}` 不是草稿。它没经过本项目后端确认，"
            + "标成已落库会放开「保存密钥 / 移除 / 刷新」三个写动作（U211-A）。"));

        // 选中项要真的落进编辑器，否则右侧表单是一片没有主人的空白。
        Assert.NotNull(vm.SelectedProviderOption);
        Assert.Equal("openai", vm.SelectedProviderOption!.ProviderId);
        // 类型不能是 `open_ai_compatible`：那会让作者以为 OpenAI 也得自己填 Base URL。
        Assert.Equal("open_ai", vm.ProviderType);
    }

    /// <summary>
    /// 「测试连接」与「添加模型」也属于放开的那一批（只读探测 / 纯新建），
    /// 而「保存 / 保存密钥 / 移除 / 刷新」必须仍然禁用。
    ///
    /// 这条是**反向**判据，不可省：只断言前两个能点的话，「把 models 页所有门删干净」
    /// 也能全绿——那会放开一片没有基线的写动作（`SaveModelCommand` 一保存就会把
    /// 项目里四条从未读到的默认路由清空）。5 处 `CanSave(ModelsSection)` 的
    /// 3 放开 / 2 保留这个划分，就是靠这一条钉住的。
    /// </summary>
    [Fact]
    public async Task LoadFailure_OpensOnlyTheNonDestructiveCommands()
    {
        var vm = new SettingsPageViewModel(
            DisplayNameService.LoadDefault(),
            ModelsLoadFailureBackend.Create(out _));

        await vm.ReloadProjectDataAsync();
        Assert.False(vm.IsModelsEditable);

        // 放开：新建类 + 只读探测。
        Assert.True(vm.AddProviderCommand.CanExecute(null));
        Assert.True(vm.AddProviderModelCommand.CanExecute(null));
        Assert.True(vm.TestProviderDraftCommand.CanExecute(null));

        // 保留：会写盘 / 依赖已落库状态。
        Assert.False(
            vm.SaveModelCommand.CanExecute(null),
            "「保存」在没有读取基线时被放开了：BuildProviderDefaultModelRoutes() 此刻"
            + "四条路由全是「无」占位，保存会把项目里从未读到的四条默认路由清空（U211-A）。");
        Assert.False(vm.SaveProviderKeyCommand.CanExecute(null));
        Assert.False(vm.RemoveProviderCommand.CanExecute(null));
        Assert.False(vm.RefreshModelsCommand.CanExecute(null));

        // 关键：**点一下列表里的内置目录项**之后保存门仍须关着。
        //
        // 这条不是重复上面那条。`SelectProviderForEditing` 会调 `SetSectionBaseline`，
        // 而 `SetBaseline` 原先无条件 `_loadedSections.Add(section)` ⇒ 选中一个供应商
        // 这个纯 UI 动作会把读取失败的分区**伪装成已加载** ⇒ `CanSave` 转真 ⇒
        // 保存按钮点亮 ⇒ 一保存就把项目里从未读到的四条默认模型路由清空。
        // 缺陷版本里这条路够不着（列表是空的、点不了任何东西），
        // U211-A 把列表填上内置目录之后它就成了一条**活的数据丢失路径**。
        await vm.SelectProviderOptionForTestsAsync("anthropic");
        Assert.Equal("anthropic", vm.SelectedProviderOption?.ProviderId);
        Assert.False(
            vm.IsModelsEditable,
            "选中一个内置目录项之后分区被标成「已加载」了——`SetBaseline` 不该凭一次"
            + "纯 UI 动作伪造读取事实（U211-A）。");
        Assert.False(
            vm.SaveModelCommand.CanExecute(null),
            "选中内置目录项之后保存门开了：这一保存会把项目里从未读到的四条默认模型路由"
            + "清空，是真的数据丢失（U211-A）。");
    }

    /// <summary>
    /// 结构判据：出路的**整条祖先链**上不许出现 `IsEnabled`。
    ///
    /// 为什么不是「祖先链上不许绑 IsModelsEditable」这种更窄的写法：换成别的任何
    /// `IsEnabled` 绑定都会重造同一个死路，缺陷的形状是「出路被挡在别人的前置条件
    /// 后面」，与挡它的是哪个属性无关。
    ///
    /// 同时反向钉住一条：**禁用门必须仍然存在**，只是不在这条链上。
    /// 少了这半句，「把 models 页所有 IsEnabled 删干净」也能让上半句全绿——
    /// 那会放开一片没有读取基线的表单，等于把死路换成脏写。
    /// </summary>
    [Fact]
    public void WayIn_HasNoDisablingGateAnywhereOnItsAncestorChain()
    {
        var path = ResolveDesktopSource("Views", "SettingsPageView.axaml");
        var document = XDocument.Load(path, LoadOptions.SetLineInfo);
        var xamlNamespace = XNamespace.Get("http://schemas.microsoft.com/winfx/2006/xaml");

        // 出路 = 「添加供应商」按钮 + 供应商列表 + 「测试连接」按钮。
        // 锚点先立：名字改了要当场红，而不是让下面的循环空转成绿。
        foreach (var name in new[] { "AddProviderButton", "ProviderCatalogList", "TestProviderDraftButton" })
        {
            var element = document
                .Descendants()
                .SingleOrDefault(candidate =>
                    (string?)candidate.Attribute(xamlNamespace + "Name") == name);

            Assert.True(
                element is not null,
                $"配置页里找不到 x:Name=\"{name}\"——「模型」页出路的守卫失去了着力点，"
                + "改名时请同步本用例而不是让它空转（U211-A）。");

            var gated = element!
                .Ancestors()
                .Where(ancestor => ancestor.Attribute("IsEnabled") is not null)
                .Select(ancestor =>
                    $"<{ancestor.Name.LocalName} IsEnabled=\"{ancestor.Attribute("IsEnabled")!.Value}\">"
                    + $"（第 {((IXmlLineInfo)ancestor).LineNumber} 行）")
                .ToList();

            Assert.True(
                gated.Count == 0,
                $"`{name}` 的祖先链上有禁用门：{string.Join('、', gated)}。"
                + "Avalonia 的 IsEnabled 沿视觉树继承，子级写 IsEnabled=\"True\" 压不回来，"
                + "⇒ 模型分区读取失败时用户唯一的出路会跟着故障一起灰掉（U211-A）。");
        }

        // 反向 1：models 页根 StackPanel 自己不许再绑 IsEnabled。
        var avaloniaNamespace = XNamespace.Get("https://github.com/avaloniaui");
        var modelsRoot = document
            .Descendants(avaloniaNamespace + "StackPanel")
            .Single(element => (string?)element.Attribute("IsVisible") == "{Binding IsModelsSelected}");
        Assert.Null(modelsRoot.Attribute("IsEnabled"));

        // 反向 2：「有读取基线才能编辑」的小节仍要各自带门。
        // 3 = 供应商编辑表单 / 可用模型 / 默认路由。
        // 数字写死是刻意的：加了小节忘了配门时这条会红，逼人当场决定
        // 「这一节该不该受读取基线约束」——那正是本缺陷缺失的那次决定。
        var gates = modelsRoot
            .Descendants()
            .Count(element =>
                (string?)element.Attribute("IsEnabled") == "{Binding IsProviderEditorEditable}");
        Assert.Equal(3, gates);
    }

    /// <summary>
    /// 运行判据：「模型」分区**真的读取失败**之后，出路与草稿表单的
    /// `IsEffectivelyEnabled` 必须仍为 `true`。
    ///
    /// 这一条钉的是「用户手指落下去到底点不点得动」，能拦住结构层看不见的失效方式
    /// （例如把门搬进 Style / 模板层）。
    /// ⚠️ 断言 `IsEnabled` 在缺陷版本里也为 `true`（这些控件谁都没绑），恒绿。
    /// ⚠️ `session.Dispatch` 用 `Func&lt;Task&lt;T&gt;&gt;` 重载——`Func&lt;Task&gt;` 那个重载
    /// 会静默不执行 body 并报绿。
    /// </summary>
    [Fact]
    public async Task ModelsSectionFailedToLoad_LeavesTheWayInInteractive()
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);

        await session.Dispatch(
            async () =>
            {
                var names = DisplayNameService.LoadDefault();
                var vm = new SettingsPageViewModel(names, ModelsLoadFailureBackend.Create(out _));
                await vm.ReloadProjectDataAsync();

                // 前提核实：读取真的失败了，门确实是关的。
                Assert.False(vm.IsModelsEditable);
                Assert.NotEmpty(vm.ProviderOptions);

                vm.SelectTabForTests("models");

                var view = new SettingsPageView { DataContext = vm };
                var window = new Window { Width = 1280, Height = 900, Content = view };
                window.Show();
                await DrainDispatcherAsync();

                // 主判据：出路 + 草稿表单必须真的可交互。
                foreach (var name in new[]
                         {
                             "AddProviderButton",
                             "ProviderCatalogList",
                             "TestProviderDraftButton",
                             "ProviderBaseUrlInput",
                         })
                {
                    var control = view.FindControl<Control>(name);
                    Assert.NotNull(control);
                    Assert.True(
                        control!.IsEffectivelyEnabled,
                        $"模型分区读取失败时 `{name}` 不可交互"
                        + $"（自身 IsEnabled={control.IsEnabled}，"
                        + $"实际可交互 IsEffectivelyEnabled={control.IsEffectivelyEnabled}）。"
                        + "这正是实机取证里「Base URL 打不进字、点添加供应商零反应」那一条，"
                        + "而新用户的第一步就在这一页（U211-A）。"
                        + "注意：断言 IsEnabled 在缺陷版本里也为 true，恒绿。");
                }

                // 对照：已落库 provider 的表单在没有基线时仍不可编辑 ⇒ 证明门没被删光。
                // 换成非草稿后 `IsProviderEditorEditable` 应立刻转假。
                vm.SelectedProviderOption!.IsDraft = false;
                vm.SelectProviderForTests(vm.SelectedProviderOption.ProviderId);
                await DrainDispatcherAsync();
                Assert.False(
                    vm.IsProviderEditorEditable,
                    "已落库 provider 在没有读取基线时被放开编辑了——那时表单里装的是默认值"
                    + "而不是它在后端的真实值，等于请作者在一份假数据上改（U211-A）。");

                window.Content = null;
                window.Close();
                await DrainDispatcherAsync();
                return true;
            },
            CancellationToken.None);
    }

    private static async Task DrainDispatcherAsync()
    {
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.SystemIdle);
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var index = 0; index < 12 && walk is not null; index++)
        {
            var candidate = Path.Combine(
                new[] { walk.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }

            walk = walk.Parent;
        }

        throw new FileNotFoundException(string.Join('/', parts));
    }

    /// <summary>
    /// U211-A 的最小后端桩：**只有 `GetProviderConfigAsync` 失败**，其余分区正常。
    ///
    /// 走的是整页真实读取路径（`ReloadProjectDataAsync`），而不是直接去戳 `_draftState`：
    /// 门是由「这一分区有没有落基线」判的，只有真实读取失败才会让
    /// `IsLoaded("models")` 保持 false、并让 `_providerConfig` 停在 null。
    ///
    /// 其余分区刻意成功：这样能顺带证明缺陷是**分区局部**的（别的页照常可编辑），
    /// 也避免把「整个后端都挂了」这种更极端的状态混进判据。
    ///
    /// `DispatchProxy` 宿主不能 `sealed`（运行时要派生它）。
    /// </summary>
    internal class ModelsLoadFailureBackend : DispatchProxy
    {
        public Dictionary<string, int> CallCounts { get; } = new(StringComparer.Ordinal);

        public static IAriadneBackendClient Create(out ModelsLoadFailureBackend backend)
        {
            var client = Create<IAriadneBackendClient, ModelsLoadFailureBackend>()!;
            backend = (ModelsLoadFailureBackend)(object)client;
            return client;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            var name = targetMethod?.Name ?? string.Empty;
            CallCounts[name] = CallCounts.TryGetValue(name, out var seen) ? seen + 1 : 1;
            if (string.Equals(name, nameof(IAriadneBackendClient.GetProviderConfigAsync), StringComparison.Ordinal))
            {
                return Task.FromException<ProviderConfigStatus>(
                    new InvalidOperationException("provider config unavailable"));
            }

            return name switch
            {
                "get_HasProjectRoot" => true,
                nameof(IAriadneBackendClient.GetCurrentProjectAsync) =>
                    Task.FromResult<CurrentProjectStatus?>(new CurrentProjectStatus("/tmp/u211a", "Ariadne")),
                nameof(IAriadneBackendClient.GetBackendDiagnosticsAsync) =>
                    Task.FromResult(new BackendDiagnosticsReport("healthy", Array.Empty<DiagnosticItem>())),
                nameof(IAriadneBackendClient.GetSecretProtectionAsync) =>
                    Task.FromResult(new SecretProtectionReport("encrypted", false)),
                nameof(IAriadneBackendClient.GetAppSettingsAsync) => Task.FromResult(AppSettingsFixture()),
                nameof(IAriadneBackendClient.ReadProjectMemoryAsync) => Task.FromResult("memory"),
                nameof(IAriadneBackendClient.GetNodePresetSettingsAsync) =>
                    Task.FromResult(new NodePresetSettings(
                        Array.Empty<NodeTypePreset>(), "gpt-x", 60_000, 0.5)),
                nameof(IAriadneBackendClient.GetPermissionsSettingsAsync) =>
                    Task.FromResult(PermissionsFixture()),
                nameof(IAriadneBackendClient.GetTemplateRepositorySettingsAsync) =>
                    Task.FromResult(new TemplateRepositorySettings("https://example.invalid/templates")),
                nameof(IAriadneBackendClient.GetAutomationSettingsAsync) =>
                    Task.FromResult(AutomationFixture()),
                nameof(IAriadneBackendClient.GetWorkflowSettingsAsync) => Task.FromResult(WorkflowFixture()),
                nameof(IAriadneBackendClient.GetUiPreferencesAsync) => Task.FromResult(PreferencesFixture()),
                nameof(IAriadneBackendClient.GetAppRuntimeSettingsAsync) =>
                    Task.FromResult(new AppRuntimeSettings("/opt/qdrant", 42_000)),
                nameof(IAriadneBackendClient.GetRagSettingsAsync) => Task.FromResult(RagFixture()),
                nameof(IAriadneBackendClient.GetGitSettingsAsync) => Task.FromResult(GitFixture()),
                // 其余方法本用例不该碰：返回 null 会让生产代码吃 NRE（mock 违约），
                // 所以直接炸掉，谁多调了一条立刻看得见。
                _ => throw new NotSupportedException(name),
            };
        }
    }

    // ── 固定桩数据 ──────────────────────────────────────────────
    // 值本身无关判据，只需让每个分区的 apply 能跑通并落 baseline。

    private static AppSettings AppSettingsFixture() => new(new AppConfig(
        1, "U211A 项目", "zh", "documents", "workflows", "skills", "exports"));

    private static PermissionPolicy PolicyFixture() => new(
        false, false, false, false, false, new[] { "/tmp/u211a" }, new[] { "/tmp/u211a" });

    private static PermissionsSettings PermissionsFixture() => new(
        PolicyFixture(),
        new Dictionary<string, PermissionPolicy?>(StringComparer.Ordinal),
        new Dictionary<string, IReadOnlyDictionary<string, bool?>>(StringComparer.Ordinal));

    private static AutomationSettings AutomationFixture() => new(
        new BudgetStatus(10, 1, null, false),
        Array.Empty<ConfirmationPolicySetting>());

    private static WorkflowSettings WorkflowFixture() => new(new WorkflowConfig(
        1, 60_000, 3, 4, true, 30));

    private static UiPreferences PreferencesFixture() => new(
        "system", "#112233", "#445566", true, null,
        new Dictionary<string, bool>(StringComparer.Ordinal), true);

    private static RagSettings RagFixture() => new(new RagConfig(
        1,
        new VectorStoreConfig(true, "qdrant", "ariadne", 1536,
            new SidecarConfig("127.0.0.1", 6333, "data", "/opt/qdrant", 42_000)),
        new FullTextStoreConfig("tantivy", "index"),
        true,
        800,
        80));

    private static GitSettings GitFixture() => new(new GitConfig(
        1, true, true, true, true, Array.Empty<string>()));
}
