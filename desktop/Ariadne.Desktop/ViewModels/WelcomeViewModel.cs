using System.Globalization;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 最近项目条目的健康度。
///
/// 两种失效原因分开表达，而不是一个 bool：它们的**出路不同**——
/// 目录不存在只能重新定位或移除；目录还在、只缺 .config/app.yaml 时
/// 「在此目录初始化」是可行的。合并成一个状态就没法给对出路（U143）。
/// </summary>
public enum RecentProjectHealth
{
    /// <summary>体检未完成。渲染上等同「正常」，避免进页面时整列先闪灰。</summary>
    Unknown,

    /// <summary>目录存在且含 .config/app.yaml。</summary>
    Healthy,

    /// <summary>目录不存在（被移动、重命名或删除）。</summary>
    Missing,

    /// <summary>目录存在但缺 .config/app.yaml，不是 Ariadne 项目。</summary>
    NotAProject,
}

public sealed class WelcomeViewModel : ViewModelBase
{
    private enum RecentProjectsState
    {
        Loading,
        Content,
        Empty,
        Error,
    }

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private readonly Func<CurrentProjectStatus, Task>? _projectOpened;
    private Func<string?, Task<string?>> _pickProjectFolder;
    private IReadOnlyList<RecentProjectItemViewModel> _recentProjects = Array.Empty<RecentProjectItemViewModel>();
    private string _statusText = string.Empty;
    private string _recentErrorText = string.Empty;
    private bool _isRecentProjectsLoading;
    private bool _isProjectActionRunning;
    private RecentProjectsState _recentState = RecentProjectsState.Loading;
    private Task? _loadTask;
    private readonly RequestGenerationSession _recentProjectsSession = new();

    public WelcomeViewModel(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        Func<CurrentProjectStatus, Task>? projectOpened = null,
        Func<string?, Task<string?>>? pickProjectFolder = null)
    {
        _displayNames = displayNames;
        _backend = backend;
        _projectOpened = projectOpened;
        _pickProjectFolder = pickProjectFolder ?? (_ => Task.FromResult<string?>(null));
        CreateProjectCommand = new RelayCommand(() => _ = CreateProjectAsync(), () => CanStartProjectAction);
        OpenProjectCommand = new RelayCommand(() => _ = OpenProjectAsync(), () => CanStartProjectAction);
        RetryRecentProjectsCommand = new RelayCommand(() => _ = LoadAsync(), () => !_isRecentProjectsLoading);
        TutorialCommand = new RelayCommand(() => _ = ShowTutorialAsync());
        FeedbackCommand = new RelayCommand(() => _ = ShowFeedbackAsync());
        _displayNames.LanguageChanged += (_, _) => RefreshLocalizedText();
    }

    public string BrandName => _displayNames.Text("ui.brand.name");

    public string BrandLetter => _displayNames.Text("ui.brand.logo_letter");

    public string Subtitle => _displayNames.Text("ui.welcome.subtitle");

    public string HeroTagline => _displayNames.Text("ui.welcome.hero_tagline");

    public string RecentProjectsTitle => _displayNames.Text("ui.welcome.recent_projects");

    public string CreateProjectText => _displayNames.Text("ui.layout.create_project");

    public string CreateProjectHint => _displayNames.Text("ui.welcome.create_hint");

    public string OpenProjectText => _displayNames.Text("ui.layout.open_project");

    public string OpenProjectHint => _displayNames.Text("ui.welcome.open_hint");

    public string TutorialText => _displayNames.Text("ui.settings.index.tutorial");

    public string FeedbackText => _displayNames.Text("ui.layout.feedback");

    public string EmptyRecentTitle => _displayNames.Text("ui.welcome.recent_empty_title");

    public string EmptyRecentHint => _displayNames.Text("ui.welcome.recent_empty_hint");

    public string RecentLoadingText => _displayNames.Text("ui.welcome.recent_loading");

    public string RetryRecentProjectsText => _displayNames.Text("ui.welcome.retry_recent");

    public string RecentProjectActionsText => _displayNames.Text("ui.welcome.recent.actions");

    public string RelocateRecentProjectText => _displayNames.Text("ui.welcome.recent.relocate");

    public string ForgetRecentProjectText => _displayNames.Text("ui.welcome.recent.forget");

