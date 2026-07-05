using System.Collections.ObjectModel;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// 配置页 ViewModel：顶部横向标签栏（7 标签）+ 下方内容区。
/// 本轮只承载视觉骨架文案与标签结构，后端接线（get_provider_config / save_* 等）留待交互阶段。
public sealed class SettingsPageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private SettingsTabViewModel _selectedTab;
    private string _selectedLanguage;

    public SettingsPageViewModel(DisplayNameService displayNames)
    {
        _displayNames = displayNames;
        _selectedLanguage = displayNames.CurrentLanguage;

        // 可用语言列表（code, 显示名）
        LanguageOptions = new ObservableCollection<LanguageOption>
        {
            new("zh", displayNames.Text("ui.settings.misc.language.zh")),
            new("en", displayNames.Text("ui.settings.misc.language.en")),
            new("ja", displayNames.Text("ui.settings.misc.language.ja")),
        };

        Tabs = new ObservableCollection<SettingsTabViewModel>
        {
            CreateTab("general", "ui.settings.tab.general"),
            CreateTab("models", "ui.settings.tab.models"),
            CreateTab("presets", "ui.settings.tab.presets"),
            CreateTab("automation", "ui.settings.tab.automation"),
            CreateTab("permissions", "ui.settings.tab.permissions"),
            CreateTab("personalization", "ui.settings.tab.personalization"),
            CreateTab("misc", "ui.settings.tab.misc"),
        };

        _selectedTab = Tabs[0];
        _selectedTab.IsSelected = true;
    }

    public string Title => _displayNames.Text("ui.settings.title");

    // 杂项页：语言选项
    public string LanguageLabel => _displayNames.Text("ui.settings.misc.language");

    public string LanguageDescText => _displayNames.Text("ui.settings.misc.language.desc");

    /// 可用语言列表。
    public ObservableCollection<LanguageOption> LanguageOptions { get; }

    /// 当前选中语言；切换时即时调用 DisplayNameService.SwitchLanguage。
    public string SelectedLanguage
    {
        get => _selectedLanguage;
        set
        {
            if (SetProperty(ref _selectedLanguage, value))
            {
                _displayNames.SwitchLanguage(value);
            }
        }
    }

    public ObservableCollection<SettingsTabViewModel> Tabs { get; }

    public SettingsTabViewModel SelectedTab
    {
        get => _selectedTab;
        private set => SetProperty(ref _selectedTab, value);
    }

    private SettingsTabViewModel CreateTab(string id, string key)
    {
        return new SettingsTabViewModel(id, _displayNames.Text(key), SelectTab);
    }

    private void SelectTab(SettingsTabViewModel tab)
    {
        foreach (var item in Tabs)
        {
            item.IsSelected = item == tab;
        }
        SelectedTab = tab;
    }
}

/// 语言选项（code + 显示名）。
public sealed record LanguageOption(string Code, string Label);

/// 配置页单个标签。
public sealed class SettingsTabViewModel : ViewModelBase
{
    private bool _isSelected;

    public SettingsTabViewModel(string id, string title, Action<SettingsTabViewModel> select)
    {
        Id = id;
        Title = title;
        SelectCommand = new RelayCommand(() => select(this));
    }

    public string Id { get; }

    public string Title { get; }

    public RelayCommand SelectCommand { get; }

    public bool IsSelected
    {
        get => _isSelected;
        set => SetProperty(ref _isSelected, value);
    }
}
