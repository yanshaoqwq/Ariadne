using System.Collections.Concurrent;
using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class WorkspaceRunSessionCoordinatorTests
{
    [Fact]
    public async Task SwitchingWorkflow_FencesLateEventsFromPreviousRun()
    {
        var backend = RunBackend.Create();
        var first = backend.EnqueueEvents();
        var second = backend.EnqueueEvents();
        using var session = new WorkspaceRunSessionCoordinator(
            backend.Client,
            TimeSpan.FromMilliseconds(5));
        var received = new List<WorkflowEventsResult>();
        session.EventsReceived += received.Add;

        session.Attach("workflow-a", "run-a", "running", resetCursor: true);
        await WaitUntilAsync(() => backend.EventRequests.Count == 1);
        session.Attach("workflow-b", "run-b", "running", resetCursor: true);
        await WaitUntilAsync(() => backend.EventRequests.Count == 2);

        second.SetResult(Events("workflow-b", "run-b", "succeeded", 4, "node_succeeded"));
        await WaitUntilAsync(() => received.Count == 1);
        first.SetResult(Events("workflow-a", "run-a", "failed", 99, "node_failed"));
        await Task.Delay(25);

        Assert.Single(received);
        Assert.Equal("workflow-b", received[0].WorkflowId);
        Assert.Equal("run-b", session.RunId);
        Assert.Equal("succeeded", session.Status);
        Assert.DoesNotContain(received, item => item.WorkflowId == "workflow-a");
    }

    [Fact]
    public async Task SameRunControl_PreservesCursorAndUsesAttachedIdentity()
    {
        var backend = RunBackend.Create();
        var first = backend.EnqueueEvents();
        var stale = backend.EnqueueEvents();
        var afterPause = backend.EnqueueEvents();
        using var session = new WorkspaceRunSessionCoordinator(
            backend.Client,
            TimeSpan.FromMilliseconds(5));

        session.Attach("workflow-a", "run-a", "running", resetCursor: true);
        await WaitUntilAsync(() => backend.EventRequests.Count == 1);
        first.SetResult(Events("workflow-a", "run-a", "running", 7, "node_started"));
        await WaitUntilAsync(() => backend.EventRequests.Count == 2);

        var result = await session.PauseAsync("author pause");
        await WaitUntilAsync(() => backend.EventRequests.Count == 3);

        Assert.Equal("paused", result.Status);
        Assert.Equal(("workflow-a", "run-a", "author pause"), backend.PauseRequests.Single());
        Assert.Equal(7, backend.EventRequests.ToArray()[2].AfterSequence);

        afterPause.SetResult(Events("workflow-a", "run-a", "stopped", 8, "run_stopped"));
        await WaitUntilAsync(() => session.Status == "stopped");
        stale.SetResult(Events("workflow-a", "run-a", "failed", 100, "node_failed"));
        await Task.Delay(25);

        Assert.Equal("stopped", session.Status);
    }

    [Fact]
    public async Task LateControlResponse_CannotReattachPreviousWorkflow()
    {
        var backend = RunBackend.Create();
        var firstEvents = backend.EnqueueEvents();
        var secondEvents = backend.EnqueueEvents();
        var pauseResponse = backend.EnqueuePause();
        using var session = new WorkspaceRunSessionCoordinator(
            backend.Client,
            TimeSpan.FromMilliseconds(5));

        session.Attach("workflow-a", "run-a", "running", resetCursor: true);
        await WaitUntilAsync(() => backend.EventRequests.Count == 1);
        var pauseTask = session.PauseAsync("late pause");
        await WaitUntilAsync(() => backend.PauseRequests.Count == 1);

        session.Attach("workflow-b", "run-b", "running", resetCursor: true);
        await WaitUntilAsync(() => backend.EventRequests.Count == 2);
        pauseResponse.SetResult(new WorkflowActionResult("workflow-a", "run-a", "paused"));

        await Assert.ThrowsAsync<OperationCanceledException>(() => pauseTask);
        Assert.Equal("workflow-b", session.WorkflowId);
        Assert.Equal("run-b", session.RunId);
        Assert.Equal("running", session.Status);

        secondEvents.SetResult(Events("workflow-b", "run-b", "succeeded", 2, "run_succeeded"));
        firstEvents.SetResult(Events("workflow-a", "run-a", "failed", 2, "node_failed"));
    }

    [Fact]
    public void SessionFence_RejectsResultAfterWorkflowReset()
    {
        var backend = RunBackend.Create();
        using var session = new WorkspaceRunSessionCoordinator(backend.Client);
        session.Attach("workflow-a", "run-a", "running", startPolling: false);
        var fence = session.CaptureFence();

        session.Reset();

        Assert.Throws<OperationCanceledException>(() => session.ThrowIfStale(fence));
    }

    [Fact]
    public async Task MismatchedBackendIdentity_IsRejectedBeforeProjection()
    {
        var backend = RunBackend.Create();
        var response = backend.EnqueueEvents();
        using var session = new WorkspaceRunSessionCoordinator(
            backend.Client,
            TimeSpan.FromMilliseconds(5));
        var received = new List<WorkflowEventsResult>();
        Exception? pollingError = null;
        session.EventsReceived += received.Add;
        session.PollingFailed += error => pollingError = error;

        session.Attach("workflow-a", "run-a", "running", resetCursor: true);
        await WaitUntilAsync(() => backend.EventRequests.Count == 1);
        response.SetResult(Events("workflow-b", "run-b", "failed", 5, "node_failed"));
        await WaitUntilAsync(() => pollingError is not null);

        Assert.Empty(received);
        Assert.IsType<InvalidOperationException>(pollingError);
        Assert.Equal("run-a", session.RunId);
        Assert.Equal("running", session.Status);
    }

    private static WorkflowEventsResult Events(
        string workflowId,
        string runId,
        string status,
        long nextSequence,
        string eventType) =>
        new(
            workflowId,
            runId,
            status,
            nextSequence,
            new[]
            {
                new WorkflowRuntimeEvent(
                    nextSequence - 1,
                    eventType,
                    null,
                    string.Empty,
                    null),
            });

    private static async Task WaitUntilAsync(Func<bool> condition)
    {
        using var timeout = new CancellationTokenSource(TimeSpan.FromSeconds(2));
        while (!condition())
        {
            await Task.Delay(5, timeout.Token);
        }
    }

    private class RunBackend : DispatchProxy
    {
        private readonly ConcurrentQueue<TaskCompletionSource<WorkflowEventsResult>> _eventResponses = new();
        private readonly ConcurrentQueue<TaskCompletionSource<WorkflowActionResult>> _pauseResponses = new();

        public IAriadneBackendClient Client { get; private set; } = null!;

        public ConcurrentQueue<EventRequest> EventRequests { get; } = new();

        public ConcurrentQueue<(string WorkflowId, string RunId, string? Reason)> PauseRequests { get; } = new();

        public static RunBackend Create()
        {
            var client = Create<IAriadneBackendClient, RunBackend>();
            var backend = (RunBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        public TaskCompletionSource<WorkflowEventsResult> EnqueueEvents()
        {
            var response = new TaskCompletionSource<WorkflowEventsResult>(
                TaskCreationOptions.RunContinuationsAsynchronously);
            _eventResponses.Enqueue(response);
            return response;
        }

        public TaskCompletionSource<WorkflowActionResult> EnqueuePause()
        {
            var response = new TaskCompletionSource<WorkflowActionResult>(
                TaskCreationOptions.RunContinuationsAsynchronously);
            _pauseResponses.Enqueue(response);
            return response;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null || args is null)
            {
                throw new InvalidOperationException("missing backend invocation");
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.GetWorkflowEventsAsync))
            {
                EventRequests.Enqueue(new EventRequest(
                    (string)args[0]!,
                    (string)args[1]!,
                    (long)args[2]!,
                    (int?)args[3]));
                if (!_eventResponses.TryDequeue(out var response))
                {
                    throw new InvalidOperationException("no queued workflow event response");
                }
                // 故意忽略 cancellation：用于证明旧后端请求即使迟到也不能污染新会话。
                return response.Task;
            }

            if (targetMethod.Name == nameof(IAriadneBackendClient.PauseWorkflowAsync))
            {
                var workflowId = (string)args[0]!;
                var runId = (string)args[1]!;
                var reason = (string?)args[2];
                PauseRequests.Enqueue((workflowId, runId, reason));
                if (_pauseResponses.TryDequeue(out var response))
                {
                    return response.Task;
                }
                return Task.FromResult(new WorkflowActionResult(workflowId, runId, "paused"));
            }

            throw new NotSupportedException(targetMethod.Name);
        }
    }

    private sealed record EventRequest(
        string WorkflowId,
        string RunId,
        long AfterSequence,
        int? Limit);
}
