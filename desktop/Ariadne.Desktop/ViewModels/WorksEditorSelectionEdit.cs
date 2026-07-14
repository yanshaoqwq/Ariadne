namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// Works 页：编辑器选区 → 项目 AI / 快捷改写 的纯选区解析与范围替换（不依赖 Avalonia）。
/// </summary>
public static class WorksEditorSelectionEdit
{
    /// <summary>
    /// 将 UI 选区解析为当前文档缓冲区内的 UTF-16 索引范围与选中文本。
    /// 以 document 切片为准（防止编辑器 SelectedText 与组装正文短暂不一致）。
    /// </summary>
    public static bool TryResolve(
        string documentContent,
        EditorTextSelection? selection,
        out int start,
        out int end,
        out string selectedText)
    {
        start = 0;
        end = 0;
        selectedText = string.Empty;
        if (selection is null)
        {
            return false;
        }

        var content = documentContent ?? string.Empty;
        var a = Math.Min(selection.Start, selection.End);
        var b = Math.Max(selection.Start, selection.End);
        if (b <= a || content.Length == 0)
        {
            return false;
        }

        start = Math.Clamp(a, 0, content.Length);
        end = Math.Clamp(b, 0, content.Length);
        if (end <= start)
        {
            return false;
        }

        selectedText = content[start..end];
        return !string.IsNullOrWhiteSpace(selectedText);
    }

    /// <summary>
    /// 仅替换 [start,end)，并校验该切片仍等于 expectedOriginal（防陈旧选区）。
    /// </summary>
    public static bool TryReplaceRange(
        string documentContent,
        int start,
        int end,
        string expectedOriginal,
        string replacement,
        out string updatedContent)
    {
        updatedContent = documentContent ?? string.Empty;
        if (start < 0 || end < start || end > updatedContent.Length)
        {
            return false;
        }

        var slice = updatedContent[start..end];
        if (!string.Equals(slice, expectedOriginal, StringComparison.Ordinal))
        {
            return false;
        }

        updatedContent = string.Concat(
            updatedContent.AsSpan(0, start),
            replacement ?? string.Empty,
            updatedContent.AsSpan(end));
        return true;
    }

    /// <summary>
    /// 构建带选区上下文的项目 AI 用户气泡文案（展示用，不替代结构化 original/suggested）。
    /// </summary>
    public static string FormatSelectionUserBubble(string instruction, string selectedText, int maxSnippet = 120)
    {
        var snippet = selectedText ?? string.Empty;
        if (snippet.Length > maxSnippet)
        {
            snippet = snippet[..(maxSnippet - 1)] + "…";
        }

        return $"{instruction.Trim()}\n\n「选中」{snippet}";
    }
}
