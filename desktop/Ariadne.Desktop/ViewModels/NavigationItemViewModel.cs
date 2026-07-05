using System.Windows.Input;
using Avalonia.Media;

namespace Ariadne.Desktop.ViewModels;

public sealed class NavigationItemViewModel : ViewModelBase
{
    private bool _isSelected;
    private int _badgeCount;

    public NavigationItemViewModel(string id, string title, Geometry? icon, Func<object> pageFactory, Action<NavigationItemViewModel> select)
    {
        Id = id;
        Title = title;
        Icon = icon;
        PageFactory = pageFactory;
        SelectCommand = new RelayCommand(() => select(this));
    }

    public string Id { get; }

    public string Title { get; }

    /// 矢量图标几何（来自主题资源 Ariadne.Icon.*），用 Path 渲染，不依赖任何字体。
    public Geometry? Icon { get; }

    public Func<object> PageFactory { get; }

    public ICommand SelectCommand { get; }

    public bool IsSelected
    {
        get => _isSelected;
        set => SetProperty(ref _isSelected, value);
    }

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
