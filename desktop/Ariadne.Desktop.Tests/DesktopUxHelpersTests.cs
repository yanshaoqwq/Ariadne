using System.Text.Json;
using System.Text.RegularExpressions;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
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
    public void FeedbackDialog_OffersRepositoryIssueAction()
    {
        var names = Names();
        var dialog = HelpDialogFactory.CreateFeedbackDialog(names);

        Assert.Equal(names.Text("ui.feedback.title"), dialog.Title);
        Assert.Equal(2, dialog.Buttons.Count);
        Assert.Equal(names.Text("ui.feedback.open_issue"), dialog.Buttons[0].Text);
        Assert.Equal(1, dialog.ConfirmResultIndex);
        Assert.Equal(0, dialog.CancelResultIndex);
        Assert.True(dialog.Buttons[0].IsDefault);
        Assert.True(dialog.Buttons[1].IsCancel);
        Assert.Equal("https://github.com/yanshaoqwq/Ariadne/issues/new", HelpDialogFactory.FeedbackIssueUrl);
        Assert.True(Uri.TryCreate(HelpDialogFactory.FeedbackIssueUrl, UriKind.Absolute, out var issueUri));
        Assert.Equal(Uri.UriSchemeHttps, issueUri!.Scheme);
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
    [InlineData("resource_limit", "ui.error.resource_limit")]
    [InlineData("external_outcome_unknown", "ui.error.external_outcome_unknown")]
    [InlineData("serialization", "ui.error.serialization")]
    [InlineData("internal", "ui.error.internal")]
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
    public void MainWindow_DiagnosticPanel_ReceivesRedactedSecondaryFailureAndCanClear()
    {
        var names = Names();
        var backend = System.Reflection.DispatchProxy.Create<IAriadneBackendClient, UnimplementedBackendProxy>();
        var window = new MainWindowViewModel(names, backend);

        var primary = UserFacingError.Format(
            BackendException.FromIpcPayload(
                "validation",
                "invalid project at /home/writer/private/project.yaml"),
            names);

        Assert.True(window.HasDiagnostic);
        Assert.False(window.IsDiagnosticExpanded);
        Assert.Equal(primary, window.DiagnosticSummaryText);
        Assert.DoesNotContain("/home/writer", window.DiagnosticDetailText, StringComparison.Ordinal);

        window.ToggleDiagnosticCommand.Execute(null);
        Assert.True(window.IsDiagnosticExpanded);
        window.ClearDiagnosticCommand.Execute(null);
        Assert.False(window.HasDiagnostic);
        Assert.Empty(window.DiagnosticDetailText);
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
    public async Task C9_IpcResponseRouter_CompletesRequestsByIdOutOfOrder()
    {
        var router = new IpcResponseRouter();
        Assert.True(router.TryRegister("slow", out var slow));
        Assert.True(router.TryRegister("fast", out var fast));

        Assert.True(router.TryComplete("fast", "fast-response"));
        Assert.Equal("fast-response", await fast);
        Assert.False(slow.IsCompleted);

        Assert.True(router.TryComplete("slow", "slow-response"));
        Assert.Equal("slow-response", await slow);
    }

    [Fact]
    public async Task C9_IpcResponseRouter_CancelsOnlyTargetRequest()
    {
        var router = new IpcResponseRouter();
        using var cancellation = new CancellationTokenSource();
        Assert.True(router.TryRegister("target", out var target));
        Assert.True(router.TryRegister("other", out var other));

        cancellation.Cancel();
        Assert.True(router.TryCancel("target", cancellation.Token));
        await Assert.ThrowsAnyAsync<OperationCanceledException>(async () => await target);
        Assert.False(other.IsCompleted);

        Assert.True(router.TryComplete("other", "ok"));
        Assert.Equal("ok", await other);
    }

    [Fact]
    public void EnJaLocalePacks_AreProductLanguagesWithRealChrome()
    {
        // Product packs ship partial primary chrome (not out_of_scope stubs).
        var names = Names();
        Assert.Contains(names.AvailableLanguages, code =>
            string.Equals(code, "en", StringComparison.OrdinalIgnoreCase));
        Assert.Contains(names.AvailableLanguages, code =>
            string.Equals(code, "ja", StringComparison.OrdinalIgnoreCase));

        var resources = Path.Combine(
            Path.GetDirectoryName(typeof(DisplayNameService).Assembly.Location)!,
            "Resources");
        var en = Path.Combine(resources, "display_name.en.json");
        if (!File.Exists(en))
        {
            en = FindRepoResource("display_name.en.json");
        }

        Assert.True(File.Exists(en));
        Assert.True(DisplayNameService.IsProductLanguagePack(en));
        var enText = File.ReadAllText(en);
        Assert.DoesNotContain("out_of_scope_for_v1", enText, StringComparison.Ordinal);
        Assert.DoesNotContain("zh-only", enText, StringComparison.Ordinal);
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
        Assert.Equal(1, await dialog.Completion);
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
        Assert.Equal((int)UnsavedLeaveChoice.Save, await dialog.Completion);
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

    [Fact]
    public void MainWindow_SettingsEntry_IsLabeledSecondaryNavItem_NotWindowControl()
    {
        // Product path: Settings lives in SecondaryNavigationItems (sidebar), shared nav DataTemplate.
        var main = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "MainWindow.axaml"));
        var app = File.ReadAllText(Path.Combine(ResolveDesktopSource("App.axaml")));
        var vm = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "MainWindowViewModel.cs"));

        Assert.Contains("ItemsSource=\"{Binding SecondaryNavigationItems}\"", main, StringComparison.Ordinal);
        Assert.Contains("CreateNav(\"settings\", \"ui.nav.settings\"", vm, StringComparison.Ordinal);
        Assert.Contains("SecondaryNavigationItems", vm, StringComparison.Ordinal);
        Assert.Contains("DataType=\"{x:Type vm:NavigationItemViewModel}\"", app, StringComparison.Ordinal);
        Assert.Contains("Text=\"{Binding Title}\"", app, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding SelectCommand}\"", app, StringComparison.Ordinal);
        // Settings must not be a bare window-control chrome button.
        Assert.DoesNotContain("SettingsNavigationItem", main, StringComparison.Ordinal);
        Assert.DoesNotContain("Classes=\"subtle top-settings\"", main, StringComparison.Ordinal);
    }

    [Fact]
    public void CompactSidebar_AutoCollapsesOnlyWhenEnteringCompactLayout()
    {
        Assert.True(MainWindow.ShouldAutoCollapseSidebar(null, isCompact: true));
        Assert.True(MainWindow.ShouldAutoCollapseSidebar(false, isCompact: true));
        Assert.False(MainWindow.ShouldAutoCollapseSidebar(true, isCompact: true));
        Assert.False(MainWindow.ShouldAutoCollapseSidebar(true, isCompact: false));
    }

    [Fact]
    public void ChromePolish_MenusSearchShellAndWindowControls_AreOnShippedThemeAndViews()
    {
        // Structural guards for high-impact UI polish (menus, search shell, window chrome).
        // Drives real shipped axaml/theme strings — a revert of polish fails this contract.
        var theme = File.ReadAllText(Path.Combine(
            ResolveDesktopSource("Resources", "Styles"),
            "AriadneTheme.axaml"));
        var main = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "MainWindow.axaml"));
        var template = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "TemplateMarketPageView.axaml"));
        var runLog = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "RunLogPageView.axaml"));
        var mainCode = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "MainWindow.axaml.cs"));

        // Menus: compact min-width (below legacy 208–220 fat-panel band).
        Assert.Contains("Selector=\"ContextMenu\"", theme, StringComparison.Ordinal);
        Assert.Contains("Selector=\"MenuFlyoutPresenter\"", theme, StringComparison.Ordinal);
        var contextMin = Regex.Match(
            theme,
            @"Selector=""ContextMenu""[\s\S]*?MinWidth""\s+Value=""(\d+)""",
            RegexOptions.CultureInvariant);
        var flyoutMin = Regex.Match(
            theme,
            @"Selector=""MenuFlyoutPresenter""[\s\S]*?MinWidth""\s+Value=""(\d+)""",
            RegexOptions.CultureInvariant);
        Assert.True(contextMin.Success, "ContextMenu MinWidth missing");
        Assert.True(flyoutMin.Success, "MenuFlyoutPresenter MinWidth missing");
        Assert.True(int.Parse(contextMin.Groups[1].Value) <= 160, "ContextMenu MinWidth still fat: " + contextMin.Groups[1].Value);
        Assert.True(int.Parse(flyoutMin.Groups[1].Value) <= 170, "MenuFlyout MinWidth still fat: " + flyoutMin.Groups[1].Value);

        // Window chrome: fine-line icons + restore geometry + control path styling.
        Assert.Contains("x:Key=\"Ariadne.Icon.Minimize\"", theme, StringComparison.Ordinal);
        Assert.Contains("x:Key=\"Ariadne.Icon.Maximize\"", theme, StringComparison.Ordinal);
        Assert.Contains("x:Key=\"Ariadne.Icon.Restore\"", theme, StringComparison.Ordinal);
        Assert.Contains("x:Key=\"Ariadne.Icon.Close\"", theme, StringComparison.Ordinal);
        Assert.Contains("Button.window-control Path.icon", theme, StringComparison.Ordinal);
        Assert.Contains("Ariadne.AccentPrimary", theme, StringComparison.Ordinal);
        Assert.Contains("scale(1.08)", theme, StringComparison.Ordinal);
        Assert.Contains("Path.icon.minimize", theme, StringComparison.Ordinal);
        Assert.Contains("Classes=\"window-control\"", main, StringComparison.Ordinal);
        Assert.Contains("icon minimize", main, StringComparison.Ordinal);
        Assert.Contains("x:Name=\"MaximizeRestoreIcon\"", main, StringComparison.Ordinal);
        Assert.Contains("Ariadne.Icon.Restore", mainCode, StringComparison.Ordinal);
        // Git default detail rail is intentionally narrower than the old 340px panel.
        var git = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "GitPageView.axaml"));
        Assert.Contains("Width=\"288\"", git, StringComparison.Ordinal);
        Assert.DoesNotContain("Width=\"340\"", git, StringComparison.Ordinal);

        // Integrated search shell on Template + RunLog (not bare TextBox + fat primary block).
        Assert.Contains("Selector=\"Border.search-shell\"", theme, StringComparison.Ordinal);
        Assert.Contains("Selector=\"Button.search-submit\"", theme, StringComparison.Ordinal);
        Assert.Contains("Selector=\"TextBox.search-input\"", theme, StringComparison.Ordinal);
        Assert.Contains("Classes=\"search-shell\"", template, StringComparison.Ordinal);
        Assert.Contains("Classes=\"search-input\"", template, StringComparison.Ordinal);
        Assert.Contains("Classes=\"search-submit\"", template, StringComparison.Ordinal);
        Assert.Contains("Classes=\"search-shell\"", runLog, StringComparison.Ordinal);
        Assert.Contains("Classes=\"search-submit\"", runLog, StringComparison.Ordinal);
        Assert.Contains("btn-compact", template, StringComparison.Ordinal);
        Assert.Contains("btn-compact", runLog, StringComparison.Ordinal);
        Assert.Contains("Selector=\"Button.btn-compact\"", theme, StringComparison.Ordinal);
        // Regression: RunLog must not ship detached primary square search next to filters.
        Assert.DoesNotContain(
            "Classes=\"primary\"\n                  Width=\"36\"",
            runLog,
            StringComparison.Ordinal);
    }

    [Fact]
    public void ProviderModelRefresh_IsReadOnlyAndRejectsDraftProviders()
    {
        var settings = File.ReadAllText(Path.Combine(
            ResolveDesktopSource("ViewModels"),
            "SettingsPageViewModel.cs"));
        var start = settings.IndexOf("private async Task FetchModelsAsync()", StringComparison.Ordinal);
        var end = settings.IndexOf("private Task<bool> SaveGeneralAsync()", start, StringComparison.Ordinal);

        Assert.True(start >= 0 && end > start);
        var refresh = settings[start..end];
        Assert.Contains("FetchProviderModelsAsync", refresh, StringComparison.Ordinal);
        Assert.Contains("CanUsePersistedProvider", refresh, StringComparison.Ordinal);
        Assert.DoesNotContain("SaveModelAsync", refresh, StringComparison.Ordinal);
        Assert.Contains("RefreshModelsCommand = new RelayCommand(() => _ = FetchModelsAsync(), CanUsePersistedProvider)", settings, StringComparison.Ordinal);
        Assert.Contains("SaveProviderKeyCommand = new RelayCommand(() => _ = SaveProviderKeyAsync(), CanUsePersistedProvider)", settings, StringComparison.Ordinal);
        Assert.Contains("!selected.IsDraft", settings, StringComparison.Ordinal);
        Assert.Contains("markDraft: !fromConfig.Configured", settings, StringComparison.Ordinal);
        var view = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "SettingsPageView.axaml"));
        Assert.Contains("Text=\"{Binding ProviderScopeHelpText}\"", view, StringComparison.Ordinal);
    }

    [Fact]
    public void ProviderRemoval_UsesImpactPreviewDangerConfirmationAndRevision()
    {
        var settings = File.ReadAllText(Path.Combine(
            ResolveDesktopSource("ViewModels"),
            "SettingsPageViewModel.cs"));
        var view = File.ReadAllText(Path.Combine(
            ResolveDesktopSource("Views"),
            "SettingsPageView.axaml"));

        Assert.Contains("PreviewProviderRemovalAsync(providerId)", settings, StringComparison.Ordinal);
        Assert.Contains("preview.BlockingReferences.Count > 0", settings, StringComparison.Ordinal);
        Assert.Contains("Severity = DialogSeverity.Danger", settings, StringComparison.Ordinal);
        Assert.Contains("RemoveProviderAsync(providerId, preview.Revision)", settings, StringComparison.Ordinal);
        Assert.Contains("Command=\"{Binding RemoveProviderCommand}\"", view, StringComparison.Ordinal);
    }

    [Fact]
    public void SettingsNavigation_UsesStandardSingleSelectionForTabsAndSections()
    {
        var settings = File.ReadAllText(Path.Combine(
            ResolveDesktopSource("ViewModels"),
            "SettingsPageViewModel.cs"));
        var view = File.ReadAllText(Path.Combine(
            ResolveDesktopSource("Views"),
            "SettingsPageView.axaml"));
        var codeBehind = File.ReadAllText(Path.Combine(
            ResolveDesktopSource("Views"),
            "SettingsPageView.axaml.cs"));

        Assert.Single(Regex.Matches(view, "ItemsSource=\\\"\\{Binding Tabs\\}\\\""));
        Assert.Contains("<ListBox ItemsSource=\"{Binding Tabs}\"", view, StringComparison.Ordinal);
        Assert.Contains("SelectedItem=\"{Binding NavigationSelection, Mode=TwoWay}\"", view, StringComparison.Ordinal);
        Assert.Contains("ItemsSource=\"{Binding SectionIndexItems}\"", view, StringComparison.Ordinal);
        Assert.Contains("SelectedItem=\"{Binding SectionNavigationSelection, Mode=TwoWay}\"", view, StringComparison.Ordinal);
        // Section anchors keep subtitle class; attributes may wrap across lines after layout polish.
        // Includes AppRuntime section added for global Qdrant runtime settings.
        Assert.Equal(23, Regex.Matches(
            view,
            "Binding [A-Za-z]+SectionTitle",
            RegexOptions.CultureInvariant).Count);
        Assert.True(Regex.IsMatch(
            view,
            "ConfirmationsSectionTitle[\\s\\S]{0,120}?Classes=\\\"subtitle\\\"",
            RegexOptions.CultureInvariant));
        Assert.Contains("SectionIndexItems", settings, StringComparison.Ordinal);
        Assert.Contains("ScrollToSectionRequested", settings, StringComparison.Ordinal);
        Assert.Contains("OnScrollToSectionRequested", codeBehind, StringComparison.Ordinal);
        Assert.Contains("SettingsContentScroll.Offset = new Vector", codeBehind, StringComparison.Ordinal);
        Assert.DoesNotContain("BringIntoView", codeBehind, StringComparison.Ordinal);
        Assert.Contains("OnAttachedToVisualTree", codeBehind, StringComparison.Ordinal);
        Assert.Contains("OnDetachedFromVisualTree", codeBehind, StringComparison.Ordinal);
        Assert.Contains("DetachBehaviors();", codeBehind, StringComparison.Ordinal);
        Assert.Contains("ClearFolderPicker(_folderPicker)", codeBehind, StringComparison.Ordinal);
    }

    [Fact]
    public void ProjectAiConversationUi_AppendsRevisionDeltaWithoutRebuildingBubbles()
    {
        var history = new List<ProjectAiChatMessage>();
        var bubbles = new System.Collections.ObjectModel.ObservableCollection<ChatBubbleViewModel>();
        var firstTurn = new ProjectAiResponse(
            "第一轮回答",
            Array.Empty<ProjectAiChatMessage>(),
            null,
            string.Empty,
            ConversationId: "works",
            ConversationRevision: 1,
            NewMessages: new[]
            {
                new ProjectAiChatMessage("user", "第一轮问题"),
                new ProjectAiChatMessage("assistant", "第一轮回答"),
            },
            ConversationSnapshot: new[]
            {
                new ProjectAiChatMessage("user", "第一轮问题"),
                new ProjectAiChatMessage("assistant", "第一轮回答"),
            },
            HistoryTruncated: true);

        var revision = ProjectAiConversationUi.Apply(firstTurn, history, bubbles, currentRevision: null);

        Assert.Equal(1, revision);
        Assert.Equal(firstTurn.ConversationSnapshot, history);
        Assert.Equal(2, bubbles.Count);
        Assert.True(ProjectAiConversationUi.ContextWasCompacted(firstTurn));

        revision = ProjectAiConversationUi.Apply(firstTurn, history, bubbles, revision);
        Assert.Equal(1, revision);
        Assert.Equal(2, bubbles.Count);

        var secondTurn = new ProjectAiResponse(
            "第二轮回答",
            Array.Empty<ProjectAiChatMessage>(),
            null,
            string.Empty,
            ConversationId: "works",
            ConversationRevision: 2,
            NewMessages: new[]
            {
                new ProjectAiChatMessage("user", "第二轮问题"),
                new ProjectAiChatMessage("assistant", "第二轮回答"),
            });

        revision = ProjectAiConversationUi.Apply(secondTurn, history, bubbles, revision);

        Assert.Equal(2, revision);
        Assert.Equal(4, history.Count);
        Assert.Equal("第二轮回答", history[^1].Content);
        Assert.Equal(4, bubbles.Count);
        Assert.Equal("第一轮问题", bubbles[0].Content);
        Assert.Equal("第二轮回答", bubbles[3].Content);
    }

    [Fact]
    public void ProjectAiPages_UseSharedRevisionDeltaProtocol()
    {
        var works = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "WorksPageViewModel.cs"));
        var workspace = File.ReadAllText(Path.Combine(ResolveDesktopSource("ViewModels"), "WorkspacePageViewModel.cs"));

        Assert.Contains("ProjectAiConversationUi.Apply(", works, StringComparison.Ordinal);
        Assert.Contains("ProjectAiConversationUi.Apply(", workspace, StringComparison.Ordinal);
        Assert.Contains("conversationRevision: _projectAiConversationRevision", works, StringComparison.Ordinal);
        Assert.Contains("conversationRevision: _projectAiConversationRevision", workspace, StringComparison.Ordinal);
        Assert.Contains("ProjectAiChatAsync(\n                instruction,\n                workflowIdToRun", works, StringComparison.Ordinal);
        Assert.Contains("var message = ProjectAiMessage;", workspace, StringComparison.Ordinal);
        Assert.Contains("ProjectAiChatAsync(\n                message,\n                workflowIdToRun", workspace, StringComparison.Ordinal);
        Assert.DoesNotContain("ProjectAiBubbles.Clear()", works, StringComparison.Ordinal);
        Assert.DoesNotContain("ProjectAiBubbles.Clear()", workspace, StringComparison.Ordinal);
    }

    [Fact]
    public void ProjectAiComposer_StretchesToConversationPanelWidth()
    {
        var composer = File.ReadAllText(Path.Combine(ResolveDesktopSource("Controls"), "ProjectAiComposer.axaml"));
        var workspace = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorkspacePageView.axaml"));
        var works = File.ReadAllText(Path.Combine(ResolveDesktopSource("Views"), "WorksPageView.axaml"));

        Assert.Contains("HorizontalContentAlignment=\"Stretch\"", composer, StringComparison.Ordinal);
        Assert.Contains("HorizontalAlignment=\"Stretch\"", composer, StringComparison.Ordinal);
        Assert.Contains("<TextBox Grid.Column=\"0\"", composer, StringComparison.Ordinal);
        Assert.Contains("RowDefinitions=\"*,Auto\" HorizontalAlignment=\"Stretch\"", works, StringComparison.Ordinal);
        Assert.Contains("RowDefinitions=\"*,Auto\" HorizontalAlignment=\"Stretch\"", workspace, StringComparison.Ordinal);
        Assert.Contains("HorizontalAlignment=\"Stretch\" />", works, StringComparison.Ordinal);
        Assert.Contains("HorizontalAlignment=\"Stretch\" />", workspace, StringComparison.Ordinal);
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
