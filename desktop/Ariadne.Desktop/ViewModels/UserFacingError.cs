using System.Text.RegularExpressions;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// Single author-facing failure→copy path (U1 structural).
/// Primary status/title text comes from stable <see cref="BackendException.Code"/> (or classified code),
/// never free-form backend/exception English. Diagnostics are redacted and optional secondary only.
/// </summary>
public static partial class UserFacingError
{
    private static readonly Regex AbsolutePath = PathRegex();
    private static readonly Regex HomePath = HomePathRegex();
    private static readonly AsyncLocal<WeakReference<IUserFailureObserver>?> Observer = new();

    public static void RegisterObserver(IUserFailureObserver observer)
    {
        Observer.Value = new WeakReference<IUserFailureObserver>(observer);
    }

    /// <summary>Primary author-facing line for status bars / toasts.</summary>
    public static string Format(Exception? ex, DisplayNameService names, string? contextKey = null)
    {
        var failure = FromException(ex);
        if (Observer.Value?.TryGetTarget(out var observer) == true)
        {
            observer.Observe(failure);
        }
        return failure.PrimaryText(names, contextKey);
    }

    /// <summary>Title-bar / chip: same primary identity, hard length cap (U43).</summary>
    public static string Short(Exception? ex, DisplayNameService names, string? contextKey = null)
    {
        var text = Format(ex, names, contextKey);
        return text.Length <= 48 ? text : text[..45] + "…";
    }

    /// <summary>Map known workflow/run status tokens to localized labels; unknown → generic idle/unknown, not raw English dump.</summary>
    public static string RuntimeStatus(string? status, DisplayNameService names)
    {
        if (string.IsNullOrWhiteSpace(status))
        {
            return names.Text("ui.status.idle");
        }

        var token = status.Trim().ToLowerInvariant();
        // Already localized Chinese labels from prior mapping: keep as-is if they match known Chinese status words.
        var key = token switch
        {
            "healthy" => "ui.status.healthy",
            "degraded" => "ui.status.degraded",
            "unavailable" => "ui.status.unavailable",
            "idle" => "ui.status.idle",
            "running" => "ui.status.running",
            "queued" => "ui.status.queued",
            "paused" => "ui.status.paused",
            "error" => "ui.status.error",
            "pending" => "ui.status.pending",
            "stopping" => "ui.status.stopping",
            "stopped" => "ui.status.stopped",
            "succeeded" or "success" or "completed" => "ui.status.succeeded",
            "failed" or "failure" => "ui.status.failed",
            "approved" => "ui.status.approved",
            "rejected" => "ui.status.rejected",
            "auto_audited" or "auto-audited" => "ui.status.auto_audited",
            "skipped" => "ui.status.completed",
            "retry_scheduled" or "retry-scheduled" => "ui.status.pending",
            "cancelled" or "canceled" => "ui.status.stopped",
            _ => null,
        };

        if (key is not null)
        {
            return names.Text(key);
        }

        // Chinese already: pass through short labels only
        if (token is "健康" or "降级" or "不可用" or "空闲" or "运行中" or "排队中" or "已暂停"
            or "错误" or "等待中" or "停止中" or "已停止" or "已成功" or "已失败" or "已完成"
            or "已通过" or "已拒绝")
        {
            return status.Trim();
        }

        // Unknown engineer token → do not dump raw status as primary
        return names.Text("ui.status.unavailable");
    }

    public static UserFailure FromException(Exception? ex)
    {
        if (ex is null)
        {
            return UserFailure.Unknown;
        }

        if (ex is BackendException be)
        {
            return new UserFailure(be.Code, be.Diagnostic, be.MessageKey, be.Parameters);
        }

        // Unwrap common wrappers
        if (ex.InnerException is BackendException innerBe)
        {
            return new UserFailure(innerBe.Code, innerBe.Diagnostic, innerBe.MessageKey, innerBe.Parameters);
        }

        // UI-local exceptions: typed mapping only — no English keyword table (U1 / 00A).
        return new UserFailure(BackendException.ClassifyLocalException(ex), ex.Message, null);
    }

