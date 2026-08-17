using System.Diagnostics;
using System.Net;
using System.Net.Sockets;
using System.Text;
using System.Text.Json;
using Ariadne.Desktop.Backend;
using Xunit;
using Xunit.Abstractions;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 桌面端**设置项往返**与**真实 IPC 性能**：真实 sidecar 进程 + 真实 JSON-line 管道。
///
/// ## 两半各自解决什么
///
/// **上半（设置往返）** 直接对着 `/goal` 那句话：「该有却没有、有却没用、
/// 不该显示却显示、描述错误」。判据是三环——**保存不抛**、**读回一致**、
/// **磁盘 YAML 真的变了**。少任何一环，都有一整类实现能骗过去：
/// 少第三环，「只改了内存返回值」全绿；少第二环，「保存成功但读回默认值」全绿。
/// U157 就是在「少一环断言」上让我自己先误判了一次。
///
/// **下半（性能）** 测的是 IPC 这一层的耗时。Rust 侧 `release_acceptance.rs`
/// 已有百万字检索基线，但它 (1) 只测 Rust 进程内，(2) **只把数字写进证据文件、
/// 没有任何耗时阈值断言**——所以它记录性能，却拦不住性能退化。
/// 而用户感知的「卡」大半发生在 IPC 这一层：一次请求要跨进程序列化、
/// 过 stdio 管道、反序列化、再走回来。这一段此前零覆盖。
///
/// ## 性能断言的写法约束（很重要）
///
/// 这台机器是 ARM 开发板（3.8G 内存），且测试与 dotnet 编译可能并行。
/// **所以阈值必须选在「即使慢 10 倍也不该到」的量级上**，
/// 断的是数量级异常（O(n²)、每次请求重开数据库、同步阻塞全表扫），
/// 不是几十毫秒的抖动。一条会因机器负载而红的性能测试，
/// 会被当成环境问题而不是被修——那比没有测试更糟。
/// 具体数字连同实测值一起打进测试输出，便于后续对比。
///
/// sidecar 未编译时跳过（与 <see cref="BackendColdStartTests"/> 同约定）。
/// </summary>
public sealed class FrontendSettingsAndPerfJourneyTests : IDisposable
{
    private const string ProviderId = "primary";
    private const string ModelId = "perf-model";

    private readonly ITestOutputHelper _output;

    private readonly DirectoryInfo _temp =
        Directory.CreateTempSubdirectory("ariadne-settings-perf-");

    public FrontendSettingsAndPerfJourneyTests(ITestOutputHelper output)
    {
        _output = output;
    }

    public void Dispose()
    {
        try
        {
            _temp.Delete(recursive: true);
        }
        catch
        {
            // 清理失败不影响断言结论。
        }
    }

    private static string? ResolveSidecar()
    {
        SidecarAppStateIsolation.RequireIsolatedAppState();
        Environment.SetEnvironmentVariable(
            "ARIADNE_SECRET_MASTER_KEY", "settings-perf-master-key");

        var fromEnv = Environment.GetEnvironmentVariable("ARIADNE_BACKEND_IPC");
        if (!string.IsNullOrWhiteSpace(fromEnv) && File.Exists(fromEnv))
        {
            return fromEnv;
        }
        return JsonLineBackendClient.DiscoverBackendCommand(
            AppContext.BaseDirectory,
            Environment.CurrentDirectory);
    }