    public bool CanStartProjectAction => !_isProjectActionRunning;

    public bool HasStatusText => !string.IsNullOrWhiteSpace(StatusText);

    public bool HasRecentProjects => _recentState == RecentProjectsState.Content && RecentProjects.Count > 0;

    public bool IsRecentLoading => _recentState == RecentProjectsState.Loading;

    public bool IsRecentEmpty => _recentState == RecentProjectsState.Empty;

    public bool IsRecentError => _recentState == RecentProjectsState.Error;

    public string RecentErrorText => _recentErrorText;

    public string RecentCountText => _displayNames.Format(
        "ui.welcome.recent_project_count",
        new Dictionary<string, string> { ["count"] = RecentProjects.Count.ToString() });

    public RelayCommand CreateProjectCommand { get; }

    public RelayCommand OpenProjectCommand { get; }

    public RelayCommand TutorialCommand { get; }

    public RelayCommand FeedbackCommand { get; }

    public RelayCommand RetryRecentProjectsCommand { get; }

    /// <summary>title 为选择器标题（新建=父目录 / 打开=项目根）。</summary>
    public void SetProjectFolderPicker(Func<string?, Task<string?>> picker)
    {
        _pickProjectFolder = picker;
    }

    public void ClearProjectFolderPicker(Func<string?, Task<string?>> picker)
    {
        if (_pickProjectFolder == picker)
        {
            _pickProjectFolder = _ => Task.FromResult<string?>(null);
        }
    }

    private void RefreshLocalizedText()
    {
        OnPropertyChanged(nameof(BrandName));
        OnPropertyChanged(nameof(BrandLetter));
        OnPropertyChanged(nameof(Subtitle));
        OnPropertyChanged(nameof(HeroTagline));
        OnPropertyChanged(nameof(RecentProjectsTitle));
        OnPropertyChanged(nameof(CreateProjectText));
        OnPropertyChanged(nameof(CreateProjectHint));
        OnPropertyChanged(nameof(OpenProjectText));
        OnPropertyChanged(nameof(OpenProjectHint));
        OnPropertyChanged(nameof(TutorialText));
        OnPropertyChanged(nameof(FeedbackText));
        OnPropertyChanged(nameof(EmptyRecentTitle));
        OnPropertyChanged(nameof(EmptyRecentHint));
        OnPropertyChanged(nameof(RecentLoadingText));
        OnPropertyChanged(nameof(RetryRecentProjectsText));
        OnPropertyChanged(nameof(RecentProjectActionsText));
        OnPropertyChanged(nameof(RelocateRecentProjectText));
        OnPropertyChanged(nameof(ForgetRecentProjectText));
        OnPropertyChanged(nameof(RecentCountText));
        OnPropertyChanged(nameof(RecentErrorText));
        foreach (var item in RecentProjects)
        {
            item.RefreshLocalizedUi();
        }
    }

    public string StatusText
    {
        get => _statusText;
        set
        {
            if (SetProperty(ref _statusText, value))
            {
                OnPropertyChanged(nameof(HasStatusText));
            }
        }
    }

    public bool IsLoading => _isRecentProjectsLoading || _isProjectActionRunning;

    public IReadOnlyList<RecentProjectItemViewModel> RecentProjects
    {
        get => _recentProjects;
        private set => SetProperty(ref _recentProjects, value);
    }

    public Task LoadAsync()
    {
        if (_loadTask is not null)
        {
            return _loadTask ?? Task.CompletedTask;
        }

        _loadTask = LoadRecentProjectsAsync();
        return _loadTask;
    }

    private async Task LoadRecentProjectsAsync()
    {
        try
        {
            await RefreshRecentProjectsAsync().ConfigureAwait(true);
        }
        finally
        {
            _loadTask = null;
        }
    }

    private void SetRecentProjectsLoading(bool value)
    {
        if (_isRecentProjectsLoading == value)
        {
            return;
        }

        _isRecentProjectsLoading = value;
        OnPropertyChanged(nameof(IsLoading));
        RetryRecentProjectsCommand.NotifyCanExecuteChanged();
    }

