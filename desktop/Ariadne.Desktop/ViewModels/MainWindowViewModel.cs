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
    private string _budgetStatusText;
    private double _budgetUsageWidth;
    private bool _autoModeEnabled;
    private bool _suppressAutoModeSave;
    private bool _sidebarExpanded = true;

    public MainWindowViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        Welcome = new WelcomeViewModel(displayNames, backend, EnterProjectAsync);
        _currentPage = Welcome;
        _projectTitle = displayNames.Text("ui.window.no_project_title");
        _backendStatus = displayNames.Text("ui.status.unavailable");
        _budgetStatusText = displayNames.Text("ui.common.none");
        ProjectMenuItems = new ObservableCollection<ProjectMenuItemViewModel>();

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

        PrimaryNavigationItems[0].IsSelected = false;
    }

    public WelcomeViewModel Welcome { get; }

    /// 全局弹窗服务（未保存离开、通用确认等）；MainWindow 内叠层渲染。
    public DialogService Dialog => DialogService.Current;

    public ObservableCollection<NavigationItemViewModel> PrimaryNavigationItems { get; }

    public ObservableCollection<NavigationItemViewModel> SecondaryNavigationItems { get; }

    public ObservableCollection<ProjectMenuItemViewModel> ProjectMenuItems { get; }

    public string AppName => _displayNames.Text("ui.brand.name");

    public string AppLogoLetter => _displayNames.Text("ui.brand.logo_letter");

    public string ToggleSidebarText => _displayNames.Text("ui.action.toggle_sidebar");

    public string MinimizeWindowText => _displayNames.Text("ui.window.minimize");

    public string MaximizeWindowText => _displayNames.Text("ui.window.maximize");

    public string CloseWindowText => _displayNames.Text("ui.window.close");

    public string BudgetLabel => _displayNames.Text("ui.layout.budget");

    public string AutoModeLabel => _displayNames.Text("ui.settings.automation.auto_mode");

    public string ProjectMenuText => _displayNames.Text("ui.layout.switch_recent_projects");

    public string CreateProjectText => _displayNames.Text("ui.layout.create_project");

    public string OpenProjectText => _displayNames.Text("ui.layout.open_project");

    public string LeaveProjectText => _displayNames.Text("ui.layout.leave_project");

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

    public string BudgetStatusText
    {
        get => _budgetStatusText;
        set => SetProperty(ref _budgetStatusText, value);
    }

    public double BudgetUsageWidth
    {
        get => _budgetUsageWidth;
        set => SetProperty(ref _budgetUsageWidth, value);
    }

    public bool AutoModeEnabled
    {
        get => _autoModeEnabled;
        set
        {
            if (!SetProperty(ref _autoModeEnabled, value) || _suppressAutoModeSave)
            {
                return;
            }
            _ = SetAutoModeAsync(value);
        }
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

    public RelayCommand CreateProjectCommand => new(() => Welcome.CreateProjectCommand.Execute(null));

    public RelayCommand OpenProjectCommand => new(() => Welcome.OpenProjectCommand.Execute(null));

    public RelayCommand LeaveProjectCommand => new(LeaveProject);

    public async Task InitializeAsync()
    {
        await Welcome.LoadAsync().ConfigureAwait(true);
        RefreshProjectMenuItems();
        var status = await _backend.GetAppStatusAsync().ConfigureAwait(true);
        if (status is null)
        {
            BackendStatus = _displayNames.Text("ui.status.unavailable");
            return;
        }

        await EnterProjectAsync(status.CurrentProject, createPage: false).ConfigureAwait(true);
        await RefreshSidebarBadgesAsync(status.Badges).ConfigureAwait(true);
    }

    private async Task EnterProjectAsync(CurrentProjectStatus project)
    {
        await EnterProjectAsync(project, createPage: true).ConfigureAwait(true);
    }

    private async Task EnterProjectAsync(CurrentProjectStatus project, bool createPage)
    {
        await Welcome.LoadAsync().ConfigureAwait(true);
        RefreshProjectMenuItems();
        ProjectTitle = _displayNames.Format("ui.window.project_title", new Dictionary<string, string>
        {
            ["name"] = project.ProjectName,
        });
        BackendStatus = ProjectTitle;
        await RefreshBudgetStatusAsync().ConfigureAwait(true);
        SelectNavigationItem(PrimaryNavigationItems[0], createPage);
    }

    private void LeaveProject()
    {
        _backend.ClearProjectRoot();
        foreach (var nav in AllNavigationItems())
        {
            nav.IsSelected = false;
            nav.BadgeCount = 0;
        }
        ProjectTitle = _displayNames.Text("ui.window.no_project_title");
        BackendStatus = _displayNames.Text("ui.status.unavailable");
        BudgetStatusText = _displayNames.Text("ui.common.none");
        BudgetUsageWidth = 0;
        _suppressAutoModeSave = true;
        AutoModeEnabled = false;
        _suppressAutoModeSave = false;
        CurrentPage = Welcome;
        _ = Welcome.LoadAsync();
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
            _ => Welcome,
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

    private void SelectNavigationItem(NavigationItemViewModel item, bool createPage)
    {
        foreach (var nav in AllNavigationItems())
        {
            nav.IsSelected = nav == item;
        }
        if (createPage || ReferenceEquals(CurrentPage, Welcome))
        {
            CurrentPage = item.PageFactory();
        }
    }

    private void RefreshProjectMenuItems()
    {
        ProjectMenuItems.Clear();
        foreach (var item in Welcome.RecentProjects)
        {
            ProjectMenuItems.Add(new ProjectMenuItemViewModel(item.Name, item.ProjectRoot, item.OpenCommand));
        }
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

    private async Task RefreshBudgetStatusAsync()
    {
        try
        {
            ApplyBudgetStatus(await _backend.GetBudgetStatusAsync().ConfigureAwait(true));
        }
        catch (Exception ex)
        {
            BudgetStatusText = ex.Message;
            BudgetUsageWidth = 0;
        }
    }

    private async Task SetAutoModeAsync(bool enabled)
    {
        try
        {
            await _backend.SetAutoModeAsync(enabled).ConfigureAwait(true);
            await RefreshBudgetStatusAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            BackendStatus = ex.Message;
            _suppressAutoModeSave = true;
            AutoModeEnabled = !enabled;
            _suppressAutoModeSave = false;
        }
    }

    private void ApplyBudgetStatus(BudgetStatus status)
    {
        var total = status.BudgetUsd <= 0 ? 0 : Math.Clamp(status.SpentUsd / status.BudgetUsd, 0, 1);
        BudgetUsageWidth = total * 92;
        BudgetStatusText = _displayNames.Format("ui.layout.budget_status", new Dictionary<string, string>
        {
            ["spent"] = status.SpentUsd.ToString("0.##"),
            ["budget"] = status.BudgetUsd.ToString("0.##"),
        });
        _suppressAutoModeSave = true;
        AutoModeEnabled = status.AutoModeEnabled;
        _suppressAutoModeSave = false;
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

public sealed class ProjectMenuItemViewModel
{
    public ProjectMenuItemViewModel(string name, string projectRoot, RelayCommand openCommand)
    {
        Name = name;
        ProjectRoot = projectRoot;
        OpenCommand = openCommand;
    }

    public string Name { get; }
    public string ProjectRoot { get; }
    public RelayCommand OpenCommand { get; }
}