    // ════════════════════════════════════════════════════════
    // 设置往返：工作流设置
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// 设置页-工作流：改 4 个数值 → 读回一致 → 磁盘 YAML 真的变了。
    ///
    /// 特意改**每一个**字段而不是只改一个：一个「只透传第一个字段、
    /// 其余用默认值填」的实现在单字段测试下全绿。
    /// 每个值都选成与默认明显不同的数，避免「碰巧等于默认」而失去区分能力。
    /// </summary>
    [Fact]
    public async Task Settings_WorkflowConfig_RoundTripsAndReachesDisk()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Settings_WorkflowConfig_RoundTripsAndReachesDisk)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "工作流设置");
        await client.CreateProjectAsync(projectRoot, "工作流设置");

        var before = await client.GetWorkflowSettingsAsync();
        var desired = before.Workflow with
        {
            DefaultTimeoutMs = 123_456,
            MaxLoopIterations = 37,
            MaxToolRounds = 11,
            CheckpointEnabled = !before.Workflow.CheckpointEnabled,
            RunEventRetentionDays = 29,
        };

        var saved = await client.SaveWorkflowSettingsAsync(new WorkflowSettings(desired));

        // 环 1：保存返回值与请求一致。
        Assert.Equal(desired.DefaultTimeoutMs, saved.Workflow.DefaultTimeoutMs);
        Assert.Equal(desired.MaxLoopIterations, saved.Workflow.MaxLoopIterations);
        Assert.Equal(desired.MaxToolRounds, saved.Workflow.MaxToolRounds);
        Assert.Equal(desired.CheckpointEnabled, saved.Workflow.CheckpointEnabled);
        Assert.Equal(desired.RunEventRetentionDays, saved.Workflow.RunEventRetentionDays);

        // 环 2：重新读一次（排除「只改了返回值」）。
        var reread = await client.GetWorkflowSettingsAsync();
        Assert.Equal(desired.DefaultTimeoutMs, reread.Workflow.DefaultTimeoutMs);
        Assert.Equal(desired.MaxLoopIterations, reread.Workflow.MaxLoopIterations);
        Assert.Equal(desired.MaxToolRounds, reread.Workflow.MaxToolRounds);
        Assert.Equal(desired.CheckpointEnabled, reread.Workflow.CheckpointEnabled);
        Assert.Equal(desired.RunEventRetentionDays, reread.Workflow.RunEventRetentionDays);

        // 环 3：磁盘。配置真相源是 .config/workflow.yaml。
        var yaml = Path.Combine(projectRoot, ".config", "workflow.yaml");
        Assert.True(File.Exists(yaml), $"工作流配置文件不存在：{yaml}");
        var text = await File.ReadAllTextAsync(yaml);
        Assert.Contains("123456", text);
        Assert.Contains("37", text);
    }

    /// <summary>
    /// 设置页-Git：开关 + 忽略路径列表往返。
    ///
    /// 列表类字段单独测：它是最容易被「序列化成 null / 丢空元素 / 丢元素」
    /// 的一类，而 U156 正是一个「空集合被序列化成 null」引发的 P0。
    /// 这里特意含中文路径与带空格路径。
    ///
    /// ⚠️ **判据不能要求「原样返回」，这一点我踩过一次**：初版断言
    /// `Assert.Equal(ignored, reread.Git.IgnoredPaths)` 失败，
    /// 实际返回 `[".cache", "参考 资料", "草稿/临时稿.md"]`——
    /// 顺序变了、尾斜杠被去掉了。查 `GitConfig::normalize_ignored_paths`
    /// （`config/models.rs:1099`）后确认这是**刻意的规范化**：
    /// 收进 `BTreeSet` 去重排序、`normalize_git_ignored_path` 去尾斜杠、
    /// 统一 `\` → `/`、并拒绝绝对路径与 `..` 逃逸。**这是对的，不是缺陷。**
    /// 所以判据改成「按规范化语义比对」——集合相等 + 尾斜杠已去。
    /// 断言原样顺序等于把一个正确的安全措施报成缺陷。
    /// </summary>
    [Fact]
    public async Task Settings_GitConfig_RoundTripsIncludingIgnoredPathList()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Settings_GitConfig_RoundTripsIncludingIgnoredPathList)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "Git设置");
        await client.CreateProjectAsync(projectRoot, "Git设置");

        var before = await client.GetGitSettingsAsync();
        // "参考 资料/" 带尾斜杠：用来钉住规范化确实发生（返回值应无尾斜杠）。
        var ignored = new[] { "草稿/临时稿.md", "参考 资料/", ".cache" };
        // 规范化后的期望：去尾斜杠 + 排序（BTreeSet 的字典序）。
        var expected = new[] { ".cache", "参考 资料", "草稿/临时稿.md" };
        var desired = before.Git with
        {
            TrackDocuments = !before.Git.TrackDocuments,
            TrackWorkflows = !before.Git.TrackWorkflows,
            TrackSkills = !before.Git.TrackSkills,
            TrackNonSensitiveConfig = !before.Git.TrackNonSensitiveConfig,
            IgnoredPaths = ignored,
        };

        var saved = await client.SaveGitSettingsAsync(new GitSettings(desired));
        Assert.Equal(desired.TrackDocuments, saved.Git.TrackDocuments);

        var reread = await client.GetGitSettingsAsync();
        Assert.Equal(desired.TrackDocuments, reread.Git.TrackDocuments);
        Assert.Equal(desired.TrackWorkflows, reread.Git.TrackWorkflows);
        Assert.Equal(desired.TrackSkills, reread.Git.TrackSkills);
        Assert.Equal(desired.TrackNonSensitiveConfig, reread.Git.TrackNonSensitiveConfig);

        // 三条都在（一条不丢），且已按规范化形式存放。
        Assert.Equal(expected, reread.Git.IgnoredPaths);
        Assert.DoesNotContain("参考 资料/", reread.Git.IgnoredPaths);

        // 磁盘：中文路径必须原样落盘（不被转义成 \uXXXX 或截断）。
        var yaml = Path.Combine(projectRoot, ".config", "git.yaml");
        Assert.True(File.Exists(yaml), $"Git 配置文件不存在：{yaml}");
        var text = await File.ReadAllTextAsync(yaml);
        Assert.Contains("草稿/临时稿.md", text);
        Assert.Contains("参考 资料", text);
    }

    /// <summary>
    /// 设置页-空列表：把忽略路径**清空**必须真的清空。
    ///
    /// 单列一条，因为它正是 U156 那个形状：空集合在序列化边界上
    /// 最容易变成 `null`，而后端对 `null` 与「空数组」的处理可能不同。
    /// 判据是「读回也是空」——一个把 null 当「不修改」的实现会让用户
    /// 永远删不掉最后一条忽略路径。
    /// </summary>
    [Fact]
    public async Task Settings_ClearIgnoredPaths_ActuallyEmptiesTheList()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Settings_ClearIgnoredPaths_ActuallyEmptiesTheList)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "清空列表");
        await client.CreateProjectAsync(projectRoot, "清空列表");

        var before = await client.GetGitSettingsAsync();
        await client.SaveGitSettingsAsync(new GitSettings(
            before.Git with { IgnoredPaths = new[] { "先加一条.md" } }));
        var withOne = await client.GetGitSettingsAsync();
        Assert.Contains("先加一条.md", withOne.Git.IgnoredPaths);

        // 再清空。
        await client.SaveGitSettingsAsync(new GitSettings(
            withOne.Git with { IgnoredPaths = Array.Empty<string>() }));

        var cleared = await client.GetGitSettingsAsync();
        Assert.Empty(cleared.Git.IgnoredPaths);
    }

    /// <summary>
    /// 设置页-预算：U112 的双重语义必须在**真实 IPC 往返**后仍然保持。
    ///
    /// CLAUDE.md 明写这两个 0 的含义**不同、且刻意如此**：
    /// - 日预算 `budget_usd` 的 `0` = 不设上限
    /// - Auto Mode `preauthorized_usd` 的 `Some(0.0)` = **显式零额度**，
    ///   「不限制」只由 `None` 表达
    ///
    /// 这条语义横跨 C# `double?` → JSON → Rust `Option&lt;f64&gt;` 三层。
    /// **纯 Rust 测试测不到中间那层**：C# 的 `null` 与 `0.0` 在
    /// `System.Text.Json` 下是否真的产出 `null` 与 `0`，只有走真实管道才知道。
    /// 一旦 `Some(0.0)` 在往返中被规整成 `None`，用户刻意设的零额度被静默解除——
    /// CLAUDE.md 称之为「安全性倒退」。
    /// </summary>
    [Fact]
    public async Task Settings_PreauthorizedZero_SurvivesIpcRoundTripAsExplicitZero()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Settings_PreauthorizedZero_SurvivesIpcRoundTripAsExplicitZero)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "预算语义");
        await client.CreateProjectAsync(projectRoot, "预算语义");

        // 显式零额度。
        var zero = await client.UpdateBudgetConfigAsync(12.5, 0.0);
        Assert.Equal(0.0, zero.PreauthorizedUsd);
        var zeroReread = await client.GetBudgetStatusAsync();
        Assert.NotNull(zeroReread.PreauthorizedUsd);
        Assert.Equal(0.0, zeroReread.PreauthorizedUsd!.Value);

        // 不限制 = null，必须与上面可区分。
        var unlimited = await client.UpdateBudgetConfigAsync(12.5, null);
        Assert.Null(unlimited.PreauthorizedUsd);
        var unlimitedReread = await client.GetBudgetStatusAsync();
        // xunit 2.x 的 Assert.Null 没有 message 重载，用 True 带上诊断信息。
        Assert.True(
            unlimitedReread.PreauthorizedUsd is null,
            $"「不限制」在往返后变成了 {unlimitedReread.PreauthorizedUsd}——"
            + "U112 的两种 0 语义被合并了，用户刻意设的零额度会被静默解除");

        // 再设回显式零：确认往返是可逆的，而不是单向丢信息。
        var backToZero = await client.UpdateBudgetConfigAsync(12.5, 0.0);
        Assert.NotNull(backToZero.PreauthorizedUsd);
        Assert.Equal(0.0, backToZero.PreauthorizedUsd!.Value);

        // 日预算 0 = 不设上限，是**另一个**字段的**另一种**语义，一并钉住。
        var noDailyCap = await client.UpdateBudgetConfigAsync(0.0, 0.0);
        Assert.Equal(0.0, noDailyCap.BudgetUsd);
        Assert.NotNull(noDailyCap.PreauthorizedUsd);
        Assert.Equal(0.0, noDailyCap.PreauthorizedUsd!.Value);
    }

    /// <summary>
    /// 设置页-节点预设：每个节点类型的模型/超时/预算往返。
    ///
    /// 这批设置直接决定工作流节点的行为。若保存后读回是默认值，
    /// 用户在设置页做的全部调整都不生效，而界面看起来一切正常。
    /// </summary>
    [Fact]
    public async Task Settings_NodePresets_RoundTripPerNodeTypeValues()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Settings_NodePresets_RoundTripPerNodeTypeValues)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "节点预设");
        await client.CreateProjectAsync(projectRoot, "节点预设");

        var before = await client.GetNodePresetSettingsAsync();
        Assert.NotEmpty(before.Presets);

        // 改全局默认 + 第一个节点类型的三个值。
        var first = before.Presets[0];
        var mutated = before.Presets
            .Select((preset, index) => index == 0
                ? preset with { ModelId = "preset-model-x", TimeoutMs = 76_543, BudgetUsd = 3.25 }
                : preset)
            .ToArray();

        var saved = await client.SaveNodePresetSettingsAsync(before with
        {
            Presets = mutated,
            DefaultTimeoutMs = 54_321,
            DefaultBudgetUsd = 9.75,
        });

        Assert.Equal(54_321, saved.DefaultTimeoutMs);
        Assert.Equal(9.75, saved.DefaultBudgetUsd);

        var reread = await client.GetNodePresetSettingsAsync();
        Assert.Equal(54_321, reread.DefaultTimeoutMs);
        Assert.Equal(9.75, reread.DefaultBudgetUsd);

        var rereadFirst = reread.Presets.FirstOrDefault(p => p.NodeType == first.NodeType);
        Assert.NotNull(rereadFirst);
        Assert.Equal("preset-model-x", rereadFirst!.ModelId);
        Assert.Equal(76_543, rereadFirst.TimeoutMs);
        Assert.Equal(3.25, rereadFirst.BudgetUsd);
    }

    /// <summary>
    /// 设置页-权限：作用域权限与工具开关往返。
    ///
    /// 权限是**安全边界**，往返丢失比普通设置严重：用户以为关掉了网络访问，
    /// 实际保存后又变回默认允许。所以这里既测「关」也测「开」，
    /// 并断言**其余作用域不被顺带改写**（一个「整表覆盖」的实现会清掉别的作用域）。
    /// </summary>
    [Fact]
    public async Task Settings_Permissions_RoundTripWithoutClobberingOtherScopes()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Settings_Permissions_RoundTripWithoutClobberingOtherScopes)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "权限设置");
        await client.CreateProjectAsync(projectRoot, "权限设置");

        var before = await client.GetPermissionsSettingsAsync();

        var controls = before.ToolControls.ToDictionary(
            pair => pair.Key,
            pair => (IReadOnlyDictionary<string, bool?>)pair.Value.ToDictionary(
                inner => inner.Key, inner => inner.Value));
        controls["global"] = new Dictionary<string, bool?> { ["write"] = true, ["register"] = false };
        controls["writer"] = new Dictionary<string, bool?> { ["write"] = true };

        var desiredPolicy = before.Policy with
        {
            AllowNetwork = false,
            AllowWebSearch = false,
            AllowHttpSkill = false,
            AllowWasmNetwork = false,
            AllowSecretRead = false,
        };

        var saved = await client.SavePermissionsSettingsAsync(before with
        {
            Policy = desiredPolicy,
            ToolControls = controls,
        });

        Assert.False(saved.Policy.AllowNetwork);
        Assert.False(saved.Policy.AllowSecretRead);

        var reread = await client.GetPermissionsSettingsAsync();
        Assert.False(reread.Policy.AllowNetwork, "关掉网络访问后读回又变成允许了——安全设置未持久化");
        Assert.False(reread.Policy.AllowWebSearch);
        Assert.False(reread.Policy.AllowHttpSkill);
        Assert.False(reread.Policy.AllowWasmNetwork);
        Assert.False(reread.Policy.AllowSecretRead);

        // 两个作用域都必须在（写一个不能把另一个冲掉）。
        Assert.True(reread.ToolControls.ContainsKey("global"));
        Assert.True(reread.ToolControls.ContainsKey("writer"));
        Assert.True(reread.ToolControls["global"]["write"]);
        Assert.False(reread.ToolControls["global"]["register"]);
    }

    /// <summary>
    /// 设置页-UI 偏好：主题与自定义三色往返。
    ///
    /// UI 偏好落在 **app-state**（跨项目），不是项目配置。
    /// 判据含「换一个客户端进程后仍在」——个性化设置若只活在内存里，
    /// 用户每次重启都要重设主题。
    /// </summary>
    [Fact]
    public async Task Settings_UiPreferences_PersistAcrossClientProcesses()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Settings_UiPreferences_PersistAcrossClientProcesses)))
        {
            return;
        }

        var projectRoot = Path.Combine(_temp.FullName, "界面偏好");
        const string brand = "#C43D63";

        using (var first = new JsonLineBackendClient(sidecar))
        {
            await first.CreateProjectAsync(projectRoot, "界面偏好");
            var before = await first.GetUiPreferencesAsync();
            await first.SaveUiPreferencesAsync(before with
            {
                Theme = "rose",
                ThemeBrandColor = brand,
                OnboardingSeen = true,
                ProjectPanelVisible = !before.ProjectPanelVisible,
            });
        }

        // 换进程读回。
        using var second = new JsonLineBackendClient(sidecar);
        await second.OpenProjectAsync(projectRoot, "界面偏好");
        var reread = await second.GetUiPreferencesAsync();

        Assert.Equal("rose", reread.Theme);
        Assert.Equal(brand, reread.ThemeBrandColor);
        Assert.True(reread.OnboardingSeen, "已看过引导的标记没持久化，用户每次启动都会被重新引导");
    }

    // ════════════════════════════════════════════════════════
    // 性能：真实 IPC 往返
    // ════════════════════════════════════════════════════════

    /// <summary>
    /// **IPC 轻量请求往返延迟**。
    ///
    /// `get_current_project` 是最轻的命令之一（读一个内存状态）。
    /// 它的耗时基本就是「跨进程一次往返」的固定成本：序列化 + 写管道 +
    /// 后端解析 + 分发 + 回写 + 前端解析。
    ///
    /// 这个数字要紧，因为前端在页面切换、状态刷新、轮询里会**成百次**调它。
    /// 若单次固定成本是 50ms，切一次页面就是半秒起步。
    ///
    /// 阈值取**中位数 &lt; 200ms**：ARM 板 + 可能有并发负载，
    /// 断的是「每次请求都在重开数据库/重读配置文件」这个量级的问题，
    /// 不是几十毫秒抖动。取中位数而非平均：单次 GC 或调度抖动不该让测试红。
    /// </summary>
    [Fact]
    public async Task Perf_LightweightIpcRoundTrip_StaysUnderBudget()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Perf_LightweightIpcRoundTrip_StaysUnderBudget)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "IPC延迟");
        await client.CreateProjectAsync(projectRoot, "IPC延迟");

        // 预热：首次调用含 sidecar 启动与首次配置读取，不计入统计。
        for (var i = 0; i < 3; i++)
        {
            await client.GetCurrentProjectAsync();
        }

        const int rounds = 40;
        var samples = new List<double>(rounds);
        for (var i = 0; i < rounds; i++)
        {
            var sw = Stopwatch.StartNew();
            await client.GetCurrentProjectAsync();
            sw.Stop();
            samples.Add(sw.Elapsed.TotalMilliseconds);
        }

        samples.Sort();
        var median = samples[samples.Count / 2];
        var p95 = samples[(int)(samples.Count * 0.95)];
        _output.WriteLine(
            $"IPC 轻量往返 {rounds} 次：中位 {median:F1}ms，p95 {p95:F1}ms，"
            + $"最快 {samples[0]:F1}ms，最慢 {samples[^1]:F1}ms");

        Assert.True(
            median < 200,
            $"IPC 轻量请求往返中位数 {median:F1}ms 过高（阈值 200ms）。"
            + "前端在页面切换与轮询里会成百次调这类命令，"
            + "单次固定成本过高会让整个界面「哪都慢一点」。"
            + $"全部样本：{string.Join(", ", samples.Select(s => s.ToString("F0")))}");
    }

    /// <summary>
    /// **大文档读写经 IPC 的耗时**：50 万字中文正文。
    ///
    /// 这是本产品的核心场景（目标 100 万字+）。一次章节保存要把整篇正文
    /// 塞进一行 JSON 过管道——若实现里有按字节扫描、多次全量拷贝，
    /// 或 JSON 转义把中文逐字变成 `\uXXXX`（体积 ×6），这里就会现形。
    ///
    /// 阈值取 **写 &lt; 30s、读 &lt; 30s**，并额外断言**吞吐 &gt; 0.5 MB/s**。
    /// 数字放得很宽是刻意的：ARM 板 + debug 构建 + 可能并发编译。
    /// 断的是数量级（比如 O(n²) 会让 50 万字直接跑到几分钟），不是常数因子。
    /// ⚠️ **debug 构建下的绝对值没有参考价值**——它只用来抓量级异常。
    /// 要看真实性能须 `cargo build --release` 后再跑，
    /// 这一点与 Rust 侧 `release_acceptance.rs` 的 `assert_release_profile` 同理。
    /// </summary>
    [Fact]
    public async Task Perf_LargeChapterSaveAndLoad_ScalesLinearly()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Perf_LargeChapterSaveAndLoad_ScalesLinearly)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "大文档性能");
        await client.CreateProjectAsync(projectRoot, "大文档性能");

        var documentId = Path.Combine(projectRoot, "chapters", "chapter-big.md");
        Directory.CreateDirectory(Path.GetDirectoryName(documentId)!);
        await File.WriteAllTextAsync(documentId, "第一章\n");

        var body = BuildChineseProse(targetChars: 500_000);
        var byteCount = Encoding.UTF8.GetByteCount(body);
        Assert.True(body.Length >= 500_000, $"语料只生成了 {body.Length} 字");

        var opened = await client.GetDocumentContentDetailsAsync(documentId);

        var writeSw = Stopwatch.StartNew();
        var report = await client.SaveDocumentContentAsync(documentId, body, opened.Metadata.Version);
        writeSw.Stop();
        Assert.NotNull(report.Metadata);

        var readSw = Stopwatch.StartNew();
        var readBack = await client.GetDocumentContentDetailsAsync(documentId);
        readSw.Stop();

        // 正确性优先：性能测试也必须验内容，否则「快但写错」会被记成成功。
        Assert.Equal(body.Length, readBack.Content.Length);
        Assert.Equal(body, readBack.Content);

        var writeMbps = byteCount / 1024.0 / 1024.0 / Math.Max(writeSw.Elapsed.TotalSeconds, 0.001);
        var readMbps = byteCount / 1024.0 / 1024.0 / Math.Max(readSw.Elapsed.TotalSeconds, 0.001);
        _output.WriteLine(
            $"50 万字（{byteCount / 1024.0 / 1024.0:F2} MB UTF-8）经 IPC："
            + $"写 {writeSw.Elapsed.TotalSeconds:F2}s（{writeMbps:F2} MB/s）、"
            + $"读 {readSw.Elapsed.TotalSeconds:F2}s（{readMbps:F2} MB/s）");

        Assert.True(
            writeSw.Elapsed.TotalSeconds < 30,
            $"保存 50 万字用了 {writeSw.Elapsed.TotalSeconds:F1}s（阈值 30s）——"
            + "量级异常，疑似存在 O(n²) 扫描或多次全量拷贝");
        Assert.True(
            readSw.Elapsed.TotalSeconds < 30,
            $"读取 50 万字用了 {readSw.Elapsed.TotalSeconds:F1}s（阈值 30s）");
        Assert.True(
            writeMbps > 0.5 && readMbps > 0.5,
            $"经 IPC 的正文吞吐过低（写 {writeMbps:F2} MB/s、读 {readMbps:F2} MB/s，阈值 0.5）。"
            + "百万字项目下用户每次保存都要等这么久");
    }

    /// <summary>
    /// **正文体积翻倍时耗时不得超线性增长**——这条才是真正抓 O(n²) 的。
    ///
    /// 绝对阈值（上一条）在慢机器上只能放得很宽，宽到 O(n²) 也可能钻过去。
    /// 比值判据与机器速度**无关**：10 万字 → 40 万字（4 倍体积），
    /// 线性实现耗时约 4 倍，O(n²) 约 16 倍。
    /// 阈值取 **&lt; 10 倍**，给调度抖动、JSON 缓冲扩容留足余量，
    /// 但仍能拦住平方级。
    ///
    /// ⚠️ 这条**刻意不设绝对时间上限**：它测的是形状，不是速度。
    /// 在慢机器上两次都慢，比值依然成立。
    /// </summary>
    [Fact]
    public async Task Perf_DocumentSaveTime_DoesNotGrowQuadratically()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Perf_DocumentSaveTime_DoesNotGrowQuadratically)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "增长曲线");
        await client.CreateProjectAsync(projectRoot, "增长曲线");

        var small = await MeasureSaveAsync(client, projectRoot, "small.md", 100_000);
        var large = await MeasureSaveAsync(client, projectRoot, "large.md", 400_000);

        var ratio = large / Math.Max(small, 1.0);
        _output.WriteLine(
            $"体积 4 倍时耗时比：{ratio:F2}×（10 万字 {small:F0}ms → 40 万字 {large:F0}ms）"
            + "；线性≈4×，平方≈16×");

        Assert.True(
            ratio < 10,
            $"正文体积从 10 万字增到 40 万字（4 倍）时，保存耗时变成 {ratio:F1} 倍"
            + $"（{small:F0}ms → {large:F0}ms）。线性实现应约 4 倍、平方级约 16 倍——"
            + "这个比值说明存在超线性开销，百万字项目下会不可用");
    }

    private static async Task<double> MeasureSaveAsync(
        IAriadneBackendClient client, string projectRoot, string name, int chars)
    {
        var path = Path.Combine(projectRoot, "chapters", name);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        await File.WriteAllTextAsync(path, "占位\n");
        var opened = await client.GetDocumentContentDetailsAsync(path);
        var body = BuildChineseProse(chars);

        var sw = Stopwatch.StartNew();
        await client.SaveDocumentContentAsync(path, body, opened.Metadata.Version);
        sw.Stop();
        return sw.Elapsed.TotalMilliseconds;
    }

    /// <summary>
    /// **并发请求不得把 IPC 串成一条队**。
    ///
    /// 后端 sidecar 有多个 worker（`MAX_CONCURRENT_IPC_REQUESTS = 8`）。
    /// 前端在一次页面加载里会**并发**发多个查询（树、徽章、设置、状态）。
    /// 若客户端或后端把它们串行化，页面加载时间就是各请求之和。
    ///
    /// 判据：并发 8 个请求的总耗时，必须**明显小于**串行 8 个的耗时。
    /// 阈值取「并发总耗时 &lt; 串行总耗时 × 0.8」——留足余量，
    /// 断的是「完全串行」这个形状，不是要求理想 8 倍加速。
    /// </summary>
    [Fact]
    public async Task Perf_ConcurrentIpcRequests_AreNotSerialized()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Perf_ConcurrentIpcRequests_AreNotSerialized)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "并发IPC");
        await client.CreateProjectAsync(projectRoot, "并发IPC");

        for (var i = 0; i < 3; i++)
        {
            await client.GetCurrentProjectAsync();
        }

        const int count = 8;

        // 串行基线。
        var serialSw = Stopwatch.StartNew();
        for (var i = 0; i < count; i++)
        {
            await client.GetSidebarBadgesAsync();
        }
        serialSw.Stop();

        // 并发。
        var concurrentSw = Stopwatch.StartNew();
        await Task.WhenAll(Enumerable.Range(0, count)
            .Select(_ => client.GetSidebarBadgesAsync()));
        concurrentSw.Stop();

        var serialMs = serialSw.Elapsed.TotalMilliseconds;
        var concurrentMs = concurrentSw.Elapsed.TotalMilliseconds;
        _output.WriteLine(
            $"{count} 个请求：串行 {serialMs:F0}ms、并发 {concurrentMs:F0}ms"
            + $"（比值 {concurrentMs / Math.Max(serialMs, 1):F2}）");

        Assert.True(
            concurrentMs < serialMs * 0.8,
            $"并发 {count} 个 IPC 请求耗时 {concurrentMs:F0}ms，与串行 {serialMs:F0}ms 相当——"
            + "请求被串行化了。前端一次页面加载会并发多个查询，"
            + "串行化会让加载时间等于各请求之和");
    }

    /// <summary>
    /// **作品树加载耗时随章节数线性**。
    ///
    /// 作品页每次进入都要拉一次树。百万字通常是几百章，
    /// 若树构建里每章都去读一次文件或做一次全量扫描，
    /// 章节数一多就会卡在进入页面这一步。
    ///
    /// 这里建 120 章（真实规模的下限），断言**单章摊销 &lt; 200ms**。
    /// 同时断言树里**真的有这些章节**——只测速度不验内容，
    /// 一个「返回空树」的实现会是最快的。
    /// </summary>
    [Fact]
    public async Task Perf_WorksTreeWithManyChapters_LoadsInBoundedTime()
    {
        var sidecar = ResolveSidecar();
        if (sidecar is null && SidecarAppStateIsolation.AllowSkipWhenSidecarMissing(
                nameof(Perf_WorksTreeWithManyChapters_LoadsInBoundedTime)))
        {
            return;
        }

        using var client = new JsonLineBackendClient(sidecar);
        var projectRoot = Path.Combine(_temp.FullName, "多章节树");
        await client.CreateProjectAsync(projectRoot, "多章节树");

        const int chapters = 120;
        var chaptersDir = Path.Combine(projectRoot, "chapters");
        Directory.CreateDirectory(chaptersDir);

        // 每章 2000 字：120 章 ≈ 24 万字，规模真实但建库时间可接受。
        var chapterBody = BuildChineseProse(2_000);
        for (var i = 1; i <= chapters; i++)
        {
            var relative = Path.Combine("chapters", $"chapter-{i:D3}.md");
            var absolute = Path.Combine(projectRoot, relative);
            await File.WriteAllTextAsync(absolute, $"# 第 {i} 章\n\n{chapterBody}");
            await client.ImportChapterAsync(new ChapterImportRequest(
                ChapterId: $"chapter-{i:D3}",
                Title: $"第 {i} 章",
                Order: i,
                SourcePath: absolute,
                TargetPath: relative,
                Overwrite: true));
        }

        var sw = Stopwatch.StartNew();
        var tree = await client.GetWorksTreeAsync();
        sw.Stop();

        var perChapterMs = sw.Elapsed.TotalMilliseconds / chapters;
        _output.WriteLine(
            $"{chapters} 章的作品树加载：{sw.Elapsed.TotalMilliseconds:F0}ms"
            + $"（单章摊销 {perChapterMs:F2}ms）");

        // 先验内容：空树是最快的，只测速度会把「返回空树」记成优秀。
        var titles = CollectTitles(tree).ToList();
        Assert.Contains($"第 {chapters} 章", titles);
        Assert.Contains("第 1 章", titles);

        Assert.True(
            perChapterMs < 200,
            $"作品树加载单章摊销 {perChapterMs:F1}ms（阈值 200ms），"
            + $"{chapters} 章共 {sw.Elapsed.TotalMilliseconds:F0}ms。"
            + "百万字项目常有几百章，进入作品页会明显卡顿");
    }

    private static IEnumerable<string> CollectTitles(WorksTreeNode node)
    {
        yield return node.Title;
        foreach (var child in node.Children)
        {
            foreach (var title in CollectTitles(child))
            {
                yield return title;
            }
        }
    }

    /// <summary>
    /// 生成指定字数的中文语料。
    ///
    /// **必须是中文**：性能问题的一大来源是把 UTF-8 当单字节处理，
    /// 或 JSON 转义把中文逐字变成 `\uXXXX`（体积 ×6）。
    /// 纯 ASCII 语料测不出这两类。
    /// 用 StringBuilder 预分配容量：拼字符串会自己造出 O(n²)，
    /// 那样测出来的是测试代码的耗时，不是产品的。
    /// </summary>
    private static string BuildChineseProse(int targetChars)
    {
        const string unit =
            "他沿着旧城河岸往北走，雨丝落在肩上，远处传来收摊的声音。"
            + "巷口的灯忽明忽暗，像是随时会熄；他把外套的领子竖起来，脚步没有停。\n";
        var builder = new StringBuilder(targetChars + unit.Length);
        while (builder.Length < targetChars)
        {
            builder.Append(unit);
        }
        return builder.ToString();
    }
}
