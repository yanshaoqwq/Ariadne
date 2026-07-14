using System.Text.Json;
using System.Text.RegularExpressions;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

public sealed class DesktopUxHelpersTests
{
    private static DisplayNameService Names() => DisplayNameService.LoadDefault();

    [Fact]
    public void VersionDialog_UsesResourceBackedLicenseAndCommercialNotice()
    {
        var names = Names();
        var dialog = HelpDialogFactory.CreateVersionDialog(names, "v0.1.0");

        Assert.Contains(names.Text("ui.version.license"), dialog.Message, StringComparison.Ordinal);
        Assert.Contains(names.Text("ui.version.commercial"), dialog.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void BackendDiscovery_PrefersPackagedRelativeSidecarAndPreservesSpaces()
    {
        using var temp = new TemporaryDirectory("Ariadne release with spaces");
        var app = Path.Combine(temp.Path, "app");
        var backend = Path.Combine(app, "Backend", OperatingSystem.IsWindows() ? "ariadne-ipc.exe" : "ariadne-ipc");
        Directory.CreateDirectory(Path.GetDirectoryName(backend)!);
        File.WriteAllText(backend, string.Empty);

        var development = Path.Combine(temp.Path, "target", "debug", OperatingSystem.IsWindows() ? "ariadne-ipc.exe" : "ariadne-ipc");
        Directory.CreateDirectory(Path.GetDirectoryName(development)!);
        File.WriteAllText(development, string.Empty);

        Assert.Equal(Path.GetFullPath(backend), JsonLineBackendClient.DiscoverBackendCommand(app, temp.Path));
    }

    [Fact]
    public void ReleaseLayoutValidator_RejectsRemoteServerBinary()
    {
        using var temp = new TemporaryDirectory("Ariadne forbidden server");
        File.WriteAllText(
            Path.Combine(temp.Path, OperatingSystem.IsWindows() ? "ariadne-server.exe" : "ariadne-server"),
            string.Empty);

        Assert.False(ReleaseLayoutValidator.TryValidate(temp.Path, out var error));
        Assert.Contains("remote REST server", error, StringComparison.Ordinal);
    }

    [Theory]
    [InlineData("network", "ui.error.network")]
    [InlineData("permission", "ui.error.permission")]
    [InlineData("not_found", "ui.error.not_found")]
    [InlineData("validation", "ui.error.validation")]
    [InlineData("budget", "ui.error.budget")]
    [InlineData("conflict", "ui.error.conflict")]
    [InlineData("cancelled", "ui.error.cancelled")]
    [InlineData("external", "ui.error.external")]
    [InlineData("io", "ui.error.io")]
    [InlineData("ipc", "ui.error.ipc")]
    [InlineData("unknown", "ui.error.unknown")]
    public void UserFacingError_PrimaryForCode_IsLocalizedKeyOnly(string code, string key)
    {
        var names = Names();
        var text = UserFacingError.PrimaryForCode(code, names);
        Assert.Equal(names.Text(key), text);
        Assert.DoesNotContain("/home/", text, StringComparison.Ordinal);
        Assert.DoesNotContain("Exception", text, StringComparison.Ordinal);
    }

    [Fact]
    public void BackendException_FromIpcPayload_UsesServerErrorCode()
    {
        var ex = BackendException.FromIpcPayload(
            "validation",
            "validation failed: port type mismatch for /home/user/secret");
        Assert.Equal("validation", ex.Code);
        Assert.Contains("port type", ex.Diagnostic, StringComparison.Ordinal);

        var names = Names();
        var primary = UserFacingError.Format(ex, names);
        Assert.Equal(names.Text("ui.error.validation"), primary);
        Assert.DoesNotContain("/home/user", primary, StringComparison.Ordinal);
        Assert.DoesNotContain("port type", primary, StringComparison.Ordinal);

        var diag = UserFacingError.FromException(ex).RedactedDiagnostic;
        Assert.NotNull(diag);
        Assert.DoesNotContain("/home/user", diag, StringComparison.Ordinal);
    }

    [Fact]
    public void BackendException_FromIpcPayload_ClassifiesWhenCodeMissing()
    {
        var ex = BackendException.FromIpcPayload(null, "Connection refused to 127.0.0.1:7788");
        Assert.Equal("unknown", ex.Code);
        var names = Names();
        Assert.Equal(names.Text("ui.error.unknown"), UserFacingError.Format(ex, names));
    }

    [Fact]
    public void BackendResult_DeserializesErrorCodeFromIpcEnvelope()
    {
        const string json = """{"ok":false,"error":"validation failed: name","error_code":"validation"}""";
        var result = JsonSerializer.Deserialize<BackendResult<object>>(json, new JsonSerializerOptions(JsonSerializerDefaults.Web));
        Assert.NotNull(result);
        Assert.False(result!.Ok);
        Assert.Equal("validation", result.ErrorCode);
        Assert.Equal("validation failed: name", result.Error);

        var ex = BackendException.FromIpcPayload(result.ErrorCode, result.Error);
        Assert.Equal("validation", ex.Code);
        Assert.Equal(Names().Text("ui.error.validation"), UserFacingError.Format(ex, Names()));
    }

    [Fact]
    public void BackendResult_LegacyOkFalseWithoutCode_StillMapsSafely()
    {
        const string json = """{"ok":false,"error":"permission denied for export"}""";
        var result = JsonSerializer.Deserialize<BackendResult<object>>(json, new JsonSerializerOptions(JsonSerializerDefaults.Web));
        Assert.NotNull(result);
        Assert.Null(result!.ErrorCode);
        // Legacy ok:false without error_code → unknown (no desktop keyword table).
        var ex = BackendException.FromIpcPayload(result.ErrorCode, result.Error);
        Assert.Equal("unknown", ex.Code);
        Assert.Equal(Names().Text("ui.error.unknown"), UserFacingError.Format(ex, Names()));
    }

    [Fact]
    public void UserFacingError_UnknownDoesNotLeakEnglishDetailAsPrimary()
    {
        var names = Names();
        var text = UserFacingError.Format(
            new Exception("weird internal panic in FooBarService at /opt/ariadne/bin"),
            names);
        Assert.Equal(names.Text("ui.error.unknown"), text);
        Assert.DoesNotContain("FooBarService", text, StringComparison.Ordinal);
        Assert.DoesNotContain("/opt/", text, StringComparison.Ordinal);
    }

    [Fact]
    public void UserFacingError_Short_TruncatesForTitleBar()
    {
        var names = Names();
        var text = UserFacingError.Short(
            BackendException.FromIpcPayload("unknown", new string('x', 200)),
            names,
            "ui.error.budget");
        Assert.True(text.Length <= 48);
        Assert.Equal(names.Text("ui.error.budget"), UserFacingError.Format(
            BackendException.FromIpcPayload("budget", "x"), names));
    }

    [Theory]
    [InlineData("running", "ui.status.running")]
    [InlineData("queued", "ui.status.queued")]
    [InlineData("succeeded", "ui.status.succeeded")]
    [InlineData("failed", "ui.status.failed")]
    [InlineData("paused", "ui.status.paused")]
    [InlineData("weird_internal_token", "ui.status.unavailable")]
    public void UserFacingError_RuntimeStatus_MapsKnownTokens(string token, string key)
    {
        var names = Names();
        Assert.Equal(names.Text(key), UserFacingError.RuntimeStatus(token, names));
    }

    [Fact]
    public void AuthorFacingStatusSurfaces_DoNotAssignExceptionMessage()
    {
        // Structural gate: author-facing status/budget/provider/repo/notification must not assign *.Message.
        var vmDir = ResolveDesktopSource("ViewModels");
        var pattern = new Regex(
            @"(StatusText|BudgetStatusText|ProviderStatus|RepositoryStatusText|NotificationText)\s*=\s*[^;\n]*\b\w*[Ee]x\w*\.Message\b",
            RegexOptions.Compiled);

        var offenders = new List<string>();
        foreach (var path in Directory.EnumerateFiles(vmDir, "*.cs"))
        {
            if (Path.GetFileName(path) is "UserFacingError.cs")
            {
                continue;
            }

            var lines = File.ReadAllLines(path);
            for (var i = 0; i < lines.Length; i++)
            {
                var line = lines[i];
                if (line.Contains("UserFacingError", StringComparison.Ordinal))
                {
                    continue;
                }

                if (pattern.IsMatch(line))
                {
                    offenders.Add($"{Path.GetFileName(path)}:{i + 1}: {line.Trim()}");
                }
            }
        }

        Assert.True(offenders.Count == 0, "Raw exception Message on author status:\n" + string.Join('\n', offenders));
    }

    [Fact]
    public void JsonLineBackendClient_ThrowsBackendExceptionOnIpcFailure()
    {
        var src = File.ReadAllText(Path.Combine(ResolveDesktopSource("Backend"), "JsonLineBackendClient.cs"));
        Assert.Contains("BackendException.FromIpcPayload", src, StringComparison.Ordinal);
        Assert.Contains("BackendException.Transport", src, StringComparison.Ordinal);
        Assert.DoesNotContain("throw new InvalidOperationException(result.Error", src, StringComparison.Ordinal);
    }

    [Fact]
    public void EnJaLocaleStubs_AreNotProductLanguages()
    {
        var names = Names();
        Assert.DoesNotContain(names.AvailableLanguages, code =>
            string.Equals(code, "en", StringComparison.OrdinalIgnoreCase)
            || string.Equals(code, "ja", StringComparison.OrdinalIgnoreCase));

        var resources = Path.Combine(
            Path.GetDirectoryName(typeof(DisplayNameService).Assembly.Location)!,
            "Resources");
        var en = Path.Combine(resources, "display_name.en.json");
        if (!File.Exists(en))
        {
            en = FindRepoResource("display_name.en.json");
        }

        Assert.True(File.Exists(en));
        Assert.False(DisplayNameService.IsProductLanguagePack(en));
        Assert.Contains("out_of_scope_for_v1", File.ReadAllText(en), StringComparison.Ordinal);
    }

    [Fact]
    public void ConfirmDialog_CreateProject_IsInputSeverity_NotWarning()
    {
        var names = Names();
        var dialog = ConfirmDialogViewModel.CreateProjectName(names);
        Assert.Equal(DialogSeverity.Input, dialog.Severity);
        Assert.True(dialog.AllowEnterConfirm);
        Assert.True(dialog.HasInput);
        Assert.Equal(80, dialog.MaxInputLength);
    }

    [Fact]
    public async Task ConfirmDialog_Danger_DisallowsEnterConfirm()
    {
        var dialog = new ConfirmDialogViewModel(
            "t",
            "m",
            new[]
            {
                new DialogButton("ok", DialogButtonVariant.Danger, 0),
                new DialogButton("cancel", DialogButtonVariant.Subtle, 1),
            })
        {
            Severity = DialogSeverity.Danger,
            ConfirmResultIndex = 0,
            CancelResultIndex = 1,
        };
        Assert.False(dialog.AllowEnterConfirm);
        dialog.RequestConfirm();
        Assert.False(dialog.Completion.IsCompleted);
        dialog.Cancel();
        Assert.Equal(1, await dialog.Completion.ConfigureAwait(false));
    }

    [Fact]
    public void ConfirmDialog_UnsavedLeaveMany_ListsPages()
    {
        var names = Names();
        var dialog = ConfirmDialogViewModel.UnsavedLeaveMany(names, new[] { "作品", "设置" });
        Assert.Equal(DialogSeverity.Warning, dialog.Severity);
        Assert.Contains("作品", dialog.Message, StringComparison.Ordinal);
        Assert.Contains("设置", dialog.Message, StringComparison.Ordinal);
        Assert.Equal(3, dialog.Buttons.Count);
    }

    [Fact]
    public void RunLogItem_UsesSemanticLevelFlags_NotFixedBrushes()
    {
        var error = new RunLogItemViewModel(new UiRunLogEntry("1", 0, "k", "error", "boom"));
        var warn = new RunLogItemViewModel(new UiRunLogEntry("2", 0, "k", "warning", "careful"));
        var info = new RunLogItemViewModel(new UiRunLogEntry("3", 0, "k", "info", "ok"));
        Assert.True(error.IsError);
        Assert.True(warn.IsWarning);
        Assert.True(info.IsInfo);
        Assert.False(error.IsInfo);
    }

    [Fact]
    public void DisplayNamePack_ContainsUxErrorAndUnsavedManyKeys()
    {
        var names = Names();
        Assert.DoesNotContain("[", names.Text("ui.error.network"));
        Assert.DoesNotContain("[", names.Text("ui.error.ipc"));
        Assert.DoesNotContain("[", names.Text("ui.error.conflict"));
        Assert.DoesNotContain("[", names.Text("ui.dialog.unsaved.save_all"));
        Assert.DoesNotContain("[", names.Text("ui.dialog.unsaved.message_many"));
        Assert.DoesNotContain("[", names.Text("ui.i18n.release_scope"));
        Assert.DoesNotContain("[", names.Text("ui.error.budget"));
        Assert.DoesNotContain("[", names.Text("ui.color.channel_r"));
        Assert.DoesNotContain("[", names.Text("ui.git.checkpoint_created_plain"));
    }

    [Fact]
    public async Task ConfirmDialog_UnsavedLeave_AllowsEnterOnSave()
    {
        var names = Names();
        var dialog = ConfirmDialogViewModel.UnsavedLeave(names, "作品");
        Assert.Equal(DialogSeverity.Question, dialog.Severity);
        Assert.True(dialog.AllowEnterConfirm);
        Assert.Contains("作品", dialog.Message, StringComparison.Ordinal);
        dialog.RequestConfirm();
        Assert.Equal((int)UnsavedLeaveChoice.Save, await dialog.Completion.ConfigureAwait(false));
    }

    [Fact]
    public void WorkspaceSearch_DoesNotHardcodeQueryPlaceholder()
    {
        var axaml = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        Assert.DoesNotContain("PlaceholderText=\"query\"", axaml, StringComparison.Ordinal);
        Assert.Contains("SearchQueryPlaceholder", axaml, StringComparison.Ordinal);
    }

    [Fact]
    public void U66_NavigationItemTemplate_IsSingleInAppDataTemplates()
    {
        var app = File.ReadAllText(Path.Combine(ResolveDesktopSource(""), "App.axaml"));
        var main = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "MainWindow.axaml"));
        Assert.Contains("DataType=\"{x:Type vm:NavigationItemViewModel}\"", app, StringComparison.Ordinal);
        // MainWindow must not re-declare the nav item template (single source).
        Assert.DoesNotContain("DataType=\"{x:Type vm:NavigationItemViewModel}\"", main, StringComparison.Ordinal);
        Assert.Contains("PrimaryNavigationItems", main, StringComparison.Ordinal);
        Assert.Contains("SecondaryNavigationItems", main, StringComparison.Ordinal);
    }

