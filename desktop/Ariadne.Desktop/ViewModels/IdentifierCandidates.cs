namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// U145：标识 / 别名候选值的合成收口。
///
/// 这些字段（章节 id、数据别名、引脚名、审批 id…）的**取值集合产品全都持有**：
/// 章节来自作品树（后端 `ChapterDocumentIndex`）、别名在边上已经定义过一次、
/// 引脚名写死在节点类型定义里。此前它们全是自由文本框，用户只能凭记忆手打，
/// 而后端对这些字段是**精确等值**匹配——手打即错，且错了只是静默无结果
/// （无候选提示、无校验、无「未匹配」反馈）。
///
/// 收口成一处而不是各自 `Distinct().Where()`：这套「去空白 → 去重 → 保序」的
/// 规则一旦在 14 个站点各写一遍，迟早有一处漏掉 trim 或漏掉去重，
/// 表现为下拉里出现两个看起来一样的选项，用户无从判断该选哪个。
/// </summary>
internal static class IdentifierCandidates
{
    /// <summary>
    /// 合成候选列表：按传入顺序保序、去空白、按 trim 后的值去重。
    ///
    /// **保序而不是排字典序**：靠前的组是「最可能正确的那个」（当前节点已连上的边、
    /// 节点类型的约定默认值），排序会把它们打散到字母表里，让用户在一串
    /// 长得差不多的 id 中重新找。
    /// </summary>
    public static List<string> Compose(params IEnumerable<string?>[] groups)
    {
        var result = new List<string>();
        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (var group in groups)
        {
            foreach (var raw in group)
            {
                var value = raw?.Trim();
                if (string.IsNullOrEmpty(value) || !seen.Add(value))
                {
                    continue;
                }

                result.Add(value);
            }
        }

        return result;
    }

    /// <summary>
    /// 把合成结果写进既有集合，**内容相同时不动集合**。
    ///
    /// 为什么不直接 Clear + 重填：`AutoCompleteBox` 订阅 `ItemsSource` 的集合变更，
    /// Clear 会让正在展开的候选面板瞬间空掉、并把用户输入的过滤词的匹配结果清掉。
    /// 而这些候选每次选中节点/连边都会重算，绝大多数情况下结果一模一样。
    /// </summary>
    public static void Sync(IList<string> target, IReadOnlyList<string> desired)
    {
        if (target.Count == desired.Count)
        {
            var same = true;
            for (var i = 0; i < desired.Count; i++)
            {
                if (!string.Equals(target[i], desired[i], StringComparison.Ordinal))
                {
                    same = false;
                    break;
                }
            }

            if (same)
            {
                return;
            }
        }

        target.Clear();
        foreach (var item in desired)
        {
            target.Add(item);
        }
    }
}
