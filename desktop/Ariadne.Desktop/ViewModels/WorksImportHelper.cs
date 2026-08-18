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
    /// 将手输或文件选择器返回的路径规范为项目相对路径。目标路径的绝对形态必须能
    /// 证明位于当前项目根内、且落在 documents/，与后端路径沙箱保持同一契约。
    /// </summary>
    /// <param name="requireInsideProject">
    /// 落点必须在项目内（我们要往那儿写）；**导入源不必**——作者从下载目录、U 盘、
    /// 别的写作软件导出目录里挑稿子是最常见的用法，不是异常情况。
    ///
    /// 后端两个校验函数刻意不同：<c>import_source_path_buf</c>（commands.rs:14409）
    /// 只禁 <c>..</c>，绝对路径原样放行；<c>project_path_buf</c>（:14418）额外要求
    /// <c>ensure_path_under_root</c>。后端还专门为源在项目外时把**它所在目录**
    /// 加进只读沙箱（commands.rs:2280 一带）。这里若沿用落点那套「必须在项目内」，
    /// 等于前端把一条后端明确支持的能力挡死——而文件选择器又不受此限，
    /// 于是形成「浏览让你选、选完告诉你不行」的死结（U163-B）。
    ///
    /// 放宽仅限「位置」这一维：<c>..</c> 与当前平台非法的路径形态仍然拒绝。
    /// </param>
    public static ImportPathValidation ValidateProjectPath(
        string? rawPath,
        string? projectRoot,
        bool requireDocumentsDirectory,
        bool requireInsideProject = true)
    {
        var raw = (rawPath ?? string.Empty).Trim();
        if (string.IsNullOrWhiteSpace(raw))
        {
            return new ImportPathValidation(string.Empty, ImportPathError.Required);
        }

        // 平台非法形态与「位置不对」是两类问题，必须用不同错误码：
        // Linux 上收到 C:\... 或 \\server\share 是数据错误，放宽项目外时不能连带放行。
        if (!OperatingSystem.IsWindows()
            && (IsWindowsDrivePath(raw) || raw.StartsWith("\\\\", StringComparison.Ordinal)))
        {
            return new ImportPathValidation(string.Empty, ImportPathError.UnsupportedPathForm);
        }

        // `~` 在 Linux 上是合法的家目录前缀，源路径放宽后 ~/Downloads/稿子.md 应当可用。
        // 但后端不认 `~`（import_source_path_buf 会把它当相对路径拼到项目根下，
        // 得到一个不存在的路径），所以必须在前端就展开成绝对路径再往下走。
        var isHomePrefixed = raw.StartsWith("~/", StringComparison.Ordinal)
                             || raw.StartsWith("~\\", StringComparison.Ordinal);
        if (isHomePrefixed)
        {
            if (!requireInsideProject && TryExpandHomePath(raw, out var expanded))
            {
                raw = expanded;
            }
            else
            {
                // 落点侧：`~` 展开后几乎必然落在项目外，保持既有拒绝语义。
                return new ImportPathValidation(string.Empty, ImportPathError.OutsideProject);
            }
        }

        string candidate;
        if (IsAbsoluteOrHomePath(raw))
        {
            if (!ProjectPathHelper.TryMakeRelativeToProjectRoot(raw, projectRoot, out candidate))
            {
                if (requireInsideProject)
                {
                    return new ImportPathValidation(string.Empty, ImportPathError.OutsideProject);
                }

                // 项目外的源：保留原始绝对路径交给后端。**不能**继续走下面那段
                // 项目相对路径的规范化——那会把绝对路径当相对路径拼到项目根后面。
                return ValidateOutsideProjectSource(raw);
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

    /// <summary>
    /// 项目外的导入源：只做「形态」检查，把绝对路径原样交给后端。
    ///
    /// 判据与后端 <c>import_source_path_buf</c> 对齐——它只跑
    /// <c>ensure_no_parent_traversal</c>，不查项目根。这里额外挡掉便携非法字符，
    /// 是为了在按钮禁用时就能给出原因，而不是等后端拒了再报错。
    /// </summary>
    private static ImportPathValidation ValidateOutsideProjectSource(string absolutePath)
    {
        var unified = absolutePath.Replace('\\', '/');
        var segments = unified.Split('/', StringSplitOptions.RemoveEmptyEntries);
        if (segments.Any(segment => segment == ".."))
        {
            return new ImportPathValidation(string.Empty, ImportPathError.ParentTraversal);
        }

        // 首段可能是 Windows 盘符（`C:`），冒号在那里是合法的；只检查其余段。
        // 非 Windows 平台的 `C:\...` 已在上游按 UnsupportedPathForm 挡掉。
        var checkable = OperatingSystem.IsWindows() && segments.Length > 0 && IsWindowsDriveSegment(segments[0])
            ? segments.Skip(1)
            : segments;
        if (segments.Length == 0
            || checkable.Where(segment => segment != ".").Any(ContainsPortableInvalidPathCharacter))
        {
            return new ImportPathValidation(string.Empty, ImportPathError.Invalid);
        }

        // 归一化 `.` 段但保留绝对形态：后端要的是能直接打开的路径。
        var normalized = Path.GetFullPath(absolutePath);
        return new ImportPathValidation(normalized, ImportPathError.None);
    }

    /// <summary>把 <c>~/</c> 前缀展开成绝对家目录路径；取不到家目录时返回 false。</summary>
    private static bool TryExpandHomePath(string raw, out string expanded)
    {
        expanded = string.Empty;
        var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
        if (string.IsNullOrWhiteSpace(home))
        {
            return false;
        }

        var rest = raw[2..].Replace('\\', '/').TrimStart('/');
        expanded = Path.Combine(home, rest);
        return true;
    }

    private static bool IsWindowsDriveSegment(string segment)
    {
        return segment.Length == 2 && char.IsLetter(segment[0]) && segment[1] == ':';
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
    /// <summary>
    /// 在当前平台上根本不合法的路径形态（Linux 上的 <c>C:\…</c> / UNC）。
    /// 与 <see cref="OutsideProject"/> 分开是必需的：导入源放宽了「位置」这一维，
    /// 若两者共用同一错误码，放宽会连带把非法形态一起放行（U163-B）。
    /// </summary>
    UnsupportedPathForm,
}

public readonly record struct ImportPathValidation(string NormalizedPath, ImportPathError Error)
{
    public bool IsValid => Error == ImportPathError.None;
}
