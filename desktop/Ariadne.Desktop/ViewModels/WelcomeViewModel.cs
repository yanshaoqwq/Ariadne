using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class WelcomeViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private Func<Task<string?>> _pickProjectFolder;
    private IReadOnlyList<RecentProjectItemViewModel> _recentProjects = Array.Empty<RecentProjectItemViewModel>();
    private string _statusText;
    private bool _isLoading;

    public WelcomeViewModel(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        Func<Task<string?>>? pickProjectFolder = null)
    {
        _displayNames = displayNames;
        _backend = backend;
        _pickProjectFolder = pickProjectFolder ?? (() => Task.FromResult<string?>(null));
        _statusText = displayNames.Text("ui.common.loading");
        CreateProjectCommand = new RelayCommand(() => _ = CreateProjectAsync());
        OpenProjectCommand = new RelayCommand(() => _ = OpenProjectAsync());
        TutorialCommand = new RelayCommand(() => StatusText = TutorialText);
        FeedbackCommand = new RelayCommand(() => StatusText = FeedbackText);
    }

    public string BrandName => _displayNames.Text("ui.brand.name");

    public string Subtitle => _displayNames.Text("ui.welcome.subtitle");

    public string RecentProjectsTitle => _displayNames.Text("ui.welcome.recent_projects");

    public string CreateProjectText => _displayNames.Text("ui.layout.create_project");

    public string OpenProjectText => _displayNames.Text("ui.layout.open_project");

    public string TutorialText => _displayNames.Text("ui.settings.index.tutorial");

    public string FeedbackText => _displayNames.Text("ui.layout.feedback");

    public RelayCommand CreateProjectCommand { get; }

    public RelayCommand OpenProjectCommand { get; }

    public RelayCommand TutorialCommand { get; }

    public RelayCommand FeedbackCommand { get; }

    public void SetProjectFolderPicker(Func<Task<string?>> picker)
    {
        _pickProjectFolder = picker;
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
            StatusText = RecentProjects.Count == 0
                ? _displayNames.Text("ui.common.no_current_project")
                : _displayNames.Format("ui.welcome.recent_project_count", new Dictionary<string, string>
                {
                    ["count"] = RecentProjects.Count.ToString(),
                });
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
        finally
        {
            IsLoading = false;
        }
    }

    private async Task CreateProjectAsync()
    {
        IsLoading = true;
        try
        {
            var root = await _pickProjectFolder().ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(root))
            {
                StatusText = _displayNames.Text("ui.common.cancel");
                return;
            }
            var report = await _backend.CreateProjectAsync(root).ConfigureAwait(true);
            StatusText = report.ProjectRoot;
            RecentProjects = WrapRecentProjects(await _backend.ListRecentProjectsAsync().ConfigureAwait(true));
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
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
            var root = RecentProjects.FirstOrDefault()?.ProjectRoot ?? await _pickProjectFolder().ConfigureAwait(true);
            if (string.IsNullOrWhiteSpace(root))
            {
                StatusText = _displayNames.Text("ui.common.cancel");
                return;
            }
            await OpenProjectRootAsync(root).ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
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

    private async Task OpenProjectRootAsync(string root)
    {
        RecentProjects = WrapRecentProjects(await _backend.OpenProjectAsync(root).ConfigureAwait(true));
        StatusText = root;
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
