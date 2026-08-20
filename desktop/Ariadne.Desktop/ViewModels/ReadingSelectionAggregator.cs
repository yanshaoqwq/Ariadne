namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 阅读态的跨块选区归并（纯逻辑，不依赖 Avalonia，可单测）。
///
/// U132：正文在阅读态被切成 9 个独立 <c>SelectableTextBlock</c>，
/// 每个块的 <c>SelectionStart/End</c> 是**块内**索引，且 <c>SelectAll()</c>/<c>Copy()</c>
/// 只操作自己那一个块——最多选中 4000–6000 字符，不是整章。
/// 所以「复制选中内容」与「全选整章」都必须在**页面级**归并，
/// 这里负责把「每块各自选了哪一段」折算成全篇的一段连续文本。
/// </summary>
public static class ReadingSelectionAggregator
{
    /// <summary>
    /// 一个片段的选区采样：块索引 + 块内起止（半开区间）。
    /// </summary>
    /// <param name="BlockIndex">块索引（<c>DocumentBlocks</c> 里的位置）。</param>
    /// <param name="Start">起始索引（含）。</param>
    /// <param name="End">结束索引（不含）。</param>
    /// <param name="SegmentIndex">
    /// U203：同一块内的**片段序号**。阅读态渲染 Markdown 后，一个块会渲染成多个
    /// <c>SelectableTextBlock</c>（标题、段落、引用各一个控件），采样顺序不做假设，
    /// 所以块内也需要一个排序键。
    /// </param>
    /// <param name="SegmentText">
    /// U203：该片段**渲染后可见**的文本。
    ///
    /// 为什么不能省掉它、继续从 <c>DocumentBlockViewModel.Text</c> 切：渲染后
    /// 可见文本已经不等于原始正文了（`# 标题` 显示成 `标题`），
    /// 拿控件给出的可见索引去切原始正文会**切错位置**——用户复制标题会得到 `# `。
    /// null 表示「按原始正文切」，保留给未渲染路径与既有用例。
    /// </param>
    public readonly record struct BlockSelection(
        int BlockIndex,
        int Start,
        int End,
        int SegmentIndex = 0,
        string? SegmentText = null);

    /// <summary>
    /// 把各块的选区采样归并成一段全篇文本。
    ///
    /// 采样顺序不做假设（视觉树的枚举顺序未必等于正文顺序），内部按块索引排序。
    /// 空选区与倒置选区（Start >= End）由下面钳位后的 <c>end > start</c> 挡掉——
    /// 此处**刻意不再前置过滤一遍**：前置的 Where 与钳位后的判断是同一件事，
    /// 摘掉前者行为完全不变（已用变异测试证实），留着只是让人以为有两道防护。
    /// </summary>
    /// <returns>没有任何非空选区时返回空串。</returns>
    public static string Aggregate(
        IReadOnlyList<DocumentBlockViewModel> blocks,
        IEnumerable<BlockSelection> selections)
    {
        var ordered = selections
            .Where(item => item.BlockIndex >= 0 && item.BlockIndex < blocks.Count)
            .OrderBy(item => item.BlockIndex)
            .ThenBy(item => item.SegmentIndex)
            .ToList();
        if (ordered.Count == 0)
        {
            return string.Empty;
        }

        var builder = new System.Text.StringBuilder();
        var appendedAny = false;
        foreach (var selection in ordered)
        {
            var text = selection.SegmentText ?? blocks[selection.BlockIndex].Text;
            // 钳位而不是信任采样：控件在文本刚变化、选区尚未同步的那一帧会给出越界值，
            // 直接切片就是 ArgumentOutOfRangeException——用户只会看到「复制没反应」。
            // end 的下界取 start（而非 0）：这同时把 End < Start 的倒置采样压成空区间。
            var start = Math.Clamp(selection.Start, 0, text.Length);
            var end = Math.Clamp(selection.End, start, text.Length);
            if (end > start)
            {
                // U203：渲染态下**段与段之间的换行不在文本里**——它是版面
                // （各片段的 Margin / LineBreak）承载的。所以跨片段拼接时必须补回换行，
                // 否则 Ctrl+A + Ctrl+C 粘出来是「第一段第二段第三段」连成一片。
                // 只在有 SegmentText（即渲染路径）时补：未渲染路径的块文本本身
                // 已含边界换行，再补就多一个空行。
                if (appendedAny && selection.SegmentText is not null)
                {
                    builder.Append('\n');
                }
                builder.Append(text, start, end - start);
                appendedAny = true;
            }
        }
        return builder.ToString();
    }

    /// <summary>
    /// 是否存在任何有效选区。
    ///
    /// 单独给出而不是让调用方判断 <see cref="Aggregate"/> 的返回值是否为空，
    /// 因为「选中的正好是一段空白」与「什么都没选」在语义上不同：
    /// 前者该复制那段空白，后者该走「无选区」分支。
    /// </summary>
    public static bool HasSelection(IEnumerable<BlockSelection> selections)
        => selections.Any(item => item.End > item.Start);
}
