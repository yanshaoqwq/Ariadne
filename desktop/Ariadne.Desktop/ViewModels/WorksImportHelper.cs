namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 作品页导入：从源文件路径推导章节字段，避免作者手填工程师式 ID/路径。
/// </summary>
public static class WorksImportHelper
{
    /// <summary>
    /// 根据源文件与当前树条目数给出默认章节 id / 标题 / 目标路径 / 排序。
    /// </summary>
    public static ImportFieldSuggestion SuggestFromSourcePath(string? sourcePath, int existingTreeCount)
    {
        var path = (sourcePath ?? string.Empty).Trim();
        var fileName = string.IsNullOrWhiteSpace(path)
            ? "chapter.md"
            : Path.GetFileName(path);
        if (string.IsNullOrWhiteSpace(fileName))
        {
            fileName = "chapter.md";
        }

        var stem = Path.GetFileNameWithoutExtension(fileName);
        if (string.IsNullOrWhiteSpace(stem))
        {
            stem = "chapter";
        }

        var title = stem.Trim();
        var chapterId = SanitizeChapterId(title);
        var target = ("documents/" + fileName).Replace('\\', '/');
        var order = (decimal)Math.Max(0, existingTreeCount);
        return new ImportFieldSuggestion(chapterId, title, target, order);
    }

    /// <summary>
    /// 将展示标题规范为可作 chapter_id 的标识（字母数字下划线、中文保留）。
    /// </summary>
    public static string SanitizeChapterId(string? raw, string fallback = "chapter")
    {
        var name = (raw ?? string.Empty).Trim();
        if (string.IsNullOrWhiteSpace(name))
        {
            name = fallback;
        }

        var chars = name.Select(ch =>
        {
            if (char.IsLetterOrDigit(ch) || ch is '_' or '-' or ':')
            {
                return ch;
            }

            // 空白与常见分隔 → 下划线；路径/非法符去掉
            if (char.IsWhiteSpace(ch) || ch is '.' or '/' or '\\')
            {
                return '_';
            }

            // 保留 CJK 等 Unicode 字母
            if (char.GetUnicodeCategory(ch) is
                System.Globalization.UnicodeCategory.OtherLetter
                or System.Globalization.UnicodeCategory.LetterNumber)
            {
                return ch;
            }

            return '_';
        }).ToArray();

        var id = new string(chars);
        while (id.Contains("__", StringComparison.Ordinal))
        {
            id = id.Replace("__", "_", StringComparison.Ordinal);
        }

        id = id.Trim('_', '-');
        if (string.IsNullOrWhiteSpace(id))
        {
            id = fallback;
        }

        if (id.Length > 64)
        {
            id = id[..64].TrimEnd('_', '-');
        }

        return id;
    }

    /// <summary>
    /// 仅在目标字段为空时应用建议值（不覆盖作者已改内容）。
    /// </summary>
    public static void ApplySuggestionIfEmpty(
        ImportFieldSuggestion suggestion,
        ref string chapterId,
        ref string chapterTitle,
        ref string targetPath,
        ref decimal? order)
    {
        if (string.IsNullOrWhiteSpace(chapterId))
        {
            chapterId = suggestion.ChapterId;
        }

        if (string.IsNullOrWhiteSpace(chapterTitle))
        {
            chapterTitle = suggestion.ChapterTitle;
        }

        if (string.IsNullOrWhiteSpace(targetPath))
        {
            targetPath = suggestion.TargetPath;
        }

        // 排序默认 0 也视为可自动提升（作者尚未有意填写）
        if (order is null or 0)
        {
            order = suggestion.Order;
        }
    }

    /// <summary>
    /// 将手输或文件选择器返回的路径规范为项目相对路径。绝对路径必须能证明位于
    /// 当前项目根内；目标路径还必须落在 documents/，与后端路径沙箱保持同一契约。
    /// </summary>
    public static ImportPathValidation ValidateProjectPath(
        string? rawPath,
        string? projectRoot,
        bool requireDocumentsDirectory)
    {
        var raw = (rawPath ?? string.Empty).Trim();
        if (string.IsNullOrWhiteSpace(raw))
        {
            return new ImportPathValidation(string.Empty, ImportPathError.Required);
        }

        if (raw.StartsWith("~/", StringComparison.Ordinal)
            || raw.StartsWith("~\\", StringComparison.Ordinal)
            || (!OperatingSystem.IsWindows()
                && (IsWindowsDrivePath(raw) || raw.StartsWith("\\\\", StringComparison.Ordinal))))
        {
            return new ImportPathValidation(string.Empty, ImportPathError.OutsideProject);
        }

        string candidate;
        if (IsAbsoluteOrHomePath(raw))
        {
            if (!ProjectPathHelper.TryMakeRelativeToProjectRoot(raw, projectRoot, out candidate))
            {
                return new ImportPathValidation(string.Empty, ImportPathError.OutsideProject);
            }
        }
        else
        {
            candidate = raw;
        }

        candidate = candidate.Replace('\\', '/');
        var segments = candidate.Split('/', StringSplitOptions.RemoveEmptyEntries);
        if (segments.Any(segment => segment == ".."))
        {
            return new ImportPathValidation(string.Empty, ImportPathError.ParentTraversal);
        }

        var normalizedSegments = segments
            .Where(segment => segment != ".")
            .ToArray();
        if (normalizedSegments.Length == 0
            || normalizedSegments.Any(ContainsPortableInvalidPathCharacter))
        {
            return new ImportPathValidation(string.Empty, ImportPathError.Invalid);
        }

        var normalized = string.Join('/', normalizedSegments);
        if (requireDocumentsDirectory)
        {
            const string prefix = "documents/";
            if (!normalized.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)
                || normalized.Length <= prefix.Length)
            {
                return new ImportPathValidation(string.Empty, ImportPathError.TargetOutsideDocuments);
            }

            normalized = prefix + normalized[prefix.Length..];
        }

        return new ImportPathValidation(normalized, ImportPathError.None);
    }

    private static bool IsAbsoluteOrHomePath(string path)
    {
        return Path.IsPathRooted(path) || IsWindowsDrivePath(path);
    }

    private static bool IsWindowsDrivePath(string path)
    {
        return path.Length >= 3
               && char.IsLetter(path[0])
               && path[1] == ':'
               && path[2] is '/' or '\\';
    }

    private static bool ContainsPortableInvalidPathCharacter(string segment)
    {
        return segment.IndexOfAny(['\0', '<', '>', ':', '"', '|', '?', '*']) >= 0;
    }
}

public readonly record struct ImportFieldSuggestion(
    string ChapterId,
    string ChapterTitle,
    string TargetPath,
    decimal Order);

public enum ImportPathError
{
    None,
    Required,
    OutsideProject,
    ParentTraversal,
    Invalid,
    TargetOutsideDocuments,
}

public readonly record struct ImportPathValidation(string NormalizedPath, ImportPathError Error)
{
    public bool IsValid => Error == ImportPathError.None;
}
