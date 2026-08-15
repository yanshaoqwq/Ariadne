using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U138 护栏：Git 回档承诺「回档前请先保存或处理当前未保存的版本更改」，
/// 而真正的拦截是 <c>ConfirmCachedProjectPagesLeaveAsync</c>——它只遍历
/// <c>_pageCache</c>，且 <c>GetOrCreatePage</c> 是**懒加载**。
///
/// ⚠️ **审查报告对这条的判断是错的**，本文件记录复核结论：
/// 报告说「用户打开项目 → 直接进 Git 页 → 回档 ⇒ 从未打开过作品页
/// ⇒ WorksPageViewModel 从未被构造 ⇒ 它的 HasUnsavedChanges 无人可问
/// ⇒ 静默放行」。实际上 <c>works</c> 在 <c>PreloadedProjectPageIds</c> 里，
/// 打开项目时 <c>LoadProjectDataPagesAsync</c> 就会 <c>GetOrCreatePage("works")</c>
/// 并写进缓存——用户不可能"从未打开过作品页"还能走到回档。
///
/// 但这个安全性质**完全依赖那份预载清单**，而清单与守卫之间没有任何约束：
/// 谁哪天为了「加快打开项目速度」把 works 从预载里移出去，
/// 报告描述的丢稿路径就真的成立，且**没有任何测试会红**
/// （现有测试都手动 GetPageForTests 把页塞进缓存，绕过了这个前提）。
/// 所以这里把「每个能持有未保存内容的页都必须被预载」钉成断言。
/// </summary>
[Collection("GlobalDialogService")]
public sealed class GitRestoreDirtyGuardTests
{
    [Fact]
    public async Task PreloadingProjectPagesInstantiatesEveryUnsavedChangesGuard()
    {
        var names = DisplayNameService.LoadDefault();
        DialogService.Initialize(names);
        var created = new List<string>();
        var window = new MainWindowViewModel(
            names,
            NoopBackend.Create(),
            id =>
            {
                created.Add(id);
                return id == "works" ? new AlwaysDirtyGuard("works", "作品") : null;
            },
            _ => { });

        await window.PreloadProjectPagesForTestsAsync();

        // 作品页承载正文，未保存的正文只存在于它的内存里——后端问不到。
        // 它必须在打开项目时就被实例化，否则回档前的脏状态检查无从下手。
        Assert.Contains("works", created);
        Assert.True(window.HasCachedUnsavedChanges);
    }

    [Fact]
    public async Task RestoreGuardSeesDirtyWorksPageWithoutEverNavigatingToIt()
    {
        var names = DisplayNameService.LoadDefault();
        DialogService.Initialize(names);
        var window = new MainWindowViewModel(
            names,
            NoopBackend.Create(),
            id => id == "works" ? new AlwaysDirtyGuard("works", "作品") : null,
            _ => { });

        await window.PreloadProjectPagesForTestsAsync();

        // 用户从未导航到作品页（CurrentPage 仍是欢迎页），但脏状态照样被看见。
        // 这正是报告认为不成立、实际成立的那条性质。
        Assert.Same(window.Welcome, window.CurrentPage);

        var guardTask = window.ConfirmCloseAsync();
        await WaitForDialogAsync();
        DialogService.Current.RequestCancelActive();

        // 取消 ⇒ 回档被拦住。缺陷版本（works 不预载）这里会直接返回 true 放行，
        // 而回档会切分支并触发项目重载，内存里未保存的正文就此丢失——
        // 用户刚刚读到的确认框明确告诉他系统会管这件事。
        Assert.False(await guardTask);
    }

    [Fact]
    public void GuardImplementorsAreAllCoveredByThePreloadList()
    {
        // 反射找出所有 IUnsavedChangesGuard 的实现，逐个核对是否在预载清单里。
        // 将来新增一个持有未保存内容的页而忘了加进预载，这条会红。
        var guardImplementors = typeof(IUnsavedChangesGuard).Assembly
            .GetTypes()
            .Where(type => !type.IsAbstract
                           && !type.IsInterface
                           && typeof(IUnsavedChangesGuard).IsAssignableFrom(type))
            .Select(type => type.Name)
            .ToList();

        Assert.Contains("WorksPageViewModel", guardImplementors);
        Assert.Contains("WorkspacePageViewModel", guardImplementors);

        var preloaded = MainWindowViewModel.PreloadedProjectPageIdsForTests;
        Assert.Contains("works", preloaded);
        Assert.Contains("workspace", preloaded);

        // SettingsPageViewModel 也实现了该接口但**刻意不在预载清单里**：
        // 它的脏状态是设置项而不是正文，用户没进过设置页就不可能改过设置。
        // 与正文不同——正文可以由工作流写入（用户没进作品页也可能有未保存内容），
        // 所以那两个必须预载。
        Assert.Contains("SettingsPageViewModel", guardImplementors);
        Assert.DoesNotContain("settings", preloaded);
    }

    private static async Task WaitForDialogAsync()
    {
        for (var attempt = 0; attempt < 200 && !DialogService.Current.IsOpen; attempt++)
        {
            await Task.Yield();
        }
        Assert.True(DialogService.Current.IsOpen, "未保存确认框没有弹出");
    }

    private sealed class AlwaysDirtyGuard : IUnsavedChangesGuard
    {
        public AlwaysDirtyGuard(string pageId, string title)
        {
            UnsavedChangesPageId = pageId;
            UnsavedChangesPageTitle = title;
        }

        public bool HasUnsavedChanges => true;
        public string UnsavedChangesPageId { get; }
        public string UnsavedChangesPageTitle { get; }
        public string? PreparedUnsavedChangesPayloadIdentity => null;

        public Task<bool> ConfirmLeaveIfNeededAsync() => Task.FromResult(false);
        public Task<bool> PrepareUnsavedChangesAsync() => Task.FromResult(true);
        public Task<bool> CommitPreparedUnsavedChangesAsync() => Task.FromResult(true);
        public Task AbortPreparedUnsavedChangesAsync() => Task.CompletedTask;
        public Task<bool> SaveUnsavedChangesAsync() => Task.FromResult(true);
        public Task DiscardUnsavedChangesAsync() => Task.CompletedTask;
    }

    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }
            // 徽章契约不允许 null——生产代码直接读字段，返回 null 就是 NRE。
            // 替身也要守这个约定，否则测的是替身的 bug 而不是被测代码。
            if (targetMethod?.Name == nameof(IAriadneBackendClient.GetSidebarBadgesAsync))
            {
                return Task.FromResult(new SidebarBadgeCounts(0, 0, 0));
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
