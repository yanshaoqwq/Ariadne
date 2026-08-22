using System.Collections.Concurrent;
using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U194-A / U194-B / U194-C 回归。
///
/// ## 三条各自的核实结论（施工前逐条查过源码）
///
/// **A（部分已健康）**：报告说「页级 `StatusText` 没有任何显示位置」——
/// 这一半**已被 U198-B 修掉**：`WorksPageView.axaml:366-392` 有 `WorksStatusHost` 浮层
/// 绑 `HasStatusText` / `StatusText` / `RecoveryText`。
/// 剩下的是报告标了「要更强」的那一半：**保存失败没有持续可见的痕迹**。
/// `DocumentSaveStateText`（`WorksPageViewModel.cs:991`）只有
/// 保存中 / 未保存 / 已保存三态，**保存失败与「还没点过保存」完全同形**（同一句「未保存」、
/// 同一个 `subtle` 灰）。而 `StatusText` 浮层会被作者的**下一个动作**顶掉
/// （每次赋值都覆盖），失败的痕迹随即消失，正文却还没落盘。
///
/// **B（成立）**：`NotificationText` 有 10 处赋值（版本号 / 初始化失败 / 离开项目失败 /
/// 反馈打开失败 / 维护横幅 / 批量保存部分失败 / 全失败 / 崩回欢迎页 / 中断日志），
/// 全写同一个属性、渲染成同一条 `Classes="subtle"` 灰字 + 一个 Fill 写死
/// `Ariadne.TextSubtle` 的 `Ellipse`（`MainWindow.axaml:138-146`）。
/// 全仓 `NotificationSeverity` 零命中 ⇒ 无分级、后写的静默覆盖先写的。
///
/// **C（成立，且是「做一半」形态）**：U194-E 落地了「终态刷预算 + 刷角标」，
/// 但触发它的 `RunTerminalStateNotifier.NotifyTerminal` 挂在
/// `WorkspaceRunSessionCoordinator.UpdateState` 上，而该状态跃迁只由**轮询**推进；
/// `MainWindowViewModel:1423` 切页时对离开的页调 `DeactivateProjectData()`，
/// 实现（`WorkspacePageViewModel.cs:4373-4377`）就是 `_runSession.CancelPolling()`
/// ⇒ **一离开画布页轮询就停，终态广播永不发生**。
/// 于是 E 条那套刷新只在「作者一直盯着画布页」时有效，而 C 条描述的正是他切走的场景。
/// </summary>
public sealed class NotificationSeverityAndSaveFailureTests
{
    // ==================== A：保存失败的持续可见痕迹 ====================

    /// <summary>
    /// 判据取**真实失败路径**：让假后端在 `SaveDocumentContentAsync` 上抛，
    /// 然后断言常驻状态行读出来的那个字符串确实是「保存失败」那一句。
    ///
    /// ⚠️ 刻意不断言「文案非空」——缺陷版本里它是「未保存」，同样非空，那种断言恒绿。
    /// 也刻意不断言「`StatusText` 被赋值」：报告原话「那正是缺陷版本的行为」。
    /// </summary>
    [Fact]
    public async Task SaveFailurePaintsThePersistentUnsavedMarkerAsError()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = WorksBackend.Create();
        var page = SeedEditedDocument(backend, names);

        backend.FailSave = true;
        await InvokeSaveAsync(page);