    [Fact]
    public void U69_IconOnlyButtons_HaveAutomationName_Gate()
    {
        var viewsDir = ResolveDesktopSource("Views");
        var controlsDir = ResolveDesktopSource("Controls");
        var offenders = new List<string>();
        foreach (var path in Directory.EnumerateFiles(viewsDir, "*.axaml")
                     .Concat(Directory.EnumerateFiles(controlsDir, "*.axaml")))
        {
            var text = File.ReadAllText(path);
            // Rough structural gate: each icon-btn opening tag must include AutomationProperties.Name
            var idx = 0;
            while ((idx = text.IndexOf("Classes=\"icon-btn", idx, StringComparison.Ordinal)) >= 0)
            {
                var start = text.LastIndexOf('<', idx);
                var end = text.IndexOf('>', idx);
                if (start < 0 || end < 0)
                {
                    break;
                }

                var tag = text[start..(end + 1)];
                if (!tag.Contains("AutomationProperties.Name", StringComparison.Ordinal))
                {
                    offenders.Add($"{Path.GetFileName(path)}: {tag.ReplaceLineEndings(" ").Trim()}");
                }

                idx = end + 1;
            }
        }

        Assert.True(offenders.Count == 0, "icon-btn missing AutomationProperties.Name:\n" + string.Join("\n", offenders));
    }

