using System.Reflection;
using System.Xml.Linq;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U198-B 守卫：「失败原因 + 下一步做什么」这套基元的**覆盖面**。
///
/// <para>
/// 缺陷形态不是「没有基建」而是「基建只装了 1/6」：`RecoveryText` / `HasRecoveryText`
/// 原本是 `SettingsPageViewModel` 的私有属性，画布 / 作品 / Git / 运行记录 / 模板
/// 五页零消费。作者第一次点运行失败，界面上只有「已失败」两个字。
/// </para>
///
/// <para>
/// ⚠️ **判据必须逐一枚举那 6 个页面**。「某处有 RecoveryText」这种存在性断言
/// 在只装 1 处时照样绿 —— 那正是本编号被漏掉这么久的原因，
/// 也是仓库里「做一半的功能会掩盖没做的一半」那条教训要求的
/// 「基元落地后必须数覆盖面 N/M」。
/// </para>
/// </summary>
public sealed class RecoveryHintCoverageTests
{
    /// <summary>
    /// 6 个主功能页：ViewModel 类型 → 视图文件名。
    /// 这份清单就是 N/M 里的那个 M；新增主功能页时加一行，
    /// 于是「新页面忘了装补救行」会在这里当场变红而不是等作者遇到。
    /// </summary>
    private static readonly (Type ViewModel, string View)[] Pages =
    {
        (typeof(SettingsPageViewModel), "SettingsPageView.axaml"),
        (typeof(WorkspacePageViewModel), "WorkspacePageView.axaml"),
        (typeof(WorksPageViewModel), "WorksPageView.axaml"),
        (typeof(GitPageViewModel), "GitPageView.axaml"),
        (typeof(RunLogPageViewModel), "RunLogPageView.axaml"),
        (typeof(TemplateMarketPageViewModel), "TemplateMarketPageView.axaml"),
    };

    /// <summary>
    /// **覆盖面判据①（VM 侧）：6 个页面逐一都必须真的暴露 `RecoveryText` / `HasRecoveryText`。**
    ///
    /// 走反射按类型逐个查，而不是 grep 出现次数：出现次数会被注释、被测试文件里的
    /// 引用抬上去（本文件自己就贡献若干次），而「这个类型上有没有这个属性」是硬事实。
    /// </summary>
    [Fact]
    public void AllSixPages_ExposeRecoveryTextAndItsVisibilityFlag()
    {
        var missing = new List<string>();
        foreach (var (viewModel, _) in Pages)
        {
            var text = viewModel.GetProperty("RecoveryText", BindingFlags.Public | BindingFlags.Instance);
            var flag = viewModel.GetProperty("HasRecoveryText", BindingFlags.Public | BindingFlags.Instance);
            if (text is null || text.PropertyType != typeof(string))
            {
                missing.Add($"{viewModel.Name}.RecoveryText");
            }
            if (flag is null || flag.PropertyType != typeof(bool))
            {
                missing.Add($"{viewModel.Name}.HasRecoveryText");
            }
        }

        Assert.True(
            missing.Count == 0,
            $"这些页面拿不到补救建议（覆盖面缺口 {missing.Count} 项）：{string.Join(", ", missing)}。" +
            "失败时作者只会看到「已失败」而没有下一步。");
    }

    /// <summary>
    /// **覆盖面判据②（视图侧）：属性有了不等于界面上看得见。**
    ///
    /// 这一条是本仓「修复要沿链路走到用户可见处」的直接落实：`IsLoading` 曾经
    /// 有实现、有维护、界面零消费点，单元测试全绿（它确实被正确地设了又清）。
    /// 所以每个视图都要同时具备**两样**：绑到 `RecoveryText` 的显示位，
    /// 以及绑到 `HasRecoveryText` 的显隐 —— 只有前者会在无建议时留一条空行，
    /// 只有后者则是个不显示内容的开关。
    /// </summary>
    [Fact]
    public void AllSixViews_BindBothRecoveryTextAndItsVisibility()
    {
        var missing = new List<string>();
        foreach (var (_, view) in Pages)
        {
            var xaml = File.ReadAllText(ResolveDesktopFile("Views", view));
            if (!xaml.Contains("Text=\"{Binding RecoveryText}\"", StringComparison.Ordinal))
            {
                missing.Add($"{view}: 没有 Text=\"{{Binding RecoveryText}}\" 显示位");
            }
            if (!xaml.Contains("IsVisible=\"{Binding HasRecoveryText}\"", StringComparison.Ordinal))
            {
                missing.Add($"{view}: 没有 IsVisible=\"{{Binding HasRecoveryText}}\" 显隐");
            }
        }

        Assert.True(
            missing.Count == 0,
            $"这些视图没把补救建议画出来（{missing.Count} 项）：{string.Join("；", missing)}");
    }

