using System.Reflection;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class SettingsLanguagePersistenceTests
{
    [Fact]
    public void LanguageSelector_BelongsToGeneralSaveScopeWithoutAutoSaveBypass()
    {
        var view = File.ReadAllText(ResolveDesktopSource("Views", "SettingsPageView.axaml"));
        var viewModel = File.ReadAllText(ResolveDesktopSource("ViewModels", "SettingsPageViewModel.cs"));
        var generalStart = view.IndexOf("IsGeneralSelected", StringComparison.Ordinal);
        var modelsStart = view.IndexOf("IsModelsSelected", generalStart, StringComparison.Ordinal);
        var miscStart = view.IndexOf("IsMiscSelected", modelsStart, StringComparison.Ordinal);

        Assert.True(generalStart >= 0 && modelsStart > generalStart && miscStart > modelsStart);
        Assert.Contains("SelectedLanguage", view[generalStart..modelsStart], StringComparison.Ordinal);
        Assert.DoesNotContain("SelectedLanguage", view[miscStart..], StringComparison.Ordinal);
        Assert.Contains("Locale = language", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("PersistLanguageAsync", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("SaveAppSettingsAsync", viewModel, StringComparison.Ordinal);
    }

    [Fact]
    public async Task LanguageSelection_IsSavedOnlyThroughGeneralTransaction()
    {
        using var resources = CreateLanguageResources();
        var names = DisplayNameService.LoadFromDirectory(resources.Path, "zh");
        var backend = LanguageBackend.Create();
        var vm = new SettingsPageViewModel(names, backend.Client);
        await vm.ReloadProjectDataAsync();

        vm.SelectedLanguage = "fr";

        Assert.Equal("fr", names.CurrentLanguage);
        Assert.Equal("fr", vm.Locale);
        Assert.True(vm.HasUnsavedChanges);
        Assert.Equal(0, backend.SaveAppSettingsCalls);
        Assert.Null(backend.SavedGeneral);

        Assert.True(await vm.SaveUnsavedChangesAsync());

        Assert.Equal("fr", backend.SavedGeneral!.App.App.Locale);
        Assert.Equal(0, backend.SaveAppSettingsCalls);
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
        Assert.Equal("fr", vm.Locale);
        Assert.True(vm.HasUnsavedChanges);

        backend.SaveFailure = null;
        Assert.True(await vm.SaveUnsavedChangesAsync());
        Assert.Equal("fr", backend.SavedGeneral!.App.App.Locale);
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
        Assert.Equal("zh", backend.SavedGeneral!.App.App.Locale);
        Assert.False(vm.HasUnsavedChanges);
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
            }));
        File.WriteAllText(
            System.IO.Path.Combine(directory.Path, "display_name.fr.json"),
            JsonSerializer.Serialize(new Dictionary<string, string>
            {
                ["ui.settings.misc.language.fr"] = "Francais",
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

    private class LanguageBackend : DispatchProxy
    {
        private TaskCompletionSource<GeneralSectionSettings>? _heldSave;

        public IAriadneBackendClient Client { get; private set; } = null!;
        public GeneralSectionSettings? SavedGeneral { get; private set; }
        public Exception? SaveFailure { get; set; }
        public int SaveAppSettingsCalls { get; private set; }
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
            _heldSave = new TaskCompletionSource<GeneralSectionSettings>(
                TaskCreationOptions.RunContinuationsAsynchronously);
        }

        public void ReleaseHeldSave()
        {
            var held = _heldSave ?? throw new InvalidOperationException("save is not held");
            _heldSave = null;
            held.SetResult(SavedGeneral!);
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
                nameof(IAriadneBackendClient.SaveGeneralSectionSettingsAsync) => SaveGeneral(
                    (GeneralSectionSettings)args![0]!),
                nameof(IAriadneBackendClient.SaveAppSettingsAsync) => CountLegacySave(),
                "get_HasProjectRoot" => true,
                _ => UnsupportedTask(targetMethod),
            };
        }

        private Task<GeneralSectionSettings> SaveGeneral(GeneralSectionSettings settings)
        {
            SavedGeneral = settings;
            SaveStarted.TrySetResult();
            if (SaveFailure is not null)
            {
                return Task.FromException<GeneralSectionSettings>(SaveFailure);
            }
            return _heldSave?.Task ?? Task.FromResult(settings);
        }

        private Task<AppSettings> CountLegacySave()
        {
            SaveAppSettingsCalls++;
            return Task.FromResult(InitialAppSettings());
        }

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
