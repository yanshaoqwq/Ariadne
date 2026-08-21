using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U213-A0：**配置分区加载失败时，「为什么」在整个屏幕上无处可查。**
///
/// # 缺陷形态（用户亲报两轮）
///
/// 配置页报「六个模块加载失败」，横幅只说「“{分区名}”未能加载。」——
/// 只有分区名，**没有原因位**。用户第一反应是猜「主密码没设」，
/// 设完之后六个分区照样失败（他第二轮反馈明确说「和主密码没有关系，设置了」），
/// 到此他没有任何下一步可走。
///
/// 根因在 `LoadSectionAsync` 的失败处理是**裸 `catch`**（连 `ex` 都没接）：
/// 异常整个丢弃，`RegisterSectionLoadFailure` 手上从来就没有原因可写。
///
/// ⚠️ **同文件内的对照证明这是这条路径的特例，不是全局风格**：
/// `catch (Exception ex)` 有 16 处，裸 `catch` 9 处 ——
/// 全应用其余失败路径都走 `UserFacingError.Format(ex, ...)`，
/// **唯独最需要诊断的配置加载把异常扔了**。
///
/// 这不只是体验问题，是可维护性事故：报告作者逐层查完代码仍无法定夺真因，
/// 因为真因就在那个被丢弃的异常里。
///
/// # 判据为什么必须落在「异常内容出现在界面文案里」
///
/// ⚠️ **不能**断言「登记了一条失败横幅」——缺陷版本**正是这么做的**，那条会恒绿。
/// ⚠️ 也**不能**只断言「文案非空」——缺陷版本的文案也非空（它有分区名）。
///
/// 所以每条用例都往后端注入一个**内容可识别**的异常（带一段独特字符串），
/// 断言那段内容**真的出现在** VM 暴露给界面的文案里。这是唯一能区分
/// 「有原因位」与「没有原因位」的判据。
///
/// # 📌 一个必须记下的结构事实：横幅在整页加载中就被重建了一次
///
/// `ApplySavedLanguage`（`SettingsPageViewModel.cs:2441`）在 general 分区落地时
/// 就会调 `RefreshLocalizedText`，而它会把所有失败横幅按当前语言**重建一遍**。
/// ⇒ **主判据读到的那个横幅对象已经是重建产物，不是登记那一刻的原件。**
///
/// 实证：变异「重建时不从 `_failedSectionErrors` 取异常、直接传 null」时，
/// 主判据与切语言判据**一起变红**。我一开始以为是自己的用例写坏了（判据借道），
/// 改完判据重做变异，仍然一起红 ⇒ 这不是用例缺陷，**是产品的真实结构**。
///
/// 这件事对读代码的人有两个后果，都容易踩：
/// 1. 「异常单独存一份」（`_failedSectionErrors`）**不是**为切语言这个边角场景加的保险，
///    它在**最主流的那条路径上就是必需的** —— 去掉它，正常打开配置页就没有原因了。
/// 2. 因此这两条用例的覆盖面是**重叠**的，不要因为「有两条」就以为验了两件事。
///    真正只由切语言那条覆盖的是「主行跟着换语言」那半句。

/// </summary>
public sealed class SectionLoadFailureExplainsWhyTests
{
    /// <summary>
    /// 注入异常里的可识别标记。
    ///
    /// 刻意选一段**不可能**出现在任何文案 / 分区名 / 本地化键里的字符串：
    /// 若判据靠的是某个恰好也出现在别处的词，它就会在缺陷版本里假绿
    /// （本仓记过这一类「二次变异才能区分『断言无效』与『数据来自别处』」）。
    /// </summary>
    private const string Marker = "u213a0-tantivy-lockbusy-marker";

