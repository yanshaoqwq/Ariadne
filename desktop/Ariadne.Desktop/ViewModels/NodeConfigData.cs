namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 节点 data 字典：UI 只编辑部分字段；保存时必须合并保留后端/其它配置键
/// （tool_enabled、input_aliases、approval_policy、skills、temperature 等），
/// 禁止用「仅 UI 字段」整表替换导致静默丢配置。
/// </summary>
public static class NodeConfigData
{
    private static readonly HashSet<string> UiOwnedKeys = new(StringComparer.OrdinalIgnoreCase)
    {
        "name",
        "work_dir",
        "user_note",
        "expose_as_tool",
        "prompt_template",
        "provider_id",
        "model_id",
        "budget_usd",
        "timeout_ms",
        "breakpoint",
        "import_path",
        "path", // document_read 后端字段
        "include_content",
        "data_in_handles",
        "query_alias",
        "limit",
        "input_alias",
        "operator",
        "expected",
        "max_iterations",
        "stop_input_alias",
        "stop_expected",
        "approval_id",
        "auto_approve",
        "artifact_id",
        "format",
        "title",
        "chapter_id",
        "chapter_document_id",
        "chapter_text_alias",
        "auto_mode",
    };

    public static bool IsUiOwnedKey(string? key) =>
        !string.IsNullOrWhiteSpace(key) && UiOwnedKeys.Contains(key);

    /// <summary>
    /// 从加载的 node.Data 抽出非 UI 字段副本（opaque config）。
    /// </summary>
    public static Dictionary<string, object?> CaptureExtra(IReadOnlyDictionary<string, object?>? source)
    {
        var extra = new Dictionary<string, object?>(StringComparer.Ordinal);
        if (source is null || source.Count == 0)
        {
            return extra;
        }

        foreach (var pair in source)
        {
            if (string.IsNullOrWhiteSpace(pair.Key) || IsUiOwnedKey(pair.Key))
            {
                continue;
            }

            extra[pair.Key] = pair.Value;
        }

        return extra;
    }

    /// <summary>
    /// 以 opaque 为底，叠写 UI 字段；返回可交给 SaveWorkflowGraph 的 data。
    /// </summary>
    public static Dictionary<string, object?> MergeUiFields(
        IReadOnlyDictionary<string, object?>? extra,
        string name,
        string workDir,
        string userNote,
        bool isStartNode,
        bool exposedAsTool,
        string promptTemplate,
        string modelId,
        string budgetUsd,
        string timeoutMs,
        bool breakpointEnabled,
        string? importPath = null,
        IReadOnlyList<string>? dataInHandles = null,
        IReadOnlyDictionary<string, object?>? utilityFields = null,
        string providerId = "")
    {
        var data = new Dictionary<string, object?>(StringComparer.Ordinal);
        if (extra is not null)
        {
            foreach (var pair in extra)
            {
                if (string.IsNullOrWhiteSpace(pair.Key) || IsUiOwnedKey(pair.Key))
                {
                    continue;
                }

                data[pair.Key] = pair.Value;
            }
        }

        // 新建节点也写入统一配置版本；旧节点加载时的更高版本由 opaque 数据保留。
        if (!data.ContainsKey("schema_version"))
        {
            data["schema_version"] = 1;
        }

        data["name"] = name ?? string.Empty;
        if (!string.IsNullOrWhiteSpace(userNote))
        {
            data["user_note"] = userNote.Trim();
        }
        if (!string.IsNullOrWhiteSpace(workDir))
        {
            data["work_dir"] = workDir;
        }

        if (isStartNode)
        {
            data["expose_as_tool"] = exposedAsTool;
        }

        if (!string.IsNullOrWhiteSpace(promptTemplate))
        {
            data["prompt_template"] = promptTemplate;
        }

        if (!string.IsNullOrWhiteSpace(modelId))
        {
            data["model_id"] = modelId;
        }

        if (!string.IsNullOrWhiteSpace(providerId))
        {
            data["provider_id"] = providerId.Trim();
        }

        // F13：后端 WorkflowLlmNodeConfig 期望 f64/u64；UI 绑定仍是 string，这里解析为数值再落盘。
        if (!string.IsNullOrWhiteSpace(budgetUsd)
            && double.TryParse(
                budgetUsd.Trim(),
                System.Globalization.NumberStyles.Float,
                System.Globalization.CultureInfo.InvariantCulture,
                out var budgetValue))
        {
            data["budget_usd"] = budgetValue;
        }

        if (!string.IsNullOrWhiteSpace(timeoutMs)
            && ulong.TryParse(
                timeoutMs.Trim(),
                System.Globalization.NumberStyles.Integer,
                System.Globalization.CultureInfo.InvariantCulture,
                out var timeoutValue)
            && timeoutValue > 0)
        {
            // JSON number；超出 long 范围的值用 double 会丢精度，UI 超时不会那么大。
            data["timeout_ms"] = timeoutValue <= long.MaxValue
                ? (long)timeoutValue
                : timeoutValue;
        }

        // 文档读：后端字段为 path（兼容保留 import_path）
        if (!string.IsNullOrWhiteSpace(importPath))
        {
            var path = importPath.Trim();
            data["path"] = path;
            data["import_path"] = path;
        }

        if (dataInHandles is { Count: > 0 })
        {
            data["data_in_handles"] = dataInHandles.ToArray();
        }

        if (utilityFields is not null)
        {
            foreach (var pair in utilityFields)
            {
                if (string.IsNullOrWhiteSpace(pair.Key))
                {
                    continue;
                }

                if (pair.Value is null || pair.Value is string s && string.IsNullOrWhiteSpace(s))
                {
                    data.Remove(pair.Key);
                    continue;
                }

                data[pair.Key] = pair.Value;
            }
        }

        if (breakpointEnabled)
        {
            data["breakpoint"] = true;
        }

        return data;
    }
}
