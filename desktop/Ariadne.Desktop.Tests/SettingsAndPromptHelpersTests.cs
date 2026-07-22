using Avalonia.Media;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Controls;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 设置脏标记、系统主题演示色、确认项默认、prompt_list 解析 — 驱动 shipped 纯函数。
/// </summary>
public sealed class SettingsAndPromptHelpersTests
{
    [Fact]
    public void NodePresetModelSelection_KeepsProviderIdentityWhenModelIdsCollide()
    {
        var inherited = new PermissionPolicy(
            false, false, false, false, false,
            Array.Empty<string>(), Array.Empty<string>());
        var vm = new NodeTypePresetViewModel(
            "writer",
            "agent.writer",
            "Writer",
            "provider-a",
            "shared-model",
            "300",
            "1",
            null,
            inherited,
            new Dictionary<string, bool?>(),
            tool => tool,
            () => { });
        var providerA = new WorkflowModelOption("provider-a", "shared-model", "Provider A");
        var providerB = new WorkflowModelOption("provider-b", "shared-model", "Provider B");

        vm.RebindModelOptions(new[] { providerA, providerB });
        Assert.Same(providerA, vm.SelectedModelOption);

        vm.SelectedModelOption = providerB;
        Assert.Equal("provider-b", vm.ProviderId);
        Assert.Equal("shared-model", vm.ModelId);
        Assert.Contains("provider-b", vm.Snapshot, StringComparison.Ordinal);
    }

    [Fact]
    public void Dirty_FalseAfterCaptureWithIdenticalSnapshot()
    {
        var current = "a\u001fb\u001fc";
        var saved = current;
        Assert.False(SettingsDirtyHelper.HasUnsavedAfterCapture(current, saved));
    }

    [Fact]
    public void Dirty_TrueWhenSnapshotDiffers()
    {
        Assert.True(SettingsDirtyHelper.HasUnsavedAfterCapture("a", "b"));
    }

    [Fact]
    public void LeaveSave_AllowsNavigateOnlyWhenNotDirty()
    {
        Assert.True(SettingsDirtyHelper.CanNavigateAfterLeaveSave(hasUnsavedAfterSave: false));
        Assert.False(SettingsDirtyHelper.CanNavigateAfterLeaveSave(hasUnsavedAfterSave: true));
    }

    [Fact]
    public void EnsureConfirmationPolicies_FillsFullCatalogFromSummaryMechanism()
    {
        var policies = SettingsDirtyHelper.EnsureConfirmationPolicies(null);
        // 4 门禁 + 3 规划输出 + 18 register 子功能 + 1 聚合 + 2 审稿 + 4 总结 + 2 patch ≥ 34
        Assert.True(policies.Count >= 30, $"expected full catalog from 总结机制, got {policies.Count}");
        Assert.Equal(
            SettingsDirtyHelper.DefaultConfirmationKinds,
            policies.Take(SettingsDirtyHelper.DefaultConfirmationKinds.Length).Select(p => p.Kind).ToArray());
        // 配置项清单 4 门禁
        Assert.Contains(policies, p => p.Kind == "chapter_write");
        Assert.Contains(policies, p => p.Kind == "budget_exceeded");
        // 写作 12 类核心
        Assert.Contains(policies, p => p.Kind == "outliner_output");
        Assert.Contains(policies, p => p.Kind == "segment_summary");
        Assert.Contains(policies, p => p.Kind == "writer_correction_patch");
        Assert.Contains(policies, p => p.Kind == "polisher_correction_patch");
        // 创作总结机制：register 子功能独立
        Assert.Contains(policies, p => p.Kind == "planner_register_character_trait");
        Assert.Contains(policies, p => p.Kind == "outliner_register_foreshadowing");
        Assert.Contains(policies, p => p.Kind == "designer_register_theme_anchor");
        Assert.All(policies.Take(SettingsDirtyHelper.DefaultConfirmationKinds.Length), p =>
        {
            Assert.Equal("manual_review", p.NormalPolicy);
            Assert.Equal("allow_by_default", p.AutoModePolicy);
            Assert.Equal(string.Empty, p.ApprovalPrompt);
        });
    }

