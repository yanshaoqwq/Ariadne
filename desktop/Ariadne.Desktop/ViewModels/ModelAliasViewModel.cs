using System.Collections.ObjectModel;

namespace Ariadne.Desktop.ViewModels;

public sealed class ModelAliasViewModel : ViewModelBase
{
    private readonly Action _onChange;
    private string _displayName;
    private string _targetProviderId;
    private string _targetModelId;
    private WorkflowModelOption? _selectedTargetOption;
    private ObservableCollection<WorkflowModelOption>? _availableLlmModelTargetOptions;

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

    /// <summary>
    /// U178-B：可选目标列表在本条别名上的投影。
    ///
    /// **共享同一个集合实例**（页面 VM 的 AvailableLlmModelTargetOptions），
    /// 不是每条别名复制一份：复制会让 RebuildAvailableLlmModelOptions 之后各行选项不同步。
    /// 换成投影是为了脱掉 per-item 的 `$parent[UserControl]` 祖先绑定（U178-B）。
    /// </summary>
    public ObservableCollection<WorkflowModelOption>? AvailableLlmModelTargetOptions
    {
        get => _availableLlmModelTargetOptions;
        internal set => SetProperty(ref _availableLlmModelTargetOptions, value);
    }

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
        var candidates = options as IReadOnlyCollection<WorkflowModelOption> ?? options.ToArray();
        var selected = IsConfigured
            ? candidates.FirstOrDefault(option =>
                !option.IsAlias
                && string.Equals(option.ProviderId, _targetProviderId, StringComparison.Ordinal)
                && string.Equals(option.ModelId, _targetModelId, StringComparison.Ordinal))
            : candidates.FirstOrDefault(option => option.IsInherited);

        // 目标模型已从 Provider 配置里消失时一并清掉 id，避免留下
        // IsConfigured 为真但无选中项的半状态。候选为空说明目录尚未加载，不做判定。
        if (selected is null && IsConfigured && candidates.Count > 0)
        {
            _targetProviderId = string.Empty;
            _targetModelId = string.Empty;
            selected = candidates.FirstOrDefault(option => option.IsInherited);
            OnPropertyChanged(nameof(TargetProviderId));
            OnPropertyChanged(nameof(TargetModelId));
            OnPropertyChanged(nameof(IsConfigured));
        }

        SetProperty(ref _selectedTargetOption, selected, nameof(SelectedTargetOption));
    }

    public string Snapshot => string.Join("|", AliasId, TargetProviderId, TargetModelId);
}
