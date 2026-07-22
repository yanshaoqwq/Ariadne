using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class SettingsProviderRefreshRaceTests
{
    [Fact]
    public async Task LateProviderSuccess_CannotOverwriteNewEditor()
    {
        var backend = RefreshBackend.Create(out var proxy);
        var late = NewSource<ProviderModelsResult>();
        proxy.Enqueue(late.Task);
        var vm = NewViewModel(backend);

        var refresh = vm.RefreshProviderModelsForTestsAsync();
        vm.SelectProviderForTests("provider-b");
        late.SetResult(Result("provider-a", "a-late"));
        await refresh;

        Assert.Equal("provider-b", vm.ProviderId);
        Assert.Contains("b-initial", vm.ModelsText, StringComparison.Ordinal);
        Assert.DoesNotContain("a-late", vm.ModelsText, StringComparison.Ordinal);
        Assert.False(vm.HasUnsavedChanges);
        Assert.Equal(1, proxy.Calls);
    }

    [Fact]
    public async Task LateProviderFailure_CannotReplaceCurrentStatus()
    {
        var backend = RefreshBackend.Create(out var proxy);
        var late = NewSource<ProviderModelsResult>();
        proxy.Enqueue(late.Task);
        var vm = NewViewModel(backend);

        var refresh = vm.RefreshProviderModelsForTestsAsync();
        vm.SelectProviderForTests("provider-b");
        var status = vm.StatusText;
        late.SetException(new InvalidOperationException("provider-a failed late"));
        await refresh;

        Assert.Equal("provider-b", vm.ProviderId);
        Assert.Equal(status, vm.StatusText);
        Assert.False(vm.HasUnsavedChanges);
        Assert.Equal(1, proxy.Calls);
    }

    [Fact]
    public async Task NewerRefreshWinsWhenResponsesCompleteOutOfOrder()
    {
        var backend = RefreshBackend.Create(out var proxy);
        var first = NewSource<ProviderModelsResult>();
        var second = NewSource<ProviderModelsResult>();
        proxy.Enqueue(first.Task);
        proxy.Enqueue(second.Task);
        var vm = NewViewModel(backend);

        var oldRefresh = vm.RefreshProviderModelsForTestsAsync();
        var newRefresh = vm.RefreshProviderModelsForTestsAsync();
        second.SetResult(Result("provider-a", "a-new"));
        await newRefresh;
        first.SetResult(Result("provider-a", "a-old"));
        await oldRefresh;

        Assert.Contains("a-new", vm.ModelsText, StringComparison.Ordinal);
        Assert.DoesNotContain("a-old", vm.ModelsText, StringComparison.Ordinal);
        Assert.Equal(2, proxy.Calls);
    }

    [Fact]
    public async Task PageReloadInvalidatesPendingRefresh()
    {
        var backend = RefreshBackend.Create(out var proxy);
        var late = NewSource<ProviderModelsResult>();
        proxy.Enqueue(late.Task);
        var vm = NewViewModel(backend);

        var refresh = vm.RefreshProviderModelsForTestsAsync();
        vm.ApplyProviderConfigForTests(new ProviderConfigStatus(
            false,
            false,
            false,
            "provider-a",
            null,
            null,
            null,
            new[] { Provider("provider-a", "a-reloaded") }));
        late.SetResult(Result("provider-a", "a-stale"));
        await refresh;

        Assert.Contains("a-reloaded", vm.ModelsText, StringComparison.Ordinal);
        Assert.DoesNotContain("a-stale", vm.ModelsText, StringComparison.Ordinal);
        Assert.False(vm.HasUnsavedChanges);
        Assert.Equal(1, proxy.Calls);
    }

    [Fact]
    public async Task ProviderRemovalInvalidatesPendingRefresh()
    {
        var backend = RefreshBackend.Create(out var proxy);
        var late = NewSource<ProviderModelsResult>();
        proxy.Enqueue(late.Task);
        var vm = NewViewModel(backend);

        var refresh = vm.RefreshProviderModelsForTestsAsync();
        vm.ApplyProviderConfigForTests(new ProviderConfigStatus(
            false,
            false,
            false,
            "provider-b",
            null,
            null,
            null,
            new[] { Provider("provider-b", "b-after-removal") }));
        late.SetResult(Result("provider-a", "a-removed-late"));
        await refresh;

        Assert.Equal("provider-b", vm.ProviderId);
        Assert.Single(vm.ProviderOptions);
        Assert.DoesNotContain("a-removed-late", vm.ModelsText, StringComparison.Ordinal);
        Assert.False(vm.HasUnsavedChanges);
        Assert.Equal(1, proxy.Calls);
    }

    private static SettingsPageViewModel NewViewModel(IAriadneBackendClient backend)
    {
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);
        vm.ApplyProviderConfigForTests(Status());
        return vm;
    }

    private static ProviderConfigStatus Status() => new(
        false,
        false,
        false,
        "provider-a",
        null,
        null,
        null,
        new[]
        {
            Provider("provider-a", "a-initial"),
            Provider("provider-b", "b-initial"),
        });

    private static ProviderKeyStatus Provider(string id, string model) => new(
        id,
        id,
        "open_ai_compatible",
        true,
        true,
        "https://example.invalid",
        new[] { new ModelConfig(model, "llm", null, null, null) },
        false);

    private static ProviderModelsResult Result(string id, string model) =>
        new(id, new[] { new ModelConfig(model, "llm", null, null, null) });

    private static TaskCompletionSource<T> NewSource<T>() =>
        new(TaskCreationOptions.RunContinuationsAsynchronously);

    private class RefreshBackend : DispatchProxy
    {
        private readonly Queue<Task<ProviderModelsResult>> _responses = new();

        public int Calls { get; private set; }

        public static IAriadneBackendClient Create(out RefreshBackend proxy)
        {
            var client = Create<IAriadneBackendClient, RefreshBackend>();
            proxy = (RefreshBackend)(object)client;
            return client;
        }

        public void Enqueue(Task<ProviderModelsResult> response) => _responses.Enqueue(response);

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == nameof(IAriadneBackendClient.FetchProviderModelsAsync))
            {
                Calls++;
                return _responses.Dequeue();
            }

            return targetMethod?.Name switch
            {
                "get_HasProjectRoot" => true,
                _ => UnsupportedTask(targetMethod),
            };
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
