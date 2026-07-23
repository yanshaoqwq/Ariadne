using System.Diagnostics;
using System.Runtime.InteropServices;
using Avalonia.Media;
using Avalonia.Threading;

namespace Ariadne.Desktop;

/// <summary>
/// 把当前个性化色图标写到各平台用户目录，供桌面快捷方式 / Dock / 开始菜单使用。
/// - Linux: ~/.local/share/icons/hicolor/*/apps/ariadne.png + 缓存刷新
/// - Windows: %LocalAppData%\Ariadne\icons\
/// - macOS: ~/Library/Application Support/Ariadne/icons/ + ariadne.icns；
///          若存在 ~/Applications/Ariadne.app 则更新其 AppIcon.icns
/// </summary>
public static class AppIconDesktopSync
{
    private static readonly int[] PngSizes = { 16, 24, 32, 48, 64, 128, 256, 512 };
    private static readonly LatestIntentGate<IconSyncRequest> SyncGate = new();

    public static void QueueSync()
    {
        Color accent;
        try
        {
            accent = AppIconPainter.ResolveColor("Ariadne.AccentPrimary", Color.FromRgb(0x35, 0x6F, 0x68));
        }
        catch
        {
            return;
        }

        if (!SyncGate.Enqueue(new IconSyncRequest(accent)))
        {
            return;
        }

        Dispatcher.UIThread.Post(ProcessPendingSync, DispatcherPriority.Background);
    }

    private static void ProcessPendingSync()
    {
        if (!SyncGate.TryTake(out var request, out var generation))
        {
            return;
        }

        IReadOnlyList<RenderedIconPayload> payloads;
        try
        {
            // Avalonia 位图在 UI 线程生成并编码；文件系统和平台缓存不阻塞 UI。
            payloads = RenderIconPayloadsOnUiThread(request.Accent);
        }
        catch
        {
            FinishPendingSync(generation);
            return;
        }

        _ = Task.Run(() => WriteRenderedIconPayloads(payloads))
            .ContinueWith(
                task =>
                {
                    _ = task.Exception;
                    Dispatcher.UIThread.Post(
                        () => FinishPendingSync(generation),
                        DispatcherPriority.Background);
                },
                CancellationToken.None,
                TaskContinuationOptions.ExecuteSynchronously,
                TaskScheduler.Default);
    }

    private static void FinishPendingSync(long generation)
    {
        if (SyncGate.Complete(generation))
        {
            return;
        }

        ProcessPendingSync();
    }

    private readonly record struct IconSyncRequest(Color Accent);

    private readonly record struct RenderedIconPayload(string Path, int Size, byte[] Png);

    private static IReadOnlyList<RenderedIconPayload> RenderIconPayloadsOnUiThread(Color accent)
    {
        var payloads = new List<RenderedIconPayload>();
        var pngBySize = new Dictionary<int, byte[]>();
        foreach (var path in EnumerateOutputPngPaths())
        {
            try
            {
                var size = GuessSizeFromPath(path) ?? 512;
                if (!pngBySize.TryGetValue(size, out var png))
                {
                    using var bmp = AppIconPainter.RenderLineBitmap(
                        accent,
                        Color.FromArgb(0, 0, 0, 0),
                        size,
                        transparentPaper: true);
                    using var stream = new MemoryStream();
                    bmp.Save(stream);
                    png = stream.ToArray();
                    pngBySize[size] = png;
                }

                payloads.Add(new RenderedIconPayload(path, size, png));
            }
            catch
            {
                // 单尺寸失败继续
            }
        }

        return payloads;
    }

