using System.Collections.ObjectModel;
using System.Reflection;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class MainWindowViewModel : ViewModelBase, IUserFailureObserver, IRunTerminalStateObserver
{
    private static readonly string AppVersion =
        typeof(MainWindowViewModel).Assembly
            .GetCustomAttribute<AssemblyInformationalVersionAttribute>()?
            .InformationalVersion
        ?? typeof(MainWindowViewModel).Assembly.GetName().Version?.ToString(3)
        ?? "0.0.0";
    private static readonly string[] ProjectSessionPageIds = { "workspace", "works", "git", "run_logs", "templates", "settings" };
    private static readonly HashSet<string> RetainedGlobalPageIds = new(StringComparer.Ordinal)
    {
        "settings",
        "templates",
    };
    private static readonly string[] PreloadedProjectPageIds = { "workspace", "works", "git" };
    /// <summary>
    /// error 档阈值：余量低于 $2 即报错色。
    /// 来自 `指导性文件/UI组件状态表.md:34`，**是绝对金额而不是百分比**——
    /// 它衡量的是「还够不够跑一次调用」，与总额大小无关。改这个数要同步改那份规范。
    /// </summary>
    private const double BudgetErrorRemainingUsd = 2.0;
    /// <summary>
    /// warning 档阈值：已花达 80% 起预告。规范没定这一档，是本轮补的。
    /// 取 80% 而非更早，是为了不让预告变成常态噪点——一条整天黄着的进度条
    /// 和一条整天绿着的进度条一样没有信息量。
    /// </summary>
    private const double BudgetWarningSpentRatio = 0.8;
    /// <summary>无项目时也可进入的页面（侧栏跳过开始页）。</summary>
    private static readonly HashSet<string> AlwaysAvailablePageIds = new(StringComparer.Ordinal)
    {
        "workspace", "works", "git", "run_logs", "templates", "settings",
    };

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private readonly Func<string, object?>? _pageFactory;
    private readonly Action<string> _saveLastNavigationId;
    private readonly ProjectAutomationState _projectAutomation;
    private readonly SemaphoreSlim _uiPreferencesSaveGate = new(1, 1);
    private readonly object _uiPreferencesSync = new();
    private UiPreferences? _globalPreferences;
    private object _currentPage;
    private string _projectTitle;
    private string _backendStatus;
    private string _notificationText = string.Empty;
    private string _budgetStatusText;
    private double _budgetUsagePercent;
    private BudgetSeverity _budgetSeverity = BudgetSeverity.Normal;
    private double? _budgetRemainingUsd;
    /// <summary>已因终态刷新过预算的运行身份，避免同一次运行重复发 IPC。</summary>
    private string? _lastRefreshedTerminalRun;
    private bool _sidebarExpanded = true;
    private bool _hasOpenProject;
    private string _currentProjectRoot = string.Empty;
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
    private readonly RequestGenerationSession _navigationSession = new();
    private bool _isProjectTransitionRunning;

    public MainWindowViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
        : this(displayNames, backend, null, SessionNavStore.SaveLastNavId)
    {
    }

    internal MainWindowViewModel(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        Func<string, object?>? pageFactory,
        Action<string>? saveLastNavigationId = null)
    {
        _displayNames = displayNames;
        _backend = backend;
        _projectAutomation = new ProjectAutomationState(displayNames, backend);
        _pageFactory = pageFactory;
        _saveLastNavigationId = saveLastNavigationId ?? SessionNavStore.SaveLastNavId;
        Welcome = new WelcomeViewModel(displayNames, backend, EnterProjectAsync);
        _currentPage = Welcome;
        _projectTitle = displayNames.Text("ui.window.no_project_title");
        _backendStatus = displayNames.Text("ui.status.unavailable");
        _budgetStatusText = displayNames.Text("ui.common.none");
        ProjectMenuItems = new ObservableCollection<ProjectMenuItemViewModel>();
        ToggleSidebarCommand = new RelayCommand(() => SidebarExpanded = !SidebarExpanded);
        ToggleDiagnosticCommand = new RelayCommand(() => IsDiagnosticExpanded = !IsDiagnosticExpanded);
        ClearDiagnosticCommand = new RelayCommand(ClearDiagnostic);
        OpenVersionCommand = new RelayCommand(() => _ = ShowVersionAsync());
        OpenFeedbackCommand = new RelayCommand(() => _ = ShowFeedbackAsync());
        CreateProjectCommand = new RelayCommand(
            () => _ = RunWelcomeCommandAfterLeaveGuardAsync(Welcome.CreateProjectAsync),
            CanStartProjectTransition);
        OpenProjectCommand = new RelayCommand(
            () => _ = RunWelcomeCommandAfterLeaveGuardAsync(Welcome.OpenProjectAsync),
            CanStartProjectTransition);
        LeaveProjectCommand = new RelayCommand(() => _ = LeaveProjectAsync());
        UserFacingError.RegisterObserver(this);
        // U194-E：运行跑完后顶栏预算数字必须跟着动。
        // 注册在构造期而不是「进项目时」：运行会话属于画布页，其生命周期与项目会话
        // 不同步（切页会重建页面 VM，但顶栏这一个实例活到窗口关闭）。
        RunTerminalStateNotifier.Register(this);
        // 上组：创作主流程
        PrimaryNavigationItems = new ObservableCollection<NavigationItemViewModel>
        {
            CreateNav("workspace", "ui.nav.workspace", IconGeometries.Workspace),
            CreateNav("works", "ui.nav.works", IconGeometries.Works),
            CreateNav("git", "ui.nav.git", IconGeometries.Git),
            CreateNav("run_logs", "ui.nav.run_logs", IconGeometries.RunLog),
        };

        // 下组：项目扩展与应用配置入口。设置与其它页面共用同一导航状态源。
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

    public string LeaveProjectText => _displayNames.Text("ui.layout.leave_project");

    public string DiagnosticTitleText => _displayNames.Text("ui.diagnostics.title");

    public string DiagnosticToggleText => _displayNames.Text(
        IsDiagnosticExpanded ? "ui.diagnostics.hide" : "ui.diagnostics.show");

    public string DiagnosticClearText => _displayNames.Text("ui.diagnostics.clear");

    /// <summary>
    /// U205-C：「清除」的悬停说明。
    ///
    /// 这个键存在的理由不是「补全文案」，而是**清除是这条路上唯一不可逆的动作**：
    /// <see cref="ClearDiagnostic"/> 把 summary / detail / failure 三份一起清空，
    /// 而诊断横幅是失败信息的兜底显示位（U194-B）⇒ 清掉之后「刚才报了什么错」
    /// 在界面上再也查不回来。它原先与「展开详情」是两个同款 subtle 键、等距 8px、
    /// 零视觉区分，所以先分组，再由这句文案说明代价。
    /// </summary>
    public string DiagnosticClearHintText => _displayNames.Text("ui.diagnostics.clear.hint");

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

    public string? FeedbackToolTipText => SidebarCollapsed ? FeedbackText : null;

    public string VersionText => _displayNames.Format("ui.layout.version_value", new Dictionary<string, string>
    {
        ["version"] = AppVersion,
    });

    public string? VersionToolTipText => SidebarCollapsed ? VersionText : null;

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
        set
        {
            if (SetProperty(ref _budgetStatusText, value))
            {
                OnPropertyChanged(nameof(BudgetSeverityToolTipText));
            }
        }
    }

    public double BudgetUsagePercent
    {
        get => _budgetUsagePercent;
        set => SetProperty(ref _budgetUsagePercent, value);
    }

    /// <summary>
    /// 预算余量分档，驱动顶栏进度条填充色与金额文字色（U194-E）。
    ///
    /// 三个布尔投影是给 XAML 用的：`Classes.budget-error="{Binding IsBudgetError}"`
    /// 比在样式选择器里比较枚举值可靠——Avalonia 没有「按 DataContext 属性值匹配」的
    /// 选择器，条件类是唯一不需要 code-behind 的路子。
    /// </summary>
    public BudgetSeverity BudgetSeverity
    {
        get => _budgetSeverity;
        private set
        {
            if (SetProperty(ref _budgetSeverity, value))
            {
                OnPropertyChanged(nameof(IsBudgetNormal));
                OnPropertyChanged(nameof(IsBudgetWarning));
                OnPropertyChanged(nameof(IsBudgetError));
                OnPropertyChanged(nameof(BudgetSeverityToolTipText));
            }
        }
    }

    public bool IsBudgetNormal => BudgetSeverity == BudgetSeverity.Normal;

    public bool IsBudgetWarning => BudgetSeverity == BudgetSeverity.Warning;

    public bool IsBudgetError => BudgetSeverity == BudgetSeverity.Error;

    /// <summary>
    /// 余量美元数；未设限额时为 null（无「余量」这个概念）。
    /// 顶栏不直接显示它——显示的仍是既有的「已花/总额」——它的用途是
    /// 悬停提示与分档判据的可测面。
    /// </summary>
    public double? BudgetRemainingUsd
    {
        get => _budgetRemainingUsd;
        private set
        {
            if (SetProperty(ref _budgetRemainingUsd, value))
            {
                OnPropertyChanged(nameof(BudgetSeverityToolTipText));
            }
        }
    }

    /// <summary>
    /// 悬停时说明「为什么这条变红了」。
    /// 颜色本身不解释原因，而「余量不足会被拒」这件事必须配文字
    /// （AGENTS.md：错误必须配文字，不能只靠一个颜色）。
    /// </summary>
    public string BudgetSeverityToolTipText
    {
        get
        {
            if (BudgetSeverity == BudgetSeverity.Normal || BudgetRemainingUsd is not { } remaining)
            {
                return BudgetStatusText;
            }
            var key = BudgetSeverity == BudgetSeverity.Error
                ? "ui.layout.budget_remaining_critical"
                : "ui.layout.budget_remaining_low";
            return _displayNames.Format(key, new Dictionary<string, string>
            {
                ["remaining"] = Math.Max(remaining, 0).ToString("0.##"),
            });
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
                OnPropertyChanged(nameof(SidebarCollapsed));
                OnPropertyChanged(nameof(FeedbackToolTipText));
                OnPropertyChanged(nameof(VersionToolTipText));
                foreach (var nav in AllNavigationItems())
                {
                    nav.SidebarExpanded = value;
                }
            }
        }
    }

    public bool SidebarCollapsed => !SidebarExpanded;

    public double SidebarWidth => SidebarExpanded ? 224 : 64;

    public object CurrentPage
    {
        get => _currentPage;
        private set => SetProperty(ref _currentPage, value);
    }

    public RelayCommand ToggleSidebarCommand { get; }

    public RelayCommand ToggleDiagnosticCommand { get; }

    public RelayCommand ClearDiagnosticCommand { get; }

    public RelayCommand OpenVersionCommand { get; }

    public RelayCommand OpenFeedbackCommand { get; }

    public RelayCommand CreateProjectCommand { get; }

    public RelayCommand OpenProjectCommand { get; }

    public RelayCommand LeaveProjectCommand { get; }

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

    /// <summary>
    /// 运行到达终态（succeeded / failed / stopped）后静默刷新顶栏预算（U194-E 后半）。
    ///
    /// 缺陷版本：`RefreshBudgetStatusAsync` 只在「进项目」与「Git 回档后」被调，
    /// 于是作者跑一整天工作流，顶栏那两个数字**一动不动**——花的钱全在后端账上，
    /// 界面上却看不出任何变化，直到下一次重开项目。
    ///
    /// **只刷新、不弹窗**：作者可能正在作品页打字，抢焦点是倒退
    /// （U194-D 已留档「后台事件不弹窗」是健康的，不要破坏它）。
    /// toast 组件是另一条线的活，这里刻意不碰。
    ///
    /// **无项目时直接返回**：离开项目后仍可能收到迟到的终态（页面 VM 尚未被回收），
    /// 那时预算查询无意义，还会把刚归位的顶栏重新填上上个项目的数字。
    /// </summary>
    void IRunTerminalStateObserver.OnRunReachedTerminalState(
        string workflowId,
        string runId,
        string status)
    {
        if (!HasOpenProject)
        {
            return;
        }
        // 同一次运行（workflow+run 二元组）只刷一次。跃迁边沿判据已在协调器一侧，
        // 这里再加一道是因为**画布页可以被重建**：新协调器从空状态挂接到同一个 runId
        // 时会再走一次「非终态 → 终态」，那属于同一次运行，不该再发一次 IPC。
        var identity = $"{workflowId}{runId}{status}";
        if (string.Equals(_lastRefreshedTerminalRun, identity, StringComparison.Ordinal))
        {
            return;
        }
        _lastRefreshedTerminalRun = identity;
        _ = RefreshBudgetStatusAsync();
        // U198-B「顺带」：终态也要刷侧栏角标。
        //
        // 原先 `RefreshSidebarBadgesAsync` 的三个调用点全在**进项目 / 离开项目**上，
        // **运行链路零调用**（与 U197-A 同根）⇒ 一次运行跑完新产生的待审确认项、
        // 新的运行记录、新的诊断，角标上一个都不涨，除非作者恰好切走再切回来。
        // 而运行终态正是这三个数字唯一会同时变动的时刻。
        //
        // 复用上面那道 identity 去重（同一次运行只刷一次）：这两件事的触发条件
        // 与「只刷一次」的理由完全相同，各自再记一个 last-refreshed 只会让
        // 两份状态有机会漂移。
        // fallback 传当前值而非 0：这里的 0 会把角标**清空**，
        // 而一次 IPC 抖动不该让作者以为待审队列空了——读不到就维持原样。
        _ = RefreshSidebarBadgesAsync(CurrentSidebarBadges());
    }

    /// <summary>
    /// 侧栏角标的当前值，用作刷新失败时的 fallback（「维持原样」而不是「清零」）。
    /// </summary>
    private SidebarBadgeCounts CurrentSidebarBadges() => new(
        BadgeOf("workspace"),
        BadgeOf("run_logs"),
        BadgeOf("settings"));

    private int BadgeOf(string id) =>
        AllNavigationItems().FirstOrDefault(item => string.Equals(item.Id, id, StringComparison.Ordinal))
            ?.BadgeCount ?? 0;

    public bool HasOpenProject
    {
        get => _hasOpenProject;
        private set => SetProperty(ref _hasOpenProject, value);
    }

    public async Task InitializeAsync()
    {
        var interruptedLeave = BatchLeaveSaveCoordinator.ReadJournal(
            BatchLeaveSaveCoordinator.DefaultJournalPath);
        try
        {
            await InitializeCoreAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            HasOpenProject = false;
            BackendStatus = _displayNames.Text("ui.status.unavailable");
            // 初始化边界直接提交到当前窗口；UserFacingError 的普通调用按异步上下文隔离观察者。
            var failure = UserFacingError.FromException(ex);
            Observe(failure);
            NotificationText = failure.PrimaryText(_displayNames);
            ResetProjectPageSession();
            CurrentPage = Welcome;
        }

        ApplyInterruptedLeaveJournal(interruptedLeave);
    }

    private async Task InitializeCoreAsync()
    {
        _lastNavId = SessionNavStore.LoadLastNavId();
        var status = await _backend.GetAppStatusAsync().ConfigureAwait(true);
        if (status is not null)
        {
            ApplyGlobalPreferences(status.Preferences);
        }
        await Welcome.LoadAsync().ConfigureAwait(true);
        RefreshProjectMenuItems();
        if (status is null)
        {
            BackendStatus = _displayNames.Text("ui.status.unavailable");
            // 无后端时仍允许侧栏进入空页面
            await TryRestoreLastNavWithoutProjectAsync().ConfigureAwait(true);
            return;
        }

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
            await _backend.SetProjectRootAsync(project.ProjectRoot).ConfigureAwait(true);
        }

        _currentProjectRoot = project.ProjectRoot;

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

    private async Task LeaveProjectAsync()
    {
        if (!await ConfirmCachedProjectPagesLeaveAsync().ConfigureAwait(true))
        {
            return;
        }

        try
        {
            await _backend.CloseProjectAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            NotificationText = UserFacingError.Format(ex, _displayNames);
            return;
        }
        HasOpenProject = false;
        _currentProjectRoot = string.Empty;
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
        // 离开项目后预算不再属于任何项目，分档必须一并归位——
        // 否则上个项目的告急红会留在顶栏，看起来像是新的（空的）会话在超支。
        BudgetSeverity = BudgetSeverity.Normal;
        BudgetRemainingUsd = null;
        // 离开项目回到开始页；侧栏暂存的 nav 仍保留，便于再次点侧栏进入
        CurrentPage = Welcome;
        await Welcome.LoadAsync().ConfigureAwait(true);
        RefreshProjectMenuItems();
    }

    private async Task RunWelcomeCommandAfterLeaveGuardAsync(Func<Task> action)
    {
        if (!CanStartProjectTransition())
        {
            return;
        }

        SetProjectTransitionRunning(true);
        try
        {
            if (!await ConfirmCachedProjectPagesLeaveAsync().ConfigureAwait(true))
            {
                return;
            }

            await action().ConfigureAwait(true);
        }
        finally
        {
            SetProjectTransitionRunning(false);
        }
    }

    private bool CanStartProjectTransition() => !_isProjectTransitionRunning;

    private void SetProjectTransitionRunning(bool value)
    {
        if (_isProjectTransitionRunning == value)
        {
            return;
        }

        _isProjectTransitionRunning = value;
        CreateProjectCommand.NotifyCanExecuteChanged();
        OpenProjectCommand.NotifyCanExecuteChanged();
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
        OnPropertyChanged(nameof(LeaveProjectText));
        OnPropertyChanged(nameof(DiagnosticTitleText));
        OnPropertyChanged(nameof(DiagnosticToggleText));
        OnPropertyChanged(nameof(DiagnosticClearText));
        // 切语言时这句提示必须一起刷：漏一个 nameof 的后果是「界面上大部分文案变了、
        // 悬停说明还是上一门语言」——项目记忆里那条「基元的适用前提不会跟着基元走」
        // 说的正是这类漏项，它不会报错、只会让一处文案卡在旧语言上。
        OnPropertyChanged(nameof(DiagnosticClearHintText));
        if (HasDiagnostic && _diagnosticFailure is { } failure)
        {
            DiagnosticSummaryText = failure.PrimaryText(_displayNames);
        }
        OnPropertyChanged(nameof(FeedbackText));
        OnPropertyChanged(nameof(FeedbackToolTipText));
        OnPropertyChanged(nameof(VersionText));
        OnPropertyChanged(nameof(VersionToolTipText));
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
        foreach (var page in _pageCache.Values.OfType<ILocalizedUiAware>())
        {
            page.RefreshLocalizedUi();
        }
    }

    private object GetOrCreatePage(string id)
    {
        if (_pageCache.TryGetValue(id, out var cached))
        {
            return cached;
        }
        object page = _pageFactory?.Invoke(id) ?? id switch
        {
            "workspace" => new WorkspacePageViewModel(
                _displayNames,
                _backend,
                PersistPanelStateAsync,
                _projectAutomation),
            "works" => new WorksPageViewModel(
                _displayNames,
                _backend,
                PersistPanelStateAsync,
                _projectAutomation),
            "git" => new GitPageViewModel(
                _displayNames,
                _backend,
                ConfirmCachedProjectPagesLeaveAsync,
                ReloadCachedProjectPagesAsync,
                PersistPanelStateAsync,
                // U182-M：Git 页「没打开项目」空态里那颗按钮走的是**这条链**
                // （与标题栏的 OpenProjectCommand 同一个），不是另写一套后端调用：
                // 离开守卫、目录预检、EnterProject、最近项目登记全在这条链上。
                () => RunWelcomeCommandAfterLeaveGuardAsync(Welcome.OpenProjectAsync),
                // U197-H：Git 页要能说出「另有页面存在未保存改动」。
                // `HasCachedUnsavedChanges` 此前**只被关窗守卫消费**
                // （`MainWindow.axaml.cs:275`），界面上从不显示 ——
                // 而它正是为「磁盘干净≠东西已入库」这件事而生的。
                () => HasCachedUnsavedChanges),
            "run_logs" => new RunLogPageViewModel(_displayNames, _backend),
            // U207-D/U198-A：装完模板后后端已把模板图并进项目画布（`install_template`
            // 走 `merge_workflow_into_project_canvas` + `save_workflow_graph_locked`），
            // 但前端画布页手里还是装模板之前那份图 ⇒ 切回去看到的是空画布 + 空态引导，
            // 作者会判定「导入失败」再点一次。这里补的就是那条通知链。
            // 排除 "templates" 自己，理由见 ReloadCachedProjectPagesExceptAsync 注释。
            "templates" => new TemplateMarketPageViewModel(
                _displayNames,
                _backend,
                () => ReloadCachedProjectPagesExceptAsync("templates"),
                ConfirmCachedProjectPagesLeaveAsync),
            "settings" => new SettingsPageViewModel(
                _displayNames,
                _backend,
                () => OpenNavigationItemByIdAsync("templates"),
                SaveGlobalPreferencesAsync,
                _projectAutomation),
            _ => Welcome,
        };
        if (page is IUiPreferencesAware preferencesAware && _globalPreferences is not null)
        {
            preferencesAware.ApplyUiPreferences(_globalPreferences);
        }
        _pageCache[id] = page;
        return page;
    }

    private void ApplyGlobalPreferences(UiPreferences preferences)
    {
        lock (_uiPreferencesSync)
        {
            _globalPreferences = preferences;
        }
        ProjectGlobalPreferences(preferences);
    }

    private void ProjectGlobalPreferences(UiPreferences preferences)
    {
        var language = _displayNames.NormalizeAvailableLanguage(preferences.Locale);
        if (!string.Equals(_displayNames.CurrentLanguage, language, StringComparison.OrdinalIgnoreCase))
        {
            _displayNames.SwitchLanguage(language);
        }
        ThemeApplication.Apply(
            preferences.Theme,
            preferences.ThemeMainColor,
            preferences.ThemeSurfaceColor,
            preferences.ThemeBrandColor,
            preferences.ThemeMainColorDark,
            preferences.ThemeSurfaceColorDark,
            preferences.ThemeBrandColorDark,
            preferences.ThemeFollowSystemColors);
        MotionPreferences.Apply(preferences.ReduceMotion);
        foreach (var page in _pageCache.Values.OfType<IUiPreferencesAware>())
        {
            page.ApplyUiPreferences(preferences);
        }
    }

    private async Task SaveGlobalPreferencesAsync(UiPreferences preferences)
    {
        await _uiPreferencesSaveGate.WaitAsync().ConfigureAwait(true);
        try
        {
            UiPreferences? current;
            lock (_uiPreferencesSync)
            {
                current = _globalPreferences;
            }
            current ??= await _backend.GetUiPreferencesAsync().ConfigureAwait(true);
            // 设置页不拥有这些运行中元数据。整对象保存时保留协调器中的最新值，
            // 防止在途面板切换或新手引导状态被较早构造的表单快照覆盖。
            var merged = preferences with
            {
                ProjectPanelPosition = current.ProjectPanelPosition,
                PanelStates = new Dictionary<string, bool>(
                    current.PanelStates ?? new Dictionary<string, bool>(),
                    StringComparer.Ordinal),
                OnboardingSeen = current.OnboardingSeen,
            };
            await _backend.SaveUiPreferencesAsync(merged).ConfigureAwait(true);
            // 保存期间页面仍可产生 panel patch。成功提交只合并设置页拥有的字段，
            // 运行元数据继续采用当前内存权威值，再统一投影一次。
            UiPreferences committed;
            lock (_uiPreferencesSync)
            {
                var latest = _globalPreferences ?? current;
                committed = merged with
                {
                    ProjectPanelPosition = latest.ProjectPanelPosition,
                    PanelStates = new Dictionary<string, bool>(
                        latest.PanelStates ?? new Dictionary<string, bool>(),
                        StringComparer.Ordinal),
                    OnboardingSeen = latest.OnboardingSeen,
                };
                _globalPreferences = committed;
            }
            ProjectGlobalPreferences(committed);
        }
        finally
        {
            _uiPreferencesSaveGate.Release();
        }
    }

    private async Task PersistPanelStateAsync(string key, bool isOpen)
    {
        UiPreferences? current;
        lock (_uiPreferencesSync)
        {
            current = _globalPreferences;
        }
        if (current is null)
        {
            current = await _backend.GetUiPreferencesAsync().ConfigureAwait(true);
            ApplyGlobalPreferences(current);
        }

        UiPreferences intended;
        lock (_uiPreferencesSync)
        {
            current = _globalPreferences ?? current;
            var panelStates = new Dictionary<string, bool>(
                current.PanelStates ?? new Dictionary<string, bool>(),
                StringComparer.Ordinal)
            {
                [key] = isOpen,
            };
            intended = current with { PanelStates = panelStates };
            // 用户意图先成为内存权威事实；旧 I/O 完成不得再回写旧快照。
            _globalPreferences = intended;
        }
        ProjectGlobalPreferences(intended);

        await _uiPreferencesSaveGate.WaitAsync().ConfigureAwait(true);
        try
        {
            UiPreferences latest;
            lock (_uiPreferencesSync)
            {
                latest = _globalPreferences ?? intended;
            }
            await _backend.SaveUiPreferencesAsync(latest).ConfigureAwait(true);
        }
        finally
        {
            _uiPreferencesSaveGate.Release();
        }
    }

    internal async Task OpenNavigationItemByIdAsync(string id)
    {
        var item = AllNavigationItems().FirstOrDefault(nav => nav.Id == id);
        if (item is not null)
        {
            await SelectNavigationItemAsync(item).ConfigureAwait(true);
        }
    }

    /// <summary>仅供 ARIADNE_UI_START_PAGE 视觉验收入口使用，不触发页面数据加载。</summary>
    internal void OpenPreviewNavigationItem(string id)
    {
        var item = AllNavigationItems().FirstOrDefault(nav => nav.Id == id);
        if (item is not null)
        {
            CommitNavigation(item, GetOrCreatePage(item.Id), persist: false);
        }
    }

    private async Task SelectNavigationItemAsync(NavigationItemViewModel item)
    {
        // 已在该页且不在 Welcome：忽略；在 Welcome 上点侧栏必须能切走
        if (item.IsSelected && !ReferenceEquals(CurrentPage, Welcome))
        {
            return;
        }

        var request = _navigationSession.Begin(_projectPageSessionGeneration);

        if (!await ConfirmCurrentPageLeaveAsync().ConfigureAwait(true))
        {
            return;
        }

        if (!AlwaysAvailablePageIds.Contains(item.Id)
            || !_navigationSession.IsCurrent(request, _projectPageSessionGeneration))
        {
            return;
        }

        // 从开始页点侧栏 = 进入工作台页（可无项目，显示空态）；记住导航以便下次恢复
        try
        {
            var page = GetOrCreatePage(item.Id);

            // U178-A：**先切页、后填数据**。此前这里是先 await 整套页面加载再 commit，
            // 于是点侧栏后屏幕在几百毫秒里完全静止——连被按下的那个导航项都不变样
            // （IsSelected 也在 CommitNavigation 里刷），用户的第一反应是「没点上」于是再点一次。
            // 配置页最贵：LoadAsync 一次并发发 14+ 次 IPC。
            //
            // **为什么 commit 能安全地提前**：CommitNavigation 只做四件**纯 UI 状态**的事——
            // 刷 IsSelected、设 CurrentPage、记 _lastNavId、持久化导航 id，
            // 它**不读任何页面数据**。页面 VM 由 GetOrCreatePage 构造完毕即是合法的空态
            // （各页的空态/骨架态本来就要能显示），所以提前挂上去拿不到半初始化状态。
            // 等待并没有消失，但用户不再面对静止的屏幕：导航项立刻高亮、页面立刻出现在
            // 骨架/loading 态（配置页的 ShowLoadingSkeleton），数据回来再填。
            //
            // ⚠️ 只有**落盘的**导航 id 仍然等加载成功（persist: false + 下面单独落盘）。
            // 「记住上次的页」记的是「上次成功进入的页」：若把加载失败的页也存下去，
            // 下次启动会去恢复一个打不开的页、再弹回开始页，一个坏页就变成粘性状态。
            CommitNavigation(item, page, persist: false);


            // 导航项上的「读取中」指示。它必须在 await 之前置真、且**无论走哪条出口**
            // 都要清掉（下面的 finally），否则一次失败的导航会留下永久转圈的侧栏项。
            item.IsPending = true;

            await EnsurePageLoadedAsync(item.Id, page).ConfigureAwait(true);

            // **为什么代际校验必须留**：它防的是快速连点两个页面时旧请求的迟到结果。
            // 提前 commit 之后这道闸的作用**变了但没有变弱**——它不再是「决定要不要切页」，
            // 而是「决定这条已过期的路径能不能继续往下写可见状态」（下面的标题、
            // BackendStatus，以及 catch 里的弹回开始页）。
            // 摘掉它就会出现「点了 B 又点 C，最后停在 B」：B 的迟到完成会把
            // C 已经提交的可见状态改回去。
            if (!_navigationSession.IsCurrent(request, _projectPageSessionGeneration))
            {
                return;
            }

            _saveLastNavigationId(item.Id);
        }
        catch (Exception ex)
        {
            if (!_navigationSession.IsCurrent(request, _projectPageSessionGeneration))
            {
                return;
            }
            // 回到开始页，但保留资源化错误摘要；工程诊断只进入统一诊断面板。
            // U163-A：弹回开始页是**最后手段**，必须留下是哪一类异常导致的。
            // 只写 NotificationText（本地化摘要）时，排查者看不出异常类型与来源页——
            // 这正是「作品页莫名跳回欢迎界面」当初难以定位的原因。
            Observe(UserFacingError.FromException(ex));
            NotificationText = UserFacingError.Format(ex, _displayNames);
            CurrentPage = Welcome;
            foreach (var nav in AllNavigationItems())
            {
                nav.IsSelected = false;
            }
            return;
        }
        finally
        {
            // 无论成功、过期返回还是异常，pending 指示都必须落地。
            // 放在 finally 而不是各条出口：出口有四个（正常、两处过期 return、catch），
            // 逐个补一遍就是在等着漏掉一个——漏掉的那条会留下一个永远在读的侧栏项。
            item.IsPending = false;
        }

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

    private void CommitNavigation(NavigationItemViewModel item, object page, bool persist)
    {
        foreach (var nav in AllNavigationItems())
        {
            nav.IsSelected = nav == item;
        }
        CurrentPage = page;
        _lastNavId = item.Id;
        if (persist)
        {
            _saveLastNavigationId(item.Id);
        }
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

        var request = _navigationSession.Begin(_projectPageSessionGeneration);
        try
        {
            var page = GetOrCreatePage(item.Id);
            await EnsurePageLoadedAsync(item.Id, page).ConfigureAwait(true);
            if (_navigationSession.IsCurrent(request, _projectPageSessionGeneration))
            {
                CommitNavigation(item, page, persist: false);
            }
        }
        catch
        {
            if (!_navigationSession.IsCurrent(request, _projectPageSessionGeneration))
            {
                return;
            }
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
                    .Select(g => new BatchLeaveSaveCoordinator.PageRequest(
                        PageId: g.UnsavedChangesPageId,
                        Title: g.UnsavedChangesPageTitle,
                        Prepare: () => g.PrepareUnsavedChangesAsync(),
                        Commit: () => g.CommitPreparedUnsavedChangesAsync(),
                        Abort: () => g.AbortPreparedUnsavedChangesAsync(),
                        ReadPayloadIdentity: () => g.PreparedUnsavedChangesPayloadIdentity))
                    .ToList();
                var result = await BatchLeaveSaveCoordinator.ExecuteAsync(
                    pages,
                    BatchLeaveSaveCoordinator.DefaultJournalPath,
                    _currentProjectRoot).ConfigureAwait(true);
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

    internal bool HasCachedUnsavedChanges => _pageCache.Values
        .OfType<IUnsavedChangesGuard>()
        .Any(guard => guard.HasUnsavedChanges);

    internal Task<bool> ConfirmCloseAsync() => ConfirmCachedProjectPagesLeaveAsync();

    private void ApplyInterruptedLeaveJournal(BatchLeaveSaveCoordinator.JournalState? journal)
    {
        if (journal is null)
        {
            return;
        }

        if (journal.Phase is "committing" or "partial")
        {
            var failedPage = journal.FailedPage
                ?? journal.PlannedPages.FirstOrDefault(page => !journal.CommittedPages.Contains(page))
                ?? "?";
            NotificationText = journal.CommittedPages.Count > 0
                ? _displayNames.Format(
                    "ui.dialog.unsaved.save_partial",
                    new Dictionary<string, string>
                    {
                        ["page"] = failedPage,
                        ["done"] = string.Join("、", journal.CommittedPages),
                    })
                : _displayNames.Format(
                    "ui.dialog.unsaved.save_failed",
                    new Dictionary<string, string> { ["page"] = failedPage });
        }

        BatchLeaveSaveCoordinator.ClearJournal(BatchLeaveSaveCoordinator.DefaultJournalPath);
    }

    private Task ReloadCachedProjectPagesAsync() => ReloadCachedProjectPagesExceptAsync(null);

    /// <summary>
    /// 重载所有已缓存的项目数据页，可排除**发起者自己**。
    ///
    /// U207-D/U198-A：模板页装完模板后要通知画布页重载，但它**不能把自己也重载一遍**。
    /// `TemplateMarketPageViewModel.ReloadProjectDataAsync` 走的是「换项目了」语义
    /// （`ResetCatalogCache`：清空检索结果 + 作废在途请求 + 清 StatusText），
    /// 在「装完模板」这个时刻触发会一次造成三处倒退：
    /// 目录列表被清空、「已导入模板：X」的成功文案被抹掉、并且
    /// 由于 `_requestGeneration` 被自增，装模板那次请求的 `FinishRequest` 会当成
    /// 过期请求直接返回 ⇒ `_isBusy` 永久为 true，整页按钮从此禁用。
    ///
    /// 所以这里不是「顺手加个可选参数」，而是复用这个回调的**前提条件**：
    /// 只有「自己的 reload 不会破坏自己当前这次交互」的页面才能直接用无参版本
    /// （Git 页恢复提交后确实想让自己整页刷新，所以它用无参版本）。
    /// </summary>
    private async Task ReloadCachedProjectPagesExceptAsync(string? exceptPageId)
    {
        foreach (var (id, page) in _pageCache.ToArray())
        {
            if (exceptPageId is not null && string.Equals(id, exceptPageId, StringComparison.Ordinal))
            {
                continue;
            }
            if (page is IProjectDataReloadable)
            {
                await ReloadPageAsync(id, page).ConfigureAwait(true);
            }
        }
        await RefreshBudgetStatusAsync().ConfigureAwait(true);
        await RefreshSidebarBadgesAsync(new SidebarBadgeCounts(0, 0, 0)).ConfigureAwait(true);
        // U196-B 收尾：顶栏维护横幅也要跟着重载走一遍。
        //
        // 没有这一行的症状是：作者点「解除维护失败」，写操作**真的**解禁了，
        // 而横幅仍旧写着「项目维护失败…请先处理后再保存或运行」——
        // 修好了却继续劝退，作者不会去试。
        // ⚠️ 同一行**顺带修好「成功回档后横幅不消失」**：回档成功也走这条重载路径。
        // 这里刷的是宿主自己的状态（不是某个缓存页），所以放在页面循环之后、
        // 与预算/徽章刷新并列 —— 三者都是「重载后宿主该重新问一遍的东西」。
        await RefreshMaintenanceStatusAsync().ConfigureAwait(true);
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
        _projectAutomation.BeginProjectSession();
        _navigationSession.Invalidate();
        _projectPageSessionGeneration++;
        _projectPageSessionCts.Cancel();
        _projectPageSessionCts.Dispose();
        _projectPageSessionCts = new CancellationTokenSource();
        _loadedPageIds.Clear();
        _pageLoadTasks.Clear();
        foreach (var id in ProjectSessionPageIds)
        {
            if (_pageCache.TryGetValue(id, out var page)
                && page is IProjectDataReloadable reloadable)
            {
                reloadable.DeactivateProjectData();
            }
            // 设置页承载全局偏好和应用级目录。保留实例避免切换项目时重置全局交互状态，
            // 但仍清空其项目草稿，由下一次项目会话重新加载项目部分。
            if (!RetainedGlobalPageIds.Contains(id))
            {
                _pageCache.Remove(id);
            }
        }
    }

    internal object GetPageForTests(string id) => GetOrCreatePage(id);

    internal void ApplyGlobalPreferencesForTests(UiPreferences preferences) =>
        ApplyGlobalPreferences(preferences);

    internal Task SaveGlobalPreferencesForTestsAsync(UiPreferences preferences) =>
        SaveGlobalPreferencesAsync(preferences);

    internal Task PersistPanelStateForTestsAsync(string key, bool isOpen) =>
        PersistPanelStateAsync(key, isOpen);

    internal Task PreloadProjectPagesForTestsAsync() => LoadProjectDataPagesAsync();

    /// <summary>测试用：走真实的预算查询 + 分档路径（不是构造一个假的分档值）。</summary>
    internal Task RefreshBudgetStatusForTestsAsync() => RefreshBudgetStatusAsync();

    /// <summary>
    /// 测试用：把「有打开的项目」置真，使迟到终态不会被 `HasOpenProject` 闸挡掉。
    /// 直接设属性而不是跑整套 EnterProject：后者要真后端起项目，
    /// 而本用例要测的是终态 → 刷新这一段接线。
    /// </summary>
    internal void MarkProjectOpenForTests() => HasOpenProject = true;

    /// <summary>
    /// 测试用：暴露预载清单本身。
    ///
    /// U138：这份清单承担着一条**没写在任何地方的安全约定**——
    /// 每个能持有未保存内容的页（`IUnsavedChangesGuard`）都必须在打开项目时被实例化，
    /// 否则 `ConfirmCachedProjectPagesLeaveAsync` 遍历 `_pageCache` 时就看不见它，
    /// Git 回档会静默放行、丢掉内存里未保存的正文。
    /// 把清单本身暴露出来，让那条约定能被断言钉住，而不是靠人记得。
    /// </summary>
    internal static IReadOnlyList<string> PreloadedProjectPageIdsForTests => PreloadedProjectPageIds;

    internal void ResetProjectPageSessionForTests() => ResetProjectPageSession();

    internal string? LastNavigationIdForTests => _lastNavId;

    internal string? SelectedNavigationIdForTests =>
        AllNavigationItems().FirstOrDefault(item => item.IsSelected)?.Id;

    private async Task SelectNavigationItemForProjectAsync(NavigationItemViewModel item)
    {
        var request = _navigationSession.Begin(_projectPageSessionGeneration);
        var page = GetOrCreatePage(item.Id);
        await EnsurePageLoadedAsync(item.Id, page).ConfigureAwait(true);
        if (_navigationSession.IsCurrent(request, _projectPageSessionGeneration))
        {
            CommitNavigation(item, page, persist: false);
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
            BudgetStatusText = UserFacingError.Short(ex, _displayNames, "ui.error.budget");
            BudgetUsagePercent = 0;
            // 读不到预算 ≠ 余量告急。留在 Normal 档，否则一次 IPC 抖动会把顶栏染红，
            // 而红色在这里的语义是「快没钱了」，不是「查询失败了」——
            // 后者已由 BudgetStatusText 那行错误文案承担。
            BudgetSeverity = BudgetSeverity.Normal;
            BudgetRemainingUsd = null;
        }
    }

    private void ApplyBudgetStatus(BudgetStatus status)
    {
        if (status.BudgetUsd <= 0)
        {
            BudgetUsagePercent = 0;
            BudgetStatusText = _displayNames.Text("ui.layout.budget_unlimited");
            // 日预算 0 = **不设上限**（U112），不是「余量为负」。
            // 若按 budget - spent 算，这里会得到 -spent 从而永久报红——
            // 顶栏一直红着等于没有分级，反而把真正的临界态淹掉。
            BudgetSeverity = BudgetSeverity.Normal;
            BudgetRemainingUsd = null;
            return;
        }
        var total = status.BudgetUsd <= 0 ? 0 : Math.Clamp(status.SpentUsd / status.BudgetUsd, 0, 1);
        BudgetUsagePercent = total * 100;
        BudgetStatusText = _displayNames.Format("ui.layout.budget_status", new Dictionary<string, string>
        {
            ["spent"] = status.SpentUsd.ToString("0.##"),
            ["budget"] = status.BudgetUsd.ToString("0.##"),
        });
        // U194-E：判据是**绝对余量**，不是百分比。
        // `BudgetUsagePercent` 算不出规范要的那条线——90% 在 $10 预算下只剩 $1（该报红），
        // 在 $1000 预算下还剩 $100（完全正常）。所以这里必须另算 budget - spent。
        var remaining = status.BudgetUsd - status.SpentUsd;
        BudgetRemainingUsd = remaining;
        BudgetSeverity = ResolveBudgetSeverity(status.BudgetUsd, remaining);
    }

    /// <summary>
    /// 预算余量分档。error 档是 `指导性文件/UI组件状态表.md:34` 的硬要求
    /// （「余量&lt;$2 → bg-status-error + 文字 text-error font-medium」）。
    ///
    /// **边界取严格小于**：余量正好 $2 仍是 warning 档而非 error——
    /// 规范写的是 `<$2`，把等于也算进去会让「刚好卡在阈值上」这一刻的呈现
    /// 与文档不一致，后人复核时无从判断哪个是有意的。
    ///
    /// warning 档取**百分比**而非又一个美元常数，理由是它与量纲无关：
    /// $5 的日预算和 $500 的日预算都能用同一条「已花 80%」判出「快到头了」，
    /// 而任何绝对金额的警戒线都必然在其中一端失真（对 $5 预算 $10 警戒线恒真，
    /// 对 $500 预算 $10 警戒线等于没有）。error 那条之所以能用绝对值，
    /// 是因为它衡量的是「还够不够跑一次调用」——那本身就是绝对量。
    ///
    /// ⚠️ 两档的顺序不能反：小额预算（如 $2.5/日）花掉一点就同时满足两档，
    /// error 必须先判，否则临界状态会被 warning 盖住。
    /// </summary>
    private static BudgetSeverity ResolveBudgetSeverity(double budgetUsd, double remainingUsd)
    {
        if (remainingUsd < BudgetErrorRemainingUsd)
        {
            return BudgetSeverity.Error;
        }
        if (budgetUsd > 0 && remainingUsd / budgetUsd <= 1 - BudgetWarningSpentRatio)
        {
            return BudgetSeverity.Warning;
        }
        return BudgetSeverity.Normal;
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
