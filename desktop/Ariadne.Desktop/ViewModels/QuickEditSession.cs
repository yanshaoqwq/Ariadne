using Ariadne.Desktop.Backend;

namespace Ariadne.Desktop.ViewModels;

/// <summary>绑定一次快捷改写建议的文档身份、版本、正文快照和选区。</summary>
public sealed record QuickEditSession(
    string DocumentId,
    string? BaseVersion,
    string DocumentContent,
    int SelectionStart,
    int SelectionEnd,
    QuickEditResult Result)
{
    public bool MatchesCurrent(string documentId, string? version, string content)
    {
        return string.Equals(DocumentId, documentId, StringComparison.Ordinal)
               && string.Equals(BaseVersion, version, StringComparison.Ordinal)
               && string.Equals(DocumentContent, content, StringComparison.Ordinal);
    }

    public bool TryApply(string documentId, string? version, string content, out string updatedContent)
    {
        updatedContent = content;
        if (!MatchesCurrent(documentId, version, content)
            || SelectionStart < 0
            || SelectionStart > SelectionEnd
            || SelectionEnd > content.Length)
        {
            return false;
        }

        // Shared range apply (Works Project AI selection path uses the same guard).
        return WorksEditorSelectionEdit.TryReplaceRange(
            content,
            SelectionStart,
            SelectionEnd,
            Result.Original,
            Result.Suggested,
            out updatedContent);
    }
}

/// <summary>只允许在应用后正文未再变化时撤销，避免覆盖用户后续输入。</summary>
public sealed record QuickEditUndoState(
    string DocumentId,
    string AppliedContent,
    string PreviousContent)
{
    public bool TryUndo(string documentId, string content, out string restoredContent)
    {
        restoredContent = content;
        if (!string.Equals(DocumentId, documentId, StringComparison.Ordinal)
            || !string.Equals(AppliedContent, content, StringComparison.Ordinal))
        {
            return false;
        }

        restoredContent = PreviousContent;
        return true;
    }
}

public static class QuickEditPreviewBuilder
{
    public const int MaxPreviewCharacters = 8_000;

    public static (string Text, bool IsTruncated) Build(string diff)
    {
        if (diff.Length <= MaxPreviewCharacters)
        {
            return (diff, false);
        }

        var headLength = MaxPreviewCharacters * 2 / 3;
        var tailLength = MaxPreviewCharacters - headLength - 3;
        return (string.Concat(diff.AsSpan(0, headLength), "\n…\n", diff.AsSpan(diff.Length - tailLength)), true);
    }
}
