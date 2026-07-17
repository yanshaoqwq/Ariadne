namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 画布视口：适应视图、平移、滚轮缩放 — 纯函数，供 View 与单测共用。
/// </summary>
public static class CanvasViewportHelpers
{
    public const double MinZoom = 0.25;
    public const double MaxZoom = 2.5;
    public const double DefaultFitPadding = 48;

    /// <summary>
    /// W2：按节点包围盒与真实视口计算 zoom + 平移，使图落入可见区（非仅非负左上角微调）。
    /// </summary>
    public static (double Zoom, double OffsetX, double OffsetY) ComputeFitTransform(
        double minX,
        double minY,
        double maxX,
        double maxY,
        double viewportWidth,
        double viewportHeight,
        double padding = DefaultFitPadding)
    {
        var contentW = Math.Max(1.0, maxX - minX);
        var contentH = Math.Max(1.0, maxY - minY);
        var availW = Math.Max(1.0, viewportWidth - (2 * padding));
        var availH = Math.Max(1.0, viewportHeight - (2 * padding));
        var zoom = Math.Clamp(Math.Min(availW / contentW, availH / contentH), MinZoom, MaxZoom);
        var usedW = contentW * zoom;
        var usedH = contentH * zoom;
        var offsetX = padding - (minX * zoom) + ((availW - usedW) * 0.5);
        var offsetY = padding - (minY * zoom) + ((availH - usedH) * 0.5);
        return (zoom, offsetX, offsetY);
    }

    /// <summary>W6：在未被工具栏/小地图占用的安全矩形内执行 Fit。</summary>
    public static (double Zoom, double OffsetX, double OffsetY) ComputeFitTransform(
        double minX,
        double minY,
        double maxX,
        double maxY,
        CanvasViewportRect safeViewport,
        double padding = DefaultFitPadding)
    {
        var safe = safeViewport.Normalize();
        var (zoom, offsetX, offsetY) = ComputeFitTransform(
            minX,
            minY,
            maxX,
            maxY,
            safe.Width,
            safe.Height,
            padding);
        return (zoom, offsetX + safe.X, offsetY + safe.Y);
    }

    /// <summary>W2：指针滚轮缩放（deltaY 正→放大）。</summary>
    public static double ApplyWheelZoom(double currentZoom, double wheelDeltaY, double step = 0.1)
    {
        var next = wheelDeltaY > 0 ? currentZoom + step : currentZoom - step;
        return Math.Clamp(next, MinZoom, MaxZoom);
    }

    /// <summary>
    /// 缩放时保持锚点下的逻辑坐标不动，避免滚轮和工具栏缩放把用户关注位置甩走。
    /// </summary>
    public static (double OffsetX, double OffsetY) ComputeAnchoredZoomOffset(
        double oldZoom,
        double newZoom,
        double offsetX,
        double offsetY,
        double anchorX,
        double anchorY)
    {
        var safeOldZoom = Math.Max(MinZoom, oldZoom);
        var clampedNewZoom = Math.Clamp(newZoom, MinZoom, MaxZoom);
        var logicalAnchorX = (anchorX - offsetX) / safeOldZoom;
        var logicalAnchorY = (anchorY - offsetY) / safeOldZoom;
        return (
            anchorX - (logicalAnchorX * clampedNewZoom),
            anchorY - (logicalAnchorY * clampedNewZoom));
    }

    /// <summary>W2：平移偏移（屏幕像素）。</summary>
    public static (double OffsetX, double OffsetY) ApplyPan(
        double offsetX,
        double offsetY,
        double deltaX,
        double deltaY) =>
        (offsetX + deltaX, offsetY + deltaY);

