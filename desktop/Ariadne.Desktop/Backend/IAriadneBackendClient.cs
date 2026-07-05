namespace Ariadne.Desktop.Backend;

public interface IAriadneBackendClient
{
    Task<T?> InvokeAsync<T>(
        string method,
        object? parameters = null,
        CancellationToken cancellationToken = default);

    Task<IReadOnlyList<RecentProjectEntry>> ListRecentProjectsAsync(CancellationToken cancellationToken = default);

    Task<AppStatus?> GetAppStatusAsync(CancellationToken cancellationToken = default);

    Task<CurrentProjectStatus?> GetCurrentProjectAsync(CancellationToken cancellationToken = default);
}
