using Avalonia;
using Avalonia.Controls;
using Avalonia.Input;
using Avalonia.Input.Platform;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class RunLogPageView : UserControl
{
    public RunLogPageView()
    {
        InitializeComponent();
        DataContextChanged += (_, _) => AttachClipboardActions();
        AttachClipboardActions();
    }

    private RunLogPageViewModel? _attachedViewModel;

    protected override void OnAttachedToVisualTree(VisualTreeAttachmentEventArgs e)
    {
        base.OnAttachedToVisualTree(e);
        AttachClipboardActions();
    }

    protected override void OnDetachedFromVisualTree(VisualTreeAttachmentEventArgs e)
    {
        if (_attachedViewModel is not null)
        {
            _attachedViewModel.RequestCopyText = null;
            _attachedViewModel = null;
        }
        base.OnDetachedFromVisualTree(e);
    }

    private void AttachClipboardActions()
    {
        if (_attachedViewModel is not null)
        {
            _attachedViewModel.RequestCopyText = null;
        }
        _attachedViewModel = DataContext as RunLogPageViewModel;
        if (_attachedViewModel is not null)
        {
            _attachedViewModel.RequestCopyText = CopyTextAsync;
        }
    }

    private async Task CopyTextAsync(string text)
    {
        var clipboard = TopLevel.GetTopLevel(this)?.Clipboard;
        if (clipboard is not null)
        {
            await clipboard.SetTextAsync(text);
        }
    }

    private void OnSearchKeyDown(object? sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter || DataContext is not RunLogPageViewModel viewModel)
        {
            return;
        }

        viewModel.SearchCommand.Execute(null);
        e.Handled = true;
    }
}
