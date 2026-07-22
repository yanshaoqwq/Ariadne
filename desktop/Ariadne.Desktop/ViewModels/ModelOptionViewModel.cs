namespace Ariadne.Desktop.ViewModels;

public sealed class ModelOptionViewModel : ViewModelBase
{
    private string _capabilityLabel;

    public ModelOptionViewModel(string modelId, string capability, string capabilityLabel)
    {
        ModelId = modelId;
        Capability = capability;
        _capabilityLabel = capabilityLabel;
    }

    public string ModelId { get; }
    public string Capability { get; }
    public string CapabilityLabel
    {
        get => _capabilityLabel;
        set => SetProperty(ref _capabilityLabel, value);
    }
}
