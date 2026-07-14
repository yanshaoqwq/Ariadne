namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 画布框选与多选：纯几何，便于单测。
/// </summary>
public static class CanvasSelectionHelpers
{
    /// <summary>归一化矩形（允许从任意角拖出）。</summary>
    public static (double X, double Y, double W, double H) NormalizeRect(
        double x0, double y0, double x1, double y1)
    {
        var left = Math.Min(x0, x1);
        var top = Math.Min(y0, y1);
        var right = Math.Max(x0, x1);
        var bottom = Math.Max(y0, y1);
        return (left, top, Math.Max(0, right - left), Math.Max(0, bottom - top));
    }

    /// <summary>
    /// 节点轴对齐包围盒是否与框选矩形相交。
    /// 节点用左上角 (nodeX,nodeY) + 宽高。
    /// </summary>
    public static bool NodeIntersectsRect(
        double nodeX, double nodeY, double nodeW, double nodeH,
        double rectX, double rectY, double rectW, double rectH)
    {
        if (rectW <= 0 || rectH <= 0 || nodeW <= 0 || nodeH <= 0)
        {
            return false;
        }

        return nodeX < rectX + rectW
               && nodeX + nodeW > rectX
               && nodeY < rectY + rectH
               && nodeY + nodeH > rectY;
    }

    /// <summary>拖动超过阈值才进入框选（与单击清空区分）。</summary>
    public static bool ExceedsMarqueeThreshold(double dx, double dy, double threshold = 4.0) =>
        (dx * dx) + (dy * dy) >= threshold * threshold;

    /// <summary>边是否挂在节点（端点之一）。</summary>
    public static bool EdgeTouchesNode(string edgeSource, string edgeTarget, string nodeId) =>
        string.Equals(edgeSource, nodeId, StringComparison.Ordinal)
        || string.Equals(edgeTarget, nodeId, StringComparison.Ordinal);

    /// <summary>边是否挂在任一已选节点上。</summary>
    public static bool EdgeTouchesAnyNode(
        string edgeSource,
        string edgeTarget,
        IEnumerable<string> nodeIds)
    {
        foreach (var id in nodeIds)
        {
            if (EdgeTouchesNode(edgeSource, edgeTarget, id))
            {
                return true;
            }
        }

        return false;
    }

    /// <summary>数据入是否已被占用（一入一线规则）。</summary>
    public static bool IsDataInOccupied(
        IEnumerable<(string Kind, string Target, string TargetHandle)> edges,
        string toNodeId,
        string toHandle)
    {
        foreach (var e in edges)
        {
            if (string.Equals(e.Kind, "data", StringComparison.OrdinalIgnoreCase)
                && string.Equals(e.Target, toNodeId, StringComparison.Ordinal)
                && string.Equals(e.TargetHandle, toHandle, StringComparison.OrdinalIgnoreCase))
            {
                return true;
            }
        }

        return false;
    }

    /// <summary>
    /// W1：Delete 目标 — 有选中边时优先删边（不连带端点节点）；否则删节点。
    /// </summary>
    public static bool PreferDeleteEdgeOverNodes(bool hasSelectedEdge, bool hasSelectedNode) =>
        hasSelectedEdge;
}
