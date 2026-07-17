using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using AvaloniaEdit.Document;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class WorksContinuousEditorTests
{
    [Fact]
    public void ContinuousEditorSelectionUsesOneGlobalRangeAcrossReadProjectionBlocks()
    {
        var vm = NewViewModel();
        var content = new string('甲', 4_200) + new string('乙', 4_200);
        vm.SeedOpenDocumentForTests("documents/long.md", "v1", content);
        Assert.True(vm.DocumentBlocks.Count > 1);
        var formerBoundary = vm.DocumentBlocks[0].Text.Length;
        var selection = new EditorTextSelection(formerBoundary - 40, formerBoundary + 40, string.Empty);

        Assert.True(WorksEditorSelectionEdit.TryResolve(
            vm.DocumentContent,
            selection,
            out var start,
            out var end,
            out var selectedText));
        Assert.Equal(formerBoundary - 40, start);
        Assert.Equal(formerBoundary + 40, end);
        Assert.Equal(content[(formerBoundary - 40)..(formerBoundary + 40)], selectedText);
        Assert.True(WorksEditorSelectionEdit.TryResolve(
            vm.DocumentContent,
            new EditorTextSelection(0, content.Length, string.Empty),
            out _,
            out _,
            out var allText));
        Assert.Equal(content, allText);
    }

    [Fact]
    public async Task BusinessTextSnapshotCanBeReadAfterThreadHop()
    {
        var buffer = new ContinuousDocumentBuffer();
        buffer.Replace("跨线程正文", resetUndoHistory: false);

        var snapshot = await Task.Run(() => buffer.Text);

        Assert.Equal("跨线程正文", snapshot);
        Assert.Equal(5, buffer.Length);
    }

    [Fact]
    public void BusinessSnapshotReplaysManyIncrementalDocumentChanges()
    {
        var buffer = new ContinuousDocumentBuffer();
        buffer.Replace("abcdef", resetUndoHistory: false);
        var expected = "abcdef";

        buffer.Document.Insert(3, "XYZ");
        expected = expected.Insert(3, "XYZ");
        buffer.Document.Remove(1, 2);
        expected = expected.Remove(1, 2);
        buffer.Document.Replace(2, 3, "跨界");
        expected = expected.Remove(2, 3).Insert(2, "跨界");
        for (var index = 0; index < 2_100; index++)
        {
            var value = (index % 10).ToString();
            buffer.Document.Insert(buffer.Document.TextLength, value);
            expected += value;
        }

        Assert.Equal(expected.Length, buffer.Length);
        Assert.Equal(expected, buffer.Text);
    }

    [Fact]
    public void EditingAcrossFormerBoundaryKeepsOneDocumentAndRefreshesReadProjectionOnExit()
    {
        var vm = NewViewModel();
        var content = new string('甲', 4_200) + new string('乙', 4_200);
        vm.SeedOpenDocumentForTests("documents/long.md", "v1", content);
        var document = vm.EditorDocument;
        var formerBoundary = vm.DocumentBlocks[0].Text.Length;

        document.Replace(formerBoundary - 1, 2, "跨界");
        document.Insert(formerBoundary + 1, new string('丙', 13_000));

        Assert.Same(document, vm.EditorDocument);
        Assert.True(vm.HasUnsavedChanges);
        Assert.Contains("跨界", vm.DocumentContent, StringComparison.Ordinal);
        Assert.Equal(content.Length + 13_000, vm.DocumentContent.Length);

        vm.IsEditMode = false;
        Assert.True(vm.DocumentBlocks.Count > 1);
        Assert.Equal(vm.DocumentContent, string.Concat(vm.DocumentBlocks.OrderBy(block => block.Index).Select(block => block.Text)));
    }

    [Fact]
    public void ExternalReplacementUsesSameDocumentAndPreservesGlobalAnchor()
    {
        var vm = NewViewModel();
        var content = new string('甲', 8_500);
        vm.SeedOpenDocumentForTests("documents/long.md", "v1", content);
        var document = vm.EditorDocument;
        var anchor = document.CreateAnchor(4_200);
        anchor.MovementType = AnchorMovementType.AfterInsertion;

        vm.DocumentContent = content.Insert(100, "新增");

        Assert.Same(document, vm.EditorDocument);
        Assert.Equal(4_202, anchor.Offset);
        Assert.Equal(content.Insert(100, "新增"), vm.DocumentContent);
    }

    [Theory]
    [InlineData("abcdef", "abcXdef", 3, 0, "X")]
    [InlineData("abcdef", "abef", 2, 2, "")]
    [InlineData("abcdef", "abXYef", 2, 2, "XY")]
    [InlineData("", "正文", 0, 0, "正文")]
    public void MinimalExternalChangeOnlyReplacesChangedMiddle(
        string current,
        string updated,
        int expectedOffset,
        int expectedRemovalLength,
        string expectedInsertion)
    {
        var change = ContinuousTextChange.Between(current, updated);

        Assert.Equal(expectedOffset, change.Offset);
        Assert.Equal(expectedRemovalLength, change.RemovalLength);
        Assert.Equal(expectedInsertion, change.Insertion);
        Assert.Equal(updated, current.Remove(change.Offset, change.RemovalLength).Insert(change.Offset, change.Insertion));
    }

    [Fact]
    public void WorksPageUsesVirtualizedContinuousEditorInsteadOfEditableBlockList()
    {
        var root = ResolveRepoRoot();
        var project = File.ReadAllText(Path.Combine(root, "desktop", "Ariadne.Desktop", "Ariadne.Desktop.csproj"));
        var app = File.ReadAllText(Path.Combine(root, "desktop", "Ariadne.Desktop", "App.axaml"));
        var view = File.ReadAllText(Path.Combine(root, "desktop", "Ariadne.Desktop", "Views", "WorksPageView.axaml"));
        var viewCode = File.ReadAllText(Path.Combine(root, "desktop", "Ariadne.Desktop", "Views", "WorksPageView.axaml.cs"));
        var viewModel = File.ReadAllText(Path.Combine(root, "desktop", "Ariadne.Desktop", "ViewModels", "WorksPageViewModel.cs"));

        Assert.Contains("Avalonia.AvaloniaEdit", project, StringComparison.Ordinal);
        Assert.Contains("avares://AvaloniaEdit/Themes/Fluent/AvaloniaEdit.xaml", app, StringComparison.Ordinal);
        Assert.Contains("<ae:TextEditor x:Name=\"DocumentEditor\"", view, StringComparison.Ordinal);
        Assert.Contains("Document=\"{Binding EditorDocument}\"", view, StringComparison.Ordinal);
        Assert.Contains("<VirtualizingStackPanel />", view, StringComparison.Ordinal);
        Assert.DoesNotContain("OnDocumentBlockEditor", view, StringComparison.Ordinal);
        Assert.DoesNotContain("SelectionForBlock", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("TryResolveBlockSelection", viewCode, StringComparison.Ordinal);
    }

    private static WorksPageViewModel NewViewModel()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, EmptyBackendProxy>();
        return new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);
    }

    private static string ResolveRepoRoot()
    {
        var path = Path.GetDirectoryName(typeof(WorksContinuousEditorTests).Assembly.Location)!;
        while (!string.IsNullOrEmpty(path) && !File.Exists(Path.Combine(path, "desktop", "Ariadne.slnx")))
        {
            path = Directory.GetParent(path)?.FullName ?? string.Empty;
        }
        return path;
    }

    private class EmptyBackendProxy : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
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
