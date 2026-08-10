using System.Linq;
using Ariadne.Desktop.ViewModels;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// 快速编辑 diff 的行级解析。
///
/// 此前作品页把整段 diff 塞进一个只读 TextBox：增删行没有任何视觉区分，
/// 用户要逐字比对才知道改了什么。改成统一视图（一行红一行绿）后，
/// 「哪些行算增、哪些算删」由这里的前缀解析决定——解析错了就是**着错色**，
/// 而着错色比不着色更危险：用户会照着错误的高亮做判断。
/// </summary>
public sealed class QuickEditDiffLineTests
{
    [Theory]
    [InlineData("- 旧句", QuickEditDiffLineKind.Removed, "旧句")]
    [InlineData("+ 新句", QuickEditDiffLineKind.Added, "新句")]
    [InlineData("  上下文", QuickEditDiffLineKind.Context, "上下文")]
    public void PrefixDecidesKindAndIsStripped(string raw, QuickEditDiffLineKind kind, string text)
    {
        var line = new QuickEditDiffLineViewModel(raw);

        Assert.Equal(kind, line.Kind);
        Assert.Equal(text, line.Text);
    }

    /// <summary>
    /// 正文本身以 "-" 或 "+" 开头时（Markdown 列表、破折号对白），
    /// 不能被误判成增删行——后端的标记是**两个字符**的 "- " / "+ "，
    /// 只比对首字符会把作者写的每一条列表项都涂成红色。
    /// </summary>
    [Theory]
    [InlineData("-无空格不是删除标记")]
    [InlineData("+无空格不是新增标记")]
    public void SingleCharacterPrefixIsNotTreatedAsMarker(string raw)
    {
        var line = new QuickEditDiffLineViewModel(raw);

        Assert.Equal(QuickEditDiffLineKind.Context, line.Kind);
        // 未剥掉任何字符：它压根不是标记。
        Assert.Equal(raw, line.Text);
    }

    /// <summary>
    /// 上下文行的正文若以空格开头（中文小说的缩进段落），
    /// 只能剥掉固定的两字符前缀，不能 Trim——否则缩进丢失，
    /// 用户会以为 AI 改动了排版。
    /// </summary>
    [Fact]
    public void ContextLineKeepsItsOwnLeadingSpaces()
    {
        var line = new QuickEditDiffLineViewModel("    缩进两格的段落");

        Assert.Equal(QuickEditDiffLineKind.Context, line.Kind);
        Assert.Equal("  缩进两格的段落", line.Text);
    }

    /// <summary>三类行的标记宽度一致，正文左边缘才能对齐。</summary>
    [Fact]
    public void MarkersAreSingleCharacterSoTextAligns()
    {
        Assert.Equal("-", new QuickEditDiffLineViewModel("- a").Marker);
        Assert.Equal("+", new QuickEditDiffLineViewModel("+ a").Marker);
        Assert.Equal(" ", new QuickEditDiffLineViewModel("  a").Marker);
    }

    /// <summary>
    /// 折叠标记（后端对连续未变行产出 "  ... (N unchanged lines)"）
    /// 属于上下文，不该被涂色。
    /// </summary>
    [Fact]
    public void CollapsedRunMarkerIsContext()
    {
        var line = new QuickEditDiffLineViewModel("  ... (12 unchanged lines)");

        Assert.Equal(QuickEditDiffLineKind.Context, line.Kind);
        Assert.False(line.IsAdded);
        Assert.False(line.IsRemoved);
    }
}
