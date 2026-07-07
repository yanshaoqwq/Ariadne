namespace Ariadne.Desktop.ViewModels;

public sealed class NodeTypePresetViewModel : ViewModelBase
{
    private readonly Action _onChange;
    private string _displayName;
    private string _modelId;
    private string _timeoutMs;
    private string _budgetUsd;

    public NodeTypePresetViewModel(
        string nodeType,
        string displayNameKey,
        string displayName,
        string modelId,
        string timeoutMs,
        string budgetUsd,
        Action onChange)
    {
        NodeType = nodeType;
        DisplayNameKey = displayNameKey;
        _displayName = displayName;
        _modelId = modelId;
        _timeoutMs = timeoutMs;
        _budgetUsd = budgetUsd;
        _onChange = onChange;
    }

    public string NodeType { get; }
    public string DisplayNameKey { get; }
    public string DisplayName { get => _displayName; set => SetProperty(ref _displayName, value); }

    public string ModelId
    {
        get => _modelId;
        set { if (SetProperty(ref _modelId, value)) _onChange(); }
    }

    public string TimeoutMs
    {
        get => _timeoutMs;
        set { if (SetProperty(ref _timeoutMs, value)) _onChange(); }
    }

    public string BudgetUsd
    {
        get => _budgetUsd;
        set { if (SetProperty(ref _budgetUsd, value)) _onChange(); }
    }

    public string Snapshot => $"{NodeType}:{ModelId}:{TimeoutMs}:{BudgetUsd}";
}
