using Avalonia;
using Avalonia.Controls;
using Avalonia.Layout;
using Avalonia.Media;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class SettingsPageView : UserControl
{
    private SettingsPageViewModel? _attached;
    private Border? _scrollPad;

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

        // 短页也要把目标区块顶到视口顶部：底部垫高 + Offset 对齐顶边。
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

            EnsureScrollPad(scroll);
            target.UpdateLayout();
            scroll.UpdateLayout();

            // 相对滚动内容根：目标在内容坐标系中的 Y
            var contentRoot = scroll.Content as Visual ?? scroll;
            var transform = target.TransformToVisual(contentRoot);
            if (transform is null)
            {
                target.BringIntoView();
                return;
            }

            var topInContent = transform.Value.Transform(new Point(0, 0)).Y;
            var nextY = Math.Max(0, topInContent - 8);
            var maxY = Math.Max(0, scroll.Extent.Height - scroll.Viewport.Height);
            if (nextY > maxY + 0.5)
            {
                EnsureScrollPad(scroll, extra: nextY - maxY + Math.Max(120, scroll.Viewport.Height * 0.35));
                scroll.UpdateLayout();
                maxY = Math.Max(0, scroll.Extent.Height - scroll.Viewport.Height);
            }

            scroll.Offset = new Vector(scroll.Offset.X, Math.Min(nextY, maxY));
        }, DispatcherPriority.Loaded);
    }

    /// <summary>
    /// 在当前可见 Tab 内容底部加透明垫高，使任意区块都能滚到视口顶。
    /// </summary>
    private void EnsureScrollPad(ScrollViewer scroll, double extra = 0)
    {
        var host = this.FindControl<Control>("SettingsContentHost")
                   ?? this.GetVisualDescendants().OfType<Control>()
                       .FirstOrDefault(c => c.Name == "SettingsContentHost");
        if (host is not Panel panel)
        {
            return;
        }

        var visibleStack = panel.Children.OfType<StackPanel>().FirstOrDefault(s => s.IsVisible);
        if (visibleStack is null)
        {
            return;
        }

        var padH = Math.Max(scroll.Viewport.Height, 280) + Math.Max(0, extra);
        if (_scrollPad is null)
        {
            _scrollPad = new Border
            {
                Name = "SettingsScrollPad",
                Background = Brushes.Transparent,
                IsHitTestVisible = false,
                Height = padH,
                HorizontalAlignment = HorizontalAlignment.Stretch,
                MinHeight = 1,
            };
        }
        else
        {
            _scrollPad.Height = padH;
        }

        if (_scrollPad.Parent is Panel old && !ReferenceEquals(old, visibleStack))
        {
            old.Children.Remove(_scrollPad);
        }

        if (_scrollPad.Parent is null)
        {
            visibleStack.Children.Add(_scrollPad);
        }
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
