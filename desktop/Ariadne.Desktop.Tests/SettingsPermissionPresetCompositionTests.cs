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

    private class PermissionPresetBackend : DispatchProxy
    {
        private readonly Queue<Task<NodePresetSettings>> _presets = new();
        private readonly Queue<Task<PermissionsSettings>> _permissions = new();

        public int SavePermissionsCalls { get; private set; }
        public int SavePresetsCalls { get; private set; }
        public NodePresetSettings? SavedPresets { get; private set; }

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
