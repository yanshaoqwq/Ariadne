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
        // U137：不再显示 workflow_id——它恒为 "default"，拿它筛选没有区分度。
        Assert.DoesNotContain("workflow-a", item.ContextText, StringComparison.Ordinal);
        Assert.Contains("run-a", item.ContextText, StringComparison.Ordinal);
        Assert.Contains("writer", item.ContextText, StringComparison.Ordinal);
        Assert.DoesNotContain("[ui.run_log.kind", item.KindText, StringComparison.Ordinal);
        Assert.Equal("错误", item.LevelText);
    }

    /// <summary>
    /// U158：运行终态日志的 message 存的是**稳定 key**，前端要渲染成本地化文案。
    ///
    /// 后端刻意存 `workflow.run.succeeded` 而非中文句子——core 不该硬编码展示文案。
    /// 若前端不翻译，用户在日志页看到的就是一行英文点分标识符。
    /// </summary>
    [Theory]
    [InlineData("workflow.run.succeeded")]
    [InlineData("workflow.run.stopped")]
    [InlineData("workflow.run.paused")]
    public void Item_LocalizesRunOutcomeMessageKeys(string messageKey)
    {
        var item = new RunLogItemViewModel(
            new UiRunLogEntry("log-x", 1, "node", "info", messageKey, "default", "run-x", null, true),
            DisplayNameService.LoadDefault());

        Assert.NotEqual(messageKey, item.Message);
        // 缺 key 时 DisplayNameService.Text 返回 `[key]`；那种情况下我们回落到原始 key，
        // 所以这两条断言合起来才能证明「key 真的存在且被翻译了」。
        Assert.DoesNotContain('[', item.Message);
        Assert.False(string.IsNullOrWhiteSpace(item.Message));
    }

    /// <summary>
    /// ⚠️ **白名单之外的 message 必须原样透出**（U158 的施工陷阱）。
    ///
    /// 日志里绝大多数条目（工作流 worker 错误、Git 恢复诊断）存的是**真实文本**而非 key。
    /// 一律走 `DisplayNameService.Text` 会把它们变成 `[原文]`——
    /// 那是把可读的错误信息变成噪音，比不翻译严重得多。
    ///
    /// 这条用例的存在本身就是判据：若有人「顺手」把翻译改成无条件查表，它立刻红。
    /// </summary>
    [Theory]
    [InlineData("workflow_worker_failed: connection refused")]
    [InlineData("git restore skipped: dirty work tree")]
    [InlineData("节点 writer 执行失败")]
    public void Item_LeavesPlainMessagesUntouched(string message)
    {
        var item = new RunLogItemViewModel(
            new UiRunLogEntry("log-y", 1, "error", "error", message, "default", "run-y", null, true),
            DisplayNameService.LoadDefault());

        Assert.Equal(message, item.Message);
    }

    [Fact]
    public async Task ClearFilters_ResetsEveryTextFilterAndRefreshesOnce()
    {
        var backend = RunLogBackend.Create();
        var viewModel = new RunLogPageViewModel(DisplayNameService.LoadDefault(), backend.Client)
        {
            SearchQuery = "failure",
            RunIdFilter = "run",
            NodeIdFilter = "node",
        };

        Assert.True(viewModel.ClearFiltersCommand.TryExecute());
        await WaitUntilAsync(() => backend.Queries.Count == 1);

        var query = Assert.Single(backend.Queries);
        Assert.Null(query.Query);
        // U137：workflow_id 恒传 null——桌面端固定单画布，按它筛选没有区分度。
        Assert.Null(query.WorkflowId);
        Assert.Null(query.RunId);
        Assert.Null(query.NodeId);
        Assert.False(viewModel.HasActiveFilter);
    }

    [Fact]
    public async Task SelectedLog_HasAVisibleCopyActionInsteadOfDeadSelection()
    {
        var viewModel = new RunLogPageViewModel(DisplayNameService.LoadDefault(), RunLogBackend.Create().Client);
        var item = new RunLogItemViewModel(Entry("log-1", 1, unread: false), DisplayNameService.LoadDefault());
        string? copied = null;
        viewModel.RequestCopyText = text =>
        {
            copied = text;
            return Task.CompletedTask;
        };
        viewModel.SelectedLog = item;

        Assert.True(viewModel.CopySelectedCommand.TryExecute());
        await WaitUntilAsync(() => copied is not null);

        Assert.Contains("log-1", copied, StringComparison.Ordinal);
        Assert.Contains(item.LevelText, copied, StringComparison.Ordinal);
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
            RunIdFilter = "run-a",
            NodeIdFilter = "writer",
        };
        await viewModel.ReloadProjectDataAsync();

        Assert.True(viewModel.MarkReadCommand.TryExecute());
        await WaitUntilAsync(() => backend.MarkReadFilters.Count == 1 && !viewModel.IsMarkingRead);

        var filter = backend.MarkReadFilters[0];
        Assert.Equal("failed", filter.Query);
        // U137：标记已读与查询共用 BuildQuery，workflow_id 一并恒为 null。
        Assert.Null(filter.WorkflowId);
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
