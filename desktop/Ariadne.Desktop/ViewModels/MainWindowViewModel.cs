using System.Collections.ObjectModel;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class MainWindowViewModel : ViewModelBase
{
    private const string AppVersion = "0.1.0";
    private static readonly string[] ProjectScopedPageIds = { "workspace", "works", "git", "run_logs", "settings" };
    private static readonly string[] PreloadedProjectPageIds = { "workspace", "works", "git" };
    /// <summary>无项目时也可进入的页面（侧栏跳过开始页）。</summary>
    private static readonly HashSet<string> AlwaysAvailablePageIds = new(StringComparer.Ordinal)
    {
        "workspace", "works", "git", "run_logs", "templates", "settings",
    };

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private object _currentPage;
    private string _projectTitle;
    private string _backendStatus;
    private string _notificationText = string.Empty;
    private string _budgetStatusText;
    private double _budgetUsagePercent;
    private bool _sidebarExpanded = true;
    private bool _hasOpenProject;
    private string? _lastNavId;
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
        // 标题栏：始终可打开/切换项目
        SwitchProjectCommand = new RelayCommand(() => _ = RunWelcomeCommandAfterLeaveGuardAsync(Welcome.OpenProjectCommand));

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

    public string ProjectMenuText => _displayNames.Text("ui.layout.switch_recent_projects");

    public string CreateProjectText => _displayNames.Text("ui.layout.create_project");

    public string OpenProjectText => _displayNames.Text("ui.layout.open_project");

    public string SwitchProjectText => _displayNames.Text("ui.layout.switch_project");

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

    public double BudgetUsagePercent
    {
        get => _budgetUsagePercent;
        set => SetProperty(ref _budgetUsagePercent, value);
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

    /// <summary>已进入工作台时打开/切换项目（与 Open 同源）。</summary>
    public RelayCommand SwitchProjectCommand { get; }

    public RelayCommand LeaveProjectCommand => new(() => _ = LeaveProjectAsync());

    public bool HasOpenProject
    {
        get => _hasOpenProject;
        private set => SetProperty(ref _hasOpenProject, value);
    }

    public async Task InitializeAsync()
    {
        await Welcome.LoadAsync().ConfigureAwait(true);
        RefreshProjectMenuItems();
        _lastNavId = SessionNavStore.LoadLastNavId();
        var status = await _backend.GetAppStatusAsync().ConfigureAwait(true);
        if (status is null)
        {
            BackendStatus = _displayNames.Text("ui.status.unavailable");
            // 无后端时仍允许侧栏进入空页面
            TryRestoreLastNavWithoutProject();
            return;
        }

        ThemeApplication.Apply(
                status.Preferences.Theme,
                status.Preferences.ThemeMainColor,
                status.Preferences.ThemeSurfaceColor,
                status.Preferences.ThemeBrandColor,
                status.Preferences.ThemeMainColorDark,
                status.Preferences.ThemeSurfaceColorDark,
                status.Preferences.ThemeBrandColorDark,
                status.Preferences.ThemeFollowSystemColors);
        if (status.CurrentProject is not null
            && !string.IsNullOrWhiteSpace(status.CurrentProject.ProjectRoot))
        {
            await EnterProjectAsync(status.CurrentProject, createPage: false).ConfigureAwait(true);
            await RefreshSidebarBadgesAsync(status.Badges).ConfigureAwait(true);
        }
        else
        {
            HasOpenProject = false;
            BackendStatus = _displayNames.Text("ui.status.healthy");
            // 上次侧栏跳过开始页：恢复到暂存的导航页
            TryRestoreLastNavWithoutProject();
        }
    }

    private async Task EnterProjectAsync(CurrentProjectStatus project)
    {
        await EnterProjectAsync(project, createPage: true).ConfigureAwait(true);
    }

    private async Task EnterProjectAsync(CurrentProjectStatus project, bool createPage)
    {
        // 同步桌面侧项目根，避免页面误判「无项目」而只显示空态
        if (!string.IsNullOrWhiteSpace(project.ProjectRoot) && !_backend.HasProjectRoot)
        {
            try
            {
                await _backend.SetProjectRootAsync(project.ProjectRoot).ConfigureAwait(true);
            }
            catch
            {
                // 后端已认可的当前项目时，尽力同步；失败仍进入 UI
            }
        }

        await ApplySavedLanguageAsync().ConfigureAwait(true);
        await Welcome.LoadAsync().ConfigureAwait(true);
        RefreshProjectMenuItems();
        HasOpenProject = true;
        ProjectTitle = _displayNames.Format("ui.window.project_title", new Dictionary<string, string>
        {
            ["name"] = project.ProjectName,
        });
        // 切换项目必须清缓存并重载，避免旧项目页面残留
        ClearProjectScopedPageCache();
        BackendStatus = _displayNames.Text("ui.status.healthy");
        NotificationText = string.Empty;
        await RefreshBudgetStatusAsync().ConfigureAwait(true);

        var targetId = !string.IsNullOrWhiteSpace(_lastNavId) && AlwaysAvailablePageIds.Contains(_lastNavId)
            ? _lastNavId!
            : "workspace";
        var target = AllNavigationItems().FirstOrDefault(n => n.Id == targetId)
                     ?? PrimaryNavigationItems[0];
        SelectNavigationItem(target, createPage: true);
        await LoadProjectDataPagesAsync().ConfigureAwait(true);
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
        if (!await ConfirmCachedProjectPagesLeaveAsync().ConfigureAwait(true))
        {
            return;
        }

        _backend.ClearProjectRoot();
        HasOpenProject = false;
        foreach (var nav in AllNavigationItems())
        {
            nav.IsSelected = false;
            nav.BadgeCount = 0;
        }
        ProjectTitle = _displayNames.Text("ui.window.no_project_title");
        BackendStatus = _displayNames.Text("ui.status.unavailable");
        NotificationText = string.Empty;
        ClearProjectScopedPageCache();
        BudgetStatusText = _displayNames.Text("ui.common.none");
        BudgetUsagePercent = 0;
        // 离开项目回到开始页；侧栏暂存的 nav 仍保留，便于再次点侧栏进入
        CurrentPage = Welcome;
        await Welcome.LoadAsync().ConfigureAwait(true);
        RefreshProjectMenuItems();
    }

    private async Task RunWelcomeCommandAfterLeaveGuardAsync(RelayCommand command)
    {
        if (!await ConfirmCachedProjectPagesLeaveAsync().ConfigureAwait(true))
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
        OnPropertyChanged(nameof(ProjectMenuText));
        OnPropertyChanged(nameof(CreateProjectText));
        OnPropertyChanged(nameof(OpenProjectText));
        OnPropertyChanged(nameof(SwitchProjectText));
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
            "git" => new GitPageViewModel(_displayNames, _backend, ConfirmCachedProjectPagesLeaveAsync, ReloadCachedProjectPagesAsync),
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
        // 已在该页且不在 Welcome：忽略；在 Welcome 上点侧栏必须能切走
        if (item.IsSelected && !ReferenceEquals(CurrentPage, Welcome))
        {
            return;
        }

        if (!await ConfirmCurrentPageLeaveAsync().ConfigureAwait(true))
        {
            return;
        }

        if (!AlwaysAvailablePageIds.Contains(item.Id))
        {
            return;
        }

        foreach (var nav in AllNavigationItems())
        {
            nav.IsSelected = nav == item;
        }

        // 从开始页点侧栏 = 进入工作台页（可无项目，显示空态）；记住导航以便下次恢复
        try
        {
            CurrentPage = item.PageFactory();
        }
        catch (Exception)
        {
            // 构造失败不抛工程师信息；回到开始页并清选中
            NotificationText = string.Empty;
            CurrentPage = Welcome;
            foreach (var nav in AllNavigationItems())
            {
                nav.IsSelected = false;
            }
            return;
        }

        _lastNavId = item.Id;
        SessionNavStore.SaveLastNavId(item.Id);

        // 无项目时标题保持「未打开项目」，状态保持健康/连接文案，不要空白无反应
        if (!HasOpenProject && string.IsNullOrWhiteSpace(ProjectTitle))
        {
            ProjectTitle = _displayNames.Text("ui.window.no_project_title");
        }

        if (string.IsNullOrWhiteSpace(BackendStatus)
            || string.Equals(BackendStatus, _displayNames.Text("ui.status.unavailable"), StringComparison.Ordinal))
        {
            // 仅在原先是不可用占位时，进入壳后标为健康（有后端时 Initialize 已设过）
            if (HasOpenProject || _backend.HasProjectRoot)
            {
                BackendStatus = _displayNames.Text("ui.status.healthy");
            }
        }

        OnPropertyChanged(nameof(HeaderStatusText));
    }

    /// <summary>无打开项目时恢复上次侧栏页（跳过开始页的暂存）。</summary>
    private void TryRestoreLastNavWithoutProject()
    {
        if (string.IsNullOrWhiteSpace(_lastNavId) || !AlwaysAvailablePageIds.Contains(_lastNavId))
        {
            return;
        }

        var item = AllNavigationItems().FirstOrDefault(n => n.Id == _lastNavId);
        if (item is null)
        {
            return;
        }

        foreach (var nav in AllNavigationItems())
        {
            nav.IsSelected = nav == item;
        }

        try
        {
            CurrentPage = item.PageFactory();
        }
        catch
        {
            CurrentPage = Welcome;
            foreach (var nav in AllNavigationItems())
            {
                nav.IsSelected = false;
            }
        }
    }

    private async Task<bool> ConfirmCurrentPageLeaveAsync()
    {
        return CurrentPage is not IUnsavedChangesGuard guard
               || await guard.ConfirmLeaveIfNeededAsync().ConfigureAwait(true);
    }

    private async Task<bool> ConfirmCachedProjectPagesLeaveAsync()
    {
        foreach (var page in _pageCache.Values)
        {
            if (page is IUnsavedChangesGuard guard
                && !await guard.ConfirmLeaveIfNeededAsync().ConfigureAwait(true))
            {
                return false;
            }
        }
        return true;
    }

    private async Task ReloadCachedProjectPagesAsync()
    {
        foreach (var page in _pageCache.Values)
        {
            if (page is IProjectDataReloadable reloadable)
            {
                await reloadable.ReloadProjectDataAsync().ConfigureAwait(true);
            }
        }
        await RefreshBudgetStatusAsync().ConfigureAwait(true);
        await RefreshSidebarBadgesAsync(new SidebarBadgeCounts(0, 0, 0)).ConfigureAwait(true);
    }

    private async Task LoadProjectDataPagesAsync()
    {
        foreach (var id in PreloadedProjectPageIds)
        {
            var page = CreatePage(id, string.Empty);
            if (page is IProjectDataReloadable reloadable)
            {
                await reloadable.ReloadProjectDataAsync().ConfigureAwait(true);
            }
        }
        await RefreshSidebarBadgesAsync(new SidebarBadgeCounts(0, 0, 0)).ConfigureAwait(true);
    }

    private void ClearProjectScopedPageCache()
    {
        foreach (var id in ProjectScopedPageIds)
        {
            _pageCache.Remove(id);
        }
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
            // 菜单打开最近项目 = 切换项目（经 leave 守卫 + EnterProject 清缓存）
            ProjectMenuItems.Add(new ProjectMenuItemViewModel(
                item.Name,
                item.ProjectRoot,
                new RelayCommand(() => _ = SwitchToProjectRootAsync(item.ProjectRoot))));
        }
    }

    private async Task SwitchToProjectRootAsync(string projectRoot)
    {
        if (!await ConfirmCachedProjectPagesLeaveAsync().ConfigureAwait(true))
        {
            return;
        }

        // 复用 Welcome 的打开逻辑（预检 + OpenProjectAsync + EnterProject）
        await Welcome.OpenProjectRootForHostAsync(projectRoot).ConfigureAwait(true);
        await Welcome.LoadAsync().ConfigureAwait(true);
        RefreshProjectMenuItems();
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
            BudgetUsagePercent = 0;
        }
    }

    private void ApplyBudgetStatus(BudgetStatus status)
    {
        if (status.BudgetUsd <= 0)
        {
            BudgetUsagePercent = 0;
            BudgetStatusText = _displayNames.Text("ui.layout.budget_unlimited");
            return;
        }
        var total = status.BudgetUsd <= 0 ? 0 : Math.Clamp(status.SpentUsd / status.BudgetUsd, 0, 1);
        BudgetUsagePercent = total * 100;
        BudgetStatusText = _displayNames.Format("ui.layout.budget_status", new Dictionary<string, string>
        {
            ["spent"] = status.SpentUsd.ToString("0.##"),
            ["budget"] = status.BudgetUsd.ToString("0.##"),
        });
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