    /// <summary>
    /// 主判据：**后端拒绝的具体理由必须出现在横幅上**。
    ///
    /// 断言分两层，对应产品的两行文案，各自不可省：
    /// - `Diagnostic`（次行，脱敏后的后端原文）含 <see cref="Marker"/>
    ///   ⇒ 这一行才是「六个分区为什么一起倒」唯一能给出线索的地方。
    /// - `Message`（主行）与旧的无原因文案**不同**
    ///   ⇒ 拦住「只加了个 VM 属性、主行还是老样子」这种半修。
    /// </summary>
    [Fact]
    public async Task SectionFailedToLoad_TheBannerCarriesTheBackendReason()
    {
        var names = DisplayNameService.LoadDefault();
        var client = ExplainingBackend.Create(out var backend);
        backend.FailWith[nameof(IAriadneBackendClient.GetProviderConfigAsync)] =
            new InvalidOperationException(Marker);
        var vm = new SettingsPageViewModel(names, client);

        await vm.ReloadProjectDataAsync();

        var failure = Assert.Single(vm.SectionLoadFailures);
        Assert.Equal("models", failure.Section);

        // 判据 1：后端理由真的到了界面上。
        Assert.Contains(Marker, failure.Diagnostic, StringComparison.Ordinal);
        Assert.True(
            failure.HasDiagnostic,
            "拿到了诊断却把次行隐藏了 ⇒ 用户还是读不到原因");

        // 判据 2：主行不再是那句没有原因的旧文案。
        var wordless = names.Format("ui.settings.load_failure", new Dictionary<string, string>
        {
            ["section"] = vm.ModelsTitle,
        });
        Assert.NotEqual(wordless, failure.Message);
        // 主行仍要点名是哪个分区（原有能力不能因为加了原因位而丢）。
        Assert.Contains(vm.ModelsTitle, failure.Message, StringComparison.Ordinal);
    }

    /// <summary>
    /// 次行必须**真的有渲染位**，不能只是一个 VM 属性。
    ///
    /// 少了这条，「加个 `Diagnostic` 属性」就能让上面那条全绿，
    /// 而界面上一个字都不显示 —— 本仓反复出现的
    /// 「有实现、有维护、界面零消费」形态（U198-B 的 `RecoveryText` 原状就是）。
    ///
    /// ⚠️ 对源码原文做断言前**必须先剥 XAML 注释**：本轮修 U210 时踩过这个坑，
    /// 变异标记里复述了被断言的字符串，`Assert.Contains` 命中注释本身、变异全绿。
    /// </summary>
    [Fact]
    public void TheReasonLine_HasARealRenderingSlotInTheView()
    {
        var view = StripXamlComments(
            File.ReadAllText(ResolveDesktopSource("Views", "SettingsPageView.axaml")));

        Assert.Contains("{Binding Diagnostic}", view, StringComparison.Ordinal);
        // 空诊断时必须收起来，否则横幅多出一行空白。
        Assert.Contains("{Binding HasDiagnostic}", view, StringComparison.Ordinal);
    }

    /// <summary>
    /// **切一次界面语言，原因不许消失**，且主行要跟着换语言。
    ///
    /// 切语言会走 `RefreshLocalizedText` 把所有失败横幅按新语言重建一遍。
    /// 若重建时手上没有异常（只在登记那一刻用完就丢），横幅会**还在**、
    /// 而「原因」这一行静默清空 —— 退回缺陷形态，且比一开始更难发现：
    /// 用户只知道「我切了下语言，解释就没了」。
    ///
    /// ⚠️ **本条的判据经过一次修正，原因值得记下**：
    /// 首版只断言「重建后 `Diagnostic` 仍含标记」，看着合理，但**没有独立验证力** ——
    /// `ApplySavedLanguage`（`SettingsPageViewModel.cs:2441`）在**整页加载过程中**
    /// 就会调一次 `RefreshLocalizedText`，也就是说横幅登记后立刻被重建了一遍，
    /// 上面那条主判据读到的**本来就是重建产物**。
    /// 变异「重建时传 null」时两条用例一起红，正是这个借道关系的证据。
    ///
    /// ⇒ 判据改成**真正切一次语言**（`SelectedLanguage = "en"`）并同时钉住两件事：
    /// 诊断（语言无关的后端原文）**留住**，主行（本地化文案）**跟着变**。
    /// 后半句是关键：它证明重建**确实发生过**，否则「原因还在」也可能只是
    /// 因为压根没重建 —— 那种绿什么都没证明。
    /// </summary>
    [Fact]
    public async Task SwitchingUiLanguage_KeepsTheReasonAndRelocalizesTheHeadline()
    {
        var names = DisplayNameService.LoadDefault();
        var client = ExplainingBackend.Create(out var backend);
        backend.FailWith[nameof(IAriadneBackendClient.GetProviderConfigAsync)] =
            new InvalidOperationException(Marker);
        var vm = new SettingsPageViewModel(names, client);
        await vm.ReloadProjectDataAsync();

        // 前提自检：切之前原因确实在（否则这条用例什么都没验）。
        var before = Assert.Single(vm.SectionLoadFailures);
        Assert.Contains(Marker, before.Diagnostic, StringComparison.Ordinal);
        var headlineBefore = before.Message;

        try
        {
            vm.SelectedLanguage = "en";

            var after = Assert.Single(vm.SectionLoadFailures);
            // 语言无关的后端原文必须留住。
            Assert.Contains(
                Marker,
                after.Diagnostic,
                StringComparison.Ordinal);
            Assert.True(
                after.HasDiagnostic,
                "切一次语言就把原因这一行清空了 ⇒ 横幅还在、解释没了");
            // 而本地化主行必须**真的换了语言** —— 这半句证明重建发生过。
            Assert.NotEqual(headlineBefore, after.Message);
        }
        finally
        {
            // 语言是全局单例状态，切回去，免得污染同进程的其他用例
            // （本仓记过「测试基建缺陷伪装成产品缺陷」：单跑绿混跑红就先查共享状态）。
            vm.SelectedLanguage = "zh";
        }
    }

