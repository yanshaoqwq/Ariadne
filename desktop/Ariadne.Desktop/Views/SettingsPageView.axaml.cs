using Avalonia;
using Avalonia.Controls;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class SettingsPageView : UserControl
{
    private SettingsPageViewModel? _attached;

    public SettingsPageView()
    {
        InitializeComponent();
        AttachBehaviors();
        DataContextChanged += (_, _) => AttachBehaviors();
    }

    private void AttachBehaviors()
    {
        if (_attached is not null)
        {
            _attached.RequestScrollToSection = null;
            _attached = null;
        }

        if (DataContext is SettingsPageViewModel vm)
        {
            vm.SetFolderPicker(PickFolderAsync);
            vm.RequestScrollToSection = ScrollToSection;
            _attached = vm;
        }
    }

    // PickFolderAsync(string? title) matches SetFolderPicker(Func<string?, Task<string?>>)

    private void ScrollToSection(string sectionId)
    {
        if (string.IsNullOrWhiteSpace(sectionId))
        {
            return;
        }

        // 等布局稳定后再 BringIntoView，避免切换 Tab 后目标尚未挂到树上。
        Dispatcher.UIThread.Post(() =>
        {
            var target = FindSectionControl(sectionId);
            if (target is null)
            {
                return;
            }

            target.BringIntoView();
            // 再补一帧，ScrollViewer 对深层目标有时需二次确认。
            Dispatcher.UIThread.Post(() => target.BringIntoView(), DispatcherPriority.Background);
        }, DispatcherPriority.Loaded);
    }

    private Control? FindSectionControl(string sectionId)
    {
        var host = this.FindControl<Control>("SettingsContentHost")
                   ?? this.GetVisualDescendants().OfType<Control>().FirstOrDefault(c => c.Name == "SettingsContentHost");
        var root = host ?? (Control)this;
        return FindByTag(root, sectionId);
    }

    private static Control? FindByTag(Control root, string sectionId)
    {
        if (root.Tag is string tag && string.Equals(tag, sectionId, StringComparison.Ordinal))
        {
            return root;
        }

        foreach (var child in root.GetVisualChildren().OfType<Control>())
        {
            var match = FindByTag(child, sectionId);
            if (match is not null)
            {
                return match;
            }
        }

        return null;
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
