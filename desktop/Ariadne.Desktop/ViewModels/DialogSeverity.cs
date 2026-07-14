namespace Ariadne.Desktop.ViewModels;

/// <summary>通用弹窗语义，驱动图标、色与默认焦点策略（U63/U64）。</summary>
public enum DialogSeverity
{
    /// <summary>中性信息 / 说明。</summary>
    Info,

    /// <summary>需要用户做选择的提问（如未保存离开）。</summary>
    Question,

    /// <summary>可能有风险但可撤销的提示。</summary>
    Warning,

    /// <summary>删除、丢弃等高风险确认。</summary>
    Danger,

    /// <summary>成功结果。</summary>
    Success,

    /// <summary>带输入的表单弹窗（新建项目等）。</summary>
    Input,
}
