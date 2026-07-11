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

    private void ScrollToSection(string sectionId)
    {
        if (string.IsNullOrWhiteSpace(sectionId))
        {
            return;
        }

        // 用 Offset 显式滚到目标顶边，避免短页/Panel 叠层时 BringIntoView 贴底无效。
        Dispatcher.UIThread.Post(() =>
        {
            var scroll = this.FindControl<ScrollViewer>("SettingsContentScroll")
                         ?? this.GetVisualDescendants().OfType<ScrollViewer>()
                             .FirstOrDefault(c => c.Name == "SettingsContentScroll");
            var target = FindSectionControl(sectionId);
            if (target is null)
            {
                return;
            }

            if (scroll is null)
            {
                target.BringIntoView();
                return;
            }

            // 先确保目标参与布局
            target.UpdateLayout();
            scroll.UpdateLayout();

            var transform = target.TransformToVisual(scroll);
            if (transform is null)
            {
                target.BringIntoView();
                return;
            }

            var topLeft = transform.Value.Transform(new Point(0, 0));
            var nextY = Math.Max(0, scroll.Offset.Y + topLeft.Y - 12);
            var maxY = Math.Max(0, scroll.Extent.Height - scroll.Viewport.Height);
            scroll.Offset = new Vector(scroll.Offset.X, Math.Min(nextY, maxY));
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
