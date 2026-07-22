using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

[Collection("GlobalDialogService")]
public sealed class SettingsProviderDraftTests
{
    [Fact]
    public async Task DiscardingNewDraftRemovesItFromInventory()
    {
        var names = DisplayNameService.LoadDefault();
        DialogService.Initialize(names);
        var vm = NewViewModel(names);

        await vm.AddProviderDraftForTestsAsync();
        var draftId = vm.ProviderId;
        Assert.True(vm.IsSelectedProviderDraft);
        Assert.Equal(3, vm.ProviderOptions.Count);

        var switching = vm.SelectProviderOptionForTestsAsync("provider-b");
        var dialog = await WaitForDialogAsync();
        dialog.Buttons.Single(button =>
            button.ResultIndex == (int)UnsavedLeaveChoice.Discard).Command!.Execute(null);
        await switching;

        Assert.Equal("provider-b", vm.ProviderId);
        Assert.DoesNotContain(vm.ProviderOptions, option => option.ProviderId == draftId);
        Assert.Equal(2, vm.ProviderOptions.Count);
        Assert.False(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task RepeatedAddAndDiscardDoesNotAccumulateGhostOptions()
    {
        var names = DisplayNameService.LoadDefault();
        DialogService.Initialize(names);
        var vm = NewViewModel(names);

        for (var index = 0; index < 2; index++)
        {
            await vm.AddProviderDraftForTestsAsync();
            var switching = vm.SelectProviderOptionForTestsAsync("provider-b");
            var dialog = await WaitForDialogAsync();
            dialog.Buttons.Single(button =>
                button.ResultIndex == (int)UnsavedLeaveChoice.Discard).Command!.Execute(null);
            await switching;
            Assert.Equal(2, vm.ProviderOptions.Count);
        }
    }

    [Fact]
    public void ManualModelDisclosureDoesNotMakeConfigurationDirty()
    {
        var names = DisplayNameService.LoadDefault();
        var vm = NewViewModel(names);

        vm.ManualModelsVisible = true;

        Assert.False(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task ProviderSelection_UsesLatestIntentWhileUnsavedDialogIsOpen()
    {
        var names = DisplayNameService.LoadDefault();
        DialogService.Initialize(names);
        var vm = NewViewModel(names, includeThirdProvider: true);
        await vm.SelectNavigationTabForTestsAsync("models");
        vm.ProviderDisplayName = "dirty provider";

        var first = vm.SelectProviderOptionForTestsAsync("provider-b");
        var dialog = await WaitForDialogAsync();
        var latest = vm.SelectProviderOptionForTestsAsync("provider-c");
        Assert.Same(first, latest);

        dialog.Buttons.Single(button =>
            button.ResultIndex == (int)UnsavedLeaveChoice.Discard).Command!.Execute(null);
        await Task.WhenAll(first, latest);

        Assert.Equal("provider-c", vm.ProviderId);
        Assert.Equal("provider-c", vm.SelectedProviderOption?.ProviderId);
        Assert.False(DialogService.Current.IsOpen);
    }

    private static SettingsPageViewModel NewViewModel(
        DisplayNameService names,
        bool includeThirdProvider = false)
    {
        var providers = new List<ProviderKeyStatus>
        {
            Provider("provider-a", "a-model"),
            Provider("provider-b", "b-model"),
        };
        if (includeThirdProvider)
        {
            providers.Add(Provider("provider-c", "c-model"));
        }
        var vm = new SettingsPageViewModel(names, NoopBackend.Create());
        vm.ApplyProviderConfigForTests(new ProviderConfigStatus(
            false,
            false,
            false,
            "provider-a",
            null,
            null,
            null,
            providers));
        return vm;
    }

    private static ProviderKeyStatus Provider(string id, string model) => new(
        id,
        id,
        "open_ai_compatible",
        true,
        true,
        "https://example.invalid",
        new[] { new ModelConfig(model, "llm", null, null, null) },
        false);

    private static async Task<ConfirmDialogViewModel> WaitForDialogAsync()
    {
        for (var attempt = 0; attempt < 100; attempt++)
        {
            if (DialogService.Current.ActiveDialog is { } dialog)
            {
                return dialog;
            }
            await Task.Delay(1);
        }
        throw new TimeoutException("unsaved provider dialog was not shown");
    }

    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args) =>
            throw new NotSupportedException(targetMethod?.Name);
    }
}
