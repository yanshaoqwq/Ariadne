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
    private bool _isLoading;
    private RecentProjectsState _recentState = RecentProjectsState.Loading;
    private Task? _loadTask;

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
        RetryRecentProjectsCommand = new RelayCommand(() => _ = LoadAsync(), () => !IsLoading);
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

    public bool CanStartProjectAction => !IsLoading;

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

    public bool IsLoading
    {
        get => _isLoading;
        private set
        {
            if (!SetProperty(ref _isLoading, value))
            {
                return;
            }

            OnPropertyChanged(nameof(CanStartProjectAction));
            CreateProjectCommand.NotifyCanExecuteChanged();
            OpenProjectCommand.NotifyCanExecuteChanged();
            RetryRecentProjectsCommand.NotifyCanExecuteChanged();
        }
    }

    public IReadOnlyList<RecentProjectItemViewModel> RecentProjects
    {
        get => _recentProjects;
        private set => SetProperty(ref _recentProjects, value);
    }

    public Task LoadAsync()
    {
        if (_loadTask is not null || IsLoading)
        {
            return _loadTask ?? Task.CompletedTask;
        }

        _loadTask = LoadRecentProjectsAsync();
        return _loadTask;
    }

    private async Task LoadRecentProjectsAsync()
    {
        IsLoading = true;
        SetRecentState(RecentProjectsState.Loading);
        try
        {
            await RefreshRecentProjectsAsync().ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            _recentErrorText = UserFacingError.Format(ex, _displayNames);
            SetRecentState(RecentProjectsState.Error);
            OnPropertyChanged(nameof(RecentErrorText));
        }
        finally
        {
            IsLoading = false;
            _loadTask = null;
        }
    }

    private async Task RefreshRecentProjectsAsync()
    {
        RecentProjects = WrapRecentProjects(await _backend.ListRecentProjectsAsync().ConfigureAwait(true));
        NotifyRecentProjectsChanged();
        SetRecentState(RecentProjects.Count == 0
            ? RecentProjectsState.Empty
            : RecentProjectsState.Content);
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

    private async Task CreateProjectAsync()
    {
        if (IsLoading)
        {
            return;
        }

        IsLoading = true;
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

            var root = ProjectPathHelper.BuildUniqueProjectRoot(parent, projectName);
            Directory.CreateDirectory(root);

            var report = await _backend.CreateProjectAsync(root, projectName).ConfigureAwait(true);
            StatusText = _displayNames.Format(
                "ui.welcome.create_project_done",
                new Dictionary<string, string>
                {
                    ["name"] = projectName,
                    ["path"] = report.ProjectRoot,
                });
            await RefreshRecentProjectsAsync().ConfigureAwait(true);
            var status = await _backend.GetCurrentProjectAsync().ConfigureAwait(true);
            if (status is not null && _projectOpened is not null)
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
            IsLoading = false;
        }
    }

    private async Task OpenProjectAsync()
    {
        if (IsLoading)
        {
            return;
        }

        IsLoading = true;
        try
        {
            var root = await _pickProjectFolder(
                _displayNames.Text("ui.dialog.open_project.pick_title")).ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(root))
            {
                StatusText = _displayNames.Text("ui.common.cancel");
                return;
            }

            // 本地预检：未初始化项目直接友好提示，避免只看到后端英文/技术报错
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

            await OpenProjectRootCoreAsync(root).ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
        finally
        {
            IsLoading = false;
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
        if (IsLoading)
        {
            return;
        }

        IsLoading = true;
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
            IsLoading = false;
        }
    }

    /// <summary>供主窗口标题栏「切换项目」调用。</summary>
    public async Task OpenProjectRootForHostAsync(string root)
    {
        if (IsLoading)
        {
            return;
        }

        IsLoading = true;
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
            IsLoading = false;
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
}

public sealed class RecentProjectItemViewModel
{
    public RecentProjectItemViewModel(RecentProjectEntry entry, Action open)
    {
        Name = entry.Name;
        ProjectRoot = entry.ProjectRoot;
        LastOpenedAt = entry.LastOpenedAt;
        OpenCommand = new RelayCommand(open);
    }

    public string Name { get; }
    public string ProjectRoot { get; }
    public string? LastOpenedAt { get; }
    public RelayCommand OpenCommand { get; }
}
