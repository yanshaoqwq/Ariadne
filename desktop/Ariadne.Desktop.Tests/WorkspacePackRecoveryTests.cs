using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class WorkspacePackRecoveryTests
{
    [Fact]
    public async Task LostResponse_QueriesReceiptWithoutPackingTwice()
    {
        var backend = PackBackend.Create();
        backend.FirstPackThrows = true;
        backend.RecoveryReport = Report("op-lost-response", "rev-result");

        var result = await WorkspacePageViewModel.PackSelectionWithRecoveryAsync(
            "workflow",
            new[] { "writer" },
            "sub-writer",
            "Writer Subflow",
            "rev-base",
            "op-lost-response",
            (IAriadneBackendClient)(object)backend);

        Assert.Equal("rev-result", result.Workflow.ContentRevision);
        Assert.Equal(1, backend.PackCalls);
        Assert.Equal(1, backend.GetOperationCalls);
        Assert.All(backend.OperationIds, id => Assert.Equal("op-lost-response", id));
    }

    [Fact]
    public async Task RequestNotDelivered_RetriesWithSameOperationIdAndRevision()
    {
        var backend = PackBackend.Create();
        backend.FirstPackThrows = true;
        backend.GetOperationThrows = true;
        backend.RetryReport = Report("op-not-delivered", "rev-result");

        var result = await WorkspacePageViewModel.PackSelectionWithRecoveryAsync(
            "workflow",
            new[] { "writer" },
            "sub-writer",
            "Writer Subflow",
            "rev-base",
            "op-not-delivered",
            (IAriadneBackendClient)(object)backend);

        Assert.Equal("rev-result", result.Workflow.ContentRevision);
        Assert.Equal(2, backend.PackCalls);
        Assert.Equal(1, backend.GetOperationCalls);
        Assert.All(backend.OperationIds, id => Assert.Equal("op-not-delivered", id));
        Assert.All(backend.ExpectedRevisions, revision => Assert.Equal("rev-base", revision));
    }

    [Fact]
    public async Task UserCancellation_DoesNotQueryOrRetryOperation()
    {
        var backend = PackBackend.Create();
        backend.FirstPackCancels = true;
        using var cancellation = new CancellationTokenSource();
        cancellation.Cancel();

        await Assert.ThrowsAnyAsync<OperationCanceledException>(() =>
            WorkspacePageViewModel.PackSelectionWithRecoveryAsync(
                "workflow",
                new[] { "writer" },
                "sub-writer",
                "Writer Subflow",
                "rev-base",
                "op-cancelled",
                (IAriadneBackendClient)(object)backend,
                cancellation.Token));

        Assert.Equal(1, backend.PackCalls);
        Assert.Equal(0, backend.GetOperationCalls);
    }

    private static WorkflowPackReport Report(string operationId, string revision)
    {
        var graph = new WorkflowGraphData(
            "workflow",
            "Workflow",
            Array.Empty<CanvasNode>(),
            Array.Empty<CanvasEdge>(),
            new Dictionary<string, object?>(),
            revision);
        return new WorkflowPackReport(
            graph,
            "sub-writer",
            graph,
            Array.Empty<WorkflowPortEndpoint>(),
            Array.Empty<WorkflowPortEndpoint>(),
            operationId);
    }

    private class PackBackend : DispatchProxy
    {
        public bool FirstPackThrows { get; set; }
        public bool FirstPackCancels { get; set; }
        public bool GetOperationThrows { get; set; }
        public WorkflowPackReport? RecoveryReport { get; set; }
        public WorkflowPackReport? RetryReport { get; set; }
        public int PackCalls { get; private set; }
        public int GetOperationCalls { get; private set; }
        public List<string?> OperationIds { get; } = new();
        public List<string?> ExpectedRevisions { get; } = new();

        public static PackBackend Create()
        {
            return (PackBackend)Create<IAriadneBackendClient, PackBackend>();
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == nameof(IAriadneBackendClient.PackWorkflowSelectionAsync))
            {
                PackCalls++;
                ExpectedRevisions.Add(args?[4] as string);
                OperationIds.Add(args?[5] as string);
                if (PackCalls == 1 && FirstPackCancels)
                {
                    return Task.FromCanceled<WorkflowPackReport>((CancellationToken)args![6]!);
                }
                if (PackCalls == 1 && FirstPackThrows)
                {
                    return Task.FromException<WorkflowPackReport>(new IOException("response lost"));
                }
                return Task.FromResult(RetryReport ?? throw new InvalidOperationException("retry report missing"));
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetPackOperationAsync))
            {
                GetOperationCalls++;
                OperationIds.Add(args?[0] as string);
                if (GetOperationThrows)
                {
                    return Task.FromException<WorkflowPackReport>(new IOException("receipt not found"));
                }
                return Task.FromResult(RecoveryReport ?? throw new InvalidOperationException("recovery report missing"));
            }
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }
            throw new NotSupportedException(targetMethod?.Name);
        }
    }
}