    /// <summary>
    /// **页面载入状态**属性名。落在这张表里的 <c>IsVisible</c> 绑定被当成
    /// 「互斥的页面态闸门」，不在表里的（<c>HasStatusText</c> / <c>HasSummaryEvents</c>
    /// 这类）当成「有数据才显示」的数据闸门 —— 后者只要有话可说就是开的，
    /// 不影响可达性；前者一次只有一个为真，走错了那份建议就永远不参与渲染。
    ///
    /// 这张表是手写的，所以配了哨兵 <see cref="ErrorRegionGateNames_StillExistOnThePageViewModels"/>：
    /// 属性一旦改名，表会静默变空、判据随之全通过 ——「grep 会静默失效并像零命中」
    /// 那条教训的同型。
    /// </summary>
    private static readonly string[] PageStateGateNames =
    {
        "ShowContent", "ShowEmpty", "IsLoading", "IsError", "IsStandaloneError", "IsContentError",
    };

    /// <summary>
    /// **覆盖面判据③：属性绑上了、位置也在视图里了，还要问「那个失败态下它可见吗」。**
    ///
    /// <para>
    /// 这条是判据②抓不到的一整类缺陷，而且实测抓到了一个：运行记录页原先只有一份
    /// 补救行，挂在 <c>IsVisible="{Binding ShowContent}"</c> 里，而 <c>ShowContent</c>
    /// 要求 <c>HasLogs</c> ⇒ **一条日志都还没有时加载失败**走的是整页错误态，
    /// 那份建议根本不参与渲染。而「一条日志都还没有」正是作者第一次点运行失败时的状态,
    /// 也就是本编号要救的那个人。判据②当时是绿的：绑定确实存在。
    /// </para>
    ///
    /// <para>
    /// 判据形式：页面每一处**页级**失败文案（<c>StatusText</c> / <c>ErrorText</c>），
    /// 其祖先页面态闸门集合必须包含某一份补救行的闸门集合（子集关系）。
    /// 同一个 StackPanel 里并排 ⇒ 两边闸门相同 ⇒ 通过；
    /// 建议藏在 <c>ShowContent</c> 里而失败文案在 <c>IsStandaloneError</c> 里 ⇒ 不通过。
    /// <c>DataTemplate</c> 里的 <c>StatusText</c> 一律跳过：那绑的是列表项自己的
    /// 状态（节点/摘要行），不是页级失败。
    /// </para>
    /// </summary>
    [Fact]
    public void EveryPageLevelFailureText_HasAReachableRecoveryRowInThatSameState()
    {
        var unreachable = new List<string>();
        var checkedRegions = 0;
        foreach (var (_, view) in Pages)
        {
            var root = XDocument.Load(ResolveDesktopFile("Views", view)).Root!;
            var recoveryGates = root.Descendants()
                .Where(element => (string?)element.Attribute("IsVisible") == "{Binding HasRecoveryText}")
                .Select(StateGatesOf)
                .ToArray();
            Assert.NotEmpty(recoveryGates);

            foreach (var primary in root.Descendants())
            {
                var text = (string?)primary.Attribute("Text");
                if (text is not "{Binding StatusText}" and not "{Binding ErrorText}")
                {
                    continue;
                }
                // 列表项自己的状态不算页级失败：它有自己的呈现位，也不该抢页级建议行。
                if (primary.Ancestors().Any(a => a.Name.LocalName == "DataTemplate"))
                {
                    continue;
                }
                checkedRegions++;
                var gates = StateGatesOf(primary);
                if (!recoveryGates.Any(candidate => candidate.IsSubsetOf(gates)))
                {
                    unreachable.Add(
                        $"{view}: {text} 显示在 [{string.Join("+", gates.DefaultIfEmpty("无闸门"))}] 态下，" +
                        $"但补救行只挂在 [{string.Join(" / ", recoveryGates.Select(g => string.Join("+", g.DefaultIfEmpty("无闸门"))))}]");
                }
            }
        }

        // 自检下限：XAML 结构变了导致一处都没扫到时，空循环也会「通过」。
        // 6 个页面至少各有一处页级失败文案。
        Assert.True(checkedRegions >= 6, $"只扫到 {checkedRegions} 处页级失败文案，解析逻辑失配了");
        Assert.True(
            unreachable.Count == 0,
            $"这些失败态下作者看不到补救建议（{unreachable.Count} 处）：{string.Join("；", unreachable)}。" +
            "属性接好了不等于到得了眼前——渲染位所在容器在那个失败态下必须是可见的。");
    }

