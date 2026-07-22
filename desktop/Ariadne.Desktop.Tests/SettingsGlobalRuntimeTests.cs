using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class SettingsGlobalRuntimeTests
{
    [Fact]
    public async Task QdrantProcessSettings_SaveOnlyThroughGlobalRuntimeCommand()
    {
        var backend = DispatchProxy.Create<IAriadneBackendClient, RuntimeBackend>();
        var proxy = (RuntimeBackend)(object)backend;
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);

        Assert.True(await vm.ReloadAppRuntimeForTestsAsync());
        Assert.Equal("/opt/qdrant-a", vm.QdrantBinaryPath);
        Assert.Equal("42000", vm.QdrantStartupTimeoutMs);

        vm.QdrantBinaryPath = "/opt/qdrant-b";
        vm.QdrantStartupTimeoutMs = "51000";
        Assert.True(await vm.SaveAppRuntimeForTestsAsync());

        Assert.Equal(
            new AppRuntimeSettings("/opt/qdrant-b", 51_000),
            proxy.SavedSettings);
        Assert.Equal(0, proxy.SaveMiscCalls);
    }

    private class RuntimeBackend : DispatchProxy
    {
        public AppRuntimeSettings? SavedSettings { get; private set; }
        public int SaveMiscCalls { get; private set; }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            return targetMethod?.Name switch
            {
                nameof(IAriadneBackendClient.GetAppRuntimeSettingsAsync) =>
                    Task.FromResult(new AppRuntimeSettings("/opt/qdrant-a", 42_000)),
                nameof(IAriadneBackendClient.SaveAppRuntimeSettingsAsync) =>
                    Save((AppRuntimeSettings)args![0]!),
                nameof(IAriadneBackendClient.SaveMiscSectionSettingsAsync) => CountMisc(),
                _ => UnsupportedTask(targetMethod),
            };
        }

        private Task<AppRuntimeSettings> Save(AppRuntimeSettings settings)
        {
            SavedSettings = settings;
            return Task.FromResult(settings);
        }

        private Task<MiscSectionSettings> CountMisc()
        {
            SaveMiscCalls++;
            throw new InvalidOperationException("project misc save must not be used");
        }

        private static object UnsupportedTask(MethodInfo? method)
        {
            var returnType = method?.ReturnType ?? typeof(Task);
            var exception = new NotSupportedException(method?.Name);
            if (returnType == typeof(Task))
            {
                return Task.FromException(exception);
            }
            var resultType = returnType.GetGenericArguments()[0];
            return typeof(Task)
                .GetMethod(nameof(Task.FromException), 1, new[] { typeof(Exception) })!
                .MakeGenericMethod(resultType)
                .Invoke(null, new object[] { exception })!;
        }
    }
}