        Assert.True(page.HasSaveFailure);
        Assert.Equal(names.Text("ui.works.save_state.save_failed"), page.DocumentSaveStateText);
        // 与「还没点过保存」区分开才是本条的全部意义 —— 那一档是这句话：
        Assert.NotEqual(names.Text("ui.works.save_state.unsaved"), page.DocumentSaveStateText);
    }

    /// <summary>
    /// 反向钉住：**保存成功不许留下失败痕迹**。
    /// 少了这条，把标志位写成「一旦置位永不复位」也能让上一条全绿。
    /// </summary>
    [Fact]
    public async Task SuccessfulSaveClearsTheFailureMarker()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = WorksBackend.Create();
        var page = SeedEditedDocument(backend, names);

        backend.FailSave = true;
        await InvokeSaveAsync(page);
        Assert.True(page.HasSaveFailure, "前置条件：先造出失败态，否则这条什么都没验");

        // 作者改好网络/权限后重试。
        backend.FailSave = false;
        await InvokeSaveAsync(page);

        Assert.False(page.HasSaveFailure);
        Assert.Equal(names.Text("ui.works.save_state.saved"), page.DocumentSaveStateText);
    }

    /// <summary>
    /// 失败痕迹必须**熬过作者的下一个动作**——这正是它与 `StatusText` 浮层的分工。
    ///
    /// `StatusText` 全页 51 处赋值，随便点一下就把失败那句顶掉；
    /// 而此刻正文仍未落盘。所以「一直可见」这件事不能交给它。
    /// </summary>
    [Fact]
    public async Task FailureMarkerSurvivesTheNextStatusTextAssignment()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = WorksBackend.Create();
        var page = SeedEditedDocument(backend, names);

        backend.FailSave = true;
        await InvokeSaveAsync(page);
        var failureStatus = page.StatusText;

        // 模拟作者的下一个动作（任何一处 StatusText 赋值都同形）：浮层那句话被顶掉。
        page.StatusText = names.Text("ui.works.quick_edit.undone");

        Assert.NotEqual(failureStatus, page.StatusText);
        // 而常驻位仍在指控这一篇没落盘：
        Assert.True(page.HasSaveFailure);
        Assert.Equal(names.Text("ui.works.save_state.save_failed"), page.DocumentSaveStateText);
    }

    /// <summary>
    /// 钉住**屏上真的有渲染位**。本仓反复出现「有实现、有维护、界面零消费」形态
    /// （`RecoveryText` 原状就是），所以 VM 属性对了不算修好。
    ///
    /// 断言查的是「那个属性被某个可见元素消费」+「主题里有对应的样式规则」——
    /// Avalonia 缺资源键/缺样式规则**静默失效**（不报错、不回落），
    /// 只绑 Classes 而没有样式的话屏幕上一点变化都没有。
    /// </summary>
    [Fact]
    public void SaveFailureMarkerHasARenderSiteInTheWorksPage()
    {
        var view = ReadSource("Views/WorksPageView.axaml");
        // 常驻状态行消费了失败标志：
        Assert.Contains("Classes.save-failed=\"{Binding HasSaveFailure}\"", view);
        Assert.Contains("Text=\"{Binding DocumentSaveStateText}\"", view);

        var theme = ReadSource("Resources/Styles/AriadneTheme.axaml");
        Assert.Contains("Selector=\"TextBlock.save-failed\"", theme);
        // 颜色走令牌，不是魔法值。
        Assert.Contains("{DynamicResource Ariadne.StatusError}", theme);

        // ⚠️ Avalonia 同优先级按**文档顺序、后者胜**，没有 CSS 那种特异性权重。
        // `TextBlock.subtle` 也设 Foreground，`.save-failed` 声明在它**之前**
        // 就会被整条盖掉（U154 正是这么失效的）——这条断言守的是那个顺序。
        Assert.True(
            theme.IndexOf("Selector=\"TextBlock.save-failed\"", StringComparison.Ordinal)
            > theme.IndexOf("Selector=\"TextBlock.subtle\"", StringComparison.Ordinal),
            "TextBlock.save-failed 必须声明在 TextBlock.subtle 之后，否则 Foreground 被盖掉、屏幕上没有任何变化");
    }

    // ==================== B：顶栏通知分级 ====================

    /// <summary>
    /// 信息类留在 Info 档。走的是**真实入口** `ApplyMaintenanceState`
    /// （生产里由后端维护状态回包调用，`MainWindowViewModel.cs:246`），不新开钩子。
    /// </summary>
    [Fact]
    public void InformationalNoticesStayAtInfoSeverity()
    {
        var window = NewWindow();

        // 维护**进行中**：作者不需要做任何事，等它跑完即可。
        window.ApplyMaintenanceState(new ProjectMaintenanceState("reindex", "active", "embedding", null));

        Assert.Equal(HeaderNoticeSeverity.Info, window.HeaderNoticeSeverity);
        Assert.False(window.IsHeaderNoticeWarning);
        Assert.False(window.IsHeaderNoticeError);
    }

    /// <summary>
    /// 有风险的那一类升到 Error 档。同一个入口、只把 `status` 换成 `failed`——
    /// 这样两条用例的差异**只有严重度这一个变量**，而不是两条不同的代码路径。
    /// </summary>
    [Fact]
    public void DataRiskNoticesAreRaisedToErrorSeverity()
    {
        var window = NewWindow();

        window.ApplyMaintenanceState(new ProjectMaintenanceState("reindex", "failed", "embedding", "磁盘写满"));

        Assert.Equal(HeaderNoticeSeverity.Error, window.HeaderNoticeSeverity);
        Assert.True(window.IsHeaderNoticeError);
        // 同一档不能同时点亮两个类，否则样式会叠加成不确定的结果。
        Assert.False(window.IsHeaderNoticeWarning);
    }

    /// <summary>
    /// 反向钉住：**低优先级不许升级**。否则「全部按最高级显示」也能让上一条全绿——
    /// 那是把「都看不见」换成「都在喊」。
    ///
    /// 判据取「同一个属性在两种输入下必须**不同**」，而不是「error 那次为真」：
    /// 后者在「恒为 error」的实现下同样全绿。
    /// </summary>
    [Fact]
    public void EveryNoticeIsNotUniformlyEscalated()
    {
        var failed = NewWindow();
        failed.ApplyMaintenanceState(new ProjectMaintenanceState("reindex", "failed", "embedding", "磁盘写满"));

        var active = NewWindow();
        active.ApplyMaintenanceState(new ProjectMaintenanceState("reindex", "active", "embedding", null));

        // 两者的通知文案都非空（都在屏上），但呈现必须分得开。
        Assert.NotEqual(string.Empty, failed.HeaderStatusText);
        Assert.NotEqual(string.Empty, active.HeaderStatusText);
        Assert.NotEqual(failed.HeaderNoticeSeverity, active.HeaderNoticeSeverity);
        Assert.True(failed.IsHeaderNoticeError, "有风险的那条必须醒目");
        Assert.False(active.IsHeaderNoticeError, "纯信息不许跟着一起报红——那只是把「都看不见」换成「都在喊」");

        // 而运行终态的成功通知也属信息类，同样不许升级：
        var finished = NewWindow();
        finished.MarkProjectOpenForTests();
        ((IRunTerminalStateObserver)finished).OnRunReachedTerminalState("wf-1", "run-1", "succeeded");
        Assert.Equal(HeaderNoticeSeverity.Info, finished.HeaderNoticeSeverity);
        Assert.False(finished.IsHeaderNoticeError);
    }

    /// <summary>
    /// 原缺陷的核心那一半：**后写的静默覆盖先写的**。
    /// 「批量保存第 3 页失败」（正文可能已丢）被随后任意一条信息类顶掉，那条消息就此消失。
    ///
    /// 用运行终态的成功通知当「后来那条」，因为它是生产里**最可能**紧随其后发生的
    /// 信息类通知（作者切走写作、后台工作流跑完）—— 恰好是 C 条新接的那条线。
    /// </summary>
    [Fact]
    public void ErrorSeverityIsNotOverwrittenByALaterInfoNotice()
    {
        var window = NewWindow();
        window.MarkProjectOpenForTests();

        window.ApplyMaintenanceState(new ProjectMaintenanceState("reindex", "failed", "embedding", "磁盘写满"));
        var errorText = window.HeaderStatusText;
        Assert.True(window.IsHeaderNoticeError, "前置条件：先造出 Error 档，否则这条什么都没验");

        ((IRunTerminalStateObserver)window).OnRunReachedTerminalState("wf-1", "run-1", "succeeded");

        Assert.Equal(errorText, window.HeaderStatusText);
        Assert.True(window.IsHeaderNoticeError);

        // 但**清空之后**低档必须能写进去 —— 否则一次 Error 会把顶栏永久锁死，
        // 那比不分级更糟（作者处理完了，界面还在指控同一件事）。
        window.NotificationText = string.Empty;
        ((IRunTerminalStateObserver)window).OnRunReachedTerminalState("wf-2", "run-2", "succeeded");
        Assert.NotEqual(string.Empty, window.NotificationText);
        Assert.Equal(HeaderNoticeSeverity.Info, window.HeaderNoticeSeverity);
    }

    /// <summary>
    /// 钉住顶栏**真的有渲染位**，且分档没有被写成恒定值。
    /// 原缺陷正是「`Ellipse` 的 Fill 写死 `Ariadne.TextSubtle`，不随严重度变」。
    /// </summary>
    [Fact]
    public void NotificationSeverityHasARenderSiteInTheHeader()
    {
        var view = ReadSource("Views/MainWindow.axaml");
        Assert.Contains("Classes.notice-warning=\"{Binding IsHeaderNoticeWarning}\"", view);
        Assert.Contains("Classes.notice-error=\"{Binding IsHeaderNoticeError}\"", view);
        // 恒定灰的内联 Fill 必须已经不在那个小圆点上 —— 内联值优先级高于样式，
        // 留着它分档样式压根压不掉（AGENTS「内联属性压不掉模板层样式」的同族陷阱）。
        Assert.DoesNotContain("<Ellipse Width=\"6\" Height=\"6\" Fill=", view);

        var theme = ReadSource("Resources/Styles/AriadneTheme.axaml");
        Assert.Contains("Selector=\"Ellipse.header-notice.notice-error\"", theme);
        Assert.Contains("Selector=\"Ellipse.header-notice.notice-warning\"", theme);
        Assert.Contains("Selector=\"TextBlock.notice-error\"", theme);

        // 同 A 条：`TextBlock.subtle` 也设 Foreground，分档样式必须在它之后声明，
        // 否则被整条盖掉、屏幕上没有任何变化（Avalonia 无选择器特异性权重）。
        Assert.True(
            theme.IndexOf("Selector=\"TextBlock.notice-error\"", StringComparison.Ordinal)
            > theme.IndexOf("Selector=\"TextBlock.subtle\"", StringComparison.Ordinal),
            "TextBlock.notice-error 必须声明在 TextBlock.subtle 之后");
    }

    // ==================== C：离页后仍能知道跑完了 ====================

    /// <summary>
    /// C 条核心：**离开画布页后仍在问「跑完了吗」**。
    ///
    /// 原缺陷是 `DeactivateProjectData()` → `CancelPolling()`，轮询整个停掉。
    /// 判据取「后台**真的又打了一次 IPC**」——`GetWorkflowRunStateAsync` 的调用次数，
    /// 那是这条链路上唯一属于生产代码的可观测量。
    ///
    /// ⚠️ 刻意不断言「`WatchTerminalStateInBackground` 被调用过」：那只证明接了线，
    /// 不证明线通着（U194-E 就是接了线而上游轮询已停，整条路死掉）。
    /// </summary>
    [Fact]
    public async Task LeavingTheCanvasKeepsWatchingForTheTerminalState()
    {
        var backend = RunWatchBackend.Create();
        using var session = new WorkspaceRunSessionCoordinator(
            backend.Client,
            pollInterval: TimeSpan.FromMilliseconds(5),
            backgroundWatchInterval: TimeSpan.FromMilliseconds(5));

        // 作者在画布页启动了一个长任务。
        session.Attach("wf-1", "run-1", "running", resetCursor: true);

        // 他切到作品页写作 —— 生产里就是 DeactivateProjectData() 这一下。
        session.WatchTerminalStateInBackground();

        await WaitUntilAsync(() => backend.RunStateRequests.Count >= 1);
        Assert.True(
            backend.RunStateRequests.Count >= 1,
            "离页后必须继续问运行状态，否则跑完了作者永远不知道");
        Assert.Equal(("wf-1", "run-1"), backend.RunStateRequests.First());
    }

    /// <summary>
    /// 反向钉住：**运行已在终态 / 压根没在跑时不许留下后台轮询**。
    ///
    /// 少了这条，把 `WatchTerminalStateInBackground` 写成「无条件起一个循环」也全绿，
    /// 而那样切十次页面就留十个后台循环在打同一个 IPC —— 那是用一个资源泄漏
    /// 换一个通知，不是修复。
    /// </summary>
    [Fact]
    public async Task NoBackgroundWatchIsLeftForARunThatAlreadyFinished()
    {
        var backend = RunWatchBackend.Create();
        using var session = new WorkspaceRunSessionCoordinator(
            backend.Client,
            pollInterval: TimeSpan.FromMilliseconds(5),
            backgroundWatchInterval: TimeSpan.FromMilliseconds(5));

        // 已经跑完了才切页：没有什么可等的。
        session.Attach("wf-1", "run-1", "succeeded", resetCursor: true);
        session.WatchTerminalStateInBackground();
        await Task.Delay(80);
        Assert.Empty(backend.RunStateRequests);

        // 压根没在跑（作者从没点运行就切页）：同样不许留循环。
        using var idle = new WorkspaceRunSessionCoordinator(
            backend.Client,
            pollInterval: TimeSpan.FromMilliseconds(5),
            backgroundWatchInterval: TimeSpan.FromMilliseconds(5));
        idle.WatchTerminalStateInBackground();
        await Task.Delay(80);
        Assert.Empty(backend.RunStateRequests);
    }

    /// <summary>
    /// 走到头：后台监视等到终态 → 广播 → 顶栏出现「跑完了」。
    ///
    /// 这是唯一一条覆盖**整条链路**的用例（协调器 → `RunTerminalStateNotifier`
    /// → `MainWindowViewModel.OnRunReachedTerminalState` → 顶栏文案）。
    /// 判据取顶栏那个字符串**等于**「工作流已跑完」那一句，不是「非空」：
    /// 缺陷版本里它是后端健康文案，同样非空。
    /// </summary>
    [Fact]
    public async Task TerminalStateReachedWhileAwayNotifiesTheHeader()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = RunWatchBackend.Create();
        var window = new MainWindowViewModel(names, WorksBackend.Create().Client);
        window.MarkProjectOpenForTests();

        using var session = new WorkspaceRunSessionCoordinator(
            backend.Client,
            pollInterval: TimeSpan.FromMilliseconds(5),
            backgroundWatchInterval: TimeSpan.FromMilliseconds(5));
        session.Attach("wf-1", "run-1", "running", resetCursor: true);

        // 作者切走，后台监视接手；此后后端才报出终态。
        session.WatchTerminalStateInBackground();
        backend.Status = "succeeded";

        await WaitUntilAsync(() => session.Status == "succeeded");
        await WaitUntilAsync(() => window.NotificationText.Length > 0);

        Assert.Equal(names.Text("ui.layout.notice.run_succeeded"), window.NotificationText);
        Assert.Equal(names.Text("ui.layout.notice.run_succeeded"), window.HeaderStatusText);
        // 成功是好消息，不许报红（分级的反向要求同样适用于这条新通道）。
        Assert.Equal(HeaderNoticeSeverity.Info, window.HeaderNoticeSeverity);
    }

    /// <summary>
    /// **钉住接线点本身**：画布页的 `DeactivateProjectData()`（切页时被调）
    /// 必须交给后台监视，而不是把轮询掐掉。
    ///
    /// ⚠️ 这条是变异测试逼出来的：把 `WatchTerminalStateInBackground()` 换回
    /// `CancelPolling()` 之后，前三条 C 用例**全绿** —— 它们直接对协调器调方法，
    /// 压根没经过生产里那个唯一的调用点。那正是本仓反复出现的
    /// 「有实现、有测试覆盖、生产零调用者」形态，也是 U194-E 本身踩过的坑。
    ///
    /// 判据取「切页之后后端**还在被问**运行状态」——离页那一刻的用户可见后果。
    /// </summary>
    [Fact]
    public async Task DeactivatingTheCanvasPageHandsOffToTheBackgroundWatch()
    {
        var backend = RunWatchBackend.Create();
        var page = new WorkspacePageViewModel(DisplayNameService.LoadDefault(), backend.Client);

        // 让页面进入「有一个运行在跑」的状态。走 AttachRunForTests 而不是真跑一遍工作流：
        // 本条要守的是**离页那一下**的行为，起一次真实运行只会引入无关的失败面。
        page.AttachRunForTests("wf-1", "run-1", "running");

        // 作者切到作品页 —— 生产里 MainWindowViewModel 对离开的页调的就是这个。
        page.DeactivateProjectData();

        // ⚠️ 这条**必须**等够生产的真实间隔（`DefaultBackgroundWatchInterval` = 3000ms）：
        // 页面 VM 自己 new 协调器，间隔不可注入。给它开一个注入口只为把测试跑快，
        // 会让用例不再验证生产实际用的那个值 —— 宁可这一条慢 3 秒。
        await WaitUntilAsync(() => backend.RunStateRequests.Count >= 1, attempts: 600);
        Assert.True(
            backend.RunStateRequests.Count >= 1,
            "DeactivateProjectData 必须把轮询降级成后台终态监视，而不是 CancelPolling —— "
            + "掐掉之后跑完/失败/停下作者在别的页面上永远不会知道");
    }

    // ==================== 共用件 ====================
    /// <summary>
    /// 顶栏 VM。B 条的判据全落在它暴露给 View 的呈现状态上。
    /// </summary>
    private static MainWindowViewModel NewWindow() =>
        new(DisplayNameService.LoadDefault(), WorksBackend.Create().Client);

    /// <summary>
    /// 轮询而不是固定 `Delay`：固定延时在这台负载很高的 ARM 机上会偶发失败，
    /// 而轮询在快的时候立刻返回、慢的时候多等一会儿。
    /// 超时后**不抛**——让紧随其后的 `Assert` 给出真正有信息量的失败信息。
    /// </summary>
    private static async Task WaitUntilAsync(Func<bool> condition, int attempts = 200)
    {
        for (var i = 0; i < attempts && !condition(); i++)
        {
            await Task.Delay(10);
        }
    }

    /// <summary>
    /// 假后端：只回答「运行状态」，并记下被问过哪些 (workflow, run)。
    /// C 条判据（离页后是否还在问）取的就是这份记录。
    /// </summary>
    private class RunWatchBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        /// <summary>后端此刻报的运行态。用例中途改它来模拟「跑完了」。</summary>
        public string Status { get; set; } = "running";

        public ConcurrentQueue<(string WorkflowId, string RunId)> RunStateRequestQueue { get; } = new();

        public IReadOnlyList<(string WorkflowId, string RunId)> RunStateRequests =>
            RunStateRequestQueue.ToArray();

        public static RunWatchBackend Create()
        {
            var client = Create<IAriadneBackendClient, RunWatchBackend>();
            var backend = (RunWatchBackend)(object)client;
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
            if (targetMethod.Name == nameof(IAriadneBackendClient.GetWorkflowRunStateAsync))
            {
                var workflowId = args?[0] as string ?? string.Empty;
                var runId = args?[1] as string ?? string.Empty;
                RunStateRequestQueue.Enqueue((workflowId, runId));
                return Task.FromResult(new WorkflowRunState(
                    workflowId,
                    runId,
                    Status,
                    null,
                    null,
                    null,
                    Array.Empty<string>()));
            }
            if (targetMethod.Name == nameof(IAriadneBackendClient.GetWorkflowEventsAsync))
            {
                // 事件轮询在这些用例里只是背景噪音：给一个永不完成的任务，
                // 让它既不推进状态、也不报错。真正被观测的是 GetWorkflowRunStateAsync。
                // ⚠️ 不能返回已完成的 result —— 那会让事件轮询自己把状态推到终态，
                // 于是「后台监视有没有在工作」这件事就测不出来了（数据来自别处）。
                return new TaskCompletionSource<WorkflowEventsResult>().Task;
            }
            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var payload = targetMethod.ReturnType.GetGenericArguments()[0];
                var value = payload.IsValueType ? Activator.CreateInstance(payload) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(payload)
                    .Invoke(null, new[] { value });
            }
            return null;
        }
    }

    /// <summary>
    /// 打开一篇文档并让它变脏（有未保存改动）—— 保存路径的前置条件。
    /// </summary>
    private static WorksPageViewModel SeedEditedDocument(WorksBackend backend, DisplayNameService names)
    {
        var page = new WorksPageViewModel(names, backend.Client);
        page.SeedOpenDocumentForTests("documents/chapter-1.md", "v1", "第一段。");
        page.DocumentContent = "第一段。作者又写了一句。";
        return page;
    }

    /// <summary>
    /// 读源码文本。渲染位判据只能这么取：本机 Avalonia headless 对控件子类会挂，
    /// 而 XAML 里「绑了没绑」这个事实本身是文本可判定的。
    /// ⚠️ 这层**不过 XAML 编译**，所以改 axaml 后必须另跑一次 `dotnet build`
    /// （AGENTS 记过：曾提交过编译不通过的主题文件而测试全绿）。
    /// </summary>
    private static string ReadSource(string relativePath)
    {
        var root = FindRepoRoot();
        var full = Path.Combine(root, "desktop", "Ariadne.Desktop", relativePath);
        Assert.True(File.Exists(full), $"取证源文件不存在：{full}");
        return File.ReadAllText(full);
    }

    private static string FindRepoRoot()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null && !Directory.Exists(Path.Combine(dir.FullName, "core", "resources")))
        {
            dir = dir.Parent;
        }
        Assert.NotNull(dir);
        return dir!.FullName;
    }

    /// <summary>
    /// 走**真实入口** `SaveCommand`（`RelayCommand(() =&gt; _ = SaveAsync(), ...)`，
    /// 即 Ctrl+S 与保存键打的那一条），不为测试新开保存钩子：
    /// 钩子会绕过 CanExecute 与真实调用形状，而那正是本条要守的东西。
    /// 命令是 fire-and-forget，所以轮询等 `IsDocumentSaving` 落回 false。
    /// </summary>
    private static async Task InvokeSaveAsync(WorksPageViewModel page)
    {
        Assert.True(page.SaveCommand.CanExecute(null), "保存键此刻不可用，测到的会是 CanExecute 而不是保存本身");
        page.SaveCommand.Execute(null);
        for (var i = 0; i < 100 && page.IsDocumentSaving; i++)
        {
            await Task.Delay(10);
        }
        Assert.False(page.IsDocumentSaving, "保存没在预期时间内收尾");
    }

    /// <summary>
    /// 假后端：只回答文档保存，可切换成失败（U194-A 的真实失败路径）。
    /// </summary>
    private class WorksBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        /// <summary>切成 true 后 `SaveDocumentContentAsync` 抛——真实失败路径。</summary>
        public bool FailSave { get; set; }

        public static WorksBackend Create()
        {
            var client = Create<IAriadneBackendClient, WorksBackend>();
            var backend = (WorksBackend)(object)client;
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
            if (targetMethod.Name == nameof(IAriadneBackendClient.SaveDocumentContentAsync))
            {
                if (FailSave)
                {
                    // 后端预算门/沙箱/权限拒绝都长这样：一个带原因的异常。
                    return Task.FromException<DocumentWriteReport>(
                        new InvalidOperationException("document sandbox rejected the write"));
                }
                var documentId = args?[0] as string ?? "documents/chapter-1.md";
                return Task.FromResult(new DocumentWriteReport(
                    new DocumentMetadata(documentId, documentId, "markdown", "text/markdown", 12, "v2"),
                    null));
            }
            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var payload = targetMethod.ReturnType.GetGenericArguments()[0];
                var value = payload.IsValueType ? Activator.CreateInstance(payload) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(payload)
                    .Invoke(null, new[] { value });
            }
            return null;
        }
    }
}
