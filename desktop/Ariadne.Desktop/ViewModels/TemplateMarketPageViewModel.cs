using System.Collections.ObjectModel;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

public sealed class TemplateMarketPageViewModel : PageViewModelBase, ILocalizedUiAware, IProjectDataReloadable
{
    private const int PageSize = 20;
    private const string OfficialRepositoryUrl = "ariadne://official-templates/v1";

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
    private readonly Func<Task> _reloadOtherProjectPages;
    private readonly Func<Task<bool>> _confirmProjectReload;
    private string _searchQuery = string.Empty;
    private string _repositoryBaseUrl = string.Empty;
    private int _page = -1;
    private bool _isBusy;
    private bool _hasMore;
    private SearchState _state = SearchState.Idle;
    private long _searchGeneration;
    private long _requestGeneration;
    private CancellationTokenSource? _requestCts;
    private bool _initialCatalogLoadStarted;

    /// <param name="reloadOtherProjectPages">
    /// 装完模板后重载**除本页以外**的已缓存项目页（U207-D/U198-A）。
    /// 后端安装时已把模板图并进项目画布并落盘，画布页却还捧着装模板前的那份图；
    /// 不通知它，作者切回画布只会看到空画布 + 空态引导，判定「导入失败」再点一次。
    /// 传 null 表示无壳独立使用（单元测试），此时不通知任何人。
    /// ⚠️ 绝不能传「连本页一起重载」的版本：本页的 ReloadProjectDataAsync 是换项目语义，
    /// 会清空目录列表、抹掉成功文案，并让本次安装请求的 FinishRequest 被当成过期请求
    /// 跳过 ⇒ 整页永久卡在忙碌态。
    /// </param>
    /// <param name="confirmProjectReload">
    /// 安装前的未保存改动闸门。安装会重写磁盘上的项目画布，随后本页要求画布页重载——
    /// 没有这道闸门，作者画布上未保存的编辑会被静默丢弃（而不是像 U198-A 描述的那样
    /// 撞 expected_revision 冲突）。走同意即可先保存再合并，一点不丢。
    /// </param>
    public TemplateMarketPageViewModel(
        DisplayNameService displayNames,
        IAriadneBackendClient backend,
        Func<Task>? reloadOtherProjectPages = null,
        Func<Task<bool>>? confirmProjectReload = null)
    {
        _displayNames = displayNames;
        _backend = backend;
        _reloadOtherProjectPages = reloadOtherProjectPages ?? (() => Task.CompletedTask);
        _confirmProjectReload = confirmProjectReload ?? (() => Task.FromResult(true));
        Templates = new ObservableCollection<TemplateCardViewModel>();
        Tags = new ObservableCollection<TemplateTagViewModel>
        {
            CreateTag("ui.template.tag.novel"),
            CreateTag("ui.template.tag.worldbuilding"),
            CreateTag("ui.template.tag.outline"),
            CreateTag("ui.template.tag.review"),
            CreateTag("ui.template.tag.summary"),
        };
        SearchCommand = new RelayCommand(() => _ = SearchAsync(), () => !IsBusy);
        LoadMoreCommand = new RelayCommand(() => _ = LoadMoreAsync(), () => CanLoadMore);
    }

    public string Title => _displayNames.Text("ui.template.title");

    public string CatalogSourceText => _displayNames.Text(
        string.IsNullOrWhiteSpace(_repositoryBaseUrl)
            ? "ui.template.catalog"
            : string.Equals(_repositoryBaseUrl, OfficialRepositoryUrl, StringComparison.OrdinalIgnoreCase)
                ? "ui.template.catalog.builtin"
                : "ui.template.catalog.custom");

    public string SearchPlaceholder => _displayNames.Text("ui.template.search.placeholder");

    public string SearchText => _displayNames.Text("ui.common.search");

    public string EmptyText => _displayNames.Text("ui.template.empty");

    public string ImportText => _displayNames.Text("ui.common.import");

    public string PermissionText => _displayNames.Text("ui.template.permission");

    public string DetailText => _displayNames.Text("ui.template.detail");

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

