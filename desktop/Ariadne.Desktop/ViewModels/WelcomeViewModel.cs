using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class WelcomeViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private readonly Func<CurrentProjectStatus, Task>? _projectOpened;
    private Func<string?, Task<string?>> _pickProjectFolder;
    private IReadOnlyList<RecentProjectItemViewModel> _recentProjects = Array.Empty<RecentProjectItemViewModel>();
    private string _statusText;
    private bool _isLoading;

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
        _statusText = displayNames.Text("ui.common.loading");
        CreateProjectCommand = new RelayCommand(() => _ = CreateProjectAsync());
        OpenProjectCommand = new RelayCommand(() => _ = OpenProjectAsync());
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

    public bool HasRecentProjects => RecentProjects.Count > 0;

    public bool IsRecentEmpty => RecentProjects.Count == 0;

    public string RecentCountText => _displayNames.Format(
        "ui.welcome.recent_project_count",
        new Dictionary<string, string> { ["count"] = RecentProjects.Count.ToString() });

    public RelayCommand CreateProjectCommand { get; }

    public RelayCommand OpenProjectCommand { get; }

    public RelayCommand TutorialCommand { get; }

    public RelayCommand FeedbackCommand { get; }

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
        OnPropertyChanged(nameof(RecentCountText));
    }

    public string StatusText
    {
        get => _statusText;
        set => SetProperty(ref _statusText, value);
    }

    public bool IsLoading
    {
        get => _isLoading;
        set => SetProperty(ref _isLoading, value);
    }

    public IReadOnlyList<RecentProjectItemViewModel> RecentProjects
    {
        get => _recentProjects;
        private set => SetProperty(ref _recentProjects, value);
    }

    public async Task LoadAsync()
    {
        IsLoading = true;
        try
        {
            RecentProjects = WrapRecentProjects(await _backend.ListRecentProjectsAsync().ConfigureAwait(true));
            NotifyRecentProjectsChanged();
            StatusText = RecentProjects.Count == 0
                ? _displayNames.Text("ui.welcome.recent_empty_title")
                : _displayNames.Format("ui.welcome.recent_project_count", new Dictionary<string, string>
                {
                    ["count"] = RecentProjects.Count.ToString(),
                });
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

    private void NotifyRecentProjectsChanged()
    {
        OnPropertyChanged(nameof(HasRecentProjects));
        OnPropertyChanged(nameof(IsRecentEmpty));
        OnPropertyChanged(nameof(RecentCountText));
    }

    private async Task CreateProjectAsync()
    {
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
            RecentProjects = WrapRecentProjects(await _backend.ListRecentProjectsAsync().ConfigureAwait(true));
            NotifyRecentProjectsChanged();
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

            await OpenProjectRootAsync(root).ConfigureAwait(true);
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
        await DialogService.Current.ConfirmAsync(HelpDialogFactory.CreateFeedbackDialog(_displayNames)).ConfigureAwait(true);
    }

    private async Task OpenProjectRootAsync(string root) =>
        await OpenProjectRootForHostAsync(root).ConfigureAwait(true);

    /// <summary>供主窗口标题栏「切换项目」调用。</summary>
    public async Task OpenProjectRootForHostAsync(string root)
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
        RecentProjects = WrapRecentProjects(await _backend.ListRecentProjectsAsync().ConfigureAwait(true));
        NotifyRecentProjectsChanged();
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
