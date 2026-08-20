using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U208-A 守卫：后端每个 <c>CommandErrorCode</c> 都要在前端映射表里有一行。
///
/// <para>
/// **为什么需要这条守卫**：后端那侧新增码时，`rest.rs` 的穷尽 match 会让编译器
/// 当场拦下（实测：加 `NotConfigured` 时 E0004 报到了状态码表）。
/// 但前端 <c>UserFacingError</c> 的码→文案表**以 `_ =>` 兜底**，
/// 漏一个不报错、不回落，只是静默显示「未知错误」——
/// 比归错变体更糟：归错变体至少给了一句具体（虽然错）的引导。
/// </para>
///
/// <para>
/// ⚠️ **判据刻意是逐一对应，不是存在性**。「表里有 ≥N 个条目」挪一行注释就绿，
/// 而真正要防的是「新增了第 N+1 个码但没加条目」。同一教训见 U209：
/// 那条既有守卫断言「存在一个能重试的分区」，而它选中的样本恰好是
/// 10 个里唯一健康的那个，覆盖率 2/10 却天天报绿。
/// </para>
/// </summary>
public sealed class ErrorCodeCopyCoverageTests
{
    /// <summary>
    /// 后端码表的权威来源是 Rust 那侧的 <c>CommandErrorCode::as_str</c>。
    /// 这里读源码而不是硬编码一份清单：硬编码的副本会和后端一起漂移，
    /// 而漂移之后它仍然全绿（两份错得一样）。
    /// </summary>
    private static List<string> BackendCodes()
    {
        var path = ResolveRepoFile(Path.Combine("core", "src", "command_error.rs"));
        var source = File.ReadAllText(path);

        // 只取 `as_str` 那个 match 体内的字面量，避免把 `message_key` 的
        // 格式串或注释里的示例也算进去。
        var start = source.IndexOf("pub const fn as_str(self)", StringComparison.Ordinal);
        Assert.True(start > 0, $"没找到 as_str 定义，command_error.rs 结构变了：{path}");
        var end = source.IndexOf("\n    }", start, StringComparison.Ordinal);
        Assert.True(end > start, "没找到 as_str 的结束位置");

        var body = source[start..end];
        var codes = Regex.Matches(body, @"=>\s*""([a-z_]+)""")
            .Select(m => m.Groups[1].Value)
            .Distinct()
            .ToList();

        // 自检下限：正则失配时循环空跑也会「通过」，这一条把「扫了 0 个」变成红。
        Assert.True(
            codes.Count >= 18,
            $"只从 as_str 解析出 {codes.Count} 个码，正则或文件结构失配了——" +
            "此时下面的覆盖检查会因为没东西可查而假绿");

        return codes;
    }

    [Fact]
    public void EveryBackendErrorCode_HasItsOwnCopyKey()
    {
        var names = DisplayNameService.LoadDefault();
        var missing = new List<string>();

        foreach (var code in BackendCodes())
        {
            // 走真实的映射路径：构造一个带该 code 的失败，看它解析出的文案
            // 是否真的是该码专属的那句，而不是被 `_ =>` 兜到「未知错误」。
            // ⚠️ `MessageKey` 必须留 null —— 非空时 `PrimaryText` 会走「直接用该键」
            // 那条早返回，根本到不了要测的那张 switch 表。
            var failure = new UserFailure(code, null);
            var resolved = failure.PrimaryText(names);
            var expected = names.Text($"ui.error.{code}");

            if (!string.Equals(resolved, expected, StringComparison.Ordinal))
            {
                missing.Add(code);
            }
        }

        Assert.True(
            missing.Count == 0,
            "这些后端错误码在前端映射表里没有条目，会静默显示「未知错误」：" +
            string.Join(", ", missing) +
            "。后端加 CommandErrorCode 时必须同批在 UserFacingError 加一行 —— " +
            "那张表有 `_ =>` 兜底，漏了不会报错。");
    }

    [Fact]
    public void EveryBackendErrorCode_HasNonPlaceholderCopyInTheDefaultPack()
    {
        var names = DisplayNameService.LoadDefault();
        var placeholders = new List<string>();

        foreach (var code in BackendCodes())
        {
            var key = $"ui.error.{code}";
            var text = names.Text(key);

            // DisplayNameService 缺键时返回 `[key]`，这是它刻意的自查形态。
            if (text.StartsWith('[') && text.EndsWith(']'))
            {
                placeholders.Add(key);
            }
        }

        Assert.True(
            placeholders.Count == 0,
            $"这些错误文案键在主语言包里缺失，界面会显示方括号原文：{string.Join(", ", placeholders)}");
    }

    private static string ResolveRepoFile(string relative)
    {
        var dir = AppContext.BaseDirectory;
        for (var i = 0; i < 8 && dir is not null; i++)
        {
            var candidate = Path.Combine(dir, relative);
            if (File.Exists(candidate))
            {
                return candidate;
            }

            dir = Path.GetDirectoryName(dir);
        }

        throw new FileNotFoundException($"从 {AppContext.BaseDirectory} 向上找不到 {relative}");
    }
}
