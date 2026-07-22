using System.Reflection;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class SettingsProjectDirectoryTests
{
    [Fact]
    public async Task PickerStoresCanonicalProjectRelativeDirectory()
    {
        var project = Directory.CreateTempSubdirectory("ariadne-settings-project-");
        try
        {
            var selected = Directory.CreateDirectory(Path.Combine(project.FullName, "content", "docs"));
            var vm = NewViewModel();
            vm.ConfigureProjectDirectoryPickerForTests(
                project.FullName,
                _ => Task.FromResult<string?>(selected.FullName));

            await vm.BrowseDocumentsDirectoryForTestsAsync();

            Assert.Equal("content/docs", vm.DocumentsDir.Replace('\\', '/'));
        }
        finally
        {
            project.Delete(recursive: true);
        }
    }

    [Fact]
    public async Task PickerRejectsDirectoryOutsideProjectWithoutChangingValue()
    {
        var project = Directory.CreateTempSubdirectory("ariadne-settings-project-");
        var outside = Directory.CreateTempSubdirectory("ariadne-settings-outside-");
        try
        {
            var vm = NewViewModel();
            vm.DocumentsDir = "documents";
            vm.ConfigureProjectDirectoryPickerForTests(
                project.FullName,
                _ => Task.FromResult<string?>(outside.FullName));

            await vm.BrowseDocumentsDirectoryForTestsAsync();

            Assert.Equal("documents", vm.DocumentsDir);
            Assert.NotEqual(vm.Title, vm.StatusText);
        }
        finally
        {
            project.Delete(recursive: true);
            outside.Delete(recursive: true);
        }
    }

    private static SettingsPageViewModel NewViewModel() =>
        new(DisplayNameService.LoadDefault(), NoopBackend.Create());

    private class NoopBackend : DispatchProxy
    {
        public static IAriadneBackendClient Create() =>
            Create<IAriadneBackendClient, NoopBackend>();

        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args) =>
            throw new NotSupportedException(targetMethod?.Name);
    }
}
