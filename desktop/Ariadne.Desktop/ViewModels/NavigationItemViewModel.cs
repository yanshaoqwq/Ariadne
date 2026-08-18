using System.Windows.Input;
using Avalonia.Media;

namespace Ariadne.Desktop.ViewModels;

public sealed class NavigationItemViewModel : ViewModelBase
{
    private bool _isSelected;
    private bool _isPending;
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

    public string Title
    {
        get => _title;
        set
        {
            if (SetProperty(ref _title, value))
            {
                OnPropertyChanged(nameof(ToolTipText));
            }
        }
    }

    public string? ToolTipText => SidebarCollapsed ? Title : null;

    /// 矢量图标几何（来自主题资源 Ariadne.Icon.*），用 Path 渲染，不依赖任何字体。
    public Geometry? Icon { get; }

    public ICommand SelectCommand { get; }

    public bool IsSelected
    {
        get => _isSelected;
        set => SetProperty(ref _isSelected, value);
    }

    /// <summary>
    /// 该页的数据加载在途（U178-A）。
    ///
    /// 提前 commit 让 <see cref="IsSelected"/> 在点击那一刻就翻转，用户立刻知道
    /// 「点上了」；这个额外的位负责回答下一个问题——「是已经好了，还是还在读」。
    /// 只有它能区分「空页面因为没内容」与「空页面因为还没读到」，
    /// 而这两者在界面上长得一模一样。
    ///
    /// 缺陷版本里没有任何 pending 概念：等待期间界面上零指示。
    /// </summary>
    public bool IsPending
    {
        get => _isPending;
        set => SetProperty(ref _isPending, value);
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
                OnPropertyChanged(nameof(ToolTipText));
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
