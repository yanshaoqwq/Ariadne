using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U208-C 守卫：项目空间 AI 的文案曾教作者用 `@` 引用材料，而 `@` 补全**不存在**。
///
/// <para>
/// 修复走的是「让文案只承诺真实存在的能力」（路 B）—— 删掉那句从句，
/// 而不是去实现 `@` 补全（那是 U206-B 的事，与本条独立）。
/// ⇒ 本守卫钉的是「文案不再承诺尚不存在的能力」，**不是**「`@` 补全已实现」。
/// 等 U206-B 落地后可以把文案加回来，那时应当**同批**删掉本文件第一条用例，
/// 而不是留着它去阻止一句正确的文案。
/// </para>
///
/// <para>
/// ⚠️ **为什么不能只断言 zh 一份**：C 条施工时发现了报告完全没记的一层 ——
/// 同一个键 `ui.works.knowledge_empty` 的 ja 值本来就不含 `@`（讲的是
/// 「没有可显示的知识」），zh/en 两份却在教用户用 `@`。
/// 三份语言包对同一个键**讲的不是同一件事**，而这种分叉
/// `DisplayNameService` 抓不到：它只在**缺键**时回落，值不一致时三份都「正常」。
/// ⇒ 第二条用例守的就是这个，判据是「承诺的能力集合三份一致」。
/// </para>
/// </summary>
public sealed class ProjectAiCopyPromiseTests
{
    /// <summary>
    /// 这四个键是 C 条实际改动的范围：**有生产引用且有真实绑定点**的那些。
    ///
    /// ⚠️ 报告点名了 5 个键，但 `ui.works.knowledge_empty` 全仓生产零引用
    ///（`.cs`/`.axaml`/`.rs` 全扫过，只躺在语言包里）⇒ 它是**死键**，
    /// 按「不改」处置：改一个没人看的键零收益，还会让它看起来像在用。
    /// 所以本清单**刻意不含它**；下面第三条用例负责钉住「它仍然是死的」，
    /// 一旦有人给它接了线，那条会红并提醒把它纳入本清单。
    /// </summary>
    private static readonly string[] BoundProjectAiKeys =
    {
        "ui.works.project_ai.empty",
        "ui.works.project_ai.placeholder",
        "ui.workspace.project_ai.placeholder",
        "ui.empty.workspace.ai.hint",
    };

    private const string DeadKnowledgeKey = "ui.works.knowledge_empty";

    /// <summary>
    /// 三份语言包里，接了线的 AI 文案都不得再教作者用 `@`。
    ///
    /// ⚠️ 判据取「`@` 这个字符不出现」而非「不含某句特定措辞」：
    /// 措辞在三种语言里各不相同（我读不懂其中一份），而 `@` 是三份共有的、
    /// 可机械核验的那个符号。**改不熟悉语言的文案时，断言是肉眼复核的唯一替代品。**
    /// </summary>
    [Fact]
    public void BoundProjectAiCopy_NeverPromisesMentionCompletion()
    {
        foreach (var (language, pack) in LoadAllPacks())
        {
            foreach (var key in BoundProjectAiKeys)
            {
                Assert.True(pack.ContainsKey(key), $"{language} 缺键 {key}");
                var value = pack[key];
                Assert.DoesNotContain("@", value, StringComparison.Ordinal);
            }
        }
    }

