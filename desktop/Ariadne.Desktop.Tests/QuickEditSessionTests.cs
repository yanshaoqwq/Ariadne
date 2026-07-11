using Ariadne.Desktop.Backend;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class QuickEditSessionTests
{
    private static QuickEditSession Session(string documentId = "documents/chapter-1.md")
    {
        const string content = "开头旧句结尾";
        return new QuickEditSession(
            documentId,
            "v1",
            content,
            2,
            4,
            new QuickEditResult("旧句", "新句", "- 旧句\n+ 新句"));
    }

    [Fact]
    public void TryApply_OnlyChangesCapturedDocumentAndSelection()
    {
        var session = Session();

        Assert.True(session.TryApply("documents/chapter-1.md", "v1", "开头旧句结尾", out var updated));
        Assert.Equal("开头新句结尾", updated);
    }

    [Theory]
    [InlineData("documents/chapter-2.md", "v1", "开头旧句结尾")]
    [InlineData("documents/chapter-1.md", "v2", "开头旧句结尾")]
    [InlineData("documents/chapter-1.md", "v1", "开头改句结尾")]
    public void TryApply_RejectsChangedDocumentVersionOrContent(
        string documentId,
        string version,
        string content)
    {
        var session = Session();

        Assert.False(session.TryApply(documentId, version, content, out var unchanged));
        Assert.Equal(content, unchanged);
    }

    [Fact]
    public void Undo_RejectsOverwritingLaterUserEdits()
    {
        var undo = new QuickEditUndoState(
            "documents/chapter-1.md",
            "开头新句结尾",
            "开头旧句结尾");

        Assert.True(undo.TryUndo("documents/chapter-1.md", "开头新句结尾", out var restored));
        Assert.Equal("开头旧句结尾", restored);
        Assert.False(undo.TryUndo("documents/chapter-1.md", "开头新句又编辑", out _));
    }

    [Fact]
    public void Preview_IsBoundedAndKeepsBothEnds()
    {
        var diff = "head" + new string('x', QuickEditPreviewBuilder.MaxPreviewCharacters) + "tail";

        var preview = QuickEditPreviewBuilder.Build(diff);

        Assert.True(preview.IsTruncated);
        Assert.True(preview.Text.Length <= QuickEditPreviewBuilder.MaxPreviewCharacters + 1);
        Assert.StartsWith("head", preview.Text);
        Assert.EndsWith("tail", preview.Text);
    }
}

public sealed class WorkflowLoadGuardTests
{
    [Theory]
    [InlineData(false, WorkflowLoadState.NoProject, false)]
    [InlineData(true, WorkflowLoadState.Loading, false)]
    [InlineData(true, WorkflowLoadState.LoadFailed, false)]
    [InlineData(true, WorkflowLoadState.Loaded, true)]
    public void Persist_RequiresProjectAndSuccessfullyLoadedBaseline(
        bool hasProjectRoot,
        WorkflowLoadState state,
        bool expected)
    {
        Assert.Equal(expected, WorkflowLoadGuard.CanPersist(hasProjectRoot, state));
    }
}
