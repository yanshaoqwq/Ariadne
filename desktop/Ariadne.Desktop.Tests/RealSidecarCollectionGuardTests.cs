using System.Text.RegularExpressions;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U181 守卫：起真实 sidecar 的测试类必须挂 <c>[Collection("RealSidecar")]</c>。
///
/// **为什么非要一条守卫**：U142 的类注释里已经把结论写明白了——
/// 「逐个测试注入要求『以后每个写测试的人都记得注入』，这正是本条缺陷的成因，
/// 把同样的要求再写一遍不会有不同结果」。
/// 而 U181 的修法恰恰是**逐个类加特性**，等于又立了一条靠自觉维持的约定。
/// 所以必须配一条会自己转红的守卫：新增跨进程测试类忘了加特性时，
/// 由这条用例当场指出来，而不是等某次 CI 随机红、再由人去猜。
///
/// **判据取源码文本扫描而非反射**：`[Collection]` 反射能读到，
/// 但「这个类会不会起 sidecar」取决于方法体里有没有
/// <c>new JsonLineBackendClient(</c>——方法体不在反射的可见范围内。
///
/// 本类**刻意不进** RealSidecar 集合：它只读源码、不起进程，
/// 没有理由排在 16 个慢类后面等着。
/// </summary>
public sealed class RealSidecarCollectionGuardTests
{
    /// <summary>起 sidecar 的标志：构造真实 IPC 客户端。</summary>
    private const string SidecarMarker = "new JsonLineBackendClient(";

    /// <summary>本集合的特性文本，缺它即为待修。</summary>
    private const string RequiredAttribute = "[Collection(\"RealSidecar\")]";

    /// <summary>本文件名，用于把自己排除在扫描之外（理由见扫描循环内注释）。</summary>
    private const string ThisFileName = "RealSidecarCollectionGuardTests.cs";

    [Fact]
    public void EveryTestClassThatSpawnsARealSidecar_JoinsTheSerializedCollection()
    {
        var offenders = new List<string>();
        var scanned = 0;

        foreach (var file in EnumerateTestSources())
        {
            // 本文件把两个标志串都作为字面量写在源码里（判据本身与失败信息），
            // 扫自己会「两个都命中 ⇒ 判为合规」，等于给自己开了张免检单。
            // 必须显式排除：否则哪天有人把判据抄进别的文件，那个文件也会自动免检。
            if (Path.GetFileName(file) == ThisFileName)
            {
                continue;
            }

            var text = File.ReadAllText(file);
            if (!text.Contains(SidecarMarker, StringComparison.Ordinal))
            {
                continue;
            }

            scanned++;
            if (!text.Contains(RequiredAttribute, StringComparison.Ordinal))
            {
                offenders.Add(Path.GetFileName(file));
            }
        }

        // 自检：扫不到任何跨进程测试说明路径解析错了，而不是「大家都合规」。
        // 少了这一条，路径一变本用例就静默全绿——那是最坏的形态：
        // 守卫没了，但看板上还是绿的。
        Assert.True(
            scanned >= 10,
            $"只扫到 {scanned} 个起 sidecar 的测试文件，远少于预期（16 个左右）。"
            + " 大概率是扫描路径解析错了，本守卫已失效——先修路径，别改这个阈值。");

        Assert.True(
            offenders.Count == 0,
            "以下测试类会起真实 sidecar 子进程，但没有加入串行集合：\n"
            + string.Join("\n", offenders.Select(name => "  " + name))
            + "\n\n为什么必须串行：这些测试共享**同一份** app-state"
            + "（`secrets.json` / `provider_catalog.json` / `recent_projects.json`），"
            + "而 xUnit 默认跨测试类并行。并发读写同一份凭据库会撞出\n"
            + "  `external service error from local_secret_store: "
            + "local secret encryption failed: aead::Error`\n"
            + "——报错说的是「加密失败」，出现位置却在**读**路径，"
            + "极易被误判成 `core/src/config/secrets.rs` 的产品缺陷（U181 就这么误诊过一次）。\n"
            + "而且它是**随机红**：随机红比稳定红危险，因为它训练人忽略红灯。\n\n"
            + $"怎么修：在类声明上方加一行 `{RequiredAttribute}`。\n"
            + "⚠️ xUnit 一个类只能属一个 collection——若该类已属别的集合，"
            + "不要直接替换，先想清楚两个集合的互斥语义能不能合并。");
    }

