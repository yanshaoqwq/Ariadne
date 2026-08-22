using Ariadne.Desktop.Backend;

namespace Ariadne.Desktop.ViewModels;

internal sealed record WorkspaceRunSessionState(
    string WorkflowId,
    string RunId,
    string Status);

internal readonly record struct WorkspaceRunSessionFence(long IdentityGeneration);

/// <summary>
/// 工作区唯一运行会话：固定 workflow/run 身份、事件游标和轮询代次，避免页面切图后旧请求串线。
/// </summary>
internal sealed class WorkspaceRunSessionCoordinator : IDisposable
{
    private const int EventBatchSize = 100;
    private static readonly TimeSpan DefaultPollInterval = TimeSpan.FromMilliseconds(750);
    /// <summary>
    /// U194-C：离页后「只查终态」的监视间隔。比事件轮询松 4 倍——
    /// 作者在别的页面写作，完成通知晚几秒毫无影响，而 IPC 频次值得省。
    /// </summary>
    private static readonly TimeSpan DefaultBackgroundWatchInterval = TimeSpan.FromMilliseconds(3000);

    private readonly IAriadneBackendClient _backend;
    private readonly TimeSpan _pollInterval;
    private readonly TimeSpan _backgroundWatchInterval;
    private CancellationTokenSource? _pollingCts;
    private long _eventCursor;
    private long _generation;
    private long _identityGeneration;
    private WorkspaceRunSessionState _state = new(string.Empty, string.Empty, string.Empty);

    public WorkspaceRunSessionCoordinator(
        IAriadneBackendClient backend,
        TimeSpan? pollInterval = null,
        TimeSpan? backgroundWatchInterval = null)
    {
        _backend = backend;
        _pollInterval = pollInterval ?? DefaultPollInterval;
        _backgroundWatchInterval = backgroundWatchInterval ?? DefaultBackgroundWatchInterval;
    }

    public event Action<WorkspaceRunSessionState, WorkspaceRunSessionState>? StateChanged;

    public event Action<WorkflowEventsResult>? EventsReceived;

    public event Action<Exception>? PollingFailed;

    public string WorkflowId => _state.WorkflowId;

    public string RunId => _state.RunId;

    public string Status => _state.Status;

    public WorkspaceRunSessionFence CaptureFence() => new(_identityGeneration);

    public void ThrowIfStale(WorkspaceRunSessionFence fence)
    {
        if (fence.IdentityGeneration != _identityGeneration)
        {
            throw new OperationCanceledException(
                "workflow run session changed while request was in flight");
        }
    }

    public async Task<WorkflowRunStarted> StartAsync(
        string workflowId,
        string? startNodeId,
        IReadOnlyDictionary<string, object?>? variables = null,
        CancellationToken cancellationToken = default)
    {
        var identityGeneration = _identityGeneration;
        var started = await _backend
            .RunWorkflowAsync(workflowId, startNodeId, variables, cancellationToken)
            .ConfigureAwait(true);
        if (identityGeneration != _identityGeneration)
        {
            throw new OperationCanceledException(
                "workflow run session changed while start request was in flight");
        }
        Attach(workflowId, started.RunId, started.Status, resetCursor: true);
        return started;
    }

    public Task<WorkflowActionResult> PauseAsync(
        string? reason,
        CancellationToken cancellationToken = default) =>
        ControlAsync(
            (workflowId, runId) => _backend.PauseWorkflowAsync(
                workflowId,
                runId,
                reason,
                cancellationToken));

    public Task<WorkflowActionResult> StopAsync(
        string? reason,
        CancellationToken cancellationToken = default) =>
        ControlAsync(
            (workflowId, runId) => _backend.StopWorkflowAsync(
                workflowId,
                runId,
                reason,
                cancellationToken));

    public Task<WorkflowActionResult> ResumeAsync(
        CancellationToken cancellationToken = default) =>
        ControlAsync(
            (workflowId, runId) => _backend.ResumeWorkflowAsync(
                workflowId,
                runId,
                cancellationToken));

