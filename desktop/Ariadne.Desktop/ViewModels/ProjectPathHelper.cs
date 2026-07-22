namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 新建项目路径：在父目录下用项目名建子文件夹，而不是直接写进所选目录根。
/// </summary>
public static class ProjectPathHelper
{
    /// <summary>
    /// 将用户输入的名称规范为合法文件夹名（去首尾空白、替换非法字符、避免空名）。
    /// </summary>
    public static string SanitizeFolderName(string? rawName, string fallback = "new-project")
    {
        var name = (rawName ?? string.Empty).Trim();
        if (string.IsNullOrWhiteSpace(name))
        {
            name = fallback;
        }

        // 统一空白为连字符，去掉路径分隔与保留字符
        var invalid = Path.GetInvalidFileNameChars();
        var chars = name.Select(ch =>
        {
            if (invalid.Contains(ch) || ch is '/' or '\\' or ':' or '*' or '?' or '"' or '<' or '>' or '|')
            {
                return '-';
            }
            return char.IsWhiteSpace(ch) ? '-' : ch;
        }).ToArray();

        var sanitized = new string(chars);
        while (sanitized.Contains("--", StringComparison.Ordinal))
        {
            sanitized = sanitized.Replace("--", "-", StringComparison.Ordinal);
        }
        sanitized = sanitized.Trim('-', '.');
        if (string.IsNullOrWhiteSpace(sanitized))
        {
            sanitized = fallback;
        }

        // Windows 保留设备名
        var reserved = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        {
            "CON", "PRN", "AUX", "NUL",
            "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
            "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        };
        if (reserved.Contains(sanitized))
        {
            sanitized = sanitized + "-project";
        }

        if (sanitized.Length > 80)
        {
            sanitized = sanitized[..80].TrimEnd('-', '.');
        }

        return sanitized;
    }

    /// <summary>
    /// 目录是否已是 Ariadne 项目（含项目身份配置）。用于打开前本地预检，避免把空 .config 误判为项目。
    /// </summary>
    public static bool LooksLikeInitializedProject(string? projectRoot)
    {
        if (string.IsNullOrWhiteSpace(projectRoot))
        {
            return false;
        }

        try
        {
            var root = Path.GetFullPath(projectRoot.Trim());
            if (!Directory.Exists(root))
            {
                return false;
            }

            return File.Exists(Path.Combine(root, ".config", "app.yaml"));
        }
        catch
        {
            return false;
        }
    }

    /// <summary>
    /// 将绝对/混杂路径规范为项目内相对路径（用于文档 id 与打开）。
    /// 优先截取 documents/planning/workflows 段；否则去前导斜杠。
    /// </summary>
    public static string ToProjectRelativePath(string? path)
    {
        if (string.IsNullOrWhiteSpace(path))
        {
            return string.Empty;
        }

        var normalized = path.Trim().Replace('\\', '/');
        foreach (var marker in new[] { "/documents/", "/planning/", "/workflows/" })
        {
            var index = normalized.IndexOf(marker, StringComparison.OrdinalIgnoreCase);
            if (index >= 0)
            {
                return normalized[(index + 1)..];
            }
        }

        // 已是相对：documents/foo.md
        foreach (var prefix in new[] { "documents/", "planning/", "workflows/" })
        {
            if (normalized.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
            {
                return normalized;
            }
        }

        return normalized.TrimStart('/');
    }

    /// <summary>
    /// 若 absoluteOrAny 落在 projectRoot 之下（或即根本身），输出项目相对路径（正斜杠）；
    /// 否则返回 false（勿静默写入绝对路径进 work_dir）。
    /// </summary>
    public static bool TryMakeRelativeToProjectRoot(
        string? absoluteOrAny,
        string? projectRoot,
        out string relativePath)
    {
        relativePath = string.Empty;
        if (string.IsNullOrWhiteSpace(absoluteOrAny) || string.IsNullOrWhiteSpace(projectRoot))
        {
            return false;
        }

        try
        {
            var full = ResolveExistingPrefixes(absoluteOrAny.Trim());
            var root = ResolveExistingPrefixes(projectRoot.Trim());
            // 统一结尾分隔，避免 /proj 与 /project 前缀误匹配
            var rootWithSep = root.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar)
                             + Path.DirectorySeparatorChar;
            if (string.Equals(full, root, StringComparison.OrdinalIgnoreCase))
            {
                relativePath = ".";
                return true;
            }

            if (!full.StartsWith(rootWithSep, StringComparison.OrdinalIgnoreCase))
            {
                return false;
            }

            var rel = full[rootWithSep.Length..];
            relativePath = rel.Replace('\\', '/').TrimEnd('/');
            if (string.IsNullOrWhiteSpace(relativePath))
            {
                relativePath = ".";
            }

            return true;
        }
        catch
        {
            return false;
        }
    }

    private static string ResolveExistingPrefixes(string path)
    {
        var full = Path.GetFullPath(path);
        var root = Path.GetPathRoot(full);
        if (string.IsNullOrWhiteSpace(root))
        {
            return full;
        }

        var current = root;
        var relative = full[root.Length..];
        foreach (var component in relative.Split(
                     new[] { Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar },
                     StringSplitOptions.RemoveEmptyEntries))
        {
            current = Path.Combine(current, component);
            FileSystemInfo? info = Directory.Exists(current)
                ? new DirectoryInfo(current)
                : File.Exists(current)
                    ? new FileInfo(current)
                    : null;
            if (info?.LinkTarget is not null)
            {
                current = info.ResolveLinkTarget(returnFinalTarget: true)?.FullName ?? current;
            }
        }

        return Path.GetFullPath(current);
    }

    /// <summary>
    /// 导出 storage_uri / 文件路径 → 可在文件管理器中打开的目录。
    /// file:// URI、普通路径、不存在的文件均尽量解析到父目录。
    /// </summary>
    public static string? ResolveRevealDirectory(string? storagePathOrUri)
    {
        if (string.IsNullOrWhiteSpace(storagePathOrUri))
        {
            return null;
        }

        var raw = storagePathOrUri.Trim();
        try
        {
            if (raw.StartsWith("file:", StringComparison.OrdinalIgnoreCase))
            {
                if (Uri.TryCreate(raw, UriKind.Absolute, out var uri) && uri.IsFile)
                {
                    raw = uri.LocalPath;
                }
                else
                {
                    raw = raw["file:".Length..].TrimStart('/');
                    // file:///home/... on Unix → /home/...
                    if (!raw.StartsWith('/') && raw.Contains('/'))
                    {
                        raw = "/" + raw;
                    }
                }
            }

            raw = Path.GetFullPath(raw);
            if (Directory.Exists(raw))
            {
                return raw;
            }

            var parent = Path.GetDirectoryName(raw);
            if (!string.IsNullOrWhiteSpace(parent) && Directory.Exists(parent))
            {
                return parent;
            }

            // 文件尚未落盘时仍返回父路径，便于打开导出目录
            return string.IsNullOrWhiteSpace(parent) ? null : parent;
        }
        catch
        {
            return null;
        }
    }

    /// <summary>
    /// 父目录 + 项目名 → 项目根路径。若已存在则追加 _2/_3…
    /// </summary>
    public static string BuildUniqueProjectRoot(string parentDirectory, string projectName)
    {
        if (string.IsNullOrWhiteSpace(parentDirectory))
        {
            throw new ArgumentException("Parent directory is required.", nameof(parentDirectory));
        }

        var folder = SanitizeFolderName(projectName);
        var parent = Path.GetFullPath(parentDirectory.Trim());
        var candidate = Path.Combine(parent, folder);
        if (!Directory.Exists(candidate) && !File.Exists(candidate))
        {
            return candidate;
        }

        for (var i = 2; i < 10000; i++)
        {
            var next = Path.Combine(parent, $"{folder}_{i}");
            if (!Directory.Exists(next) && !File.Exists(next))
            {
                return next;
            }
        }

        return Path.Combine(parent, $"{folder}_{Guid.NewGuid():N}"[..Math.Min(48, folder.Length + 20)]);
    }
}
