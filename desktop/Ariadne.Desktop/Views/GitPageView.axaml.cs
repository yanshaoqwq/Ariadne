using Avalonia;
using Avalonia.Controls;
using Avalonia.Input.Platform;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Views;

public partial class GitPageView : UserControl
{
    private GitPageViewModel? _attachedViewModel;

    public GitPageView()
    {
        InitializeComponent();
        DataContextChanged += (_, _) => AttachClipboardActions();
        AttachClipboardActions();
    }

    private void AttachClipboardActions()
    {
        if (_attachedViewModel is not null && !ReferenceEquals(_attachedViewModel, DataContext))
        {
            _attachedViewModel.RequestCopyText = null;
            _attachedViewModel = null;
        }

        if (DataContext is GitPageViewModel viewModel)
        {
            viewModel.RequestCopyText = CopyTextAsync;
            _attachedViewModel = viewModel;
        }
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

    private async Task CopyTextAsync(string text)
    {
        var clipboard = TopLevel.GetTopLevel(this)?.Clipboard;
        if (clipboard is not null)
        {
            await clipboard.SetTextAsync(text);
        }
    }
}
