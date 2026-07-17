namespace Ariadne.Desktop.ViewModels;

public interface IProjectDataReloadable
{
    Task ReloadProjectDataAsync(CancellationToken cancellationToken = default);

    void DeactivateProjectData();
}