    /// <summary>
    /// W6：把节点完整屏幕矩形保持在视口内，并避开真实浮层矩形。
    /// 返回逻辑坐标，供拖动、新建和粘贴共用。
    /// </summary>
    public static (double X, double Y) KeepNodeReachable(
        double logicalX,
        double logicalY,
        double nodeWidth,
        double nodeHeight,
        double zoom,
        double offsetX,
        double offsetY,
        double viewportWidth,
        double viewportHeight,
        IReadOnlyList<CanvasViewportRect> occlusions,
        double gap = 8)
    {
        var safeZoom = Math.Max(MinZoom, zoom);
        var screenWidth = Math.Max(1, nodeWidth * safeZoom);
        var screenHeight = Math.Max(1, nodeHeight * safeZoom);
        var minX = Math.Min(gap, Math.Max(0, viewportWidth - screenWidth));
        var minY = Math.Min(gap, Math.Max(0, viewportHeight - screenHeight));
        var maxX = Math.Max(minX, viewportWidth - gap - screenWidth);
        var maxY = Math.Max(minY, viewportHeight - gap - screenHeight);
        var originX = Math.Clamp((logicalX * safeZoom) + offsetX, minX, maxX);
        var originY = Math.Clamp((logicalY * safeZoom) + offsetY, minY, maxY);
        var x = originX;
        var y = originY;
        var blockers = occlusions
            .Select(rect => rect.Normalize().Inflate(gap))
            .Where(rect => rect.Width > 0 && rect.Height > 0)
            .ToArray();

        for (var pass = 0; pass < Math.Max(1, blockers.Length * 2); pass++)
        {
            var current = new CanvasViewportRect(x, y, screenWidth, screenHeight);
            var blocker = blockers.FirstOrDefault(current.Intersects);
            if (blocker.Width <= 0 || blocker.Height <= 0)
            {
                break;
            }

            var candidates = new[]
            {
                (X: blocker.X - screenWidth, Y: y),
                (X: blocker.Right, Y: y),
                (X: x, Y: blocker.Y - screenHeight),
                (X: x, Y: blocker.Bottom),
            };
            var best = candidates
                .Select(candidate =>
                {
                    var cx = Math.Clamp(candidate.X, minX, maxX);
                    var cy = Math.Clamp(candidate.Y, minY, maxY);
                    var rect = new CanvasViewportRect(cx, cy, screenWidth, screenHeight);
                    var intersections = blockers.Count(rect.Intersects);
                    var distance = Math.Pow(cx - originX, 2) + Math.Pow(cy - originY, 2);
                    return (X: cx, Y: cy, Score: (intersections * 1_000_000_000d) + distance);
                })
                .OrderBy(candidate => candidate.Score)
                .First();
            x = best.X;
            y = best.Y;
        }

        return ((x - offsetX) / safeZoom, (y - offsetY) / safeZoom);
    }
}

public readonly record struct CanvasViewportRect(double X, double Y, double Width, double Height)
{
    public double Right => X + Width;

    public double Bottom => Y + Height;

    public CanvasViewportRect Normalize()
    {
        var left = Math.Min(X, Right);
        var top = Math.Min(Y, Bottom);
        return new CanvasViewportRect(left, top, Math.Abs(Width), Math.Abs(Height));
    }

    public CanvasViewportRect Inflate(double amount) => new(
        X - amount,
        Y - amount,
        Width + (amount * 2),
        Height + (amount * 2));

    public bool Intersects(CanvasViewportRect other) =>
        X < other.Right
        && Right > other.X
        && Y < other.Bottom
        && Bottom > other.Y;

    public double IntersectionArea(CanvasViewportRect other)
    {
        var width = Math.Max(0, Math.Min(Right, other.Right) - Math.Max(X, other.X));
        var height = Math.Max(0, Math.Min(Bottom, other.Bottom) - Math.Max(Y, other.Y));
        return width * height;
    }
}

/// <summary>
/// W11：按边中点切线生成候选位，使用真实标签尺寸避开节点和已放置标签。
/// 这是整图布局步骤，不进入节点拖动的 PointerMoved 热路径。
/// </summary>
public static class CanvasEdgeLabelLayoutHelpers
{
    // TextBlock MaxWidth 180 + Border 水平 padding 10 + 边框 2。
    public const double MaximumLabelWidth = 192;
    public const double FallbackLabelHeight = 18;

