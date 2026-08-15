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
    /// 安全可视区在任一方向上不得被浮层裁到比这更窄，否则「避让」会把画布削成一条缝。
    /// 宁可保留一点重叠：重叠还能靠平移绕开，可视区没了就无路可走。
    /// </summary>
    public const double MinimumSafeSpan = 120;

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
    /// U141：让**视口**追上节点，而不是把节点拽回视口。
    ///
    /// 旧版 <c>KeepNodeReachable</c> 把「可达性」实现成对逻辑坐标的钳位再反算，
    /// 于是节点的 X/Y 会被改写成视口边缘值并随工作流存盘——改的是用户数据，
    /// 画布也因此退化成「一屏 + 缩放」。这里改为只动 offset：
    /// 逻辑坐标是内容，offset 是显示，两者不能互相污染。
    ///
    /// 返回让节点整块落进 <paramref name="safeViewport"/> 所需的新 offset。
    /// 节点已经可见时原样返回，避免每帧抖动。
    /// </summary>
    public static (double OffsetX, double OffsetY) EnsureNodeVisibleOffset(
        double logicalX,
        double logicalY,
        double nodeWidth,
        double nodeHeight,
        double zoom,
        double offsetX,
        double offsetY,
        CanvasViewportRect safeViewport)
    {
        var safeZoom = Math.Max(MinZoom, zoom);
        var safe = safeViewport.Normalize();
        if (safe.Width <= 0 || safe.Height <= 0)
        {
            return (offsetX, offsetY);
        }

        var screenWidth = Math.Max(1, nodeWidth * safeZoom);
        var screenHeight = Math.Max(1, nodeHeight * safeZoom);
        return (
            AxisOffsetToReveal(logicalX, screenWidth, safeZoom, offsetX, safe.X, safe.Width),
            AxisOffsetToReveal(logicalY, screenHeight, safeZoom, offsetY, safe.Y, safe.Height));
    }

    /// <summary>
    /// 单轴求解：只在节点越界的那一侧补足位移，且节点比可视区还长时对齐前缘
    /// （否则「两端都要进来」这个约束无解，会来回抖）。
    /// </summary>
    private static double AxisOffsetToReveal(
        double logical,
        double screenLength,
        double zoom,
        double offset,
        double safeStart,
        double safeLength)
    {
        var start = (logical * zoom) + offset;
        var end = start + screenLength;
        if (screenLength >= safeLength)
        {
            return safeStart - (logical * zoom);
        }

        if (start < safeStart)
        {
            return offset + (safeStart - start);
        }

        if (end > safeStart + safeLength)
        {
            return offset - (end - (safeStart + safeLength));
        }

        return offset;
    }

    /// <summary>
    /// U141：把浮层（工具条、小地图、右栏…）从视口里挖掉，得到「点得到」的安全矩形。
    ///
    /// 这是防遮挡的**正确落点**：只影响 Fit / 追随视口这类显示决策，
    /// 一个字节的用户数据都不会被改。挖到过窄就整块放弃避让，只留边距内缩
    /// （见 <see cref="MinimumSafeSpan"/>）。
    /// </summary>
    public static CanvasViewportRect ComputeSafeViewport(
        double viewportWidth,
        double viewportHeight,
        IReadOnlyList<CanvasViewportRect> occlusions,
        double inset = 12)
    {
        var width = Math.Max(1, viewportWidth);
        var height = Math.Max(1, viewportHeight);
        var left = Math.Min(inset, width * 0.25);
        var top = Math.Min(inset, height * 0.25);
        var right = Math.Max(left + 1, width - inset);
        var bottom = Math.Max(top + 1, height - inset);

        foreach (var raw in occlusions)
        {
            var blocker = raw.Normalize();
            if (blocker.Width <= 0 || blocker.Height <= 0)
            {
                continue;
            }

            // 只从最近的一侧切：浮层贴着哪条边，就把那条边推进来。
            // 四个 cut 里有非正数说明浮层已经在矩形之外（含右栏停靠在画布右侧
            // 之外的情形），直接跳过——否则会白挖掉一块可视区。
            var cutLeft = blocker.Right - left;
            var cutRight = right - blocker.X;
            var cutTop = blocker.Bottom - top;
            var cutBottom = bottom - blocker.Y;
            if (cutLeft <= 0 || cutRight <= 0 || cutTop <= 0 || cutBottom <= 0)
            {
                continue;
            }

            // 取代价最小的一侧。每侧都以 MinimumSafeSpan 兜底：宁可与浮层留一点重叠，
            // 也不把可视区削成一条缝——重叠还能靠平移绕开，可视区没了就无路可走。
            var minimumCut = Math.Min(Math.Min(cutLeft, cutRight), Math.Min(cutTop, cutBottom));
            if (Math.Abs(minimumCut - cutLeft) < 1e-9)
            {
                left = Math.Min(left + cutLeft + inset, right - MinimumSafeSpan);
            }
            else if (Math.Abs(minimumCut - cutRight) < 1e-9)
            {
                right = Math.Max(right - cutRight - inset, left + MinimumSafeSpan);
            }
            else if (Math.Abs(minimumCut - cutTop) < 1e-9)
            {
                top = Math.Min(top + cutTop + inset, bottom - MinimumSafeSpan);
            }
            else
            {
                bottom = Math.Max(bottom - cutBottom - inset, top + MinimumSafeSpan);
            }
        }

        if (right - left < MinimumSafeSpan || bottom - top < MinimumSafeSpan)
        {
            return new CanvasViewportRect(
                Math.Min(inset, width * 0.25),
                Math.Min(inset, height * 0.25),
                Math.Max(1, width - (inset * 2)),
                Math.Max(1, height - (inset * 2)));
        }

        return new CanvasViewportRect(left, top, right - left, bottom - top);
    }

    /// <summary>
    /// U141：内容层尺寸不能等于视口尺寸——那是「一屏画布」的根源：层只有一屏大，
    /// 层外的节点既不参与测量，也可能因宿主/主题的裁剪而消失。
    ///
    /// 这里按「图包围盒 ∪ 当前可见逻辑区域」再留一屏余量取尺寸，于是层始终比
    /// 内容大一圈，节点拖到视口外仍在层内。
    ///
    /// **负半轴不靠尺寸覆盖**：层的局部原点就是逻辑 0，负坐标节点落在层矩形之外，
    /// 靠内容层 <c>ClipToBounds=False</c> 正常渲染与命中（真正该裁的是外层视口
    /// CanvasHost，那才是「屏幕边界」）。想用尺寸覆盖负半轴就得平移层原点，
    /// 那会连带改写所有边 Geometry 的坐标基准，代价远大于收益。
    /// </summary>
    public static (double Width, double Height) ComputeContentLayerSize(
        double maxX,
        double maxY,
        double viewportWidth,
        double viewportHeight,
        double zoom,
        double offsetX,
        double offsetY,
        double margin = 1200)
    {
        var safeZoom = Math.Max(MinZoom, zoom);
        var viewW = Math.Max(1, viewportWidth) / safeZoom;
        var viewH = Math.Max(1, viewportHeight) / safeZoom;
        // 可见逻辑区域的右/下边界：screen = logical * zoom + offset 的反解。
        var visibleRight = (-offsetX / safeZoom) + viewW;
        var visibleBottom = (-offsetY / safeZoom) + viewH;
        var right = Math.Max(
            double.IsFinite(maxX) ? maxX : visibleRight,
            visibleRight);
        var bottom = Math.Max(
            double.IsFinite(maxY) ? maxY : visibleBottom,
            visibleBottom);
        return (
            Math.Max(1, right + margin),
            Math.Max(1, bottom + margin));
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
