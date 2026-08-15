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
}
