namespace Ariadne.Desktop.ViewModels;

public interface IUnsavedChangesGuard
{
    bool HasUnsavedChanges { get; }

    /// <summary>批量离开对话框中展示的页面名称（本地化后）。</summary>
    string UnsavedChangesPageTitle { get; }

    /// <summary>单页离开：立即弹出确认并在此方法内执行保存/丢弃。</summary>
    Task<bool> ConfirmLeaveIfNeededAsync();

    /// <summary>
    /// 批量离开 prepare：校验并缓存提交载荷，不得持久化。
    /// 失败返回 false；成功后必须能调用 <see cref="CommitPreparedUnsavedChangesAsync"/>。
    /// </summary>
    Task<bool> PrepareUnsavedChangesAsync();

    /// <summary>
    /// 批量离开 commit：仅写入 prepare 阶段缓存的载荷。
    /// 未 prepare 时不得隐式写盘。
    /// </summary>
    Task<bool> CommitPreparedUnsavedChangesAsync();

    /// <summary>放弃 prepare 缓存（内存），不写盘。</summary>
    Task AbortPreparedUnsavedChangesAsync();

    /// <summary>单页保存：prepare + commit 的便捷组合。</summary>
    Task<bool> SaveUnsavedChangesAsync();

    /// <summary>批量离开：丢弃未保存更改（应无副作用或仅内存回滚）。</summary>
    Task DiscardUnsavedChangesAsync();
}