    /// <summary>
    /// U196-D：从失败的那个节点重跑。
    ///
    /// 刻意走 <c>ControlAsync</c> 与其他三个控制指令同一条路：它在回包后
    /// <c>Attach</c> 新状态，而 <c>Attach</c> 会**重新起轮询**——运行进终态时轮询
    /// 已经停了，不重启的话重跑确实在后端跑着，而画布上一动不动、
    /// 状态行永远停在「已失败」。那种缺陷与「重跑根本没发生」在屏幕上完全同形。
    /// </summary>
    public Task<WorkflowActionResult> RetryFailedNodeAsync(
        string nodeId,
        CancellationToken cancellationToken = default) =>
        ControlAsync(
            (workflowId, runId) => _backend.RetryFailedNodeAsync(
                workflowId,
                runId,
                nodeId,
                cancellationToken));

    public void Attach(
        string workflowId,
        string runId,
        string? status,
        bool resetCursor = false,
        bool startPolling = true)
    {
        if (string.IsNullOrWhiteSpace(workflowId))
        {
            throw new ArgumentException("workflowId cannot be empty", nameof(workflowId));
        }
        if (string.IsNullOrWhiteSpace(runId))
        {
            throw new ArgumentException("runId cannot be empty", nameof(runId));
        }

        var identityChanged = !string.Equals(workflowId, WorkflowId, StringComparison.Ordinal)
            || !string.Equals(runId, RunId, StringComparison.Ordinal);
        CancelPolling();
        if (identityChanged || resetCursor)
        {
            _eventCursor = 0;
        }
        if (identityChanged)
        {
            _identityGeneration++;
        }
        var resolvedStatus = string.IsNullOrWhiteSpace(status)
            ? identityChanged ? "running" : Status
            : status;
        UpdateState(new WorkspaceRunSessionState(
            workflowId,
            runId,
            resolvedStatus));
        if (startPolling)
        {
            StartPolling();
        }
    }

    public void Reset()
    {
        CancelPolling();
        _eventCursor = 0;
        _identityGeneration++;
        UpdateState(new WorkspaceRunSessionState(string.Empty, string.Empty, string.Empty));
    }

    public void CancelPolling()
    {
        _generation++;
        _pollingCts?.Cancel();
        _pollingCts?.Dispose();
        _pollingCts = null;
    }

    /// <summary>
    /// U194-C：**离开画布页时把事件轮询降级成「只查终态」的后台监视，而不是掐掉。**
    ///
    /// 原缺陷：`MainWindowViewModel` 切页时对离开的页调 `DeactivateProjectData()`，
    /// 画布页的实现就是 `CancelPolling()` ⇒ 一离开画布页轮询就停。
    /// 于是作者启动一个几十分钟的工作流、切到作品页写作，跑完了 / 失败了 / 停在待确认，
    /// **界面上永远不会有任何变化**，只能定期切回画布页看。
    ///
    /// 这条同时让 U194-E 那套「终态刷预算 + 刷角标」真正生效：
    /// 它挂在 <see cref="UpdateState"/> 的终态跃迁上，而状态跃迁只由轮询推进
    /// ⇒ 轮询停了，那套刷新只在「作者一直盯着画布页」时有效，
    /// 而 C 条描述的场景恰恰是他切走了。属「做一半的功能掩盖没做的一半」。
    ///
    /// ## 为什么不是继续跑原来那个轮询
    ///
    /// 原轮询每 750ms 拉最多 100 条事件（`GetWorkflowEventsAsync`），
    /// 是为了让画布上的节点状态、日志流实时更新——那些在别的页面上**没有任何消费者**。
    /// 后台只需要「跑完了吗」这一个比特，所以改调 `GetWorkflowRunStateAsync`
    /// 并把间隔放宽到 <see cref="BackgroundWatchInterval"/>：
    /// 作者在写作，一次完成通知晚几秒毫无影响，而 IPC 频次降到 1/4。
    ///
    /// ## 为什么不动 _eventCursor
    ///
    /// 后台监视**刻意不推进事件游标**：作者切回画布页时会重新 `Attach` 起完整轮询，
    /// 那一轮要从原游标继续拉，日志才不缺段。若这里顺手推进了游标，
    /// 离页期间产生的事件就永远拉不回来了——日志中间会出现一个静默的洞。
    /// </summary>
    public void WatchTerminalStateInBackground()
    {
        // 没在跑、或已经跑到终态：没有什么可等的。
        // 这道闸是必需的——否则每次切页都会留下一个永不满足的轮询任务，
        // 切十次页面就有十个后台循环在打 IPC。
        if (string.IsNullOrWhiteSpace(WorkflowId)
            || string.IsNullOrWhiteSpace(RunId)
            || IsTerminal(Status))
        {
            CancelPolling();
            return;
        }

        // 先掐掉重的那个（CancelPolling 会 ++_generation，让旧循环下一轮自行退出），
        // 再用新代次起轻的。两者共用 _pollingCts 槽位与 _generation 判据，
        // 所以「切回画布页 → Attach → StartPolling」会同样把后台监视顶掉，
        // 不会出现两个循环并行打同一个 run。
        CancelPolling();
        var workflowId = WorkflowId;
        var runId = RunId;
        var generation = _generation;
        var cts = new CancellationTokenSource();
        _pollingCts = cts;
        _ = WatchTerminalAsync(workflowId, runId, generation, cts);
    }

