namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// Pure sticky-selection policy for Works editor (testable without Avalonia).
/// Sticky holds the last non-empty span so Project AI still sees it after focus moves to chat.
/// </summary>
public static class EditorStickySelectionPolicy
{
    /// <summary>
    /// Update sticky from a live editor selection sample.
    /// </summary>
    /// <param name="current">Previous sticky (may be null).</param>
    /// <param name="start">Selection start (already min).</param>
    /// <param name="end">Selection end (already max).</param>
    /// <param name="selectedText">Selected text (may be empty).</param>
    /// <param name="clearWhenEmpty">
    /// When true (e.g. intentional deselect while editor still focused), empty caret clears sticky.
    /// When false (e.g. LostFocus as focus moves to Project AI), empty sample does not clear sticky.
    /// </param>
    public static EditorTextSelection? Update(
        EditorTextSelection? current,
        int start,
        int end,
        string? selectedText,
        bool clearWhenEmpty)
    {
        if (end > start && !string.IsNullOrWhiteSpace(selectedText))
        {
            return new EditorTextSelection(start, end, selectedText!);
        }

        return clearWhenEmpty ? null : current;
    }

    /// <summary>Document open/switch must drop sticky so old indices never apply to a new buffer.</summary>
    public static EditorTextSelection? ClearOnDocumentChange() => null;
}
