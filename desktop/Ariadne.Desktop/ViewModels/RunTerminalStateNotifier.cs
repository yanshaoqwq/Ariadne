namespace Ariadne.Desktop.ViewModels;

/// <summary>应用级「运行到达终态」的接收方。</summary>
internal interface IRunTerminalStateObserver
{
    /// <summary>某个运行刚进入 succeeded / failed / stopped。</summary>
    void OnRunReachedTerminalState(string workflowId, string runId, string status);
}

/// <summary>
/// 运行终态的应用级广播（U194-E 后半）。
///
/// ## 为什么需要它
///
/// 顶栏预算条是**应用级**部件，而运行会话协调器是**页级**的
/// （`WorkspacePageViewModel` 在自己的构造函数里 new 一个，外部拿不到句柄）。
/// 缺陷版本的后果：`RefreshBudgetStatusAsync` 只在「进项目」和「Git 回档后」被调，
/// 作者跑完一整天工作流，顶栏数字仍是**进项目那一刻**的——
/// 花了多少钱这件事，只有在下一次重开项目时才会更新。
///
/// ## 为什么是静态弱引用而不是定时器
///
/// 定时轮询被明确排除：那是拿「每 N 秒一次无条件 IPC」换一个本来可以**事件驱动**的
/// 刷新，且与仓库既有决定（`runtime_autosave_ms` 按「移除」处置）的取向相反。
/// 这里的信号是精确的——只在状态**跨进**终态的那一次跃迁上发一次。
///
/// 形状对齐既有的 `UserFacingError.RegisterObserver`：跨层通知在本工程里就是这么做的。
/// **弱引用**是为了让被替换掉的 ViewModel 自然退订——观察方是 ViewModel，
/// 没有确定的析构时机，强引用表会把整棵页面树钉在内存里。
/// </summary>
internal static class RunTerminalStateNotifier
{
    private static readonly object Sync = new();
    private static readonly List<WeakReference<IRunTerminalStateObserver>> Observers = new();

    public static void Register(IRunTerminalStateObserver observer)
    {
        lock (Sync)
        {
            // 同一实例重复注册会让一次终态触发两次刷新。按引用去重，
            // 顺手清掉已回收的槽位（弱引用表不会自己收缩）。
            for (var index = Observers.Count - 1; index >= 0; index--)
            {
                if (!Observers[index].TryGetTarget(out var existing))
                {
                    Observers.RemoveAt(index);
                    continue;
                }
                if (ReferenceEquals(existing, observer))
                {
                    return;
                }
            }
            Observers.Add(new WeakReference<IRunTerminalStateObserver>(observer));
        }
    }

    /// <summary>
    /// 广播一次终态到达。**调用方负责只在跃迁边沿调用**——
    /// 每轮轮询都调会把它变成 750ms 的定时刷新，那正是这里要避免的东西。
    /// </summary>
    public static void NotifyTerminal(string workflowId, string runId, string status)
    {
        IRunTerminalStateObserver[] targets;
        lock (Sync)
        {
            var alive = new List<IRunTerminalStateObserver>(Observers.Count);
            for (var index = Observers.Count - 1; index >= 0; index--)
            {
                if (Observers[index].TryGetTarget(out var observer))
                {
                    alive.Add(observer);
                }
                else
                {
                    Observers.RemoveAt(index);
                }
            }
            targets = alive.ToArray();
        }

        foreach (var observer in targets)
        {
            // 一个观察方抛异常不能让其余观察方收不到通知：这条链路是「顺带刷新」，
            // 不该成为运行状态投影的新故障点。
            try
            {
                observer.OnRunReachedTerminalState(workflowId, runId, status);
            }
            catch
            {
            }
        }
    }
}
