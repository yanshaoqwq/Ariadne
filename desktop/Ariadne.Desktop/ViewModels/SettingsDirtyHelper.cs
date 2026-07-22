namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 设置页脏标记与确认项默认值的纯逻辑，供单测与 ViewModel 共用。
/// 确认项全集对齐《配置项与确认项清单》《创作总结机制》《总结机制具体实现计划》§8。
/// </summary>
public static class SettingsDirtyHelper
{
    /// <summary>register 子功能（与 core RegisterFunction 一致）。</summary>
    public static readonly string[] RegisterFunctionSuffixes =
    {
        "character_profile",
        "character_plan",
        "character_trait",
        "relationship",
        "foreshadowing",
        "theme_anchor",
    };

    public static readonly string[] RegisterAgents = { "outliner", "designer", "planner" };

    /// <summary>
    /// 自动化确认项全集：预算/权限 4 项 + 规划输出 + register 子功能 + 审稿 + 总结四步 + patch。
    /// 设置页必须展示完整列表，禁止空列表。
    /// </summary>
    public static readonly string[] DefaultConfirmationKinds = BuildDefaultConfirmationKinds();

    private static string[] BuildDefaultConfirmationKinds()
    {
        var list = new List<string>
        {
            // 配置项清单 §四 自动化门禁
            "chapter_write",
            "summary_write",
            "high_risk_permission",
            "budget_exceeded",
            // 规划节点输出 / 纲领 patch
            "outliner_output",
            "designer_output",
            "planner_output",
        };

        // register 子功能 × 三 agent（创作总结机制：子功能独立配置）
        foreach (var agent in RegisterAgents)
        {
            foreach (var func in RegisterFunctionSuffixes)
            {
                list.Add($"{agent}_register_{func}");
            }
        }

        // 兼容旧聚合键
        list.Add("planner_register");

        list.AddRange(new[]
        {
            "critic_review",
            "prudent_review",
            "segment_summary",
            "event_summary",
            "chapter_summary",
            "stage_summary",
            "writer_correction_patch",
            "polisher_correction_patch",
        });

        return list.ToArray();
    }

    /// <summary>
    /// Load 结束时是否应清除脏：无论成功失败，只要已 Capture 过当前快照，就不应为脏。
    /// </summary>
    public static bool HasUnsavedAfterCapture(string currentSnapshot, string savedSnapshot) =>
        !string.Equals(currentSnapshot, savedSnapshot, StringComparison.Ordinal);

    /// <summary>
    /// 离开对话框点「保存」后是否允许导航：仅当脏已清除。
    /// </summary>
    public static bool CanNavigateAfterLeaveSave(bool hasUnsavedAfterSave) => !hasUnsavedAfterSave;

    /// <summary>
    /// 合并已加载策略 + 全集 keys，永远返回完整非空列表。
    /// 旧 <c>planner_register</c> 会扩散到各 register 子功能（尚未单独配置时）。
    /// </summary>
    public static IReadOnlyList<(string Kind, string NormalPolicy, string AutoModePolicy, string ApprovalPrompt)> EnsureConfirmationPolicies(
        IEnumerable<(string Kind, string NormalPolicy, string AutoModePolicy, string ApprovalPrompt)>? loaded)
    {
        var map = new Dictionary<string, (string Normal, string Auto, string Prompt)>(StringComparer.Ordinal);
        if (loaded is not null)
        {
            foreach (var item in loaded)
            {
                if (!string.IsNullOrWhiteSpace(item.Kind))
                {
                    map[item.Kind] = (item.NormalPolicy, item.AutoModePolicy, item.ApprovalPrompt ?? string.Empty);
                }
            }
        }

        // 旧聚合键 → 子功能
        if (map.TryGetValue("planner_register", out var agg))
        {
            foreach (var agent in RegisterAgents)
            {
                foreach (var func in RegisterFunctionSuffixes)
                {
                    var key = $"{agent}_register_{func}";
                    if (!map.ContainsKey(key))
                    {
                        map[key] = agg;
                    }
                }
            }
        }

        var list = new List<(string, string, string, string)>(DefaultConfirmationKinds.Length);
        foreach (var kind in DefaultConfirmationKinds)
        {
            if (map.TryGetValue(kind, out var policies))
            {
                list.Add((kind, policies.Normal, policies.Auto, policies.Prompt));
                map.Remove(kind);
            }
            else
            {
                list.Add((kind, "manual_review", "allow_by_default", string.Empty));
            }
        }

        // 保留未知但已存的项（兼容）
        foreach (var kv in map.OrderBy(k => k.Key, StringComparer.Ordinal))
        {
            list.Add((kv.Key, kv.Value.Normal, kv.Value.Auto, kv.Value.Prompt));
        }

        return list;
    }

    /// <summary>Hex 归一化为 #RRGGBB 大写，避免脏假阳性。</summary>
    public static string NormalizeHexForSnapshot(string? hex)
    {
        if (ColorChannelEditor.TryParseHex(hex, out var r, out var g, out var b))
        {
            return ColorChannelEditor.ToHex(r, g, b);
        }

        return (hex ?? string.Empty).Trim();
    }

    /// <summary>
    /// 确认项左栏子索引分组（id, display_name key）。
    /// 对齐配置项清单 / 创作总结机制结构。
    /// </summary>
    public static readonly (string Id, string DisplayKey)[] ConfirmationSubIndexGroups =
    {
        ("conf_gates", "ui.settings.section.confirmations.gates"),
        ("conf_planning", "ui.settings.section.confirmations.planning"),
        ("conf_register", "ui.settings.section.confirmations.register"),
        ("conf_review", "ui.settings.section.confirmations.review"),
        ("conf_summary", "ui.settings.section.confirmations.summary"),
        ("conf_patch", "ui.settings.section.confirmations.patch"),
    };

    /// <summary>根据 confirmation_kind 归入子分组 id。</summary>
    public static string ConfirmationGroupIdForKind(string kind)
    {
        if (kind is "chapter_write" or "summary_write" or "high_risk_permission" or "budget_exceeded")
        {
            return "conf_gates";
        }

        if (kind is "outliner_output" or "designer_output" or "planner_output")
        {
            return "conf_planning";
        }

        if (kind.Contains("_register", StringComparison.Ordinal) || kind == "planner_register")
        {
            return "conf_register";
        }

        if (kind is "critic_review" or "prudent_review")
        {
            return "conf_review";
        }

        if (kind is "segment_summary" or "event_summary" or "chapter_summary" or "stage_summary")
        {
            return "conf_summary";
        }

        if (kind is "writer_correction_patch" or "polisher_correction_patch")
        {
            return "conf_patch";
        }

        return "conf_gates";
    }
}