    public static IReadOnlyList<CanvasEdgeLabelPlacement> PlaceLabels(
        IReadOnlyList<CanvasEdgeLabelRequest> requests,
        IReadOnlyList<CanvasViewportRect> nodeBounds,
        double gap = 6)
    {
        var nodes = nodeBounds
            .Select(rect => rect.Normalize().Inflate(gap))
            .Where(rect => rect.Width > 0 && rect.Height > 0)
            .ToArray();
        var occupiedLabels = new List<CanvasViewportRect>();
        var placements = new CanvasEdgeLabelPlacement[requests.Count];

        foreach (var entry in requests
                     .Select((request, index) => (Request: request, Index: index))
                     .OrderByDescending(entry => entry.Request.IsPriority)
                     .ThenBy(entry => entry.Index))
        {
            var request = entry.Request;
            var width = Math.Clamp(request.Width, 1, MaximumLabelWidth);
            var height = Math.Max(1, request.Height);
            var candidates = CandidateRects(request, width, height, gap).ToArray();
            var best = candidates
                .Select((rect, candidateIndex) =>
                {
                    var nodeCollisions = nodes.Count(rect.Intersects);
                    var labelCollisions = occupiedLabels.Count(rect.Intersects);
                    var overlapArea = nodes.Sum(rect.IntersectionArea)
                                      + occupiedLabels.Sum(rect.IntersectionArea);
                    var centerX = rect.X + (rect.Width * 0.5);
                    var centerY = rect.Y + (rect.Height * 0.5);
                    var displacement = Math.Pow(centerX - request.AnchorX, 2)
                                       + Math.Pow(centerY - request.AnchorY, 2);
                    return (
                        Rect: rect,
                        Collisions: nodeCollisions + labelCollisions,
                        OverlapArea: overlapArea,
                        Displacement: displacement,
                        CandidateIndex: candidateIndex);
                })
                .OrderBy(candidate => candidate.Collisions)
                .ThenBy(candidate => candidate.OverlapArea)
                .ThenBy(candidate => candidate.Displacement)
                .ThenBy(candidate => candidate.CandidateIndex)
                .First();

            // 密集图没有空位时保留选中边，其余标签隐藏，避免全部堆叠成不可读色块。
            var visible = best.Collisions == 0 || request.IsPriority;
            placements[entry.Index] = new CanvasEdgeLabelPlacement(
                request.Id,
                best.Rect.X,
                best.Rect.Y,
                best.Rect.Width,
                best.Rect.Height,
                visible);
            if (visible)
            {
                occupiedLabels.Add(best.Rect.Inflate(gap));
            }
        }

        return placements;
    }

    public static (double Width, double Height) FallbackSize(string? text)
    {
        var width = 12.0;
        foreach (var rune in (text ?? string.Empty).EnumerateRunes())
        {
            width += rune.Value > 0x7f ? 10 : 6;
        }
        return (Math.Clamp(width, 28, MaximumLabelWidth), FallbackLabelHeight);
    }

    private static IEnumerable<CanvasViewportRect> CandidateRects(
        CanvasEdgeLabelRequest request,
        double width,
        double height,
        double gap)
    {
        var tangentX = request.TangentX;
        var tangentY = request.TangentY;
        var magnitude = Math.Sqrt((tangentX * tangentX) + (tangentY * tangentY));
        if (magnitude < 0.001)
        {
            tangentX = 1;
            tangentY = 0;
            magnitude = 1;
        }
        tangentX /= magnitude;
        tangentY /= magnitude;
        var normalX = -tangentY;
        var normalY = tangentX;
        var firstOffset = (height * 0.5) + gap + 3;
        var normalOffsets = new[]
        {
            firstOffset,
            -firstOffset,
            firstOffset + 18,
            -(firstOffset + 18),
            firstOffset + 36,
            -(firstOffset + 36),
        };

        foreach (var normalOffset in normalOffsets)
        {
            yield return CenteredRect(
                request.AnchorX + (normalX * normalOffset),
                request.AnchorY + (normalY * normalOffset),
                width,
                height);
        }

        var tangentOffset = Math.Clamp(width * 0.55, 24, 80);
        foreach (var normalOffset in normalOffsets.Take(4))
        {
            yield return CenteredRect(
                request.AnchorX + (normalX * normalOffset) + (tangentX * tangentOffset),
                request.AnchorY + (normalY * normalOffset) + (tangentY * tangentOffset),
                width,
                height);
            yield return CenteredRect(
                request.AnchorX + (normalX * normalOffset) - (tangentX * tangentOffset),
                request.AnchorY + (normalY * normalOffset) - (tangentY * tangentOffset),
                width,
                height);
        }
    }