    public string SearchQuery
    {
        get => _searchQuery;
        set => SetProperty(ref _searchQuery, value);
    }

    public ObservableCollection<TemplateCardViewModel> Templates { get; }

    public ObservableCollection<TemplateTagViewModel> Tags { get; }

    public RelayCommand SearchCommand { get; }

    public RelayCommand LoadMoreCommand { get; }

    private TemplateTagViewModel CreateTag(string key)
    {
        var title = _displayNames.Text(key);
        return new TemplateTagViewModel(key, title, tag =>
        {
            tag.IsSelected = !tag.IsSelected;
            _ = SearchAsync();
        }, () => !IsBusy);
    }

    public void RefreshLocalizedUi()
    {
        foreach (var tag in Tags)
        {
            tag.Title = _displayNames.Text(tag.DisplayNameKey);
        }
        OnPropertyChanged(string.Empty);
    }

    private async Task<string> LoadRepositoryAsync(
        CancellationToken cancellationToken,
        bool refresh)
    {
        if (!refresh && !string.IsNullOrWhiteSpace(_repositoryBaseUrl))
        {
            return _repositoryBaseUrl;
        }

        var settings = await _backend.GetTemplateRepositorySettingsAsync(cancellationToken).ConfigureAwait(true);
        _repositoryBaseUrl = settings.BaseUrl;
        OnPropertyChanged(nameof(CatalogSourceText));
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
        var tags = SelectedTags();
        _page = -1;
        _hasMore = false;
        Templates.Clear();
        NotifyTemplateCollectionChanged();
        SetState(SearchState.Loading);
        StatusText = string.Empty;
        var (requestGeneration, cancellationToken) = BeginRequest();
        try
        {
            var baseUrl = await LoadRepositoryAsync(cancellationToken, refresh: true).ConfigureAwait(true);
            var results = await _backend
                .SearchTemplatesAsync(baseUrl, query, tags, 0, cancellationToken)
                .ConfigureAwait(true);
            if (!IsCurrent(searchGeneration, requestGeneration))
            {
                return;
            }

            AppendResults(results, baseUrl);
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
                StatusText = ReportFailure(ex, _displayNames);
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
        var tags = SelectedTags();
        var targetPage = _page + 1;
        var (requestGeneration, cancellationToken) = BeginRequest();
        try
        {
            var baseUrl = await LoadRepositoryAsync(cancellationToken, refresh: false).ConfigureAwait(true);
            var results = await _backend
                .SearchTemplatesAsync(baseUrl, query, tags, targetPage, cancellationToken)
                .ConfigureAwait(true);
            if (!IsCurrent(searchGeneration, requestGeneration))
            {
                return;
            }

            AppendResults(results, baseUrl);
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
                StatusText = ReportFailure(ex, _displayNames);
                SetState(SearchState.Error);
            }
        }
        finally
        {
            FinishRequest(requestGeneration);
        }
    }

