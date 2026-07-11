namespace Ariadne.Desktop.ViewModels;

/// <summary>供应商列表项；草稿可挂一份表单快照，切换时不丢编辑中内容。</summary>
public sealed class ProviderOptionViewModel : ViewModelBase
{
    private string _displayName;
    private string _keyStatus;
    private bool _isSelected;
    private bool _isDraft;
    private ProviderFormSnapshot? _formSnapshot;
    private readonly Action<ProviderOptionViewModel>? _select;

    public ProviderOptionViewModel(
        string providerId,
        string displayName,
        string keyStatus,
        Action<ProviderOptionViewModel>? select = null,
        bool isDraft = false)
    {
        ProviderId = providerId;
        _displayName = displayName;
        _keyStatus = keyStatus;
        _isDraft = isDraft;
        _select = select;
        SelectCommand = new RelayCommand(() => _select?.Invoke(this));
    }

    public string ProviderId { get; }

    public string DisplayName
    {
        get => _displayName;
        set
        {
            if (SetProperty(ref _displayName, value))
            {
                OnPropertyChanged(nameof(DisplayTitle));
            }
        }
    }

    public string KeyStatus
    {
        get => _keyStatus;
        set => SetProperty(ref _keyStatus, value);
    }

    public bool IsDraft
    {
        get => _isDraft;
        set => SetProperty(ref _isDraft, value);
    }

    public bool IsSelected
    {
        get => _isSelected;
        set => SetProperty(ref _isSelected, value);
    }

    public bool HasFormSnapshot => _formSnapshot is not null;

    public string DisplayTitle => string.IsNullOrWhiteSpace(DisplayName)
        ? ProviderId
        : $"{DisplayName} ({ProviderId})";

    public RelayCommand SelectCommand { get; }

    public void CaptureForm(ProviderFormSnapshot snapshot)
    {
        _formSnapshot = snapshot;
        if (!string.IsNullOrWhiteSpace(snapshot.DisplayName))
        {
            DisplayName = snapshot.DisplayName;
        }
    }

    public ProviderFormSnapshot? PeekForm() => _formSnapshot;

    public void ClearFormSnapshot() => _formSnapshot = null;
}

/// <summary>供应商编辑表单快照（草稿切换用）。</summary>
public sealed class ProviderFormSnapshot
{
    public required string ProviderId { get; init; }
    public required string ProviderType { get; init; }
    public required string DisplayName { get; init; }
    public required string BaseUrl { get; init; }
    public required bool Enabled { get; init; }
    public required bool MakeDefaultLlm { get; init; }
    public required bool MakeDefaultEmbedding { get; init; }
    public required bool MakeDefaultReranker { get; init; }
    public required string ModelsText { get; init; }
    public required string EmbeddingModelId { get; init; }
}

public sealed class ThemeGroupViewModel
{
    public ThemeGroupViewModel(string title, IEnumerable<ThemeOption> options)
    {
        Title = title;
        Options = options.ToList();
    }

    public string Title { get; }
    public IReadOnlyList<ThemeOption> Options { get; }
}
