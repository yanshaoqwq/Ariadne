using Ariadne.Desktop.Backend;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U142 回归：测试进程绝不能写用户真实应用状态目录。
///
/// 原始事故不是「某个测试写错了文件」，而是**没有任何东西拦着它写**。
/// 因此判据取「隔离是否真的生效」——即客户端交给 sidecar 的 app-state 根
/// 是否落在临时区内——而不是「测试有没有记得设环境变量」：
/// 后者在缺陷版本下也能靠人工自觉通过，拦不住下一个忘记设的人。
/// </summary>
public sealed class SidecarAppStateIsolationTests
{
    /// <summary>
    /// 核心判据。缺陷版本下必红：
    /// 没有模块初始化器时 <c>SpecialFolder.ApplicationData</c> 解析到
    /// <c>~/.config/Ariadne</c>（本机经符号链接落到 /custdata/.config/Ariadne），
    /// 不在临时隔离根之下，<c>IsIsolated</c> 返回 false。
    /// </summary>
    [Fact]
    public void ClientAppStateRoot_StaysInsideIsolatedTempRoot()
    {
        var effective = SidecarAppStateIsolation.ClientVisibleAppStateRoot();

        Assert.True(
            SidecarAppStateIsolation.IsIsolated(effective),
            $"app-state 根解析到 {effective}，逃出隔离区 {SidecarAppStateIsolation.Root}；"
            + " 起 sidecar 的测试会污染用户的 recent_projects.json（U142）");
    }

    /// <summary>
    /// 隔离根必须与用户真实目录**不同**。
    ///
    /// 单独立一条是因为上一条存在退化通过的可能：若隔离根本身被误设成
    /// 用户真实目录，`IsIsolated` 仍会返回 true 而污染照旧。
    /// </summary>
    [Fact]
    public void IsolatedRoot_IsNotTheRealUserAppStateDirectory()
    {
        var real = SidecarAppStateIsolation.RealUserAppStateRoot();
        if (real is null)
        {
            return; // 无 HOME 的环境（容器）无从比较。
        }

        var effective = SidecarAppStateIsolation.ClientVisibleAppStateRoot();
        Assert.False(
            PathsResolveToSame(effective, real),
            $"测试的 app-state 根就是用户真实目录 {real}——隔离等于没做");

        // 真实目录可能经符号链接出现（~/.config → /custdata/.config），
        // 只比字符串会漏判，因此额外比一次解析后的物理路径。
        Assert.False(
            SidecarAppStateIsolation.IsIsolated(real),
            "用户真实目录被判成了隔离区内，说明隔离根设置有误");
    }

    /// <summary>
    /// 哨兵变量必须传给后端。
    ///
    /// 前两条只覆盖「C# 侧算出的路径」。若某条链路绕过客户端自算
    /// （例如 sidecar 自己回落到 platform_app_state_root），仍会写用户目录——
    /// 那一层由 Rust 侧的 <c>ARIADNE_APP_STATE_REQUIRE_ISOLATION</c> 兜底，
    /// 这里断言它确实被设上了，否则兜底形同虚设。
    /// </summary>
    [Fact]
    public void IsolationSentinel_IsHandedToBackend()
    {
        var sentinel = Environment.GetEnvironmentVariable("ARIADNE_APP_STATE_REQUIRE_ISOLATION");

        Assert.False(
            string.IsNullOrWhiteSpace(sentinel),
            "未设置 ARIADNE_APP_STATE_REQUIRE_ISOLATION：sidecar 回落到默认目录时不会报错");
        Assert.True(
            SidecarAppStateIsolation.IsIsolated(sentinel),
            $"哨兵指向 {sentinel}，本身就不在隔离区内");
    }

    /// <summary>
    /// XDG 变量必须一并改道。
    ///
    /// <c>JsonLineBackendClient</c> 在 <c>ApplyProjectEnvironment</c> 里用自算的
    /// <c>_appStateRoot</c> **无条件覆盖**子进程的 ARIADNE_APP_STATE_ROOT——
    /// 只设该变量而不改 XDG_CONFIG_HOME 会被它盖掉，隔离静默失效。
    /// 这条钉住那个「看起来多余、其实必需」的改动。
    /// </summary>
    [Fact]
    public void XdgRoots_AreRedirectedSoClientComputesIsolatedPath()
    {
        foreach (var name in new[] { "XDG_CONFIG_HOME", "XDG_DATA_HOME" })
        {
            var value = Environment.GetEnvironmentVariable(name);
            Assert.True(
                SidecarAppStateIsolation.IsIsolated(value),
                $"{name}={value} 未指向隔离区；客户端会算出用户真实目录并覆盖子进程变量");
        }
    }

    /// <summary>
    /// 所有起真实 sidecar 的测试都必须先过自检。
    ///
    /// 这是**防回归的那一条**：将来有人新增一个起 sidecar 的测试文件而忘了
    /// 隔离，模块初始化器仍然保护得住，但若有人顺手把初始化器删了，
    /// 这条会指出哪些文件失去了显式自检。
    /// </summary>
    [Fact]
    public void EverySidecarTest_CallsIsolationSelfCheck()
    {
        var testsDir = ResolveTestsSourceDir();
        var offenders = new List<string>();

        foreach (var path in Directory.EnumerateFiles(testsDir, "*.cs", SearchOption.AllDirectories))
        {
            if (path.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
                || path.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            {
                continue;
            }

            var source = File.ReadAllText(path);
            // 只有真正 new 出客户端并连上 sidecar 的文件才需要自检；
            // 仅引用类型名（如断言源码文本）的文件不算。
            if (!source.Contains("new JsonLineBackendClient(sidecar", StringComparison.Ordinal))
            {
                continue;
            }

            if (!source.Contains("RequireIsolatedAppState", StringComparison.Ordinal))
            {
                offenders.Add(Path.GetFileName(path));
            }
        }

        Assert.True(
            offenders.Count == 0,
            "以下测试起了真实 sidecar 却没做 app-state 隔离自检（U142）：\n"
            + string.Join('\n', offenders));
    }

    private static bool PathsResolveToSame(string left, string right)
    {
        try
        {
            return string.Equals(
                Path.GetFullPath(left).TrimEnd(Path.DirectorySeparatorChar),
                Path.GetFullPath(right).TrimEnd(Path.DirectorySeparatorChar),
                StringComparison.Ordinal);
        }
        catch
        {
            return false;
        }
    }

    private static string ResolveTestsSourceDir()
    {
        var walk = new DirectoryInfo(AppContext.BaseDirectory);
        for (var i = 0; i < 10 && walk is not null; i++)
        {
            var candidate = Path.Combine(walk.FullName, "desktop", "Ariadne.Desktop.Tests");
            if (Directory.Exists(candidate))
            {
                return candidate;
            }

            walk = walk.Parent;
        }

        throw new DirectoryNotFoundException("未找到 desktop/Ariadne.Desktop.Tests 源码目录");
    }
}