    private async Task WatchTerminalAsync(
        string workflowId,
        string runId,
        long generation,
        CancellationTokenSource cts)
    {
        var cancellationToken = cts.Token;
        try
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                try
                {
                    await Task.Delay(_backgroundWatchInterval, cancellationToken).ConfigureAwait(true);
                }
                catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
                {
                    return;
                }

                WorkflowRunState state;
                try
                {
                    state = await _backend
                        .GetWorkflowRunStateAsync(workflowId, runId, cancellationToken)
                        .ConfigureAwait(true);
                }
                catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
                {
                    return;
                }
                catch
                {
                    // 后台监视失败**刻意不报给 PollingFailed**：那个事件写的是画布页的
                    // 状态行，而作者此刻在别的页面上——弹一条他看不见、也无从处置的
                    // 「轮询失败」只会在他切回来时留下一句陈旧的错误。
                    // 直接退出：切回画布页会重起完整轮询，那时的失败才有显示位。
                    return;
                }

                if (!IsCurrent(generation, workflowId, runId))
                {
                    return;
                }
                // 走 UpdateState 这个统一收口，终态广播（U194-E 的预算/角标刷新
                // 与 C 条的顶栏通知）便自动生效，不必在这里重复一遍判定逻辑。
                UpdateState(_state with { Status = state.Status ?? Status });
                if (IsTerminal(state.Status))
                {
                    return;
                }
            }
        }
        finally
        {
            if (generation == _generation && ReferenceEquals(_pollingCts, cts))
            {
                _pollingCts = null;
                cts.Dispose();
            }
        }
    }

    public static bool IsTerminal(string? status) =>
        status is "stopped" or "succeeded" or "failed";

    public void Dispose()
    {
        CancelPolling();
        GC.SuppressFinalize(this);
    }

    private async Task<WorkflowActionResult> ControlAsync(
        Func<string, string, Task<WorkflowActionResult>> action)
    {
        if (string.IsNullOrWhiteSpace(WorkflowId) || string.IsNullOrWhiteSpace(RunId))
        {
            throw new InvalidOperationException("workflow run session is not attached");
        }

        var workflowId = WorkflowId;
        var runId = RunId;
        var result = await action(workflowId, runId).ConfigureAwait(true);
        if (!string.Equals(workflowId, WorkflowId, StringComparison.Ordinal)
            || !string.Equals(runId, RunId, StringComparison.Ordinal))
        {
            throw new OperationCanceledException(
                "workflow run session changed while control request was in flight");
        }
        EnsureResultIdentity(result.WorkflowId, result.RunId, workflowId, runId);
        Attach(result.WorkflowId, result.RunId, result.Status);
        return result;
    }

    private void StartPolling()
    {
        if (string.IsNullOrWhiteSpace(WorkflowId) || string.IsNullOrWhiteSpace(RunId))
        {
            return;
        }

        var workflowId = WorkflowId;
        var runId = RunId;
        var generation = ++_generation;
        var cts = new CancellationTokenSource();
        _pollingCts = cts;
        _ = PollAsync(workflowId, runId, generation, cts);
    }

    private async Task PollAsync(
        string workflowId,
        string runId,
        long generation,
        CancellationTokenSource cts)
    {
        var cancellationToken = cts.Token;
        try
        {
            while (!cancellationToken.IsCancellationRequested)
            {
                WorkflowEventsResult result;
                try
                {
                    result = await _backend
                        .GetWorkflowEventsAsync(
                            workflowId,
                            runId,
                            _eventCursor,
                            EventBatchSize,
                            cancellationToken)
                        .ConfigureAwait(true);
                }
                catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
                {
                    return;
                }
                catch (Exception ex)
                {
                    if (IsCurrent(generation, workflowId, runId))
                    {
                        PollingFailed?.Invoke(ex);
                    }
                    return;
                }

                if (!IsCurrent(generation, workflowId, runId))
                {
                    return;
                }
                try
                {
                    EnsureResultIdentity(result.WorkflowId, result.RunId, workflowId, runId);
                }
                catch (Exception ex)
                {
                    PollingFailed?.Invoke(ex);
                    return;
                }

                _eventCursor = result.NextSequence;
                UpdateState(_state with { Status = result.Status ?? Status });
                EventsReceived?.Invoke(result);
                if (IsTerminal(result.Status))
                {
                    return;
                }

                try
                {
                    await Task.Delay(_pollInterval, cancellationToken).ConfigureAwait(true);
                }
                catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
                {
                    return;
                }
            }
        }
        finally
        {
            if (generation == _generation && ReferenceEquals(_pollingCts, cts))
            {
                _pollingCts = null;
                cts.Dispose();
            }
        }
    }

    private bool IsCurrent(long generation, string workflowId, string runId) =>
        generation == _generation
        && string.Equals(workflowId, WorkflowId, StringComparison.Ordinal)
        && string.Equals(runId, RunId, StringComparison.Ordinal);

    private static void EnsureResultIdentity(
        string resultWorkflowId,
        string resultRunId,
        string expectedWorkflowId,
        string expectedRunId)
    {
        if (!string.Equals(resultWorkflowId, expectedWorkflowId, StringComparison.Ordinal)
            || !string.Equals(resultRunId, expectedRunId, StringComparison.Ordinal))
        {
            throw new InvalidOperationException(
                "workflow backend returned a result for a different run session");
        }
    }

    private void UpdateState(WorkspaceRunSessionState next)
    {
        if (_state == next)
        {
            return;
        }

        var previous = _state;
        _state = next;
        StateChanged?.Invoke(previous, next);
        // U194-E：终态广播接在这里，因为这是**所有**状态跃迁的唯一收口——
        // 轮询回包、Pause/Stop/Resume 的控制结果、确认项解析后的 Attach 都经过它。
        // 接在轮询循环里会漏掉「点停止后立刻拿到 stopped」那条（控制结果直接 Attach，
        // 轮询已被 CancelPolling 掐掉，不会再有下一轮回包）。
        //
        // ⚠️ **必须是跃迁边沿**：`_state == next` 那个短路保证了同一终态不会连发
        //（轮询在终态那一轮 return 之前也只会走到这里一次），
        // 所以下游拿到的是「每次运行结束一次」，而不是每 750ms 一次。
        if (IsTerminal(next.Status) && !IsTerminal(previous.Status))
        {
            RunTerminalStateNotifier.NotifyTerminal(next.WorkflowId, next.RunId, next.Status);
        }
    }
}
