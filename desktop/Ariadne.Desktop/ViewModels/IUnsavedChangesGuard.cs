namespace Ariadne.Desktop.ViewModels;

public interface IUnsavedChangesGuard
{
    bool HasUnsavedChanges { get; }

    Task<bool> ConfirmLeaveIfNeededAsync();
}
