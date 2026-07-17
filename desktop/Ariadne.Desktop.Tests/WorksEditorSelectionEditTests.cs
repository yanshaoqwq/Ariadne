using Ariadne.Desktop.Backend;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class WorksEditorSelectionEditTests
{
    [Fact]
    public void TryResolve_UsesDocumentSliceNotStaleSelectionText()
    {
        const string doc = "前缀目标段后缀";
        // UI selection indexes cover 目标段; Text field intentionally wrong.
        var selection = new EditorTextSelection(2, 5, "错的");
        Assert.True(WorksEditorSelectionEdit.TryResolve(doc, selection, out var start, out var end, out var text));
        Assert.Equal(2, start);
        Assert.Equal(5, end);
        Assert.Equal("目标段", text);
    }

    [Fact]
    public void TryResolve_RejectsEmptySelection_NormalizesInvertedRange()
    {
        Assert.False(WorksEditorSelectionEdit.TryResolve("abc", null, out _, out _, out _));
        Assert.False(WorksEditorSelectionEdit.TryResolve("abc", new EditorTextSelection(1, 1, ""), out _, out _, out _));
        // Inverted caret order is normalized to a real span (same as editor SelectionStart/End).
        Assert.True(WorksEditorSelectionEdit.TryResolve("abc", new EditorTextSelection(2, 1, "b"), out var s, out var e, out var t));
        Assert.Equal(1, s);
        Assert.Equal(2, e);
        Assert.Equal("b", t);
    }

    [Fact]
    public void TryReplaceRange_OnlyTouchesSelection()
    {
        const string doc = "开头旧句结尾";
        Assert.True(WorksEditorSelectionEdit.TryReplaceRange(
            doc, 2, 4, "旧句", "新句", out var updated));
        Assert.Equal("开头新句结尾", updated);
        Assert.Equal("开头", updated[..2]);
        Assert.Equal("结尾", updated[^2..]);
    }

    [Fact]
    public void TryReplaceRange_RejectsStaleOriginal()
    {
        const string doc = "开头旧句结尾";
        Assert.False(WorksEditorSelectionEdit.TryReplaceRange(
            doc, 2, 4, "别的", "新句", out var unchanged));
        Assert.Equal(doc, unchanged);
    }

    [Fact]
    public void QuickEditSession_Apply_UsesSharedRangeHelper_ProjectAiPath()
    {
        // Same apply entry Project AI selection path uses after quick_edit returns.
        const string content = "甲乙丙丁戊";
        var session = new QuickEditSession(
            "doc-1",
            "v1",
            content,
            1,
            3,
            new QuickEditResult("乙丙", "XY", "-乙丙\n+XY"));

        Assert.True(session.TryApply("doc-1", "v1", content, out var updated));
        Assert.Equal("甲XY丁戊", updated);
        Assert.StartsWith("甲", updated, StringComparison.Ordinal);
        Assert.EndsWith("丁戊", updated, StringComparison.Ordinal);
    }

    [Fact]
    public void WorksPage_SendProjectAi_UsesSelectionResolveAndQuickEditWhenSelectionPresent()
    {
        // Structural gate: selection path must call TryResolve + QuickEdit + TryApply, not whole-doc ProjectAiChat only.
        var src = File.ReadAllText(ResolveWorksVmSource());
        Assert.Contains("WorksEditorSelectionEdit.TryResolve", src, StringComparison.Ordinal);
        Assert.Contains("SendProjectAiSelectionEditAsync", src, StringComparison.Ordinal);
        Assert.Contains("QuickEditAsync(new QuickEditRequest", src, StringComparison.Ordinal);
        Assert.Contains("session.TryApply", src, StringComparison.Ordinal);
        // Free-form chat remains for no-selection path.
        Assert.Contains("ProjectAiChatAsync", src, StringComparison.Ordinal);
    }

    [Fact]
    public void FormatSelectionUserBubble_IncludesInstructionAndSnippet()
    {
        var text = WorksEditorSelectionEdit.FormatSelectionUserBubble("改紧凑一点", "很长的选中正文内容");
        Assert.Contains("改紧凑一点", text, StringComparison.Ordinal);
        Assert.Contains("选中", text, StringComparison.Ordinal);
    }

    [Fact]
    public void SummarySourceMapper_MapsUtf8BytesToUtf16_ForChineseAndEmoji()
    {
        const string text = "甲😀乙";

        Assert.True(WorksSummarySourceMapper.TryMapUtf8Range(
            text,
            byteStart: 3,
            byteEnd: 7,
            out var start,
            out var end));
        Assert.Equal(1, start);
        Assert.Equal(3, end);
        Assert.Equal("😀", text[start..end]);
    }

    [Theory]
    [InlineData(4, 7)]
    [InlineData(3, 6)]
    [InlineData(-1, 3)]
    [InlineData(0, 99)]
    public void SummarySourceMapper_RejectsSplitCodePointOrInvalidBounds(long start, long end)
    {
        Assert.False(WorksSummarySourceMapper.TryMapUtf8Range(
            "甲😀乙",
            start,
            end,
            out _,
            out _));
    }

    [Fact]
    public void SummarySourceMapper_MapsUtf16CaretToUtf8_AndRejectsSurrogateSplit()
    {
        const string text = "甲😀乙";

        Assert.True(WorksSummarySourceMapper.TryMapUtf16OffsetToUtf8(text, 1, out var emojiStart));
        Assert.Equal(3, emojiStart);
        Assert.True(WorksSummarySourceMapper.TryMapUtf16OffsetToUtf8(text, 3, out var emojiEnd));
        Assert.Equal(7, emojiEnd);
        Assert.False(WorksSummarySourceMapper.TryMapUtf16OffsetToUtf8(text, 2, out _));
    }

    [Fact]
    public void WorksPage_ContinuousDocumentRangeCrossesReadProjectionBoundary()
    {
        var backend = System.Reflection.DispatchProxy.Create<IAriadneBackendClient, EmptyBackendProxy>();
        var vm = new WorksPageViewModel(
            Ariadne.Desktop.Localization.DisplayNameService.LoadDefault(),
            backend);
        var content = new string('甲', 4_100) + new string('乙', 4_100);
        vm.SeedOpenDocumentForTests("documents/ch1.md", "v1", content);
        var readBoundary = vm.DocumentBlocks[0].Text.Length;
        var selection = new EditorTextSelection(readBoundary - 50, readBoundary + 50, string.Empty);

        Assert.True(WorksEditorSelectionEdit.TryResolve(
            vm.DocumentContent,
            selection,
            out var start,
            out var end,
            out var selectedText));
        Assert.Equal(readBoundary - 50, start);
        Assert.Equal(readBoundary + 50, end);
        Assert.Equal(content[(readBoundary - 50)..(readBoundary + 50)], selectedText);
    }

    private static string ResolveWorksVmSource()
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 10 && walk is not null; i++)
        {
            var candidate = Path.Combine(walk.FullName, "desktop", "Ariadne.Desktop", "ViewModels", "WorksPageViewModel.cs");
            if (File.Exists(candidate))
            {
                return candidate;
            }

            walk = walk.Parent;
        }

        throw new FileNotFoundException("WorksPageViewModel.cs");
    }

    private class EmptyBackendProxy : System.Reflection.DispatchProxy
    {
        protected override object? Invoke(System.Reflection.MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return false;
            }
            if (targetMethod?.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (targetMethod?.ReturnType.IsGenericType == true
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var type = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(type)
                    .Invoke(null, new[] { type.IsValueType ? Activator.CreateInstance(type) : null });
            }
            return targetMethod?.ReturnType.IsValueType == true
                ? Activator.CreateInstance(targetMethod.ReturnType)
                : null;
        }
    }
}
