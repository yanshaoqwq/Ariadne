using Avalonia.Controls;
using Avalonia.Platform.Storage;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class SettingsPageView : UserControl
{
    public SettingsPageView()
    {
        InitializeComponent();
        AttachBehaviors();
        DataContextChanged += (_, _) => AttachBehaviors();
    }

    private void AttachBehaviors()
    {
        if (DataContext is SettingsPageViewModel vm)
        {
            vm.SetFolderPicker(PickFolderAsync);
        }
    }

    private async Task<string?> PickFolderAsync(string? title)
    {
        var topLevel = TopLevel.GetTopLevel(this);
        if (topLevel is null)
        {
            return null;
        }

        var folders = await topLevel.StorageProvider.OpenFolderPickerAsync(new FolderPickerOpenOptions
        {
            Title = string.IsNullOrWhiteSpace(title) ? null : title,
            AllowMultiple = false,
        });
        return folders.FirstOrDefault()?.Path.LocalPath;
    }
}
