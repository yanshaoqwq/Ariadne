using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U131 回归：「大纲」按钮曾把 <c>"@planning/outline.md"</c> 追加进正文末尾，
/// 用户随后 Ctrl+S 就把这行垃圾**持久化进小说**；而那个路径全后端零存在
/// （真实约定是 <c>planning/chapters/{id}.md</c>）。
///
/// 判据取「正文字符是否一字未动」而非「命令能否执行」——后者在缺陷版本下也是真。
/// </summary>
public sealed class WorksOutlinePanelTests
{
    private const string ChapterId = "chapter-7";
    private const string OriginalBody = "第一段正文。\n第二段正文。";

    [Fact]
    public void ShowOutline_NeverMutatesDocumentContent()
    {
        var (vm, _) = CreateViewModel();

        Assert.True(vm.ToggleOutlinePanelCommand.TryExecute());

        // 缺陷版本会变成 OriginalBody + "\n@planning/outline.md"。
        Assert.Equal(OriginalBody, vm.DocumentContent);
        Assert.DoesNotContain("@planning/outline.md", vm.DocumentContent, StringComparison.Ordinal);
    }

    [Fact]
    public void ShowOutline_DoesNotForceEditMode()
    {
        var (vm, _) = CreateViewModel();
        vm.IsEditMode = false;

        Assert.True(vm.ToggleOutlinePanelCommand.TryExecute());

        // 缺陷版本强行 IsEditMode = true：只想看一眼大纲却被推进编辑态，
        // 于是任何误触键盘都会改到正文。
        Assert.False(vm.IsEditMode);
        Assert.True(vm.IsOutlinePanelOpen);
    }

    [Fact]
    public void ShowOutline_ReadsChapterScopedOutlinePath()
    {
        var (vm, recorder) = CreateViewModel();

        Assert.True(vm.ToggleOutlinePanelCommand.TryExecute());

        // 必须读该章自己的大纲，而不是那个不存在的 planning/outline.md。
        Assert.Equal($"planning/chapters/{ChapterId}.md", recorder.LastRequestedPath);
    }

    [Fact]
    public void ShowOutline_TogglesPanelClosedOnSecondInvocation()
    {
        var (vm, _) = CreateViewModel();

        Assert.True(vm.ToggleOutlinePanelCommand.TryExecute());
        Assert.True(vm.IsOutlinePanelOpen);

        Assert.True(vm.ToggleOutlinePanelCommand.TryExecute());
        Assert.False(vm.IsOutlinePanelOpen);
    }

    [Fact]
    public void OutlinePanelWidth_IsZeroWhenClosed()
    {
        var (vm, _) = CreateViewModel();

        Assert.Equal(0d, vm.OutlinePanelWidth);

        Assert.True(vm.ToggleOutlinePanelCommand.TryExecute());
        Assert.True(vm.OutlinePanelWidth > 0d);
    }

    private static (WorksPageViewModel Vm, OutlineBackendProxy Recorder) CreateViewModel()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, OutlineBackendProxy>();
        var recorder = (OutlineBackendProxy)backend;
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);
        vm.SeedOpenDocumentForTests($"documents/{ChapterId}.md", "v1", OriginalBody);
        vm.SeedSummaryChapterForTests(ChapterId);
        return (vm, recorder);
    }

    private class OutlineBackendProxy : DispatchProxy
    {
        public string? LastRequestedPath { get; private set; }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }

            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetDocumentContentByPathAsync)
                && args is { Length: > 0 }
                && args[0] is string path)
            {
                LastRequestedPath = path;
                return Task.FromResult("本章大纲：主角与旧友重逢。");
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