    /// <summary>
    /// <see cref="PageStateGateNames"/> 的前提哨兵：表里每个名字都得真是某个页面 VM 上的
    /// <c>bool</c> 属性。改名后表会静默失效 —— 那时判据③把页面态闸门全当数据闸门，
    /// 于是「建议藏在错误的态里」这类缺陷会重新变成绿的。
    /// </summary>
    [Fact]
    public void ErrorRegionGateNames_StillExistOnThePageViewModels()
    {
        foreach (var gate in PageStateGateNames)
        {
            var owners = Pages
                .Select(page => page.ViewModel.GetProperty(gate, BindingFlags.Public | BindingFlags.Instance))
                .Where(property => property is not null && property.PropertyType == typeof(bool))
                .ToArray();
            Assert.True(
                owners.Length > 0,
                $"页面态闸门 `{gate}` 在 6 个页面 VM 上都不存在（bool 属性）——" +
                "它被改名或删掉了，PageStateGateNames 必须同步改，否则判据③静默失效。");
        }
    }

    /// <summary>祖先链上属于「页面载入态」的 <c>IsVisible</c> 绑定集合。</summary>
    private static HashSet<string> StateGatesOf(XElement element)
    {
        var gates = new HashSet<string>(StringComparer.Ordinal);
        for (var current = element; current is not null; current = current.Parent)
        {
            var binding = (string?)current.Attribute("IsVisible");
            if (binding is null || !binding.StartsWith("{Binding ", StringComparison.Ordinal))
            {
                continue;
            }
            // `{Binding X, Converter=...}` 这类也取第一段属性名。
            var name = binding[9..].TrimEnd('}').Split(',')[0].Trim();
            if (PageStateGateNames.Contains(name, StringComparer.Ordinal))
            {
                gates.Add(name);
            }
        }
        return gates;
    }

    /// <summary>
    /// **主判据（真实链路）：画布页上一次运行跑失败之后，后端的补救建议要到得了作者眼前。**
    ///
    /// 判据刻意取「运行进入 failed 之后 `vm.RecoveryText` 等于后端那句话」，
    /// 而**不是**「能否构造出一个补救建议」—— 后者在发射点没接线时照样全绿
    /// （U117 的教训：判据要落在 `run_workflow_impl` 之后的真实状态上）。
    /// </summary>
    [Fact]
    public async Task CanvasRunFailure_SurfacesTheBackendRecoverySuggestion()
    {
        var backend = FailingRunBackend.Create();
        backend.RecoverySuggestion = "重试次数已耗尽；请检查网络、provider 状态或工具参数后手动恢复";
        var vm = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await vm.ReloadProjectDataAsync();

        await DriveRunToFailedAsync(vm, backend);

        Assert.Equal(backend.RecoverySuggestion, vm.RecoveryText);
        Assert.True(vm.HasRecoveryText);
        // 前置：没有这一条，用例在「页面根本没跑到 failed」时也可能因为
        // RecoveryText 恰好被别处写成同一句而假绿（二次变异那条教训）。
        Assert.Equal("failed", vm.CurrentRunStatus);
        // 建议只取一次：接在跃迁边沿而不是每轮轮询。
        Assert.Equal(1, backend.RunStateCalls);
        // 取证环境自检：替身漏实现哪个 IPC 都会走 catch → ReportFailure，
        // 把 RecoveryText 写成 ui.recovery.unknown。那种「来自别处的值」
        // 曾让本用例在变异态下几乎读到一个看似合理的结果。
        Assert.Empty(backend.UnsupportedCalls);
    }

