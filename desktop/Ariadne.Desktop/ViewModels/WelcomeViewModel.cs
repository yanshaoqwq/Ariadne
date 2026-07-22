using System.Globalization;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

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
        OnPropertyChanged(nameof(RecentCountText));
        OnPropertyChanged(nameof(RecentErrorText));
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
        return entries.Select(entry => new RecentProjectItemViewModel(entry, () => _ = OpenProjectRootAsync(entry.ProjectRoot))).ToArray();
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
            await DialogService.Current.ConfirmAsync(new ConfirmDialogViewModel(
                _displayNames.Text("ui.dialog.open_project.not_project_title"),
                _displayNames.Format(
                    "ui.dialog.open_project.not_project_message",
                    new Dictionary<string, string> { ["path"] = root }),
                new[]
                {
                    new DialogButton(_displayNames.Text("ui.common.close"), DialogButtonVariant.Primary, 0),
                })
            {
                CancelResultIndex = 0,
            }).ConfigureAwait(true);
            StatusText = _displayNames.Text("ui.dialog.open_project.not_project_status");
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

    internal Task RefreshRecentProjectsForTestsAsync() => RefreshRecentProjectsAsync();
}

public sealed class RecentProjectItemViewModel
{
    public RecentProjectItemViewModel(RecentProjectEntry entry, Action open)
    {
        Name = entry.Name;
        ProjectRoot = entry.ProjectRoot;
        LastOpenedAt = FormatLastOpened(entry.LastOpenedMs);
        OpenCommand = new RelayCommand(open);
    }

    public string Name { get; }
    public string ProjectRoot { get; }
    public string? LastOpenedAt { get; }
    public RelayCommand OpenCommand { get; }

    private static string? FormatLastOpened(ulong lastOpenedMs)
    {
        if (lastOpenedMs == 0 || lastOpenedMs > long.MaxValue)
        {
            return null;
        }

        try
        {
            var dto = DateTimeOffset.FromUnixTimeMilliseconds((long)lastOpenedMs).ToLocalTime();
            return dto.ToString("g", CultureInfo.CurrentCulture);
        }
        catch (ArgumentOutOfRangeException)
        {
            return null;
        }
    }
}