    [Fact]
    public void ConfirmationGroupIdForKind_MatchesSummaryMechanismBuckets()
    {
        Assert.Equal("conf_gates", SettingsDirtyHelper.ConfirmationGroupIdForKind("chapter_write"));
        Assert.Equal("conf_planning", SettingsDirtyHelper.ConfirmationGroupIdForKind("outliner_output"));
        Assert.Equal("conf_register", SettingsDirtyHelper.ConfirmationGroupIdForKind("planner_register_character_trait"));
        Assert.Equal("conf_register", SettingsDirtyHelper.ConfirmationGroupIdForKind("outliner_register_foreshadowing"));
        Assert.Equal("conf_review", SettingsDirtyHelper.ConfirmationGroupIdForKind("critic_review"));
        Assert.Equal("conf_summary", SettingsDirtyHelper.ConfirmationGroupIdForKind("segment_summary"));
        Assert.Equal("conf_patch", SettingsDirtyHelper.ConfirmationGroupIdForKind("writer_correction_patch"));
        Assert.Equal(6, SettingsDirtyHelper.ConfirmationSubIndexGroups.Length);
    }

    [Fact]
    public void EnsureConfirmationPolicies_LegacyPlannerRegisterSpreadsToSubfunctions()
    {
        var loaded = new[]
        {
            ("chapter_write", "allow_by_default", "auto_approval", "审计章节"),
            ("planner_register", "allow_by_default", "auto_approval", "审计注册表"),
        };
        var policies = SettingsDirtyHelper.EnsureConfirmationPolicies(loaded);
        Assert.True(policies.Count >= 30);
        var chapter = policies.First(p => p.Kind == "chapter_write");
        Assert.Equal("allow_by_default", chapter.NormalPolicy);
        // 子功能继承旧聚合键
        var trait = policies.First(p => p.Kind == "planner_register_character_trait");
        Assert.Equal("allow_by_default", trait.NormalPolicy);
        Assert.Equal("auto_approval", trait.AutoModePolicy);
        Assert.Equal("审计注册表", trait.ApprovalPrompt);
    }

    [Fact]
    public void ConfirmationPolicyViewModel_AutoModeOffMapsToAllowByDefaultNotManualReview()
    {
        // Runtime Auto Mode: allow_by_default → Skip, auto_approval → AutoAudit.
        // UI Off must NOT claim "manual_review" — that would be a no-op lie vs shipped enum.
        var dirty = 0;
        var policy = new ConfirmationPolicyViewModel(
            "chapter_write",
            "章节写回",
            normalPolicy: "manual_review",
            autoModePolicy: "allow_by_default",
            approvalPrompt: "审计章节写回",
            markDirty: () => dirty++);

        Assert.False(policy.AutoModeAutoApproval);
        Assert.Equal("allow_by_default", policy.AutoModePolicy);
        Assert.Equal("manual_review", policy.NormalPolicy);
        Assert.Equal("审计章节写回", policy.ApprovalPrompt);

        policy.AutoModeAutoApproval = true;
        Assert.Equal("auto_approval", policy.AutoModePolicy);
        Assert.True(dirty >= 1);

        policy.AutoModeAutoApproval = false;
        Assert.Equal("allow_by_default", policy.AutoModePolicy);
        Assert.NotEqual("manual_review", policy.AutoModePolicy);
    }

    [Fact]
    public void SettingsPageView_AutoModeColumnUsesDistinctToggleLabelsNotReview()
    {
        // Product path: Auto Mode toggle must bind PolicyAutoOn/Off, not PolicyReview (审核).
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        string? viewPath = null;
        for (var depth = 0; directory is not null && depth < 10; depth++)
        {
            var candidate = Path.Combine(directory.FullName, "desktop", "Ariadne.Desktop", "Views", "SettingsPageView.axaml");
            if (File.Exists(candidate))
            {
                viewPath = candidate;
                break;
            }
            directory = directory.Parent;
        }
        Assert.False(string.IsNullOrEmpty(viewPath));
        var view = File.ReadAllText(viewPath!);
        var autoModeBlockStart = view.IndexOf("IsChecked=\"{Binding AutoModeAutoApproval, Mode=TwoWay}\"", StringComparison.Ordinal);
        Assert.True(autoModeBlockStart >= 0);
        var autoModeBlock = view.Substring(autoModeBlockStart, Math.Min(420, view.Length - autoModeBlockStart));
        Assert.Contains("PolicyAutoOnText", autoModeBlock, StringComparison.Ordinal);
        Assert.Contains("PolicyAutoOffText", autoModeBlock, StringComparison.Ordinal);
        Assert.DoesNotContain("PolicyReviewText", autoModeBlock, StringComparison.Ordinal);
        Assert.DoesNotContain("PolicyAllowText", autoModeBlock, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding ApprovalPrompt, Mode=TwoWay}\"", view, StringComparison.Ordinal);
        Assert.Contains("ApprovalPromptPlaceholder", view, StringComparison.Ordinal);