    public static string PrimaryForCode(string? code, DisplayNameService names, string? contextKey = null)
        => new UserFailure(BackendException.NormalizeCode(code), null).PrimaryText(names, contextKey);

    public static string Sanitize(string raw)
    {
        if (string.IsNullOrWhiteSpace(raw))
        {
            return string.Empty;
        }

        var s = AbsolutePath.Replace(raw, "…");
        s = HomePath.Replace(s, "~…");
        s = s.Replace('\r', ' ').Replace('\n', ' ').Trim();
        while (s.Contains("  ", StringComparison.Ordinal))
        {
            s = s.Replace("  ", " ", StringComparison.Ordinal);
        }

        if (s.Length > 96)
        {
            s = s[..93] + "…";
        }

        return s;
    }

    [GeneratedRegex(@"(/[^ \t\r\n:]+)+|([A-Za-z]:\\[^ \t\r\n:]+)+", RegexOptions.Compiled)]
    private static partial Regex PathRegex();

    [GeneratedRegex(@"~(/[^ \t\r\n:]+)+", RegexOptions.Compiled)]
    private static partial Regex HomePathRegex();
}

public interface IUserFailureObserver
{
    void Observe(UserFailure failure);
}

/// <summary>Stable failure identity + optional redacted diagnostic (secondary only).</summary>
public readonly record struct UserFailure(
    string Code,
    string? Diagnostic,
    string? MessageKey = null,
    IReadOnlyDictionary<string, string>? Parameters = null)
{
    public static UserFailure Unknown { get; } = new("unknown", null, null);

    public string PrimaryText(DisplayNameService names, string? contextKey = null)
    {
        if (!string.IsNullOrWhiteSpace(MessageKey))
        {
            var keyed = names.Text(MessageKey);
            if (!keyed.StartsWith('[') || !keyed.EndsWith(']'))
            {
                return Parameters is { Count: > 0 }
                    ? names.Format(MessageKey, Parameters)
                    : keyed;
            }
        }

        var key = Code switch
        {
            "network" => "ui.error.network",
            "permission" => "ui.error.permission",
            "not_found" => "ui.error.not_found",
            // U208-A：后端新增的码在这张表里没有条目时，会被 `_ =>` 兜到
            // `ui.error.unknown`「未知错误」——比归错变体更糟。
            // ⇒ 后端每加一个 CommandErrorCode，这里必须同批加一行。
            // 守卫在 `ErrorCodeCopyCoverageTests`（逐一对应，不是存在性判据）。
            "not_configured" => "ui.error.not_configured",
            "validation" => "ui.error.validation",
            "budget" => "ui.error.budget",
            "conflict" => "ui.error.conflict",
            "cancelled" => "ui.error.cancelled",
            "external" => "ui.error.external",
            "io" => "ui.error.io",
            "ipc" => "ui.error.ipc",
            "legacy_run" => "ui.error.legacy_run",
            "resource_limit" => "ui.error.resource_limit",
            "paused" => "ui.error.paused",
            "stopped" => "ui.error.stopped",
            "external_outcome_unknown" => "ui.error.external_outcome_unknown",
            "serialization" => "ui.error.serialization",
            "internal" => "ui.error.internal",
            _ => contextKey ?? "ui.error.unknown",
        };

        // Primary is always a localization key — never interpolate English diagnostic into the status line.
        return names.Text(key);
    }

    public string? RedactedDiagnostic
    {
        get
        {
            if (string.IsNullOrWhiteSpace(Diagnostic))
            {
                return null;
            }

            var s = UserFacingError.Sanitize(Diagnostic);
            return string.IsNullOrWhiteSpace(s) ? null : s;
        }
    }
}
