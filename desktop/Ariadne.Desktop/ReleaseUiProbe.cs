using System.Diagnostics;
using System.Reflection;
using System.Text.Json;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Media.Imaging;
using Avalonia.Threading;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;

namespace Ariadne.Desktop;

internal sealed record UiPerformanceSample(
    int NodeCount,
    int FrameCount,
    double InitialLayoutMs,
    double P95FrameIntervalMs,
    double P95FrameWorkMs,
    long P95AllocatedBytes);

internal sealed record UiPerformanceEvidence(
    int SchemaVersion,
    string Probe,
    string BuildProfile,
    IReadOnlyList<UiPerformanceSample> Samples);

internal static class ReleaseUiProbe
{
    private const string PreviewModeEnvironmentVariable = "ARIADNE_RELEASE_UI_PREVIEW";
    private const int FramesPerNodeCount = 120;
    private const int WarmupFramesPerNodeCount = 12;
    private static readonly TimeSpan InitialInteractionWarmup = TimeSpan.FromSeconds(30);
    private static readonly TimeSpan FrameCallbackTimeout = TimeSpan.FromSeconds(30);
#if DEBUG
    private const string BuildProfile = "debug";
#else
    private const string BuildProfile = "release";
#endif

    public static bool TryStart(IClassicDesktopStyleApplicationLifetime desktop)
    {
        var args = desktop.Args ?? Array.Empty<string>();
        if (args.Length >= 1 && string.Equals(args[0], "--canvas-shot", StringComparison.Ordinal))
        {
            StartCanvasShot(desktop, args);
            return true;
        }

        if (args.Length != 2 || !string.Equals(args[0], "--release-ui-probe", StringComparison.Ordinal))
        {
            return false;
        }

        var backend = DispatchProxy.Create<IAriadneBackendClient, NullBackendProxy>();
        var viewModel = new WorkspacePageViewModel(DisplayNameService.Current, backend);
        var view = new WorkspacePageView { DataContext = viewModel };
        var window = new Window
        {
            Width = 1600,
            Height = 900,
            Content = view,
        };
        window.Opened += async (_, _) =>
        {
            var exitCode = 0;
            try
            {
                await RunAsync(window, view, viewModel, args[1]).ConfigureAwait(true);
            }
            catch (Exception error)
            {
                exitCode = 1;
                var path = Path.GetFullPath(args[1]);
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                await File.WriteAllTextAsync(path, JsonSerializer.Serialize(new
                {
                    schema_version = 1,
                    probe = "desktop_ui_performance",
                    error = error.GetType().Name,
                    error_stage = ReadProgressStage(args[1]),
                    error_message = error.Message,
                })).ConfigureAwait(true);
            }
            finally
            {
                Environment.Exit(exitCode);
            }
        };
        desktop.MainWindow = window;
        return true;
    }

