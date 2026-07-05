using System.Collections.ObjectModel;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class MainWindowViewModel : ViewModelBase
{
    private const string AppVersion = "0.1.0";

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private object _currentPage;
    private string _projectTitle;
    private string _backendStatus;
    private bool _sidebarExpanded = true;

    public MainWindowViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        Welcome = new WelcomeViewModel(displayNames, backend);
        _currentPage = Welcome;
        _projectTitle = displayNames.Text("ui.window.no_project_title");
        _backendStatus = displayNames.Text("ui.status.unavailable");

        // 上组：创作主流程
        PrimaryNavigationItems = new ObservableCollection<NavigationItemViewModel>
        {
            CreateNav("workspace", "ui.nav.workspace", IconGeometries.Workspace),
            CreateNav("works", "ui.nav.works", IconGeometries.Works),
            CreateNav("git", "ui.nav.git", IconGeometries.Git),
            CreateNav("run_logs", "ui.nav.run_logs", IconGeometries.RunLog),
        };

        // 下组：扩展与配置（与上组之间留大间隔）
        SecondaryNavigationItems = new ObservableCollection<NavigationItemViewModel>
        {
            CreateNav("templates", "ui.nav.templates", IconGeometries.Templates),
            CreateNav("settings", "ui.nav.settings", IconGeometries.Settings),
        };

        PrimaryNavigationItems[0].IsSelected = true;
        _currentPage = PrimaryNavigationItems[0].PageFactory();
    }

    public WelcomeViewModel Welcome { get; }

    /// 全局弹窗服务（未保存离开、通用确认等）；MainWindow 内叠层渲染。
    public DialogService Dialog => DialogService.Current;

    public ObservableCollection<NavigationItemViewModel> PrimaryNavigationItems { get; }

    public ObservableCollection<NavigationItemViewModel> SecondaryNavigationItems { get; }

    public string AppName => _displayNames.Text("ui.brand.name");

    public string AppLogoLetter => _displayNames.Text("ui.brand.logo_letter");

    public string ToggleSidebarText => _displayNames.Text("ui.action.toggle_sidebar");

    public string MinimizeWindowText => _displayNames.Text("ui.window.minimize");

    public string MaximizeWindowText => _displayNames.Text("ui.window.maximize");

    public string CloseWindowText => _displayNames.Text("ui.window.close");

    public string BudgetLabel => _displayNames.Text("ui.layout.budget");

    public string AutoModeLabel => _displayNames.Text("ui.settings.automation.auto_mode");

    public string FeedbackText => _displayNames.Text("ui.layout.feedback");

    public string VersionText => _displayNames.Format("ui.layout.version_value", new Dictionary<string, string>
    {
        ["version"] = AppVersion,
    });

    public string ProjectTitle
    {
        get => _projectTitle;
        set => SetProperty(ref _projectTitle, value);
    }

    public string BackendStatus
    {
        get => _backendStatus;
        set => SetProperty(ref _backendStatus, value);
    }

    public bool SidebarExpanded
    {
        get => _sidebarExpanded;
        set => SetProperty(ref _sidebarExpanded, value);
    }

    public object CurrentPage
    {
        get => _currentPage;
        private set => SetProperty(ref _currentPage, value);
    }

    public RelayCommand ToggleSidebarCommand => new(() => SidebarExpanded = !SidebarExpanded);

    public RelayCommand OpenVersionCommand => new(() => BackendStatus = VersionText);

    public RelayCommand OpenFeedbackCommand => new(() => BackendStatus = FeedbackText);

    public async Task InitializeAsync()
    {
        await Welcome.LoadAsync().ConfigureAwait(true);
        var status = await _backend.GetAppStatusAsync().ConfigureAwait(true);
        if (status is null)
        {
            BackendStatus = _displayNames.Text("ui.status.unavailable");
            return;
        }

        ProjectTitle = _displayNames.Format("ui.window.project_title", new Dictionary<string, string>
        {
            ["name"] = status.CurrentProject.ProjectName,
        });
        BackendStatus = ProjectTitle;
        await RefreshSidebarBadgesAsync(status.Badges).ConfigureAwait(true);
    }

    private NavigationItemViewModel CreateNav(string id, string key, Avalonia.Media.Geometry? icon)
    {
        return new NavigationItemViewModel(
            id,
            _displayNames.Text(key),
            icon,
            () => CreatePage(id, key),
            item => _ = SelectNavigationItemAsync(item));
    }

    private object CreatePage(string id, string key)
    {
        return id switch
        {
            "workspace" => new WorkspacePageViewModel(_displayNames, _backend),
            "works" => new WorksPageViewModel(_displayNames, _backend),
            "git" => new GitPageViewModel(_displayNames, _backend),
            "run_logs" => new RunLogPageViewModel(_displayNames, _backend),
            "templates" => new TemplateMarketPageViewModel(_displayNames, _backend),
            "settings" => new SettingsPageViewModel(_displayNames, _backend),
            _ => new PlaceholderPageViewModel(_displayNames.Text(key), _displayNames.Text("ui.common.wired"), string.Empty),
        };
    }

    private async Task SelectNavigationItemAsync(NavigationItemViewModel item)
    {
        if (item.IsSelected)
        {
            return;
        }

        if (CurrentPage is IUnsavedChangesGuard guard && !await guard.ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
        {
            return;
        }

        foreach (var nav in AllNavigationItems())
        {
            nav.IsSelected = nav == item;
        }
        CurrentPage = item.PageFactory();
    }

    private void SetBadge(string id, int value)
    {
        var item = AllNavigationItems().FirstOrDefault(nav => nav.Id == id);
        if (item is not null)
        {
            item.BadgeCount = value;
        }
    }

    private async Task RefreshSidebarBadgesAsync(SidebarBadgeCounts fallback)
    {
        SidebarBadgeCounts badges;
        try
        {
            badges = await _backend.GetSidebarBadgesAsync().ConfigureAwait(true);
        }
        catch
        {
            badges = fallback;
        }

        SetBadge("workspace", badges.Confirmations);
        SetBadge("run_logs", badges.RunLogs);
        SetBadge("settings", badges.Diagnostics);
    }

    private IEnumerable<NavigationItemViewModel> AllNavigationItems()
    {
        foreach (var nav in PrimaryNavigationItems)
        {
            yield return nav;
        }
        foreach (var nav in SecondaryNavigationItems)
        {
            yield return nav;
        }
    }
}