    private void AppendResults(IReadOnlyList<TemplateSummary> results, string repositoryBaseUrl)
    {
        foreach (var item in results)
        {
            Templates.Add(new TemplateCardViewModel(
                item,
                repositoryBaseUrl,
                ResolveDisplayText(item.Name),
                string.Join(", ", item.Tags.Select(ResolveDisplayText)),
                () => _ = ShowDetailsAsync(repositoryBaseUrl, item),
                () => _ = InstallTemplateAsync(repositoryBaseUrl, item),
                CanInstallTemplate));
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
        SearchCommand.NotifyCanExecuteChanged();
        foreach (var tag in Tags)
        {
            tag.SelectCommand.NotifyCanExecuteChanged();
        }
        LoadMoreCommand.NotifyCanExecuteChanged();
        NotifyTemplateCommandsChanged();
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
        SearchCommand.NotifyCanExecuteChanged();
        foreach (var tag in Tags)
        {
            tag.SelectCommand.NotifyCanExecuteChanged();
        }
        LoadMoreCommand.NotifyCanExecuteChanged();
        NotifyTemplateCommandsChanged();
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
        NotifyTemplateCommandsChanged();
    }

    private async Task ShowDetailsAsync(string repositoryBaseUrl, TemplateSummary template)
    {
        var (requestGeneration, cancellationToken) = BeginRequest();
        try
        {
            var detail = await _backend
                .GetTemplateDetailAsync(repositoryBaseUrl, template.Id, cancellationToken)
                .ConfigureAwait(true);
            if (requestGeneration != _requestGeneration)
            {
                return;
            }
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
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch (Exception ex)
        {
            if (requestGeneration == _requestGeneration)
            {
                StatusText = ReportFailure(ex, _displayNames);
            }
        }
        finally
        {
            FinishRequest(requestGeneration);
        }
    }

    private async Task InstallTemplateAsync(string repositoryBaseUrl, TemplateSummary template)
    {
        var (requestGeneration, cancellationToken) = BeginRequest();
        try
        {
            var project = await _backend.GetCurrentProjectAsync(cancellationToken).ConfigureAwait(true);
            if (project is null || string.IsNullOrWhiteSpace(project.ProjectRoot))
            {
                throw new InvalidOperationException(_displayNames.Text("ui.empty.need_project.title"));
            }
            var expectedProjectRoot = project.ProjectRoot;
            if (template.RequiresPermissions
                && !await ConfirmTemplatePermissionsAsync(
                    repositoryBaseUrl,
                    template,
                    cancellationToken).ConfigureAwait(true))
            {
                StatusText = _displayNames.Text("ui.common.cancel");
                return;
            }

            // 安装会重写磁盘上的项目画布，之后画布页必须重载才能看见新节点；
            // 重载会覆盖画布页内存里的图，所以先让作者处置未保存改动（保存 / 丢弃 / 取消）。
            if (!await _confirmProjectReload().ConfigureAwait(true))
            {
                StatusText = _displayNames.Text("ui.common.cancel");
                return;
            }

            await _backend.InstallTemplateAsync(
                repositoryBaseUrl,
                template.Id,
                expectedProjectRoot,
                cancellationToken).ConfigureAwait(true);
            if (requestGeneration != _requestGeneration)
            {
                return;
            }
            StatusText = _displayNames.Format("ui.template.imported", new Dictionary<string, string>
            {
                ["name"] = ResolveDisplayText(template.Name),
            });
            // U207-D：判据是「切到画布页节点可见」，不是「不再撞 expected_revision 冲突」——
            // 画布永远空着时后者也满足。所以这一步必须真的把新画布拉回来。
            await _reloadOtherProjectPages().ConfigureAwait(true);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
        }
        catch (Exception ex)
        {
            if (requestGeneration == _requestGeneration)
            {
                StatusText = ReportFailure(ex, _displayNames);
            }
        }
        finally
        {
            FinishRequest(requestGeneration);
        }
    }

    private async Task<bool> ConfirmTemplatePermissionsAsync(
        string repositoryBaseUrl,
        TemplateSummary template,
        CancellationToken cancellationToken)
    {
        var detail = await _backend
            .GetTemplateDetailAsync(repositoryBaseUrl, template.Id, cancellationToken)
            .ConfigureAwait(true);
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
        return string.Join(Environment.NewLine, permissions.Select(permission => "- " + ResolvePermissionText(permission)));
    }

    private string ResolvePermissionText(string permission)
    {
        var key = permission.Trim().ToLowerInvariant() switch
        {
            "http_skill" => "ui.template.permission.http_skill",
            "network" or "wasm_network" => "ui.template.permission.network",
            "web_search" => "ui.template.permission.web_search",
            "secret_read" => "ui.template.permission.secret_read",
            "filesystem_read" => "ui.template.permission.filesystem_read",
            "filesystem_write" => "ui.template.permission.filesystem_write",
            "workflow_run" => "ui.template.permission.workflow_run",
            _ => string.Empty,
        };
        return string.IsNullOrEmpty(key)
            ? _displayNames.Format(
                "ui.template.permission.compatibility",
                new Dictionary<string, string> { ["permission"] = permission })
            : _displayNames.Text(key);
    }

    private IReadOnlyList<string> SelectedTags() => Tags
        .Where(tag => tag.IsSelected)
        .Select(tag => tag.DisplayNameKey)
        .ToArray();

    private bool CanInstallTemplate() => _backend.HasProjectRoot && !IsBusy;

    private void NotifyTemplateCommandsChanged()
    {
        foreach (var template in Templates)
        {
            template.InstallCommand.NotifyCanExecuteChanged();
        }
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

    internal Task InstallForTestsAsync(TemplateCardViewModel template) =>
        InstallTemplateAsync(template.RepositoryBaseUrl, template.Summary);

    internal string PermissionSummaryForTests(TemplateDetail detail) =>
        TemplatePermissionSummary(detail);

    internal async Task EnsureInitialCatalogLoadedAsync()
    {
        if (_initialCatalogLoadStarted)
        {
            return;
        }
        _initialCatalogLoadStarted = true;
        await SearchAsync().ConfigureAwait(true);
    }

    public Task ReloadProjectDataAsync(CancellationToken cancellationToken = default)
    {
        cancellationToken.ThrowIfCancellationRequested();
        // 本页跨项目保留实例，而仓库地址按项目配置，缓存目录属于上一个项目。
        ResetCatalogCache();
        NotifyTemplateCommandsChanged();
        return Task.CompletedTask;
    }

    public void DeactivateProjectData()
    {
        ResetCatalogCache();
        NotifyTemplateCommandsChanged();
    }

    /// <summary>
    /// 丢弃与具体项目绑定的目录缓存，使下次进入本页重新按当前项目的仓库地址检索。
    /// </summary>
    private void ResetCatalogCache()
    {
        // 作废在途请求，避免旧项目的结果落进新项目的列表。
        _searchGeneration++;
        _requestGeneration++;
        _requestCts?.Cancel();
        _requestCts = null;
        _initialCatalogLoadStarted = false;
        _repositoryBaseUrl = string.Empty;
        _page = -1;
        _hasMore = false;
        Templates.Clear();
        SetState(SearchState.Idle);
        StatusText = string.Empty;
        OnPropertyChanged(nameof(CatalogSourceText));
        OnPropertyChanged(nameof(CanLoadMore));
    }

    private string ResolveDisplayText(string value) => value.StartsWith("ui.", StringComparison.Ordinal)
        ? _displayNames.Text(value)
        : value;
}

public sealed class TemplateCardViewModel
{
    public TemplateCardViewModel(
        TemplateSummary summary,
        string repositoryBaseUrl,
        string displayName,
        string tagsText,
        Action showDetails,
        Action install,
        Func<bool> canInstall)
    {
        Summary = summary;
        RepositoryBaseUrl = repositoryBaseUrl;
        Id = summary.Id;
        Name = displayName;
        RequiresPermissions = summary.RequiresPermissions;
        TagsText = tagsText;
        ShowDetailsCommand = new RelayCommand(showDetails);
        InstallCommand = new RelayCommand(install, canInstall);
    }

    public TemplateSummary Summary { get; }
    public string RepositoryBaseUrl { get; }
    public string Id { get; }
    public string Name { get; }
    public bool RequiresPermissions { get; }
    public string TagsText { get; }
    public RelayCommand ShowDetailsCommand { get; }
    public RelayCommand InstallCommand { get; }
}

public sealed class TemplateTagViewModel : ViewModelBase
{
    private string _title;
    private bool _isSelected;

    public TemplateTagViewModel(
        string displayNameKey,
        string title,
        Action<TemplateTagViewModel> select,
        Func<bool>? canSelect = null)
    {
        DisplayNameKey = displayNameKey;
        _title = title;
        SelectCommand = new RelayCommand(() => select(this), canSelect);
    }

    public string DisplayNameKey { get; }
    public string Title { get => _title; set => SetProperty(ref _title, value); }
    public bool IsSelected { get => _isSelected; set => SetProperty(ref _isSelected, value); }
    public RelayCommand SelectCommand { get; }
}
