using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class WorksEditorCommandTests
{
    [Fact]
    public void OpenQuickEdit_OnlyOpensComposerAndRequestsFocus()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, NoopBackendProxy>();
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);
        vm.SeedOpenDocumentForTests("documents/ch1.md", "v1", "正文");
        vm.IsEditMode = false;
        var focusRequests = 0;
        vm.RequestFocusQuickEditInstruction = () => focusRequests++;

        Assert.True(vm.OpenQuickEditCommand.TryExecute());
        Assert.True(vm.IsEditMode);
        Assert.Equal(1, focusRequests);
        Assert.False(vm.QuickAiCommand.TryExecute());
    }

    [Fact]
    public void RelayCommand_TryExecute_RejectsUnavailableAction()
    {
        var executions = 0;
        var command = new RelayCommand(() => executions++, () => false);

        Assert.False(command.TryExecute());
        Assert.Equal(0, executions);
    }

    [Fact]
    public void WorksPage_KeyboardShortcutsUseGuardedCommandsAndVisibleSaveState()
    {
        var root = ResolveRepoRoot();
        var viewCode = File.ReadAllText(Path.Combine(root, "desktop", "Ariadne.Desktop", "Views", "WorksPageView.axaml.cs"));
        var view = File.ReadAllText(Path.Combine(root, "desktop", "Ariadne.Desktop", "Views", "WorksPageView.axaml"));

        Assert.Contains("OpenQuickEditCommand.TryExecute", viewCode, StringComparison.Ordinal);
        Assert.Contains("SaveCommand.TryExecute", viewCode, StringComparison.Ordinal);
        Assert.Contains("KeyDown=\"OnWorksPageKeyDown\"", view, StringComparison.Ordinal);
        Assert.Contains("DocumentSaveStateText", view, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"QuickEditInstructionBox\"", view, StringComparison.Ordinal);
    }

    private static string ResolveRepoRoot()
    {
        var path = Path.GetDirectoryName(typeof(WorksEditorCommandTests).Assembly.Location)!;
        while (!string.IsNullOrEmpty(path) && !File.Exists(Path.Combine(path, "desktop", "Ariadne.slnx")))
        {
            path = Directory.GetParent(path)?.FullName ?? string.Empty;
        }

        return path;
    }

    private class NoopBackendProxy : DispatchProxy
    {
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
