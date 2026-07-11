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
            // 新建：选父目录；打开：选已有项目根
            vm.SetProjectFolderPicker(title => PickFolderAsync(title));
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
