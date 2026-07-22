using Avalonia;
using Avalonia.Controls;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class SettingsPageView : UserControl
{
    private SettingsPageViewModel? _attachedViewModel;
    private readonly Func<string?, Task<string?>> _folderPicker;
    private bool _isAttachedToVisualTree;
    private int _sectionOffsetCommitCount;

    public SettingsPageView()
    {
        InitializeComponent();
        _folderPicker = PickFolderAsync;
        DataContextChanged += OnDataContextChanged;
    }

    protected override void OnAttachedToVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnAttachedToVisualTree(e);
        _isAttachedToVisualTree = true;
        AttachBehaviors();
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        _isAttachedToVisualTree = false;
        DetachBehaviors();
        base.OnDetachedFromVisualTree(e);
    }

    private void OnDataContextChanged(object? sender, EventArgs e)
    {
        if (_isAttachedToVisualTree)
        {
            AttachBehaviors();
        }
    }

    private void AttachBehaviors()
    {
        DetachBehaviors();
        if (DataContext is SettingsPageViewModel vm)
        {
            vm.ScrollToSectionRequested += OnScrollToSectionRequested;
            vm.SetFolderPicker(_folderPicker);
            _attachedViewModel = vm;
        }
    }

    private void DetachBehaviors()
    {
        if (_attachedViewModel is not null)
        {
            _attachedViewModel.ScrollToSectionRequested -= OnScrollToSectionRequested;
            _attachedViewModel.ClearFolderPicker(_folderPicker);
        }
        _attachedViewModel = null;
    }

    private void OnScrollToSectionRequested(
        object? sender,
        SettingsSectionNavigationRequest request)
    {
        if (sender is not SettingsPageViewModel source)
        {
            return;
        }

        Dispatcher.UIThread.Post(
            () => ScrollToSection(source, request),
            DispatcherPriority.Loaded);
    }

    private void ScrollToSection(
        SettingsPageViewModel source,
        SettingsSectionNavigationRequest request)
    {
        if (!ReferenceEquals(_attachedViewModel, source))
        {
            return;
        }

        var anchor = this.FindControl<Control>(request.AnchorName);
        if (anchor is null
            || anchor.TranslatePoint(new Point(0, 0), SettingsContentHost) is not Point position)
        {
            source.ReportSectionNavigationFailure(request.SectionTitle);
            return;
        }

        var maxOffset = Math.Max(
            0,
            SettingsContentScroll.Extent.Height - SettingsContentScroll.Viewport.Height);
        SettingsContentScroll.Offset = new Vector(
            SettingsContentScroll.Offset.X,
            Math.Clamp(position.Y, 0, maxOffset));
        _sectionOffsetCommitCount++;
    }

    internal int SectionOffsetCommitCountForTests => _sectionOffsetCommitCount;

    internal double SectionOffsetForTests => SettingsContentScroll.Offset.Y;

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
