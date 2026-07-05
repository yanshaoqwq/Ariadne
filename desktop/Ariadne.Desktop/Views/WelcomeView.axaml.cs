using Avalonia.Controls;
using Avalonia.Platform.Storage;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class WelcomeView : UserControl
{
    public WelcomeView()
    {
        InitializeComponent();
        AttachProjectFolderPicker();
        DataContextChanged += (_, _) =>
        {
            AttachProjectFolderPicker();
        };
    }

    private void AttachProjectFolderPicker()
    {
        if (DataContext is WelcomeViewModel vm)
        {
            vm.SetProjectFolderPicker(PickProjectFolderAsync);
        }
    }

    private async Task<string?> PickProjectFolderAsync()
    {
        var topLevel = TopLevel.GetTopLevel(this);
        if (topLevel is null)
        {
            return null;
        }
        var folders = await topLevel.StorageProvider.OpenFolderPickerAsync(new FolderPickerOpenOptions
        {
            AllowMultiple = false,
        });
        return folders.FirstOrDefault()?.Path.LocalPath;
    }
}