    private static async Task RunAsync(
        Window window,
        WorkspacePageView view,
        WorkspacePageViewModel viewModel,
        string outputPath)
    {
        if (!string.Equals(BuildProfile, "release", StringComparison.Ordinal))
        {
            throw new InvalidOperationException(
                "desktop UI release evidence must be generated from a Release build");
        }
        WriteProgress(outputPath, "opened");
        var topLevel = TopLevel.GetTopLevel(window)
            ?? throw new InvalidOperationException("release UI probe window has no top level");
        var samples = new List<UiPerformanceSample>();
        foreach (var nodeCount in new[] { 100, 500, 1000 })
        {
            WriteProgress(outputPath, $"building-{nodeCount}");
            viewModel.LoadReleaseProbeGraph(BuildGraph(nodeCount));
            ApplyPreviewState(viewModel);
            WriteProgress(outputPath, $"loaded-{nodeCount}");
            view.PrepareReleaseProbe();
            WriteProgress(outputPath, $"prepared-{nodeCount}");
            var layoutStarted = Stopwatch.GetTimestamp();
            await NextFrameAsync(topLevel, TimeSpan.FromMinutes(3)).ConfigureAwait(true);
            await NextFrameAsync(topLevel, TimeSpan.FromMinutes(3)).ConfigureAwait(true);
            var initialLayoutMs = (Stopwatch.GetTimestamp() - layoutStarted) * 1000.0 / Stopwatch.Frequency;
            WriteProgress(outputPath, $"framing-{nodeCount}");

            var node = viewModel.Nodes[nodeCount / 2];
            var frameIntervals = new List<double>(FramesPerNodeCount);
            var frameWork = new List<double>(FramesPerNodeCount);
            var allocations = new List<long>(FramesPerNodeCount);
            viewModel.BeginContinuousCanvasEdit();
            try
            {
                WriteProgress(outputPath, $"warming-{nodeCount}");
                var warmupTimestamp = Stopwatch.GetTimestamp();
                var warmupStarted = warmupTimestamp;
                var warmupFrame = 0;
                do
                {
                    var warmup = await MeasureFrameAsync(
                            topLevel,
                            view,
                            node,
                            warmupFrame,
                            warmupTimestamp)
                        .ConfigureAwait(true);
                    warmupTimestamp = warmup.Timestamp;
                    warmupFrame++;
                }
                while (warmupFrame < WarmupFramesPerNodeCount
                       || (samples.Count == 0
                           && Stopwatch.GetElapsedTime(warmupStarted) < InitialInteractionWarmup));

                WriteProgress(outputPath, $"framing-{nodeCount}");
                var previousTimestamp = Stopwatch.GetTimestamp();
                for (var frame = 0; frame < FramesPerNodeCount; frame++)
                {
                    FrameSample sample;
                    try
                    {
                        sample = await MeasureFrameAsync(topLevel, view, node, frame, previousTimestamp)
                            .ConfigureAwait(true);
                    }
                    catch (TimeoutException error)
                    {
                        throw new TimeoutException(
                            $"animation frame callback timed out at node_count={nodeCount}, frame={frame}",
                            error);
                    }
                    previousTimestamp = sample.Timestamp;
                    if (frame > 0)
                    {
                        frameIntervals.Add(sample.IntervalMs);
                    }
                    frameWork.Add(sample.WorkMs);
                    allocations.Add(sample.AllocatedBytes);
                    if ((frame + 1) % 20 == 0)
                    {
                        WriteProgress(outputPath, $"framing-{nodeCount}-{frame + 1}");
                    }
                }
            }
            finally
            {
                view.CompleteReleaseProbeDrag(node);
                viewModel.EndContinuousCanvasEdit();
            }

            samples.Add(new UiPerformanceSample(
                nodeCount,
                FramesPerNodeCount,
                initialLayoutMs,
                Percentile(frameIntervals, 0.95),
                Percentile(frameWork, 0.95),
                (long)Math.Round(Percentile(allocations.Select(value => (double)value).ToArray(), 0.95))));
            WriteProgress(outputPath, $"completed-{nodeCount}");
        }

        var fullPath = Path.GetFullPath(outputPath);
        Directory.CreateDirectory(Path.GetDirectoryName(fullPath)!);
        await File.WriteAllTextAsync(fullPath, JsonSerializer.Serialize(
            new UiPerformanceEvidence(1, "desktop_ui_performance", BuildProfile, samples),
            new JsonSerializerOptions
            {
                PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
                WriteIndented = true,
            })).ConfigureAwait(true);
        WriteProgress(outputPath, "completed");
    }

