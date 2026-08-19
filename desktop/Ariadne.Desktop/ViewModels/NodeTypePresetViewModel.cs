using System.Collections.ObjectModel;
using Ariadne.Desktop.Backend;

namespace Ariadne.Desktop.ViewModels;

public sealed class NodeTypePresetViewModel : ViewModelBase
{
    private readonly Action _onChange;
    private string _displayName;
    private string _providerId;
    private string _modelId;
    private string? _modelAlias;
    private WorkflowModelOption? _selectedModelOption;
    private string _timeoutMs;
    private string _budgetUsd;
    private string _presetNodeModelLabel = string.Empty;
    private string _presetNodeTimeoutLabel = string.Empty;
    private string _presetNodeBudgetLabel = string.Empty;
    private string _presetAccessTitle = string.Empty;
    private string _presetToolsTitle = string.Empty;
    private ObservableCollection<WorkflowModelOption>? _availableLlmModelOptions;

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
        Action onChange,
        string? modelAlias = null)
    {
        NodeType = nodeType;
        DisplayNameKey = displayNameKey;
        _displayName = displayName;
        _providerId = providerId;
        _modelId = modelId;
        _modelAlias = string.IsNullOrWhiteSpace(modelAlias) ? null : modelAlias;
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

    // U178-B：5 项页面级文案 + 1 个共享选项列表在本条预设上的投影。
    // 预设行数 = 节点类型数（十余条），每条原先付 6 次祖先绑定。
    // 文案不走 {loc:Text}，理由同 ProviderModelEditorRow：设置页会切语言。
    public string PresetNodeModelLabel
    {
        get => _presetNodeModelLabel;
        internal set => SetProperty(ref _presetNodeModelLabel, value);
    }

    public string PresetNodeTimeoutLabel
    {
        get => _presetNodeTimeoutLabel;
        internal set => SetProperty(ref _presetNodeTimeoutLabel, value);
    }

    public string PresetNodeBudgetLabel
    {
        get => _presetNodeBudgetLabel;
        internal set => SetProperty(ref _presetNodeBudgetLabel, value);
    }

    public string PresetAccessTitle
    {
        get => _presetAccessTitle;
        internal set => SetProperty(ref _presetAccessTitle, value);
    }

    public string PresetToolsTitle
    {
        get => _presetToolsTitle;
        internal set => SetProperty(ref _presetToolsTitle, value);
    }

    /// <summary>
    /// U178-B：可选模型列表在本条预设上的投影。
    /// **共享页面 VM 的 AvailableLlmModelOptions 实例**——RebuildAvailableLlmModelOptions
    /// 是原地 Clear/Add，共享引用才能让所有预设行同步看到新选项。
    /// </summary>
    public ObservableCollection<WorkflowModelOption>? AvailableLlmModelOptions
    {
        get => _availableLlmModelOptions;
        internal set => SetProperty(ref _availableLlmModelOptions, value);
    }

    public string ProviderId => _providerId;
    public string? ModelAlias => _modelAlias;

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

            _modelAlias = value.IsAlias ? value.AliasId : null;
            _providerId = value.IsAlias ? string.Empty : value.ProviderId;
            _modelId = value.IsAlias ? string.Empty : value.ModelId;
            OnPropertyChanged(nameof(ModelAlias));
            OnPropertyChanged(nameof(ProviderId));
            OnPropertyChanged(nameof(ModelId));
            _onChange();
        }
    }

    public void RebindModelOptions(IEnumerable<WorkflowModelOption> options)
    {
        var optionArray = options.ToArray();
        WorkflowModelOption? selected;
        if (!string.IsNullOrWhiteSpace(_modelAlias))
        {
            selected = optionArray.FirstOrDefault(option =>
                string.Equals(option.AliasId, _modelAlias, StringComparison.Ordinal));
        }
        else
        {
            var candidates = optionArray
                .Where(option => !option.IsAlias
                                 && string.Equals(option.ModelId, _modelId, StringComparison.Ordinal))
                .Take(2)
                .ToArray();
            selected = string.IsNullOrWhiteSpace(_providerId)
                ? (candidates.Length == 1 ? candidates[0] : null)
                : candidates.FirstOrDefault(option =>
                    string.Equals(option.ProviderId, _providerId, StringComparison.Ordinal));
        }
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
        ModelAlias ?? string.Empty,
        ProviderId,
        ModelId,
        TimeoutMs,
        BudgetUsd,
        Permissions.Snapshot,
        string.Join(",", ToolControls.Select(tool => $"{tool.ToolId}:{tool.IsEnabled?.ToString() ?? "inherit"}")),
    });
}
