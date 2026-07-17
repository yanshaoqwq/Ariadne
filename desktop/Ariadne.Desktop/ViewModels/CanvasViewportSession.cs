namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 画布视口会话：统一持有缩放、偏移和平移手势状态。
/// View 只把 Current 投影到控件变换，不再从 RenderTransform 反推业务状态。
/// </summary>
public sealed class CanvasViewportSession
{
    private double _zoom = 1.0;
    private double _offsetX;
    private double _offsetY;
    private double _panStartX;
    private double _panStartY;
    private double _panOriginX;
    private double _panOriginY;

    public CanvasViewportState Current => new(_zoom, _offsetX, _offsetY);

    public double Zoom => _zoom;

    public double OffsetX => _offsetX;

    public double OffsetY => _offsetY;

    public bool IsPanning { get; private set; }

    public CanvasViewportState SetZoom(double value)
    {
        _zoom = NormalizeZoom(value);
        return Current;
    }

    public CanvasViewportState SetOffset(double offsetX, double offsetY)
    {
        _offsetX = offsetX;
        _offsetY = offsetY;
        return Current;
    }

    public CanvasViewportState ZoomAt(double requestedZoom, double anchorX, double anchorY)
    {
        var nextZoom = NormalizeZoom(requestedZoom);
        if (Math.Abs(nextZoom - _zoom) < 1e-9)
        {
            return Current;
        }

        var (nextOffsetX, nextOffsetY) = CanvasViewportHelpers.ComputeAnchoredZoomOffset(
            _zoom,
            nextZoom,
            _offsetX,
            _offsetY,
            anchorX,
            anchorY);
        _zoom = nextZoom;
        _offsetX = nextOffsetX;
        _offsetY = nextOffsetY;
        return Current;
    }

    public CanvasViewportState Fit(
        double minX,
        double minY,
        double maxX,
        double maxY,
        CanvasViewportRect safeViewport)
    {
        var (zoom, offsetX, offsetY) = CanvasViewportHelpers.ComputeFitTransform(
            minX,
            minY,
            maxX,
            maxY,
            safeViewport);
        _zoom = NormalizeZoom(zoom);
        _offsetX = offsetX;
        _offsetY = offsetY;
        return Current;
    }

    public void BeginPan(double screenX, double screenY)
    {
        IsPanning = true;
        _panStartX = screenX;
        _panStartY = screenY;
        _panOriginX = _offsetX;
        _panOriginY = _offsetY;
    }

    public CanvasViewportState UpdatePan(double screenX, double screenY)
    {
        if (!IsPanning)
        {
            return Current;
        }

        var (offsetX, offsetY) = CanvasViewportHelpers.ApplyPan(
            _panOriginX,
            _panOriginY,
            screenX - _panStartX,
            screenY - _panStartY);
        _offsetX = offsetX;
        _offsetY = offsetY;
        return Current;
    }

    public void EndPan()
    {
        IsPanning = false;
    }

    public (double X, double Y) ToLogical(double screenX, double screenY)
    {
        return (
            (screenX - _offsetX) / _zoom,
            (screenY - _offsetY) / _zoom);
    }

    private static double NormalizeZoom(double value) => Math.Clamp(
        Math.Round(value, 2),
        CanvasViewportHelpers.MinZoom,
        CanvasViewportHelpers.MaxZoom);
}

public readonly record struct CanvasViewportState(double Zoom, double OffsetX, double OffsetY);
