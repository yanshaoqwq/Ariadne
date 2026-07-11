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
        var order = Math.Max(0, existingTreeCount).ToString();
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
        ref string order)
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

        // 排序默认 "0" 也视为可自动提升（作者尚未有意填写）
        if (string.IsNullOrWhiteSpace(order) || order.Trim() == "0")
        {
            order = suggestion.Order;
        }
    }
}

public readonly record struct ImportFieldSuggestion(
    string ChapterId,
    string ChapterTitle,
    string TargetPath,
    string Order);
