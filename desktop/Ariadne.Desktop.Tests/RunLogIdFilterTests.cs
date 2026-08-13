using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U137 回归：运行记录页原先要求用户**肉眼抄 UUID 再手敲**才能筛选。
///
/// 三个自由文本框对应后端的 `run_id = ?` / `node_id = ?` **精确等值**匹配
/// （同一条 SQL 里 message 用的是 LIKE），差一个字符就是空结果、且除了
/// "没有结果"以外不给任何提示。而每条日志明细**已经渲染了这些 ID**，
/// 却是纯 TextBlock、不可点击。
///
/// 判据落在**真实发出的请求**上：「点了明细里的 run_id，下一次查询是否真的
/// 带上了它」。只断言「chip 列表非空」是不够的——缺陷版本里 ID 也是显示着的，
/// 只是点不动。
/// </summary>
public sealed class RunLogIdFilterTests
{
    [Fact]
    public async Task ClickingRunIdChipFiltersByThatRunId()
    {
        var backend = RunLogFilterBackend.Create();
        backend.QueryHandler = _ => new[] { Entry("log-1", "run-7", "writer") };
        var viewModel = new RunLogPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();
        backend.Queries.Clear();

        var item = Assert.Single(viewModel.Logs);
        var runChip = Assert.Single(item.ContextChips, chip => chip.Field == RunLogFilterField.RunId);
        Assert.True(runChip.Command.TryExecute());
        await WaitUntilAsync(() => backend.Queries.Count >= 1);

        // 缺陷版本的 ID 是纯 TextBlock：压根没有可执行的命令，
        // 用户只能肉眼记住再手敲。
        Assert.Equal("run-7", backend.Queries[^1].RunId);
        Assert.Equal("run-7", viewModel.RunIdFilter);
    }

    [Fact]
    public async Task ClickingNodeIdChipFiltersByThatNodeId()
    {
        var backend = RunLogFilterBackend.Create();
        backend.QueryHandler = _ => new[] { Entry("log-1", "run-7", "critic") };
        var viewModel = new RunLogPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();
        backend.Queries.Clear();

        var item = Assert.Single(viewModel.Logs);
        var nodeChip = Assert.Single(item.ContextChips, chip => chip.Field == RunLogFilterField.NodeId);
        Assert.True(nodeChip.Command.TryExecute());
        await WaitUntilAsync(() => backend.Queries.Count >= 1);

        Assert.Equal("critic", backend.Queries[^1].NodeId);
    }

    [Fact]
    public async Task RemovingActiveFilterChipDropsItFromTheNextQuery()
    {
        var backend = RunLogFilterBackend.Create();
        backend.QueryHandler = _ => new[] { Entry("log-1", "run-7", "writer") };
        var viewModel = new RunLogPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();

        var item = Assert.Single(viewModel.Logs);
        var runChip = Assert.Single(item.ContextChips, chip => chip.Field == RunLogFilterField.RunId);
        Assert.True(runChip.Command.TryExecute());
        await WaitUntilAsync(() => viewModel.HasActiveIdFilters);

        var activeChip = Assert.Single(viewModel.ActiveIdFilters);
        backend.Queries.Clear();
        Assert.True(activeChip.Command.TryExecute());
        await WaitUntilAsync(() => backend.Queries.Count >= 1);

        // chip 可移除是产品决策的一半——只能加不能减的筛选等于一次性陷阱：
        // 用户点错一个 ID 后就只能靠「清除全部筛选」把搜索词一起丢掉。
        Assert.Null(backend.Queries[^1].RunId);
        Assert.False(viewModel.HasActiveIdFilters);
    }

    [Fact]
    public async Task ClickingSameFieldTwiceReplacesInsteadOfAccumulating()
    {
        var backend = RunLogFilterBackend.Create();
        backend.QueryHandler = _ => new[]
        {
            Entry("log-1", "run-7", "writer"),
            Entry("log-2", "run-9", "critic"),
        };
        var viewModel = new RunLogPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();

        var first = viewModel.Logs[0].ContextChips.Single(chip => chip.Field == RunLogFilterField.RunId);
        Assert.True(first.Command.TryExecute());
        await WaitUntilAsync(() => viewModel.RunIdFilter == "run-7");

        var second = viewModel.Logs[1].ContextChips.Single(chip => chip.Field == RunLogFilterField.RunId);
        backend.Queries.Clear();
        Assert.True(second.Command.TryExecute());
        await WaitUntilAsync(() => backend.Queries.Count >= 1);

        // 后端是等值匹配，「两个 run_id 同时生效」的语义不存在——
        // 累加只会得到永远为空的结果集。
        Assert.Equal("run-9", backend.Queries[^1].RunId);
        Assert.Single(viewModel.ActiveIdFilters);
    }

    [Fact]
    public async Task IdChipsStayClickableOnEntriesLoadedByLoadMore()
    {
        var backend = RunLogFilterBackend.Create();
        var firstPage = Enumerable.Range(0, 101)
            .Select(index => Entry($"log-{index}", $"run-{index}", "writer"))
            .ToArray();
        backend.QueryHandler = query => query.AfterLogId is null
            ? firstPage
            : new[] { Entry("log-tail", "run-tail", "polisher") };
        var viewModel = new RunLogPageViewModel(DisplayNameService.LoadDefault(), backend.Client);
        await viewModel.ReloadProjectDataAsync();
        Assert.True(viewModel.HasMore);

        Assert.True(viewModel.LoadMoreCommand.TryExecute());
        await WaitUntilAsync(() => viewModel.Logs.Any(log => log.LogId == "log-tail"));

        var tail = viewModel.Logs.Single(log => log.LogId == "log-tail");
        var chip = Assert.Single(tail.ContextChips, item => item.Field == RunLogFilterField.RunId);
        backend.Queries.Clear();
        Assert.True(chip.Command.TryExecute());
        await WaitUntilAsync(() => backend.Queries.Count >= 1);

        // 条目在两处被创建（首屏 + 加载更多）。各自 new 的话漏接一处，
        // 就是「前 100 条的 ID 能点、往下翻的点不动」——只在特定路径失效，
        // 最难被发现，所以专测这条路径。
        Assert.Equal("run-tail", backend.Queries[^1].RunId);
    }

    private static UiRunLogEntry Entry(string logId, string runId, string nodeId) =>
        new(logId, 1, "node", "info", "消息", "default", runId, nodeId, false);

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

    private class RunLogFilterBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public Func<RunLogQuery, IReadOnlyList<UiRunLogEntry>> QueryHandler { get; set; } =
            _ => Array.Empty<UiRunLogEntry>();
        public List<RunLogQuery> Queries { get; } = new();

        public static RunLogFilterBackend Create()
        {
            var client = Create<IAriadneBackendClient, RunLogFilterBackend>();
            var backend = (RunLogFilterBackend)(object)client;
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
                return Task.FromResult(QueryHandler(query));
            }
            if (targetMethod?.Name == nameof(IAriadneBackendClient.MarkRunLogsReadAsync))
            {
                return Task.FromResult(0);
            }
            if (targetMethod?.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (targetMethod?.ReturnType.IsGenericType == true
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object?[] { null });
            }
            return null;
        }
    }
}
