using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U139 回归：确认项审阅面板——审阅 AI 产出是人机协作的核心界面，原实现三处都不对：
///
/// ① **位置**：面板是画布层内 `ZIndex=15` 的浮层，右侧还留着 `BorderThickness="0,0,1,0"`
///    那道边线，盖住画布但不占满页。
/// ② **形态**：diff 用 `TextBox IsReadOnly`（`MinHeight=360`，右栏最大的一块面）承载，
///    外加**常驻**的「驳回理由」输入框——审阅态默认摆着两个输入框，其中一个是假的。
/// ③ **引用确认项问 AI 无入口**：后端 `@确认项:<id>` 链路（`resolve_reference` →
///    `resolve_confirmation` 返回 summary+state+diff）**一直是通的**，但前端
///    `references = Array.Empty<string>()` 写死，等于能力齐备、入口不存在。
///
/// 判据全部落在**真实出站请求 / 用户可见状态**上，而不是「命令能否执行」——
/// 后者在缺陷版本下同样为真（`AskAi` 那条压根不存在，而两个输入框都「能执行」）。
/// </summary>
public sealed class ConfirmationReviewPanelTests
{
    private const string ConfirmationId = "confirmation-42";

    [Fact]
    public async Task AskingAiAboutConfirmationSendsItAsAReference()
    {
        var (viewModel, backend) = await CreateReviewingAsync();
        backend.ProjectAiCalls.Clear();

        Assert.True(viewModel.AskAiAboutConfirmationCommand.TryExecute());
        await WaitUntilAsync(() => backend.ProjectAiCalls.Count >= 1);

        // 判据落在**真实出站请求**上：缺陷版本里 references 被写死成 Array.Empty<string>()，
        // 后端因此永远收不到引用，`@确认项:<id>` 那条早已完整的展开链路从未被触发。
        // 只断言「命令能否执行」是不够的——缺陷版本连这个命令都没有，
        // 而一旦加上命令、忘了传 references，那种断言照样全绿。
        var call = Assert.Single(backend.ProjectAiCalls);
        Assert.NotNull(call.References);
        Assert.Contains($"@确认项:{ConfirmationId}", call.References!);
    }

    [Fact]
    public async Task ConfirmationReferenceCarriesThePrefixBackendCanParse()
    {
        var (viewModel, backend) = await CreateReviewingAsync();
        backend.ProjectAiCalls.Clear();

        Assert.True(viewModel.AskAiAboutConfirmationCommand.TryExecute());
        await WaitUntilAsync(() => backend.ProjectAiCalls.Count >= 1);

        // 前缀不是装饰：顶层 `parse_project_reference` 要求引用里含 ':' 或 '/'，
        // 裸 id 会被判成非法引用（只有内层 store 的 resolve_reference 容忍裸 id）。
        // 所以「传了 references」还不够，必须传成后端解析得动的形态。
        var reference = Assert.Single(Assert.Single(backend.ProjectAiCalls).References!);
        Assert.StartsWith("@确认项:", reference, StringComparison.Ordinal);
        Assert.EndsWith(ConfirmationId, reference, StringComparison.Ordinal);
    }

    [Fact]
    public async Task RejectReasonStaysHiddenUntilRejectIsClicked()
    {
        var (viewModel, _) = await CreateReviewingAsync();

        // 缺陷版本：理由输入框是**常驻**的（面板底部一行标签 + 一个 TextBox），
        // 于是审阅态默认摆着两个输入框——其中一个（只读 diff）是假的，
        // 另一个（理由）在绝大多数场景（点通过）里根本不需要填。
        Assert.False(viewModel.IsRejectArmed);

        Assert.True(viewModel.RejectConfirmationCommand.TryExecute());

        // 第一次点「驳回」只展开理由线，**不提交**；同一按钮变成「确认拒绝」。
        Assert.True(viewModel.IsRejectArmed);
        Assert.Equal(
            DisplayNameService.LoadDefault().Text("ui.workspace.confirmation.reject.commit"),
            viewModel.RejectButtonText);
    }

    [Fact]
    public async Task FirstRejectClickDoesNotResolveTheConfirmation()
    {
        var (viewModel, backend) = await CreateReviewingAsync();

        Assert.True(viewModel.RejectConfirmationCommand.TryExecute());
        // 给异步提交留出窗口：若第一次点击真的提交了，这段时间足够它落到 mock 上。
        await Task.Delay(60);

        // 「点驳回后展开理由」这个设计只有在**第一次点击不提交**时才成立。
        // 若第一次就提交，理由框展开出来也没用——用户还没写就已经驳回了。
        Assert.Empty(backend.ResolveCalls);
        Assert.True(viewModel.IsRejectArmed);
    }

