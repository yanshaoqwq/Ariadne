namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 阅读位置 ↔ 字符偏移的换算（纯逻辑，不依赖 Avalonia，可单测）。
///
/// U129：阅读模式与编辑器是**两套完全不同的滚动坐标系**——
/// 阅读侧是「第几个块 + 块内滚过多少」，编辑器侧是「行号 / 像素偏移」。
/// 两者唯一能互换的共同量是**全篇字符偏移**，所以视图切换时保存的锚点
/// 就是一个 int：切换前把当前视图的位置折算成偏移，切换后再从偏移折算回去。
///
/// 精度取舍：块内位置用**线性比例插值**（比例 × 块字符数），不做逐行测量。
/// 理由是正文为中文、单栏、行高固定，字符密度在块内高度均匀，
/// 误差通常在一两行内；而逐行精确定位要访问 <c>SelectableTextBlock</c> 的
/// TextLayout（protected，拿不到），代价与收益完全不成比例。
/// 对照物是缺陷版本的「每次从头开始」——差一两行与差十屏不是一个量级的问题。
/// </summary>
public static class ReadingPositionMapper
{
    /// <summary>
    /// 阅读侧采样：第 <paramref name="blockIndex"/> 块、块内滚过 <paramref name="withinBlockRatio"/>
    /// 的位置对应的全篇字符偏移。
    /// </summary>
    /// <param name="blocks">当前阅读块列表（顺序即正文顺序）。</param>
    /// <param name="blockIndex">首个可见块的索引。</param>
    /// <param name="withinBlockRatio">该块已滚过的比例，允许传入越界值，内部钳到 [0,1]。</param>
    public static int OffsetFromBlockProgress(
        IReadOnlyList<DocumentBlockViewModel> blocks,
        int blockIndex,
        double withinBlockRatio)
    {
        if (blocks.Count == 0)
        {
            return 0;
        }

        var index = Math.Clamp(blockIndex, 0, blocks.Count - 1);
        var block = blocks[index];
        // NaN 会让 Clamp 返回 NaN、再乘出 NaN 偏移，最终 (int) 转换得到 0——
        // 那等于静默退回「从头开始」，正是本条缺陷的现象。显式挡掉。
        var ratio = double.IsNaN(withinBlockRatio)
            ? 0d
            : Math.Clamp(withinBlockRatio, 0d, 1d);
        var within = (int)Math.Round(ratio * block.Text.Length, MidpointRounding.AwayFromZero);
        return block.StartOffset + Math.Clamp(within, 0, block.Text.Length);
    }

    /// <summary>
    /// 阅读侧恢复：把全篇字符偏移折算回「第几块 + 块内比例」。
    /// </summary>
    /// <returns>没有块时返回 false（此时调用方不该做任何滚动）。</returns>
    public static bool TryLocateOffset(
        IReadOnlyList<DocumentBlockViewModel> blocks,
        int offset,
        out int blockIndex,
        out double withinBlockRatio)
    {
        blockIndex = 0;
        withinBlockRatio = 0d;
        if (blocks.Count == 0)
        {
            return false;
        }

        var target = Math.Max(0, offset);
        for (var index = 0; index < blocks.Count; index++)
        {
            var block = blocks[index];
            // 用半开区间 [StartOffset, EndOffset)：块边界上的偏移必须归**后**一块，
            // 归前一块会得到 ratio = 1（滚到块尾），视觉上差一整块。
            // 末块例外——正文末尾的偏移等于 EndOffset，没有「后一块」可归。
            var isLastBlock = index == blocks.Count - 1;
            if (target < block.EndOffset || isLastBlock)
            {
                blockIndex = index;
                withinBlockRatio = block.Text.Length == 0
                    ? 0d
                    : Math.Clamp((double)(target - block.StartOffset) / block.Text.Length, 0d, 1d);
                return true;
            }
        }

        // 循环必然在末块命中，这里只是让编译器与未来的改动者都不必猜。
        blockIndex = blocks.Count - 1;
        withinBlockRatio = 1d;
        return true;
    }

    /// <summary>
    /// 换文档时锚点必须失效。
    ///
    /// 与 <see cref="EditorStickySelectionPolicy.ClearOnDocumentChange"/> 同理：
    /// 上一篇的字符偏移套到新正文上会滚到一个毫无关系的位置，
    /// 而这种错位比「从头开始」更难被用户理解成 bug。
    /// </summary>
    public static int? ClearOnDocumentChange() => null;
}