    /// <summary>
    /// **`recovery_suggestion` 的两种形态都要吃（U196-E / U198-B 同一条）。**
    ///
    /// 后端在两个地方产出它：`workflow/runtime.rs` 给的是**成文中文**，
    /// 而 IPC 契约里也出现过**文案 key** 形态（见 `BackendModelSerializationTests`
    /// 里的 `error.workflow.worker_failed.recovery`）。只吃一种的话，
    /// 另一种要么整句丢失、要么把 `error.xxx.yyy` 这串标识符原样印给作者。
    /// </summary>
    [Fact]
    public void RecoverySuggestion_AcceptsBothLocalizationKeysAndPlainProse()
    {
        var names = DisplayNameService.LoadDefault();

        // 形态一：成文中文 —— 原样呈现。
        const string prose = "调整预算或审批后继续运行";
        Assert.Equal(prose, UserFacingError.RecoveryFromSuggestion(prose, names));

        // 形态二：语言包里有的 key —— 取译文，绝不把 key 本身印出去。
        // 用后端真实会发的那个 key（`error.workflow.worker_failed.recovery`，
        // 见 `BackendModelSerializationTests`）：它**在语言包里是有的**
        // （`display_name.json:980`），所以这条同时证明 key 形态不是空想。
        var resolved = UserFacingError.RecoveryFromSuggestion(
            "error.workflow.worker_failed.recovery", names);
        Assert.Equal(names.Text("error.workflow.worker_failed.recovery"), resolved);
        Assert.DoesNotContain("error.workflow", resolved, StringComparison.Ordinal);

        // 形态二的坏路：key 形态但语言包里没有 —— 必须落到空串。
        // `DisplayNameService` 缺键时返回 `[key]`，那不是文案，印出来是 bug 而不是提示。
        Assert.Equal(
            string.Empty,
            UserFacingError.RecoveryFromSuggestion("error.no_such.recovery_key", names));
    }

    /// <summary>
    /// **配置页也必须吃到「按失败码兜底」这一级**（回归守卫）。
    ///
    /// <para>
    /// 这条守的是一个实测存在过的回归：`HandleSettingsFailure` 在
    /// <c>ReportFailure</c> 之后手写了一句 <c>RecoveryText = string.Empty</c>，
    /// 再只按 <c>RecoveryAction</c> 重算一遍。而 <c>recovery_action</c> 的后端产出点
    /// **只有检索/Qdrant 配置分区一处**（`commands.rs:10091-10156`）⇒ 那句清空
    /// 等于把配置页锁在「1/6 时代」的行为：权限被拒、服务商没配好这些
    /// **首次使用最常见**的失败上，配置页照旧一个字都没有。
    /// </para>
    ///
    /// <para>
    /// 判据取「不带 recovery_action 的 `permission` 失败之后 RecoveryText 等于
    /// `ui.recovery.permission`」—— 也就是**第二级**的产物。刻意不用带
    /// recovery_action 的用例：那种输入下两种实现给出同样的结果，
    /// 于是回归回来了也照样绿（`StructuredBackendFailure_UsesSectionFieldAndLocalizedRecoveryAction`
    /// 正是那种输入，它拦不住这条）。
    /// </para>
    /// </summary>
    [Fact]
    public void SettingsPage_FallsBackToTheErrorCodeHint_WhenBackendGaveNoRecoveryAction()
    {
        var names = DisplayNameService.LoadDefault();
        var vm = new SettingsPageViewModel(names, NoopSettingsBackend.Create());

        vm.ReportBackendFailureForTests(
            BackendException.FromIpcPayload(
                "permission",
                "write outside sandbox",
                "ui.error.permission"),
            "general");

        Assert.Equal(names.Text("ui.recovery.permission"), vm.RecoveryText);
        Assert.True(vm.HasRecoveryText);
        // 主文案没被建议顶掉：两行各说一件事。
        Assert.Equal(names.Text("ui.error.permission"), vm.StatusText);
    }

