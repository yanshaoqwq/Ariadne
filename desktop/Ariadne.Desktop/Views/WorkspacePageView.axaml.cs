using Avalonia;
using Avalonia.Controls;

namespace Ariadne.Desktop.Views;

public partial class WorkspacePageView : UserControl
{
    // 记住展开时的下栏高度，收起→展开时恢复。
    private GridLength _savedLibraryHeight = new(220);

    public WorkspacePageView()
    {
        InitializeComponent();
    }

    // 收起/展开下栏节点库：
    // - 展开时占固定像素高（可经 GridSplitter 拖拽调整）。
    // - 收起时仅留标题条（Auto 高），吸附在底部，隐藏拖拽分隔条。
    private void OnToggleLibrary(object? sender, Avalonia.Interactivity.RoutedEventArgs e)
    {
        if (WorkspaceGrid is null || LibrarySplitter is null || LibraryContent is null)
        {
            return;
        }

        var row = WorkspaceGrid.RowDefinitions[2];
        var opening = !LibraryContent.IsVisible;

        if (opening)
        {
            LibraryContent.IsVisible = true;
            LibrarySplitter.IsVisible = true;
            row.Height = _savedLibraryHeight;
        }
        else
        {
            // 记录当前高度（若有效）后收起。
            if (row.Height.IsAbsolute && row.Height.Value > 60)
            {
                _savedLibraryHeight = row.Height;
            }
            LibraryContent.IsVisible = false;
            LibrarySplitter.IsVisible = false;
            row.Height = GridLength.Auto;
        }
    }
}
