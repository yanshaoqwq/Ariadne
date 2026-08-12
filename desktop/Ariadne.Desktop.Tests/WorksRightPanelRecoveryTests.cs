using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U133 回归：设置项「作品页项目面板」关掉后，作品页内**没有任何办法**把导航树叫回来。
///
/// 缺陷链是 `IsRightPanelToggleVisible => IsProjectPanelVisible || IsImportPanelOpen`：
/// 关掉设置项 ⇒ 药丸不可见 ⇒ `ToggleRightPanelCommand.CanExecute()` 返回 false
/// ⇒ 只能回设置页重新勾选。一个个性化偏好把功能锁死了。
///
/// 判据取「关掉设置项后能否在页面内恢复」，而不是「属性值对不对」——
/// 后者在缺陷版本下也自洽（它确实按自己的逻辑算出了 false）。
/// </summary>
public sealed class WorksRightPanelRecoveryTests
{
    [Fact]
    public void TogglePillStaysAvailableWhenPreferenceDisablesPanel()
    {
        var vm = CreateViewModel();

        vm.ApplyUiPreferences(CreatePreferences(projectPanelVisible: false));

        // 缺陷版本这两条都是 false：药丸消失 + 命令不可执行 = 页面内彻底没有出路。
        Assert.True(vm.IsRightPanelToggleVisible);
        Assert.True(vm.ToggleRightPanelCommand.CanExecute(null));
    }

    [Fact]
    public void PanelCanBeReopenedInPageAfterPreferenceDisabledIt()
    {
        var vm = CreateViewModel();
        vm.ApplyUiPreferences(CreatePreferences(projectPanelVisible: false));
        Assert.False(vm.IsRightPanelVisible);

        Assert.True(vm.ToggleRightPanelCommand.TryExecute());

        // 这是本条缺陷的核心：用户在作品页内必须能把导航树叫回来。
        Assert.True(vm.IsRightPanelVisible);
        Assert.True(vm.RightPanelColumnWidth.Value > 0d);
    }

    [Fact]
    public void PreferenceOnlySuppliesTheDefaultOpenState()
    {
        var closed = CreateViewModel();
        closed.ApplyUiPreferences(CreatePreferences(projectPanelVisible: false));
        Assert.False(closed.IsRightPanelOpen);

        var opened = CreateViewModel();
        opened.ApplyUiPreferences(CreatePreferences(projectPanelVisible: true));
        Assert.True(opened.IsRightPanelOpen);

        // 设置项的职责只到这里：决定**进页面时**收着还是展开。
        // 它不该继续参与「能不能开合」——那是页面内的事。
        Assert.True(closed.IsRightPanelToggleVisible);
        Assert.True(opened.IsRightPanelToggleVisible);
    }

    [Fact]
    public void SavedPanelStateWinsOverPreferenceDefault()
    {
        var vm = CreateViewModel();

        // 用户上次在页面内展开过 ⇒ 保存的状态优先于设置项的默认值。
        vm.ApplyUiPreferences(CreatePreferences(
            projectPanelVisible: false,
            savedRightPanelOpen: true));

        Assert.True(vm.IsRightPanelOpen);
    }

    [Fact]
    public void TogglingInPageIsPersistedEvenWhenPreferenceIsOff()
    {
        var vm = CreateViewModel(out var persisted);
        vm.ApplyUiPreferences(CreatePreferences(projectPanelVisible: false));

        Assert.True(vm.ToggleRightPanelCommand.TryExecute());

        // 缺陷版本在设置项关着时**不落盘**（`if (!IsProjectPanelVisible) return;`），
        // 于是「在页面里展开 → 切页回来又收起了」，用户会以为展开失败。
        Assert.Contains(true, persisted);
    }

    private static UiPreferences CreatePreferences(
        bool projectPanelVisible,
        bool? savedRightPanelOpen = null)
    {
        var panelStates = new Dictionary<string, bool>();
        if (savedRightPanelOpen is { } saved)
        {
            // key 与 WorksPageViewModel.RightPanelPreferenceKey 一致。
            panelStates["works.right_panel"] = saved;
        }
        return new UiPreferences(
            Theme: "system",
            GitAutoColor: "#00A0A0",
            GitManualColor: "#A000A0",
            ProjectPanelVisible: projectPanelVisible,
            ProjectPanelPosition: null,
            PanelStates: panelStates,
            OnboardingSeen: true);
    }

    private static WorksPageViewModel CreateViewModel() => CreateViewModel(out _);

    private static WorksPageViewModel CreateViewModel(out List<bool> persisted)
    {
        var recorded = new List<bool>();
        persisted = recorded;
        var backend = DispatchProxy.Create<IAriadneBackendClient, RightPanelBackendProxy>();
        // 用现成的构造参数注入，不新增测试专用属性——那种属性本身就是
        // 死代码扫描器要标记的东西（生产零调用者的公开 API）。
        return new WorksPageViewModel(
            DisplayNameService.LoadDefault(),
            backend,
            persistPanelState: (_, isOpen) =>
            {
                recorded.Add(isOpen);
                return Task.CompletedTask;
            });
    }

    private class RightPanelBackendProxy : DispatchProxy
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
