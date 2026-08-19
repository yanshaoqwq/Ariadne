using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U187-A 回归：**后端修好的能力被前端一行 `.Where()` 重新埋掉**。
///
/// 缺陷链路：`commands.rs` 的 `list_confirmations` 从 `7378ddc` 起就合并了
/// 运行态 pending **与 `confirmation_logs` 里的已决议历史**（按 id 去重、运行态优先），
/// 但前端 `LoadConfirmationsAsync` 里写着
/// `entries.Where(IsPendingConfirmation)` —— 已决议项在消费端被整批丢弃。
/// 用户可见现象与修复**之前**完全一致：昨天审了 30 项，今天想查「我到底批准过什么」，
/// 一片空白。`ListConfirmationsAsync` 全前端只有那 1 处生产调用点，没有别的入口能看到历史
/// （运行记录页的 `confirmation` kind 筛选永久为空：3 个 `append_run_log` 写入点无一写它）。
///
/// ## 判据为什么必须落在两个集合的分工上
///
/// 单纯删掉过滤会造出一个**更严重**的缺陷：
/// `HasPendingConfirmations => Confirmations.Count > 0` 驱动
/// `ShowConfirmationFullPanel`，而审阅面板**替换整个画布**。
/// 历史项混进 `Confirmations` 后该条件恒真 ⇒ 作者每次打开项目都被一个盖住画布的面板拦住，
/// 再也回不到画布。所以这里的判据是「**分流**」而不是「不过滤」：
/// 待审进 `Confirmations`、已决议进 `ResolvedConfirmations`，
/// 面板展开 / badge 计数只看前者。
///
/// 报告也明确写了「不能只断言 `ListConfirmationsAsync` 返回了历史条目
/// —— 后端本来就返回，缺陷在消费端」，所以每条用例都走
/// **真实的 `ReloadProjectDataAsync` → `LoadConfirmationsAsync` 路径**，
/// 断言的是 VM 暴露给绑定的状态。
/// </summary>
public sealed class ConfirmationHistoryVisibilityTests
{
    /// <summary>
    /// 主用例：混合投喂后两个集合各就各位，且**待审计数不被历史撑起来**。
    ///
    /// 变异点就落在这里——把 `.Where(IsPendingConfirmation)` 加回去，
    /// `ResolvedConfirmations` 变空、`HasResolvedConfirmations` 变 false，本条转红。
    /// </summary>
    [Fact]
    public async Task ResolvedConfirmationsAreKeptSeparatelyInsteadOfBeingDropped()
    {
        var backend = HistoryBackend.Create();
        backend.Confirmations = new[]
        {
            Entry("confirmation-pending-1", "pending", "pending"),
            Entry("confirmation-approved-1", "approved", "时间线核对无误"),
            Entry("confirmation-rejected-1", "rejected", "与第 3 章的伤势设定冲突"),
            Entry("confirmation-auto-1", "auto_audited", "auto_audited"),
        };

        var viewModel = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();

        // ① 待审列表只放 pending：缺陷版本这一条也成立（它把别的都丢了），
        //    所以它单独证明不了修复——真正的判据是 ② 与 ③ 同时成立。
        var pending = Assert.Single(viewModel.Confirmations);
        Assert.Equal("confirmation-pending-1", pending.ConfirmationId);

        // ② 已决议项**没有被丢掉**，进了历史集合。缺陷版本在此变红。
        Assert.Equal(3, viewModel.ResolvedConfirmations.Count);
        Assert.True(viewModel.HasResolvedConfirmations);
        Assert.Equal(
            new[] { "confirmation-approved-1", "confirmation-auto-1", "confirmation-rejected-1" },
            viewModel.ResolvedConfirmations
                .Select(item => item.ConfirmationId)
                .OrderBy(id => id, StringComparer.Ordinal)
                .ToArray());

        // ③ 关键陷阱的护栏：待审计数**只数 pending**。
        //    若历史项混进 Confirmations，这里会是 4，且 HasPendingConfirmations 恒真
        //    ⇒ 审阅面板永久盖住画布，比原缺陷严重得多。
        Assert.True(viewModel.HasPendingConfirmations);
        Assert.Contains("1", viewModel.ConfirmationCountText, StringComparison.Ordinal);
        Assert.DoesNotContain("4", viewModel.ConfirmationCountText, StringComparison.Ordinal);
    }

