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

    /// <summary>
    /// 「下一步做什么」（U198-B）。主文案回答「出了什么事」，这条回答「我现在能做什么」。
    ///
    /// ## 两级取法，以及为什么第二级不可省
    ///
    /// 1. `BackendException.RecoveryAction` → `ui.settings.recovery.{action}`：
    ///    后端给出的**精确**建议，优先。
    /// 2. 失败码 → `ui.recovery.{code}`：兜底。
    ///
    /// ⚠️ **只做第一级等于什么都没做**：全仓 `recovery_action` 的产出点只有
    /// `commands.rs:10091-10156` 一处（检索/Qdrant 配置分区），也就是说
    /// 配置页的检索分区之外，`RecoveryAction` **永远是 null**。
    /// 报告里「把配置页那套 RecoveryText 搬到其它页」如果照字面理解为「搬第一级」，
    /// 得到的是五个恒空字段——一个装好了、永不亮的灯。第二级（按失败码给建议）
    /// 才是让其余五页真的有话可说的那一半。
    ///
    /// ## 为什么有些码刻意没有建议
    ///
    /// `cancelled` / `paused` / `stopped` 是**作者自己发起**的中止，没有"补救"可言。
    /// 硬凑一句「请重试」会把这一行变成噪声，作者学会忽略它之后，
    /// 真正需要看的那次也不会看——这一行的价值全在它不常出现。
    /// </summary>
    public static string Recovery(Exception? ex, DisplayNameService names)
    {
        // 走 BackendException 的原始字段而不是 UserFailure：RecoveryAction 没有
        // 进 UserFailure（它是"精确建议"，不属于稳定失败身份的一部分）。
        var recoveryAction = ex switch
        {
            BackendException be => be.RecoveryAction,
            { InnerException: BackendException inner } => inner.RecoveryAction,
            _ => null,
        };
        if (!string.IsNullOrWhiteSpace(recoveryAction))
        {
            var precise = Localized(names, $"ui.settings.recovery.{recoveryAction}");
            if (!string.IsNullOrEmpty(precise))
            {
                return precise;
            }
        }

        var failure = FromException(ex);
        return RecoveryForCode(failure.Code, names);
    }

    /// <summary>按失败码取兜底建议；主文案已经说清下一步的码返回空串。</summary>
    public static string RecoveryForCode(string? code, DisplayNameService names)
    {
        var normalized = BackendException.NormalizeCode(code);
        return CodesWithoutRecoveryHint.Contains(normalized)
            ? string.Empty
            : Localized(names, $"ui.recovery.{normalized}");
    }

    /// <summary>
    /// **刻意没有补救建议的失败码**，两类原因，别当成漏掉的：
    ///
    /// 1. 作者自己发起的中止（`cancelled` / `paused` / `stopped`）——没有"补救"可言。
    /// 2. `ui.error.{code}` 主文案里**已经**写了下一步该做什么。
    ///    例如 `ui.error.conflict` = 「内容已被其它操作更新，请刷新后重试。」
    ///    —— 再补一行「请刷新后重试」就是把同一句话在一屏里印两遍。
    ///    这一行的价值全在它**不常出现**：一旦变成每次失败都挂一句废话，
    ///    作者学会跳过它之后，真正要紧的那次也不会看。
    ///
    /// ⇒ 后端新增 `CommandErrorCode` 时，这里必须**二选一**：给它一句建议，
    ///   或把它列进这张表并确认主文案确实交代了动作。
    ///   守卫是 `RecoveryHintCoverageTests`，判据取「与 `ui.error.*` 逐一对应」——
    ///   「存在若干条 ui.recovery.* 键」那种存在性判据在只补一条时照样绿。
    /// </summary>
    private static readonly HashSet<string> CodesWithoutRecoveryHint = new(StringComparer.Ordinal)
    {
        // 第 1 类：作者自己中止
        "cancelled", "paused", "stopped",
        // 第 2 类：主文案已含动作
        "network", "validation", "conflict", "external", "io", "ipc",
        "legacy_run", "resource_limit", "internal", "operation_failed", "indexing_not_ready",
    };

    /// <summary>
    /// 后端已成文的建议（工作流 `recovery_suggestion` / `pause_reason` 这一类）。
    ///
    /// ⚠️ 它有**两种形态**，都要吃（U196-E / U198-B）：
    /// - 文案 key（形如 `error.workflow.worker_failed.recovery`）⇒ 查语言包；
    /// - 成文中文（`workflow/runtime.rs:recovery_suggestion()` 直接返回的句子）⇒ 原样显示。
    ///
    /// 判别方式是「像不像 key」而不是「查得到就是 key」：查不到的 key 必须落到
    /// 空串，绝不能把 `[error.xxx]` 这种缺键占位符当成建议印给作者。
    /// </summary>
    public static string RecoveryFromSuggestion(string? suggestion, DisplayNameService names)
    {
        if (string.IsNullOrWhiteSpace(suggestion))
        {
            return string.Empty;
        }

        var trimmed = suggestion.Trim();
        return LooksLikeKey(trimmed) ? Localized(names, trimmed) : trimmed;
    }

    /// <summary>缺键时 <see cref="DisplayNameService.Text"/> 返回 <c>[key]</c>；那不是文案。</summary>
    private static string Localized(DisplayNameService names, string key)
    {
        var text = names.Text(key);
        return text.StartsWith('[') && text.EndsWith(']') ? string.Empty : text;
    }

    /// <summary>形如 `a.b.c` 的全小写点分标识符才算 key；中文句子不会误判为 key。</summary>
    private static bool LooksLikeKey(string value) => KeyShape().IsMatch(value);

    [GeneratedRegex(@"^[a-z][a-z0-9_]*(\.[a-z0-9_]+)+$", RegexOptions.Compiled)]
    private static partial Regex KeyShape();

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
