using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class RunLogStateTests
{
    [Fact]
    public void Item_PreservesSourceAndUnreadState()
    {
        var item = new RunLogItemViewModel(
            new UiRunLogEntry(
                "log-1",
                1,
                "node",
                "error",
                "failed",
                "workflow-a",
                "run-a",
                "writer",
                true),
            DisplayNameService.LoadDefault());

        Assert.True(item.IsUnread);
        Assert.True(item.HasContext);
        Assert.Contains("workflow-a", item.ContextText, StringComparison.Ordinal);
        Assert.Contains("run-a", item.ContextText, StringComparison.Ordinal);
        Assert.Contains("writer", item.ContextText, StringComparison.Ordinal);
        Assert.DoesNotContain("[ui.run_log.kind", item.KindText, StringComparison.Ordinal);
    }

    [Fact]
    public async Task Paging_RequestsNewestFirstAndUsesLastVisibleEntryAsCursor()
    {
        var backend = RunLogBackend.Create();
        backend.QueryHandler = query =>
        {
            if (query.AfterLogId is null)
            {
                return Enumerable.Range(0, 101)
                    .Select(index => Entry($"log-{200 - index}", 200 - index, unread: true))
                    .ToArray();
            }

            return new[]
            {
                Entry("log-100", 100, unread: false),
                Entry("log-99", 99, unread: false),
            };
        };
        var viewModel = new RunLogPageViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await viewModel.ReloadProjectDataAsync();

        Assert.Equal(100, viewModel.Logs.Count);
        Assert.True(viewModel.HasMore);
        Assert.True(backend.Queries[0].Descending);
        Assert.Equal(101, backend.Queries[0].Limit);

        Assert.True(viewModel.LoadMoreCommand.TryExecute());
        await WaitUntilAsync(() => backend.Queries.Count == 2 && !viewModel.IsLoadingMore);

        Assert.Equal("log-101", backend.Queries[1].AfterLogId);
        Assert.Equal(101, backend.Queries[1].AfterTimestampMs);
        Assert.Equal(102, viewModel.Logs.Count);
        Assert.False(viewModel.HasMore);
    }

    [Fact]
    public async Task RefreshFailure_PreservesLastGoodPageAndShowsContentError()
    {
        var backend = RunLogBackend.Create();
        backend.QueryHandler = _ => new[] { Entry("log-1", 1, unread: true) };
        var viewModel = new RunLogPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();

        backend.QueryError = new InvalidOperationException("offline");
        await viewModel.ReloadProjectDataAsync();

        Assert.Single(viewModel.Logs);
        Assert.True(viewModel.ShowContent);
        Assert.True(viewModel.IsContentError);
        Assert.False(viewModel.IsStandaloneError);
    }

    [Fact]
    public async Task MarkRead_UsesCurrentFilterScopeInsteadOfClearingAllLogs()
    {
        var backend = RunLogBackend.Create();
        backend.QueryHandler = _ => new[]
        {
            new UiRunLogEntry(
                "log-1",
                1,
                "node",
                "error",
                "failed",
                "workflow-a",
                "run-a",
                "writer",
                true),
        };
        backend.MarkReadHandler = _ => 1;
        var viewModel = new RunLogPageViewModel(DisplayNameService.LoadDefault(), backend.Client)
        {
            SearchQuery = "failed",
            WorkflowIdFilter = "workflow-a",
            RunIdFilter = "run-a",
            NodeIdFilter = "writer",
        };
        await viewModel.ReloadProjectDataAsync();

        Assert.True(viewModel.MarkReadCommand.TryExecute());
        await WaitUntilAsync(() => backend.MarkReadFilters.Count == 1 && !viewModel.IsMarkingRead);

        var filter = backend.MarkReadFilters[0];
        Assert.Equal("failed", filter.Query);
        Assert.Equal("workflow-a", filter.WorkflowId);
        Assert.Equal("run-a", filter.RunId);
        Assert.Equal("writer", filter.NodeId);
        Assert.Null(filter.Limit);
        Assert.Contains("1", viewModel.StatusText, StringComparison.Ordinal);
    }

    private static UiRunLogEntry Entry(string id, long timestamp, bool unread)
    {
        return new UiRunLogEntry(id, timestamp, "node", "info", id, "wf", "run", "node", unread);
    }

    private static async Task WaitUntilAsync(Func<bool> predicate)
    {
        for (var attempt = 0; attempt < 100 && !predicate(); attempt++)
        {
            await Task.Delay(10);
        }
        Assert.True(predicate());
    }

    private class RunLogBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public Func<RunLogQuery, IReadOnlyList<UiRunLogEntry>> QueryHandler { get; set; } =
            _ => Array.Empty<UiRunLogEntry>();
        public Func<RunLogQuery, int> MarkReadHandler { get; set; } = _ => 0;
        public Exception? QueryError { get; set; }
        public List<RunLogQuery> Queries { get; } = new();
        public List<RunLogQuery> MarkReadFilters { get; } = new();

        public static RunLogBackend Create()
        {
            var client = Create<IAriadneBackendClient, RunLogBackend>();
            var backend = (RunLogBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.QueryRunLogsAsync))
            {
                var query = (RunLogQuery)args![0]!;
                Queries.Add(query);
                if (QueryError is not null)
                {
                    return Task.FromException<IReadOnlyList<UiRunLogEntry>>(QueryError);
                }
                return Task.FromResult(QueryHandler(query));
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.MarkRunLogsReadAsync))
            {
                var filter = (RunLogQuery)args![0]!;
                MarkReadFilters.Add(filter);
                return Task.FromResult(MarkReadHandler(filter));
            }

            throw new NotSupportedException(targetMethod?.Name);
        }
    }
}
