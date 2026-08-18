using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U163-A：作品页不得在正常操作下把用户弹回欢迎界面。
///
/// 判据落在**用户做那个动作后实际发生了什么**上：先在「未打开项目」状态下进过作品页
/// （侧栏允许，见 AlwaysAvailablePageIds），再打开项目 —— 这是真实操作序列。
/// 断言取「切换项目这一步有没有抛异常」，因为抛出的 ObjectDisposedException
/// 会冒泡到 MainWindowViewModel 切页的 catch(Exception) 并执行 CurrentPage = Welcome。
///
/// 根因不是审查文档推测的「页面构造失败」，而是 LoadWorksTreeAsync 把一个
/// `using` 作用域的 CancellationTokenSource 存进了 _worksTreeLoadCts 字段：
/// 方法返回时对象被 Dispose，字段却仍指着它（「无项目根」那条早返回还绕过了清理），
/// 下一次 Cancel() 就抛 ObjectDisposedException——它不是取消异常，无人吞。
/// </summary>
public sealed class WorksPageBounceToWelcomeTests
{
    [Fact]
    public async Task VisitWorksWithoutProject_ThenSwitchProject_DoesNotThrowDisposedCts()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, NoProjectRootBackend>();
        var works = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);

        // 1) 未打开项目时进作品页：树加载走「无项目根」那条早返回
        await works.ReloadProjectDataAsync();

        // 2) 随后切换/打开项目：MainWindowViewModel.ResetProjectPageSession 会调这个
        var bounce = Record.Exception(() => works.DeactivateProjectData());

        Assert.True(
            bounce is null,
            "切换项目时作品页抛了 " + bounce?.GetType().Name
            + "，它会冒泡到切页的 catch 并把用户弹回欢迎界面（丢掉当前章节与阅读位置）："
            + bounce?.Message);
    }

    [Fact]
    public async Task ReloadWorksPageTwiceWithoutProject_DoesNotThrowDisposedCts()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, NoProjectRootBackend>();
        var works = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);

        await works.ReloadProjectDataAsync();
        // Git 回档 / 项目数据刷新都会再次 reload 同一个缓存页实例
        var bounce = await Record.ExceptionAsync(() => works.ReloadProjectDataAsync());

        Assert.True(
            bounce is null,
            "第二次加载作品页抛了 " + bounce?.GetType().Name
            + "，用户会被弹回欢迎界面而不是停在作品页：" + bounce?.Message);
    }

    /// 无项目根的后端：只为走到 LoadWorksTreeAsync 的早返回分支。
    private class NoProjectRootBackend : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            if (targetMethod.Name == "get_HasProjectRoot")
            {
                return false;
            }

            if (targetMethod.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object?[]
                    {
                        resultType.IsValueType ? Activator.CreateInstance(resultType) : null,
                    });
            }

            return targetMethod.ReturnType.IsValueType
                ? Activator.CreateInstance(targetMethod.ReturnType)
                : null;
        }
    }
}
