using System.Reflection;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U207-B：配置页在**别的** tab 加载失败时，不许对当前这一页说「已禁止保存」。
///
/// # 缺陷形态（实机取证）
///
/// 打开配置页停在「通用」，右上角横幅写着「此配置分区加载失败，已禁止保存。」
/// 而「通用」分区其实加载完好，失败的是「模型」——另一个 tab。
/// 在项目名里敲三个字符，横幅立刻变成「有未保存的更改：通用」、FAB 出现、
/// 恢复按钮点亮 ⇒ **保存从未被禁**。功能是通的，文案在劝退。
///
/// 根因是页面级状态行把 14 个分区的成败**按位或**成一个 bool
/// （`LoadAsync` 里的 `failed |= !await apply()`），聚合掉了分区粒度；
/// 而保存门禁本身是按分区判的（`CanSave` → `_draftState.IsLoaded(section)`）。
///
/// # 判据为什么必须落在「当前 tab 完好 + 别的 tab 失败」这个组合态
///
/// 断言 `failed` 的计算方式、或断言「有失败时状态行非空」，**缺陷版本一样满足**。
/// 唯一能区分的判据是在组合态下同时钉住两件事：
/// 状态行**不含**阻断措辞，且保存路径（FAB 的 `SaveCurrentTabCommand`）**真的可点**。
///
/// 另外「点名是哪个分区失败」这个能力必须单独钉住：不钉的话它会静默退化成
/// 一句泛泛的「部分配置未能加载」，作者不知道该去哪一页重试——
/// 那等于把劝退缺陷换成了信息丢失缺陷。
/// </summary>
public sealed class SettingsPartialSectionLoadStatusTests
{
    /// <summary>
    /// 组合态主判据：「模型」分区读失败，当前 tab 停在完好的「通用」。
    ///
    /// 三条断言各自不可省：
    /// - 状态行**不含**阻断措辞 ⇒ 拦住「在完好页面上劝退」；
    /// - 状态行**点名**「模型」 ⇒ 拦住「改成只看当前 tab」这种把劝退换成信息丢失的修法；
    /// - 改一个字后 `HasUnsavedChanges`（FAB 的可见性绑定）与
    ///   `SaveCurrentTabCommand.CanExecute`（FAB 的命令）双双为真 ⇒
    ///   钉住「保存其实一直可用」这个实测事实，也就是那句阻断措辞为什么是假的。
    /// </summary>
    [Fact]
    public async Task OtherTabFailedToLoad_CurrentTabKeepsSavingAndStatusDropsTheBlockingWording()
    {
        var names = DisplayNameService.LoadDefault();
        var client = WholePageBackend.Create(out var backend);
        backend.FailingMethods.Add(nameof(IAriadneBackendClient.GetProviderConfigAsync));
        var vm = new SettingsPageViewModel(names, client);

        await vm.ReloadProjectDataAsync();

        // 前提核实：失败的确实只有「模型」，当前 tab 是完好的「通用」。
        Assert.Equal("general", vm.SelectedTab.Id);
        Assert.True(vm.IsGeneralEditable);
        Assert.False(vm.IsModelsEditable);
        var failure = Assert.Single(vm.SectionLoadFailures);
        Assert.Equal("models", failure.Section);

        // 判据 1：不许出现阻断措辞。
        var blocking = names.Text("ui.settings.status.section_load_failed");
        Assert.DoesNotContain(blocking, vm.StatusText, StringComparison.Ordinal);

        // 判据 2：必须点名失败分区（「告知」不许退化成泛泛一句）。
        Assert.Contains(vm.ModelsTitle, vm.StatusText, StringComparison.Ordinal);

        // 判据 3：保存路径真的通——FAB 的两个绑定都要为真。
        vm.ProjectName = "改了三个字";
        Assert.True(vm.HasUnsavedChanges);
        Assert.True(vm.SaveCurrentTabCommand.CanExecute(null));
        Assert.True(vm.RestoreCurrentTabCommand.CanExecute(null));
    }

    /// <summary>
    /// 对照组：全部分区加载成功时，状态行必须回到「已保存」，且**不点名任何分区**。
    ///
    /// 少了这一条，「无条件显示失败通知」也能让上一条用例全绿。
    /// </summary>
    [Fact]
    public async Task CleanLoad_ReportsSavedAndNamesNoSection()
    {
        var names = DisplayNameService.LoadDefault();
        var client = WholePageBackend.Create(out _);
        var vm = new SettingsPageViewModel(names, client);

        await vm.ReloadProjectDataAsync();

        Assert.Empty(vm.SectionLoadFailures);
        Assert.Equal(names.Text("ui.common.configured"), vm.StatusText);
        Assert.DoesNotContain(vm.ModelsTitle, vm.StatusText, StringComparison.Ordinal);
    }