    private static CanvasViewportRect CenteredRect(
        double centerX,
        double centerY,
        double width,
        double height) =>
        new(centerX - (width * 0.5), centerY - (height * 0.5), width, height);
}

public readonly record struct CanvasEdgeLabelRequest(
    string Id,
    double AnchorX,
    double AnchorY,
    double TangentX,
    double TangentY,
    double Width,
    double Height,
    bool IsPriority = false);

public readonly record struct CanvasEdgeLabelPlacement(
    string Id,
    double X,
    double Y,
    double Width,
    double Height,
    bool IsVisible)
{
    public CanvasViewportRect Bounds => new(X, Y, Width, Height);
}

/// <summary>W15：按当前图包围盒生成的小地图坐标变换。</summary>
public readonly record struct CanvasMiniMapTransform(
    double LogicalCenterX,
    double LogicalCenterY,
    double Scale,
    double ContentWidth,
    double ContentHeight)
{
    public (double X, double Y) LogicalToMiniMap(double logicalX, double logicalY) =>
        (
            (ContentWidth * 0.5) + ((logicalX - LogicalCenterX) * Scale),
            (ContentHeight * 0.5) + ((logicalY - LogicalCenterY) * Scale));

    public (double X, double Y) MiniMapToLogical(double miniX, double miniY) =>
        (
            LogicalCenterX + ((miniX - (ContentWidth * 0.5)) / Scale),
            LogicalCenterY + ((miniY - (ContentHeight * 0.5)) / Scale));

    public (double X, double Y) NodeMarkerPosition(
        double nodeX,
        double nodeY,
        double nodeWidth,
        double nodeHeight,
        double markerWidth = CanvasMiniMapHelpers.MarkerWidth,
        double markerHeight = CanvasMiniMapHelpers.MarkerHeight)
    {
        var (centerX, centerY) = LogicalToMiniMap(
            nodeX + (nodeWidth * 0.5),
            nodeY + (nodeHeight * 0.5));
        return (
            Math.Clamp(centerX - (markerWidth * 0.5), 0, Math.Max(0, ContentWidth - markerWidth)),
            Math.Clamp(centerY - (markerHeight * 0.5), 0, Math.Max(0, ContentHeight - markerHeight)));
    }

    public (double X, double Y, double Width, double Height) ViewportFrame(
        double logicalLeft,
        double logicalTop,
        double logicalWidth,
        double logicalHeight)
    {
        var (rawLeft, rawTop) = LogicalToMiniMap(logicalLeft, logicalTop);
        var rawRight = rawLeft + (Math.Max(0, logicalWidth) * Scale);
        var rawBottom = rawTop + (Math.Max(0, logicalHeight) * Scale);
        var left = Math.Clamp(Math.Min(rawLeft, rawRight), 0, ContentWidth);
        var top = Math.Clamp(Math.Min(rawTop, rawBottom), 0, ContentHeight);
        var right = Math.Clamp(Math.Max(rawLeft, rawRight), 0, ContentWidth);
        var bottom = Math.Clamp(Math.Max(rawTop, rawBottom), 0, ContentHeight);
        var width = Math.Max(0, right - left);
        var height = Math.Max(0, bottom - top);
        (left, width) = EnsureMinimumFrame(left, width, ContentWidth, 8.0);
        (top, height) = EnsureMinimumFrame(top, height, ContentHeight, 6.0);
        return (left, top, width, height);
    }

    private static (double Position, double Length) EnsureMinimumFrame(
        double position,
        double length,
        double contentLength,
        double preferredMinimum)
    {
        if (length <= 0 || length >= preferredMinimum || contentLength <= 0)
        {
            return (position, length);
        }

        var expanded = Math.Min(preferredMinimum, contentLength);
        var center = position + (length * 0.5);
        return (Math.Clamp(center - (expanded * 0.5), 0, contentLength - expanded), expanded);
    }
}

