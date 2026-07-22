using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

[Collection("GlobalDialogService")]
public sealed class MainWindowCloseGuardTests
{
    [Fact]
    public async Task CloseChecksDirtyCachedPageEvenWhenItIsNotCurrent()
    {
        var names = DisplayNameService.LoadDefault();
        DialogService.Initialize(names);
        var guard = new ControlledUnsavedGuard();
        var window = new MainWindowViewModel(
            names,
            NoopBackend.Create(),
            id => id == "works" ? guard : null,
            _ => { });

        _ = window.GetPageForTests("works");

        Assert.Same(window.Welcome, window.CurrentPage);
        Assert.True(window.HasCachedUnsavedChanges);

        var close = window.ConfirmCloseAsync();
        await WaitForDialogAsync();
        DialogService.Current.RequestCancelActive();

        Assert.False(await close);
        Assert.Equal(0, guard.PrepareCalls);
        Assert.Equal(0, guard.CommitCalls);
    }

    private static async Task WaitForDialogAsync()
    {
        for (var attempt = 0; attempt < 100 && !DialogService.Current.IsOpen; attempt++)
        {
            await Task.Yield();
        }
        Assert.True(DialogService.Current.IsOpen);
    }

    private sealed class ControlledUnsavedGuard : IUnsavedChangesGuard
    {
        public bool HasUnsavedChanges => true;
        public string UnsavedChangesPageId => "works";
        public string? PreparedUnsavedChangesPayloadIdentity => null;
        public string UnsavedChangesPageTitle => "Works";
        public int PrepareCalls { get; private set; }
        public int CommitCalls { get; private set; }

        public Task<bool> ConfirmLeaveIfNeededAsync() => Task.FromResult(false);

        public Task<bool> PrepareUnsavedChangesAsync()
        {
            PrepareCalls++;
            return Task.FromResult(true);
        }

        public Task<bool> CommitPreparedUnsavedChangesAsync()
        {
            CommitCalls++;
            return Task.FromResult(true);
        }

        public Task AbortPreparedUnsavedChangesAsync() => Task.CompletedTask;

        public Task<bool> SaveUnsavedChangesAsync() => Task.FromResult(true);

        public Task DiscardUnsavedChangesAsync() => Task.CompletedTask;
    }

    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args) =>
            throw new NotSupportedException(targetMethod?.Name);
    }
}
