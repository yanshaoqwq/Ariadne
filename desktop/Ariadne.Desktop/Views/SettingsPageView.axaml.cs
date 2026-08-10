using Avalonia;
using Avalonia.Automation;
using Avalonia.Controls;
using Avalonia.Input.Platform;
using Avalonia.Interactivity;
using Avalonia.Platform.Storage;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class SettingsPageView : UserControl
{
    private SettingsPageViewModel? _attachedViewModel;
    private readonly Func<string?, Task<string?>> _folderPicker;
    private readonly Func<string?, Task<string?>> _filePicker;
    private bool _isAttachedToVisualTree;
    private int _sectionOffsetCommitCount;

    public SettingsPageView()
    {
        InitializeComponent();
        _folderPicker = PickFolderAsync;
        _filePicker = PickFileAsync;
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
            vm.FocusValidationFieldRequested += OnFocusValidationFieldRequested;
            vm.SetFolderPicker(_folderPicker);
            vm.SetFilePicker(_filePicker);
            _attachedViewModel = vm;
        }
    }

    private void DetachBehaviors()
    {
        if (_attachedViewModel is not null)
        {
            _attachedViewModel.ScrollToSectionRequested -= OnScrollToSectionRequested;
            _attachedViewModel.FocusValidationFieldRequested -= OnFocusValidationFieldRequested;
            _attachedViewModel.ClearFolderPicker(_folderPicker);
            _attachedViewModel.ClearFilePicker(_filePicker);
        }
        _attachedViewModel = null;
    }

    private void OnFocusValidationFieldRequested(
        object? sender,
        SettingsFieldFocusRequest request)
    {
        if (sender is not SettingsPageViewModel source)
        {
            return;
        }
        Dispatcher.UIThread.Post(
            () => FocusValidationField(source, request),
            DispatcherPriority.Loaded);
    }

    private void FocusValidationField(
        SettingsPageViewModel source,
        SettingsFieldFocusRequest request)
    {
        if (!ReferenceEquals(_attachedViewModel, source))
        {
            return;
        }
        var candidates = this.GetVisualDescendants()
            .OfType<Control>()
            .Where(control => string.Equals(
                AutomationProperties.GetName(control),
                request.AccessibleName,
                StringComparison.Ordinal));
        var target = request.Item is null
            ? candidates.FirstOrDefault()
            : candidates.FirstOrDefault(control => ReferenceEquals(control.DataContext, request.Item));
        if (target is null)
        {
            return;
        }
        target.BringIntoView();
        target.Focus();
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

    private async Task<string?> PickFileAsync(string? title)
    {
        var topLevel = TopLevel.GetTopLevel(this);
        if (topLevel is null)
        {
            return null;
        }

        var files = await topLevel.StorageProvider.OpenFilePickerAsync(new FilePickerOpenOptions
        {
            Title = string.IsNullOrWhiteSpace(title) ? null : title,
            AllowMultiple = false,
        });
        return files.FirstOrDefault()?.Path.LocalPath;
    }

    private async void OnCopyDiagnostics(object? sender, RoutedEventArgs e)
    {
        if (DataContext is not SettingsPageViewModel vm || string.IsNullOrWhiteSpace(vm.DiagnosticsCopyText))
        {
            return;
        }

        var clipboard = TopLevel.GetTopLevel(this)?.Clipboard;
        if (clipboard is not null)
        {
            try
            {
                await clipboard.SetTextAsync(vm.DiagnosticsCopyText);
            }
            catch (Exception)
            {
                // async void 处理器的异常无人可捕，剪贴板不可用时只能就地吞掉。
            }
        }
    }
}
