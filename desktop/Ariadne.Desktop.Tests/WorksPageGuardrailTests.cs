using System;
using System.IO;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U208-B / U208-F 守卫：两处「界面允许作者做一件注定失败或他没要求的事」。
///
/// <para>
/// **B（P1）**：未打开项目时「新建章节」可点、可填、可提交，走到后端才被拒，
/// 而拒绝话术是「输入内容不符合要求，请检查后重试」——
/// 把作者引向「检查自己刚填的字」，而他填的完全正确，缺的是一个项目。
/// 这是 U208-A 那类「错误归因把人指向反方向」的前端版本。
/// </para>
///
/// <para>
/// **F（P2）**：AI 发送失败后自动切到「修改」tab，作者没点过它。
/// 根因是取选区**之前**就无条件 `IsEditMode = true`，把「可能需要编辑模式」
/// 执行成了「一定切过去」。纯问答（阅读态问「这一章节奏怎么样」）根本不碰正文。
/// </para>
/// </summary>
public sealed class WorksPageGuardrailTests
{
    /// <summary>
    /// F 的主判据：编辑模式切换必须在 `TryResolve` **成功分支之内**。
    ///
    /// ⚠️ 刻意不断言「切 tab 的代码被删了」——那种判据在
    /// 「删掉了但选区改写因此写不回正文」时也绿，等于把一个缺陷换成另一个。
    /// 这里要的是**位置**：切换仍在（选区改写需要它），但只在真要改正文时发生。
    /// 这与 U196-C 的「修复本体是位置不是文案」同形。
    /// </summary>
    [Fact]
    public void EditModeSwitch_HappensOnlyInsideTheSelectionRewriteBranch()
    {
        var source = ReadWorksViewModel();

        var tryResolve = source.IndexOf(
            "WorksEditorSelectionEdit.TryResolve(",
            StringComparison.Ordinal);
        Assert.True(tryResolve > 0, "找不到 TryResolve 调用，函数结构变了");

        var sendCore = source.IndexOf(
            "internal async Task SendProjectAiCoreAsync(",
            StringComparison.Ordinal);
        Assert.True(sendCore > 0 && sendCore < tryResolve, "找不到 SendProjectAiCoreAsync");

        // 函数开头到 TryResolve 之间不允许出现编辑模式切换。
        // ⚠️ **必须先剥注释**：这段代码的注释里成段引用了原写法
        //（「原写法是『取选区之前先 IsEditMode = true』」），
        // 不剥的话守卫会把解释缺陷的注释当成缺陷本身。
        // 我第一版就是这么红的 —— 而这恰好是本仓已记的教训
        //（守卫先剥注释再匹配），值得留在这里提醒下一个人。
        var preamble = StripLineComments(source[sendCore..tryResolve]);
        Assert.DoesNotContain(
            "IsEditMode = true",
            preamble,
            StringComparison.Ordinal);

        // 但它必须仍然存在于后面（选区改写要写回正文，编辑器必须在场）。
        var afterResolve = StripLineComments(source[tryResolve..]);
        Assert.Contains("IsEditMode = true", afterResolve, StringComparison.Ordinal);
    }

    /// <summary>剥掉 `//` 行注释，只留真实代码。</summary>
    private static string StripLineComments(string source)
    {
        var lines = source.Split('\n');
        for (var i = 0; i < lines.Length; i++)
        {
            var trimmed = lines[i].TrimStart();
            if (trimmed.StartsWith("//", StringComparison.Ordinal)
                || trimmed.StartsWith("///", StringComparison.Ordinal)
                || trimmed.StartsWith("*", StringComparison.Ordinal))
            {
                lines[i] = string.Empty;
            }
        }

        return string.Join('\n', lines);
    }

