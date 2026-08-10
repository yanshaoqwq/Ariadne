namespace Ariadne.Desktop.ViewModels;

public sealed class ProviderModelEditorRow : ViewModelBase
{
    private readonly Action _changed;
    private string _modelId;
    private string _capability;
    private string _maxContextTokens;
    private string _inputCost;
    private string _outputCost;
    private string _modelIdError = string.Empty;
    private string _capabilityError = string.Empty;
    private string _maxContextTokensError = string.Empty;
    private string _inputCostError = string.Empty;
    private string _outputCostError = string.Empty;

    public ProviderModelEditorRow(
        string modelId,
        string capability,
        string maxContextTokens,
        string inputCost,
        string outputCost,
        Action changed,
        Action<ProviderModelEditorRow> remove)
    {
        _modelId = modelId;
        _capability = capability;
        _maxContextTokens = maxContextTokens;
        _inputCost = inputCost;
        _outputCost = outputCost;
        _changed = changed;
        RemoveCommand = new RelayCommand(() => remove(this));
    }

    public string ModelId { get => _modelId; set => SetAndNotify(ref _modelId, value); }
    public string Capability { get => _capability; set => SetAndNotify(ref _capability, value); }
    public string MaxContextTokens { get => _maxContextTokens; set => SetAndNotify(ref _maxContextTokens, value); }
    public string InputCost { get => _inputCost; set => SetAndNotify(ref _inputCost, value); }
    public string OutputCost { get => _outputCost; set => SetAndNotify(ref _outputCost, value); }
    public string ModelIdError { get => _modelIdError; private set => SetError(ref _modelIdError, value); }
    public string CapabilityError { get => _capabilityError; private set => SetError(ref _capabilityError, value); }
    public string MaxContextTokensError { get => _maxContextTokensError; private set => SetError(ref _maxContextTokensError, value); }
    public string InputCostError { get => _inputCostError; private set => SetError(ref _inputCostError, value); }
    public string OutputCostError { get => _outputCostError; private set => SetError(ref _outputCostError, value); }
    public bool HasModelIdError => !string.IsNullOrWhiteSpace(ModelIdError);
    public bool HasCapabilityError => !string.IsNullOrWhiteSpace(CapabilityError);
    public bool HasMaxContextTokensError => !string.IsNullOrWhiteSpace(MaxContextTokensError);
    public bool HasInputCostError => !string.IsNullOrWhiteSpace(InputCostError);
    public bool HasOutputCostError => !string.IsNullOrWhiteSpace(OutputCostError);
    public RelayCommand RemoveCommand { get; }

    public string Snapshot => string.Join("|", ModelId, Capability, MaxContextTokens, InputCost, OutputCost);

    public bool Validate(
        IReadOnlySet<string> duplicateIds,
        ProviderModelValidationMessages messages)
    {
        var modelId = ModelId.Trim();
        ModelIdError = string.IsNullOrWhiteSpace(modelId)
            ? messages.RequiredModelId
            : duplicateIds.Contains(modelId)
                ? messages.DuplicateModelId
                : string.Empty;
        CapabilityError = string.IsNullOrWhiteSpace(Capability)
            ? messages.RequiredCapability
            : string.Empty;
        MaxContextTokensError = ValidatePositiveInteger(MaxContextTokens, messages.InvalidContext);
        InputCostError = ValidateNonNegativeNumber(InputCost, messages.InvalidInputCost);
        OutputCostError = ValidateNonNegativeNumber(OutputCost, messages.InvalidOutputCost);
        return !HasModelIdError
            && !HasCapabilityError
            && !HasMaxContextTokensError
            && !HasInputCostError
            && !HasOutputCostError;
    }

    private void SetAndNotify(ref string field, string value, [System.Runtime.CompilerServices.CallerMemberName] string? propertyName = null)
    {
        if (SetProperty(ref field, value ?? string.Empty, propertyName))
        {
            _changed();
        }
    }

    private static string ValidatePositiveInteger(string value, string message)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return string.Empty;
        }
        return int.TryParse(
                   value.Trim(),
                   System.Globalization.NumberStyles.Integer,
                   System.Globalization.CultureInfo.InvariantCulture,
                   out var parsed)
               && parsed > 0
            ? string.Empty
            : message;
    }

    private static string ValidateNonNegativeNumber(string value, string message)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return string.Empty;
        }
        return double.TryParse(
                   value.Trim(),
                   System.Globalization.NumberStyles.Float,
                   System.Globalization.CultureInfo.InvariantCulture,
                   out var parsed)
               && double.IsFinite(parsed)
               && parsed >= 0
            ? string.Empty
            : message;
    }

    private void SetError(ref string field, string value, [System.Runtime.CompilerServices.CallerMemberName] string? propertyName = null)
    {
        if (SetProperty(ref field, value ?? string.Empty, propertyName))
        {
            var flag = $"Has{propertyName!.Replace("Error", string.Empty, StringComparison.Ordinal)}Error";
            OnPropertyChanged(flag);
        }
    }
}

public sealed record ProviderModelValidationMessages(
    string RequiredModelId,
    string DuplicateModelId,
    string RequiredCapability,
    string InvalidContext,
    string InvalidInputCost,
    string InvalidOutputCost);
