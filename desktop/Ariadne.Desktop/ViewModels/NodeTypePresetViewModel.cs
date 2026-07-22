using System.Collections.ObjectModel;
using Ariadne.Desktop.Backend;

namespace Ariadne.Desktop.ViewModels;

public sealed class NodeTypePresetViewModel : ViewModelBase
{
    private readonly Action _onChange;
    private string _displayName;
    private string _providerId;
    private string _modelId;
    private WorkflowModelOption? _selectedModelOption;
    private string _timeoutMs;
    private string _budgetUsd;

    public NodeTypePresetViewModel(
        string nodeType,
        string displayNameKey,
        string displayName,
        string providerId,
        string modelId,
        string timeoutMs,
        string budgetUsd,
        PermissionPolicy? permissionPolicy,
        PermissionPolicy inheritedPermissionPolicy,
        IReadOnlyDictionary<string, bool?> toolControls,
        Func<string, string> toolLabel,
        Action onChange)
    {
        NodeType = nodeType;
        DisplayNameKey = displayNameKey;
        _displayName = displayName;
        _providerId = providerId;
        _modelId = modelId;
        _timeoutMs = timeoutMs;
        _budgetUsd = budgetUsd;
        _onChange = onChange;
        Permissions = new PermissionScopeProfileViewModel(
            nodeType,
            displayName,
            permissionPolicy,
            inheritedPermissionPolicy,
            onChange);
        ToolControls = new ObservableCollection<ToolControlItemViewModel>();
        foreach (var toolId in new[] { "find", "search", "web-search", "register", "write" }
                     .Concat(toolControls.Keys)
                     .Distinct(StringComparer.Ordinal))
        {
            toolControls.TryGetValue(toolId, out var enabled);
            ToolControls.Add(new ToolControlItemViewModel(
                toolId,
                toolLabel(toolId),
                enabled,
                ToolControlItemViewModel.IsDangerToolId(toolId),
                canInherit: true,
                markDirty: onChange));
        }
    }

    public string NodeType { get; }
    public string DisplayNameKey { get; }
    public string DisplayName { get => _displayName; set => SetProperty(ref _displayName, value); }
    public PermissionScopeProfileViewModel Permissions { get; }
    public ObservableCollection<ToolControlItemViewModel> ToolControls { get; }

    public string ProviderId => _providerId;

    public string ModelId
    {
        get => _modelId;
        set { if (SetProperty(ref _modelId, value)) _onChange(); }
    }

    public WorkflowModelOption? SelectedModelOption
    {
        get => _selectedModelOption;
        set
        {
            if (!SetProperty(ref _selectedModelOption, value) || value is null)
            {
                return;
            }

            _providerId = value.ProviderId;
            _modelId = value.ModelId;
            OnPropertyChanged(nameof(ProviderId));
            OnPropertyChanged(nameof(ModelId));
            _onChange();
        }
    }

    public void RebindModelOptions(IEnumerable<WorkflowModelOption> options)
    {
        var candidates = options
            .Where(option => string.Equals(option.ModelId, _modelId, StringComparison.Ordinal))
            .Take(2)
            .ToArray();
        var selected = string.IsNullOrWhiteSpace(_providerId)
            ? (candidates.Length == 1 ? candidates[0] : null)
            : candidates.FirstOrDefault(option =>
                string.Equals(option.ProviderId, _providerId, StringComparison.Ordinal));
        SetProperty(ref _selectedModelOption, selected, nameof(SelectedModelOption));
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

    public string Snapshot => string.Join("|", new[]
    {
        NodeType,
        ProviderId,
        ModelId,
        TimeoutMs,
        BudgetUsd,
        Permissions.Snapshot,
        string.Join(",", ToolControls.Select(tool => $"{tool.ToolId}:{tool.IsEnabled?.ToString() ?? "inherit"}")),
    });
}
