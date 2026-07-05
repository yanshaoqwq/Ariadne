using System.Collections.ObjectModel;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// 模板市场页 ViewModel：搜索 + 标签 + 卡片网格 + 权限确认。
/// 本轮只承载视觉骨架文案，后端接线（search_templates / install_template 等）留待交互阶段。
public sealed class TemplateMarketPageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private string _searchQuery = string.Empty;
    private string _statusText = string.Empty;
    private string _repositoryBaseUrl = string.Empty;

    public TemplateMarketPageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        Templates = new ObservableCollection<TemplateSummary>();
        SearchCommand = new RelayCommand(() => _ = SearchAsync());
        InstallFirstCommand = new RelayCommand(() => _ = InstallFirstAsync());
        LoadMoreCommand = new RelayCommand(() => _ = SearchAsync());
        _ = LoadRepositoryAsync();
    }

    public string Title => _displayNames.Text("ui.template.title");

    public string OnlineSearchText => _displayNames.Text("ui.template.online_search");

    public string SearchPlaceholder => _displayNames.Text("ui.template.search.placeholder");

    public string SearchText => _displayNames.Text("ui.common.search");

    public string EmptyText => _displayNames.Text("ui.template.empty");

    public string ImportText => _displayNames.Text("ui.common.import");

    public string PermissionText => _displayNames.Text("ui.template.permission");

    public string BackToTopText => _displayNames.Text("ui.common.back_to_top");

    public string LoadMoreText => _displayNames.Text("ui.common.load_more");

    public string StatusText
    {
        get => _statusText;
        set => SetProperty(ref _statusText, value);
    }

    public string SearchQuery
    {
        get => _searchQuery;
        set => SetProperty(ref _searchQuery, value);
    }

    public ObservableCollection<TemplateSummary> Templates { get; }

    public RelayCommand SearchCommand { get; }

    public RelayCommand InstallFirstCommand { get; }

    public RelayCommand LoadMoreCommand { get; }

    private async Task LoadRepositoryAsync()
    {
        try
        {
            var settings = await _backend.GetTemplateRepositorySettingsAsync().ConfigureAwait(true);
            _repositoryBaseUrl = settings.BaseUrl;
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task SearchAsync()
    {
        try
        {
            if (string.IsNullOrWhiteSpace(_repositoryBaseUrl))
            {
                await LoadRepositoryAsync().ConfigureAwait(true);
            }
            var results = await _backend.SearchTemplatesAsync(_repositoryBaseUrl, SearchQuery).ConfigureAwait(true);
            Templates.Clear();
            foreach (var item in results)
            {
                Templates.Add(item);
            }
            StatusText = Templates.Count == 0 ? EmptyText : $"{Templates.Count}";
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }

    private async Task InstallFirstAsync()
    {
        var template = Templates.FirstOrDefault();
        if (template is null)
        {
            StatusText = EmptyText;
            return;
        }
        try
        {
            var report = await _backend.InstallTemplateAsync(_repositoryBaseUrl, template.Id).ConfigureAwait(true);
            StatusText = _displayNames.Format("ui.template.imported", new Dictionary<string, string>
            {
                ["name"] = report.WorkflowId,
            });
        }
        catch (Exception ex)
        {
            StatusText = ex.Message;
        }
    }
}
