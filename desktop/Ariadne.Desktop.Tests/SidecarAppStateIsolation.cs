using System.Runtime.CompilerServices;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U142 防线：把**整个测试进程**的应用状态目录钉死在一次性临时目录里。
///
/// 事故经过：桌面契约测试起真实 sidecar 时没有隔离 app-state，
/// `recent_projects.json` 因此写进用户真实目录；而该文件只保留 20 条
/// （`core/src/frontend/project.rs` 的 `entries.truncate(20)`），
/// 测试残留把用户自己建过的项目全部挤了出去——用户侧表现为
/// 「最近项目全部打不开」。
///
/// **为什么放在 `[ModuleInitializer]` 而不是每个测试的 setup**：
/// 环境变量是进程级的，而污染只需要**一个**漏网的测试就会发生。
/// 逐个测试注入要求「以后每个写测试的人都记得注入」——这正是本条缺陷
/// 的成因，把同样的要求再写一遍不会有不同结果。模块初始化器在程序集加载时
/// 由运行时调用，早于任何测试、也早于任何 <c>JsonLineBackendClient</c> 构造，
/// 新增测试**无需知道它存在**就已经被保护。
///
/// **为什么同时改 XDG 变量**：`ARIADNE_APP_STATE_ROOT` 只堵住后端自己的解析。
/// 但 <c>JsonLineBackendClient</c> 在构造函数里用
/// <c>SpecialFolder.ApplicationData</c> 算出 <c>_appStateRoot</c>，再在
/// <c>ApplyProjectEnvironment</c> 里**无条件覆盖**子进程的
/// <c>ARIADNE_APP_STATE_ROOT</c>——即父进程设了也会被它盖掉。
/// 所以必须连 .NET 解析 ApplicationData 的源头（<c>XDG_CONFIG_HOME</c>）一起换掉，
/// 才能让客户端算出的路径本身就落在隔离区内。
/// <c>XDG_DATA_HOME</c> 同理：Rust 侧 `platform_app_state_root()` 在 Linux 上
/// 回落到它，不换掉的话「忘记设 ARIADNE_APP_STATE_ROOT」仍会写到真实目录。
/// </summary>
internal static class SidecarAppStateIsolation
{
    /// <summary>本进程的隔离根；所有应用状态都必须落在它之下。</summary>
    internal static string Root { get; private set; } = string.Empty;

    [ModuleInitializer]
    internal static void Install()
    {
        if (!string.IsNullOrEmpty(Root))
        {
            return;
        }

        var root = Directory.CreateTempSubdirectory("ariadne-test-appstate-").FullName;
        Root = root;

        // 顺序无关，但三者缺一不可，理由见类型注释。
        Environment.SetEnvironmentVariable("XDG_CONFIG_HOME", root);
        Environment.SetEnvironmentVariable("XDG_DATA_HOME", root);
        Environment.SetEnvironmentVariable("ARIADNE_APP_STATE_ROOT", Path.Combine(root, "Ariadne"));

        // 交给后端的哨兵：sidecar 解析出的 app-state 根若不在隔离区内就 panic。
        // 这条是**最后一道**防线——上面三个变量都是「说服客户端算对路径」，
        // 只要有一条链路绕过它们，哨兵会让测试当场炸掉而不是静默写用户目录。
        // 传绝对路径而非布尔值：真实目录可能经符号链接出现
        // （本机 ~/.config → /custdata/.config），按 HOME 前缀猜会漏判。
        Environment.SetEnvironmentVariable("ARIADNE_APP_STATE_REQUIRE_ISOLATION", root);

        AppDomain.CurrentDomain.ProcessExit += (_, _) =>
        {
            try
            {
                Directory.Delete(root, recursive: true);
            }
            catch
            {
                // 清理失败不影响结论：临时目录由系统回收。
            }
        };
    }

    /// <summary>
    /// U181：**整个测试进程共用同一把凭据主密钥**。
    ///
    /// 事故经过：7 个起真实 sidecar 的测试类各在 `ResolveSidecar()` 里设了一把
    /// **不同**的 `ARIADNE_SECRET_MASTER_KEY`（`first-run-master-key`、
    /// `canvas-authoring-master-key` …）。但 Provider 凭据存在**应用级**
    /// `secrets.json`（`commands.rs:5274`），全进程只有一份，
    /// 且在整个测试进程生命周期里累积。于是：
    /// 类 A 用 K1 加密写下文件 → 类 B 带 K2 调 `set_secret` →
    /// `set_secret` 是读-改-写（`secrets.rs:598` 先 `read_values()`）→
    /// K2 去解 K1 的密文 ⇒ `local secret encryption failed: aead::Error`。
    ///
    /// **为什么单靠 `[Collection("RealSidecar")]` 串行不够**（实测，不是推理）：
    /// 加满 16 个特性后跑 SettingsToRun + FrontendProduction + CanvasAuthoring 三类，
    /// 仍有 **7 条稳定失败**——第一个类跑完并留下 K1 的密文，
    /// 后两个类整类全红。串行只把「随机红」变成「稳定红」，
    /// 因为成因是**磁盘残留 + 密钥不一致**，与并发交错无关。
    /// 两条防线各管一半：集合管并发写坏文件，本方法管跨类解不开。
    ///
    /// **为什么统一成一把而不是每类清一次文件**：清文件要求「以后每个写跨进程测试的人
    /// 都记得清」——这正是 U142 类注释里已经否掉的思路。
    /// 统一密钥不需要任何人记得：值由这里给，各类照调即可。
    /// 也不能各自设值后再改回来（`finally` 恢复）：sidecar 是在 spawn 那一刻
    /// 读环境变量的，子进程存活期跨越恢复点，恢复只会让谁拿到哪把更难预测。
    /// </summary>
    internal const string SharedSecretMasterKey = "ariadne-tests-shared-master-key";