    /// <summary>
    /// 点名带来的义务：重试成功后状态行必须**停止**点名那个分区。
    ///
    /// 越具体的文案，过期时越像系统在胡说。此前 <c>RetryFailedSectionAsync</c> 走
    /// `UpdateDirtyState(updateStatus: false)`，状态行不刷新 —— 泛泛措辞时看不出来，
    /// 一旦点名就变成可见的错误信息。
    /// </summary>
    [Fact]
    public async Task SuccessfulRetry_StopsNamingTheRecoveredSection()
    {
        var names = DisplayNameService.LoadDefault();
        var client = WholePageBackend.Create(out var backend);
        backend.FailingMethods.Add(nameof(IAriadneBackendClient.GetProviderConfigAsync));
        backend.HealAfterFirstFailure.Add(nameof(IAriadneBackendClient.GetProviderConfigAsync));
        var vm = new SettingsPageViewModel(names, client);

        await vm.ReloadProjectDataAsync();
        Assert.Contains(vm.ModelsTitle, vm.StatusText, StringComparison.Ordinal);

        var failure = Assert.Single(vm.SectionLoadFailures);
        failure.RetryCommand.Execute(null);
        for (var attempt = 0; attempt < 200 && vm.HasSectionLoadFailures; attempt++)
        {
            await Task.Delay(1);
        }

        // 重试必须**重新发一次后端读取**。
        // 这条断言不是锦上添花：`BeginLoadSection` 的 `Deferred` 把「已在飞行中的 Task」
        // 捕获进 read 闭包，而失败分支登记的 retry 复用同一个 read ⇒
        // 重试只是把那个**已 faulted 的 Task** 再 await 一遍，永远不可能成功。
        // 只断言「失败项被清掉」看不出是哪一环坏的；断言调用次数能直接指认这一点。
        Assert.Equal(2, backend.CallCounts[nameof(IAriadneBackendClient.GetProviderConfigAsync)]);
        Assert.False(vm.HasSectionLoadFailures);
        Assert.True(vm.IsModelsEditable);
        Assert.DoesNotContain(vm.ModelsTitle, vm.StatusText, StringComparison.Ordinal);
        Assert.Equal(names.Text("ui.common.configured"), vm.StatusText);
    }

    /// <summary>
    /// 阻断措辞不是删掉，是**搬到真正被阻断的那一刻**。
    ///
    /// `RunSectionSaveAsync` 里那句同名文案由 `_draftState.IsLoaded(section)` 判定，
    /// 粒度本来就是对的：用户真去保存一个没加载上的分区，就该被明确告知禁止。
    /// 这条守卫防的是下一个人「顺手清理不再使用的 key」把它一起删掉——
    /// 那会让唯一诚实的那处阻断提示消失。
    /// </summary>
    [Fact]
    public void BlockingWording_StaysAtTheBlockedSaveAttempt()
    {
        var source = File.ReadAllText(ResolveDesktopSource("ViewModels", "SettingsPageViewModel.cs"));
        var occurrences = source.Split("ui.settings.status.section_load_failed").Length - 1;
        Assert.Equal(1, occurrences);

        var index = source.IndexOf("ui.settings.status.section_load_failed", StringComparison.Ordinal);
        var enclosing = source.LastIndexOf("private async Task<bool> RunSectionSaveAsync", index, StringComparison.Ordinal);
        Assert.True(enclosing >= 0, "阻断措辞必须留在 RunSectionSaveAsync（保存被真正拒绝的那一刻）");
        // 判据取「它与 IsLoaded(section) 同处一句」——那才是按分区判定的证据，
        // 而不是「它出现在文件里某处」。
        Assert.Contains(
            "_draftState.IsLoaded(section)",
            source[enclosing..(index + 64)],
            StringComparison.Ordinal);
    }

    /// <summary>
    /// 新增可见文案必须三语同批建键。
    ///
    /// `DisplayNameService` 缺键时**静默回落中文**，界面不报错也不显示 `[key]`
    /// （回落发生在 overlay 未命中时），所以「跑起来文案正常」证明不了 en/ja 建了键——
    /// 唯一的发现途径就是这条键集合守卫。占位符一并断言：
    /// 少了 `{sections}` 的译文会把「点名」这个能力在那门语言里静默作废。
    /// </summary>
    [Fact]
    public void PartialFailureCopy_ExistsInEveryLanguagePackWithItsPlaceholder()
    {
        const string key = "ui.settings.status.sections_load_failed_partial";
        var resourceDir = ResolveResourceDirectory();
        foreach (var file in new[] { "display_name.json", "display_name.en.json", "display_name.ja.json" })
        {
            var path = Path.Combine(resourceDir, file);
            var pack = JsonSerializer.Deserialize<Dictionary<string, string>>(File.ReadAllText(path))!;
            Assert.True(pack.ContainsKey(key), $"{file} 缺 {key}");
            Assert.Contains("{sections}", pack[key], StringComparison.Ordinal);
            // 阻断措辞的原键必须仍在（前一条用例守着它的生产用法）。
            Assert.True(pack.ContainsKey("ui.settings.status.section_load_failed"), file);
        }
    }