    /// <summary>
    /// B 的主判据：`CanCreateChapter` 必须先看有没有项目。
    ///
    /// ⚠️ 判据落在「门禁包含 HasProjectRoot」而非「返回 false」：
    /// 缺陷版本在字段没填时也返回 false，断言「某个状态下不可用」会在缺陷下照样绿。
    /// 要区分的是**为什么**不可用。
    /// </summary>
    [Fact]
    public void CreateChapter_RequiresAnOpenProject()
    {
        var source = ReadWorksViewModel();

        var start = source.IndexOf("private bool CanCreateChapter()", StringComparison.Ordinal);
        Assert.True(start > 0, "找不到 CanCreateChapter");
        var end = source.IndexOf("\n    }", start, StringComparison.Ordinal);
        Assert.True(end > start, "找不到 CanCreateChapter 的结束位置");

        var body = source[start..end];
        Assert.Contains("_backend.HasProjectRoot", body, StringComparison.Ordinal);
    }

    /// <summary>
    /// 灰掉按钮不够，必须说清为什么 —— 且那句话要真的挂在界面上。
    ///
    /// ⚠️ 这一条独立于上一条：门禁对了但没有文案，作者仍然看着一颗
    /// 点不动的按钮不知所以。本仓已有范式（`RunEntryTooltip` /
    /// `RestoreBlockedText`），这里守的是「本条也遵守了那个范式」。
    /// </summary>
    [Fact]
    public void CreateChapterBlockedReason_IsBoundInTheMarkup()
    {
        // ⚠️ **剥 XAML 注释后再匹配**。本条对源码原文做 `Contains`，
        // `<!-- … -->` 里若出现同样的字符串就会假命中 ——
        // U210 施工时正是这样假绿的（变异标记里复述了被断言的字符串）。
        // 本文件上面那条用例已经因为同类原因红过一次（我的注释引用了缺陷代码），
        // 那次教训只落在了 C# 行注释上，XAML 这一侧当时没跟着做。
        var markup = StripXamlComments(ReadWorksMarkup());

        Assert.Contains(
            "{Binding CreateChapterBlockedText}",
            markup,
            StringComparison.Ordinal);
        Assert.Contains(
            "{Binding HasCreateChapterBlockedText}",
            markup,
            StringComparison.Ordinal);
    }

    /// <summary>剥掉 <c>&lt;!-- --&gt;</c> 注释，只留真实标记。</summary>
    private static string StripXamlComments(string markup)
        => System.Text.RegularExpressions.Regex.Replace(
            markup, "<!--.*?-->", string.Empty,
            System.Text.RegularExpressions.RegexOptions.Singleline);

    /// <summary>那句原因文案的键必须真的存在，否则界面显示方括号原文。</summary>
    [Fact]
    public void CreateChapterBlockedReason_HasRealCopyNotAPlaceholder()
    {
        var names = Ariadne.Desktop.Localization.DisplayNameService.LoadDefault();
        var text = names.Text("ui.works.create_chapter.needs_project");

        Assert.False(
            text.StartsWith('[') && text.EndsWith(']'),
            $"文案键缺失，界面会显示 {text}");
        Assert.DoesNotContain("不符合要求", text, StringComparison.Ordinal);
    }

    private static string ReadWorksViewModel()
        => File.ReadAllText(ResolveDesktopFile(
            Path.Combine("Ariadne.Desktop", "ViewModels", "WorksPageViewModel.cs")));

    private static string ReadWorksMarkup()
        => File.ReadAllText(ResolveDesktopFile(
            Path.Combine("Ariadne.Desktop", "Views", "WorksPageView.axaml")));

    private static string ResolveDesktopFile(string relative)
    {
        var dir = AppContext.BaseDirectory;
        for (var i = 0; i < 8 && dir is not null; i++)
        {
            var candidate = Path.Combine(dir, "desktop", relative);
            if (File.Exists(candidate))
            {
                return candidate;
            }

            candidate = Path.Combine(dir, relative);
            if (File.Exists(candidate))
            {
                return candidate;
            }

            dir = Path.GetDirectoryName(dir);
        }

        throw new FileNotFoundException($"从 {AppContext.BaseDirectory} 向上找不到 {relative}");
    }
}