    [Fact]
    public async Task SecondRejectClickSubmitsWithTheTypedReason()
    {
        var (viewModel, backend) = await CreateReviewingAsync();

        Assert.True(viewModel.RejectConfirmationCommand.TryExecute());
        viewModel.ConfirmationReason = "与第 2 章的伤势设定冲突";
        Assert.True(viewModel.RejectConfirmationCommand.TryExecute());
        await WaitUntilAsync(() => backend.ResolveCalls.Count >= 1);

        // 判据取**出站请求**里的 decision 与 review_reason：理由若没随请求发出，
        // 那条展开的输入线就只是个摆设（写了也丢），比不给输入框更糟。
        var call = Assert.Single(backend.ResolveCalls);
        Assert.Equal("reject", call.Decision);
        Assert.Equal("与第 2 章的伤势设定冲突", call.ReviewReason);
        Assert.Equal(ConfirmationId, call.ConfirmationId);
    }

    [Fact]
    public async Task ApproveButtonCancelsRejectInsteadOfApprovingWhileArmed()
    {
        var (viewModel, backend) = await CreateReviewingAsync();

        Assert.True(viewModel.RejectConfirmationCommand.TryExecute());
        Assert.True(viewModel.ApproveOrCancelCommand.TryExecute());
        await Task.Delay(60);

        // 武装态下主按钮是**退出口**而不是「通过」：否则用户点了驳回、想反悔，
        // 手边那个键会直接把 AI 的产出放行——与他刚表达的意图正好相反。
        Assert.Empty(backend.ResolveCalls);
        Assert.False(viewModel.IsRejectArmed);
    }

    [Fact]
    public async Task DiffIsExposedAsColouredLinesNotOneTextBlob()
    {
        var (viewModel, _) = await CreateReviewingAsync(
            "  第一段保持原样\n- 旧的一句\n+ 新的一句");

        // 缺陷版本用 `TextBox IsReadOnly` + MinHeight=360 承载 diff：一个字也打不进去，
        // 却会亮边、抢焦点、占 Tab 停靠位，而且**无法分行着色**——增删两侧看起来一模一样。
        // 判据取「VM 暴露的是可着色的行集合」而非「有没有 Diff 字符串」：
        // 后者在缺陷版本下也是真（它一直有 SelectedConfirmation.Diff）。
        Assert.True(viewModel.HasConfirmationDiff);
        Assert.Equal(3, viewModel.ConfirmationDiffLines.Count);

        var removed = Assert.Single(viewModel.ConfirmationDiffLines, line => line.IsRemoved);
        var added = Assert.Single(viewModel.ConfirmationDiffLines, line => line.IsAdded);
        // 行首标记被剥掉、类别落到 IsAdded/IsRemoved 上——着色才有依据可绑。
        Assert.Equal("旧的一句", removed.Text);
        Assert.Equal("新的一句", added.Text);
        Assert.Equal("第一段保持原样", viewModel.ConfirmationDiffLines[0].Text);
    }

    [Fact]
    public async Task DiffPanelIsNotRenderedWhenTheConfirmationCarriesNoPatch()
    {
        var (viewModel, _) = await CreateReviewingAsync(diff: string.Empty);

        // 没有 diff 时不该留一块空的大面（U130 同类问题：常驻空位照占版面）。
        Assert.False(viewModel.HasConfirmationDiff);
        Assert.Empty(viewModel.ConfirmationDiffLines);
    }

    [Fact]
    public async Task EnteringReviewSwitchesTheRightRailToProjectAi()
    {
        var backend = ReviewBackend.Create();
        backend.Confirmations = new[] { PendingConfirmation("- 旧\n+ 新") };
        var viewModel = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        // 先把右栏停在节点检查器：这正是缺陷版本进入审阅态时的样子。
        Assert.True(viewModel.ShowNodeDetailsCommand.TryExecute());
        Assert.True(viewModel.IsNodeDetailsTab);

        await viewModel.ReloadProjectDataAsync();

        // 审阅面板此刻替换了整个画布位置，节点检查器（讲的是画布上选中的节点）无事可讲；
        // 用户需要的是问 AI「这段改得对不对」。右栏**不隐藏**——它与面板分列两列。
        Assert.True(viewModel.ShowConfirmationFullPanel);
        Assert.True(viewModel.IsProjectAiTab);
        Assert.True(viewModel.IsRightPanelOpen);
    }

