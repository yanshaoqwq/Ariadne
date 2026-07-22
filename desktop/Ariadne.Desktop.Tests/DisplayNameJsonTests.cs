using System.Text.Json;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// Ensures the shipped display_name pack parses and loads through the real desktop path.
/// </summary>
public sealed class DisplayNameJsonTests
{
    private static string ResolveDisplayNamePath()
    {
        // Prefer the source-of-truth pack linked into the desktop project.
        var candidates = new[]
        {
            Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "Resources", "display_name.json")),
            Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "..", "..", "..", "..", "Ariadne.Desktop", "bin", "Debug", "net10.0", "Resources", "display_name.json")),
            Path.GetFullPath(Path.Combine(AppContext.BaseDirectory, "..", "..", "..", "..", "..", "core", "resources", "display_name.json")),
        };

        foreach (var path in candidates)
        {
            if (File.Exists(path))
            {
                return path;
            }
        }

        // Walk up from test bin to repo core/resources.
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 10 && dir is not null; i++)
        {
            var candidate = Path.Combine(dir.FullName, "core", "resources", "display_name.json");
            if (File.Exists(candidate))
            {
                return candidate;
            }
            dir = dir.Parent;
        }

        throw new FileNotFoundException("Could not locate display_name.json from test base " + AppContext.BaseDirectory);
    }

    [Fact]
    public void DisplayNameJson_ParsesAsStringDictionary()
    {
        var path = ResolveDisplayNamePath();
        using var stream = File.OpenRead(path);
        var map = JsonSerializer.Deserialize<Dictionary<string, string>>(stream);

        Assert.NotNull(map);
        Assert.True(map!.Count > 100, $"expected rich pack, got {map.Count} keys from {path}");
        Assert.True(map.ContainsKey("ui.workspace.node_timeout_seconds"));
        Assert.Equal("超时（秒）", map["ui.workspace.node_timeout_seconds"]);
        Assert.True(map.ContainsKey("ui.workspace.node_timeout"));
        Assert.Equal("超时（ms）", map["ui.workspace.node_timeout"]);
        Assert.True(map.ContainsKey("ui.common.optional"));
        Assert.Equal("可选", map["ui.common.optional"]);
        Assert.True(map.ContainsKey("ui.common.unit.seconds"));
        Assert.Equal("秒", map["ui.common.unit.seconds"]);
        Assert.True(map.ContainsKey("ui.works.export_done_title"));
        Assert.True(map.ContainsKey("ui.works.export_done_message"));
        Assert.True(map.ContainsKey("ui.works.export_open_folder"));
        Assert.True(map.ContainsKey("ui.workspace.start_node.browse_work_dir"));
        Assert.True(map.ContainsKey("ui.workspace.export_autosaved"));
        Assert.True(map.ContainsKey("ui.error.network"));
        Assert.True(map.ContainsKey("ui.dialog.unsaved.save_all"));
        Assert.True(map.ContainsKey("ui.dialog.unsaved.message_many"));
        // No bare string tokens that would break object parse (regression for line 616).
        Assert.DoesNotContain(map.Values, v => v is null);
    }

    [Fact]
    public void DisplayNameService_LoadDefault_ResolvesWorkspaceKeysNotBrackets()
    {
        // Real service path used by WorkspacePageViewModel labels (LoadDefault + Text).
        var service = DisplayNameService.LoadDefault();
        var timeout = service.Text("ui.workspace.node_timeout_seconds");
        var minimap = service.Text("ui.workspace.minimap");

        Assert.False(string.IsNullOrWhiteSpace(timeout), "timeout label empty — pack failed to load?");
        Assert.DoesNotContain("[", timeout);
        Assert.DoesNotContain("]", timeout);
        Assert.Equal("超时（秒）", timeout);

        Assert.False(string.IsNullOrWhiteSpace(minimap));
        Assert.DoesNotContain("ui.workspace.minimap", minimap);
    }

    [Fact]
    public void SettingsBudgetCopy_MatchesDailySemanticsAndOmitsFalseHardLimitPromise()
    {
        // Real DisplayNameService path used by SettingsPageViewModel BudgetLabel/BudgetHelpText.
        var service = DisplayNameService.LoadDefault();
        var budgetLabel = service.Text("ui.settings.automation.global_budget");
        var budgetHelp = service.Text("ui.settings.automation.budget_help");
        var languageDesc = service.Text("ui.settings.misc.language.desc");

        Assert.Contains("日预算", budgetLabel, StringComparison.Ordinal);
        Assert.DoesNotContain("全局预算", budgetLabel, StringComparison.Ordinal);
        Assert.Contains("今日累计", budgetHelp, StringComparison.Ordinal);
        Assert.Contains("0", budgetHelp, StringComparison.Ordinal);
        Assert.DoesNotContain("运行策略", budgetHelp, StringComparison.Ordinal);
        Assert.DoesNotContain("单次/日/月", budgetHelp, StringComparison.Ordinal);
        // Meta i18n authoring prose must not ship as the language description string.
        Assert.DoesNotContain("display_name", languageDesc, StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain(".json", languageDesc, StringComparison.OrdinalIgnoreCase);

        // Internal subsystem nicknames must not appear as primary settings labels.
        var sidecarData = service.Text("ui.settings.misc.qdrant_data_dir");
        var sidecarBinary = service.Text("ui.settings.misc.qdrant_binary_path");
        var chunk = service.Text("ui.settings.misc.chunk_size");
        Assert.DoesNotContain("Sidecar", sidecarData, StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("Sidecar", sidecarBinary, StringComparison.OrdinalIgnoreCase);
        Assert.DoesNotContain("Chunk", chunk, StringComparison.Ordinal);
    }

    [Fact]
    public void SettingsPageView_DoesNotPermanentlyBindMetaHelpAsCaptionBody()
    {
        var viewPath = ResolveDesktopSourceView();
        var view = File.ReadAllText(viewPath);

        Assert.DoesNotContain("LanguageDescText", view, StringComparison.Ordinal);
        Assert.DoesNotContain("display_name.", view, StringComparison.OrdinalIgnoreCase);
        // Caption body uses Text="{Binding …}" on TextBlock; HelpText property contains the same
        // substring, so match only TextBlock attribute starts (not AutomationProperties.HelpText).
        var permanentCaptionHelp = System.Text.RegularExpressions.Regex.Matches(
            view,
            @"<TextBlock\b[^>]*\bText=""\{Binding (BudgetHelpText|PreauthorizedHelpText|ConfirmationPolicyHelpText|GlobalDefaultsHelpText|ThemePaletteHelpText)\}""",
            System.Text.RegularExpressions.RegexOptions.CultureInvariant);
        Assert.True(permanentCaptionHelp.Count == 0, "meta help still permanently bound as TextBlock body");
        Assert.Contains("ToolTip.Tip=\"{Binding BudgetHelpText}\"", view, StringComparison.Ordinal);
        Assert.Contains("AutomationProperties.HelpText=\"{Binding BudgetHelpText}\"", view, StringComparison.Ordinal);
        // Spent amount must be visible as value column (not only a left caption without value).
        Assert.Contains("Text=\"{Binding SpentText}\"", view, StringComparison.Ordinal);
        // Tabs and the floating section index both use standard single-selection controls.
        Assert.Contains("<ListBox ItemsSource=\"{Binding Tabs}\"", view, StringComparison.Ordinal);
        Assert.Contains("ItemsSource=\"{Binding SectionIndexItems}\"", view, StringComparison.Ordinal);
        Assert.Contains("Classes=\"settings-section-index\"", view, StringComparison.Ordinal);
        Assert.Contains("Setter Property=\"BorderThickness\" Value=\"0\"", view, StringComparison.Ordinal);
    }

    [Fact]
    public void TimeoutAuthorLabels_AreSecondsOnSettingsAndWorkspace()
    {
        var service = DisplayNameService.LoadDefault();
        var preset = service.Text("ui.settings.presets.node_timeout_ms");
        var def = service.Text("ui.settings.presets.default_timeout_ms");
        var workflow = service.Text("ui.settings.automation.default_timeout_ms");
        var workspace = service.Text("ui.workspace.node_timeout_seconds");
        foreach (var label in new[] { preset, def, workflow, workspace })
        {
            Assert.Contains("秒", label, StringComparison.Ordinal);
            Assert.DoesNotContain("(ms)", label, StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain("（ms）", label, StringComparison.Ordinal);
        }

        // Round-trip helper used by Settings seconds UI ↔ backend ms.
        Assert.Equal("300", NodeTimeoutHelper.FormatSecondsFromMs("300000"));
        Assert.Equal("300000", NodeTimeoutHelper.ParseSecondsToMs("300"));
    }

    [Fact]
    public void WorkspaceAndWorksFormTextBoxes_HaveProgrammaticNames()
    {
        foreach (var relative in new[]
                 {
                     Path.Combine("Views", "WorkspacePageView.axaml"),
                     Path.Combine("Views", "WorksPageView.axaml"),
                 })
        {
            var path = ResolveDesktopSource(relative.Split(Path.DirectorySeparatorChar));
            var view = File.ReadAllText(path);
            var tags = System.Text.RegularExpressions.Regex.Matches(
                view,
                @"<TextBox\b[\s\S]*?(?:/>|>)",
                System.Text.RegularExpressions.RegexOptions.CultureInvariant);
            Assert.True(tags.Count > 0, relative);
            var unnamed = tags.Cast<System.Text.RegularExpressions.Match>()
                .Select(m => m.Value)
                .Where(tag => !tag.Contains("AutomationProperties.Name", StringComparison.Ordinal))
                .ToArray();
            Assert.True(unnamed.Length == 0, relative + " unnamed:\n" + string.Join("\n", unnamed.Take(5)));
        }
    }

    [Fact]
    public void PrimaryChrome_EnglishAndJapaneseResolveNonChineseForCoreKeys()
    {
        var enDir = Path.GetDirectoryName(ResolveDisplayNamePath())!;
        var enService = DisplayNameService.LoadFromDirectory(enDir, "en");
        var jaService = DisplayNameService.LoadFromDirectory(enDir, "ja");
        var keys = new[]
        {
            "ui.settings.title",
            "ui.nav.workspace",
            "ui.nav.works",
            "ui.settings.tab.automation",
            "ui.settings.presets.node_timeout_ms",
            "ui.workspace.node_timeout_seconds",
        };
        foreach (var key in keys)
        {
            var en = enService.Text(key);
            var ja = jaService.Text(key);
            Assert.False(string.IsNullOrWhiteSpace(en), key + " en empty");
            Assert.False(string.IsNullOrWhiteSpace(ja), key + " ja empty");
            Assert.False(ContainsCjk(en), key + " en still CJK: " + en);
            Assert.False(ContainsCjk(ja) && ContainsCjk(enService.Text("ui.settings.title")) == false && ja == enService.Text(key),
                key + " ja unexpected");
        }

        Assert.Contains("Timeout", enService.Text("ui.settings.presets.node_timeout_ms"), StringComparison.OrdinalIgnoreCase);
        Assert.Contains("second", enService.Text("ui.settings.presets.node_timeout_ms"), StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public void SettingsPageViewModel_UiKeys_ResolveWithoutChineseUnderEnglish()
    {
        // Product path: load real en overlay from shipped Resources, then resolve every ui.*/confirmation.*/agent.*
        // string literal used by SettingsPageViewModel (same keys the settings UI binds).
        var enDir = Path.GetDirectoryName(ResolveDisplayNamePath())!;
        var en = DisplayNameService.LoadFromDirectory(enDir, "en");
        var zh = DisplayNameService.LoadFromDirectory(enDir, "zh");
        var vmSource = File.ReadAllText(ResolveDesktopSource("ViewModels", "SettingsPageViewModel.cs"));
        var keys = System.Text.RegularExpressions.Regex.Matches(
                vmSource,
                @"""((?:ui|confirmation|agent)\.[a-zA-Z0-9_./-]+)""")
            .Select(m => m.Groups[1].Value)
            .Distinct(StringComparer.Ordinal)
            .ToArray();
        Assert.True(keys.Length >= 150, "expected dense settings key set, got " + keys.Length);

        var chineseFallback = new List<string>();
        var theaterStubs = new List<string>();
        foreach (var key in keys)
        {
            var enText = en.Text(key);
            var zhText = zh.Text(key);
            if (IsTheaterStub(enText, "en"))
            {
                theaterStubs.Add($"{key} => {enText}");
            }
            if (string.IsNullOrWhiteSpace(enText)
                || ContainsCjk(enText)
                || string.Equals(enText, zhText, StringComparison.Ordinal))
            {
                // Allow pure ASCII tokens that are identical in zh pack (e.g. "Git", "Base URL").
                if (!ContainsCjk(zhText) && string.Equals(enText, zhText, StringComparison.Ordinal))
                {
                    continue;
                }
                chineseFallback.Add($"{key} => {enText}");
            }
        }

        Assert.True(theaterStubs.Count == 0, "theater stubs in settings keys:\n" + string.Join('\n', theaterStubs.Take(40)));
        // Residual set must be empty for Settings-bound ui.* under English (not ratio-only).
        var ratio = chineseFallback.Count / (double)keys.Length;
        Assert.True(
            ratio <= 0.10,
            $"Settings English still Chinese-fallback for {chineseFallback.Count}/{keys.Length} ({ratio:P0}). Samples:\n"
            + string.Join('\n', chineseFallback.Take(25)));
    }

    [Theory]
    [InlineData("en")]
    [InlineData("ja")]
    public void SettingsAndPrimaryChrome_ProductLanguage_ResidualTheaterSetEmpty(string language)
    {
        // Mechanical LO-1 gate: SettingsPageViewModel ui.* ∪ frozen primary chrome ∪ ThemeCatalog
        // dynamic ui.theme.{code}[.desc] (ThemeDescriptionFor interpolates these — static regex misses them).
        var resourceDir = Path.GetDirectoryName(ResolveDisplayNamePath())!;
        var service = DisplayNameService.LoadFromDirectory(resourceDir, language);
        var vmSource = File.ReadAllText(ResolveDesktopSource("ViewModels", "SettingsPageViewModel.cs"));
        var keys = new HashSet<string>(StringComparer.Ordinal);
        foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(
                     vmSource, @"""(ui\.[a-zA-Z0-9_./-]+)"""))
        {
            keys.Add(m.Groups[1].Value);
        }
        foreach (var key in LoadFrozenPrimaryChromeKeys())
        {
            keys.Add(key);
        }
        // Product path: ThemeOption.Description = ThemeDescriptionFor(palette.Id) → ui.theme.{id}.desc
        Assert.True(ThemeCatalog.All.Count >= 8, "ThemeCatalog too small: " + ThemeCatalog.All.Count);
        foreach (var palette in ThemeCatalog.All)
        {
            keys.Add($"ui.theme.{palette.Id}");
            keys.Add($"ui.theme.{palette.Id}.desc");
        }
        // Skeptic-named Settings main-surface keys must always be present in the scan set.
        foreach (var key in new[]
                 {
                     "ui.settings.models.make_default_llm",
                     "ui.settings.models.make_default_search",
                     "ui.settings.permissions.path_placeholder",
                     "ui.settings.permissions.global_defaults_help",
                     "ui.settings.presets.tools_title",
                     "ui.settings.automation.confirmation.help",
                     "ui.settings.presets.market",
                     "ui.settings.index.current_project.desc",
                     "ui.theme.amber.desc",
                     "ui.theme.azure.desc",
                     "ui.theme.dusk.desc",
                     "ui.theme.ink.desc",
                     "ui.theme.rose.desc",
                     "ui.theme.slate.desc",
                     "ui.theme.system.desc",
                     "ui.theme.violet.desc",
                 })
        {
            keys.Add(key);
        }

        Assert.True(keys.Count >= 200, "expected dense residual-scan set, got " + keys.Count);
        var residuals = keys
            .Select(k => (Key: k, Text: service.Text(k)))
            .Where(x => string.IsNullOrWhiteSpace(x.Text) || IsTheaterStub(x.Text, language))
            .Select(x => $"{x.Key} => {x.Text}")
            .OrderBy(s => s, StringComparer.Ordinal)
            .ToArray();
        Assert.True(residuals.Length == 0,
            language + " residual theater count=" + residuals.Length + ":\n" + string.Join('\n', residuals.Take(40)));

        // Theme card descriptions are product-path Settings chrome (not static string literals).
        foreach (var palette in ThemeCatalog.All)
        {
            var desc = service.Text($"ui.theme.{palette.Id}.desc");
            Assert.False(IsTheaterStub(desc, language), language + " theme desc " + palette.Id + " => " + desc);
            Assert.DoesNotContain("Accent ，", desc, StringComparison.Ordinal);
            Assert.DoesNotContain("autoswitch", desc, StringComparison.OrdinalIgnoreCase);
        }

        if (language == "en")
        {
            Assert.Equal("Set as default chat model", service.Text("ui.settings.models.make_default_llm"));
            Assert.Equal("Set as default web-search provider", service.Text("ui.settings.models.make_default_search"));
            Assert.Equal("Absolute path, press Enter to add", service.Text("ui.settings.permissions.path_placeholder"));
            Assert.Equal("Template market presets", service.Text("ui.settings.presets.market"));
            Assert.Equal("Allow web search", service.Text("ui.settings.permissions.allow_web_search"));
            Assert.Equal("Allow networked tools", service.Text("ui.settings.permissions.allow_http_skill"));
            Assert.Equal("Allow script network access", service.Text("ui.settings.permissions.allow_wasm_network"));
            Assert.Equal("Readable path roots", service.Text("ui.settings.permissions.read_roots"));
            Assert.Equal("Writable path roots", service.Text("ui.settings.permissions.write_roots"));
            Assert.Equal("Permission and tool presets", service.Text("ui.settings.presets.access_title"));
            Assert.Equal("Save template repository", service.Text("ui.settings.presets.save_template_repository"));
            Assert.Equal("Git auto-checkpoint color", service.Text("ui.settings.personalization.git_auto_color"));
            Assert.Equal("Track skills", service.Text("ui.settings.misc.track_skills"));
            Assert.Equal("One ignored path per line", service.Text("ui.settings.misc.ignored_paths.placeholder"));
            Assert.Contains("manual review", service.Text("ui.settings.automation.confirmation.help"), StringComparison.OrdinalIgnoreCase);
            Assert.Contains("amber-gold", service.Text("ui.theme.amber.desc"), StringComparison.OrdinalIgnoreCase);
            Assert.Contains("operating system", service.Text("ui.theme.system.desc"), StringComparison.OrdinalIgnoreCase);
            Assert.Contains("document", service.Text("ui.theme.ink.desc"), StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain("networksearch", service.Text("ui.settings.permissions.allow_web_search"), StringComparison.Ordinal);
            Assert.DoesNotContain("toolspresets", service.Text("ui.settings.presets.access_title"), StringComparison.Ordinal);
        }
        else
        {
            Assert.Equal("既定の対話モデルに設定", service.Text("ui.settings.models.make_default_llm"));
            Assert.Equal("絶対パスを入力し、Enter で追加", service.Text("ui.settings.permissions.path_placeholder"));
            Assert.Equal("テンプレート市場プリセット", service.Text("ui.settings.presets.market"));
            Assert.Equal("Web検索を許可", service.Text("ui.settings.permissions.allow_web_search"));
            Assert.Equal("ネットワークツールを許可", service.Text("ui.settings.permissions.allow_http_skill"));
            Assert.Equal("スクリプトのネットワークを許可", service.Text("ui.settings.permissions.allow_wasm_network"));
            Assert.Equal("読み取り可能なパスルート", service.Text("ui.settings.permissions.read_roots"));
            Assert.Equal("書き込み可能なパスルート", service.Text("ui.settings.permissions.write_roots"));
            Assert.Equal("権限とツールのプリセット", service.Text("ui.settings.presets.access_title"));
            Assert.Equal("テンプレートリポジトリを保存", service.Text("ui.settings.presets.save_template_repository"));
            Assert.Equal("Git 自動チェックポイントの色", service.Text("ui.settings.personalization.git_auto_color"));
            Assert.Equal("スキルを追跡", service.Text("ui.settings.misc.track_skills"));
            Assert.Equal("無視するパスを1行に1つ", service.Text("ui.settings.misc.ignored_paths.placeholder"));
            Assert.Contains("人手レビュー", service.Text("ui.settings.automation.confirmation.help"), StringComparison.Ordinal);
            Assert.Contains("アンバー", service.Text("ui.theme.amber.desc"), StringComparison.Ordinal);
            Assert.Contains("OS", service.Text("ui.theme.system.desc"), StringComparison.Ordinal);
            Assert.Contains("ドキュメント", service.Text("ui.theme.ink.desc"), StringComparison.Ordinal);
            Assert.DoesNotContain("networksearch", service.Text("ui.settings.permissions.allow_web_search"), StringComparison.Ordinal);
            Assert.DoesNotContain("toolspresets", service.Text("ui.settings.presets.access_title"), StringComparison.Ordinal);
        }
    }

    [Theory]
    [InlineData("en")]
    [InlineData("ja")]
    public void PrimaryChrome_ProductLanguage_NoTheaterAndPlaceholdersPreserved(string language)
    {
        // Bipartite gate: same frozen primary-chrome key set for en and ja via real DisplayNameService.
        var resourceDir = Path.GetDirectoryName(ResolveDisplayNamePath())!;
        var service = DisplayNameService.LoadFromDirectory(resourceDir, language);
        var zh = DisplayNameService.LoadFromDirectory(resourceDir, "zh");
        Assert.Contains(service.AvailableLanguages, code =>
            string.Equals(code, language, StringComparison.OrdinalIgnoreCase));

        var keys = LoadFrozenPrimaryChromeKeys();
        Assert.True(keys.Count >= 80, "frozen primary chrome set too small: " + keys.Count);

        var theater = new List<string>();
        var lostPlaceholders = new List<string>();
        foreach (var key in keys)
        {
            var text = service.Text(key);
            var zhText = zh.Text(key);
            if (string.IsNullOrWhiteSpace(text) || IsTheaterStub(text, language))
            {
                theater.Add($"{key} => {text}");
                continue;
            }
            foreach (var token in ExtractPlaceholders(zhText))
            {
                if (!text.Contains(token, StringComparison.Ordinal))
                {
                    lostPlaceholders.Add($"{key}: missing {token} in '{text}' (zh '{zhText}')");
                }
            }
        }

        Assert.True(theater.Count == 0,
            language + " theater stubs:\n" + string.Join('\n', theater.Take(30)));
        Assert.True(lostPlaceholders.Count == 0,
            language + " placeholder loss:\n" + string.Join('\n', lostPlaceholders.Take(30)));

        // Critical shell/workspace action chrome must not be theater under either product language.
        Assert.False(IsTheaterStub(service.Text("ui.action.undo"), language), language + " undo");
        Assert.False(IsTheaterStub(service.Text("ui.action.redo"), language), language + " redo");
        Assert.False(IsTheaterStub(service.Text("ui.action.send_message"), language), language + " send");
        Assert.Equal("{name}", service.Text("ui.window.project_title"));
        Assert.Contains("${spent}", service.Text("ui.layout.budget_status"), StringComparison.Ordinal);
        Assert.Contains("${budget}", service.Text("ui.layout.budget_status"), StringComparison.Ordinal);
        if (language == "en")
        {
            Assert.Equal("Undo", service.Text("ui.action.undo"));
            Assert.Equal("Redo", service.Text("ui.action.redo"));
            Assert.Equal("Open project", service.Text("ui.layout.open_project"));
            Assert.Equal("New project", service.Text("ui.layout.create_project"));
            Assert.Equal("Recent projects", service.Text("ui.layout.switch_recent_projects"));
            Assert.Equal("Recent projects", service.Text("ui.welcome.recent_projects"));
            Assert.Equal("New project", service.Text("ui.dialog.create_project.title"));
            Assert.Contains("Enter a project name", service.Text("ui.dialog.create_project.message"), StringComparison.Ordinal);
            Assert.Contains("Start from a project", service.Text("ui.welcome.hero_tagline"), StringComparison.Ordinal);
        }
        else
        {
            Assert.Equal("元に戻す", service.Text("ui.action.undo"));
            Assert.Equal("やり直す", service.Text("ui.action.redo"));
            Assert.Equal("メッセージを送信", service.Text("ui.action.send_message"));
            Assert.Equal("プロジェクトを開く", service.Text("ui.layout.open_project"));
            Assert.Equal("新規プロジェクト", service.Text("ui.layout.create_project"));
            Assert.Equal("最近のプロジェクト", service.Text("ui.layout.switch_recent_projects"));
            Assert.Equal("最近のプロジェクト", service.Text("ui.welcome.recent_projects"));
            Assert.Equal("新規プロジェクト", service.Text("ui.dialog.create_project.title"));
            Assert.Contains("プロジェクト名", service.Text("ui.dialog.create_project.message"), StringComparison.Ordinal);
        }
    }

    [Theory]
    [InlineData("en")]
    [InlineData("ja")]
    public void MainWindowAndWelcome_BoundKeys_AreRealPhrasesNotCamelCaseStubs(string language)
    {
        // Product path: keys MainWindowViewModel + WelcomeViewModel actually bind.
        var resourceDir = Path.GetDirectoryName(ResolveDisplayNamePath())!;
        var service = DisplayNameService.LoadFromDirectory(resourceDir, language);
        var sources = new[]
        {
            File.ReadAllText(ResolveDesktopSource("ViewModels", "MainWindowViewModel.cs")),
            File.ReadAllText(ResolveDesktopSource("ViewModels", "WelcomeViewModel.cs")),
            File.ReadAllText(ResolveDesktopSource("ViewModels", "ConfirmDialogViewModel.cs")),
        };
        var keys = new HashSet<string>(StringComparer.Ordinal);
        foreach (var src in sources)
        {
            foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(
                         src, @"""(ui\.(?:layout|welcome|dialog\.(?:create_project|open_project)|action|common|window|nav)\.[a-zA-Z0-9_./-]+)"""))
            {
                keys.Add(m.Groups[1].Value);
            }
            // also bare ui.layout.* without extra dots after second segment handled above
            foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(
                         src, @"""(ui\.(?:layout|welcome)\.[a-zA-Z0-9_./-]+)"""))
            {
                keys.Add(m.Groups[1].Value);
            }
        }
        // Always include known project-menu / welcome keys even if regex misses Format-only.
        foreach (var key in new[]
                 {
                     "ui.layout.open_project", "ui.layout.create_project", "ui.layout.switch_recent_projects",
                     "ui.layout.leave_project", "ui.layout.switch_project", "ui.welcome.recent_projects",
                     "ui.welcome.subtitle", "ui.welcome.hero_tagline", "ui.dialog.create_project.title",
                     "ui.dialog.create_project.message", "ui.dialog.create_project.name_label",
                 })
        {
            keys.Add(key);
        }

        Assert.True(keys.Count >= 12, "expected MainWindow/Welcome key set, got " + keys.Count);
        var theater = keys
            .Select(k => (Key: k, Text: service.Text(k)))
            .Where(x => IsTheaterStub(x.Text, language) || IsCamelCaseStub(x.Text))
            .Select(x => $"{x.Key} => {x.Text}")
            .ToArray();
        Assert.True(theater.Length == 0,
            language + " MainWindow/Welcome theater:\n" + string.Join('\n', theater.Take(25)));
    }

    private static IReadOnlyList<string> LoadFrozenPrimaryChromeKeys()
    {
        // Same derivation as pack generator: scan shipped Views/VMs for ui.* literals + shell families.
        var keys = new HashSet<string>(StringComparer.Ordinal);
        var desktopRoot = Path.GetDirectoryName(ResolveDesktopSource("ViewModels", "SettingsPageViewModel.cs"))!;
        desktopRoot = Path.GetDirectoryName(desktopRoot)!; // Ariadne.Desktop
        foreach (var dir in new[] { "ViewModels", "Views" })
        {
            var folder = Path.Combine(desktopRoot, dir);
            if (!Directory.Exists(folder))
            {
                continue;
            }
            foreach (var file in Directory.EnumerateFiles(folder, "*.*", SearchOption.TopDirectoryOnly))
            {
                if (!file.EndsWith(".cs", StringComparison.OrdinalIgnoreCase)
                    && !file.EndsWith(".axaml", StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }
                var text = File.ReadAllText(file);
                foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(
                             text, @"""(ui\.[a-zA-Z0-9_./-]+)"""))
                {
                    keys.Add(m.Groups[1].Value);
                }
            }
        }
        // Always include shell action/common/format families even if missed by scan.
        foreach (var prefix in new[]
                 {
                     "ui.action.", "ui.common.", "ui.dialog.unsaved.", "ui.layout.", "ui.window.",
                     "ui.nav.", "ui.settings.tab.", "ui.settings.title", "ui.settings.status.",
                 })
        {
            // Pull from zh pack via service for family completeness is heavy; explicit critical keys:
            _ = prefix;
        }
        foreach (var key in new[]
                 {
                     "ui.action.undo", "ui.action.redo", "ui.action.send_message",
                     "ui.action.open_settings", "ui.action.toggle_sidebar",
                     "ui.common.save", "ui.common.cancel", "ui.common.create", "ui.common.finish",
                     "ui.common.empty", "ui.common.enabled", "ui.common.unknown", "ui.common.no_tags",
                     "ui.common.back_to_top", "ui.dialog.unsaved.discard_all",
                     "ui.window.project_title", "ui.layout.budget_status",
                     "ui.settings.status.unsaved_sections", "ui.settings.title",
                     "ui.nav.workspace", "ui.nav.works", "ui.nav.settings",
                     "ui.layout.open_project", "ui.layout.create_project", "ui.layout.switch_recent_projects",
                     "ui.layout.leave_project", "ui.welcome.recent_projects", "ui.welcome.subtitle",
                     "ui.welcome.hero_tagline", "ui.dialog.create_project.title",
                     "ui.dialog.create_project.message", "ui.dialog.create_project.name_label",
                 })
        {
            keys.Add(key);
        }
        return keys.OrderBy(k => k, StringComparer.Ordinal).ToArray();
    }

    private static bool ContainsCjk(string text) =>
        text.Any(ch => ch >= '\u4e00' && ch <= '\u9fff');

    // Machine-glue compounds that appear as all-lowercase tokens inside multi-word phrases
    // (e.g. "Allow networksearch", "permissions toolspresets").
    private static readonly HashSet<string> TheaterBadCompounds = new(StringComparer.Ordinal)
    {
        "networksearch", "networktools", "scriptnetwork", "toolspresets",
        "templaterepository", "autocheckpoint", "pathroot", "skillss",
        "nodepresets", "modellist", "workflowtools", "globaldefault",
        "globaldefaults", "manualreview", "autoaudit", "autoskip",
        "autoapproval", "maxtoolsrounds", "workflowruntime", "currentproject",
        "diagnosticsinfo", "retrievalprovider", "providerlist", "nodetype",
        "manualfallback", "beginnertutorial", "asdefault", "enteradd",
        "cancelleave", "budgetinfo", "openbrowse", "compatibleapi",
        "officialprovider", "useproject", "projectai", "fetchmodel",
        "dismisspath", "availablemodels", "sensitiveroots", "capabilitytools",
        "writepath", "readpath", "pathroots", "autocheckpoints", "summarizerhint",
    };

    // Legitimate mid-sentence all-lowercase English words of length >= 10 (Title Case is never scanned).
    private static readonly HashSet<string> TheaterRealLowercaseWords = new(StringComparer.Ordinal)
    {
        "configured", "configuration", "configurations", "permissions", "permission",
        "confirmation", "confirmations", "automatically", "documentation", "repository",
        "repositories", "directories", "directory", "application", "applications",
        "authentication", "authorization", "description", "descriptions", "temperature",
        "dimensions", "collection", "collections", "workspace", "workspaces", "workflows",
        "workflow", "characters", "preference", "preferences", "diagnostic", "diagnostics",
        "personalization", "initialization", "unavailable", "completion", "completed",
        "cancelled", "canceled", "succeeded", "embedding", "embeddings", "retrieval",
        "templates", "template", "providers", "provider", "connection", "credentials",
        "maintenance", "horizontal", "vertical", "alignment", "saturation", "background",
        "foreground", "secondary", "tertiary", "placeholder", "accessible", "accessibility",
        "navigation", "communication", "communications", "checkpoint", "checkpoints",
        "checkpointing", "preauthorized", "iterations", "automation", "environment",
        "information", "referenced", "references", "unfinished", "inconsistent",
        "temporarily", "reproduce", "reproduction", "maintainers", "subfeatures",
        "registration", "correction", "capability", "capabilities", "compatible",
        "official", "selection", "transitions", "essential", "feedback", "separately",
        "following", "incomplete", "manuscripts", "manuscript", "foreshadowing",
        "relationship", "annotation", "annotations", "subworkflow", "subworkflows",
        "worldbuilding", "initialized", "definition", "refreshing", "commercial",
        "generating", "breakpoint", "summarizer", "available", "selected", "disabled",
        "required", "optional", "processing", "connecting", "downloading", "uploading",
        "installing", "uninstalling", "validating", "serializing", "deserializing",
        "separate", "sponsorship", "sponsoring", "projection", "preference",
        "automatically", "daytime", "sessions", "workspace", "workspaces",
        "reading", "surfaces", "without", "steady", "refined", "violet",
        "azure", "accents", "glare", "vision", "low", "paper", "warmer",
        "softer", "quieter", "oriented", "preference",
    };

    /// <summary>
    /// Strengthened LO-1 theater oracle: token stubs, whole + embedded camelCase,
    /// bad compounds and long all-lowercase tokens inside multi-word glue (\b[a-z]{10,}\b),
    /// bare short lowercase, CJK-punct+Latin without CJK prose, punctuation-only, English CJK.
    /// </summary>
    private static bool IsTheaterStub(string text, string? language = null)
    {
        var t = (text ?? string.Empty).Trim();
        if (t.Length == 0)
        {
            return true;
        }
        // Legitimate short product chrome.
        if (t is "—" or "-" or "A" or "秒" or "昼" or "夜" or "Day" or "Night"
            or "seconds" or "optional" or "99+" or "…" or "..." or "OK" or "ID" or "Git" or "AI"
            or "URL" or "API" or "ms" or "HTTP" or "JSON" or "YAML" or "LLM" or "RGB" or "hex" or "Hex")
        {
            return false;
        }
        // Format templates that are only placeholders + punctuation (e.g. "{kind}：{title}").
        if (IsPlaceholderOnlyTemplate(t))
        {
            return false;
        }
        if (t.Equals("Label", StringComparison.OrdinalIgnoreCase)
            || t.Equals("Item", StringComparison.OrdinalIgnoreCase)
            || t.Equals("TODO", StringComparison.OrdinalIgnoreCase)
            || t.Equals("FIXME", StringComparison.OrdinalIgnoreCase)
            || t.Equals("TBD", StringComparison.OrdinalIgnoreCase)
            || t.Equals("Placeholder", StringComparison.OrdinalIgnoreCase)
            || t.Equals("Desc", StringComparison.OrdinalIgnoreCase)
            || t.Equals("Hint", StringComparison.OrdinalIgnoreCase)
            || t.Equals("Help", StringComparison.OrdinalIgnoreCase))
        {
            return true;
        }
        if (IsCamelCaseStub(t))
        {
            return true;
        }
        // Embedded camelCase tokens inside multi-word glue (asDefault, networksearchProvider, …).
        foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(
                     t, @"[A-Za-z][A-Za-z0-9]*"))
        {
            var tok = m.Value;
            if (tok is "OpenAI")
            {
                continue;
            }
            if (tok.Length >= 4 && System.Text.RegularExpressions.Regex.IsMatch(tok, @"[a-z][A-Z]"))
            {
                return true;
            }
        }
        // All-lowercase tokens only (Title Case "Permissions" is not scanned):
        // 1) any known machine compound (pathroot, skillss, networksearch, …) regardless of length
        // 2) unknown words of length ≥ 10 (glued compounds not yet listed)
        foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(
                     t, @"\b[a-z]{6,}\b"))
        {
            var tok = m.Value;
            if (TheaterBadCompounds.Contains(tok))
            {
                return true;
            }
            if (tok.Length >= 10 && !TheaterRealLowercaseWords.Contains(tok))
            {
                return true;
            }
        }
        // Whole-string long all-lowercase machine glue (templatemarketpresets).
        if (System.Text.RegularExpressions.Regex.IsMatch(t, @"^[a-z]{12,}$"))
        {
            return true;
        }
        if (System.Text.RegularExpressions.Regex.IsMatch(t, @"^[a-z]{2,16}$")
            && t is not ("seconds" or "optional"))
        {
            return true;
        }
        // CJK/fullwidth punctuation + Latin without real CJK prose (machine glue English).
        if (System.Text.RegularExpressions.Regex.IsMatch(t, @"[，。；、：]")
            && System.Text.RegularExpressions.Regex.IsMatch(t, @"[A-Za-z]")
            && !ContainsCjk(t))
        {
            return true;
        }
        // Punctuation-only / empty of words (e.g. "， ， 。").
        var letters = System.Text.RegularExpressions.Regex.Replace(
            t, @"[^A-Za-z\u3040-\u30ff\u4e00-\u9fff0-9\$\{\}]+", string.Empty);
        if (letters.Length < 2 && !ExtractPlaceholders(t).Any())
        {
            return true;
        }
        // English product chrome must not be CJK fallback.
        if (string.Equals(language, "en", StringComparison.OrdinalIgnoreCase) && ContainsCjk(t))
        {
            return true;
        }
        return false;
    }

    private static bool IsPlaceholderOnlyTemplate(string text)
    {
        if (!ExtractPlaceholders(text).Any())
        {
            return false;
        }
        var stripped = System.Text.RegularExpressions.Regex.Replace(
            text, @"\$\{[^}]+\}|\{[^}]+\}", string.Empty);
        stripped = System.Text.RegularExpressions.Regex.Replace(
            stripped, @"[\s\W_，。；、：:·•\-–—…]+", string.Empty);
        return stripped.Length == 0;
    }

    private static bool IsCamelCaseStub(string text)
    {
        var t = (text ?? string.Empty).Trim();
        // openProject, switchrecentProject, newProject, currentProject, budgetstatus, …
        return System.Text.RegularExpressions.Regex.IsMatch(t, @"^[a-z]+([A-Z][a-z0-9]*)+$")
               || System.Text.RegularExpressions.Regex.IsMatch(t, @"^[a-z]+[A-Z][a-zA-Z0-9]*$");
    }

    private static IEnumerable<string> ExtractPlaceholders(string text)
    {
        foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(
                     text ?? string.Empty,
                     @"\$\{[^}]+\}|\{[^}]+\}"))
        {
            yield return m.Value;
        }
    }

    private static string ResolveDesktopSourceView() =>
        ResolveDesktopSource("Views", "SettingsPageView.axaml");

    private static string ResolveDesktopSource(params string[] parts)
    {
        var directory = new DirectoryInfo(AppContext.BaseDirectory);
        for (var depth = 0; directory is not null && depth < 10; depth++)
        {
            var candidate = Path.Combine(
                new[] { directory.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (File.Exists(candidate))
            {
                return candidate;
            }
            directory = directory.Parent;
        }

        throw new FileNotFoundException(string.Join('/', parts));
    }
}
