namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// C5-a：节点拖拽主视觉更新按渲染帧合并 — 纯状态机，View 与单测共用。
/// 同一帧内多次 PointerMoved 只应调度一次 layout/edge Geometry 同步。
/// </summary>
public static class CanvasDragFrameHelpers
{
    /// <summary>
    /// 若本帧尚未调度，标记已调度并返回 true（调用方应 Post Render 回调）。
    /// 若已调度，返回 false（丢弃重复 PointerMoved 的视觉同步）。
    /// </summary>
    public static bool TryScheduleFrameSync(ref bool frameAlreadyScheduled)
    {
        if (frameAlreadyScheduled)
        {
            return false;
        }

        frameAlreadyScheduled = true;
        return true;
    }

    /// <summary>Render 回调入口 / 松手 flush：清除调度标记，允许下一帧再次同步。</summary>
    public static void OnFrameSyncStarted(ref bool frameAlreadyScheduled)
    {
        frameAlreadyScheduled = false;
    }

    /// <summary>
    /// C5-a：release/capture-lost 必须在清空 drag 状态前同步 flush 主视觉
    ///（否则挂起的 Render 回调见 dragging=false 会空转漏最后一帧）。
    /// </summary>
    public static bool MustFlushMainVisualsBeforeClearingDragState => true;

    /// <summary>
    /// C5-b：连续拖动结束时是否必须根据最终坐标重算 dirty。
    /// 始终 true — 不能依赖 move 期间 X/Y setter 副作用。
    /// </summary>
    public static bool MustRefreshDirtyAfterContinuousEditEnd => true;

    /// <summary>
    /// C5-a：PointerMoved 是否应立即执行主节点/边 Geometry（否：仅写坐标，帧回调再同步）。
    /// </summary>
    public static bool ShouldApplyMainVisualsOnPointerMoved => false;
}
