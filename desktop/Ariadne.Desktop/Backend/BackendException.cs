namespace Ariadne.Desktop.Backend;

/// Typed IPC / transport failure. Primary UI must map <see cref="Code"/> via
/// <see cref="ViewModels.UserFacingError"/> — never surface <see cref="Diagnostic"/> as the main status string.
public sealed class BackendException : Exception
{
    public BackendException(string code, string? diagnostic = null, Exception? innerException = null)
        : base(string.IsNullOrWhiteSpace(diagnostic) ? code : diagnostic, innerException)
    {
        Code = NormalizeCode(code);
        Diagnostic = string.IsNullOrWhiteSpace(diagnostic) ? null : diagnostic.Trim();
    }

    /// Stable failure identity (e.g. network, validation, ipc).
    public string Code { get; }

    /// Optional localization key from IPC (`error_key`).
    public string? MessageKey { get; init; }

    /// Optional engineer/diagnostic detail (paths, provider text). Not author primary copy.
    public string? Diagnostic { get; }

    public IReadOnlyDictionary<string, string> Parameters { get; init; } =
        new Dictionary<string, string>(StringComparer.Ordinal);

    public string? Field { get; init; }

    public string? Section { get; init; }

    public string? RecoveryAction { get; init; }

    public string? CorrelationId { get; init; }

    public static BackendException FromIpcPayload(
        string? errorCode,
        string? errorMessage,
        string? errorKey = null,
        IReadOnlyDictionary<string, string>? errorParams = null,
        string? errorField = null,
        string? errorSection = null,
        string? recoveryAction = null,
        string? correlationId = null)
    {
        var diagnostic = string.IsNullOrWhiteSpace(errorMessage) ? null : errorMessage.Trim();
        // U1: product path consumes server error_code only. Legacy missing code → unknown
        // (legacy free-form classification lives solely in core IPC adapter, not here).
        var code = string.IsNullOrWhiteSpace(errorCode) ? "unknown" : errorCode.Trim();
        var ex = new BackendException(code, diagnostic)
        {
            MessageKey = string.IsNullOrWhiteSpace(errorKey) ? null : errorKey.Trim(),
            Parameters = errorParams is null
                ? new Dictionary<string, string>(StringComparer.Ordinal)
                : errorParams.ToDictionary(item => item.Key, item => item.Value, StringComparer.Ordinal),
            Field = string.IsNullOrWhiteSpace(errorField) ? null : errorField.Trim(),
            Section = string.IsNullOrWhiteSpace(errorSection) ? null : errorSection.Trim(),
            RecoveryAction = string.IsNullOrWhiteSpace(recoveryAction) ? null : recoveryAction.Trim(),
            CorrelationId = string.IsNullOrWhiteSpace(correlationId) ? null : correlationId.Trim(),
        };
        return ex;
    }

    public static BackendException Transport(string code, string diagnostic)
        => new(code, diagnostic);

    public static string NormalizeCode(string? code)
    {
        if (string.IsNullOrWhiteSpace(code))
        {
            return "unknown";
        }

        var c = code.Trim().ToLowerInvariant().Replace('-', '_');
        return c switch
        {
            "perm" or "eacces" => "permission",
            "notfound" or "missing" => "not_found",
            "validate" or "invalid" => "validation",
            "timeout" or "timed_out" => "network",
            "transport" or "backend" => "ipc",
            _ => c,
        };
    }

    /// Non-IPC exceptions (UI-local) map to transport/unknown only — no English keyword table.
    public static string ClassifyLocalException(Exception? ex)
    {
        if (ex is null)
        {
            return "unknown";
        }

        if (ex is BackendException be)
        {
            return be.Code;
        }

        if (ex is TimeoutException or TaskCanceledException or OperationCanceledException)
        {
            return "cancelled";
        }

        if (ex is UnauthorizedAccessException)
        {
            return "permission";
        }

        if (ex is FileNotFoundException or DirectoryNotFoundException)
        {
            return "not_found";
        }

        if (ex is IOException)
        {
            return "io";
        }

        return "unknown";
    }
}
