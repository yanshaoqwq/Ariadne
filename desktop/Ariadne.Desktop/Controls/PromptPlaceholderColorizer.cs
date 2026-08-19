using Avalonia.Media;
using AvaloniaEdit.Document;
using AvaloniaEdit.Rendering;
using Ariadne.Desktop.ViewModels;

namespace Ariadne.Desktop.Controls;

/// <summary>
/// U115：提示词编辑器里 `{{...}}` 占位符的**高亮**。
///
/// # 为什么高亮值得做（不是装饰）
///
/// 拼错的变量会让节点 fail-loud（U114 已保证运行时报错）。但那意味着用户要
/// **跑一次工作流、花一次 LLM 钱**才知道自己少打了一个点。高亮把这个反馈提前到
/// 敲键的那一刻——同一件事，成本差一个数量级。
///
/// # 三档颜色，不是两档
///
/// - **合法**（引用 / 已知命名空间）：强调色。可用。
/// - **待确认**（裸名，后端去 inputs 里找）：柔和的次级色，**不报警**。
///   编辑期没有 inputs，无从判断真伪；标红是误报，而误报会训练用户忽略颜色。
/// - **确定会失败**（空占位符 / `skill.` 废弃命名空间 / 非法引用语法）：错误色。
///
/// 中间那一档是本设计的关键。少了它就只能在「全部标红」（噪音）与
/// 「全不标红」（漏报真错误）之间二选一。
/// </summary>
internal sealed class PromptPlaceholderColorizer : DocumentColorizingTransformer
{
    private readonly Func<PromptPlaceholderSyntax.PlaceholderKind, IBrush?> _brushFor;

    /// <param name="brushFor">按种类取画刷；从主题现取，不在这里写颜色。</param>
    internal PromptPlaceholderColorizer(
        Func<PromptPlaceholderSyntax.PlaceholderKind, IBrush?> brushFor)
    {
        _brushFor = brushFor;
    }

    /// <summary>
    /// 逐行着色。
    ///
    /// **按行扫而不是全文扫一次**：`ColorizeLine` 只在该行进入视口时被调用，
    /// 所以扫描量与**可见行数**成正比，与提示词总长无关。全文扫一次再按行查表
    /// 反而要为屏幕外的内容付钱（提示词模板可以很长）。
    ///
    /// ⚠️ 跨行的占位符（`{{` 与 `}}` 不在同一行）因此**不会**被高亮。
    /// 这是有意的取舍：后端 `render_prompt_template` 用 `find("{{")` 不管换行，
    /// 所以跨行写法是**合法**的，只是罕见（谁会在变量名中间敲回车）。
    /// 为它引入跨行状态机会把「按可见行付费」这条性质破坏掉。
    /// </summary>
    protected override void ColorizeLine(DocumentLine line)
    {
        if (line.Length == 0)
        {
            return;
        }

        var text = CurrentContext.Document.GetText(line.Offset, line.Length);
        foreach (var placeholder in PromptPlaceholderSyntax.Parse(text))
        {
            var brush = _brushFor(placeholder.Kind);
            if (brush is null)
            {
                // 主题查不到就**不改**：DynamicResource 缺键时属性停在未赋值状态，
                // 在这里强行设 null 会把文字画成透明（U162 的同类陷阱）。
                continue;
            }

            ChangeLinePart(
                line.Offset + placeholder.Start,
                line.Offset + placeholder.End,
                element => element.TextRunProperties.SetForegroundBrush(brush));
        }
    }
}
