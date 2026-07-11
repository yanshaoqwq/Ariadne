namespace Ariadne.Desktop.ViewModels;

public enum WorkflowLoadState
{
    NoProject,
    Loading,
    Loaded,
    LoadFailed,
}

public static class WorkflowLoadGuard
{
    public static bool CanPersist(bool hasProjectRoot, WorkflowLoadState state)
    {
        return hasProjectRoot && state == WorkflowLoadState.Loaded;
    }
}