    /// <summary>
    /// 同一个键在三份包里承诺的**能力集合**必须一致。
    ///
    /// 这里用「是否提到 `@`」作为能力标记 —— C 条踩到的真实分叉就是这一维：
    /// ja 不提、zh/en 提。判据落在「三份的标记相同」，而不是「都为假」，
    /// 这样 U206-B 落地后把 `@` 加回来时，本条会要求**三份一起加**，
    /// 仍然拦得住「只改中文、另两份忘了」。
    ///
    /// ⚠️ 这一条与上一条**不是**冗余：上一条全为假时本条自动成立，
    /// 但上一条将来会被合法删除（见类注释），本条要留下来继续守分叉。
    ///
    /// ⚠️ **刻意只覆盖接了线的键，不含死键**。死键 <c>ui.works.knowledge_empty</c>
    /// 的分叉至今仍在（zh/en 教用 `@`、ja 不提），而 C 条已定夺「不改它」——
    /// 把它纳进来只会让本用例立刻红在一个**没人看得到的**值上，
    /// 逼着下一个人去改一个零收益的键，或者干脆把整条用例注掉。
    /// 它的前提由第三条用例看守：一旦接了线，那条会红并要求重判。
    /// </summary>
    [Fact]
    public void EveryPackPromisesTheSameCapabilitiesForTheSameKey()
    {
        var packs = LoadAllPacks();

        foreach (var key in BoundProjectAiKeys)
        {
            var promises = new List<(string Language, bool MentionsAt)>();
            foreach (var (language, pack) in packs)
            {
                if (pack.TryGetValue(key, out var value))
                {
                    promises.Add((language, value.Contains('@', StringComparison.Ordinal)));
                }
            }

            if (promises.Count < 2)
            {
                continue;
            }

            var distinct = promises.Select(item => item.MentionsAt).Distinct().Count();
            Assert.True(
                distinct == 1,
                $"键 {key} 在各语言包里承诺的能力不一致："
                + string.Join(
                    " / ",
                    promises.Select(item => $"{item.Language}={(item.MentionsAt ? "教用 @" : "不提 @")}"))
                + "。DisplayNameService 只在**缺键**时回落，值讲的不是同一件事时三份都「正常」，"
                + "唯一发现途径就是本用例（U208-C 施工时实测撞见，报告一字未提）。");
        }
    }

    /// <summary>
    /// 死键仍然是死的 —— 一旦有人给它接线，本条会红。
    ///
    /// ⚠️ 判据方向是**反的**：不是「它必须永远没人用」，而是
    /// 「它被接线时必须有人回来把它纳入 <see cref="BoundProjectAiKeys"/>」。
    /// 上面第一条用例刻意不含它（改一个没人看的键零收益），
    /// 但那个豁免的前提是「真的没人看」—— 前提变了就得重判，
    /// 这一条负责在前提变化的当场告诉下一个人。
    /// </summary>
    [Fact]
    public void DeadKnowledgeKey_StaysUnreferencedOrGetsReclassified()
    {
        var desktopRoot = Path.Combine(ResolveRepoRoot(), "desktop");
        var referenced = Directory
            .EnumerateFiles(desktopRoot, "*.*", SearchOption.AllDirectories)
            .Where(path => path.EndsWith(".cs", StringComparison.Ordinal)
                           || path.EndsWith(".axaml", StringComparison.Ordinal))
            // 排除 bin/obj 里的生成物与测试自身（本文件就写着这个键名）。
            .Where(path => !path.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}",
                               StringComparison.Ordinal)
                           && !path.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}",
                               StringComparison.Ordinal)
                           && !path.Contains("Ariadne.Desktop.Tests", StringComparison.Ordinal))
            .Any(path => File.ReadAllText(path).Contains(DeadKnowledgeKey, StringComparison.Ordinal));

        Assert.False(
            referenced,
            $"{DeadKnowledgeKey} 有了生产引用 —— 它不再是死键。"
            + "请把它加进 BoundProjectAiKeys 并按 U208-C 的口径检查它的三份文案"
            + "（zh/en 那两份至今仍在教作者用 @，而 @ 补全尚未实现）。");
    }

    private static List<(string Language, IReadOnlyDictionary<string, string> Pack)> LoadAllPacks()
    {
        var root = Path.Combine(ResolveRepoRoot(), "core", "resources");
        var result = new List<(string, IReadOnlyDictionary<string, string>)>();
        foreach (var (suffix, language) in new[] { ("", "zh"), (".en", "en"), (".ja", "ja") })
        {
            var path = Path.Combine(root, $"display_name{suffix}.json");
            Assert.True(File.Exists(path), $"语言包不存在：{path}");
            var parsed = JsonSerializer.Deserialize<Dictionary<string, string>>(
                File.ReadAllText(path));
            Assert.NotNull(parsed);
            result.Add((language, parsed!));
        }

        return result;
    }

    private static string ResolveRepoRoot()
    {
        var dir = AppContext.BaseDirectory;
        for (var i = 0; i < 8 && dir is not null; i++)
        {
            if (File.Exists(Path.Combine(dir, "core", "resources", "display_name.json")))
            {
                return dir;
            }

            dir = Path.GetDirectoryName(dir);
        }

        throw new DirectoryNotFoundException($"从 {AppContext.BaseDirectory} 向上找不到仓库根");
    }
}
