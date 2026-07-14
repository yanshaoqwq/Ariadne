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

    /// <summary>W2：指针滚轮缩放（deltaY 正→放大）。</summary>
    public static double ApplyWheelZoom(double currentZoom, double wheelDeltaY, double step = 0.1)
    {
        var next = wheelDeltaY > 0 ? currentZoom + step : currentZoom - step;
        return Math.Clamp(next, MinZoom, MaxZoom);
    }

    /// <summary>W2：平移偏移（屏幕像素）。</summary>
    public static (double OffsetX, double OffsetY) ApplyPan(
        double offsetX,
        double offsetY,
        double deltaX,
        double deltaY) =>
        (offsetX + deltaX, offsetY + deltaY);
}

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
