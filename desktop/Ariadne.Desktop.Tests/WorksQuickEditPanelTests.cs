using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U130 回归：快速 AI 改写面板此前**只绑 <c>IsEditMode</c>**，即「进修改模式」与
/// 「要 AI 改写」是同一个动作，且**关不掉**。1400×900 实测正文区从 526px
/// 塌成 209px，而面板占 289px——写作区比 AI 输入框还小。
///
/// 判据取「两个状态是否真的独立」而不是「命令能否执行」——后者在缺陷版本下
/// 同样为真（`OpenQuickEdit` 只是 `IsEditMode = true`，永远成功）。
/// </summary>
public sealed class WorksQuickEditPanelTests
{
    private const string ChapterId = "chapter-3";
    private const string OriginalBody = "第一段正文。\n第二段正文。";

    [Fact]
    public void OpenQuickEdit_DoesNotForceEditMode()
    {
        var vm = CreateViewModel();
        vm.IsEditMode = false;

        Assert.True(vm.OpenQuickEditCommand.TryExecute());

        // 缺陷版本这里是 true：用户按 Ctrl+K 只想跟 AI 说一句，
        // 却被推进编辑态——此后任何误触键盘都直接改到小说正文。
        Assert.False(vm.IsEditMode);
        Assert.True(vm.IsQuickEditOpen);
    }

    [Fact]
    public void QuickEditPanel_CanBeClosedWithoutLeavingEditMode()
    {
        var vm = CreateViewModel();
        vm.IsEditMode = true;
        Assert.True(vm.OpenQuickEditCommand.TryExecute());

        Assert.True(vm.CloseQuickEditCommand.TryExecute());

        // 关面板不等于退出修改模式：用户想继续手写，只是不需要 AI 面板占着地方。
        // 缺陷版本压根没有关闭入口，唯一「关掉」的办法是退回阅读模式（连带失去编辑器）。
        Assert.False(vm.IsQuickEditOpen);
        Assert.True(vm.IsEditMode);
    }

    [Fact]
    public void EnteringEditMode_DoesNotOpenQuickEditPanel()
    {
        var vm = CreateViewModel();

        Assert.True(vm.EditModeCommand.TryExecute());

        // 这是本条缺陷最直接的形态：切到「修改」就被塞一个 289px 的 AI 面板。
        // 想手写一段不需要 AI 面板——两件事本来无关。
        Assert.True(vm.IsEditMode);
        Assert.False(vm.IsQuickEditOpen);
    }

    [Fact]
    public void QuickEditPanel_StaysOpenWhenSwitchingToReadMode()
    {
        var vm = CreateViewModel();
        Assert.True(vm.OpenQuickEditCommand.TryExecute());

        Assert.True(vm.ReadModeCommand.TryExecute());

        // 反向解耦：改写建议是「这段改成那段」的对照阅读，阅读态照样该能看。
        // 缺陷版本里 IsVisible 直接绑 IsEditMode，回阅读态面板连内容一起消失。
        Assert.False(vm.IsEditMode);
        Assert.True(vm.IsQuickEditOpen);
    }

    [Fact]
    public void ClosingQuickEdit_DiscardsStalePendingSuggestion()
    {
        var vm = CreateViewModel();
        Assert.True(vm.OpenQuickEditCommand.TryExecute());
        vm.QuickEditDiff = "-旧句\n+新句";
        Assert.True(vm.HasQuickEditDiff);

        Assert.True(vm.CloseQuickEditCommand.TryExecute());

        // 留着旧 diff，下次开窗会看到一条与当前正文早已不同步的对照，
        // 而 CanApplyQuickEdit 又会拒绝应用——等于摆一个死按钮。
        Assert.False(vm.HasQuickEditDiff);
    }

    private static WorksPageViewModel CreateViewModel()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, QuickEditBackendProxy>();
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);
        vm.SeedOpenDocumentForTests($"documents/{ChapterId}.md", "v1", OriginalBody);
        return vm;
    }

    private class QuickEditBackendProxy : DispatchProxy
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
