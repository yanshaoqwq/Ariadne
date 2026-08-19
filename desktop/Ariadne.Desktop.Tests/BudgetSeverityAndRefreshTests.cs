using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U194-E 回归：顶栏预算条一路青绿到 100%，且跑完一整天数字都不动。
///
/// 判据落在**暴露给 View 的呈现状态**（`BudgetSeverity` / `IsBudgetError`）上，
/// 而不是「有没有算出百分比」——缺陷版本的百分比一直是对的，
/// 错的是它**唯一**能表达的东西就是百分比，而规范定的那条线是绝对余量。
/// </summary>
public sealed class BudgetSeverityAndRefreshTests
{
    [Fact]
    public async Task RemainingBelowTwoDollarsPaintsTheHeaderErrorState()
    {
        var backend = BudgetBackend.Create(budgetUsd: 100, spentUsd: 99);
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await window.RefreshBudgetStatusForTestsAsync();

        // 余量 $1：后端预算门随时会拒下一次调用，界面必须在此之前就变色。
        Assert.Equal(BudgetSeverity.Error, window.BudgetSeverity);
        Assert.True(window.IsBudgetError);
        Assert.Equal(1, window.BudgetRemainingUsd);
    }

    [Fact]
    public async Task HalfSpentBudgetStaysInTheNormalState()
    {
        var backend = BudgetBackend.Create(budgetUsd: 100, spentUsd: 50);
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await window.RefreshBudgetStatusForTestsAsync();

        // 余量 $50：既不告急也不接近上限，顶栏保持强调青绿（不挂任何分档类）。
        Assert.Equal(BudgetSeverity.Normal, window.BudgetSeverity);
        Assert.False(window.IsBudgetError);
        Assert.False(window.IsBudgetWarning);
    }

    /// <summary>
    /// 边界钉死：规范（`指导性文件/UI组件状态表.md:34`）写的是「余量**&lt;**$2」，
    /// 所以取**严格小于**——余量正好 $2 不是 error 档。
    ///
    /// 两侧都钉：只钉一侧的话，把判据写成 `<=` 或 `<` 都能让用例全绿，
    /// 那这条「边界」就没有被任何断言约束住。
    /// </summary>
    [Theory]
    [InlineData(1.99, true)]
    [InlineData(2.00, false)]
    [InlineData(2.01, false)]
    public async Task ErrorStateBoundaryIsStrictlyBelowTwoDollars(double remaining, bool expectError)
    {
        var backend = BudgetBackend.Create(budgetUsd: 100, spentUsd: 100 - remaining);
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await window.RefreshBudgetStatusForTestsAsync();

        Assert.Equal(expectError, window.IsBudgetError);
        Assert.Equal(expectError ? BudgetSeverity.Error : BudgetSeverity.Warning, window.BudgetSeverity);
    }

    /// <summary>
    /// 日预算 `0` = **不设上限**（U112），不是「余量为负」。
    ///
    /// 这条是本文件最重要的一条：按 `budget - spent` 直算会得到 -spent，
    /// 于是**未设限额的用户顶栏永久报红**——而红色在这里的语义是「快没钱了」，
    /// 一直红着等于把分级本身作废，还会把真正的临界态淹掉。
    /// </summary>
    [Fact]
    public async Task UnlimitedBudgetIsNeverPaintedAsError()
    {
        var backend = BudgetBackend.Create(budgetUsd: 0, spentUsd: 137.5);
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);

        await window.RefreshBudgetStatusForTestsAsync();