    /// <summary>
    /// 起 sidecar 前注入共享主密钥。
    ///
    /// 用主密码而非「允许明文」：走的是真实加密路径，
    /// 与用户配好主密码后的生产形态一致（原各类注释里的理由，保留）。
    /// </summary>
    internal static void UseSharedSecretMasterKey()
    {
        Environment.SetEnvironmentVariable(
            "ARIADNE_SECRET_MASTER_KEY", SharedSecretMasterKey);
    }

    /// <summary>路径是否落在本进程的隔离根之下。</summary>
    internal static bool IsIsolated(string? path)
    {
        if (string.IsNullOrWhiteSpace(path) || string.IsNullOrEmpty(Root))
        {
            return false;
        }

        try
        {
            // 用规范化后的绝对路径比较：隔离根来自 CreateTempSubdirectory，
            // 而被测路径可能带 ".." 或符号链接，直接比字符串会误判。
            var full = Path.GetFullPath(path);
            var root = Path.GetFullPath(Root).TrimEnd(Path.DirectorySeparatorChar);
            return full == root
                || full.StartsWith(root + Path.DirectorySeparatorChar, StringComparison.Ordinal);
        }
        catch
        {
            return false;
        }
    }

    /// <summary>
    /// 客户端真正会交给 sidecar 子进程的 app-state 根。
    ///
    /// 必须复算 <c>JsonLineBackendClient</c> 构造函数里的那个表达式，而不是读
    /// 我们自己设的 <c>ARIADNE_APP_STATE_ROOT</c>：客户端在
    /// <c>ApplyProjectEnvironment</c> 里用它**无条件覆盖**子进程的同名变量，
    /// 父进程设了什么并不算数。这里算的才是子进程实际收到的值。
    /// </summary>
    internal static string ClientVisibleAppStateRoot() => Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
        "Ariadne");

    /// <summary>
    /// 起 sidecar 前的自检：客户端算出的 app-state 根必须在隔离区内。
    ///
    /// 放在每个起 sidecar 的测试入口，是为了让「这条测试受 U142 保护」这件事
    /// 在文件里可见、可 grep；真正的保护由模块初始化器提供。
    /// </summary>
    internal static void RequireIsolatedAppState()
    {
        var effective = ClientVisibleAppStateRoot();
        if (!IsIsolated(effective))
        {
            throw new InvalidOperationException(
                $"测试进程的 app-state 根解析到 {effective}，不在隔离区 {Root} 内。"
                + " 起真实 sidecar 会把 recent_projects.json 写进用户目录并挤掉用户自己的项目（U142）。");
        }
    }

    /// <summary>
    /// 用户真实应用状态目录（不受本进程环境变量影响的那个）。
    /// 断言「没被测试写到」时用它作参照。
    /// </summary>
    internal static string? RealUserAppStateRoot()
    {
        // 不能用 GetFolderPath：它已经被上面的 XDG_CONFIG_HOME 改道了。
        // 这里要的是「如果没有隔离，会写到哪」，所以直接按 HOME 还原。
        var home = Environment.GetEnvironmentVariable("HOME");
        return string.IsNullOrWhiteSpace(home)
            ? null
            : Path.Combine(home, ".config", "Ariadne");
    }

    /// <summary>
    /// 找不到 sidecar 时该失败还是该跳过 —— **必须由环境显式表态**。
    ///
    /// U156 的教训：跨进程测试普遍写成
    /// <code>var sidecar = ResolveSidecar(); if (sidecar is null) return;</code>
    /// 理由是对的（纯前端环境不该误报失败），**但手段是错的**：
    /// **xUnit 把 `return` 记成「通过」** ⇒ 没有覆盖伪装成有覆盖。
    /// U156 那个 P0（点运行什么都不会发生）本来有 12 条跨进程测试能拦住它，
    /// 但 CI 里不编 Rust 后端时那些测试**全部静默跳过、全部记成绿**，
    /// 于是「测试全绿」与「主功能全废」同时成立了一整周。
    ///
    /// 改成：默认**失败**，只有显式设了 <c>ARIADNE_TESTS_ALLOW_MISSING_SIDECAR=1</c>
    /// 才跳过。这样「我知道这里没后端、我接受没有这层覆盖」变成一个
    /// 要有人主动写下来的决定，而不是一个谁都不会注意到的默认值。
    ///
    /// 返回 true 表示调用方应当跳过本条用例。
    /// </summary>
    internal static bool AllowSkipWhenSidecarMissing(string testHint)
    {
        var opted = Environment.GetEnvironmentVariable("ARIADNE_TESTS_ALLOW_MISSING_SIDECAR");
        if (string.Equals(opted, "1", StringComparison.Ordinal)
            || string.Equals(opted, "true", StringComparison.OrdinalIgnoreCase))
        {
            return true;
        }

        throw new InvalidOperationException(
            $"找不到 sidecar 可执行文件，`{testHint}` 无法验证跨进程行为。\n"
            + "先跑 `cargo build --bins`（产物是 target/debug/ariadne-ipc），"
            + "或用 ARIADNE_BACKEND_IPC 指向它。\n"
            + "⚠️ 这里刻意**失败而不是静默跳过**：xUnit 会把 `return` 记成通过，"
            + "那样「没有覆盖」会伪装成「有覆盖」——U156 那个 P0"
            + "（点运行什么都不会发生）就是这么在 12 条跨进程测试全绿的情况下溜过去的。\n"
            + "确实要在无后端环境里跳过，请显式设 ARIADNE_TESTS_ALLOW_MISSING_SIDECAR=1。");
    }
}
