namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 工作流导出目标节点：整图导出与「导出所选」语义分离，避免工具栏导出被当前选中缩窄。
/// </summary>
public static class WorkflowExportSelection
{
    /// <summary>
    /// <paramref name="requireSelection"/> 为 true 时只返回选中 id（若为空则空数组）；
    /// 为 false 时始终返回 <paramref name="allNodeIds"/> 副本（整图导出）。
    /// </summary>
    public static string[] ResolveNodeIds(
        bool requireSelection,
        string? selectedNodeId,
        IReadOnlyList<string> allNodeIds)
    {
        var all = allNodeIds is null || allNodeIds.Count == 0
            ? Array.Empty<string>()
            : allNodeIds.Where(id => !string.IsNullOrWhiteSpace(id)).ToArray();

        if (!requireSelection)
        {
            return all;
        }

        if (string.IsNullOrWhiteSpace(selectedNodeId))
        {
            return Array.Empty<string>();
        }

        return new[] { selectedNodeId };
    }
}