public static class CanvasMiniMapHelpers
{
    public const double ContentWidth = 140;
    public const double ContentHeight = 84;
    public const double MarkerWidth = 10;
    public const double MarkerHeight = 6;
    public const double Padding = 6;

    /// <summary>
    /// 图为空时保留旧 1400×840 逻辑范围；有节点时始终按真实图 bounds 居中适配。
    /// maxX/maxY 是包含节点尺寸后的右/下边界。
    /// </summary>
    public static CanvasMiniMapTransform ComputeTransform(
        double minX,
        double minY,
        double maxX,
        double maxY)
    {
        if (!double.IsFinite(minX)
            || !double.IsFinite(minY)
            || !double.IsFinite(maxX)
            || !double.IsFinite(maxY)
            || maxX <= minX
            || maxY <= minY)
        {
            minX = 0;
            minY = 0;
            maxX = 1400;
            maxY = 840;
        }

        var spanX = Math.Max(1.0, maxX - minX);
        var spanY = Math.Max(1.0, maxY - minY);
        var availableWidth = Math.Max(1.0, ContentWidth - (Padding * 2));
        var availableHeight = Math.Max(1.0, ContentHeight - (Padding * 2));
        var scale = Math.Max(0.000001, Math.Min(availableWidth / spanX, availableHeight / spanY));
        return new CanvasMiniMapTransform(
            LogicalCenterX: (minX + maxX) * 0.5,
            LogicalCenterY: (minY + maxY) * 0.5,
            Scale: scale,
            ContentWidth: ContentWidth,
            ContentHeight: ContentHeight);
    }
}

/// <summary>W9：缩放层级与精细编辑门禁。</summary>
public static class CanvasSemanticZoomHelpers
{
    public const double DetailThreshold = 0.75;
    public const double PrecisionControlThreshold = 0.8;
    public const double FocusZoom = 1.0;

    public static bool ShowDetails(double zoom) => zoom >= DetailThreshold;

    public static bool AllowPrecisionControls(double zoom) => zoom >= PrecisionControlThreshold;
}

/// <summary>W4：键盘方向键在画布节点之间进行空间导航。</summary>
public static class CanvasKeyboardNavigationHelpers
{
    public static string? FindDirectionalNode(
        string currentNodeId,
        IReadOnlyList<CanvasKeyboardNode> nodes,
        CanvasKeyboardDirection direction)
    {
        var current = nodes.FirstOrDefault(node =>
            string.Equals(node.Id, currentNodeId, StringComparison.Ordinal));
        if (string.IsNullOrEmpty(current.Id))
        {
            return null;
        }

        var currentX = current.X + (current.Width * 0.5);
        var currentY = current.Y + (current.Height * 0.5);
        return nodes
            .Where(node => !string.Equals(node.Id, currentNodeId, StringComparison.Ordinal))
            .Select(node =>
            {
                var dx = node.X + (node.Width * 0.5) - currentX;
                var dy = node.Y + (node.Height * 0.5) - currentY;
                var primary = direction switch
                {
                    CanvasKeyboardDirection.Left => -dx,
                    CanvasKeyboardDirection.Right => dx,
                    CanvasKeyboardDirection.Up => -dy,
                    _ => dy,
                };
                var secondary = direction is CanvasKeyboardDirection.Left or CanvasKeyboardDirection.Right
                    ? Math.Abs(dy)
                    : Math.Abs(dx);
                // 显著惩罚偏离方向轴的候选，避免“只略微在下方、却远在左侧”的节点抢焦点。
                var score = primary + (secondary * 2.0);
                return new { node.Id, Primary = primary, Secondary = secondary, Score = score };
            })
            .Where(candidate => candidate.Primary > 0.5)
            .OrderBy(candidate => candidate.Score)
            .ThenBy(candidate => candidate.Secondary)
            .ThenBy(candidate => candidate.Id, StringComparer.Ordinal)
            .Select(candidate => candidate.Id)
            .FirstOrDefault();
    }
}

