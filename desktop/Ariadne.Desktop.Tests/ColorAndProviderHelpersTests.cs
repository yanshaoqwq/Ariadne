using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>Exercises shipped color parse/format and provider id allocation helpers.</summary>
public sealed class ColorAndProviderHelpersTests
{
    [Theory]
    [InlineData("#2E726B", 0x2E, 0x72, 0x6B)]
    [InlineData("2E726B", 0x2E, 0x72, 0x6B)]
    [InlineData("#ABC", 0xAA, 0xBB, 0xCC)]
    [InlineData("0xFF8800", 0xFF, 0x88, 0x00)]
    public void TryParseHex_AcceptsCommonForms(string input, int r, int g, int b)
    {
        Assert.True(ColorChannelEditor.TryParseHex(input, out var pr, out var pg, out var pb));
        Assert.Equal((byte)r, pr);
        Assert.Equal((byte)g, pg);
        Assert.Equal((byte)b, pb);
    }

    [Theory]
    [InlineData("")]
    [InlineData("zzz")]
    [InlineData("#12")]
    [InlineData(null)]
    public void TryParseHex_RejectsInvalid(string? input)
    {
        Assert.False(ColorChannelEditor.TryParseHex(input, out _, out _, out _));
    }

    [Fact]
    public void ColorChannelEditor_RgbSlidersUpdateHexAndPreview()
    {
        var editor = new ColorChannelEditor();
        editor.R = 46;
        editor.G = 114;
        editor.B = 107;
        Assert.Equal("#2E726B", editor.Hex);
        Assert.Equal("#2E726B", editor.ToHexValue());
        Assert.Contains("46", editor.RgbSummary);
    }

    [Fact]
    public void ColorPaletteMap_BuildHexMap_IsHueByLightnessGrid()
    {
        var map = ColorPaletteMap.BuildHexMap(hueSteps: 12, lightSteps: 5);
        // 至少 12×5 + 每行灰 + 常用色
        Assert.True(map.Count >= 12 * 5 + 5);
        Assert.Equal(13, ColorPaletteMap.Columns(12));
        Assert.All(map, hex => Assert.True(ColorChannelEditor.TryParseHex(hex, out _, out _, out _)));
        // 红系浅色应偏红
        Assert.StartsWith("#", ColorPaletteMap.HslToHex(0, 0.7, 0.5));
        var red = ColorPaletteMap.HslToHex(0, 0.8, 0.45);
        Assert.True(ColorChannelEditor.TryParseHex(red, out var r, out var g, out var b));
        Assert.True(r > g && r > b);
    }

    [Fact]
    public void ColorChannelEditor_HexInputUpdatesChannels()
    {
        var editor = new ColorChannelEditor();
        editor.Hex = "#F59E0B";
        Assert.Equal(245, editor.R);
        Assert.Equal(158, editor.G);
        Assert.Equal(11, editor.B);
    }

    [Fact]
    public void ProviderIdAllocator_AvoidsCollisions()
    {
        var existing = new[] { "provider", "provider_2", "openai" };
        var id = ProviderIdAllocator.Allocate(existing, "provider");
        Assert.Equal("provider_3", id);
        Assert.DoesNotContain(id, existing, StringComparer.OrdinalIgnoreCase);

        var openAi = ProviderIdAllocator.Allocate(existing, "openai");
        Assert.Equal("openai_2", openAi);
    }

    [Fact]
    public void ProviderIdAllocator_SanitizesPreferredBase()
    {
        var id = ProviderIdAllocator.Allocate(Array.Empty<string>(), "My Provider!");
        Assert.Equal("my_provider_", id);
        Assert.DoesNotContain(" ", id);
        Assert.DoesNotContain("!", id);
    }

    [Theory]
    [InlineData(true, true)]
    [InlineData(false, false)]
    public void PreferFormSnapshotOverConfig_MatchesPresence(bool hasSnapshot, bool expected)
    {
        Assert.Equal(expected, ProviderFormResolver.PreferFormSnapshotOverConfig(hasSnapshot));
    }