    [Fact]
    public void U2_DisplayTextHardcodeScan_RejectsKnownBadPlaceholders()
    {
        var viewsDir = ResolveDesktopSource("Views");
        var banned = new[]
        {
            "PlaceholderText=\"query\"",
            "PlaceholderText=\"Search\"",
            "Content=\"OK\"",
            "Content=\"Cancel\"",
            "ToolTip.Tip=\"Settings\"",
        };
        var hits = new List<string>();
        foreach (var path in Directory.EnumerateFiles(viewsDir, "*.axaml"))
        {
            var text = File.ReadAllText(path);
            foreach (var b in banned)
            {
                if (text.Contains(b, StringComparison.Ordinal))
                {
                    hits.Add($"{Path.GetFileName(path)}: {b}");
                }
            }
        }

        Assert.True(hits.Count == 0, "hardcoded display text:\n" + string.Join("\n", hits));
    }

    [Fact]
    public void U70_WriteThreeColorOverlay_SetsLogTokens()
    {
        var src = File.ReadAllText(Path.Combine(ResolveDesktopSource(""), "ThemeApplication.cs"));
        var writeIdx = src.IndexOf("public static void WriteThreeColorOverlay", StringComparison.Ordinal);
        Assert.True(writeIdx >= 0);
        var endIdx = src.IndexOf("public static bool ResolveIsDark", writeIdx, StringComparison.Ordinal);
        Assert.True(endIdx > writeIdx, "expected ResolveIsDark after WriteThreeColorOverlay");
        var slice = src[writeIdx..endIdx];
        Assert.Contains("Ariadne.LogErrorBg", slice, StringComparison.Ordinal);
        Assert.Contains("Ariadne.LogWarningBg", slice, StringComparison.Ordinal);
        Assert.Contains("Ariadne.LogInfoBg", slice, StringComparison.Ordinal);
        Assert.Contains("Ariadne.StatusError", slice, StringComparison.Ordinal);
        Assert.Contains("Ariadne.StatusWarning", slice, StringComparison.Ordinal);
        Assert.Contains("Ariadne.StatusInfo", slice, StringComparison.Ordinal);
    }