    private static void ApplyPreviewState(WorkspacePageViewModel viewModel)
    {
        var mode = Environment.GetEnvironmentVariable(PreviewModeEnvironmentVariable);
        if (string.IsNullOrWhiteSpace(mode) || viewModel.Nodes.Count == 0)
        {
            return;
        }

        viewModel.RightPanelColumnWidth = new GridLength(360);
        if (string.Equals(mode, "edge", StringComparison.OrdinalIgnoreCase))
        {
            viewModel.SelectNode(viewModel.Nodes[Math.Min(4, viewModel.Nodes.Count - 1)]);
            var edge = viewModel.RelatedEdges.FirstOrDefault(item => item.IsData)
                       ?? viewModel.RelatedEdges.FirstOrDefault();
            edge?.SelectCommand.TryExecute();
            return;
        }

        viewModel.SelectNode(viewModel.Nodes[Math.Min(1, viewModel.Nodes.Count - 1)]);
        viewModel.ShowNodeDetailsCommand.TryExecute();
    }

    // --canvas-shot <png> [nodeCount]：渲染一张有代表性的节点画布供视觉验收，不参与性能测量。
    private static void StartCanvasShot(IClassicDesktopStyleApplicationLifetime desktop, string[] args)
    {
        var outputPath = args.Length > 1 ? args[1] : "canvas-preview.png";
        var nodeCount = args.Length > 2 && int.TryParse(args[2], out var parsed) && parsed > 0 ? parsed : 12;

        var backend = DispatchProxy.Create<IAriadneBackendClient, NullBackendProxy>();
        var viewModel = new WorkspacePageViewModel(DisplayNameService.Current, backend);
        var view = new WorkspacePageView { DataContext = viewModel };
        var window = new Window
        {
            Width = 1600,
            Height = 900,
            Content = view,
        };
        window.Opened += async (_, _) =>
        {
            var exitCode = 0;
            try
            {
                var topLevel = TopLevel.GetTopLevel(window)
                    ?? throw new InvalidOperationException("canvas shot window has no top level");
                viewModel.LoadReleaseProbeGraph(BuildGraph(nodeCount));
                view.PrepareReleaseProbe();
                // 等一帧让节点容器完成布局，再按视口取景并选中一个节点展示选中环与点阵。
                await NextFrameAsync(topLevel, TimeSpan.FromMinutes(1)).ConfigureAwait(true);
                viewModel.FitViewCommand.TryExecute();
                // 放大到可编辑倍率，露出引脚/运行按钮等精度控件，便于目检卡片细节。
                viewModel.SetCanvasZoom(1.4);
                viewModel.SelectNode(viewModel.Nodes[Math.Min(2, viewModel.Nodes.Count - 1)]);
                await NextFrameAsync(topLevel, TimeSpan.FromMinutes(1)).ConfigureAwait(true);
                await NextFrameAsync(topLevel, TimeSpan.FromMinutes(1)).ConfigureAwait(true);
                CapturePng(view, outputPath);
            }
            catch (Exception)
            {
                exitCode = 1;
            }
            finally
            {
                Environment.Exit(exitCode);
            }
        };
        desktop.MainWindow = window;
    }

    private static void CapturePng(Visual target, string path)
    {
        var size = target.Bounds.Size;
        var pixelSize = new PixelSize(
            Math.Max(1, (int)Math.Ceiling(size.Width)),
            Math.Max(1, (int)Math.Ceiling(size.Height)));
        using var bitmap = new RenderTargetBitmap(pixelSize, new Vector(96, 96));
        bitmap.Render(target);
        var fullPath = Path.GetFullPath(path);
        Directory.CreateDirectory(Path.GetDirectoryName(fullPath)!);
        bitmap.Save(fullPath);
    }