    private static string ResolveResourceDirectory()
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null)
        {
            var candidate = Path.Combine(directory.FullName, "core", "resources");
            if (File.Exists(Path.Combine(candidate, "display_name.json")))
            {
                return candidate;
            }
            directory = directory.Parent;
        }
        throw new DirectoryNotFoundException("core/resources not found from " + AppContext.BaseDirectory);
    }

    private static string ResolveDesktopSource(params string[] segments)
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null)
        {
            var candidate = Path.Combine(
                new[] { directory.FullName, "desktop", "Ariadne.Desktop" }.Concat(segments).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            directory = directory.Parent;
        }
        throw new FileNotFoundException("desktop source not found: " + string.Join('/', segments));
    }

    /// <summary>
    /// 整页加载用的后端桩：**除**指定分区外全部成功。
    ///
    /// 刻意不是「只桩两三个方法、其余抛」：整页加载会打 14 个分区的读取，
    /// 其它分区若也失败，`SectionLoadFailures` 里就不止一项，
    /// 「状态行只点名真正失败的那一个」这条判据随之失去分辨力。
    ///
    /// `DispatchProxy` 宿主不能 sealed（运行时要派生它）。
    /// </summary>
    private class WholePageBackend : DispatchProxy
    {
        /// <summary>要让哪些后端读取抛出（按方法名）。</summary>
        public HashSet<string> FailingMethods { get; } = new(StringComparer.Ordinal);

        /// <summary>已抛过一次的方法名。重试时放行，用来验证「重试成功后状态行不再点名它」。</summary>
        public HashSet<string> HealAfterFirstFailure { get; } = new(StringComparer.Ordinal);

        /// <summary>每个后端方法被调用的次数。用来区分「重试真的重发了请求」与「只是复用旧 Task」。</summary>
        public Dictionary<string, int> CallCounts { get; } = new(StringComparer.Ordinal);

        public static IAriadneBackendClient Create(out WholePageBackend backend)
        {
            var client = Create<IAriadneBackendClient, WholePageBackend>()!;
            backend = (WholePageBackend)(object)client;
            return client;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            var name = targetMethod?.Name ?? string.Empty;
            CallCounts[name] = CallCounts.TryGetValue(name, out var seen) ? seen + 1 : 1;
            if (FailingMethods.Contains(name))
            {
                if (HealAfterFirstFailure.Contains(name))
                {
                    // 只失败第一次：第二次（重试）放行，模拟「后端恢复了」。
                    FailingMethods.Remove(name);
                }
                return Failure(targetMethod!, new InvalidOperationException(name + " unavailable"));
            }

            return name switch
            {
                "get_HasProjectRoot" => true,
                nameof(IAriadneBackendClient.GetCurrentProjectAsync) =>
                    Task.FromResult<CurrentProjectStatus?>(new CurrentProjectStatus("/tmp/u207b", "Ariadne")),
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
                nameof(IAriadneBackendClient.SaveGeneralSectionSettingsAsync) =>
                    Task.FromResult((GeneralSectionSettings)args![0]!),
                _ => Failure(targetMethod!, new NotSupportedException(name)),
            };
        }

        /// <summary>按返回类型把异常包成对应的 Task，保持「后端读取失败」的真实形状。</summary>
        private static object? Failure(MethodInfo method, Exception exception)
        {
            var returnType = method.ReturnType;
            if (returnType == typeof(void))
            {
                throw exception;
            }
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

    // ── 固定桩数据 ──────────────────────────────────────────────
    // 值本身无关判据，只需让每个分区的 apply 能跑通并落 baseline。

    private static AppSettings AppSettingsFixture() => new(new AppConfig(
        1, "U207B 项目", "zh", "documents", "workflows", "skills", "exports"));

    private static ProviderConfigStatus ProviderFixture() => new(
        false, false, false, null, null, null, null, Array.Empty<ProviderKeyStatus>());

    private static PermissionPolicy PolicyFixture() => new(
        false, false, false, false, false, new[] { "/tmp/u207b" }, new[] { "/tmp/u207b" });

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