    /// <summary>
    /// U181 第二条守卫：**没有任何测试类可以自己设凭据主密钥**。
    ///
    /// 为什么单独立一条：串行化只消除并发交错，而 aead 失败的另一半成因是
    /// 「同一份 `secrets.json` 被两把不同密钥先后读写」——那一半靠
    /// <see cref="SidecarAppStateIsolation.UseSharedSecretMasterKey"/> 统一密钥收口。
    /// 若有人日后又在自己的测试类里写一句
    /// <c>SetEnvironmentVariable("ARIADNE_SECRET_MASTER_KEY", "我的专属key")</c>，
    /// 串行照旧、特性照旧，但 aead 会**立刻回来**（这条已实测：
    /// 只把一个类改回专属密钥，那个类 4 条全红）。
    /// 所以判据取「除隔离工具本身外，源码里不得出现这个变量名」。
    /// </summary>
    [Fact]
    public void NoTestClassSetsItsOwnSecretMasterKey()
    {
        const string envName = "ARIADNE_SECRET_MASTER_KEY";
        var offenders = new List<string>();

        foreach (var file in EnumerateTestSources())
        {
            var name = Path.GetFileName(file);
            // 允许两个文件提到它：定义共享密钥的工具本身，以及本守卫（判据字面量）。
            if (name == ThisFileName || name == "SidecarAppStateIsolation.cs")
            {
                continue;
            }

            var text = File.ReadAllText(file);

            // 先剥掉注释再判断，而**不是**逐行匹配「同一行里既有变量名又有
            // SetEnvironmentVariable」。首版就是逐行写的，结果漏判：
            // 实际代码把调用拆成两行——
            //     Environment.SetEnvironmentVariable(
            //         "ARIADNE_SECRET_MASTER_KEY", "user-action-master-key");
            // 两个标志分处两行，逐行匹配永远都不会同时看到 ⇒ 变异测试全绿，
            // 而那正是这条守卫唯一要拦的写法。判据必须跨行看。
            var code = StripComments(text);
            if (!code.Contains(envName, StringComparison.Ordinal))
            {
                continue;
            }

            // 变量名出现在非注释代码里就算——读它（GetEnvironmentVariable）没有理由，
            // 各类要的是「设一把自己的」；真要读也该走共享常量。
            offenders.Add(name);
        }

        Assert.True(
            offenders.Count == 0,
            "以下测试类自己设了凭据主密钥：\n"
            + string.Join("\n", offenders.Select(name => "  " + name))
            + $"\n\n{envName} 是**进程级**变量，而 Provider 凭据存在**应用级**"
            + " `secrets.json`（`commands.rs:5274`），全测试进程只有一份、"
            + "且在进程生命周期里累积。各设一把的后果：\n"
            + "  类 A 用 K1 加密写下文件 → 类 B 带 K2 调 set_secret"
            + "（读-改-写，`secrets.rs:598`）→ K2 解不开 K1 的密文 ⇒ aead::Error。\n"
            + "串行化拦不住这一半——它只把随机红变成稳定红。\n\n"
            + "怎么修：删掉那一行，改调"
            + " `SidecarAppStateIsolation.UseSharedSecretMasterKey()`。");
    }

    /// <summary>
    /// 去掉 <c>//</c> 与 <c>///</c> 注释行，只留可执行代码。
    ///
    /// 几个测试类的注释里合法地引用了那个变量名来解释「为什么必须先解锁凭据库」，
    /// 不剥注释会把说明文字判成违规。只处理整行注释：本工程里这个变量
    /// 不会出现在块注释或行尾注释里，多写的分支反而更容易出错。
    /// </summary>
    private static string StripComments(string text) => string.Join(
        '\n',
        text.Split('\n').Where(line =>
            !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));

    /// <summary>
    /// 测试工程自己的源码目录。
    ///
    /// 沿 <c>Ariadne.slnx</c> 上溯定位解决方案根，与 `ThemeStyleUsageTests` 同一套写法：
    /// 不能用相对路径拼，测试的工作目录是 `bin/Debug/net10.0`。
    /// </summary>
    private static IEnumerable<string> EnumerateTestSources()
    {
        var root = Path.Combine(ResolveSolutionDir(), "Ariadne.Desktop.Tests");
        foreach (var file in Directory.EnumerateFiles(root, "*.cs", SearchOption.AllDirectories))
        {
            // 跳过生成产物：obj/ 下有编译器生成的 .cs，扫它们没有意义还会拖慢。
            var relative = Path.GetRelativePath(root, file);
            if (relative.StartsWith("obj" + Path.DirectorySeparatorChar, StringComparison.Ordinal)
                || relative.StartsWith("bin" + Path.DirectorySeparatorChar, StringComparison.Ordinal))
            {
                continue;
            }

            yield return file;
        }
    }

    private static string ResolveSolutionDir()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }

        Assert.NotNull(dir);
        return dir!;
    }
}