        Assert.NotEqual(BudgetSeverity.Error, window.BudgetSeverity);
        Assert.False(window.IsBudgetError);
        Assert.False(window.IsBudgetWarning);
        Assert.Null(window.BudgetRemainingUsd);
        Assert.Equal(
            DisplayNameService.LoadDefault().Text("ui.layout.budget_unlimited"),
            window.BudgetStatusText);
    }

    /// <summary>
    /// 终态刷新那半：运行到达终态后，顶栏预算必须真的重新问一次后端。
    ///
    /// 判据取**后端被问的次数**而不是「数字变了没有」：假后端的返回值是固定的，
    /// 断言数字变化会要求我先造一个「花费会自己增长」的后端，
    /// 那测的就变成假后端自己了。次数是这条链路上唯一属于生产代码的可观测量。
    /// </summary>
    [Fact]
    public async Task ReachingTerminalStateRefreshesTheHeaderBudget()
    {
        var backend = BudgetBackend.Create(budgetUsd: 100, spentUsd: 10);
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);

        // 终态刷新有 `if (!HasOpenProject) return;` 这道闸（离开项目后仍可能收到
        // 迟到的终态，那时刷预算会把刚归位的顶栏填上上个项目的数字）。
        // 生产里进项目时会置位，用例得显式推到同一状态，否则测的是那道闸而不是刷新。
        window.MarkProjectOpenForTests();

        await window.RefreshBudgetStatusForTestsAsync();
        var afterInitialLoad = backend.BudgetQueryCount;
        Assert.True(afterInitialLoad >= 1, "初次加载本身就该问一次，否则下面的增量判据没有基线");

        // 通过接口调用**生产实现**（显式接口实现，故须转型）——不为测试新开钩子：
        // 钩子会让用例绕过真实的调用入口，而那个入口正是本条要守的东西。
        ((IRunTerminalStateObserver)window).OnRunReachedTerminalState("wf-1", "run-1", "succeeded");
        await WaitForBudgetQueryAsync(backend, afterInitialLoad);

        Assert.True(
            backend.BudgetQueryCount > afterInitialLoad,
            $"运行到终态后必须重新查预算：初次 {afterInitialLoad} 次，终态后仍是 {backend.BudgetQueryCount} 次");
    }

    /// <summary>
    /// 同一次运行只刷一次 —— 画布页可以被重建，新协调器从空状态挂接到同一个 runId
    /// 时会再走一次「非终态 → 终态」，那属于同一次运行，不该再发一次 IPC。
    ///
    /// 这条与上一条**互补**：上一条保证「会刷」，这条保证「不会刷个不停」。
    /// 少了这条，把去重整个摘掉照样只有绿灯。
    /// </summary>
    [Fact]
    public async Task SameRunReachingTerminalStateTwiceOnlyRefreshesOnce()
    {
        var backend = BudgetBackend.Create(budgetUsd: 100, spentUsd: 10);
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend.Client);

        window.MarkProjectOpenForTests();

        await window.RefreshBudgetStatusForTestsAsync();
        var baseline = backend.BudgetQueryCount;
        var observer = (IRunTerminalStateObserver)window;

        observer.OnRunReachedTerminalState("wf-1", "run-1", "succeeded");
        await WaitForBudgetQueryAsync(backend, baseline);
        var afterFirstTerminal = backend.BudgetQueryCount;

        // 同一个 (workflow, run, status) 再来一次：不该再发 IPC。
        observer.OnRunReachedTerminalState("wf-1", "run-1", "succeeded");
        await Task.Delay(120);

        Assert.Equal(afterFirstTerminal, backend.BudgetQueryCount);

        // 而**另一次**运行到终态仍然要刷 —— 去重的粒度是「同一次运行」，不是「一次就够」。
        observer.OnRunReachedTerminalState("wf-1", "run-2", "succeeded");
        await WaitForBudgetQueryAsync(backend, afterFirstTerminal);
        Assert.True(
            backend.BudgetQueryCount > afterFirstTerminal,
            "不同 runId 到终态应各刷一次，去重粒度不能宽到把后续运行也吞掉");
    }


    /// <summary>
    /// `OnRunReachedTerminalState` 里是 `_ = RefreshBudgetStatusAsync()`（刻意不 await——
    /// 事件回调不该阻塞运行态推进），所以用例必须等那个后台任务真正问到后端。
    /// 轮询而不是固定 `Delay`：固定延时在这台负载很高的机器上会偶发失败，
    /// 而轮询在快的时候立刻返回、慢的时候多等一会儿。
    /// </summary>
    private static async Task WaitForBudgetQueryAsync(BudgetBackend backend, int baseline)
    {
        for (var i = 0; i < 50 && backend.BudgetQueryCount <= baseline; i++)
        {
            await Task.Delay(20);
        }
    }

    /// <summary>
    /// 假后端：只回答预算查询，并记下被问了几次（终态刷新那半的判据）。
    /// </summary>
    private class BudgetBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;

        public double BudgetUsd { get; set; }

        public double SpentUsd { get; set; }

        public int BudgetQueryCount { get; private set; }

        public static BudgetBackend Create(double budgetUsd, double spentUsd)
        {
            var client = Create<IAriadneBackendClient, BudgetBackend>();
            var backend = (BudgetBackend)(object)client;
            backend.Client = client;
            backend.BudgetUsd = budgetUsd;
            backend.SpentUsd = spentUsd;
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
            if (targetMethod.Name == nameof(IAriadneBackendClient.GetBudgetStatusAsync))
            {
                BudgetQueryCount++;
                return Task.FromResult(new BudgetStatus(BudgetUsd, SpentUsd, null, false));
            }
            if (targetMethod.Name == nameof(IAriadneBackendClient.GetSidebarBadgesAsync))
            {
                return Task.FromResult(new SidebarBadgeCounts(0, 0, 0));
            }
            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object?[] { resultType.IsValueType ? Activator.CreateInstance(resultType) : null });
            }
            return targetMethod.ReturnType.IsValueType
                ? Activator.CreateInstance(targetMethod.ReturnType)
                : null;
        }
    }
}
