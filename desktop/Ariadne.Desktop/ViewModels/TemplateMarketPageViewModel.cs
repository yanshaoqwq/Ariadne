using System.Collections.ObjectModel;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class TemplateMarketPageViewModel : ViewModelBase
{
    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private string _searchQuery = string.Empty;
    private string _statusText = string.Empty;
    private string _repositoryBaseUrl = string.Empty;
    private int _page;

    public TemplateMarketPageViewModel(DisplayNameService displayNames, IAriadneBackendClient backend)
    {
        _displayNames = displayNames;
        _backend = backend;
        Templates = new ObservableCollection<TemplateCardViewModel>();
        Tags = new ObservableCollection<TemplateTagViewModel>
        {
            CreateTag("ui.template.tag.novel"),
            CreateTag("ui.template.tag.worldbuilding"),
            CreateTag("ui.template.tag.outline"),
            CreateTag("ui.template.tag.review"),
            CreateTag("ui.template.tag.summary"),
        };
        SearchCommand = new RelayCommand(() => _ = SearchAsync());
        InstallFirstCommand = new RelayCommand(() => _ = InstallFirstAsync());
        LoadMoreCommand = new RelayCommand(() => _ = LoadMoreAsync());
        _ = LoadRepositoryAsync();
    }

    public string Title => _displayNames.Text("ui.template.title");

    public string OnlineSearchText => _displayNames.Text("ui.template.online_search");

    public string SearchPlaceholder => _displayNames.Text("ui.template.search.placeholder");

    public string SearchText => _displayNames.Text("ui.common.search");

    public string EmptyText => _displayNames.Text("ui.template.empty");

    public string ImportText => _displayNames.Text("ui.common.import");

    public string PermissionText => _displayNames.Text("ui.template.permission");

    public string DetailText => _displayNames.Text("ui.template.detail");

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

    public ObservableCollection<TemplateCardViewModel> Templates { get; }

    public ObservableCollection<TemplateTagViewModel> Tags { get; }

    public RelayCommand SearchCommand { get; }

    public RelayCommand InstallFirstCommand { get; }

    public RelayCommand LoadMoreCommand { get; }

    private TemplateTagViewModel CreateTag(string key)
    {
        var title = _displayNames.Text(key);
        return new TemplateTagViewModel(title, () =>
        {
            SearchQuery = title;
            _ = SearchAsync();
        });
    }

    private async Task LoadRepositoryAsync()
    {
        try
        {
            var settings = await _backend.GetTemplateRepositorySettingsAsync().ConfigureAwait(true);
            _repositoryBaseUrl = settings.BaseUrl;
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private async Task SearchAsync()
    {
        _page = 0;
        await SearchPageAsync(clear: true).ConfigureAwait(true);
    }

    private async Task LoadMoreAsync()
    {
        _page++;
        await SearchPageAsync(clear: false).ConfigureAwait(true);
    }

    private async Task SearchPageAsync(bool clear)
    {
        try
        {
            if (string.IsNullOrWhiteSpace(_repositoryBaseUrl))
            {
                await LoadRepositoryAsync().ConfigureAwait(true);
            }
            var results = await _backend.SearchTemplatesAsync(_repositoryBaseUrl, SearchQuery, _page).ConfigureAwait(true);
            if (clear)
            {
                Templates.Clear();
            }
            foreach (var item in results)
            {
                Templates.Add(new TemplateCardViewModel(
                    item,
                    () => _ = ShowDetailsAsync(item),
                    () => _ = InstallTemplateAsync(item)));
            }
            StatusText = Templates.Count == 0 ? EmptyText : $"{Templates.Count}";
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
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
        await InstallTemplateAsync(template.Summary).ConfigureAwait(true);
    }

    private async Task ShowDetailsAsync(TemplateSummary template)
    {
        try
        {
            var detail = await _backend.GetTemplateDetailAsync(_repositoryBaseUrl, template.Id).ConfigureAwait(true);
            StatusText = _displayNames.Format("ui.template.detail.version", new Dictionary<string, string>
            {
                ["version"] = detail.Version,
            });
            var message = detail.Name
                          + Environment.NewLine
                          + _displayNames.Format("ui.template.detail.version", new Dictionary<string, string>
                          {
                              ["version"] = detail.Version,
                          })
                          + Environment.NewLine
                          + Environment.NewLine
                          + _displayNames.Text("ui.template.permission_dialog.desc")
                          + Environment.NewLine
                          + TemplatePermissionSummary(detail);
            var dialog = new ConfirmDialogViewModel(
                _displayNames.Text("ui.template.detail"),
                message,
                new[]
                {
                    new DialogButton(_displayNames.Text("ui.common.close"), DialogButtonVariant.Primary, 0),
                })
            {
                CancelResultIndex = 0,
            };
            await DialogService.Current.ConfirmAsync(dialog).ConfigureAwait(true);
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private async Task InstallTemplateAsync(TemplateSummary template)
    {
        try
        {
            if (template.RequiresPermissions && !await ConfirmTemplatePermissionsAsync(template).ConfigureAwait(true))
            {
                StatusText = _displayNames.Text("ui.common.cancel");
                return;
            }

            var report = await _backend.InstallTemplateAsync(_repositoryBaseUrl, template.Id).ConfigureAwait(true);
            StatusText = _displayNames.Format("ui.template.imported", new Dictionary<string, string>
            {
                ["name"] = report.WorkflowId,
            });
        }
        catch (Exception ex)
        {
            StatusText = UserFacingError.Format(ex, _displayNames);
        }
    }

    private async Task<bool> ConfirmTemplatePermissionsAsync(TemplateSummary template)
    {
        var detail = await _backend.GetTemplateDetailAsync(_repositoryBaseUrl, template.Id).ConfigureAwait(true);
        var permissionSummary = TemplatePermissionSummary(detail);
        var message = _displayNames.Text("ui.template.permission_dialog.desc")
                      + Environment.NewLine
                      + Environment.NewLine
                      + permissionSummary;
        var dialog = new ConfirmDialogViewModel(
            _displayNames.Text("ui.template.permission_dialog.title"),
            message,
            new[]
            {
                new DialogButton(_displayNames.Text("ui.template.permission_dialog.confirm"), DialogButtonVariant.Primary, 0),
                new DialogButton(_displayNames.Text("ui.common.cancel"), DialogButtonVariant.Subtle, 1),
            })
        {
            CancelResultIndex = 1,
        };
        StatusText = _displayNames.Format("ui.template.detail.version", new Dictionary<string, string>
        {
            ["version"] = detail.Version,
        });
        return await DialogService.Current.ConfirmAsync(dialog).ConfigureAwait(true) == 0;
    }

    private string TemplatePermissionSummary(TemplateDetail detail)
    {
        var permissions = ExtractStringArray(detail.Manifest, "required_permissions");
        if (permissions.Count == 0)
        {
            permissions = ExtractStringArray(detail.Manifest, "permissions");
        }
        if (permissions.Count == 0)
        {
            return _displayNames.Text("ui.template.permission_dialog.empty");
        }
        return string.Join(Environment.NewLine, permissions.Select(permission => "- " + permission));
    }

    private static IReadOnlyList<string> ExtractStringArray(object? value, string key)
    {
        if (value is JsonElement element)
        {
            return ExtractStringArray(element, key);
        }
        return Array.Empty<string>();
    }

    private static IReadOnlyList<string> ExtractStringArray(JsonElement element, string key)
    {
        if (element.ValueKind == JsonValueKind.Object && element.TryGetProperty(key, out var property))
        {
            return ExtractStringArray(property);
        }
        return Array.Empty<string>();
    }

    private static IReadOnlyList<string> ExtractStringArray(JsonElement property)
    {
        if (property.ValueKind == JsonValueKind.Array)
        {
            return property.EnumerateArray()
                .Select(item => item.ValueKind == JsonValueKind.String ? item.GetString() : item.ToString())
                .Where(item => !string.IsNullOrWhiteSpace(item))
                .Select(item => item!)
                .ToArray();
        }
        if (property.ValueKind == JsonValueKind.String)
        {
            var value = property.GetString();
            return string.IsNullOrWhiteSpace(value) ? Array.Empty<string>() : new[] { value };
        }
        return Array.Empty<string>();
    }
}

public sealed class TemplateCardViewModel
{
    public TemplateCardViewModel(TemplateSummary summary, Action showDetails, Action install)
    {
        Summary = summary;
        Id = summary.Id;
        Name = summary.Name;
        RequiresPermissions = summary.RequiresPermissions;
        TagsText = string.Join(", ", summary.Tags);
        ShowDetailsCommand = new RelayCommand(showDetails);
        InstallCommand = new RelayCommand(install);
    }

    public TemplateSummary Summary { get; }
    public string Id { get; }
    public string Name { get; }
    public bool RequiresPermissions { get; }
    public string TagsText { get; }
    public RelayCommand ShowDetailsCommand { get; }
    public RelayCommand InstallCommand { get; }
}

public sealed class TemplateTagViewModel
{
    public TemplateTagViewModel(string title, Action select)
    {
        Title = title;
        SelectCommand = new RelayCommand(select);
    }

    public string Title { get; }
    public RelayCommand SelectCommand { get; }
}
