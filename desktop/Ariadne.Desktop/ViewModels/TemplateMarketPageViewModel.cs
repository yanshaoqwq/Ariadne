using System.Collections.ObjectModel;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class TemplateMarketPageViewModel : ViewModelBase
{
    private const int PageSize = 20;

    private enum SearchState
    {
        Idle,
        Loading,
        Results,
        Empty,
        Error,
        EndOfList,
    }

    private readonly DisplayNameService _displayNames;
    private readonly IAriadneBackendClient _backend;
    private string _searchQuery = string.Empty;
    private string _statusText = string.Empty;
    private string _repositoryBaseUrl = string.Empty;
    private int _page = -1;
    private bool _isBusy;
    private bool _hasMore;
    private SearchState _state = SearchState.Idle;
    private long _searchGeneration;
    private long _requestGeneration;
    private CancellationTokenSource? _requestCts;
    private bool _initialCatalogLoadStarted;

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
        InstallFirstCommand = new RelayCommand(() => _ = InstallFirstAsync(), () => Templates.Count > 0 && !IsBusy);
        LoadMoreCommand = new RelayCommand(() => _ = LoadMoreAsync(), () => CanLoadMore);
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

    public string LoadingText => _displayNames.Text("ui.template.loading");

    public string RetryText => _displayNames.Text("ui.template.retry");

    public string EndOfListText => _displayNames.Text("ui.template.end");

    public string RepositoryMissingText => _displayNames.Text("ui.template.repository_missing");

    public bool IsBusy => _isBusy;

    public bool IsIdle => _state == SearchState.Idle;

    public bool IsLoading => _state == SearchState.Loading;

    public bool IsEmpty => _state == SearchState.Empty;

    public bool IsError => _state == SearchState.Error;

    public bool IsEndOfList => _state == SearchState.EndOfList;

    public bool HasResults => Templates.Count > 0;

    public bool CanLoadMore => _hasMore && !IsBusy;

    public bool IsLoadMoreVisible => _hasMore;

    public bool CanInteract => !IsBusy;

    public string StatusText
    {
        get => _statusText;
        private set => SetProperty(ref _statusText, value);
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
        }, () => !IsBusy);
    }

    private async Task<string> LoadRepositoryAsync(CancellationToken cancellationToken)
    {
        if (!string.IsNullOrWhiteSpace(_repositoryBaseUrl))
        {
            return _repositoryBaseUrl;
        }

        var settings = await _backend.GetTemplateRepositorySettingsAsync(cancellationToken).ConfigureAwait(true);
        _repositoryBaseUrl = settings.BaseUrl;
        if (string.IsNullOrWhiteSpace(_repositoryBaseUrl))
        {
            throw new InvalidOperationException(RepositoryMissingText);
        }

        return _repositoryBaseUrl;
    }

    private async Task SearchAsync()
    {
        var searchGeneration = ++_searchGeneration;
        var query = SearchQuery;
        _page = -1;
        _hasMore = false;
        Templates.Clear();
        NotifyTemplateCollectionChanged();
        SetState(SearchState.Loading);
        StatusText = string.Empty;
        var (requestGeneration, cancellationToken) = BeginRequest();
        try
        {
            var baseUrl = await LoadRepositoryAsync(cancellationToken).ConfigureAwait(true);
            var results = await _backend
                .SearchTemplatesAsync(baseUrl, query, 0, cancellationToken)
                .ConfigureAwait(true);
            if (!IsCurrent(searchGeneration, requestGeneration))
            {
                return;
            }

            AppendResults(results);
            _page = 0;
            _hasMore = results.Count >= PageSize;
            SetState(results.Count == 0
                ? SearchState.Empty
                : _hasMore ? SearchState.Results : SearchState.EndOfList);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            if (IsCurrent(searchGeneration, requestGeneration))
            {
                SetState(SearchState.Idle);
            }
        }
        catch (Exception ex)
        {
            if (IsCurrent(searchGeneration, requestGeneration))
            {
                StatusText = UserFacingError.Format(ex, _displayNames);
                SetState(SearchState.Error);
            }
        }
        finally
        {
            FinishRequest(requestGeneration);
        }
    }

    private async Task LoadMoreAsync()
    {
        if (!CanLoadMore)
        {
            return;
        }

        var searchGeneration = _searchGeneration;
        var query = SearchQuery;
        var targetPage = _page + 1;
        var (requestGeneration, cancellationToken) = BeginRequest();
        try
        {
            var baseUrl = await LoadRepositoryAsync(cancellationToken).ConfigureAwait(true);
            var results = await _backend
                .SearchTemplatesAsync(baseUrl, query, targetPage, cancellationToken)
                .ConfigureAwait(true);
            if (!IsCurrent(searchGeneration, requestGeneration))
            {
                return;
            }

            AppendResults(results);
            _page = targetPage;
            _hasMore = results.Count >= PageSize;
            SetState(_hasMore ? SearchState.Results : SearchState.EndOfList);
            StatusText = string.Empty;
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            if (IsCurrent(searchGeneration, requestGeneration))
            {
                SetState(HasResults ? SearchState.Results : SearchState.Idle);
            }
        }
        catch (Exception ex)
        {
            if (IsCurrent(searchGeneration, requestGeneration))
            {
                StatusText = UserFacingError.Format(ex, _displayNames);
                SetState(SearchState.Error);
            }
        }
        finally
        {
            FinishRequest(requestGeneration);
        }
    }

    private void AppendResults(IReadOnlyList<TemplateSummary> results)
    {
        foreach (var item in results)
        {
            Templates.Add(new TemplateCardViewModel(
                item,
                ResolveDisplayText(item.Name),
                string.Join(", ", item.Tags.Select(ResolveDisplayText)),
                () => _ = ShowDetailsAsync(item),
                () => _ = InstallTemplateAsync(item)));
        }
        NotifyTemplateCollectionChanged();
    }

    private (long RequestGeneration, CancellationToken CancellationToken) BeginRequest()
    {
        _requestCts?.Cancel();
        _requestCts?.Dispose();
        _requestCts = new CancellationTokenSource();
        _isBusy = true;
        OnPropertyChanged(nameof(IsBusy));
        OnPropertyChanged(nameof(CanInteract));
        LoadMoreCommand.NotifyCanExecuteChanged();
        InstallFirstCommand.NotifyCanExecuteChanged();
        return (++_requestGeneration, _requestCts.Token);
    }

    private bool IsCurrent(long searchGeneration, long requestGeneration)
    {
        return searchGeneration == _searchGeneration
            && requestGeneration == _requestGeneration;
    }

    private void FinishRequest(long requestGeneration)
    {
        if (requestGeneration != _requestGeneration)
        {
            return;
        }

        _isBusy = false;
        OnPropertyChanged(nameof(IsBusy));
        OnPropertyChanged(nameof(CanInteract));
        LoadMoreCommand.NotifyCanExecuteChanged();
        InstallFirstCommand.NotifyCanExecuteChanged();
        _requestCts?.Dispose();
        _requestCts = null;
    }

    private void SetState(SearchState state)
    {
        if (_state == state)
        {
            return;
        }

        _state = state;
        OnPropertyChanged(nameof(IsIdle));
        OnPropertyChanged(nameof(IsLoading));
        OnPropertyChanged(nameof(IsEmpty));
        OnPropertyChanged(nameof(IsError));
        OnPropertyChanged(nameof(IsEndOfList));
        OnPropertyChanged(nameof(IsLoadMoreVisible));
        LoadMoreCommand.NotifyCanExecuteChanged();
    }

    private void NotifyTemplateCollectionChanged()
    {
        OnPropertyChanged(nameof(HasResults));
        InstallFirstCommand.NotifyCanExecuteChanged();
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
            var message = ResolveDisplayText(detail.Name)
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
                ["name"] = ResolveDisplayText(template.Name),
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

    internal Task SearchForTestsAsync() => SearchAsync();

    internal Task LoadMoreForTestsAsync() => LoadMoreAsync();

    internal async Task EnsureInitialCatalogLoadedAsync()
    {
        if (_initialCatalogLoadStarted)
        {
            return;
        }
        _initialCatalogLoadStarted = true;
        await SearchAsync().ConfigureAwait(true);
    }

    private string ResolveDisplayText(string value) => value.StartsWith("ui.", StringComparison.Ordinal)
        ? _displayNames.Text(value)
        : value;
}

public sealed class TemplateCardViewModel
{
    public TemplateCardViewModel(
        TemplateSummary summary,
        string displayName,
        string tagsText,
        Action showDetails,
        Action install)
    {
        Summary = summary;
        Id = summary.Id;
        Name = displayName;
        RequiresPermissions = summary.RequiresPermissions;
        TagsText = tagsText;
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
    public TemplateTagViewModel(string title, Action select, Func<bool>? canSelect = null)
    {
        Title = title;
        SelectCommand = new RelayCommand(select, canSelect);
    }

    public string Title { get; }
    public RelayCommand SelectCommand { get; }
}