        var names = DisplayNameService.LoadDefault();
        var autoOn = names.Text("ui.settings.automation.confirmation.auto_on");
        var autoOff = names.Text("ui.settings.automation.confirmation.auto_off");
        Assert.Contains("审计", autoOn, StringComparison.Ordinal);
        Assert.Contains("跳过", autoOff, StringComparison.Ordinal);
        Assert.DoesNotContain("审核", autoOff, StringComparison.Ordinal);
    }

    [Fact]
    public void SpectrumPopupAnchor_PrefersBottomLeftThenBottomRight()
    {
        // 纯几何：左下优先；左侧溢出时改右下
        Assert.Equal("bl", SpectrumPopupAnchor.ChooseCorner(
            swatchLeft: 40, swatchRight: 80, swatchBottom: 400,
            popupW: 280, popupH: 260, viewW: 800, viewH: 600));
        Assert.Equal("br", SpectrumPopupAnchor.ChooseCorner(
            swatchLeft: 20, swatchRight: 60, swatchBottom: 400,
            popupW: 280, popupH: 260, viewW: 200, viewH: 600));
    }

    [Fact]
    public void NormalizeHexForSnapshot_Uppercases()
    {
        Assert.Equal("#8A8F98", SettingsDirtyHelper.NormalizeHexForSnapshot("#8a8f98"));
        Assert.Equal("#F59E0B", SettingsDirtyHelper.NormalizeHexForSnapshot("f59e0b"));
    }

    [Fact]
    public void SystemTheme_DemoSwatchIsNotUnusableBlack()
    {
        var (main, surface, brand) = ThemeCatalog.SystemDemoSwatches();
        Assert.False(ThemeCatalog.IsUnusableDemoSwatch(main, surface));
        // surface 必须明显亮于纯黑
        Assert.True(surface.R > 40 || surface.G > 40 || surface.B > 40);
        Assert.True(ColorChannelEditor.TryParseHex(ThemeApplication.ToHex(brand), out _, out _, out _));
    }

    [Fact]
    public void SelectActiveCustomColors_PicksDarkWhenFollowAndDark()
    {
        var selected = ThemeApplication.SelectActiveCustomColors(
            isDark: true,
            followSystemColors: true,
            mainLight: "#FFFFFF",
            surfaceLight: "#EEEEEE",
            brandLight: "#112233",
            mainDark: "#111111",
            surfaceDark: "#222222",
            brandDark: "#AABBCC");
        Assert.Equal("#111111", selected.Main);
        Assert.Equal("#222222", selected.Surface);
        Assert.Equal("#AABBCC", selected.Brand);
    }

    [Fact]
    public void SelectActiveCustomColors_FallsBackToLightWhenDarkMissing()
    {
        var selected = ThemeApplication.SelectActiveCustomColors(
            isDark: true,
            followSystemColors: true,
            mainLight: "#FFFFFF",
            surfaceLight: "#EEEEEE",
            brandLight: "#112233",
            mainDark: null,
            surfaceDark: "",
            brandDark: null);
        Assert.Equal("#FFFFFF", selected.Main);
        Assert.Equal("#EEEEEE", selected.Surface);
        Assert.Equal("#112233", selected.Brand);
    }

    [Fact]
    public void PromptCatalog_ResolvesWriterFromRealPromptList()
    {
        PromptCatalog.ResetCacheForTests();
        var prompt = PromptCatalog.ResolveNodePrompt("writer");
        Assert.False(string.IsNullOrWhiteSpace(prompt));
        Assert.Contains("Writer", prompt, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void PromptCatalog_ResolveFromMap_PrefersAgentPrompt()
    {
        var map = new Dictionary<string, PromptCatalog.PromptEntry>(StringComparer.Ordinal)
        {
            ["agent_prompt.outliner"] = new("OUTLINER_BODY", "d"),
            ["node_template.outliner.default"] = new("TEMPLATE_BODY", "d"),
        };
        Assert.Equal("OUTLINER_BODY", PromptCatalog.ResolveNodePromptFromMap("outliner", map));
        Assert.Equal(string.Empty, PromptCatalog.ResolveNodePromptFromMap("start", map));
    }

    [Theory]
    [InlineData("writer")]
    [InlineData("planner")]
    [InlineData("critic")]
    public void PromptCatalog_KnownAgents_NonEmptyFromShippedFile(string type)
    {
        PromptCatalog.ResetCacheForTests();
        Assert.False(string.IsNullOrWhiteSpace(PromptCatalog.ResolveNodePrompt(type)));
    }
}