    private void SetProjectActionRunning(bool value)
    {
        if (_isProjectActionRunning == value)
        {
            return;
        }

        _isProjectActionRunning = value;
        OnPropertyChanged(nameof(IsLoading));
        OnPropertyChanged(nameof(CanStartProjectAction));
        CreateProjectCommand.NotifyCanExecuteChanged();
        OpenProjectCommand.NotifyCanExecuteChanged();
        foreach (var item in RecentProjects)
        {
            item.NotifyCanExecuteChanged();
        }
    }

    private async Task RefreshRecentProjectsAsync()
    {
        var request = _recentProjectsSession.Begin();
        SetRecentProjectsLoading(true);
        SetRecentState(RecentProjectsState.Loading);
        try
        {
            var entries = await _backend
                .ListRecentProjectsAsync(request.CancellationToken)
                .ConfigureAwait(true);
            if (!_recentProjectsSession.IsCurrent(request))
            {
                return;
            }

            RecentProjects = WrapRecentProjects(entries);
            _recentErrorText = string.Empty;
            OnPropertyChanged(nameof(RecentErrorText));
            NotifyRecentProjectsChanged();
            SetRecentState(RecentProjects.Count == 0
                ? RecentProjectsState.Empty
                : RecentProjectsState.Content);

            // 列表先渲染、体检随后回填：体检要读磁盘，等它完成再显示会让
            // 欢迎页在慢盘/失效网络路径上白屏。条目默认 Unknown（视觉同正常），
            // 结论到了才置灰，不会先闪一下。
            await ProbeRecentProjectHealthAsync(RecentProjects, request).ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (request.CancellationToken.IsCancellationRequested)
        {
        }
        catch (Exception ex)
        {
            if (_recentProjectsSession.IsCurrent(request))
            {
                _recentErrorText = UserFacingError.Format(ex, _displayNames);
                SetRecentState(RecentProjectsState.Error);
                OnPropertyChanged(nameof(RecentErrorText));
            }
        }
        finally
        {
            if (_recentProjectsSession.IsCurrent(request))
            {
                SetRecentProjectsLoading(false);
            }
        }
    }

    private void SetRecentState(RecentProjectsState state)
    {
        if (_recentState == state)
        {
            return;
        }

        _recentState = state;
        OnPropertyChanged(nameof(HasRecentProjects));
        OnPropertyChanged(nameof(IsRecentLoading));
        OnPropertyChanged(nameof(IsRecentEmpty));
        OnPropertyChanged(nameof(IsRecentError));
    }

    private void NotifyRecentProjectsChanged()
    {
        OnPropertyChanged(nameof(HasRecentProjects));
        OnPropertyChanged(nameof(IsRecentEmpty));
        OnPropertyChanged(nameof(RecentCountText));
    }

    internal async Task CreateProjectAsync()
    {
        if (_isProjectActionRunning)
        {
            return;
        }

        SetProjectActionRunning(true);
        try
        {
            // 1) 先取项目名
            var nameDialog = ConfirmDialogViewModel.CreateProjectName(_displayNames);
            var nameResult = await DialogService.Current.ConfirmAsync(nameDialog).ConfigureAwait(true);
            if (nameResult != 0)
            {
                StatusText = _displayNames.Text("ui.common.cancel");
                return;
            }

            var projectName = nameDialog.InputText.Trim();
            if (string.IsNullOrWhiteSpace(projectName))
            {
                StatusText = _displayNames.Text("ui.dialog.create_project.name_required");
                return;
            }

            // 2) 再选父目录（不是直接把项目塞进该目录根）
            var parent = await _pickProjectFolder(
                _displayNames.Text("ui.dialog.create_project.pick_parent_title")).ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(parent))
            {
                StatusText = _displayNames.Text("ui.common.cancel");
                return;
            }

            await CreateProjectAtAsync(parent, projectName).ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
        finally
        {
            SetProjectActionRunning(false);
        }
    }

    internal async Task<CurrentProjectStatus?> CreateProjectAtAsync(
        string parentDirectory,
        string projectName,
        CancellationToken cancellationToken = default)
    {
        var root = ProjectPathHelper.BuildUniqueProjectRoot(parentDirectory, projectName);
        var report = await _backend
            .CreateProjectAsync(root, projectName, cancellationToken)
            .ConfigureAwait(true);
        if (!ProjectInitializationReportIsComplete(report, root))
        {
            StatusText = _displayNames.Text("ui.welcome.create_project_incomplete");
            return null;
        }

        var status = new CurrentProjectStatus(report.ProjectRoot, report.ProjectName);
        await RefreshRecentProjectsAsync().ConfigureAwait(true);
        if (_projectOpened is not null)
        {
            await _projectOpened(status).ConfigureAwait(true);
        }
        StatusText = _displayNames.Format(
            "ui.welcome.create_project_done",
            new Dictionary<string, string>
            {
                ["name"] = report.ProjectName,
                ["path"] = report.ProjectRoot,
            });
        return status;
    }

    private static bool ProjectInitializationReportIsComplete(
        ProjectInitReport report,
        string requestedRoot)
    {
        if (!report.Ready
            || !report.GitInitialized
            || string.IsNullOrWhiteSpace(report.ProjectRoot)
            || string.IsNullOrWhiteSpace(report.ProjectName)
            || report.CreatedDirs is null
            || report.CreatedDirs.Count == 0
            || report.CreatedConfigFiles is null
            || report.CreatedConfigFiles.Count == 0)
        {
            return false;
        }

        try
        {
            var comparison = OperatingSystem.IsWindows()
                ? StringComparison.OrdinalIgnoreCase
                : StringComparison.Ordinal;
            return string.Equals(
                Path.GetFullPath(report.ProjectRoot),
                Path.GetFullPath(requestedRoot),
                comparison);
        }
        catch
        {
            return false;
        }
    }

    internal async Task OpenProjectAsync()
    {
        if (_isProjectActionRunning)
        {
            return;
        }

        SetProjectActionRunning(true);
        try
        {
            var root = await _pickProjectFolder(
                _displayNames.Text("ui.dialog.open_project.pick_title")).ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(root))
            {
                StatusText = _displayNames.Text("ui.common.cancel");
                return;
            }

            await OpenProjectRootCoreAsync(root).ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
        finally
        {
            SetProjectActionRunning(false);
        }
    }

    private IReadOnlyList<RecentProjectItemViewModel> WrapRecentProjects(IReadOnlyList<RecentProjectEntry> entries)
    {
        return entries.Select(entry => new RecentProjectItemViewModel(
            entry,
            _displayNames,
            () => _ = OpenProjectRootAsync(entry.ProjectRoot),
            () => _ = RelocateRecentProjectAsync(entry),
            () => _ = ForgetRecentProjectAsync(entry),
            () => CanStartProjectAction)).ToArray();
    }

    /// <summary>
    /// 给列表逐条做存在性体检，并把结论回填到条目上。
    ///
    /// 体检是**磁盘 IO**，列表最多 20 条，且失效条目往往落在已卸载的外部盘或
    /// 网络路径上——那里的 <c>Directory.Exists</c> 可能阻塞到超时。在 UI 线程
    /// 同步跑一遍会让欢迎页直接卡住，所以整批丢进 <c>Task.Run</c>，
    /// 只把结论切回 UI 线程回填。
    ///
    /// 结论回填用 <c>ConfigureAwait(true)</c> 回到原上下文：<c>Health</c> 会触发
    /// <c>PropertyChanged</c>，绑定必须在 UI 线程上收到。
    /// </summary>
    private async Task ProbeRecentProjectHealthAsync(
        IReadOnlyList<RecentProjectItemViewModel> items,
        RequestGeneration request)
    {
        if (items.Count == 0)
        {
            return;
        }

        var roots = items.Select(item => item.ProjectRoot).ToArray();
        var results = await Task.Run(
            () => roots.Select(InspectRecentProject).ToArray(),
            request.CancellationToken).ConfigureAwait(true);

        // 体检期间用户可能已经刷新过列表；过期结论不能覆盖新列表的状态。
        if (!_recentProjectsSession.IsCurrent(request))
        {
            return;
        }

        for (var i = 0; i < items.Count && i < results.Length; i++)
        {
            items[i].ApplyHealth(results[i]);
        }
    }

    /// <summary>
    /// 单条体检。区分「目录不存在」与「目录在但不是项目」——两者出路不同。
    /// </summary>
    private static RecentProjectHealth InspectRecentProject(string projectRoot)
    {
        if (string.IsNullOrWhiteSpace(projectRoot))
        {
            return RecentProjectHealth.Missing;
        }

        try
        {
            var root = Path.GetFullPath(projectRoot.Trim());
            if (!Directory.Exists(root))
            {
                return RecentProjectHealth.Missing;
            }

            // 与 ProjectPathHelper.LooksLikeInitializedProject 同一判据，
            // 保证「列表显示可用」与「点开真的能开」不会给出相反结论。
            return File.Exists(Path.Combine(root, ".config", "app.yaml"))
                ? RecentProjectHealth.Healthy
                : RecentProjectHealth.NotAProject;
        }
        catch
        {
            // 权限不足、路径非法等一律按不可用处理：让用户看到失效标记，
            // 好过给一个点下去才报错的「正常」条目。
            return RecentProjectHealth.Missing;
        }
    }

    private async Task RelocateRecentProjectAsync(RecentProjectEntry entry)
    {
        if (_isProjectActionRunning)
        {
            return;
        }

        SetProjectActionRunning(true);
        try
        {
            var root = await _pickProjectFolder(
                _displayNames.Text("ui.welcome.recent.relocate_picker_title")).ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(root))
            {
                StatusText = _displayNames.Text("ui.common.cancel");
                return;
            }
            if (!ProjectPathHelper.LooksLikeInitializedProject(root))
            {
                await ShowNotProjectDialogAsync(root).ConfigureAwait(true);
                return;
            }

            var status = await _backend
                .RelocateRecentProjectAsync(entry.ProjectRoot, root)
                .ConfigureAwait(true);
            await RefreshRecentProjectsAsync().ConfigureAwait(true);
            StatusText = _displayNames.Format(
                "ui.welcome.recent.relocated",
                new Dictionary<string, string> { ["path"] = status.ProjectRoot });
            if (_projectOpened is not null)
            {
                await _projectOpened(status).ConfigureAwait(true);
            }
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
        finally
        {
            SetProjectActionRunning(false);
        }
    }

    private async Task ForgetRecentProjectAsync(RecentProjectEntry entry)
    {
        if (_isProjectActionRunning)
        {
            return;
        }

        var dialog = new ConfirmDialogViewModel(
            _displayNames.Text("ui.welcome.recent.forget_confirm_title"),
            _displayNames.Format(
                "ui.welcome.recent.forget_confirm_message",
                new Dictionary<string, string>
                {
                    ["name"] = entry.Name,
                    ["path"] = entry.ProjectRoot,
                }),
            new[]
            {
                new DialogButton(_displayNames.Text("ui.welcome.recent.forget"), DialogButtonVariant.Danger, 0),
                new DialogButton(_displayNames.Text("ui.common.cancel"), DialogButtonVariant.Subtle, 1),
            })
        {
            CancelResultIndex = 1,
        };
        if (await DialogService.Current.ConfirmAsync(dialog).ConfigureAwait(true) != 0)
        {
            return;
        }

        SetProjectActionRunning(true);
        try
        {
            var entries = await _backend
                .ForgetRecentProjectAsync(entry.ProjectRoot)
                .ConfigureAwait(true);
            var items = WrapRecentProjects(entries);
            RecentProjects = items;
            NotifyRecentProjectsChanged();
            SetRecentState(RecentProjects.Count == 0
                ? RecentProjectsState.Empty
                : RecentProjectsState.Content);
            StatusText = _displayNames.Format(
                "ui.welcome.recent.forgotten",
                new Dictionary<string, string> { ["name"] = entry.Name });
            // 重建后的条目健康度是 Unknown，不体检的话剩余失效项会变回「看着正常」。
            await ProbeRecentProjectHealthAsync(items, _recentProjectsSession.Begin())
                .ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
        finally
        {
            SetProjectActionRunning(false);
        }
    }

    private async Task ShowTutorialAsync()
    {
        StatusText = TutorialText;
        await DialogService.Current.ConfirmAsync(HelpDialogFactory.CreateTutorialDialog(_displayNames)).ConfigureAwait(true);
    }

    private async Task ShowFeedbackAsync()
    {
        StatusText = FeedbackText;
        var result = await DialogService.Current
            .ConfirmAsync(HelpDialogFactory.CreateFeedbackDialog(_displayNames))
            .ConfigureAwait(true);
        if (result == 1 && !ExternalLinkOpener.TryOpen(HelpDialogFactory.FeedbackIssueUrl))
        {
            StatusText = _displayNames.Text("ui.feedback.open_failed");
        }
    }

    private async Task OpenProjectRootAsync(string root)
    {
        if (_isProjectActionRunning)
        {
            return;
        }

        SetProjectActionRunning(true);
        try
        {
            await OpenProjectRootCoreAsync(root).ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
        finally
        {
            SetProjectActionRunning(false);
        }
    }

    /// <summary>供主窗口标题栏「切换项目」调用。</summary>
    public async Task OpenProjectRootForHostAsync(string root)
    {
        if (_isProjectActionRunning)
        {
            return;
        }

        SetProjectActionRunning(true);
        try
        {
            await OpenProjectRootCoreAsync(root).ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
        finally
        {
            SetProjectActionRunning(false);
        }
    }

    private async Task OpenProjectRootCoreAsync(string root)
    {
        // 最近列表也可能指向已删/未初始化目录，与「打开」共用本地预检
        if (!ProjectPathHelper.LooksLikeInitializedProject(root))
        {
            await ShowNotProjectDialogAsync(root).ConfigureAwait(true);
            return;
        }

        var status = await _backend.OpenProjectAsync(root).ConfigureAwait(true);
        await RefreshRecentProjectsAsync().ConfigureAwait(true);
        StatusText = status.ProjectRoot;
        if (_projectOpened is not null)
        {
            await _projectOpened(status).ConfigureAwait(true);
        }
    }

    /// <summary>
    /// 打不开时的出路对话框。
    ///
    /// 原版只有一个「关闭」按钮：用户撞上死路后没有任何可做的事，
    /// 得自己猜到去菜单里找「重新定位」。现在按失效原因给不同出路——
    /// 目录还在（只是缺 app.yaml）时「在此目录初始化」是最省事的一条，
    /// 目录已不存在时给它反而是误导，所以只在前者出现。
    /// </summary>
    private async Task ShowNotProjectDialogAsync(string root)
    {
        var health = InspectRecentProject(root);
        var missing = health == RecentProjectHealth.Missing;

        var title = _displayNames.Text(missing
            ? "ui.dialog.open_project.missing_title"
            : "ui.dialog.open_project.not_project_title");
        var message = _displayNames.Format(
            missing
                ? "ui.dialog.open_project.missing_message"
                : "ui.dialog.open_project.not_project_message",
            new Dictionary<string, string> { ["path"] = root });
        if (!missing)
        {
            // 目录存在这件事本身是关键信息：用户据此才知道「不是没了，是没初始化」。
            message += "\n\n" + _displayNames.Text("ui.dialog.open_project.not_project_hint");
        }

        var buttons = new List<DialogButton>();
        if (!missing)
        {
            buttons.Add(new DialogButton(
                _displayNames.Text("ui.dialog.open_project.initialize_here"),
                DialogButtonVariant.Primary,
                buttons.Count));
        }

        var relocateIndex = buttons.Count;
        buttons.Add(new DialogButton(
            _displayNames.Text("ui.dialog.open_project.relocate"),
            missing ? DialogButtonVariant.Primary : DialogButtonVariant.Subtle,
            relocateIndex));

        var forgetIndex = buttons.Count;
        buttons.Add(new DialogButton(
            _displayNames.Text("ui.dialog.open_project.forget"),
            DialogButtonVariant.Danger,
            forgetIndex));

        var closeIndex = buttons.Count;
        buttons.Add(new DialogButton(
            _displayNames.Text("ui.common.close"),
            DialogButtonVariant.Subtle,
            closeIndex));

        var choice = await DialogService.Current.ConfirmAsync(
            new ConfirmDialogViewModel(title, message, buttons)
            {
                CancelResultIndex = closeIndex,
            }).ConfigureAwait(true);

        StatusText = _displayNames.Text(missing
            ? "ui.dialog.open_project.missing_status"
            : "ui.dialog.open_project.not_project_status");

        if (choice == closeIndex || choice < 0)
        {
            // choice < 0 是「已有弹窗占着，本次请求被直接拒了」（ConfirmAsync 返回 -1）。
            // 必须当成「什么都不做」——落到下面任何一条出路都等于用户没点却被执行了。
            return;
        }
        if (choice == forgetIndex)
        {
            await ForgetProjectRootAsync(root).ConfigureAwait(true);
            return;
        }
        if (choice == relocateIndex)
        {
            await RelocateProjectRootAsync(root).ConfigureAwait(true);
            return;
        }

        // 只剩「在此目录初始化」：目录存在时才会有这个按钮。
        await InitializeProjectAtAsync(root).ConfigureAwait(true);
    }

    /// <summary>
    /// 在已存在但未初始化的目录上就地建项目。
    ///
    /// 走 <c>CreateProjectAsync</c> 传目录本身（而不是
    /// <c>BuildUniqueProjectRoot</c> 再造一层子目录）——用户指的就是这个目录，
    /// 给它建个 xxx_2 兄弟目录只会让人更糊涂。
    /// </summary>
    private async Task InitializeProjectAtAsync(string root)
    {
        try
        {
            var name = new DirectoryInfo(root.TrimEnd(
                Path.DirectorySeparatorChar,
                Path.AltDirectorySeparatorChar)).Name;
            var report = await _backend.CreateProjectAsync(root, name).ConfigureAwait(true);
            var status = new CurrentProjectStatus(report.ProjectRoot, report.ProjectName);
            await RefreshRecentProjectsAsync().ConfigureAwait(true);
            StatusText = _displayNames.Format(
                "ui.welcome.recent.initialized_here",
                new Dictionary<string, string> { ["path"] = report.ProjectRoot });
            if (_projectOpened is not null)
            {
                await _projectOpened(status).ConfigureAwait(true);
            }
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    /// <summary>
    /// 从对话框直接重新定位。与菜单里的 <c>RelocateCommand</c> 走同一后端命令，
    /// 差别只是入口——用户撞上死路的当下就能改，不用再去翻菜单。
    /// </summary>
    private async Task RelocateProjectRootAsync(string previousRoot)
    {
        try
        {
            var picked = await _pickProjectFolder(
                _displayNames.Text("ui.welcome.recent.relocate_picker_title")).ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(picked))
            {
                StatusText = _displayNames.Text("ui.common.cancel");
                return;
            }
            if (!ProjectPathHelper.LooksLikeInitializedProject(picked))
            {
                // 新选的位置同样可能不是项目：直接复用本对话框，避免死路套死路。
                await ShowNotProjectDialogAsync(picked).ConfigureAwait(true);
                return;
            }

            var status = await _backend
                .RelocateRecentProjectAsync(previousRoot, picked)
                .ConfigureAwait(true);
            await RefreshRecentProjectsAsync().ConfigureAwait(true);
            StatusText = _displayNames.Format(
                "ui.welcome.recent.relocated",
                new Dictionary<string, string> { ["path"] = status.ProjectRoot });
            if (_projectOpened is not null)
            {
                await _projectOpened(status).ConfigureAwait(true);
            }
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    /// <summary>从对话框直接移除失效条目（不再二次确认：用户刚看过路径与原因）。</summary>
    private async Task ForgetProjectRootAsync(string root)
    {
        try
        {
            var entries = await _backend.ForgetRecentProjectAsync(root).ConfigureAwait(true);
            var items = WrapRecentProjects(entries);
            RecentProjects = items;
            NotifyRecentProjectsChanged();
            SetRecentState(items.Count == 0
                ? RecentProjectsState.Empty
                : RecentProjectsState.Content);
            StatusText = _displayNames.Format(
                "ui.welcome.recent.forgotten",
                new Dictionary<string, string> { ["name"] = root });
            await ProbeRecentProjectHealthAsync(items, _recentProjectsSession.Begin())
                .ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    internal Task RefreshRecentProjectsForTestsAsync() => RefreshRecentProjectsAsync();
}

public sealed class RecentProjectItemViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly ulong _lastOpenedMs;
    private RecentProjectHealth _health = RecentProjectHealth.Unknown;

    public RecentProjectItemViewModel(
        RecentProjectEntry entry,
        DisplayNameService displayNames,
        Action open,
        Action relocate,
        Action forget,
        Func<bool> canInteract)
    {
        _displayNames = displayNames;
        _lastOpenedMs = entry.LastOpenedMs;
        Name = entry.Name;
        ProjectRoot = entry.ProjectRoot;
        RefreshLocalizedUi();
        OpenCommand = new RelayCommand(open, canInteract);
        RelocateCommand = new RelayCommand(relocate, canInteract);
        ForgetCommand = new RelayCommand(forget, canInteract);
    }

    public string Name { get; }
    public string ProjectRoot { get; }
    public string? LastOpenedText { get; private set; }
    public bool HasLastOpened => !string.IsNullOrWhiteSpace(LastOpenedText);
    public RelayCommand OpenCommand { get; }
    public RelayCommand RelocateCommand { get; }
    public RelayCommand ForgetCommand { get; }

    /// <summary>
    /// 条目健康度。<see cref="RecentProjectHealth.Unknown"/> 表示体检尚未完成——
    /// 此时**不得**渲染成失效，否则每次进欢迎页所有条目都会先闪一下灰。
    /// </summary>
    public RecentProjectHealth Health
    {
        get => _health;
        private set
        {
            if (SetProperty(ref _health, value))
            {
                OnPropertyChanged(nameof(IsUnavailable));
                OnPropertyChanged(nameof(UnavailableText));
                OnPropertyChanged(nameof(HasUnavailableText));
                OnPropertyChanged(nameof(UnavailableHint));
            }
        }
    }

    /// <summary>列表项是否应置灰。</summary>
    public bool IsUnavailable =>
        Health is RecentProjectHealth.Missing or RecentProjectHealth.NotAProject;

    /// <summary>
    /// 失效原因文案。两种原因**刻意分开**：目录不存在只能重新定位或移除，
    /// 而目录还在、只是缺 .config/app.yaml，则「在此目录初始化」是可行出路——
    /// 混成一句话会让用户对后者也以为无药可救。
    /// </summary>
    public string? UnavailableText => Health switch
    {
        RecentProjectHealth.Missing => _displayNames.Text("ui.welcome.recent.unavailable_missing"),
        RecentProjectHealth.NotAProject => _displayNames.Text("ui.welcome.recent.unavailable_not_project"),
        _ => null,
    };

    public bool HasUnavailableText => !string.IsNullOrWhiteSpace(UnavailableText);

    /// <summary>失效条目的补充提示（「目录已移动或删除」）。</summary>
    public string? UnavailableHint => IsUnavailable
        ? _displayNames.Text("ui.welcome.recent.unavailable_hint")
        : null;

    /// <summary>体检结论回填（由 <see cref="WelcomeViewModel"/> 在后台线程算完后调用）。</summary>
    internal void ApplyHealth(RecentProjectHealth health) => Health = health;

    internal void NotifyCanExecuteChanged()
    {
        OpenCommand.NotifyCanExecuteChanged();
        RelocateCommand.NotifyCanExecuteChanged();
        ForgetCommand.NotifyCanExecuteChanged();
    }

    internal void RefreshLocalizedUi()
    {
        var time = FormatLastOpened(_lastOpenedMs, _displayNames.CurrentLanguage);
        LastOpenedText = time is null
            ? null
            : _displayNames.Format(
                "ui.welcome.recent.last_opened",
                new Dictionary<string, string> { ["time"] = time });
        OnPropertyChanged(nameof(LastOpenedText));
        OnPropertyChanged(nameof(HasLastOpened));
        // 失效文案同样是本地化文本，切语言时必须一起刷新。
        OnPropertyChanged(nameof(UnavailableText));
        OnPropertyChanged(nameof(HasUnavailableText));
        OnPropertyChanged(nameof(UnavailableHint));
    }

    private static string? FormatLastOpened(ulong lastOpenedMs, string language)
    {
        if (lastOpenedMs == 0 || lastOpenedMs > long.MaxValue)
        {
            return null;
        }

        try
        {
            var dto = DateTimeOffset.FromUnixTimeMilliseconds((long)lastOpenedMs).ToLocalTime();
            var culture = language switch
            {
                "en" => CultureInfo.GetCultureInfo("en-US"),
                "ja" => CultureInfo.GetCultureInfo("ja-JP"),
                _ => CultureInfo.GetCultureInfo("zh-CN"),
            };
            return dto.ToString("g", culture);
        }
        catch (ArgumentOutOfRangeException)
        {
            return null;
        }
    }
}