    [Theory]
    [InlineData("长夜行", "长夜行")]
    [InlineData("  My Novel  ", "My-Novel")]
    [InlineData("a/b\\c:d", "a-b-c-d")]
    [InlineData("...", "new-project")]
    [InlineData("CON", "CON-project")]
    public void SanitizeFolderName_ProducesSafeFolder(string input, string expected)
    {
        Assert.Equal(expected, ProjectPathHelper.SanitizeFolderName(input));
    }

    [Fact]
    public void BuildUniqueProjectRoot_NestsUnderParent()
    {
        var parent = Path.GetTempPath().TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
        var root = ProjectPathHelper.BuildUniqueProjectRoot(parent, "Demo Book");
        Assert.StartsWith(parent, root);
        Assert.Contains("Demo-Book", root);
        Assert.False(string.Equals(root, parent, StringComparison.OrdinalIgnoreCase));
    }

    [Fact]
    public void LooksLikeInitializedProject_RequiresProjectIdentityConfig()
    {
        var temp = Path.Combine(Path.GetTempPath(), "ariadne-ux-test-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(temp);
        try
        {
            Assert.False(ProjectPathHelper.LooksLikeInitializedProject(temp));
            Directory.CreateDirectory(Path.Combine(temp, ".config"));
            Assert.False(ProjectPathHelper.LooksLikeInitializedProject(temp));
            File.WriteAllText(Path.Combine(temp, ".config", "app.yaml"), "project_name: Test");
            Assert.True(ProjectPathHelper.LooksLikeInitializedProject(temp));
            Assert.False(ProjectPathHelper.LooksLikeInitializedProject(null));
            Assert.False(ProjectPathHelper.LooksLikeInitializedProject("/no/such/path/zzz"));
        }
        finally
        {
            try { Directory.Delete(temp, recursive: true); } catch { /* ignore */ }
        }
    }

    [Theory]
    [InlineData("/home/me/draft/第一章.md", "第一章", "第一章", "documents/第一章.md")]
    [InlineData("/books/ch2.txt", "ch2", "ch2", "documents/ch2.txt")]
    [InlineData("/tmp/My Chapter Draft.md", "My_Chapter_Draft", "My Chapter Draft", "documents/My Chapter Draft.md")]
    public void SuggestFromSourcePath_FillsAuthorFriendlyFields(
        string source,
        string expectedId,
        string expectedTitle,
        string expectedTarget)
    {
        var s = WorksImportHelper.SuggestFromSourcePath(source, existingTreeCount: 3);
        Assert.Equal(expectedId, s.ChapterId);
        Assert.Equal(expectedTitle, s.ChapterTitle);
        Assert.Equal(expectedTarget, s.TargetPath);
        Assert.Equal(3m, s.Order);
    }

    [Fact]
    public void ApplySuggestionIfEmpty_DoesNotClobberFilledFields()
    {
        var suggestion = WorksImportHelper.SuggestFromSourcePath("/tmp/auto.md", 1);
        var id = "keep-id";
        var title = "Keep Title";
        var target = "documents/keep.md";
        decimal? order = 9m;
        WorksImportHelper.ApplySuggestionIfEmpty(suggestion, ref id, ref title, ref target, ref order);
        Assert.Equal("keep-id", id);
        Assert.Equal("Keep Title", title);
        Assert.Equal("documents/keep.md", target);
        Assert.Equal(9m, order);

        id = "";
        title = "";
        target = "";
        order = 0m;
        WorksImportHelper.ApplySuggestionIfEmpty(suggestion, ref id, ref title, ref target, ref order);
        Assert.Equal(suggestion.ChapterId, id);
        Assert.Equal(suggestion.ChapterTitle, title);
        Assert.Equal(suggestion.TargetPath, target);
        Assert.Equal(1m, order);
    }

    [Fact]
    public void AppendPathLine_DedupesAndAppends()
    {
        var text = SettingsPageViewModel.AppendPathLine("", "/a");
        text = SettingsPageViewModel.AppendPathLine(text, "/b");
        text = SettingsPageViewModel.AppendPathLine(text, "/a");
        Assert.Equal("/a" + Environment.NewLine + "/b", text);
    }

    [Theory]
    [InlineData("30000", "30")]
    [InlineData("1500", "1.5")]
    [InlineData("0", "0")]
    [InlineData("", "")]
    [InlineData(null, "")]
    public void NodeTimeoutHelper_FormatSecondsFromMs(string? ms, string expected)
    {
        Assert.Equal(expected, NodeTimeoutHelper.FormatSecondsFromMs(ms));
    }

    [Theory]
    [InlineData("30", "30000")]
    [InlineData("1.5", "1500")]
    [InlineData("0", "0")]
    [InlineData("", "")]
    [InlineData(null, "")]
    public void NodeTimeoutHelper_ParseSecondsToMs_RoundTrip(string? seconds, string expectedMs)
    {
        Assert.Equal(expectedMs, NodeTimeoutHelper.ParseSecondsToMs(seconds));
        if (!string.IsNullOrEmpty(expectedMs) && long.TryParse(expectedMs, out _))
        {
            // 秒→ms→秒 不丢精度（整数毫秒）
            Assert.Equal(
                NodeTimeoutHelper.FormatSecondsFromMs(expectedMs),
                NodeTimeoutHelper.FormatSecondsFromMs(NodeTimeoutHelper.ParseSecondsToMs(seconds)));
        }
    }

    [Fact]
    public void NodeTimeoutHelper_ParseNullable_RejectsGarbage()
    {
        Assert.Null(NodeTimeoutHelper.ParseNullableDouble("nope"));
        Assert.Null(NodeTimeoutHelper.ParseNullableLongMs("abc"));
        Assert.Equal(12.5, NodeTimeoutHelper.ParseNullableDouble("12.5"));
        Assert.Equal(30000L, NodeTimeoutHelper.ParseNullableLongMs("30000"));
    }

    [Theory]
    [InlineData("/home/me/proj/documents/ch1.md", "documents/ch1.md")]
    [InlineData("documents/ch1.md", "documents/ch1.md")]
    [InlineData("/x/planning/outline.md", "planning/outline.md")]
    [InlineData("C:/proj/workflows/a.json", "workflows/a.json")]
    [InlineData("", "")]
    public void ToProjectRelativePath_StripsToProjectSegment(string input, string expected)
    {
        Assert.Equal(expected, ProjectPathHelper.ToProjectRelativePath(input));
    }

    [Fact]
    public void NodeConfigData_CaptureExtra_DropsUiOwnedKeepsOpaque()
    {
        var loaded = new Dictionary<string, object?>
        {
            ["name"] = "writer-1",
            ["prompt_template"] = "write {{input}}",
            ["provider_id"] = "provider-a",
            ["model_id"] = "gpt",
            ["tool_enabled"] = new Dictionary<string, object?> { ["search"] = true, ["web"] = false },
            ["input_aliases"] = new Dictionary<string, object?> { ["input"] = "body" },
            ["approval_policy"] = "manual_review",
            ["skills"] = new[] { "outline", "style" },
            ["temperature"] = 0.7,
        };

        var extra = NodeConfigData.CaptureExtra(loaded);
        Assert.False(extra.ContainsKey("name"));
        Assert.False(extra.ContainsKey("prompt_template"));
        Assert.False(extra.ContainsKey("provider_id"));
        Assert.False(extra.ContainsKey("model_id"));
        Assert.True(extra.ContainsKey("tool_enabled"));
        Assert.True(extra.ContainsKey("input_aliases"));
        Assert.Equal("manual_review", extra["approval_policy"]);
        Assert.Equal(0.7, extra["temperature"]);
    }

    [Fact]
    public void NodeConfigData_MergeUiFields_RoundTripsOpaqueThroughLoadSavePath()
    {
        // 模拟 CreateNodeFromCanvas 读到的完整 data → CaptureExtra → 作者改 UI → MergeUiFields（即 ToData）
        var loaded = new Dictionary<string, object?>
        {
            ["name"] = "old-name",
            ["work_dir"] = "novels/old",
            ["prompt_template"] = "old prompt",
            ["budget_usd"] = "0",
            ["timeout_ms"] = "60000",
            ["tool_enabled"] = new Dictionary<string, object?> { ["search"] = true },
            ["input_aliases"] = new Dictionary<string, object?> { ["input"] = "chapter_body" },
            ["approval_policy"] = "allow_by_default",
            ["temperature"] = 0.4,
        };

        var extra = NodeConfigData.CaptureExtra(loaded);
        var saved = NodeConfigData.MergeUiFields(
            extra,
            name: "renamed-node",
            workDir: "novels/新篇",
            userNote: "备注给 AI",
            isStartNode: false,
            exposedAsTool: false,
            promptTemplate: "new prompt",
            modelId: "claude",
            budgetUsd: "2.5",
            timeoutMs: "30000",
            breakpointEnabled: true,
            providerId: "anthropic-main");

        Assert.Equal("renamed-node", saved["name"]);
        Assert.Equal("novels/新篇", saved["work_dir"]);
        Assert.Equal("备注给 AI", saved["user_note"]);
        Assert.Equal("new prompt", saved["prompt_template"]);
        Assert.Equal("claude", saved["model_id"]);
        Assert.Equal("anthropic-main", saved["provider_id"]);
        // F13：必须写出 JSON number，才能被 core WorkflowLlmNodeConfig (f64/u64) 反序列化。
        Assert.Equal(2.5, Convert.ToDouble(saved["budget_usd"]));
        Assert.Equal(30000L, Convert.ToInt64(saved["timeout_ms"]));
        Assert.Equal(true, saved["breakpoint"]);
        // opaque 必须原样保留（同一引用或等价值）
        Assert.Same(loaded["tool_enabled"], saved["tool_enabled"]);
        Assert.Same(loaded["input_aliases"], saved["input_aliases"]);
        Assert.Equal("allow_by_default", saved["approval_policy"]);
        Assert.Equal(0.4, saved["temperature"]);
        Assert.False(saved.ContainsKey("expose_as_tool"));
    }

    [Fact]
    public void WorkflowNodeViewModel_ToData_PreservesOpaqueAfterRetain()
    {
        // 走真实 VM 入口：RetainOpaqueData + ToData（与 SaveWorkflowGraph / ApplyNodeConfig 同路径）
        // backend 仅存字段，ToData 不调用 IPC
        var node = new WorkflowNodeViewModel(
            "llm-1",
            "llm",
            "LLM",
            defaultWorkDir: string.Empty,
            x: 10,
            y: 20,
            runRequested: _ => { },
            clearSelection: () => { },
            markDirty: () => { })
        {
            Name = "chapter-writer",
            PromptTemplate = "write well",
            ModelId = "m",
            BudgetUsd = "1",
            TimeoutMs = "12000",
        };

        var toolEnabled = new Dictionary<string, object?> { ["search"] = true };
        var aliases = new Dictionary<string, object?> { ["input"] = "body" };
        node.RetainOpaqueData(new Dictionary<string, object?>
        {
            ["name"] = "ignored-ui",
            ["tool_enabled"] = toolEnabled,
            ["input_aliases"] = aliases,
            ["skills"] = "polisher",
        });

        var data = node.ToData();
        Assert.Equal("chapter-writer", data["name"]);
        Assert.Equal("write well", data["prompt_template"]);
        Assert.Same(toolEnabled, data["tool_enabled"]);
        Assert.Same(aliases, data["input_aliases"]);
        Assert.Equal("polisher", data["skills"]);
        // F13：ToData（Save/BuildGraph 同源）写出 number，供 core serde 消费
        Assert.Equal(1.0, Convert.ToDouble(data["budget_usd"]));
        Assert.Equal(12000L, Convert.ToInt64(data["timeout_ms"]));

        // BuildGraph 同源：CanvasNode.Data 必须带 opaque
        var canvas = node.ToCanvasNode();
        Assert.Same(toolEnabled, canvas.Data["tool_enabled"]);
        Assert.Same(aliases, canvas.Data["input_aliases"]);
        Assert.Equal("chapter-writer", canvas.Label);
        Assert.Equal(1.0, Convert.ToDouble(canvas.Data["budget_usd"]));
        Assert.Equal(12000L, Convert.ToInt64(canvas.Data["timeout_ms"]));
    }

    [Fact]
    public void TryMakeRelativeToProjectRoot_AcceptsUnderRoot_RejectsOutside()
    {
        var root = Path.Combine(Path.GetTempPath(), "ariadne-rel-" + Guid.NewGuid().ToString("N"));
        var nested = Path.Combine(root, "novels", "正篇");
        Directory.CreateDirectory(nested);
        try
        {
            Assert.True(ProjectPathHelper.TryMakeRelativeToProjectRoot(nested, root, out var rel));
            Assert.Equal("novels/正篇", rel.Replace('\\', '/'));

            Assert.True(ProjectPathHelper.TryMakeRelativeToProjectRoot(root, root, out var rootRel));
            Assert.Equal(".", rootRel);

            var outside = Path.Combine(Path.GetTempPath(), "ariadne-out-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(outside);
            try
            {
                Assert.False(ProjectPathHelper.TryMakeRelativeToProjectRoot(outside, root, out _));
            }
            finally
            {
                try { Directory.Delete(outside, recursive: true); } catch { /* ignore */ }
            }

            // 前缀陷阱：/proj 不得匹配 /project
            var sibling = root + "-extra";
            Directory.CreateDirectory(sibling);
            try
            {
                Assert.False(ProjectPathHelper.TryMakeRelativeToProjectRoot(sibling, root, out _));
            }
            finally
            {
                try { Directory.Delete(sibling, recursive: true); } catch { /* ignore */ }
            }
        }
        finally
        {
            try { Directory.Delete(root, recursive: true); } catch { /* ignore */ }
        }
    }

    [Fact]
    public void TryMakeRelativeToProjectRoot_RejectsSymlinkEscape()
    {
        if (OperatingSystem.IsWindows())
        {
            return;
        }

        var root = Directory.CreateTempSubdirectory("ariadne-rel-root-");
        var outside = Directory.CreateTempSubdirectory("ariadne-rel-outside-");
        try
        {
            Directory.CreateSymbolicLink(Path.Combine(root.FullName, "linked"), outside.FullName);
            Assert.False(ProjectPathHelper.TryMakeRelativeToProjectRoot(
                Path.Combine(root.FullName, "linked"),
                root.FullName,
                out _));
        }
        finally
        {
            root.Delete(recursive: true);
            outside.Delete(recursive: true);
        }
    }

    [Fact]
    public void ResolveRevealDirectory_FileUriAndParent()
    {
        var root = Path.Combine(Path.GetTempPath(), "ariadne-reveal-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        var file = Path.Combine(root, "combined-markdown.md");
        File.WriteAllText(file, "x");
        try
        {
            var fromFile = ProjectPathHelper.ResolveRevealDirectory(file);
            Assert.Equal(Path.GetFullPath(root), fromFile);

            var fileUri = new Uri(file).AbsoluteUri;
            var fromUri = ProjectPathHelper.ResolveRevealDirectory(fileUri);
            Assert.Equal(Path.GetFullPath(root), fromUri);

            var fromDir = ProjectPathHelper.ResolveRevealDirectory(root);
            Assert.Equal(Path.GetFullPath(root), fromDir);

            Assert.Null(ProjectPathHelper.ResolveRevealDirectory(""));
            Assert.Null(ProjectPathHelper.ResolveRevealDirectory(null));
        }
        finally
        {
            try { Directory.Delete(root, recursive: true); } catch { /* ignore */ }
        }
    }

    [Theory]
    [InlineData("1.5", 1.5)]
    [InlineData("$2.25", 2.25)]
    [InlineData("300000", 300000)]
    [InlineData("", -1)] // empty → fallback
    [InlineData("nope", -1)]
    public void CultureNumberParse_ParseDouble_InvariantFirst(string input, double expected)
    {
        Assert.Equal(expected, CultureNumberParse.ParseDouble(input, fallback: -1), precision: 5);
    }

    [Fact]
    public void CultureNumberParse_ParseLong_AcceptsIntegerText()
    {
        Assert.Equal(300000L, CultureNumberParse.ParseLong("300000", 0));
        Assert.Equal(0L, CultureNumberParse.ParseLong("abc", 0));
        Assert.Equal(42L, CultureNumberParse.ParseLong("42.0", -1));
    }

    [Theory]
    [InlineData(246, 247, 246, true)]
    [InlineData(255, 255, 255, true)]
    [InlineData(53, 111, 104, false)]
    [InlineData(20, 40, 50, false)]
    public void AppIconRecolor_IsPaperPixel(int r, int g, int b, bool expected)
    {
        Assert.Equal(expected, AppIconRecolor.IsPaperPixel((byte)r, (byte)g, (byte)b));
    }

    [Fact]
    public void AppIconRecolor_MapPixel_InkGoesToAccent()
    {
        // 母版青绿墨线 → 自定义强调色
        var (nr, ng, nb, na) = AppIconRecolor.MapPixel(
            r: 53, g: 111, b: 104, a: 255,
            accentR: 200, accentG: 40, accentB: 40,
            paperR: 246, paperG: 247, paperB: 246);
        Assert.Equal(255, na);
        // 应明显偏红强调色，而非纸白
        Assert.True(nr > 100, $"expected red-leaning ink, got {nr},{ng},{nb}");
        Assert.True(nr > ng && nr > nb);
    }

    [Fact]
    public void AppIconRecolor_MapPixel_PaperStaysPaper()
    {
        var (nr, ng, nb, na) = AppIconRecolor.MapPixel(
            r: 246, g: 247, b: 246, a: 255,
            accentR: 200, accentG: 40, accentB: 40,
            paperR: 240, paperG: 238, paperB: 230);
        Assert.Equal(255, na);
        Assert.Equal(240, nr);
        Assert.Equal(238, ng);
        Assert.Equal(230, nb);
    }

    [Fact]
    public void AppIconRecolor_MapPixel_TransparentPaperClearsPlate()
    {
        // 任务栏：paperA=0 → 纸面像素完全透明，不出现底板
        var (nr, ng, nb, na) = AppIconRecolor.MapPixel(
            r: 246, g: 247, b: 246, a: 255,
            accentR: 53, accentG: 111, accentB: 104,
            paperR: 246, paperG: 247, paperB: 246,
            paperA: 0);
        Assert.Equal(0, na);
        Assert.Equal(0, nr);
        Assert.Equal(0, ng);
        Assert.Equal(0, nb);
    }

    [Fact]
    public void AppIconDesktopSync_EnumerateOutputPngPaths_IncludesUserDirs()
    {
        var paths = AppIconDesktopSync.EnumerateOutputPngPaths();
        Assert.NotEmpty(paths);
        Assert.All(paths, p => Assert.Contains("ariadne", p, StringComparison.OrdinalIgnoreCase));
        Assert.Contains(paths, p => p.EndsWith($"{Path.DirectorySeparatorChar}ariadne.png", StringComparison.Ordinal)
                                    || p.EndsWith("/ariadne.png", StringComparison.Ordinal));
    }

    [Fact]
    public void AppIconDesktopSync_MacPaths_AreUnderLibraryApplicationSupport()
    {
        var macDir = AppIconDesktopSync.GetMacApplicationSupportIconsDir();
        Assert.Contains("Application Support", macDir, StringComparison.Ordinal);
        Assert.Contains(Path.Combine("Ariadne", "icons"), macDir);
        var icns = AppIconDesktopSync.GetMacIcnsPath();
        Assert.EndsWith("ariadne.icns", icns, StringComparison.Ordinal);
        var bundleIcon = AppIconDesktopSync.GetMacUserAppBundleIconPath();
        Assert.Contains(Path.Combine("Applications", "Ariadne.app"), bundleIcon);
        Assert.EndsWith("AppIcon.icns", bundleIcon, StringComparison.Ordinal);
    }

    [Theory]
    [InlineData("ariadne-256.png", 256)]
    [InlineData("ariadne.png", 512)]
    public void AppIconDesktopSync_GuessSizeFromPath(string fileName, int expected)
    {
        var path = Path.Combine("icons", fileName);
        Assert.Equal(expected, AppIconDesktopSync.GuessSizeFromPath(path));
    }

    [Fact]
    public void AppIconDesktopSync_GuessSizeFromHicolorPath()
    {
        var path = Path.Combine("hicolor", "128x128", "apps", "ariadne.png");
        Assert.Equal(128, AppIconDesktopSync.GuessSizeFromPath(path));
    }

    [Fact]
    public void WorkflowExportSelection_FullExportIgnoresSelection()
    {
        var all = new[] { "a", "b", "c" };
        // 工具栏导出图：即使有选中，也必须导出全部
        var ids = WorkflowExportSelection.ResolveNodeIds(
            requireSelection: false,
            selectedNodeId: "b",
            allNodeIds: all);
        Assert.Equal(all, ids);
    }

    [Fact]
    public void WorkflowExportSelection_SelectionExportUsesOnlySelected()
    {
        var all = new[] { "a", "b", "c" };
        var ids = WorkflowExportSelection.ResolveNodeIds(
            requireSelection: true,
            selectedNodeId: "b",
            allNodeIds: all);
        Assert.Equal(new[] { "b" }, ids);

        var empty = WorkflowExportSelection.ResolveNodeIds(
            requireSelection: true,
            selectedNodeId: null,
            allNodeIds: all);
        Assert.Empty(empty);
    }

    [Fact]
    public void ProviderOption_FormSnapshot_RetainsDraftFields()
    {
        var option = new ProviderOptionViewModel(
            "provider_2",
            "新供应商",
            "未配置",
            isDraft: true);
        option.CaptureForm(new ProviderFormSnapshot
        {
            ProviderId = "provider_2",
            ProviderType = "anthropic",
            DisplayName = "Claude 草稿",
            BaseUrl = "https://example.test",
            Enabled = true,
            MakeDefaultLlm = true,
            MakeDefaultEmbedding = false,
            MakeDefaultReranker = false,
            MakeDefaultSearch = false,
            ModelsText = "claude-3,chat",
            EmbeddingModelId = string.Empty,
        });

        Assert.True(option.HasFormSnapshot);
        Assert.Equal("Claude 草稿", option.DisplayName);
        var peek = option.PeekForm();
        Assert.NotNull(peek);
        Assert.Equal("anthropic", peek!.ProviderType);
        Assert.Equal("https://example.test", peek.BaseUrl);
        Assert.Equal("claude-3,chat", peek.ModelsText);
        Assert.True(peek.MakeDefaultLlm);

        // 丢弃脏编辑时不应覆盖：再 Capture 新快照才变
        option.CaptureForm(new ProviderFormSnapshot
        {
            ProviderId = "provider_2",
            ProviderType = "open_ai_compatible",
            DisplayName = "重置",
            BaseUrl = string.Empty,
            Enabled = false,
            MakeDefaultLlm = false,
            MakeDefaultEmbedding = false,
            MakeDefaultReranker = false,
            MakeDefaultSearch = false,
            ModelsText = string.Empty,
            EmbeddingModelId = string.Empty,
        });
        Assert.Equal("open_ai_compatible", option.PeekForm()!.ProviderType);
        Assert.Equal("重置", option.DisplayName);
    }
}