    private static async Task<FrameSample> MeasureFrameAsync(
        TopLevel topLevel,
        WorkspacePageView view,
        WorkflowNodeViewModel node,
        int frame,
        long previousTimestamp)
    {
        var completion = new TaskCompletionSource<FrameSample>(TaskCreationOptions.RunContinuationsAsynchronously);
        topLevel.RequestAnimationFrame(_ =>
        {
            var workStarted = Stopwatch.GetTimestamp();
            var before = GC.GetAllocatedBytesForCurrentThread();
            view.ApplyReleaseProbeDragFrame(node, node.X + 0.5, node.Y + (frame % 3 == 0 ? 0.25 : 0));
            var after = GC.GetAllocatedBytesForCurrentThread();
            var timestamp = Stopwatch.GetTimestamp();
            completion.TrySetResult(new FrameSample(
                timestamp,
                (timestamp - previousTimestamp) * 1000.0 / Stopwatch.Frequency,
                (timestamp - workStarted) * 1000.0 / Stopwatch.Frequency,
                Math.Max(0, after - before)));
        });
        view.RequestReleaseProbeFrame(node);
        // 存活超时只识别无回调；性能合格仍由完整样本的 p95 阈值判定。
        return await completion.Task.WaitAsync(FrameCallbackTimeout).ConfigureAwait(true);
    }

    private static Task<TimeSpan> NextFrameAsync(TopLevel topLevel, TimeSpan timeout)
    {
        var completion = new TaskCompletionSource<TimeSpan>(TaskCreationOptions.RunContinuationsAsynchronously);
        topLevel.RequestAnimationFrame(timestamp => completion.TrySetResult(timestamp));
        topLevel.InvalidateVisual();
        return completion.Task.WaitAsync(timeout);
    }

    private static WorkflowGraphData BuildGraph(int nodeCount)
    {
        var nodes = Enumerable.Range(0, nodeCount)
            .Select(index => new CanvasNode(
                $"probe-{index}",
                index == 0 ? "start" : "llm",
                $"probe-{index}",
                new Dictionary<string, object?>(),
                new CanvasPosition((index % 5) * 300, (index / 5) * 200)))
            .ToArray();
        var edges = new List<CanvasEdge>(nodeCount + nodeCount / 4);
        for (var index = 1; index < nodeCount; index++)
        {
            edges.Add(new CanvasEdge(
                $"probe-edge-{index}",
                $"probe-{index - 1}",
                $"probe-{index}",
                "output",
                "input",
                "control",
                null,
                null));
            if (index >= 4 && index % 4 == 0)
            {
                edges.Add(new CanvasEdge(
                    $"probe-side-edge-{index}",
                    $"probe-{index - 4}",
                    $"probe-{index}",
                    "output",
                    "input",
                    "data",
                    null,
                    null));
            }
        }

        return new WorkflowGraphData(
            "release-ui-probe",
            $"release-ui-probe-{nodeCount}",
            nodes,
            edges,
            new Dictionary<string, object?>(),
            $"probe-{nodeCount}",
            $"probe-{nodeCount}");
    }

    private static double Percentile(IReadOnlyList<double> values, double percentile)
    {
        if (values.Count == 0)
        {
            return 0;
        }

        var sorted = values.OrderBy(value => value).ToArray();
        var index = Math.Clamp((int)Math.Ceiling(sorted.Length * percentile) - 1, 0, sorted.Length - 1);
        return sorted[index];
    }

    private static void WriteProgress(string outputPath, string stage)
    {
        var path = Path.GetFullPath(outputPath) + ".progress";
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, stage);
    }

    private static string ReadProgressStage(string outputPath)
    {
        try
        {
            var path = Path.GetFullPath(outputPath) + ".progress";
            return File.Exists(path) ? File.ReadAllText(path) : "not-started";
        }
        catch
        {
            return "unknown";
        }
    }

    private readonly record struct FrameSample(
        long Timestamp,
        double IntervalMs,
        double WorkMs,
        long AllocatedBytes);

    private class NullBackendProxy : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return false;
            }

            var returnType = targetMethod?.ReturnType;
            if (returnType == typeof(Task))
            {
                return Task.CompletedTask;
            }

            if (returnType is { IsGenericType: true }
                && returnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var valueType = returnType.GetGenericArguments()[0];
                return typeof(Task)
                    .GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(valueType)
                    .Invoke(null, new[] { valueType.IsValueType ? Activator.CreateInstance(valueType) : null });
            }

            return returnType?.IsValueType == true ? Activator.CreateInstance(returnType) : null;
        }
    }
}