    private static void WriteRenderedIconPayloads(IReadOnlyList<RenderedIconPayload> payloads)
    {
        var sizeToPath = new Dictionary<int, string>();
        foreach (var payload in payloads)
        {
            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(payload.Path)!);
                File.WriteAllBytes(payload.Path, payload.Png);
                sizeToPath[payload.Size] = payload.Path;
            }
            catch
            {
                // 单尺寸失败继续
            }
        }

        if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
        {
            try
            {
                var icns = TryBuildMacIcns(sizeToPath);
                if (icns is not null)
                {
                    TryUpdateMacAppBundleIcon(icns);
                }
            }
            catch
            {
                // ignore
            }
        }

        RefreshOsIconCache();
    }

    internal sealed class LatestIntentGate<T>
    {
        private readonly object _gate = new();
        private T _pending = default!;
        private long _generation;
        private bool _hasPending;
        private bool _scheduled;

        public bool Enqueue(T value)
        {
            lock (_gate)
            {
                _pending = value;
                _generation++;
                _hasPending = true;
                if (_scheduled)
                {
                    return false;
                }

                _scheduled = true;
                return true;
            }
        }

        public bool TryTake(out T value, out long generation)
        {
            lock (_gate)
            {
                if (!_hasPending)
                {
                    value = default!;
                    generation = 0;
                    return false;
                }

                value = _pending;
                generation = _generation;
                _hasPending = false;
                return true;
            }
        }

        public bool Complete(long generation)
        {
            lock (_gate)
            {
                if (_hasPending && _generation > generation)
                {
                    return false;
                }

                _scheduled = false;
                return true;
            }
        }
    }

    /// <summary>须在 UI 线程调用（Avalonia 渲染）。</summary>
    public static IReadOnlyList<string> WriteIconsOnUiThread(Color accent, Color paper)
    {
        var written = new List<string>();
        var sizeToPath = new Dictionary<int, string>();

        foreach (var path in EnumerateOutputPngPaths())
        {
            try
            {
                var size = GuessSizeFromPath(path) ?? 512;
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                // 桌面 / 开始菜单 / hicolor：线描（大尺寸清晰）
                // 运行中任务栏窗口图标另走 CreateWindowIcon 实心母版
                using var bmp = AppIconPainter.RenderLineBitmap(
                    accent,
                    Color.FromArgb(0, 0, 0, 0),
                    size,
                    transparentPaper: true);
                bmp.Save(path);
                written.Add(path);
                sizeToPath[size] = path;
            }
            catch
            {
                // 单尺寸失败继续
            }
        }

        // macOS：用 iconutil 打 icns（若工具可用）
        if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
        {
            try
            {
                var icns = TryBuildMacIcns(sizeToPath);
                if (icns is not null)
                {
                    written.Add(icns);
                    TryUpdateMacAppBundleIcon(icns);
                }
            }
            catch
            {
                // ignore
            }
        }

        RefreshOsIconCache();
        return written;
    }

    /// <summary>各平台应写入的 PNG 路径（可单测）。</summary>
    public static IReadOnlyList<string> EnumerateOutputPngPaths()
    {
        var list = new List<string>();

        // —— Linux hicolor ——
        if (RuntimeInformation.IsOSPlatform(OSPlatform.Linux)
            || RuntimeInformation.IsOSPlatform(OSPlatform.FreeBSD))
        {
            var hicolor = GetLinuxHicolorRoot();
            foreach (var s in PngSizes)
            {
                list.Add(Path.Combine(hicolor, $"{s}x{s}", "apps", "ariadne.png"));
            }
        }

        // —— 跨平台用户数据：Win LocalAppData / Linux ~/.local/share / macOS ~/Library/Application Support ——
        var appIcons = GetCrossPlatformIconsDir();
        foreach (var s in PngSizes)
        {
            list.Add(Path.Combine(appIcons, $"ariadne-{s}.png"));
        }

        list.Add(Path.Combine(appIcons, "ariadne.png")); // 主 512 别名

        // —— macOS 专用：显式 Library/Application Support（与 LocalApplicationData 一致，但路径可测）——
        if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
        {
            var macDir = GetMacApplicationSupportIconsDir();
            foreach (var s in PngSizes)
            {
                list.Add(Path.Combine(macDir, $"ariadne-{s}.png"));
            }

            list.Add(Path.Combine(macDir, "ariadne.png"));
        }

        return list.Distinct(StringComparer.Ordinal).ToList();
    }

    public static string GetCrossPlatformIconsDir()
    {
        var data = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
        if (string.IsNullOrWhiteSpace(data))
        {
            data = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                ".local", "share");
        }

        return Path.Combine(data, "Ariadne", "icons");
    }

    /// <summary>~/Library/Application Support/Ariadne/icons（macOS 规范路径；其它平台也可拼出便于测试）。</summary>
    public static string GetMacApplicationSupportIconsDir()
    {
        var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        return Path.Combine(home, "Library", "Application Support", "Ariadne", "icons");
    }

    public static string GetLinuxHicolorRoot()
    {
        var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        return Path.Combine(home, ".local", "share", "icons", "hicolor");
    }

    public static string GetLinuxPrimaryIconPath() =>
        Path.Combine(GetLinuxHicolorRoot(), "512x512", "apps", "ariadne.png");

    public static string GetMacIcnsPath() =>
        Path.Combine(GetMacApplicationSupportIconsDir(), "ariadne.icns");

    /// <summary>用户可选的启动器 .app（若存在则更新图标）。</summary>
    public static string GetMacUserAppBundleIconPath()
    {
        var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        return Path.Combine(home, "Applications", "Ariadne.app", "Contents", "Resources", "AppIcon.icns");
    }

    public static string? GetWindowsIcoPath()
    {
        if (!RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
        {
            return null;
        }

        return Path.Combine(GetCrossPlatformIconsDir(), "ariadne.ico");
    }

    /// <summary>从已写 PNG 生成 .icns（需要 macOS 自带 iconutil / sips）。</summary>
    private static string? TryBuildMacIcns(IReadOnlyDictionary<int, string> sizeToPath)
    {
        var iconsDir = GetMacApplicationSupportIconsDir();
        Directory.CreateDirectory(iconsDir);
        var iconset = Path.Combine(iconsDir, "Ariadne.iconset");
        if (Directory.Exists(iconset))
        {
            Directory.Delete(iconset, recursive: true);
        }

        Directory.CreateDirectory(iconset);

        // iconutil 约定文件名
        void CopySize(int size, string fileName)
        {
            if (!sizeToPath.TryGetValue(size, out var src) || !File.Exists(src))
            {
                // 回退：用最接近的更大尺寸
                src = sizeToPath.OrderBy(kv => Math.Abs(kv.Key - size)).FirstOrDefault().Value;
            }

            if (string.IsNullOrEmpty(src) || !File.Exists(src))
            {
                return;
            }

            var dest = Path.Combine(iconset, fileName);
            // sips 可缩放；若无 sips 则直接复制
            if (!TryRun("sips", $"-z {size} {size} \"{src}\" --out \"{dest}\""))
            {
                File.Copy(src, dest, overwrite: true);
            }
        }

        CopySize(16, "icon_16x16.png");
        CopySize(32, "icon_16x16@2x.png");
        CopySize(32, "icon_32x32.png");
        CopySize(64, "icon_32x32@2x.png");
        CopySize(128, "icon_128x128.png");
        CopySize(256, "icon_128x128@2x.png");
        CopySize(256, "icon_256x256.png");
        CopySize(512, "icon_256x256@2x.png");
        CopySize(512, "icon_512x512.png");
        CopySize(512, "icon_512x512@2x.png"); // 无 1024 源时用 512

        var icns = GetMacIcnsPath();
        if (!TryRun("iconutil", $"-c icns \"{iconset}\" -o \"{icns}\""))
        {
            return null;
        }

        try
        {
            Directory.Delete(iconset, recursive: true);
        }
        catch
        {
            // 保留 iconset 也无妨
        }

        return File.Exists(icns) ? icns : null;
    }

    private static void TryUpdateMacAppBundleIcon(string icnsPath)
    {
        var bundleIcon = GetMacUserAppBundleIconPath();
        var resources = Path.GetDirectoryName(bundleIcon);
        if (string.IsNullOrEmpty(resources) || !Directory.Exists(Path.GetDirectoryName(resources)))
        {
            // ~/Applications/Ariadne.app 不存在则跳过（安装器可创建）
            return;
        }

        try
        {
            Directory.CreateDirectory(resources);
            File.Copy(icnsPath, bundleIcon, overwrite: true);
            // 触碰 bundle 促使 LaunchServices / Dock 刷新
            var appRoot = Path.GetFullPath(Path.Combine(resources, "..", ".."));
            TryRun("touch", $"\"{appRoot}\"");
            // 不 killall Dock（扰民）；用户重开 Dock 项或注销后会更新
        }
        catch
        {
            // ignore
        }
    }

    public static int? GuessSizeFromPath(string path)
    {
        var name = Path.GetFileNameWithoutExtension(path);
        if (name.StartsWith("ariadne-", StringComparison.OrdinalIgnoreCase)
            && int.TryParse(name.AsSpan("ariadne-".Length), out var s1))
        {
            return s1;
        }

        // .../256x256/apps/ariadne.png
        var appsDir = Path.GetDirectoryName(path);
        var sizeFolder = Path.GetFileName(Path.GetDirectoryName(appsDir));
        if (sizeFolder is not null && sizeFolder.Contains('x', StringComparison.OrdinalIgnoreCase))
        {
            var part = sizeFolder.Split('x', 'X')[0];
            if (int.TryParse(part, out var s2))
            {
                return s2;
            }
        }

        if (string.Equals(name, "ariadne", StringComparison.OrdinalIgnoreCase))
        {
            return 512;
        }

        return null;
    }

    private static void RefreshOsIconCache()
    {
        try
        {
            if (RuntimeInformation.IsOSPlatform(OSPlatform.Linux))
            {
                var hicolor = GetLinuxHicolorRoot();
                TryRun("gtk-update-icon-cache", $"-f -t \"{hicolor}\"");
                TryRun("xdg-desktop-menu", "forceupdate");
                TryRun("kbuildsycoca5", "");
                TryRun("kbuildsycoca6", "");
            }
            else if (RuntimeInformation.IsOSPlatform(OSPlatform.OSX))
            {
                // 通知 Launch Services 刷新（若存在）
                var icns = GetMacIcnsPath();
                if (File.Exists(icns))
                {
                    TryRun("touch", $"\"{icns}\"");
                }

                TryRun("/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
                    $"-f \"{GetMacApplicationSupportIconsDir()}\"");
            }
        }
        catch
        {
            // ignore
        }
    }

    private static bool TryRun(string fileName, string args)
    {
        try
        {
            using var p = Process.Start(new ProcessStartInfo
            {
                FileName = fileName,
                Arguments = args,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                CreateNoWindow = true,
            });
            if (p is null)
            {
                return false;
            }

            p.WaitForExit(8000);
            return p.ExitCode == 0;
        }
        catch
        {
            return false;
        }
    }
}