    [Fact]
    public async Task ManualTabChoiceDuringReviewIsNotOverriddenByRefresh()
    {
        var (viewModel, _) = await CreateReviewingAsync();
        Assert.True(viewModel.IsProjectAiTab);

        // 审阅中用户自己切去看节点配置，然后刷新确认项列表。
        Assert.True(viewModel.ShowNodeDetailsCommand.TryExecute());
        Assert.True(viewModel.RefreshConfirmationsCommand.TryExecute());
        await Task.Delay(60);

        // 自动切换只在**进入**审阅态时发生一次。否则每次刷新都把用户拽回项目 AI 页，
        // 等于剥夺了切换能力（U133 的教训：默认值不该变成不可逆的强制）。
        Assert.True(viewModel.IsNodeDetailsTab);
    }

    /// <summary>构造一个已加载待审项的审阅态 ViewModel（走真实 LoadConfirmations 路径）。</summary>
    private static async Task<(WorkspacePageViewModel ViewModel, ReviewBackend Backend)> CreateReviewingAsync(
        string diff = "  第一段保持原样\n- 旧的一句\n+ 新的一句")
    {
        var backend = ReviewBackend.Create();
        backend.Confirmations = new[] { PendingConfirmation(diff) };
        var viewModel = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();
        return (viewModel, backend);
    }

    private static async Task WaitUntilAsync(Func<bool> condition)
    {
        for (var attempt = 0; attempt < 200; attempt++)
        {
            if (condition())
            {
                return;
            }
            await Task.Delay(10);
        }
        Assert.Fail("等待条件超时");
    }

    private static ConfirmationLogEntry PendingConfirmation(string diff) => new(
        ConfirmationId,
        "writing_patch",
        "writer",
        1_700_000_000_000,
        "pending",
        "pending",
        "第 3 章第二段改写待审",
        diff,
        "default",
        "run-7");

    private class ReviewBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        public IReadOnlyList<ConfirmationLogEntry> Confirmations { get; set; } =
            Array.Empty<ConfirmationLogEntry>();

        /// <summary>每次 project_ai_chat 的**出站参数**，判据就落在这里。</summary>
        public List<ProjectAiCall> ProjectAiCalls { get; } = new();

        /// <summary>每次 resolve_confirmation 的出站参数：判「有没有真的提交」用。</summary>
        public List<ResolveCall> ResolveCalls { get; } = new();

        public static ReviewBackend Create()
        {
            var client = Create<IAriadneBackendClient, ReviewBackend>();
            var backend = (ReviewBackend)(object)client;
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
                case nameof(IAriadneBackendClient.ProjectAiChatAsync):
                    value = RecordProjectAiCall(targetMethod, args);
                    break;
                case nameof(IAriadneBackendClient.ResolveConfirmationAsync):
                    ResolveCalls.Add(new ResolveCall(
                        (string?)args?[2] ?? string.Empty,
                        (string?)args?[3] ?? string.Empty,
                        (string?)args?[4]));
                    value = new ResolveConfirmationResult(
                        new WorkflowActionResult("default", "run-7", "running"),
                        Confirmations.Count > 0
                            ? Confirmations[0]
                            : PendingConfirmation(string.Empty),
                        new SidebarBadgeCounts(0, 0, 0));
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
                // 未显式伪造的调用返回 default：本用例只关心确认项与 project_ai_chat 两条路径，
                // 其余（模型列表、章节选项等）失败会被 VM 的 catch 写进 StatusText，不影响判据。
                value ??= resultType.IsValueType ? Activator.CreateInstance(resultType) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }

            return value;
        }

        /// <summary>按**形参名**取值：签名里新增可选参数时不会静默取错位置。</summary>
        private ProjectAiResponse RecordProjectAiCall(MethodInfo method, object?[]? args)
        {
            var parameters = method.GetParameters();
            object? ByName(string name)
            {
                for (var index = 0; index < parameters.Length; index++)
                {
                    if (parameters[index].Name == name)
                    {
                        return args?[index];
                    }
                }
                return null;
            }

            ProjectAiCalls.Add(new ProjectAiCall(
                (string?)ByName("message") ?? string.Empty,
                ByName("references") as IReadOnlyList<string>,
                (string?)ByName("referenceWorkflowId"),
                (string?)ByName("referenceRunId")));

            return new ProjectAiResponse(
                "这段与第 2 章的设定冲突。",
                Array.Empty<ProjectAiChatMessage>(),
                null,
                string.Empty);
        }
    }

    public sealed record ProjectAiCall(
        string Message,
        IReadOnlyList<string>? References,
        string? ReferenceWorkflowId,
        string? ReferenceRunId);

    public sealed record ResolveCall(
        string ConfirmationId,
        string Decision,
        string? ReviewReason);
}