    /// <summary>
    /// 呈现层护栏：历史项**真的到达界面**，不是只停在 VM 的一个集合里。
    ///
    /// 判据不取「集合非空」（那是上一条已经证过的事），而取
    /// **视图里存在把它绑出来的绑定**，且开合与可见性条件正确：
    /// - `ItemsSource={Binding ResolvedConfirmations}` 与
    ///   `DataType=ResolvedConfirmationItemViewModel` 的模板都在
    ///   ⇒ 集合非空时列表真的会渲染出行来（缺任一个都会**静默**不画）；
    /// - 折叠区整块绑 `HasResolvedConfirmations` ⇒ 无历史时不留空盒子；
    /// - 行内把 `ResultText` / `KindText` / `ReasonText` 都绑出来
    ///   ⇒ 「决议结果 / 类型 / 理由」这三样是报告点名要的内容。
    ///
    /// 为什么用源码断言：本机 Avalonia headless 只是部分可用，
    /// 而这里要验的是「绑定有没有写上」——绑定缺失在 Avalonia 里**既不编译期报错、
    /// 也不运行期抛异常**（U180 实测：`{Binding}` 打错名字只是那块不画），
    /// 所以文本层护栏在这类缺陷上与开窗等价，且不受 headless 盲区影响。
    /// 配套的 `dotnet build desktop/Ariadne.slnx` 负责保证这份 XAML 真的能编译。
    /// </summary>
    [Fact]
    public void ResolvedConfirmationsReachThePresentationLayer()
    {
        var axaml = File.ReadAllText(Path.Combine(ResolveDesktopDir("Views"), "WorkspacePageView.axaml"));

        // 集合真的被绑上，且有对应类型的模板——两者缺一，行就静默不画。
        Assert.Contains(
            "ItemsSource=\"{Binding ResolvedConfirmations}\"",
            axaml,
            StringComparison.Ordinal);
        Assert.Contains(
            "DataType=\"{x:Type vm:ResolvedConfirmationItemViewModel}\"",
            axaml,
            StringComparison.Ordinal);

        // 折叠区按「有没有历史」显隐，而**不是**按 HasPendingConfirmations——
        // 后者会让历史随待审项一起消失，等于没修。
        Assert.Contains(
            "IsVisible=\"{Binding HasResolvedConfirmations}\"",
            axaml,
            StringComparison.Ordinal);
        Assert.Contains(
            "IsVisible=\"{Binding IsConfirmationHistoryExpanded}\"",
            axaml,
            StringComparison.Ordinal);
        Assert.Contains(
            "Command=\"{Binding ToggleConfirmationHistoryCommand}\"",
            axaml,
            StringComparison.Ordinal);

        // 报告点名要的三样：决议结果 approved/rejected、Kind、理由。
        foreach (var binding in new[] { "ResultText", "KindText", "ReasonText", "DecidedAtText" })
        {
            Assert.Contains($"Text=\"{{Binding {binding}}}\"", axaml, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// 无待审项时历史仍然够得着（U187-A 的第二个陷阱）。
    ///
    /// 历史的呈现体在审阅面板里，而那个面板原本只在 `HasPendingConfirmations` 为真时渲染
    /// ⇒ 「昨天全审完、今天想回看批过什么」这个**最需要历史的时刻**恰恰打不开它。
    /// 所以判据取「全部已决议时，点入口后 `ShowConfirmationFullPanel` 为真」。
    ///
    /// ⚠️ 同时钉住反向性质：**面板不能自己弹开**。
    /// 若把 `HasResolvedConfirmations` 写进 `ShowConfirmationFullPanel` 的条件，
    /// 历史一旦存在面板就恒开、永久盖住画布——那正是这轮修复要避免的事。
    /// </summary>
    [Fact]
    public async Task HistoryStaysReachableWithNoPendingItemsWithoutHijackingTheCanvas()
    {
        var backend = HistoryBackend.Create();
        backend.Confirmations = new[]
        {
            Entry("confirmation-approved-1", "approved", "读过，没问题"),
            Entry("confirmation-rejected-1", "rejected", "与第 3 章冲突"),
        };

        var viewModel = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();

        // 前提：一条待审都没有，历史两条。
        Assert.Empty(viewModel.Confirmations);
        Assert.False(viewModel.HasPendingConfirmations);
        Assert.Equal(2, viewModel.ResolvedConfirmations.Count);

        // 反向性质：没点之前面板**不许**自己盖住画布，横幅也不该出现。
        Assert.False(viewModel.ShowConfirmationFullPanel);
        Assert.False(viewModel.ShowConfirmationBanner);

        // 点了入口才打开，并且直接展开到能看见内容（不是打开一个还要再点的空面板）。
        Assert.True(viewModel.ShowConfirmationHistoryCommand.TryExecute());
        Assert.True(viewModel.ShowConfirmationFullPanel);
        Assert.True(viewModel.IsConfirmationHistoryExpanded);

        // 「收起看画布」必须真的把画布还回来：按钮说了话要做到。
        Assert.True(viewModel.ToggleConfirmationPanelCommand.TryExecute());
        Assert.False(viewModel.ShowConfirmationFullPanel);
    }

    /// <summary>
    /// 历史行的文案是**人话**，不是后端的 snake_case 内部值。
    ///
    /// 两处真实陷阱，都会把内部值印到界面上：
    /// ① `state` 是 `approved` / `auto_audited` 这种内部串；
    /// ② `handling_method` 在**没写理由**时装的是状态词本身
    ///    （见 `commands.rs` 的 `confirmation_log_entry_from_runtime`：
    ///    `review_reason` 为空时回落到 `"approved"` / `"rejected"` / …）
    ///    ⇒ 直接当理由渲染会得到「理由：rejected」这种行。
    /// </summary>
    [Fact]
    public async Task HistoryRowsShowHumanCopyNotBackendSentinels()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = HistoryBackend.Create();
        backend.Confirmations = new[]
        {
            // 有真理由
            Entry("confirmation-rejected-1", "rejected", "与第 3 章的伤势设定冲突"),
            // 无理由：后端把状态词塞进 handling_method
            Entry("confirmation-approved-1", "approved", "approved"),
        };

        var viewModel = new WorkspacePageViewModel(names, backend.Client);
        await viewModel.ReloadProjectDataAsync();

        var rejected = Assert.Single(viewModel.ResolvedConfirmations, item => item.IsRejected);
        var approved = Assert.Single(viewModel.ResolvedConfirmations, item => item.IsApproved);

        // 结果词走文案键，不是把 state 原样印出来。
        Assert.Equal(names.Text("ui.workspace.confirmation.history.result.rejected"), rejected.ResultText);
        Assert.Equal(names.Text("ui.workspace.confirmation.history.result.approved"), approved.ResultText);
        Assert.DoesNotContain("[ui.", rejected.ResultText, StringComparison.Ordinal);

        // 真理由要显示出来。
        Assert.True(rejected.HasReason);
        Assert.Contains("与第 3 章的伤势设定冲突", rejected.ReasonText, StringComparison.Ordinal);

        // 哨兵值不许当理由：这里若为 true，界面上会出现「理由：approved」。
        Assert.False(approved.HasReason);
        Assert.Equal(string.Empty, approved.ReasonText);

        // Kind 是后端一直在发、前端从来没用的字段；历史里必须显示，
        // 否则 summarizer 批量产出的四类确认项在列表里长得一模一样。
        Assert.Contains("writer_correction_patch", rejected.KindText, StringComparison.Ordinal);
        Assert.DoesNotContain("[ui.", rejected.KindText, StringComparison.Ordinal);

        // 决议时间要能读出来（审计链要回答「什么时候批的」）。
        Assert.Contains("2023", rejected.DecidedAtText, StringComparison.Ordinal);
    }

    /// <summary>
    /// 那句**已经过期的注释必须已被删掉**。
    ///
    /// 报告特别点了这一条：注释写着「后端已只返回 pending；前端再保险过滤」，
    /// 而 `7378ddc` 之后后端**同时返回历史**。留着它，下一个人读到会认为过滤是安全的，
    /// 于是把 `.Where()` 加回来——这正是缺陷的掩护。
    ///
    /// 查的是**调用**而不是字符串出现：注释里提到历史决策是合理的，
    /// 所以只禁 `entries.Where(IsPendingConfirmation)` 这个实际过滤表达式。
    /// </summary>
    [Fact]
    public void LoadPathNoLongerFiltersHistoryOutAndTheStaleCommentIsGone()
    {
        var source = File.ReadAllText(
            Path.Combine(ResolveDesktopDir("ViewModels"), "WorkspacePageViewModel.cs"));

        Assert.DoesNotContain("entries.Where(IsPendingConfirmation)", source, StringComparison.Ordinal);
        // 只禁**断言句**「后端已只返回 pending；前端再保险过滤」，不禁这几个字本身：
        // 新注释里把那句作为历史引用起来解释「它为什么过期」是合理的记录，
        // 一并禁掉会逼人删掉解释、于是下一个人又失去了这段来龙去脉。
        // 这正是 AGENTS 那条「源码文本断言要查『调用』而非『字符串出现』」的同形要求。
        Assert.DoesNotContain("前端再保险过滤", source, StringComparison.Ordinal);

        // 反面：`IsPendingConfirmation` 本身要留着——它现在承担的是**分流**判定。
        Assert.Contains("IsPendingConfirmation(entry)", source, StringComparison.Ordinal);
        Assert.Contains("ResolvedConfirmations.Add(", source, StringComparison.Ordinal);
    }

    /// <summary>
    /// 从测试程序集位置回溯到 `desktop/Ariadne.Desktop/&lt;子目录&gt;`。
    /// </summary>
    private static string ResolveDesktopDir(string subDirectory)
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null)
        {
            var candidate = Path.Combine(dir.FullName, "Ariadne.Desktop", subDirectory);
            if (Directory.Exists(candidate))
            {
                return candidate;
            }
            dir = dir.Parent;
        }
        throw new DirectoryNotFoundException($"找不到 Ariadne.Desktop/{subDirectory}");
    }

