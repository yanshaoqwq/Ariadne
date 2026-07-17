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

    private readonly IAriadneBackendClient _backend;
    private readonly TimeSpan _pollInterval;
    private CancellationTokenSource? _pollingCts;
    private long _eventCursor;
    private long _generation;
    private long _identityGeneration;
    private WorkspaceRunSessionState _state = new(string.Empty, string.Empty, string.Empty);

    public WorkspaceRunSessionCoordinator(
        IAriadneBackendClient backend,
        TimeSpan? pollInterval = null)
    {
        _backend = backend;
        _pollInterval = pollInterval ?? DefaultPollInterval;
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
        CancellationToken cancellationToken = default)
    {
        var identityGeneration = _identityGeneration;
        var started = await _backend
            .RunWorkflowAsync(workflowId, startNodeId, cancellationToken)
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
    }
}