    /// <summary>
    /// 反向：**拿不到原因时不许印出以冒号结尾的残句**。
    ///
    /// 带 `{reason}` 的键在 `error` 为 null 时会拼出「“模型”未能加载：」——
    /// 比不说原因**更像系统坏了**。所以两个文案键都要留着，
    /// 这条钉住「落回旧键」这个分支真的存在。
    ///
    /// ⚠️ 这条同时是一个**前提哨兵**：它断言旧键 `ui.settings.load_failure`
    /// 仍然被产品消费。哪天有人以为「新键取代了旧键」把旧键删掉，
    /// 这里会红，而不是等到线上出现残句。
    /// </summary>
    [Fact]
    public void TheWordlessFallbackKey_IsStillReachable()
    {
        var names = DisplayNameService.LoadDefault();
        var source = File.ReadAllText(
            ResolveDesktopSource("ViewModels", "SettingsPageViewModel.cs"));

        // 两个键都必须在源码里被引用：新键负责有原因的路径，旧键负责兜底。
        Assert.Contains("\"ui.settings.load_failure\"", source, StringComparison.Ordinal);
        Assert.Contains("\"ui.settings.load_failure_with_reason\"", source, StringComparison.Ordinal);

        // 且两个键都真的有文案（缺键时 DisplayNameService 返回 "[key]"，
        // 那种"有值"骗得过存在性断言）。
        foreach (var key in new[] { "ui.settings.load_failure", "ui.settings.load_failure_with_reason" })
        {
            var text = names.Text(key);
            Assert.False(
                text.StartsWith('[') && text.EndsWith(']'),
                $"{key} 没有文案，DisplayNameService 回落成了 [key] 占位");
        }
    }

    /// <summary>剥掉 <c>&lt;!-- --&gt;</c> 注释，只留真实标记。</summary>
    private static string StripXamlComments(string markup)
        => System.Text.RegularExpressions.Regex.Replace(
            markup, "<!--.*?-->", string.Empty,
            System.Text.RegularExpressions.RegexOptions.Singleline);