    /// <summary>
    /// 构造一条确认项日志。<paramref name="handlingMethod"/> 对应后端的
    /// `handling_method`：写了理由时装理由，没写时装状态词本身
    /// （见 `commands.rs` 的 `confirmation_log_entry_from_runtime`）。
    /// </summary>
    private static ConfirmationLogEntry Entry(
        string confirmationId,
        string state,
        string handlingMethod,
        string kind = "writer_correction_patch",
        long timestampMs = 1_700_000_000_000) => new(
        confirmationId,
        kind,
        "writer",
        timestampMs,
        state,
        handlingMethod,
        $"{confirmationId} 的摘要",
        "  上下文\n- 旧的一句\n+ 新的一句",
        "default",
        "run-7");

    /// <summary>
    /// 只伪造确认项与画布两条路径；其余调用返回 default。
    /// DispatchProxy 要在运行时派生该类型，所以**不能** sealed。
    /// </summary>
    private class HistoryBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        public IReadOnlyList<ConfirmationLogEntry> Confirmations { get; set; } =
            Array.Empty<ConfirmationLogEntry>();

        public static HistoryBackend Create()
        {
            var client = Create<IAriadneBackendClient, HistoryBackend>();
            var backend = (HistoryBackend)(object)client;
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

            object? value;
            switch (targetMethod.Name)
            {
                case nameof(IAriadneBackendClient.ListConfirmationsAsync):
                    value = Confirmations;
                    break;
                case nameof(IAriadneBackendClient.LoadProjectCanvasAsync):
                    value = new WorkflowGraphData(
                        "default",
                        "Project Canvas",
                        Array.Empty<CanvasNode>(),
                        Array.Empty<CanvasEdge>(),
                        new Dictionary<string, object?>(),
                        ContentRevision: "canvas-revision");
                    break;
                default:
                    value = null;
                    break;
            }

            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                value ??= resultType.IsValueType ? Activator.CreateInstance(resultType) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }

            return value;
        }
    }
}
