using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class ProjectAutomationStateTests
{
    [Fact]
    public async Task Toggle_CommitsBackendReadbackAndIsSharedAcrossPages()
    {
        var backend = AutomationBackend.Create();
        var state = new ProjectAutomationState(DisplayNameService.LoadDefault(), backend.Client);
        var workspace = new WorkspacePageViewModel(
            DisplayNameService.LoadDefault(), backend.Client, projectAutomation: state);
        var settings = new SettingsPageViewModel(
            DisplayNameService.LoadDefault(), backend.Client, projectAutomation: state);

        state.ApplyBackendValue(false);
        await state.SetEnabledAsync(true);

        Assert.True(state.IsEnabled);
        Assert.Same(state, workspace.ProjectAutomation);
        Assert.Equal(1, backend.SetCalls);
        Assert.Equal(1, backend.GetCalls);
        Assert.DoesNotContain(
            settings.GetType().GetProperties(),
            property => property.Name == "AutoModeEnabled");
    }

    [Fact]
    public async Task BeginProjectSession_InvalidatesLoadedValueAndReloadsNewProjectState()
    {
        var backend = AutomationBackend.Create();
        var state = new ProjectAutomationState(DisplayNameService.LoadDefault(), backend.Client);
        state.ApplyBackendValue(true);

        await state.EnsureLoadedAsync();
        Assert.Equal(0, backend.GetCalls);

        backend.Enabled = false;
        state.BeginProjectSession();
        await state.EnsureLoadedAsync();

        Assert.False(state.IsEnabled);
        Assert.Equal(1, backend.GetCalls);
    }

    [Fact]
    public void AutoModeToggle_LivesInSharedProjectAiComposer_NotSettingsDraft()
    {
        var composer = File.ReadAllText(ResolveDesktopSource("Controls", "ProjectAiComposer.axaml"));
        var settings = File.ReadAllText(ResolveDesktopSource("Views", "SettingsPageView.axaml"));

        Assert.Contains("ProjectAutomation.ToggleCommand", composer, StringComparison.Ordinal);
        Assert.Contains("ProjectAutomation.IsEnabled", composer, StringComparison.Ordinal);
        Assert.DoesNotContain("AutoModeEnabled", settings, StringComparison.Ordinal);
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        while (directory is not null)
        {
            var candidate = Path.Combine(
                new[] { directory.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            directory = directory.Parent;
        }
        throw new FileNotFoundException(string.Join('/', parts));
    }

    private class AutomationBackend : DispatchProxy
    {
        public IAriadneBackendClient Client { get; private set; } = null!;
        public bool Enabled { get; set; }
        public int GetCalls { get; private set; }
        public int SetCalls { get; private set; }

        public static AutomationBackend Create()
        {
            var client = Create<IAriadneBackendClient, AutomationBackend>();
            var backend = (AutomationBackend)(object)client;
            backend.Client = client;
            return backend;
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }
            if (targetMethod.Name == "get_HasProjectRoot")
            {
                return true;
            }

            object? value = targetMethod.Name switch
            {
                nameof(IAriadneBackendClient.GetAutomationSettingsAsync) => ReadSettings(),
                nameof(IAriadneBackendClient.SetAutoModeAsync) => SetEnabled((bool)args![0]!),
                _ => targetMethod.ReturnType.IsValueType
                    ? Activator.CreateInstance(targetMethod.ReturnType)
                    : null,
            };
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
                    .Invoke(null, new[] { value });
            }
            return value;
        }

        private AutomationSettings ReadSettings()
        {
            GetCalls++;
            return new AutomationSettings(
                new BudgetStatus(0, 0, 0, Enabled),
                Array.Empty<ConfirmationPolicySetting>());
        }

        private object? SetEnabled(bool enabled)
        {
            SetCalls++;
            Enabled = enabled;
            return null;
        }
    }
}
