using System.Collections.ObjectModel;
using System.Reflection;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class MainWindowViewModel : ViewModelBase, IUserFailureObserver
{
    private static readonly string AppVersion =
        typeof(MainWindowViewModel).Assembly
            .GetCustomAttribute<AssemblyInformationalVersionAttribute>()?
            .InformationalVersion
        ?? typeof(MainWindowViewModel).Assembly.GetName().Version?.ToString(3)
        ?? "0.0.0";
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
    private string _maintenanceBannerText = string.Empty;
    private bool _isMaintenanceBlocking;
    private string _diagnosticSummaryText = string.Empty;
    private string _diagnosticDetailText = string.Empty;
    private UserFailure? _diagnosticFailure;
    private bool _hasDiagnostic;
    private bool _isDiagnosticExpanded;
    private readonly Dictionary<string, object> _pageCache = new();
    private readonly HashSet<string> _loadedPageIds = new(StringComparer.Ordinal);
    private readonly Dictionary<string, Task> _pageLoadTasks = new(StringComparer.Ordinal);
    private CancellationTokenSource _projectPageSessionCts = new();
    private long _projectPageSessionGeneration;

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
        ToggleDiagnosticCommand = new RelayCommand(() => IsDiagnosticExpanded = !IsDiagnosticExpanded);
        ClearDiagnosticCommand = new RelayCommand(ClearDiagnostic);
        UserFacingError.RegisterObserver(this);
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

    public string DiagnosticTitleText => _displayNames.Text("ui.diagnostics.title");

    public string DiagnosticToggleText => _displayNames.Text(
        IsDiagnosticExpanded ? "ui.diagnostics.hide" : "ui.diagnostics.show");

    public string DiagnosticClearText => _displayNames.Text("ui.diagnostics.clear");

    public string DiagnosticSummaryText
    {
        get => _diagnosticSummaryText;
        private set => SetProperty(ref _diagnosticSummaryText, value);
    }

    public string DiagnosticDetailText
    {
        get => _diagnosticDetailText;
        private set => SetProperty(ref _diagnosticDetailText, value);
    }

    public bool HasDiagnostic
    {
        get => _hasDiagnostic;
        private set => SetProperty(ref _hasDiagnostic, value);
    }

    public bool IsDiagnosticExpanded
    {
        get => _isDiagnosticExpanded;
        private set
        {
            if (SetProperty(ref _isDiagnosticExpanded, value))
            {
                OnPropertyChanged(nameof(DiagnosticToggleText));
            }
        }
    }

    /// <summary>D3：维护中/失败时的标题栏横幅文案；空表示无门禁。</summary>
    public string MaintenanceBannerText
    {
        get => _maintenanceBannerText;
        private set => SetProperty(ref _maintenanceBannerText, value);
    }

    public bool IsMaintenanceBlocking
    {
        get => _isMaintenanceBlocking;
        private set => SetProperty(ref _isMaintenanceBlocking, value);
    }

    /// <summary>测试/刷新入口：从后端拉取维护状态并更新横幅。</summary>
    public async Task RefreshMaintenanceStatusAsync()
    {
        if (!_backend.HasProjectRoot)
        {
            ClearMaintenanceBanner();
            return;
        }

        try
        {
            var state = await _backend.GetProjectMaintenanceAsync().ConfigureAwait(true);
            ApplyMaintenanceState(state);
        }
        catch
        {
            // 查询失败不阻塞主 UI；写路径仍由后端门禁拒绝。
            ClearMaintenanceBanner();
        }
    }

    internal void ApplyMaintenanceState(Backend.ProjectMaintenanceState? state)
    {
        if (state is null
            || string.IsNullOrWhiteSpace(state.Status)
            || (state.Status != "active" && state.Status != "failed"))
        {
            ClearMaintenanceBanner();
            return;
        }

        IsMaintenanceBlocking = true;
        var kind = string.IsNullOrWhiteSpace(state.Kind) ? "maintenance" : state.Kind;
        var phase = string.IsNullOrWhiteSpace(state.Phase) ? state.Status : state.Phase;
        var error = string.IsNullOrWhiteSpace(state.Error) ? string.Empty : state.Error;
        MaintenanceBannerText = state.Status == "failed"
            ? _displayNames.Format(
                "ui.maintenance.banner_failed",
                new Dictionary<string, string>
                {
                    ["kind"] = kind,
                    ["phase"] = phase,
                    ["error"] = error,
                })
            : _displayNames.Format(
                "ui.maintenance.banner_active",
                new Dictionary<string, string>
                {
                    ["kind"] = kind,
                    ["phase"] = phase,
                });
        NotificationText = MaintenanceBannerText;
    }

    private void ClearMaintenanceBanner()
    {
        IsMaintenanceBlocking = false;
        MaintenanceBannerText = string.Empty;
    }

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
                OnPropertyChanged(nameof(SidebarCollapsed));
                foreach (var nav in AllNavigationItems())
                {
                    nav.SidebarExpanded = value;
                }
            }
        }
    }

    public bool SidebarCollapsed => !SidebarExpanded;

    public double SidebarWidth => SidebarExpanded ? 204 : 52;

    public object CurrentPage
    {
        get => _currentPage;
        private set => SetProperty(ref _currentPage, value);
    }

    public RelayCommand ToggleSidebarCommand { get; }

    public RelayCommand ToggleDiagnosticCommand { get; }

    public RelayCommand ClearDiagnosticCommand { get; }

    public RelayCommand OpenVersionCommand => new(() => _ = ShowVersionAsync());

    public RelayCommand OpenFeedbackCommand => new(() => _ = ShowFeedbackAsync());

    public RelayCommand CreateProjectCommand => new(() => _ = RunWelcomeCommandAfterLeaveGuardAsync(Welcome.CreateProjectCommand));

    public RelayCommand OpenProjectCommand => new(() => _ = RunWelcomeCommandAfterLeaveGuardAsync(Welcome.OpenProjectCommand));

    /// <summary>已进入工作台时打开/切换项目（与 Open 同源）。</summary>
    public RelayCommand SwitchProjectCommand { get; }

    public RelayCommand LeaveProjectCommand => new(() => _ = LeaveProjectAsync());

    public void Observe(UserFailure failure)
    {
        var detail = failure.RedactedDiagnostic;
        if (string.IsNullOrWhiteSpace(detail))
        {
            return;
        }

        DiagnosticSummaryText = failure.PrimaryText(_displayNames);
        DiagnosticDetailText = detail;
        _diagnosticFailure = failure;
        HasDiagnostic = true;
        IsDiagnosticExpanded = false;
    }

    private void ClearDiagnostic()
    {
        IsDiagnosticExpanded = false;
        HasDiagnostic = false;
        DiagnosticSummaryText = string.Empty;
        DiagnosticDetailText = string.Empty;
        _diagnosticFailure = null;
    }

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
            await TryRestoreLastNavWithoutProjectAsync().ConfigureAwait(true);
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
            await EnterProjectAsync(status.CurrentProject).ConfigureAwait(true);
            await RefreshSidebarBadgesAsync(status.Badges).ConfigureAwait(true);
        }
        else
        {
            HasOpenProject = false;
            BackendStatus = _displayNames.Text("ui.status.healthy");
            // 上次侧栏跳过开始页：恢复到暂存的导航页
            await TryRestoreLastNavWithoutProjectAsync().ConfigureAwait(true);
        }
    }

    private async Task EnterProjectAsync(CurrentProjectStatus project)
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
        // 页面会话按项目隔离；旧实例先失效，再创建当前项目的唯一实例。
        ResetProjectPageSession();
        BackendStatus = _displayNames.Text("ui.status.healthy");
        NotificationText = string.Empty;
        await RefreshBudgetStatusAsync().ConfigureAwait(true);
        await RefreshMaintenanceStatusAsync().ConfigureAwait(true);

        var targetId = !string.IsNullOrWhiteSpace(_lastNavId) && AlwaysAvailablePageIds.Contains(_lastNavId)
            ? _lastNavId!
            : "workspace";
        var target = AllNavigationItems().FirstOrDefault(n => n.Id == targetId)
                     ?? PrimaryNavigationItems[0];
        var pageSessionGeneration = _projectPageSessionGeneration;
        await SelectNavigationItemForProjectAsync(target).ConfigureAwait(true);
        if (pageSessionGeneration != _projectPageSessionGeneration)
        {
            return;
        }
        await LoadProjectDataPagesAsync(pageSessionGeneration).ConfigureAwait(true);
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
        ResetProjectPageSession();
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
        var result = await DialogService.Current
            .ConfirmAsync(HelpDialogFactory.CreateFeedbackDialog(_displayNames))
            .ConfigureAwait(true);
        if (result == 1 && !ExternalLinkOpener.TryOpen(HelpDialogFactory.FeedbackIssueUrl))
        {
            NotificationText = _displayNames.Text("ui.feedback.open_failed");
        }
    }

    private NavigationItemViewModel CreateNav(string id, string key, Avalonia.Media.Geometry? icon)
    {
        return new NavigationItemViewModel(
            id,
            _displayNames.Text(key),
            icon,
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
        OnPropertyChanged(nameof(DiagnosticTitleText));
        OnPropertyChanged(nameof(DiagnosticToggleText));
        OnPropertyChanged(nameof(DiagnosticClearText));
        if (HasDiagnostic && _diagnosticFailure is { } failure)
        {
            DiagnosticSummaryText = failure.PrimaryText(_displayNames);
        }
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

    private object GetOrCreatePage(string id)
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

    internal async Task OpenNavigationItemByIdAsync(string id)
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
            var page = GetOrCreatePage(item.Id);
            CurrentPage = page;
            await EnsurePageLoadedAsync(item.Id, page).ConfigureAwait(true);
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
    private async Task TryRestoreLastNavWithoutProjectAsync()
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
            var page = GetOrCreatePage(item.Id);
            CurrentPage = page;
            await EnsurePageLoadedAsync(item.Id, page).ConfigureAwait(true);
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

    /// <summary>
    /// 离开项目/切换/回档前：只读收集全部 dirty 页，一次确认后统一保存或丢弃（U65）。
    /// 任一步保存失败则中止并保持当前项目，不再边问边改。
    /// </summary>
    private async Task<bool> ConfirmCachedProjectPagesLeaveAsync()
    {
        var dirty = _pageCache.Values
            .OfType<IUnsavedChangesGuard>()
            .Where(g => g.HasUnsavedChanges)
            .ToList();
        if (dirty.Count == 0)
        {
            return true;
        }

        var titles = dirty.Select(g => g.UnsavedChangesPageTitle).ToList();
        var choice = await DialogService.Current.ConfirmUnsavedLeaveManyAsync(titles).ConfigureAwait(true);
        switch (choice)
        {
            case UnsavedLeaveChoice.Save:
            {
                // U65: prepare all (no durable write) → journaled commit each page.
                var pages = dirty
                    .Select(g => (
                        Title: g.UnsavedChangesPageTitle,
                        Prepare: (Func<Task<bool>>)(() => g.PrepareUnsavedChangesAsync()),
                        Commit: (Func<Task<bool>>)(() => g.CommitPreparedUnsavedChangesAsync())))
                    .ToList();
                var journalPath = Path.Combine(
                    Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                    "Ariadne",
                    "leave-save.journal.json");
                var result = await BatchLeaveSaveCoordinator.ExecuteAsync(pages, journalPath).ConfigureAwait(true);
                if (result.AllSucceeded)
                {
                    return true;
                }

                if (result.CommittedPages.Count > 0)
                {
                    NotificationText = _displayNames.Format(
                        "ui.dialog.unsaved.save_partial",
                        new Dictionary<string, string>
                        {
                            ["page"] = result.FailedPage ?? "?",
                            ["done"] = string.Join("、", result.CommittedPages),
                        });
                }
                else
                {
                    NotificationText = _displayNames.Format(
                        "ui.dialog.unsaved.save_failed",
                        new Dictionary<string, string> { ["page"] = result.FailedPage ?? "?" });
                }

                return false;
            }
            case UnsavedLeaveChoice.Discard:
                foreach (var guard in dirty)
                {
                    await guard.DiscardUnsavedChangesAsync().ConfigureAwait(true);
                }
                return true;
            default:
                return false;
        }
    }

    private async Task ReloadCachedProjectPagesAsync()
    {
        foreach (var (id, page) in _pageCache.ToArray())
        {
            if (page is IProjectDataReloadable)
            {
                await ReloadPageAsync(id, page).ConfigureAwait(true);
            }
        }
        await RefreshBudgetStatusAsync().ConfigureAwait(true);
        await RefreshSidebarBadgesAsync(new SidebarBadgeCounts(0, 0, 0)).ConfigureAwait(true);
    }

    private async Task LoadProjectDataPagesAsync(long? expectedGeneration = null)
    {
        var generation = expectedGeneration ?? _projectPageSessionGeneration;
        foreach (var id in PreloadedProjectPageIds)
        {
            if (generation != _projectPageSessionGeneration)
            {
                return;
            }
            var page = GetOrCreatePage(id);
            await EnsurePageLoadedAsync(id, page).ConfigureAwait(true);
        }
        if (generation != _projectPageSessionGeneration)
        {
            return;
        }
        await RefreshSidebarBadgesAsync(new SidebarBadgeCounts(0, 0, 0)).ConfigureAwait(true);
    }

    private async Task EnsurePageLoadedAsync(string id, object page)
    {
        if (page is not IProjectDataReloadable reloadable || _loadedPageIds.Contains(id))
        {
            return;
        }

        if (_pageLoadTasks.TryGetValue(id, out var pending))
        {
            await pending.ConfigureAwait(true);
            return;
        }

        var generation = _projectPageSessionGeneration;
        var cancellationToken = _projectPageSessionCts.Token;
        var loadTask = LoadPageForSessionAsync(id, page, reloadable, generation, cancellationToken);
        _pageLoadTasks[id] = loadTask;
        try
        {
            await loadTask.ConfigureAwait(true);
        }
        finally
        {
            if (generation == _projectPageSessionGeneration
                && _pageLoadTasks.TryGetValue(id, out var current)
                && ReferenceEquals(current, loadTask))
            {
                _pageLoadTasks.Remove(id);
            }
        }
    }

    private async Task LoadPageForSessionAsync(
        string id,
        object page,
        IProjectDataReloadable reloadable,
        long generation,
        CancellationToken cancellationToken)
    {
        try
        {
            await reloadable.ReloadProjectDataAsync(cancellationToken).ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            return;
        }

        if (!cancellationToken.IsCancellationRequested
            && generation == _projectPageSessionGeneration
            && _pageCache.TryGetValue(id, out var current)
            && ReferenceEquals(current, page))
        {
            _loadedPageIds.Add(id);
        }
    }

    private async Task ReloadPageAsync(string id, object page)
    {
        if (_pageLoadTasks.TryGetValue(id, out var pending))
        {
            await pending.ConfigureAwait(true);
        }
        if (!_pageCache.TryGetValue(id, out var current) || !ReferenceEquals(current, page))
        {
            return;
        }

        _loadedPageIds.Remove(id);
        await EnsurePageLoadedAsync(id, page).ConfigureAwait(true);
    }

    private void ResetProjectPageSession()
    {
        _projectPageSessionGeneration++;
        _projectPageSessionCts.Cancel();
        _projectPageSessionCts.Dispose();
        _projectPageSessionCts = new CancellationTokenSource();
        _loadedPageIds.Clear();
        _pageLoadTasks.Clear();
        foreach (var id in ProjectScopedPageIds)
        {
            if (_pageCache.TryGetValue(id, out var page)
                && page is IProjectDataReloadable reloadable)
            {
                reloadable.DeactivateProjectData();
            }
            _pageCache.Remove(id);
        }
    }

    internal object GetPageForTests(string id) => GetOrCreatePage(id);

    internal Task PreloadProjectPagesForTestsAsync() => LoadProjectDataPagesAsync();

    internal void ResetProjectPageSessionForTests() => ResetProjectPageSession();

    private async Task SelectNavigationItemForProjectAsync(NavigationItemViewModel item)
    {
        foreach (var nav in AllNavigationItems())
        {
            nav.IsSelected = nav == item;
        }
        var page = GetOrCreatePage(item.Id);
        CurrentPage = page;
        await EnsurePageLoadedAsync(item.Id, page).ConfigureAwait(true);
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
            BudgetStatusText = UserFacingError.Short(ex, _displayNames, "ui.error.budget");
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
