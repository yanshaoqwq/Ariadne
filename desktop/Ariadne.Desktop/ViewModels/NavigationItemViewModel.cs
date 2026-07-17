using System.Windows.Input;
using Avalonia.Media;

namespace Ariadne.Desktop.ViewModels;

public sealed class NavigationItemViewModel : ViewModelBase
{
    private bool _isSelected;
    private int _badgeCount;
    private string _title;
    private bool _sidebarExpanded = true;

    public NavigationItemViewModel(string id, string title, Geometry? icon, Action<NavigationItemViewModel> select)
    {
        Id = id;
        _title = title;
        Icon = icon;
        SelectCommand = new RelayCommand(() => select(this));
    }

    public string Id { get; }

    public string Title { get => _title; set => SetProperty(ref _title, value); }

    /// 矢量图标几何（来自主题资源 Ariadne.Icon.*），用 Path 渲染，不依赖任何字体。
    public Geometry? Icon { get; }

    public ICommand SelectCommand { get; }

    public bool IsSelected
    {
        get => _isSelected;
        set => SetProperty(ref _isSelected, value);
    }

    /// <summary>侧栏展开态：由主窗同步，驱动导航模板（U66）。</summary>
    public bool SidebarExpanded
    {
        get => _sidebarExpanded;
        set
        {
            if (SetProperty(ref _sidebarExpanded, value))
            {
                OnPropertyChanged(nameof(SidebarCollapsed));
            }
        }
    }

    public bool SidebarCollapsed => !SidebarExpanded;

    public int BadgeCount
    {
        get => _badgeCount;
        set
        {
            if (SetProperty(ref _badgeCount, value))
            {
                OnPropertyChanged(nameof(HasBadge));
            }
        }
    }

    public bool HasBadge => BadgeCount > 0;
}
