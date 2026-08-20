using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Tests;

/// <summary>U203 用例共用的 ViewModel 起手式（不需要 Avalonia 运行时的那一半）。</summary>
internal static class MarkdownRenderTestHarness
{
    /// <summary>造一个「已打开文档、处于阅读态」的作品页 ViewModel。</summary>
    public static WorksPageViewModel SeedReadingViewModel(string content)
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, SilentBackend>();
        var vm = new WorksPageViewModel(DisplayNameService.LoadDefault(), backend);
        vm.SeedOpenDocumentForTests("documents/chapter-1.md", "v1", content);
        vm.IsEditMode = false;
        return vm;
    }

    private class SilentBackend : DispatchProxy
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
