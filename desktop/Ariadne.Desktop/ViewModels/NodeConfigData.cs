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
        "model_id",
        "budget_usd",
        "timeout_ms",
        "breakpoint",
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
        bool breakpointEnabled)
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

        if (!string.IsNullOrWhiteSpace(budgetUsd))
        {
            data["budget_usd"] = budgetUsd;
        }

        if (!string.IsNullOrWhiteSpace(timeoutMs))
        {
            data["timeout_ms"] = timeoutMs;
        }

        if (breakpointEnabled)
        {
            data["breakpoint"] = true;
        }

        return data;
    }
}