public enum CanvasKeyboardDirection
{
    Left,
    Right,
    Up,
    Down,
}

public readonly record struct CanvasKeyboardNode(
    string Id,
    double X,
    double Y,
    double Width,
    double Height);

/// <summary>W13：按工作区实际宽度约束右栏并切换执行区断点。</summary>
public static class WorkspaceResponsiveLayoutHelpers
{
    public const double MinimumCanvasWidth = 520;
    public const double MinimumRightPanelWidth = 260;
    public const double MaximumRightPanelWidth = 560;
    public const double RightPanelSplitterWidth = 4;
    public const double ExecutionStackBreakpoint = 720;
    public const double MaximumOverlayWidth = 360;
    public const double OverlayHorizontalInset = 48;

    public static WorkspaceResponsiveLayout Compute(
        double availableWidth,
        double requestedRightPanelWidth,
        bool isRightPanelOpen)
    {
        var width = double.IsFinite(availableWidth) && availableWidth > 0
            ? availableWidth
            : double.PositiveInfinity;
        var requested = NormalizeRequestedRightPanelWidth(requestedRightPanelWidth);
        var useOverlay = width < MinimumCanvasWidth
                         + MinimumRightPanelWidth
                         + RightPanelSplitterWidth;
        var maxDockedWidth = useOverlay
            ? 0
            : Math.Clamp(
                width - MinimumCanvasWidth - RightPanelSplitterWidth,
                MinimumRightPanelWidth,
                MaximumRightPanelWidth);
        var dockedWidth = isRightPanelOpen && !useOverlay
            ? Math.Min(requested, maxDockedWidth)
            : 0;
        var overlayWidth = double.IsPositiveInfinity(width)
            ? MaximumOverlayWidth
            : Math.Clamp(
                Math.Max(1, width - OverlayHorizontalInset),
                MinimumRightPanelWidth,
                MaximumOverlayWidth);
        return new WorkspaceResponsiveLayout(
            UseOverlayRightPanel: useOverlay,
            DockedRightPanelWidth: dockedWidth,
            MaximumDockedRightPanelWidth: maxDockedWidth,
            OverlayRightPanelWidth: overlayWidth);
    }

    public static double NormalizeRequestedRightPanelWidth(double width) =>
        Math.Clamp(
            double.IsFinite(width) ? width : 360,
            MinimumRightPanelWidth,
            MaximumRightPanelWidth);

    public static bool UseStackedExecutionLayout(double primaryPaneWidth) =>
        double.IsFinite(primaryPaneWidth)
        && primaryPaneWidth > 0
        && primaryPaneWidth < ExecutionStackBreakpoint;
}

public readonly record struct WorkspaceResponsiveLayout(
    bool UseOverlayRightPanel,
    double DockedRightPanelWidth,
    double MaximumDockedRightPanelWidth,
    double OverlayRightPanelWidth);

/// <summary>
/// W8：运行控制可执行矩阵 — 按生命周期，而非「有 run id 就全亮」。
/// </summary>
public static class CanvasRunControlHelpers
{
    public static bool CanPause(string? status)
    {
        var s = Normalize(status);
        return s is "running" or "queued" or "starting";
    }

    public static bool CanResume(string? status)
    {
        var s = Normalize(status);
        return s is "paused";
    }

    public static bool CanStop(string? status)
    {
        var s = Normalize(status);
        return s is "running" or "queued" or "starting" or "paused" or "waiting_confirmation";
    }

    public static bool IsTerminal(string? status)
    {
        var s = Normalize(status);
        return s is "stopped" or "succeeded" or "failed" or "cancelled" or "";
    }

    private static string Normalize(string? status) =>
        (status ?? string.Empty).Trim().ToLowerInvariant();
}