    private static string ResolveDesktopSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var attempt = 0; attempt < 12 && walk is not null; attempt++)
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
    /// 能按方法名注入**指定异常**的后端替身。
    ///
    /// 与 `SettingsPartialSectionLoadStatusTests` 的 `WholePageBackend` 分工不同：
    /// 那个只需要「失败」，本文件需要「失败**并且**带上可识别内容」，
    /// 所以这里存的是 Exception 实例而不是方法名集合。
    ///
    /// `DispatchProxy` 的宿主类**不能 sealed**（运行时要派生它）。
    /// </summary>
    private class ExplainingBackend : DispatchProxy
    {
        /// <summary>方法名 → 要抛出的异常。</summary>
        public Dictionary<string, Exception> FailWith { get; } = new(StringComparer.Ordinal);

        public static IAriadneBackendClient Create(out ExplainingBackend backend)
        {
            var client = Create<IAriadneBackendClient, ExplainingBackend>()!;
            backend = (ExplainingBackend)(object)client;
            return client;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            var name = targetMethod?.Name ?? string.Empty;
            if (FailWith.TryGetValue(name, out var failure))
            {
                return Failure(targetMethod!, failure);
            }

            return name switch
            {
                "get_HasProjectRoot" => true,
                nameof(IAriadneBackendClient.GetCurrentProjectAsync) =>
                    Task.FromResult<CurrentProjectStatus?>(
                        new CurrentProjectStatus("/tmp/u213a0", "Ariadne")),
                nameof(IAriadneBackendClient.GetBackendDiagnosticsAsync) =>
                    Task.FromResult(new BackendDiagnosticsReport("healthy", Array.Empty<DiagnosticItem>())),
                nameof(IAriadneBackendClient.GetSecretProtectionAsync) =>
                    Task.FromResult(new SecretProtectionReport("encrypted", false)),
                nameof(IAriadneBackendClient.GetAppSettingsAsync) => Task.FromResult(AppSettingsFixture()),
                nameof(IAriadneBackendClient.ReadProjectMemoryAsync) => Task.FromResult("memory"),
                nameof(IAriadneBackendClient.GetProviderConfigAsync) => Task.FromResult(ProviderFixture()),
                nameof(IAriadneBackendClient.GetNodePresetSettingsAsync) => Task.FromResult(PresetFixture()),
                nameof(IAriadneBackendClient.GetPermissionsSettingsAsync) => Task.FromResult(PermissionsFixture()),
                nameof(IAriadneBackendClient.GetTemplateRepositorySettingsAsync) =>
                    Task.FromResult(new TemplateRepositorySettings("https://example.invalid/templates")),
                nameof(IAriadneBackendClient.GetAutomationSettingsAsync) => Task.FromResult(AutomationFixture()),
                nameof(IAriadneBackendClient.GetWorkflowSettingsAsync) => Task.FromResult(WorkflowFixture()),
                nameof(IAriadneBackendClient.GetUiPreferencesAsync) => Task.FromResult(PreferencesFixture()),
                nameof(IAriadneBackendClient.GetAppRuntimeSettingsAsync) =>
                    Task.FromResult(new AppRuntimeSettings("/opt/qdrant", 42_000)),
                nameof(IAriadneBackendClient.GetRagSettingsAsync) => Task.FromResult(RagFixture()),
                nameof(IAriadneBackendClient.GetGitSettingsAsync) => Task.FromResult(GitFixture()),
                _ => Failure(targetMethod!, new NotSupportedException(name)),
            };
        }

        /// <summary>按返回类型把异常包成对应的 Task，保持「后端读取失败」的真实形状。</summary>
        private static object? Failure(MethodInfo method, Exception exception)
        {
            var returnType = method.ReturnType;
            if (returnType == typeof(Task))
            {
                return Task.FromException(exception);
            }
            if (returnType.IsGenericType && returnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                return typeof(Task)
                    .GetMethod(nameof(Task.FromException), 1, new[] { typeof(Exception) })!
                    .MakeGenericMethod(returnType.GetGenericArguments()[0])
                    .Invoke(null, new object[] { exception })!;
            }
            throw exception;
        }
    }

    // ── 固定桩数据（值无关判据，只需让各分区的 apply 跑通并落 baseline）──

    private static AppSettings AppSettingsFixture() => new(new AppConfig(
        1, "U213A0 项目", "zh", "documents", "workflows", "skills", "exports"));

    private static ProviderConfigStatus ProviderFixture() => new(
        false, false, false, null, null, null, null, Array.Empty<ProviderKeyStatus>());

    private static PermissionPolicy PolicyFixture() => new(
        false, false, false, false, false, new[] { "/tmp/u213a0" }, new[] { "/tmp/u213a0" });

    private static PermissionsSettings PermissionsFixture() => new(
        PolicyFixture(),
        new Dictionary<string, PermissionPolicy?>(StringComparer.Ordinal),
        new Dictionary<string, IReadOnlyDictionary<string, bool?>>(StringComparer.Ordinal));

    private static NodePresetSettings PresetFixture() => new(
        Array.Empty<NodeTypePreset>(), "gpt-x", 60_000, 0.5);

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
