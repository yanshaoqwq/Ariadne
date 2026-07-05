using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class WelcomeViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private IReadOnlyList<RecentProjectEntry> _recentProjects = Array.Empty<RecentProjectEntry>();
    private string _statusText;
    private bool _isLoading;

    public WelcomeViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        _statusText = displayNames.Text("ui.common.loading");
    }

    public string BrandName => _displayNames.Text("ui.brand.name");

    public string Subtitle => _displayNames.Text("ui.welcome.subtitle");

    public string RecentProjectsTitle => _displayNames.Text("ui.welcome.recent_projects");

    public string CreateProjectText => _displayNames.Text("ui.layout.create_project");

    public string OpenProjectText => _displayNames.Text("ui.layout.open_project");

    public string TutorialText => _displayNames.Text("ui.settings.index.tutorial");

    public string FeedbackText => _displayNames.Text("ui.layout.feedback");

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

    public IReadOnlyList<RecentProjectEntry> RecentProjects
    {
        get => _recentProjects;
        private set => SetProperty(ref _recentProjects, value);
    }

    public async Task LoadAsync()
    {
        IsLoading = true;
        try
        {
            RecentProjects = await _backend.ListRecentProjectsAsync().ConfigureAwait(true);
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
}
