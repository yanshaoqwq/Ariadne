using System.Reflection;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class WorksImportFormTests
{
    [Fact]
    public void ValidateProjectPath_NormalizesProjectPathsAndRejectsEscapes()
    {
        var root = Path.Combine(Path.GetTempPath(), $"ariadne-import-{Guid.NewGuid():N}");
        var source = Path.Combine(root, "planning", "imports", "chapter.md");

        var absolute = WorksImportHelper.ValidateProjectPath(source, root, requireDocumentsDirectory: false);
        var target = WorksImportHelper.ValidateProjectPath("Documents\\chapter.md", root, requireDocumentsDirectory: true);
        var traversal = WorksImportHelper.ValidateProjectPath("documents/../secrets.md", root, requireDocumentsDirectory: true);
        var outsideTarget = WorksImportHelper.ValidateProjectPath("planning/chapter.md", root, requireDocumentsDirectory: true);

        Assert.True(absolute.IsValid);
        Assert.Equal("planning/imports/chapter.md", absolute.NormalizedPath);
        Assert.True(target.IsValid);
        Assert.Equal("documents/chapter.md", target.NormalizedPath);
        Assert.Equal(ImportPathError.ParentTraversal, traversal.Error);
        Assert.Equal(ImportPathError.TargetOutsideDocuments, outsideTarget.Error);
    }

    [Fact]
    public void ImportForm_RequiresExplicitConfirmationForChapterOrTargetConflict()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, NoopBackendProxy>();
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);
        vm.WorksTreeRoots.Add(new WorksTreeItemViewModel(
            "chapter:one",
            "第一章",
            "documents/chapter.md",
            () => { },
            "chapter",
            "chapter-one"));

        vm.ImportChapterId = "chapter-one";
        vm.ImportChapterTitle = "第一章新版";
        vm.ImportOrder = 1m;
        vm.ImportSourcePath = "planning/imports/chapter.md";
        vm.ImportTargetPath = "documents/chapter.md";

        Assert.True(vm.HasImportConflict);
        Assert.False(vm.ImportCommand.CanExecute(null));
        vm.AllowImportOverwrite = true;
        Assert.True(vm.ImportCommand.CanExecute(null));
        var proxy = (NoopBackendProxy)(object)backend;
        Assert.True(vm.ImportCommand.TryExecute());
        Assert.NotNull(proxy.LastImportRequest);
        Assert.Equal("planning/imports/chapter.md", proxy.LastImportRequest!.SourcePath);
        Assert.Equal("documents/chapter.md", proxy.LastImportRequest.TargetPath);
        Assert.True(proxy.LastImportRequest.Overwrite);
        vm.ImportTargetPath = "../outside.md";
        Assert.False(vm.AllowImportOverwrite);
        Assert.True(vm.HasImportTargetError);
        Assert.False(vm.ImportCommand.CanExecute(null));
    }

    [Fact]
    public void ChapterImportRequest_SerializesExplicitOverwriteIntent()
    {
        var request = new ChapterImportRequest(
            "chapter-one",
            "第一章",
            1,
            "planning/imports/chapter.md",
            "documents/chapter.md",
            true);

        var json = JsonSerializer.Serialize(request, new JsonSerializerOptions(JsonSerializerDefaults.Web));

        Assert.Contains("\"overwrite\":true", json, StringComparison.Ordinal);
    }

    [Fact]
    public void WorksImportPanel_UsesPersistentLabelsNumericOrderAndConflictState()
    {
        var root = ResolveRepoRoot();
        var view = File.ReadAllText(Path.Combine(
            root,
            "desktop",
            "Ariadne.Desktop",
            "Views",
            "WorksPageView.axaml"));

        Assert.Contains("<NumericUpDown", view, StringComparison.Ordinal);
        Assert.Contains("ImportSourcePathText", view, StringComparison.Ordinal);
        Assert.Contains("ImportTargetPreviewText", view, StringComparison.Ordinal);
        Assert.Contains("ImportConfirmationText", view, StringComparison.Ordinal);
        Assert.Contains("AllowImportOverwrite", view, StringComparison.Ordinal);
        Assert.Contains("HasImportConflict", view, StringComparison.Ordinal);
    }

    private static string ResolveRepoRoot()
    {
        var path = Path.GetDirectoryName(typeof(WorksImportFormTests).Assembly.Location)!;
        while (!string.IsNullOrEmpty(path) && !File.Exists(Path.Combine(path, "desktop", "Ariadne.slnx")))
        {
            path = Directory.GetParent(path)?.FullName ?? string.Empty;
        }

        return path;
    }

    private class NoopBackendProxy : DispatchProxy
    {
        public ChapterImportRequest? LastImportRequest { get; private set; }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }

            if (targetMethod?.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }

            if (targetMethod?.Name == nameof(IAriadneBackendClient.ImportChapterAsync)
                && args is { Length: > 0 }
                && args[0] is ChapterImportRequest request)
            {
                LastImportRequest = request;
                return Task.FromResult(new ChapterImportReport(null, null));
            }

            if (targetMethod?.ReturnType.IsGenericType == true
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object?[] { null });
            }

            return null;
        }
    }
}
