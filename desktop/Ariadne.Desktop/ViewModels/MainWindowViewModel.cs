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
    private string _notificationText = string.Empty;
    private string _budgetStatusText;
    private double _budgetUsageWidth;
    private bool _autoModeEnabled;
    private bool _suppressAutoModeSave;
    private bool _sidebarExpanded = true;
    private readonly Dictionary<string, object> _pageCache = new();

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
        ToggleSidebarCommand = new RelayCommand(() => SidebarExpanded = !SidebarExpanded);

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
        _displayNames.LanguageChanged += (_, _) => RefreshLocalizedText();
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
    public string AutoModeStateText => AutoModeEnabled
        ? _displayNames.Text("ui.common.enabled")
        : _displayNames.Text("ui.common.disabled");

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
        set
        {
            if (SetProperty(ref _backendStatus, value))
            {
                OnPropertyChanged(nameof(HeaderStatusText));
            }
        }
    }

    public string NotificationText
    {
        get => _notificationText;
        set
        {
            if (SetProperty(ref _notificationText, value))
            {
                OnPropertyChanged(nameof(HeaderStatusText));
            }
        }
    }

    public string HeaderStatusText => string.IsNullOrWhiteSpace(NotificationText) ? BackendStatus : NotificationText;

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
            if (!SetProperty(ref _autoModeEnabled, value))
            {
                return;
            }
            OnPropertyChanged(nameof(AutoModeStateText));
            if (_suppressAutoModeSave)
            {
                return;
            }
            _ = SetAutoModeAsync(value);
        }
    }

    public bool SidebarExpanded
    {
        get => _sidebarExpanded;
        set
        {
            if (SetProperty(ref _sidebarExpanded, value))
            {
                OnPropertyChanged(nameof(SidebarWidth));
            }
        }
    }

    public double SidebarWidth => SidebarExpanded ? 204 : 44;

    public object CurrentPage
    {
        get => _currentPage;
        private set => SetProperty(ref _currentPage, value);
    }

    public RelayCommand ToggleSidebarCommand { get; }

    public RelayCommand OpenVersionCommand => new(() => _ = ShowVersionAsync());

    public RelayCommand OpenFeedbackCommand => new(() => _ = ShowFeedbackAsync());

    public RelayCommand CreateProjectCommand => new(() => _ = RunWelcomeCommandAfterLeaveGuardAsync(Welcome.CreateProjectCommand));

    public RelayCommand OpenProjectCommand => new(() => _ = RunWelcomeCommandAfterLeaveGuardAsync(Welcome.OpenProjectCommand));

    public RelayCommand LeaveProjectCommand => new(() => _ = LeaveProjectAsync());

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
        await ApplySavedLanguageAsync().ConfigureAwait(true);
        await Welcome.LoadAsync().ConfigureAwait(true);
        RefreshProjectMenuItems();
        ProjectTitle = _displayNames.Format("ui.window.project_title", new Dictionary<string, string>
        {
            ["name"] = project.ProjectName,
        });
        if (createPage)
        {
            _pageCache.Clear();
        }
        BackendStatus = _displayNames.Text("ui.status.healthy");
        NotificationText = string.Empty;
        await RefreshBudgetStatusAsync().ConfigureAwait(true);
        SelectNavigationItem(PrimaryNavigationItems[0], createPage);
    }

    private async Task ApplySavedLanguageAsync()
    {
        try
        {
            var appSettings = await _backend.GetAppSettingsAsync().ConfigureAwait(true);
            _displayNames.SwitchLanguage(appSettings.App.Locale);
        }
        catch
        {
            // 项目尚未完全可用时保留当前语言；后续进入设置页仍会按配置刷新。
        }
    }

    private async Task LeaveProjectAsync()
    {
        if (!await ConfirmCurrentPageLeaveAsync().ConfigureAwait(true))
        {
            return;
        }

        _backend.ClearProjectRoot();
        foreach (var nav in AllNavigationItems())
        {
            nav.IsSelected = false;
            nav.BadgeCount = 0;
        }
        ProjectTitle = _displayNames.Text("ui.window.no_project_title");
        BackendStatus = _displayNames.Text("ui.status.unavailable");
        NotificationText = string.Empty;
        _pageCache.Clear();
        BudgetStatusText = _displayNames.Text("ui.common.none");
        BudgetUsageWidth = 0;
        _suppressAutoModeSave = true;
        AutoModeEnabled = false;
        _suppressAutoModeSave = false;
        CurrentPage = Welcome;
        _ = Welcome.LoadAsync();
    }

    private async Task RunWelcomeCommandAfterLeaveGuardAsync(RelayCommand command)
    {
        if (!await ConfirmCurrentPageLeaveAsync().ConfigureAwait(true))
        {
            return;
        }

        command.Execute(null);
    }

    private async Task ShowVersionAsync()
    {
        NotificationText = VersionText;
        await DialogService.Current.ConfirmAsync(HelpDialogFactory.CreateVersionDialog(_displayNames, VersionText)).ConfigureAwait(true);
    }

    private async Task ShowFeedbackAsync()
    {
        NotificationText = FeedbackText;
        await DialogService.Current.ConfirmAsync(HelpDialogFactory.CreateFeedbackDialog(_displayNames)).ConfigureAwait(true);
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

    private void RefreshLocalizedText()
    {
        OnPropertyChanged(nameof(AppName));
        OnPropertyChanged(nameof(AppLogoLetter));
        OnPropertyChanged(nameof(ToggleSidebarText));
        OnPropertyChanged(nameof(MinimizeWindowText));
        OnPropertyChanged(nameof(MaximizeWindowText));
        OnPropertyChanged(nameof(CloseWindowText));
        OnPropertyChanged(nameof(BudgetLabel));
        OnPropertyChanged(nameof(AutoModeLabel));
        OnPropertyChanged(nameof(AutoModeStateText));
        OnPropertyChanged(nameof(ProjectMenuText));
        OnPropertyChanged(nameof(CreateProjectText));
        OnPropertyChanged(nameof(OpenProjectText));
        OnPropertyChanged(nameof(LeaveProjectText));
        OnPropertyChanged(nameof(FeedbackText));
        OnPropertyChanged(nameof(VersionText));
        OnPropertyChanged(nameof(HeaderStatusText));
        foreach (var item in AllNavigationItems())
        {
            item.Title = item.Id switch
            {
                "workspace" => _displayNames.Text("ui.nav.workspace"),
                "works" => _displayNames.Text("ui.nav.works"),
                "git" => _displayNames.Text("ui.nav.git"),
                "run_logs" => _displayNames.Text("ui.nav.run_logs"),
                "templates" => _displayNames.Text("ui.nav.templates"),
                "settings" => _displayNames.Text("ui.nav.settings"),
                _ => item.Title,
            };
        }
    }

    private object CreatePage(string id, string key)
    {
        if (_pageCache.TryGetValue(id, out var cached))
        {
            return cached;
        }
        object page = id switch
        {
            "workspace" => new WorkspacePageViewModel(_displayNames, _backend),
            "works" => new WorksPageViewModel(_displayNames, _backend),
            "git" => new GitPageViewModel(_displayNames, _backend),
            "run_logs" => new RunLogPageViewModel(_displayNames, _backend),
            "templates" => new TemplateMarketPageViewModel(_displayNames, _backend),
            "settings" => new SettingsPageViewModel(_displayNames, _backend, () => OpenNavigationItemByIdAsync("templates")),
            _ => Welcome,
        };
        _pageCache[id] = page;
        return page;
    }

    private async Task OpenNavigationItemByIdAsync(string id)
    {
        var item = AllNavigationItems().FirstOrDefault(nav => nav.Id == id);
        if (item is not null)
        {
            await SelectNavigationItemAsync(item).ConfigureAwait(true);
        }
    }

    private async Task SelectNavigationItemAsync(NavigationItemViewModel item)
    {
        if (item.IsSelected)
        {
            return;
        }

        if (!await ConfirmCurrentPageLeaveAsync().ConfigureAwait(true))
        {
            return;
        }

        foreach (var nav in AllNavigationItems())
        {
            nav.IsSelected = nav == item;
        }
        CurrentPage = item.PageFactory();
    }

    private async Task<bool> ConfirmCurrentPageLeaveAsync()
    {
        return CurrentPage is not IUnsavedChangesGuard guard
               || await guard.ConfirmLeaveIfNeededAsync().ConfigureAwait(true);
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
            ProjectMenuItems.Add(new ProjectMenuItemViewModel(
                item.Name,
                item.ProjectRoot,
                new RelayCommand(() => _ = RunWelcomeCommandAfterLeaveGuardAsync(item.OpenCommand))));
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
            NotificationText = ex.Message;
            _suppressAutoModeSave = true;
            AutoModeEnabled = !enabled;
            _suppressAutoModeSave = false;
        }
    }

    private void ApplyBudgetStatus(BudgetStatus status)
    {
        if (status.BudgetUsd <= 0)
        {
            BudgetUsageWidth = 0;
            BudgetStatusText = _displayNames.Text("ui.layout.budget_unlimited");
            _suppressAutoModeSave = true;
            AutoModeEnabled = status.AutoModeEnabled;
            _suppressAutoModeSave = false;
            return;
        }
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
