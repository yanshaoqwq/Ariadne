namespace Ariadne.Desktop.ViewModels;

public sealed class ModelOptionViewModel : ViewModelBase
{
    public ModelOptionViewModel(string modelId, string capability)
    {
        ModelId = modelId;
        Capability = capability;
    }

    public string ModelId { get; }
    public string Capability { get; }
}