    /// <summary>D3：维护状态经后端 API 客户端 + VM 横幅绑定，active/failed 时可见。</summary>
    [Fact]
    public void D3_MaintenanceStatus_IsWiredOnDesktopShell()
    {
        var client = File.ReadAllText(Path.Combine(ResolveDesktopSource("Backend"), "JsonLineBackendClient.cs"));
        var iface = File.ReadAllText(Path.Combine(ResolveDesktopSource("Backend"), "IAriadneBackendClient.cs"));
        var models = File.ReadAllText(Path.Combine(ResolveDesktopSource("Backend"), "AriadneBackendModels.cs"));
        var vm = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "MainWindowViewModel.cs"));
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "MainWindow.axaml"));
        var names = Names();

        Assert.Contains("get_project_maintenance", client, StringComparison.Ordinal);
        Assert.Contains("GetProjectMaintenanceAsync", iface, StringComparison.Ordinal);
        Assert.Contains("ProjectMaintenanceState", models, StringComparison.Ordinal);
        Assert.Contains("RefreshMaintenanceStatusAsync", vm, StringComparison.Ordinal);
        Assert.Contains("ApplyMaintenanceState", vm, StringComparison.Ordinal);
        Assert.Contains("IsMaintenanceBlocking", view, StringComparison.Ordinal);
        Assert.Contains("MaintenanceBannerText", view, StringComparison.Ordinal);
        Assert.False(string.IsNullOrWhiteSpace(names.Text("ui.maintenance.banner_active")));
        Assert.False(string.IsNullOrWhiteSpace(names.Text("ui.maintenance.banner_failed")));

        // Real VM path: applying failed state must surface non-empty banner text.
        var backend = System.Reflection.DispatchProxy.Create<IAriadneBackendClient, UnimplementedBackendProxy>();
        var windowVm = new MainWindowViewModel(names, backend);
        windowVm.ApplyMaintenanceState(new Backend.ProjectMaintenanceState(
            Kind: "git_restore",
            Status: "failed",
            Phase: "rebuilding_full_text_indexes",
            Error: "index lock"));
        Assert.True(windowVm.IsMaintenanceBlocking);
        Assert.Contains("git_restore", windowVm.MaintenanceBannerText, StringComparison.Ordinal);
        Assert.Contains("index lock", windowVm.MaintenanceBannerText, StringComparison.Ordinal);
    }

    /// <summary>DispatchProxy 要求非 sealed。</summary>
    private class UnimplementedBackendProxy : System.Reflection.DispatchProxy
    {
        protected override object? Invoke(System.Reflection.MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod is null)
            {
                return null;
            }

            if (targetMethod.ReturnType == typeof(bool) && targetMethod.Name == "get_HasProjectRoot")
            {
                return false;
            }

            if (targetMethod.ReturnType == typeof(void) || targetMethod.ReturnType == typeof(Task))
            {
                return targetMethod.ReturnType == typeof(Task) ? Task.CompletedTask : null;
            }

            if (targetMethod.ReturnType.IsGenericType
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var t = targetMethod.ReturnType.GetGenericArguments()[0];
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(t)
                    .Invoke(null, new object?[] { t.IsValueType ? Activator.CreateInstance(t) : null });
            }

            return targetMethod.ReturnType.IsValueType
                ? Activator.CreateInstance(targetMethod.ReturnType)
                : null;
        }
    }

    /// <summary>
    /// Product rule: no global TextBox accent focus border; only unified Project AI composer
    /// (Works + Workspace via shared control) gets theme-color border on focus-within.
    /// </summary>
    [Fact]
    public void ProjectAiComposer_IsUnifiedAndOnlyAccentFocusSurface()
    {
        var theme = File.ReadAllText(Path.Combine(ResolveDesktopSource("Resources", "Styles"), "AriadneTheme.axaml"));
        var works = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorksPageView.axaml"));
        var workspace = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        var composer = File.ReadAllText(Path.Combine(ResolveDesktopSource("Controls"), "ProjectAiComposer.axaml"));

        // Shared control is the single markup surface for Project AI input.
        Assert.Contains("Classes=\"ai-composer\"", composer, StringComparison.Ordinal);
        Assert.Contains("ProjectAiMessage", composer, StringComparison.Ordinal);
        Assert.Contains("SendProjectAiCommand", composer, StringComparison.Ordinal);
        Assert.Contains("BorderThickness=\"0\"", composer, StringComparison.Ordinal);

        // Both product pages host the same control (no duplicated composer markup).
        Assert.Contains("ctl:ProjectAiComposer", works, StringComparison.Ordinal);
        Assert.Contains("ctl:ProjectAiComposer", workspace, StringComparison.Ordinal);
        Assert.DoesNotContain("Classes=\"ai-composer\"", works, StringComparison.Ordinal);
        Assert.DoesNotContain("Classes=\"ai-composer\"", workspace, StringComparison.Ordinal);

        // Theme: only ai-composer:focus-within may paint AccentPrimary border.
        Assert.Contains("Border.ai-composer:focus-within", theme, StringComparison.Ordinal);
        var focusWithinBlock = theme.IndexOf("Border.ai-composer:focus-within", StringComparison.Ordinal);
        Assert.True(focusWithinBlock >= 0);
        var focusWithinSlice = theme.Substring(focusWithinBlock, Math.Min(280, theme.Length - focusWithinBlock));
        Assert.Contains("Ariadne.AccentPrimary", focusWithinSlice, StringComparison.Ordinal);

        // Global TextBox:focus must NOT use AccentPrimary (suppress cheap blue edge).
        var textBoxFocus = theme.IndexOf("TextBox:focus /template/ Border#PART_BorderElement", StringComparison.Ordinal);
        Assert.True(textBoxFocus >= 0, "expected explicit TextBox:focus style to override Fluent accent");
        var textBoxFocusSlice = theme.Substring(textBoxFocus, Math.Min(220, theme.Length - textBoxFocus));
        Assert.DoesNotContain("Ariadne.AccentPrimary", textBoxFocusSlice, StringComparison.Ordinal);
        Assert.Contains("Ariadne.BorderDefault", textBoxFocusSlice, StringComparison.Ordinal);
        Assert.Contains("CaretBrush", theme, StringComparison.Ordinal);
    }

    [Fact]
    public void ThemeApplication_ApplyUsesSelectActiveCustomColors()
    {
        var src = File.ReadAllText(Path.Combine(ResolveDesktopSource(""), "ThemeApplication.cs"));
        Assert.Contains("SelectActiveCustomColors(", src, StringComparison.Ordinal);
        // Apply body must call the shared helper (not only define it).
        var applyIdx = src.IndexOf("public static void Apply(", StringComparison.Ordinal);
        var selectIdx = src.IndexOf("SelectActiveCustomColors(", applyIdx + 1, StringComparison.Ordinal);
        Assert.True(selectIdx > applyIdx, "Apply must call SelectActiveCustomColors");
    }

    [Fact]
    public void RunLog_ErrorPathDoesNotClearToEmptyState()
    {
        var src = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "RunLogPageViewModel.cs"));
        Assert.Contains("PageLoadState.Error", src, StringComparison.Ordinal);
        Assert.Contains("Do not Logs.Clear()", src, StringComparison.Ordinal);
    }

    [Fact]
    public void ThemeApplication_SelectActiveCustomColors_PicksDarkSetWhenFollowingSystem()
    {
        var selected = ThemeApplication.SelectActiveCustomColors(
            isDark: true,
            followSystemColors: true,
            mainLight: "#F5F5F5",
            surfaceLight: "#FFFFFF",
            brandLight: "#2E726B",
            mainDark: "#121212",
            surfaceDark: "#1E1E1E",
            brandDark: "#6FB9AD");
        Assert.Equal("#121212", selected.Main);
        Assert.Equal("#1E1E1E", selected.Surface);
        Assert.Equal("#6FB9AD", selected.Brand);
    }

    private static string ResolveDesktopSource(params string[] parts)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 10 && walk is not null; i++)
        {
            var candidate = Path.Combine(new[] { walk.FullName, "desktop", "Ariadne.Desktop" }.Concat(parts).ToArray());
            if (Directory.Exists(candidate) || File.Exists(candidate))
            {
                return candidate;
            }
            walk = walk.Parent;
        }

        throw new FileNotFoundException("Could not resolve " + string.Join('/', parts));
    }


    [Fact]
    public async Task BatchLeaveSaveCoordinator_PrepareAllBeforeAnyCommit()
    {
        var commits = 0;
        var prepares = 0;
        var pages = new List<(string, Func<Task<bool>>, Func<Task<bool>>)>
        {
            ("A", async () => { prepares++; await Task.Yield(); return true; }, async () => { commits++; await Task.Yield(); return true; }),
            ("B", async () => { prepares++; await Task.Yield(); return true; }, async () => { commits++; await Task.Yield(); return true; }),
        };
        var journal = Path.Combine(Path.GetTempPath(), "ariadne-leave-test-" + Guid.NewGuid().ToString("n") + ".json");
        try
        {
            var result = await BatchLeaveSaveCoordinator.ExecuteAsync(pages, journal);
            Assert.True(result.AllSucceeded);
            Assert.Equal(2, prepares);
            Assert.Equal(2, commits);
            Assert.False(File.Exists(journal));
        }
        finally
        {
            if (File.Exists(journal)) File.Delete(journal);
        }
    }

    [Fact]
    public async Task BatchLeaveSaveCoordinator_PrepareFailure_DoesNotCommit()
    {
        var commits = 0;
        var pages = new List<(string, Func<Task<bool>>, Func<Task<bool>>)>
        {
            ("A", () => Task.FromResult(true), () => { commits++; return Task.FromResult(true); }),
            ("B", () => Task.FromResult(false), () => { commits++; return Task.FromResult(true); }),
        };
        var result = await BatchLeaveSaveCoordinator.ExecuteAsync(pages, journalPath: null);
        Assert.False(result.AllSucceeded);
        Assert.Equal(0, commits);
        Assert.Equal("B", result.FailedPage);
    }

    [Fact]
    public async Task BatchLeaveSaveCoordinator_MidCommit_WritesJournal()
    {
        var journal = Path.Combine(Path.GetTempPath(), "ariadne-leave-partial-" + Guid.NewGuid().ToString("n") + ".json");
        try
        {
            var pages = new List<(string, Func<Task<bool>>, Func<Task<bool>>)>
            {
                ("A", () => Task.FromResult(true), () => Task.FromResult(true)),
                ("B", () => Task.FromResult(true), () => Task.FromResult(false)),
            };
            var result = await BatchLeaveSaveCoordinator.ExecuteAsync(pages, journal);
            Assert.False(result.AllSucceeded);
            Assert.Equal(new[] { "A" }, result.CommittedPages);
            Assert.True(File.Exists(journal));
            var j = BatchLeaveSaveCoordinator.ReadJournal(journal);
            Assert.NotNull(j);
            Assert.Equal("partial", j!.Phase);
            Assert.Contains("A", j.CommittedPages);
        }
        finally
        {
            if (File.Exists(journal)) File.Delete(journal);
        }
    }

    [Fact]
    public void UnsavedGuards_ImplementRealPrepareNotOnlyDefault()
    {
        var settings = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "SettingsPageViewModel.cs"));
        var workspace = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "WorkspacePageViewModel.cs"));
        var works = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "WorksPageViewModel.cs"));
        Assert.Contains("PrepareUnsavedChangesAsync()", settings, StringComparison.Ordinal);
        Assert.Contains("CommitPreparedUnsavedChangesAsync()", settings, StringComparison.Ordinal);
        Assert.Contains("PrepareUnsavedChangesAsync()", workspace, StringComparison.Ordinal);
        Assert.Contains("ValidateWorkflowGraphAsync", workspace, StringComparison.Ordinal);
        Assert.Contains("PrepareUnsavedChangesAsync()", works, StringComparison.Ordinal);
        var main = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "MainWindowViewModel.cs"));
        Assert.Contains("BatchLeaveSaveCoordinator.ExecuteAsync", main, StringComparison.Ordinal);
        Assert.DoesNotContain("foreach (var guard in dirty)\n                {\n                    if (!await guard.SaveUnsavedChangesAsync()", main);
    }


        private static string FindRepoResource(string fileName)
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 10 && walk is not null; i++)
        {
            var candidate = Path.Combine(walk.FullName, "core", "resources", fileName);
            if (File.Exists(candidate))
            {
                return candidate;
            }
            walk = walk.Parent;
        }

        throw new FileNotFoundException(fileName);
    }
}

internal sealed class TemporaryDirectory : IDisposable
{
    public TemporaryDirectory(string childName)
    {
        Path = System.IO.Path.Combine(System.IO.Path.GetTempPath(), $"ariadne-tests-{Guid.NewGuid():N}", childName);
        Directory.CreateDirectory(Path);
    }

    public string Path { get; }

    public void Dispose()
    {
        try
        {
            Directory.Delete(System.IO.Path.GetDirectoryName(Path)!, recursive: true);
        }
        catch
        {
            // Test cleanup is best effort on Windows where antivirus may briefly hold files.
        }
    }
}
