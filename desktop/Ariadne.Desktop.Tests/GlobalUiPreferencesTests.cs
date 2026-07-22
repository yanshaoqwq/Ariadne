using System.Reflection;
using Avalonia.Media;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class GlobalUiPreferencesTests
{
    [Fact]
    public void GlobalPreferences_AreAppliedToEveryPersonalizedProjectPage()
    {
        var names = DisplayNameService.LoadDefault();
        var backend = EmptyBackendProxy.Create();
        var preferences = Preferences(
            projectPanelVisible: true,
            new Dictionary<string, bool>
            {
                ["workspace.right_panel"] = true,
                ["works.right_panel"] = false,
                ["git.right_panel"] = false,
            });
        var workspace = new WorkspacePageViewModel(names, backend);
        var works = new WorksPageViewModel(names, backend);
        var git = new GitPageViewModel(names, backend);

        workspace.ApplyUiPreferences(preferences);
        works.ApplyUiPreferences(preferences);
        git.ApplyUiPreferences(preferences);

        Assert.True(workspace.IsRightPanelOpen);
        Assert.True(works.IsProjectPanelVisible);
        Assert.False(works.IsRightPanelOpen);
        Assert.False(git.IsRightPanelOpen);
    }

    [Fact]
    public void ProjectPanelPreference_HidesProjectPanelButKeepsImportPanelReachable()
    {
        var works = new WorksPageViewModel(DisplayNameService.LoadDefault(), EmptyBackendProxy.Create());
        works.ApplyUiPreferences(Preferences(projectPanelVisible: false));

        Assert.False(works.IsProjectPanelVisible);
        Assert.False(works.IsRightPanelToggleVisible);
        Assert.False(works.IsRightPanelVisible);

        works.OpenImportPanelCommand.Execute(null);

        Assert.True(works.IsImportPanelOpen);
        Assert.True(works.IsRightPanelToggleVisible);
        Assert.True(works.IsRightPanelVisible);
    }

    [Theory]
    [InlineData("workspace.right_panel")]
    [InlineData("works.right_panel")]
    [InlineData("git.right_panel")]
    public void UserPanelToggle_PersistsTheGlobalPanelKey(string expectedKey)
    {
        var names = DisplayNameService.LoadDefault();
        var backend = EmptyBackendProxy.Create();
        string? persistedKey = null;
        bool? persistedValue = null;
        Task Persist(string key, bool isOpen)
        {
            persistedKey = key;
            persistedValue = isOpen;
            return Task.CompletedTask;
        }

        var page = expectedKey switch
        {
            "workspace.right_panel" => (IUiPreferencesAware)new WorkspacePageViewModel(names, backend, Persist),
            "works.right_panel" => new WorksPageViewModel(names, backend, Persist),
            _ => new GitPageViewModel(names, backend, persistPanelState: Persist),
        };
        page.ApplyUiPreferences(Preferences(
            projectPanelVisible: true,
            new Dictionary<string, bool> { [expectedKey] = true }));

        switch (page)
        {
            case WorkspacePageViewModel workspace:
                workspace.ToggleRightPanelCommand.Execute(null);
                break;
            case WorksPageViewModel works:
                works.ToggleRightPanelCommand.Execute(null);
                break;
            case GitPageViewModel git:
                git.ToggleRightPanelCommand.Execute(null);
                break;
        }

        Assert.Equal(expectedKey, persistedKey);
        Assert.False(persistedValue);
    }

    [Fact]
    public void GitCheckpointMarker_UsesGlobalPersonalizationColors()
    {
        var names = DisplayNameService.LoadDefault();
        var item = new GitHistoryItemViewModel(
            "1234567890",
            "checkpoint",
            Array.Empty<string>(),
            Array.Empty<string>(),
            0,
            null,
            "auto",
            isAutoCheckpoint: true,
            isManualCheckpoint: false,
            "#112233",
            "#445566",
            isHead: false,
            laneIndex: 0,
            "HEAD",
            "merge",
            "details",
            "restore",
            "copy",
            names,
            _ => { },
            _ => { },
            _ => Task.CompletedTask,
            _ => Task.CompletedTask,
            () => true);

        Assert.Equal(Color.Parse("#112233"), Assert.IsType<SolidColorBrush>(item.MarkerBrush).Color);

        item.ApplyMarkerColors("#aabbcc", "#ddeeff");

        Assert.Equal(Color.Parse("#aabbcc"), Assert.IsType<SolidColorBrush>(item.MarkerBrush).Color);
    }

    [Fact]
    public async Task PersonalizationSave_PreservesNewerGlobalRuntimeMetadata()
    {
        var backend = PreferenceBackendProxy.Create(out var proxy);
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend);
        var current = Preferences(
            projectPanelVisible: true,
            new Dictionary<string, bool> { ["workspace.right_panel"] = true }) with
        {
            OnboardingSeen = true,
            ProjectPanelPosition = new[] { 10, 20 },
        };
        window.ApplyGlobalPreferencesForTests(current);
        var staleFormSnapshot = Preferences(
            projectPanelVisible: false,
            new Dictionary<string, bool> { ["workspace.right_panel"] = false }) with
        {
            Theme = "dark",
            OnboardingSeen = false,
            ProjectPanelPosition = null,
        };

        await window.SaveGlobalPreferencesForTestsAsync(staleFormSnapshot);

        Assert.NotNull(proxy.SavedPreferences);
        Assert.Equal("dark", proxy.SavedPreferences!.Theme);
        Assert.False(proxy.SavedPreferences.ProjectPanelVisible);
        Assert.True(proxy.SavedPreferences.PanelStates["workspace.right_panel"]);
        Assert.True(proxy.SavedPreferences.OnboardingSeen);
        Assert.Equal(new[] { 10, 20 }, proxy.SavedPreferences.ProjectPanelPosition);
    }

    [Fact]
    public async Task PanelPreference_OldSaveCompletionCannotRollbackNewerUserIntent()
    {
        var backend = SequencedPreferenceBackendProxy.Create(out var proxy);
        var window = new MainWindowViewModel(DisplayNameService.LoadDefault(), backend);
        window.ApplyGlobalPreferencesForTests(Preferences(
            panelStates: new Dictionary<string, bool> { ["workspace.right_panel"] = true }));
        var workspace = Assert.IsType<WorkspacePageViewModel>(window.GetPageForTests("workspace"));

        var first = window.PersistPanelStateForTestsAsync("workspace.right_panel", false);
        await proxy.SaveStarted[0].Task;
        var second = window.PersistPanelStateForTestsAsync("workspace.right_panel", true);
        Assert.True(workspace.IsRightPanelOpen);

        proxy.SaveRelease[0].TrySetResult(true);
        await proxy.SaveStarted[1].Task;
        Assert.True(workspace.IsRightPanelOpen);

        var third = window.PersistPanelStateForTestsAsync("workspace.right_panel", false);
        Assert.False(workspace.IsRightPanelOpen);
        proxy.SaveRelease[1].TrySetResult(true);
        await proxy.SaveStarted[2].Task;
        proxy.SaveRelease[2].TrySetResult(true);
        await Task.WhenAll(first, second, third);

        Assert.False(workspace.IsRightPanelOpen);
        Assert.False(proxy.SavedPreferences.Last().PanelStates["workspace.right_panel"]);
    }

    private static UiPreferences Preferences(
        bool projectPanelVisible = true,
        Dictionary<string, bool>? panelStates = null) => new(
        "system",
        "#112233",
        "#445566",
        projectPanelVisible,
        null,
        panelStates ?? new Dictionary<string, bool>(),
        false,
        ReduceMotion: true,
        Locale: "zh");

    private class EmptyBackendProxy : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            DispatchProxy.Create<IAriadneBackendClient, EmptyBackendProxy>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args) =>
            throw new InvalidOperationException($"Unexpected backend call: {targetMethod?.Name}");
    }

    private class PreferenceBackendProxy : DispatchProxy
    {
        public UiPreferences? SavedPreferences { get; private set; }

        public static IAriadneBackendClient Create(out PreferenceBackendProxy proxy)
        {
            var client = DispatchProxy.Create<IAriadneBackendClient, PreferenceBackendProxy>();
            proxy = (PreferenceBackendProxy)(object)client;
            return client;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == nameof(IAriadneBackendClient.SaveUiPreferencesAsync))
            {
                SavedPreferences = Assert.IsType<UiPreferences>(args![0]);
                return Task.CompletedTask;
            }
            throw new InvalidOperationException($"Unexpected backend call: {targetMethod?.Name}");
        }
    }

    private class SequencedPreferenceBackendProxy : DispatchProxy
    {
        private int _saveIndex;

        public TaskCompletionSource<bool>[] SaveStarted { get; } =
            Enumerable.Range(0, 3)
                .Select(_ => new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously))
                .ToArray();
        public TaskCompletionSource<bool>[] SaveRelease { get; } =
            Enumerable.Range(0, 3)
                .Select(_ => new TaskCompletionSource<bool>(TaskCreationOptions.RunContinuationsAsynchronously))
                .ToArray();
        public List<UiPreferences> SavedPreferences { get; } = new();

        public static IAriadneBackendClient Create(out SequencedPreferenceBackendProxy proxy)
        {
            var client = DispatchProxy.Create<IAriadneBackendClient, SequencedPreferenceBackendProxy>();
            proxy = (SequencedPreferenceBackendProxy)(object)client;
            return client;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name != nameof(IAriadneBackendClient.SaveUiPreferencesAsync))
            {
                throw new InvalidOperationException($"Unexpected backend call: {targetMethod?.Name}");
            }

            var index = Interlocked.Increment(ref _saveIndex) - 1;
            SavedPreferences.Add(Assert.IsType<UiPreferences>(args![0]));
            SaveStarted[index].TrySetResult(true);
            return SaveRelease[index].Task;
        }
    }
}