    /// <summary>
    /// **U198-B「顺带」：运行到达终态时侧栏角标要重新问一次后端。**
    ///
    /// <para>
    /// 原先 <c>RefreshSidebarBadgesAsync</c> 的调用点全在进项目 / 离开项目上，
    /// **运行链路零调用**（与 U197-A 同根）⇒ 一次运行跑完新产生的待审确认项、
    /// 运行记录、诊断，角标一个都不涨，除非作者恰好切走再切回来。
    /// </para>
    ///
    /// <para>
    /// 判据取**后端被问的次数**而不是「角标数字变了没有」：假后端返回固定值，
    /// 断言数字变化会要求先造一个「会自己增长」的假后端，那测的就变成假后端本身了
    /// （与 `BudgetSeverityAndRefreshTests` 同一取舍）。
    /// 走显式接口调用生产入口，不为测试新开钩子 —— 钩子会绕过要守的那个入口。
    /// </para>
    /// </summary>
    [Fact]
    public async Task ReachingTerminalState_AlsoRefreshesTheSidebarBadges()
    {
        var backend = BadgeCountingBackend.Create();
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);
        // 终态刷新有 `if (!HasOpenProject) return;` 这道闸；不置位就测的是那道闸。
        window.MarkProjectOpenForTests();
        var baseline = backend.BadgeQueryCount;

        ((IRunTerminalStateObserver)window).OnRunReachedTerminalState("wf-1", "run-1", "failed");

