using System.Text.Json;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U150：前端 `{{ref:...}}` 词法层，与 Rust 侧共读同一份语料。
///
/// # 这条交叉验证是「C# 再实现一遍」这个决定的唯一防线
///
/// U150 需要在**每次按键后**重扫占位符（高亮 + 折叠随输入实时更新）。
/// 走 IPC 意味着每敲一个字符跨一次进程边界：JSON 序列化 + 管道往返 +
/// 反序列化，还得处理「上一次请求未回、用户又敲了三下」的乱序。
/// 为一个纯词法判断付这个代价不成比例。
///
/// 代价是**两份语法定义会漂移**——AGENTS.md 明确写着「任何手抄的字段镜像
/// 都是这类缺陷的温床，优先收敛成单一来源」。
/// 收口办法：`core/tests/fixtures/content_reference_cases.json` 是**唯一**
/// 期望值来源，本用例与 Rust 侧 `shared_fixture_matches_the_rust_reference_lexer`
/// 读同一个文件、断言同一批期望值。任一侧改了语法而没同步，红的是那一侧。
///
/// ⚠️ **只在一侧跑 fixture 不算交叉验证**——那只是「C# 实现符合我写的语料」，
/// 而语料本身可能就抄错了。两侧都跑才构成「两个实现互为对照」。
/// </summary>
public sealed class ContentReferenceSyntaxTests
{
    /// <summary>
    /// 共读语料：C# 侧解析结果必须与语料一致。
    ///
    /// ⚠️ **刻意不比较偏移数值**：Rust 的 `TextRange` 是 UTF-8 byte 半开区间，
    /// C# string 索引是 UTF-16 code unit。正文是中文，同一个占位符在两种口径下
    /// 数值必然不同（一个汉字 3 byte vs 1 code unit）。
    /// 两侧共同断言的不变式是「**按各自偏移切出来的子串 == raw**」——
    /// 那才是偏移量真正要保证的性质，且在两种口径下都成立。
    /// </summary>
    [Fact]
    public void SharedFixtureMatchesTheFrontendLexer()
    {
        var fixture = LoadFixture();
        var cases = fixture.GetProperty("cases").EnumerateArray().ToList();
        Assert.NotEmpty(cases);

        foreach (var testCase in cases)
        {
            var name = testCase.GetProperty("name").GetString() ?? "(未命名)";
            var text = testCase.GetProperty("text").GetString() ?? string.Empty;
            var expected = testCase.GetProperty("expected").EnumerateArray().ToList();

            var occurrences = ContentReferenceSyntax.Parse(text);
            Assert.Equal(expected.Count, occurrences.Count);

            for (var index = 0; index < expected.Count; index++)
            {
                var want = expected[index];
                var actual = occurrences[index];
                var label = $"{name} 第 {index + 1} 条";

                // 不变式：按偏移切出来的子串 == raw。两种偏移口径下都成立。
                Assert.Equal(actual.Raw, text[actual.Start..actual.End]);
                Assert.Equal(want.GetProperty("raw").GetString(), actual.Raw);

                if (want.TryGetProperty("expect_error", out var expectError)
                    && expectError.GetBoolean())
                {
                    // 只断言「都判为非法」这个结论，**不比错误文案**：
                    // 两侧措辞可以不同，重要的是判定一致。
                    Assert.False(actual.IsValid, $"{label}: 语料标了 expect_error，C# 侧却解析成功");
                    continue;
                }

                Assert.True(actual.IsValid, $"{label}: 语料期望成功，实际失败：{actual.ParseError}");
                Assert.Equal(want.GetProperty("document_id").GetString(), actual.DocumentId);

                var wantVersion = want.TryGetProperty("version", out var version)
                    ? version.GetString()
                    : null;
                Assert.Equal(wantVersion, actual.Version);

                var wantLocator = want.GetProperty("locator").GetString();
                var actualLocator = actual.Locator switch
                {
                    ContentReferenceSyntax.LocatorKind.Lines => "lines",
                    ContentReferenceSyntax.LocatorKind.Bytes => "bytes",
                    _ => "whole",
                };
                Assert.Equal(wantLocator, actualLocator);

                if (wantLocator != "whole")
                {
                    Assert.Equal(want.GetProperty("range_start").GetInt64(), actual.RangeStart);
                    Assert.Equal(want.GetProperty("range_end").GetInt64(), actual.RangeEnd);
                }
            }
        }
    }

    /// <summary>
    /// 前置守卫：语料里必须**同时**有合法与非法用例。
    ///
    /// 没有这一条，上面那条交叉验证可能在一份「全是合法用例」的语料上全绿，
    /// 而两侧对**非法输入**的判定完全可以不一致——那正是最容易漂移的部分
    /// （合法语法两边都照着文档写，边界情形各凭理解）。
    /// </summary>
    [Fact]
    public void FixtureCoversBothValidAndInvalidSyntax()
    {
        var cases = LoadFixture().GetProperty("cases").EnumerateArray().ToList();
        var entries = cases
            .SelectMany(item => item.GetProperty("expected").EnumerateArray())
            .ToList();

        var invalid = entries.Count(item =>
            item.TryGetProperty("expect_error", out var flag) && flag.GetBoolean());
        var valid = entries.Count - invalid;

        Assert.True(valid > 0, "语料里没有合法用例");
        Assert.True(
            invalid > 0,
            "语料里没有**非法**用例。两侧对非法输入的判定最容易漂移——"
            + "合法语法双方都照文档写，边界情形各凭理解。");
    }

    /// <summary>
    /// 未闭合的 `{{ref:` 不记成占位符——与 Rust 侧同一取舍。
    ///
    /// 它没有确定的结束位置，无法就地替换、也无法给它划折叠区。
    /// 这是**用户打字打到一半**的常态输入（刚敲完 `{{ref:` 还没敲 `}}`），
    /// 把它记成一条「非法引用」会让编辑器在打字途中一直闪红。
    /// </summary>
    [Theory]
    [InlineData("前面 {{ref:chapter-01.md")]
    [InlineData("{{ref:")]
    [InlineData("{{ref:a.md#L1-L2 后面忘了闭合")]
    public void UnclosedPlaceholderIsNotAnOccurrence(string text)
    {
        Assert.Empty(ContentReferenceSyntax.Parse(text));
    }

    private static JsonElement LoadFixture()
    {
        var path = ResolveFixturePath();
        using var document = JsonDocument.Parse(File.ReadAllText(path));
        return document.RootElement.Clone();
    }

    private static string ResolveFixturePath()
    {
        var dir = AppContext.BaseDirectory;
        while (dir is not null && !File.Exists(Path.Combine(dir, "Ariadne.slnx")))
        {
            dir = Path.GetDirectoryName(dir);
        }
        Assert.NotNull(dir);
        // desktop/ 与 core/ 是兄弟目录，所以要再上一级。
        var repoRoot = Path.GetDirectoryName(dir!);
        Assert.NotNull(repoRoot);
        var path = Path.Combine(repoRoot!, "core", "tests", "fixtures", "content_reference_cases.json");
        Assert.True(File.Exists(path), $"找不到共读语料：{path}");
        return path;
    }
}
