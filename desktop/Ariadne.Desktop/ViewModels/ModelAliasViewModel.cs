namespace Ariadne.Desktop.ViewModels;

public sealed class ModelAliasViewModel : ViewModelBase
{
    private readonly Action _onChange;
    private string _displayName;
    private string _targetProviderId;
    private string _targetModelId;
    private WorkflowModelOption? _selectedTargetOption;

    public ModelAliasViewModel(
        string aliasId,
        string displayNameKey,
        string displayName,
        string targetProviderId,
        string targetModelId,
        Action onChange)
    {
        AliasId = aliasId;
        DisplayNameKey = displayNameKey;
        _displayName = displayName;
        _targetProviderId = targetProviderId?.Trim() ?? string.Empty;
        _targetModelId = targetModelId?.Trim() ?? string.Empty;
        _onChange = onChange;
    }

    public string AliasId { get; }
    public string DisplayNameKey { get; }
    public string DisplayName { get => _displayName; set => SetProperty(ref _displayName, value); }
    public string TargetProviderId => _targetProviderId;
    public string TargetModelId => _targetModelId;
    public bool IsConfigured => !string.IsNullOrWhiteSpace(_targetProviderId)
                                && !string.IsNullOrWhiteSpace(_targetModelId);

    public WorkflowModelOption? SelectedTargetOption
    {
        get => _selectedTargetOption;
        set
        {
            if (!SetProperty(ref _selectedTargetOption, value) || value is null)
            {
                return;
            }

            _targetProviderId = value.IsInherited ? string.Empty : value.ProviderId;
            _targetModelId = value.IsInherited ? string.Empty : value.ModelId;
            OnPropertyChanged(nameof(TargetProviderId));
            OnPropertyChanged(nameof(TargetModelId));
            OnPropertyChanged(nameof(IsConfigured));
            _onChange();
        }
    }

    public void RebindTargetOptions(IEnumerable<WorkflowModelOption> options)
    {
        var selected = IsConfigured
            ? options.FirstOrDefault(option =>
                !option.IsAlias
                && string.Equals(option.ProviderId, _targetProviderId, StringComparison.Ordinal)
                && string.Equals(option.ModelId, _targetModelId, StringComparison.Ordinal))
            : options.FirstOrDefault(option => option.IsInherited);
        SetProperty(ref _selectedTargetOption, selected, nameof(SelectedTargetOption));
    }

    public string Snapshot => string.Join("|", AliasId, TargetProviderId, TargetModelId);
}