        for (var i = 0; i < 300 && backend.BadgeQueryCount == baseline; i++)
        {
            await Task.Delay(10);
        }
        Assert.True(
            backend.BadgeQueryCount > baseline,
            $"运行到终态后必须重新问一次侧栏角标：基线 {baseline} 次，终态后仍是 {backend.BadgeQueryCount} 次。" +
            "不刷的后果是作者跑完一轮，待审队列涨了而侧栏上看不出来。");
    }

    /// <summary>
    /// **每个后端错误码要么有一句建议，要么有一条「为什么不需要」的理由 —— 二选一，不能都没有。**
    ///
    /// 判据与 `ErrorCodeCopyCoverageTests` 同构：拿后端 `CommandErrorCode` 全集
    /// 逐一对照，而不是断言「有若干条 ui.recovery.* 键」。存在性判据在只补一条
    /// 建议时也绿 —— 而这里真正要拦的是「后端加了第 N+1 个码，前端不知道」。
    ///
    /// 「有理由」= 落在 `UserFacingError` 那张 `CodesWithoutRecoveryHint` 里，
    /// 即作者自己中止的态，或主文案已经交代了动作的码。
    /// </summary>
    [Fact]
    public void EveryBackendErrorCode_EitherHasARecoveryHintOrIsExplicitlyExempt()
    {
        var names = DisplayNameService.LoadDefault();
        var source = File.ReadAllText(ResolveRepoFile(Path.Combine("core", "src", "command_error.rs")));
        var start = source.IndexOf("pub const fn as_str(self)", StringComparison.Ordinal);
        Assert.True(start > 0, "没找到 as_str 定义，command_error.rs 结构变了");
        var end = source.IndexOf("\n    }", start, StringComparison.Ordinal);
        var codes = System.Text.RegularExpressions.Regex
            .Matches(source[start..end], @"=>\s*""([a-z_]+)""")
            .Select(match => match.Groups[1].Value)
            .Distinct()
            .ToArray();
        // 自检下限：正则失配时空循环也会「通过」。
        Assert.True(codes.Length >= 18, $"只解析出 {codes.Length} 个码，正则或文件结构失配了");

        var unhandled = new List<string>();
        foreach (var code in codes)
        {
            var hint = UserFacingError.RecoveryForCode(code, names);
            if (!string.IsNullOrEmpty(hint))
            {
                // 有建议：不能是 `[key]` 占位符（Localized 已挡，这里是双保险），
                // 也不能与主文案一字不差（那等于把同一句印两遍）。
                Assert.False(hint.StartsWith('['), $"ui.recovery.{code} 是缺键占位符");
                Assert.NotEqual(names.Text($"ui.error.{code}"), hint);
                continue;
            }
            // 无建议：必须是刻意豁免的。反射读那张私有表，避免在测试里抄一份副本
            // ——抄的副本会和产品一起漂移，然后两边错得一样、照样全绿。
            if (!ExemptCodes().Contains(code))
            {
                unhandled.Add(code);
            }
        }

        Assert.True(
            unhandled.Count == 0,
            $"这些后端错误码既没有补救建议、也没登记豁免理由：{string.Join(", ", unhandled)}。" +
            "后端加 CommandErrorCode 时必须二选一：补一句 ui.recovery.{code}，" +
            "或把它列进 UserFacingError.CodesWithoutRecoveryHint 并确认主文案已交代动作。");
    }

    /// <summary>
    /// **一次失败之后的成功不能还挂着上一次的「下一步」。**
    ///
    /// 这条守的是陈旧建议：失败 → 建议出现 → 作者改好了 → 保存成功，
    /// 此时状态行写着「已保存」而补救行还写着「请检查网络」，
    /// 指向一个已经不存在的问题。清除时机在 `PageViewModelBase.StatusText` 的 setter 上
    /// （非失败路径赋值即清），刻意**不是**靠各页面在每条成功路径上手写一行清除
    /// ——那种做法必然漏，而且漏了没有任何征兆。
    /// </summary>
    [Fact]
    public void RecoveryHint_IsClearedWhenTheNextStatusIsNotAFailure()
    {
        var names = DisplayNameService.LoadDefault();
        var page = new StatusProbePage();

        page.Fail(BackendException.FromIpcPayload("permission", "denied"), names);
        Assert.True(page.HasRecoveryText);

        page.StatusText = "已保存";

        Assert.False(page.HasRecoveryText);
        Assert.Equal(string.Empty, page.RecoveryText);
        // 主文案本身不受影响：清的只是建议那一行。
        Assert.Equal("已保存", page.StatusText);
    }

    /// <summary>只暴露 protected 成员用于探测基类语义的最小页面。</summary>
    private sealed class StatusProbePage : PageViewModelBase
    {
        public void Fail(Exception ex, DisplayNameService names) => StatusText = ReportFailure(ex, names);
    }

    private static IReadOnlyCollection<string> ExemptCodes()
    {
        var field = typeof(UserFacingError).GetField(
            "CodesWithoutRecoveryHint",
            BindingFlags.Static | BindingFlags.NonPublic);
        Assert.NotNull(field);
        var value = field!.GetValue(null) as IEnumerable<string>;
        Assert.NotNull(value);
        return value!.ToArray();
    }

    private static string ResolveRepoFile(string relative)
    {
        var dir = AppContext.BaseDirectory;
        for (var i = 0; i < 8 && dir is not null; i++)
        {
            var candidate = Path.Combine(dir, relative);
            if (File.Exists(candidate))
            {
                return candidate;
            }
            dir = Path.GetDirectoryName(dir);
        }
        throw new FileNotFoundException($"从 {AppContext.BaseDirectory} 向上找不到 {relative}");
    }

    private static string ResolveDesktopFile(params string[] parts)
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null)
        {
            var candidate = Path.Combine(
                new[] { directory.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            directory = directory.Parent;
        }

        throw new FileNotFoundException(string.Join('/', parts));
    }

    /// <summary>
    /// 把 run 会话按到 failed 上，走 `Attach` 这条真实跃迁（不起轮询：
    /// 测试里不要后台请求）。`LoadRunFailureRecoveryAsync` 是 fire-and-forget，
    /// 所以要等它那次 `get_workflow_run_state` 真的回来。
    /// </summary>
    private static async Task DriveRunToFailedAsync(
        WorkspacePageViewModel vm,
        FailingRunBackend backend)
    {
        var session = typeof(WorkspacePageViewModel)
            .GetField("_runSession", BindingFlags.Instance | BindingFlags.NonPublic)!
            .GetValue(vm)!;
        var attach = session.GetType().GetMethod("Attach", BindingFlags.Instance | BindingFlags.Public)!;
        // 先 running 再 failed：跃迁边沿判据要求 previous != "failed"，
        // 直接 Attach 成 failed 也能触发（previous 是空串），但那不是产品里的形状。
        attach.Invoke(session, new object?[] { "default", "run-1", "running", false, false });
        attach.Invoke(session, new object?[] { "default", "run-1", "failed", false, false });

        // ⚠️ 等待条件刻意是「建议**等于**后端那句」而不是「有建议了」：
        // 页面上还有别的失败路径也会写 RecoveryText（替身漏实现某个 IPC 时
        // 就会写成 ui.recovery.unknown），等「有值」会在错误的值上提前退出，
        // 于是断言读到的是**来自别处**的数据。轮询而非固定 Delay：
        // 这台机器上固定延时不是慢就是不稳。
        for (var i = 0; i < 300; i++)
        {
            if (string.Equals(vm.RecoveryText, backend.RecoverySuggestion, StringComparison.Ordinal))
            {
                return;
            }
            await Task.Delay(10);
        }
    }

    /// <summary>
    /// 只数 <c>get_sidebar_badges</c> 被问了几次；其余 IPC 给可用的默认值
    /// （不抛：MainWindowViewModel 构造与终态处理会碰到别的调用，
    /// 抛出来会把这条判据变成「测替身」）。
    /// （DispatchProxy 的 TProxy 不能是 sealed。）
    /// </summary>
    private class BadgeCountingBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public int BadgeQueryCount { get; private set; }

        public static BadgeCountingBackend Create()
        {
            var client = Create<IAriadneBackendClient, BadgeCountingBackend>();
            var backend = (BadgeCountingBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }
            if (targetMethod.Name == "get_HasProjectRoot")
            {
                return true;
            }
            if (targetMethod.Name == nameof(IAriadneBackendClient.GetSidebarBadgesAsync))
            {
                BadgeQueryCount++;
                return Task.FromResult(new SidebarBadgeCounts(2, 3, 4));
            }
            if (targetMethod.Name == nameof(IAriadneBackendClient.GetBudgetStatusAsync))
            {
                return Task.FromResult(new BudgetStatus(100, 10, null, false));
            }
            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                // 引用类型给 null：这条判据只关心角标那次调用被发了没有。
                var value = resultType.IsValueType ? Activator.CreateInstance(resultType) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }
            return targetMethod.ReturnType.IsValueType
                ? Activator.CreateInstance(targetMethod.ReturnType)
                : null;
        }
    }

    /// <summary>
    /// 配置页那条判据只走 <c>ReportBackendFailureForTests</c>（纯内存），
    /// 不发任何出站请求 —— 所以这个替身刻意**一律抛**：真有人偷偷发请求，
    /// 用例会当场报出方法名，而不是悄悄拿到一个默认值继续跑。
    /// （DispatchProxy 的 TProxy 不能是 sealed。）
    /// </summary>
    private class NoopSettingsBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, NoopSettingsBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args) =>
            throw new NotSupportedException(targetMethod?.Name);
    }

    // DispatchProxy 的 TProxy 不能是 sealed（运行时要为它生成子类）。
    private class FailingRunBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public string RecoverySuggestion { get; set; } = string.Empty;
        public int RunStateCalls { get; private set; }
        public List<string> UnsupportedCalls { get; } = new();

        public static FailingRunBackend Create()
        {
            var client = DispatchProxy.Create<IAriadneBackendClient, FailingRunBackend>();
            var backend = (FailingRunBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }
            if (targetMethod.Name == "get_HasProjectRoot")
            {
                return true;
            }

            object? value = targetMethod.Name switch
            {
                nameof(IAriadneBackendClient.LoadProjectCanvasAsync) => EmptyCanvas(),
                nameof(IAriadneBackendClient.ListWorkflowGraphsAsync) => Array.Empty<WorkflowSummary>(),
                nameof(IAriadneBackendClient.ListConfirmationsAsync) => Array.Empty<ConfirmationLogEntry>(),
                nameof(IAriadneBackendClient.ListInDoubtOperationsAsync) => Array.Empty<WorkflowOperation>(),
                // 下面三个不是「顺手补全」：`ReloadProjectDataAsync` 会调它们，漏一个
                // 就走 catch → ReportFailure → RecoveryText 被写成 ui.recovery.unknown，
                // 主判据于是读到**这条失败路径**的产物而不是运行失败链路的。
                // 实测就是这三个漏着的（自检断言抓出来的），所以它们是必需项。
                nameof(IAriadneBackendClient.GetProviderConfigAsync) => ConfiguredProviders(),
                nameof(IAriadneBackendClient.GetAutomationSettingsAsync) => IdleAutomation(),
                nameof(IAriadneBackendClient.GetWorksTreeAsync) => EmptyWorksTree(),
                nameof(IAriadneBackendClient.GetWorkflowRunStateAsync) => FailedRunState(),
                _ => Unsupported(targetMethod),
            };

            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }
            return value;
        }

        private static WorkflowGraphData EmptyCanvas() => new(
            "default",
            "Project Canvas",
            Array.Empty<CanvasNode>(),
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            ContentRevision: "canvas-revision");

        /// <summary>服务商已配好：不要让「没配服务商」这件事自己成为一条失败路径。</summary>
        private static ProviderConfigStatus ConfiguredProviders() => new(
            HasOpenAiKey: true,
            HasAnthropicKey: false,
            HasGeminiKey: false,
            DefaultLlmProviderId: "openai",
            DefaultEmbeddingProviderId: null,
            DefaultRerankerProviderId: null,
            DefaultSearchProviderId: null,
            Providers: Array.Empty<ProviderKeyStatus>());

        /// <summary>预算不设限（U112：`budget_usd = 0` 即不限制），确认策略为空。</summary>
        private static AutomationSettings IdleAutomation() => new(
            new BudgetStatus(0, 0, PreauthorizedUsd: null, AutoModeEnabled: false),
            Array.Empty<ConfirmationPolicySetting>());

        private static WorksTreeNode EmptyWorksTree() => new(
            "root",
            "project",
            "Project",
            "/tmp/ariadne-recovery-probe",
            Array.Empty<WorksTreeNode>());

        private WorkflowRunState FailedRunState()
        {
            RunStateCalls++;
            return new WorkflowRunState(
                "default",
                "run-1",
                "failed",
                PauseReason: null,
                StopReason: null,
                Failure: new WorkflowRunFailure(
                    "external",
                    "node_execute",
                    "provider returned 401",
                    RecoverySuggestion),
                Events: Array.Empty<string>());
        }

        /// <summary>
        /// 替身没实现的 IPC：**先登记再抛**。
        ///
        /// ⚠️ 登记这一步不是日志癖好，它是 <c>Assert.Empty(UnsupportedCalls)</c> 这条
        /// 取证环境自检**唯一**的数据来源 —— 原来这个方法是 <c>static</c> 的，
        /// 拿不到实例，那条断言因此恒真：一个什么都没断言的断言，
        /// 偏偏立在「本用例是否读到了来自别处的值」这个最要紧的判据上。
        /// 它要拦的是：替身漏实现某个 IPC → 页面走 catch → <c>ReportFailure</c> →
        /// <c>RecoveryText</c> 被写成 <c>ui.recovery.unknown</c>，
        /// 于是主判据读到的是**这条失败路径**的产物，而不是运行失败那条链路的。
        /// </summary>
        private object? Unsupported(MethodInfo method)
        {
            UnsupportedCalls.Add(method.Name);
            if (method.ReturnType == typeof(Task) || method.ReturnType.IsGenericType)
            {
                throw new NotSupportedException(method.Name);
            }
            return method.ReturnType.IsValueType ? Activator.CreateInstance(method.ReturnType) : null;
        }
    }
}
