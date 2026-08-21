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
        // U130 之后**刻意不再**把作者推进修改模式：按 Ctrl+K 要的是「跟 AI 说一句」，
        // 改写结果落在 diff 预览里，同意之后才需要编辑器在场。
        // 本行原先断言 `IsEditMode` 为真 —— 那是 U130 之前的行为，
        // 产品改对了而用例没跟改（`WorksPageViewModel.cs:1463` 的注释写明了这个决定）。
        // ⚠️ 判据改成**反向钉住**，而不是删掉这一行：删掉的话
        // 「哪天有人又把切模式加回 OpenQuickEdit」不会有任何东西变红。
        Assert.False(
            vm.IsEditMode,
            "OpenQuickEdit 把作者推进了修改模式 —— U130 刻意取消了这个行为"
            + "（Ctrl+K 只是「跟 AI 说一句」，不是「我要开始改」）。");
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
