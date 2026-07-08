namespace Ariadne.Desktop.ViewModels;

public sealed class ProviderOptionViewModel : ViewModelBase
{
    public ProviderOptionViewModel(string providerId, string displayName, string keyStatus)
    {
        ProviderId = providerId;
        DisplayName = displayName;
        KeyStatus = keyStatus;
    }

    public string ProviderId { get; }

    public string DisplayName { get; }

    public string KeyStatus { get; }

    public string DisplayTitle => $"{DisplayName} ({ProviderId})";
}
