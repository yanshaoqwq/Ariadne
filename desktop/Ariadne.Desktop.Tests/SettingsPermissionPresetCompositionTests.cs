using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class SettingsPermissionPresetCompositionTests
{
    [Fact]
    public async Task InheritedPreset_UsesWorkflowScopeAndDoesNotBecomeDirtyWhenParentChanges()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        proxy.Enqueue(
            Task.FromResult(Presets("project-a", permissionPolicy: null)),
            Task.FromResult(Permissions(
                globalNetwork: false,
                workflowNetwork: true)));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);

        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        var preset = Assert.Single(vm.NodePresets);
        Assert.True(preset.Permissions.InheritGlobal);
        Assert.True(preset.Permissions.AllowNetwork);
        Assert.False(vm.HasUnsavedChanges);

        var workflow = Assert.Single(vm.ScopedPermissionProfiles, item => item.Scope == "workflow_nodes");
        workflow.AllowNetwork = false;

        Assert.False(preset.Permissions.AllowNetwork);
        Assert.True(vm.HasUnsavedChanges);
        Assert.True(await vm.SaveUnsavedChangesAsync());
        Assert.Equal(1, proxy.SavePermissionsCalls);
        Assert.Equal(0, proxy.SavePresetsCalls);
    }

    [Fact]
    public async Task ExplicitPresetOverride_IsNotReplacedByLaterWorkflowPermissionChanges()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        proxy.Enqueue(
            Task.FromResult(Presets("project-a", permissionPolicy: null)),
            Task.FromResult(Permissions(
                globalNetwork: false,
                workflowNetwork: true)));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);
        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        var preset = Assert.Single(vm.NodePresets);

        preset.Permissions.InheritGlobal = false;
        Assert.True(preset.Permissions.AllowNetwork);
        var workflow = Assert.Single(vm.ScopedPermissionProfiles, item => item.Scope == "workflow_nodes");
        workflow.AllowNetwork = false;

        Assert.True(preset.Permissions.AllowNetwork);
        Assert.True(await vm.SaveUnsavedChangesAsync());
        Assert.NotNull(proxy.SavedPresets);
        Assert.True(Assert.Single(proxy.SavedPresets!.Presets).PermissionPolicy!.AllowNetwork);
    }

    [Fact]
    public async Task OlderPermissionPresetGeneration_CannotOverwriteNewProjectProjection()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        var oldPresets = NewSource<NodePresetSettings>();
        var oldPermissions = NewSource<PermissionsSettings>();
        var newPresets = NewSource<NodePresetSettings>();
        var newPermissions = NewSource<PermissionsSettings>();
        proxy.Enqueue(oldPresets.Task, oldPermissions.Task);
        proxy.Enqueue(newPresets.Task, newPermissions.Task);
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);

        var oldLoad = vm.ReloadPermissionPresetProjectionForTestsAsync();
        var newLoad = vm.ReloadPermissionPresetProjectionForTestsAsync();
        newPermissions.SetResult(Permissions(globalNetwork: true, workflowNetwork: false));
        newPresets.SetResult(Presets("project-b", permissionPolicy: null));

        Assert.True(await newLoad);
        var current = Assert.Single(vm.NodePresets);
        Assert.Equal("project-b", current.ModelId);
        Assert.False(current.Permissions.AllowNetwork);

        oldPresets.SetResult(Presets("project-a", permissionPolicy: null));
        oldPermissions.SetResult(Permissions(globalNetwork: false, workflowNetwork: true));

        Assert.False(await oldLoad);
        current = Assert.Single(vm.NodePresets);
        Assert.Equal("project-b", current.ModelId);
        Assert.False(current.Permissions.AllowNetwork);
    }

    [Fact]
    public async Task PartialPermissionPresetLoad_DoesNotExposeMixedSavableProjection()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        proxy.Enqueue(
            Task.FromResult(Presets("partial", permissionPolicy: null)),
            Task.FromException<PermissionsSettings>(new InvalidOperationException("permissions failed")));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);

        Assert.False(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        Assert.Empty(vm.NodePresets);
        Assert.Empty(vm.ScopedPermissionProfiles);
        Assert.False(vm.IsPresetsEditable);
        Assert.False(vm.IsPermissionsEditable);
    }

    [Fact]
    public async Task GlobalPermissions_RemainEditableWhenProjectPresetsAreUnavailable()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        proxy.Enqueue(
            Task.FromException<NodePresetSettings>(new InvalidOperationException("no project presets")),
            Task.FromResult(Permissions(globalNetwork: true, workflowNetwork: false)));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);

        Assert.False(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        Assert.Empty(vm.NodePresets);
        Assert.NotEmpty(vm.ScopedPermissionProfiles);
        Assert.False(vm.IsPresetsEditable);
        Assert.True(vm.IsPermissionsEditable);

        vm.AllowNetwork = false;
        Assert.True(await vm.SaveUnsavedChangesAsync());
        Assert.Equal(1, proxy.SavePermissionsCalls);
        Assert.Equal(0, proxy.SavePresetsCalls);
    }

    [Fact]
    public async Task UnknownScopedPolicies_ArePreservedWhenKnownPermissionsAreSaved()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        var global = Policy(network: true, root: "/global");
        proxy.Enqueue(
            Task.FromResult(Presets("project-a", permissionPolicy: null)),
            Task.FromResult(new PermissionsSettings(
                global,
                new Dictionary<string, PermissionPolicy?>
                {
                    ["workflow_nodes"] = null,
                    ["project_ai"] = null,
                    ["future_scope"] = Policy(network: false, root: "/future"),
                },
                new Dictionary<string, IReadOnlyDictionary<string, bool?>>())));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);

        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        Assert.True(vm.HasCompatibilityPermissionScopes);

        vm.AllowNetwork = false;
        Assert.True(await vm.SaveUnsavedChangesAsync());
        Assert.NotNull(proxy.SavedPermissions);
        Assert.False(proxy.SavedPermissions!.ScopedPolicies["future_scope"]!.AllowNetwork);
        Assert.Equal(new[] { "/future" }, proxy.SavedPermissions.ScopedPolicies["future_scope"]!.ReadableFileRoots);
    }

    [Fact]
    public async Task PermissionProfiles_ApplyRealPoliciesAndManualChangesBecomeCustom()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        proxy.Enqueue(
            Task.FromResult(Presets("project-a", permissionPolicy: null)),
            Task.FromResult(Permissions(globalNetwork: false, workflowNetwork: true)));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);

        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        typeof(SettingsPageViewModel)
            .GetField("_projectRoot", BindingFlags.Instance | BindingFlags.NonPublic)!
            .SetValue(vm, "/project");
        vm.SelectedPermissionProfile = Assert.Single(
            vm.PermissionProfileOptions,
            option => option.Value == "recommended");

        Assert.True(vm.AllowNetwork);
        Assert.True(vm.AllowWebSearch);
        Assert.False(vm.AllowHttpSkill);
        Assert.False(vm.AllowWasmNetwork);
        Assert.False(vm.AllowSecretRead);
        Assert.Equal("/project", vm.ReadableRootsText);
        Assert.Equal("/project", vm.WritableRootsText);
        Assert.All(vm.ScopedPermissionProfiles, profile => Assert.True(profile.InheritGlobal));
        Assert.Equal("recommended", vm.SelectedPermissionProfile?.Value);

        vm.AllowSecretRead = true;

        Assert.Equal("custom", vm.SelectedPermissionProfile?.Value);
        Assert.True(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task ScopedPermissionRoots_CanBeAddedWithFolderPicker()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        proxy.Enqueue(
            Task.FromResult(Presets("project-a", permissionPolicy: null)),
            Task.FromResult(Permissions(globalNetwork: false, workflowNetwork: true)));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);
        vm.SetFolderPicker(_ => Task.FromResult<string?>("/selected"));

        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        var workflow = Assert.Single(vm.ScopedPermissionProfiles, item => item.Scope == "workflow_nodes");
        workflow.InheritGlobal = false;
        workflow.BrowseReadableRootsCommand.Execute(null);
        for (var attempt = 0; attempt < 50 && !workflow.ReadableRootsText.Contains("/selected", StringComparison.Ordinal); attempt++)
        {
            await Task.Delay(1);
        }

        Assert.Contains("/selected", workflow.ReadableRootsText, StringComparison.Ordinal);
        Assert.True(vm.HasUnsavedChanges);
    }

    [Fact]
    public async Task FailedPermissionProjection_OffersLocalRetryWithoutStartingANewLoad()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        proxy.Enqueue(
            Task.FromResult(Presets("project-a", permissionPolicy: null)),
            Task.FromException<PermissionsSettings>(new InvalidOperationException("temporary failure")));
        proxy.Enqueue(
            Task.FromResult(Presets("project-b", permissionPolicy: null)),
            Task.FromResult(Permissions(globalNetwork: true, workflowNetwork: true)));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);

        Assert.False(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        Assert.Contains(vm.SectionLoadFailures, item => item.Section == "permissions");

        var retry = Assert.Single(vm.SectionLoadFailures, item => item.Section == "permissions");
        retry.RetryCommand.Execute(null);
        for (var attempt = 0; attempt < 100 && vm.HasSectionLoadFailures; attempt++)
        {
            await Task.Delay(1);
        }

        Assert.False(vm.HasSectionLoadFailures);
        Assert.True(vm.IsPermissionsEditable);
        Assert.True(vm.IsPresetsEditable);
        Assert.Equal("project-b", Assert.Single(vm.NodePresets).ModelId);
    }

    [Fact]
    public async Task ModelAliases_AppearWithConcreteModelsAndRemapWithoutRewritingReferences()
    {
        var backend = PermissionPresetBackend.Create(out var proxy);
        var presets = new NodePresetSettings(
            new[]
            {
                new NodeTypePreset(
                    "llm",
                    "node.type.llm",
                    string.Empty,
                    30_000,
                    1,
                    null,
                    new Dictionary<string, bool?>(),
                    string.Empty,
                    "planning"),
            },
            string.Empty,
            30_000,
            1,
            string.Empty,
            new Dictionary<string, ModelAliasTarget>
            {
                ["planning"] = new("provider-a", "model-a"),
            },
            "planning");
        proxy.Enqueue(
            Task.FromResult(presets),
            Task.FromResult(Permissions(globalNetwork: false, workflowNetwork: false)));
        var vm = new SettingsPageViewModel(DisplayNameService.LoadDefault(), backend);
        vm.ApplyProviderConfigForTests(new ProviderConfigStatus(
            false,
            false,
            false,
            "provider-a",
            null,
            null,
            null,
            new[]
            {
                Provider("provider-a", "model-a"),
                Provider("provider-b", "model-b"),
            }));

        Assert.True(await vm.ReloadPermissionPresetProjectionForTestsAsync());
        Assert.Equal(3, vm.ModelAliases.Count);
        var planning = Assert.Single(vm.ModelAliases, alias => alias.AliasId == "planning");
        Assert.Equal("provider-a", planning.SelectedTargetOption?.ProviderId);
        var aliasOptions = vm.AvailableLlmModelOptions.Where(option => option.IsAlias).ToArray();
        Assert.Equal(new[] { "planning", "writing", "review" }, aliasOptions.Select(option => option.AliasId));
        Assert.All(aliasOptions, option =>
            Assert.True(vm.AvailableLlmModelOptions.IndexOf(option) < vm.AvailableLlmModelOptions.Count - 2));
        Assert.Contains(aliasOptions, option =>
            option.AliasId == "writing" && option.DisplayName.Contains("未配置", StringComparison.Ordinal));
        Assert.Contains(aliasOptions, option =>
            option.AliasId == "review" && option.DisplayName.Contains("未配置", StringComparison.Ordinal));
        Assert.Contains(vm.AvailableLlmModelOptions, option =>
            option.ProviderId == "provider-a" && option.ModelId == "model-a");
        Assert.Equal("planning", vm.SelectedDefaultModelOption?.AliasId);
        Assert.Equal("planning", Assert.Single(vm.NodePresets).SelectedModelOption?.AliasId);

        planning.SelectedTargetOption = Assert.Single(
            vm.AvailableLlmModelTargetOptions,
            option => option.ProviderId == "provider-b" && option.ModelId == "model-b");

        Assert.True(vm.HasUnsavedChanges);
        Assert.True(await vm.SaveUnsavedChangesAsync());
        Assert.NotNull(proxy.SavedPresets);
        Assert.Equal("provider-b", proxy.SavedPresets.ModelAliases!["planning"].ProviderId);
        Assert.Equal("planning", proxy.SavedPresets.DefaultModelAlias);
        Assert.Equal("planning", Assert.Single(proxy.SavedPresets.Presets).ModelAlias);
    }

    private static TaskCompletionSource<T> NewSource<T>() =>
        new(TaskCreationOptions.RunContinuationsAsynchronously);

    private static NodePresetSettings Presets(string modelId, PermissionPolicy? permissionPolicy) => new(
        new[]
        {
            new NodeTypePreset(
                "llm",
                "node.type.llm",
                modelId,
                30_000,
                1,
                permissionPolicy,
                new Dictionary<string, bool?>()),
        },
        modelId,
        30_000,
        1);

    private static PermissionsSettings Permissions(bool globalNetwork, bool workflowNetwork)
    {
        var global = Policy(globalNetwork, "/global");
        return new PermissionsSettings(
            global,
            new Dictionary<string, PermissionPolicy?>
            {
                ["workflow_nodes"] = Policy(workflowNetwork, "/workflow"),
                ["project_ai"] = null,
            },
            new Dictionary<string, IReadOnlyDictionary<string, bool?>>());
    }

    private static PermissionPolicy Policy(bool network, string root) => new(
        network,
        network,
        network,
        network,
        false,
        new[] { root },
        new[] { root });

    private static ProviderKeyStatus Provider(string id, string model) => new(
        id,
        id,
        "open_ai_compatible",
        true,
        true,
        "https://example.invalid",
        new[] { new ModelConfig(model, "llm", null, null, null) },
        false);

    internal class PermissionPresetBackend : DispatchProxy
    {
        private readonly Queue<Task<NodePresetSettings>> _presets = new();
        private readonly Queue<Task<PermissionsSettings>> _permissions = new();

        public int SavePermissionsCalls { get; private set; }
        public int SavePresetsCalls { get; private set; }
        public NodePresetSettings? SavedPresets { get; private set; }
        public PermissionsSettings? SavedPermissions { get; private set; }

        public static IAriadneBackendClient Create(out PermissionPresetBackend proxy)
        {
            var client = Create<IAriadneBackendClient, PermissionPresetBackend>();
            proxy = (PermissionPresetBackend)(object)client;
            return client;
        }

        public void Enqueue(
            Task<NodePresetSettings> presets,
            Task<PermissionsSettings> permissions)
        {
            _presets.Enqueue(presets);
            _permissions.Enqueue(permissions);
        }

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            return targetMethod?.Name switch
            {
                nameof(IAriadneBackendClient.GetNodePresetSettingsAsync) => _presets.Dequeue(),
                nameof(IAriadneBackendClient.GetPermissionsSettingsAsync) => _permissions.Dequeue(),
                nameof(IAriadneBackendClient.SavePermissionsSettingsAsync) => SavePermissions(
                    (PermissionsSettings)args![0]!),
                nameof(IAriadneBackendClient.SaveNodePresetSettingsAsync) => SavePresets(
                    (NodePresetSettings)args![0]!),
                "get_HasProjectRoot" => true,
                _ => UnsupportedTask(targetMethod),
            };
        }

        private Task<PermissionsSettings> SavePermissions(PermissionsSettings settings)
        {
            SavePermissionsCalls++;
            SavedPermissions = settings;
            return Task.FromResult(settings);
        }

        private Task<NodePresetSettings> SavePresets(NodePresetSettings settings)
        {
            SavePresetsCalls++;
            SavedPresets = settings;
            return Task.FromResult(settings);
        }

        private static object? UnsupportedTask(MethodInfo? method)
        {
            if (method is null || method.ReturnType == typeof(void))
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
