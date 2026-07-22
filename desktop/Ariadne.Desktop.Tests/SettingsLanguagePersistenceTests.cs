using System.Reflection;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

[Collection("GlobalDialogService")]
public sealed class SettingsLanguagePersistenceTests
{
    [Fact]
    public void LanguageSelector_BelongsToGlobalPersonalizationSaveScope()
    {
        var view = File.ReadAllText(ResolveDesktopSource("Views", "SettingsPageView.axaml"));
        var viewModel = File.ReadAllText(ResolveDesktopSource("ViewModels", "SettingsPageViewModel.cs"));
        var generalStart = view.IndexOf("IsGeneralSelected", StringComparison.Ordinal);
        var modelsStart = view.IndexOf("IsModelsSelected", generalStart, StringComparison.Ordinal);
        var personalizationStart = view.IndexOf("IsPersonalizationSelected", modelsStart, StringComparison.Ordinal);
        var miscStart = view.IndexOf("IsMiscSelected", personalizationStart, StringComparison.Ordinal);

        Assert.True(generalStart >= 0 && modelsStart > generalStart
            && personalizationStart > modelsStart && miscStart > personalizationStart);
        Assert.DoesNotContain("SelectedLanguage", view[generalStart..modelsStart], StringComparison.Ordinal);
        Assert.Contains("SelectedLanguage", view[personalizationStart..miscStart], StringComparison.Ordinal);
        Assert.DoesNotContain("Locale = language", viewModel, StringComparison.Ordinal);
        Assert.Contains("ApplySavedLanguage(preferences.Locale)", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("PersistLanguageAsync", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("SaveAppSettingsAsync", viewModel, StringComparison.Ordinal);
        Assert.Contains("nameof(ProviderScopeHelpText)", viewModel, StringComparison.Ordinal);

        var mainWindow = File.ReadAllText(ResolveDesktopSource("ViewModels", "MainWindowViewModel.cs"));
        // Product applies locale via ApplyGlobalPreferences(status.Preferences) → preferences.Locale
        // (not the brittle continuous substring status.Preferences.Locale).
        Assert.Contains("ApplyGlobalPreferences(status.Preferences)", mainWindow, StringComparison.Ordinal);
        Assert.Contains("preferences.Locale", mainWindow, StringComparison.Ordinal);
        Assert.DoesNotContain("ApplySavedLanguageAsync", mainWindow, StringComparison.Ordinal);
    }

    [Fact]
    public async Task LanguageSelection_IsSavedOnlyThroughGlobalPersonalizationTransaction()
    {
        using var resources = CreateLanguageResources();
        var names = DisplayNameService.LoadFromDirectory(resources.Path, "zh");
        var backend = LanguageBackend.Create();
        var vm = new SettingsPageViewModel(names, backend.Client);
        await vm.ReloadProjectDataAsync();

        vm.SelectedLanguage = "fr";

        Assert.Equal("fr", names.CurrentLanguage);
        Assert.Equal("zh", vm.Locale);
        Assert.True(vm.HasUnsavedChanges);
        Assert.Equal(0, backend.SaveUiPreferencesCalls);
        Assert.Null(backend.SavedGeneral);
        Assert.Null(backend.SavedPreferences);

        Assert.True(await vm.SaveUnsavedChangesAsync());

        Assert.Equal("fr", backend.SavedPreferences!.Locale);
        Assert.Null(backend.SavedGeneral);
        Assert.Equal(1, backend.SaveUiPreferencesCalls);
        Assert.False(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task FailedLanguageSave_KeepsPreviewAsRetryableDraft()
    {
        using var resources = CreateLanguageResources();
        var names = DisplayNameService.LoadFromDirectory(resources.Path, "zh");
        var backend = LanguageBackend.Create();
        var vm = new SettingsPageViewModel(names, backend.Client);
        await vm.ReloadProjectDataAsync();
        backend.SaveFailure = new InvalidOperationException("injected save failure");

        vm.SelectedLanguage = "fr";

        Assert.False(await vm.SaveUnsavedChangesAsync());
        Assert.Equal("fr", names.CurrentLanguage);
        Assert.Equal("zh", vm.Locale);
        Assert.True(vm.HasUnsavedChanges);

        backend.SaveFailure = null;
        Assert.True(await vm.SaveUnsavedChangesAsync());
        Assert.Equal("fr", backend.SavedPreferences!.Locale);
        Assert.False(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task LanguageEditedDuringSave_RemainsDirtyAndLastSelectionWins()
    {
        using var resources = CreateLanguageResources();
        var names = DisplayNameService.LoadFromDirectory(resources.Path, "zh");
        var backend = LanguageBackend.Create();
        var vm = new SettingsPageViewModel(names, backend.Client);
        await vm.ReloadProjectDataAsync();
        backend.HoldNextSave();
        vm.SelectedLanguage = "fr";

        var firstSave = vm.SaveUnsavedChangesAsync();
        await backend.SaveStarted.Task;
        vm.SelectedLanguage = "zh";
        backend.ReleaseHeldSave();

        Assert.False(await firstSave);
        Assert.Equal("zh", names.CurrentLanguage);
        Assert.Equal("zh", vm.Locale);
        Assert.True(vm.HasUnsavedChanges);

        Assert.True(await vm.SaveUnsavedChangesAsync());
        Assert.Equal("zh", backend.SavedPreferences!.Locale);
        Assert.False(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task SaveAllDirtySections_StopsAfterFirstSectionFailure()
    {
        using var resources = CreateLanguageResources();
        var names = DisplayNameService.LoadFromDirectory(resources.Path, "zh");
        var backend = LanguageBackend.Create();
        var vm = new SettingsPageViewModel(names, backend.Client);
        await vm.ReloadProjectDataAsync();
        backend.GeneralSaveFailure = new InvalidOperationException("injected general failure");
        vm.ProjectName = "Changed";
        vm.SelectedLanguage = "fr";

        Assert.False(await vm.SaveUnsavedChangesAsync());

        Assert.Equal(1, backend.SaveGeneralCalls);
        Assert.Equal(0, backend.SaveUiPreferencesCalls);
        Assert.True(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task PreparedSettingsRejectEditsMadeBeforeCommitWithoutWriting()
    {
        using var resources = CreateLanguageResources();
        var names = DisplayNameService.LoadFromDirectory(resources.Path, "zh");
        var backend = LanguageBackend.Create();
        var vm = new SettingsPageViewModel(names, backend.Client);
        await vm.ReloadProjectDataAsync();
        vm.ProjectName = "Prepared";

        Assert.True(await vm.PrepareUnsavedChangesAsync());
        vm.ProjectName = "Edited after prepare";

        Assert.False(await vm.CommitPreparedUnsavedChangesAsync());
        Assert.Equal(0, backend.SaveGeneralCalls);
        Assert.True(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task PreparedSettingsUseImmutableDtosWhenAnotherSectionChangesDuringCommit()
    {
        using var resources = CreateLanguageResources();
        var names = DisplayNameService.LoadFromDirectory(resources.Path, "zh");
        var backend = LanguageBackend.Create();
        var vm = new SettingsPageViewModel(names, backend.Client);
        await vm.ReloadProjectDataAsync();
        vm.ProjectName = "Prepared";
        vm.SelectedLanguage = "fr";
        backend.HoldNextSave();

        Assert.True(await vm.PrepareUnsavedChangesAsync());
        var commit = vm.CommitPreparedUnsavedChangesAsync();
        await backend.SaveStarted.Task;
        vm.SelectedLanguage = "zh";
        backend.ReleaseHeldSave();

        Assert.False(await commit);
        Assert.Equal("Prepared", backend.SavedGeneral!.App.App.ProjectName);
        Assert.Equal("fr", backend.SavedPreferences!.Locale);
        Assert.Equal("zh", vm.SelectedLanguage);
        Assert.True(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task NavigationSelection_UsesLatestIntentWhileLeaveSaveIsPending()
    {
        using var resources = CreateLanguageResources();
        var names = DisplayNameService.LoadFromDirectory(resources.Path, "zh");
        DialogService.Initialize(names);
        var backend = LanguageBackend.Create();
        var vm = new SettingsPageViewModel(names, backend.Client);
        await vm.ReloadProjectDataAsync();
        vm.ProjectName = "Changed";

        var first = vm.SelectNavigationTabForTestsAsync("automation");
        var dialog = await WaitForDialogAsync();
        var latest = vm.SelectNavigationTabForTestsAsync("permissions");
        Assert.Same(first, latest);
        dialog.Buttons.Single(button =>
            button.ResultIndex == (int)UnsavedLeaveChoice.Save).Command!.Execute(null);
        await Task.WhenAll(first, latest);

        Assert.Equal("permissions", vm.SelectedTab.Id);
        Assert.Equal(1, backend.SaveGeneralCalls);
        Assert.False(DialogService.Current.IsOpen);
    }

    [Fact]
    public async Task NavigationSelection_ClickingCurrentTabCancelsPendingTarget()
    {
        using var resources = CreateLanguageResources();
        var names = DisplayNameService.LoadFromDirectory(resources.Path, "zh");
        DialogService.Initialize(names);
        var backend = LanguageBackend.Create();
        var vm = new SettingsPageViewModel(names, backend.Client);
        await vm.ReloadProjectDataAsync();
        vm.ProjectName = "Changed";

        var navigation = vm.SelectNavigationTabForTestsAsync("automation");
        var dialog = await WaitForDialogAsync();
        vm.NavigationSelection = vm.SelectedTab;
        dialog.Buttons.Single(button =>
            button.ResultIndex == (int)UnsavedLeaveChoice.Save).Command!.Execute(null);
        await navigation;

        Assert.Equal("general", vm.SelectedTab.Id);
        Assert.Equal(1, backend.SaveGeneralCalls);
        Assert.False(DialogService.Current.IsOpen);
    }

    [Fact]
    public void LanguagePreview_RefreshesEveryVisibleScopeDescription()
    {
        using var resources = CreateLanguageResources();
        var names = DisplayNameService.LoadFromDirectory(resources.Path, "zh");
        var backend = LanguageBackend.Create();
        var vm = new SettingsPageViewModel(names, backend.Client);
        var changed = new HashSet<string>(StringComparer.Ordinal);
        vm.PropertyChanged += (_, args) =>
        {
            if (args.PropertyName is not null)
            {
                changed.Add(args.PropertyName);
            }
        };

        vm.SelectedLanguage = "fr";

        Assert.Equal("Providers global FR", vm.ProviderScopeHelpText);
        Assert.Equal("Presets mixed FR", vm.PresetScopeHelpText);
        Assert.Equal("Permissions global FR", vm.PermissionsScopeHelpText);
        Assert.Equal("Personalization global FR", vm.PersonalizationScopeHelpText);
        Assert.Equal("General project FR", vm.GeneralScopeHelpText);
        Assert.Equal("Automation project FR", vm.AutomationScopeHelpText);
        Assert.Equal("Runtime global FR", vm.AppRuntimeScopeHelpText);
        Assert.Equal("Retrieval project FR", vm.RetrievalScopeHelpText);
        Assert.Contains(nameof(SettingsPageViewModel.ProviderScopeHelpText), changed);
        Assert.Contains(nameof(SettingsPageViewModel.PresetScopeHelpText), changed);
        Assert.Contains(nameof(SettingsPageViewModel.PermissionsScopeHelpText), changed);
        Assert.Contains(nameof(SettingsPageViewModel.PersonalizationScopeHelpText), changed);
        Assert.Contains(nameof(SettingsPageViewModel.GeneralScopeHelpText), changed);
        Assert.Contains(nameof(SettingsPageViewModel.AutomationScopeHelpText), changed);
        Assert.Contains(nameof(SettingsPageViewModel.AppRuntimeScopeHelpText), changed);
        Assert.Contains(nameof(SettingsPageViewModel.RetrievalScopeHelpText), changed);
    }

    private static TemporaryDirectory CreateLanguageResources()
    {
        var directory = new TemporaryDirectory("language-resources");
        File.WriteAllText(
            System.IO.Path.Combine(directory.Path, "display_name.json"),
            JsonSerializer.Serialize(new Dictionary<string, string>
            {
                ["ui.settings.misc.language.zh"] = "Chinese",
                ["ui.common.configured"] = "Configured",
                ["ui.common.loading"] = "Loading",
                ["ui.settings.status.unsaved"] = "Unsaved",
                ["ui.settings.status.saving"] = "Saving",
                ["ui.settings.status.section_load_failed"] = "Load failed",
                ["ui.error.unknown"] = "Save failed",
                ["ui.settings.models.scope_help"] = "Providers global ZH",
                ["ui.settings.presets.scope_help"] = "Presets mixed ZH",
                ["ui.settings.permissions.scope_help"] = "Permissions global ZH",
                ["ui.settings.personalization.scope_help"] = "Personalization global ZH",
                ["ui.settings.general.scope_help"] = "General project ZH",
                ["ui.settings.automation.scope_help"] = "Automation project ZH",
                ["ui.settings.misc.app_runtime_scope_help"] = "Runtime global ZH",
                ["ui.settings.misc.retrieval_scope_help"] = "Retrieval project ZH",
            }));
        File.WriteAllText(
            System.IO.Path.Combine(directory.Path, "display_name.fr.json"),
            JsonSerializer.Serialize(new Dictionary<string, string>
            {
                ["ui.settings.misc.language.fr"] = "Francais",
                ["ui.settings.models.scope_help"] = "Providers global FR",
                ["ui.settings.presets.scope_help"] = "Presets mixed FR",
                ["ui.settings.permissions.scope_help"] = "Permissions global FR",
                ["ui.settings.personalization.scope_help"] = "Personalization global FR",
                ["ui.settings.general.scope_help"] = "General project FR",
                ["ui.settings.automation.scope_help"] = "Automation project FR",
                ["ui.settings.misc.app_runtime_scope_help"] = "Runtime global FR",
                ["ui.settings.misc.retrieval_scope_help"] = "Retrieval project FR",
            }));
        return directory;
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        for (var depth = 0; directory is not null && depth < 10; depth++)
        {
            var candidate = System.IO.Path.Combine(
                new[] { directory.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            directory = directory.Parent;
        }

        throw new FileNotFoundException(string.Join('/', parts));
    }

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
        throw new TimeoutException("settings leave dialog was not shown");
    }

    private class LanguageBackend : DispatchProxy
    {
        private TaskCompletionSource? _heldSave;

        public IAriadneBackendClient Client { get; private set; } = null!;
        public GeneralSectionSettings? SavedGeneral { get; private set; }
        public UiPreferences? SavedPreferences { get; private set; }
        public Exception? SaveFailure { get; set; }
        public Exception? GeneralSaveFailure { get; set; }
        public int SaveGeneralCalls { get; private set; }
        public int SaveUiPreferencesCalls { get; private set; }
        public TaskCompletionSource SaveStarted { get; private set; } =
            new(TaskCreationOptions.RunContinuationsAsynchronously);

        public static LanguageBackend Create()
        {
            var client = DispatchProxy.Create<IAriadneBackendClient, LanguageBackend>();
            var backend = (LanguageBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        public void HoldNextSave()
        {
            SaveStarted = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            _heldSave = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        }

        public void ReleaseHeldSave()
        {
            var held = _heldSave ?? throw new InvalidOperationException("save is not held");
            _heldSave = null;
            held.SetResult();
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            return targetMethod.Name switch
            {
                nameof(IAriadneBackendClient.GetAppSettingsAsync) => Task.FromResult(InitialAppSettings()),
                nameof(IAriadneBackendClient.ReadProjectMemoryAsync) => Task.FromResult(string.Empty),
                nameof(IAriadneBackendClient.GetCurrentProjectAsync) => Task.FromResult<CurrentProjectStatus?>(
                    new CurrentProjectStatus("/tmp/ariadne-language-test", "Ariadne")),
                nameof(IAriadneBackendClient.GetUiPreferencesAsync) => Task.FromResult(InitialPreferences()),
                nameof(IAriadneBackendClient.SaveUiPreferencesAsync) => SavePreferences(
                    (UiPreferences)args![0]!),
                nameof(IAriadneBackendClient.SaveGeneralSectionSettingsAsync) => SaveGeneral(
                    (GeneralSectionSettings)args![0]!),
                "get_HasProjectRoot" => true,
                _ => UnsupportedTask(targetMethod),
            };
        }

        private async Task<GeneralSectionSettings> SaveGeneral(GeneralSectionSettings settings)
        {
            SavedGeneral = settings;
            SaveGeneralCalls++;
            SaveStarted.TrySetResult();
            if (GeneralSaveFailure is not null)
            {
                throw GeneralSaveFailure;
            }
            var held = _heldSave;
            if (held is not null)
            {
                await held.Task;
            }
            return settings;
        }

        private Task SavePreferences(UiPreferences preferences)
        {
            SavedPreferences = preferences;
            SaveUiPreferencesCalls++;
            SaveStarted.TrySetResult();
            if (SaveFailure is not null)
            {
                return Task.FromException(SaveFailure);
            }
            return _heldSave?.Task ?? Task.CompletedTask;
        }

        private static UiPreferences InitialPreferences() => new(
            "system",
            "#8a8f98",
            "#f59e0b",
            true,
            null,
            new Dictionary<string, bool>(),
            false,
            Locale: "zh");

        private static AppSettings InitialAppSettings() => new(new AppConfig(
            1,
            "Ariadne",
            "zh",
            "documents",
            "workflows",
            "skills",
            "exports"));

        private static object? UnsupportedTask(MethodInfo method)
        {
            if (method.ReturnType == typeof(void))
            {
                return null;
            }
            if (method.ReturnType == typeof(Task))
            {
                return Task.FromException(new NotSupportedException(method.Name));
            }
            if (method.ReturnType.IsGenericType
                && method.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = method.ReturnType.GetGenericArguments()[0];
                return typeof(Task)
                    .GetMethods(BindingFlags.Public | BindingFlags.Static)
                    .Single(candidate => candidate.Name == nameof(Task.FromException)
                        && candidate.IsGenericMethodDefinition)
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new object[] { new NotSupportedException(method.Name) });
            }
            return method.ReturnType.IsValueType ? Activator.CreateInstance(method.ReturnType) : null;
        }
    }
}
