using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class SettingsDraftStateTests
{
    [Fact]
    public void SaveCompletionConfirmsOnlySubmittedGeneration()
    {
        var state = new SettingsDraftState();
        var generation = state.BeginLoad();
        Assert.True(state.AcceptLoaded(generation, "general", Values(("name", "before"))));

        var attempt = state.TryBeginSave("general", Values(("name", "submitted")));
        Assert.NotNull(attempt);
        var editedDuringSave = Values(("name", "edited-during-save"));

        Assert.True(state.HasUnsubmittedChanges(editedDuringSave));
        Assert.True(state.CompleteSave(attempt!));
        Assert.True(state.IsDirty(editedDuringSave));
    }

    [Fact]
    public void FailureKeepsBaselineAndConcurrentSaveIsRejected()
    {
        var state = new SettingsDraftState();
        var generation = state.BeginLoad();
        state.AcceptLoaded(generation, "misc", Values(("port", "6333")));
        var attempt = state.TryBeginSave("misc", Values(("port", "6334")));

        Assert.NotNull(attempt);
        Assert.Null(state.TryBeginSave("misc", Values(("port", "6335"))));
        state.FailSave(attempt!);
        Assert.True(state.IsDirty(Values(("port", "6334"))));
        Assert.False(state.IsSaving("misc"));
    }

    [Fact]
    public void FailedLoadDoesNotCreateEditableBaseline()
    {
        var state = new SettingsDraftState();
        var first = state.BeginLoad();
        state.AcceptLoaded(first, "general", Values(("name", "loaded")));
        var second = state.BeginLoad();

        Assert.False(state.IsLoaded("general"));
        Assert.False(state.AcceptLoaded(first, "general", Values(("name", "stale"))));
        Assert.True(state.AcceptLoaded(second, "models", Values(("id", "openai"))));
        Assert.True(state.IsLoaded("models"));
    }

    [Fact]
    public void SaveCompletionUsesCanonicalPersistedValuesAsBaseline()
    {
        var state = new SettingsDraftState();
        var generation = state.BeginLoad();
        state.AcceptLoaded(generation, "automation", Values(("budget", "0")));
        var attempt = state.TryBeginSave("automation", Values(("budget", "01.000")));

        Assert.NotNull(attempt);
        Assert.True(state.CompleteSave(attempt!, Values(("budget", "1"))));
        Assert.False(state.IsDirty(Values(("budget", "1"))));
        Assert.True(state.IsDirty(Values(("budget", "01.000"))));
    }

    private static IReadOnlyDictionary<string, string> Values(params (string Key, string Value)[] values) =>
        values.ToDictionary(item => item.Key, item => item.Value, StringComparer.Ordinal);
}
