using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 六个主功能页的公共状态行基类：**状态文案 + 补救建议是一对，不是两件事**。
///
/// ## 为什么把它提成基类（U198-B）
///
/// 「失败原因 + 下一步做什么」这套基元此前只装在配置页上（`RecoveryText` /
/// `HasRecoveryText` 原来是 `SettingsPageViewModel` 的私有属性），
/// 画布 / 作品 / Git / 运行记录 / 模板五页**零消费** —— 覆盖面 1/6。
/// 作者第一次点运行失败，界面只有「已失败」两个字。
///
/// 复制五份属性能让覆盖面变成 6/6，但那种 6/6 是**约定**而不是**结构**：
/// 下一个新页面照样会漏。提成基类之后「有状态行的页面必然有补救行」，
/// 漏不掉——这是把 `IsLoading` 那类「有实现、有维护、界面零消费」的形态
/// 从源头上排除掉。
///
/// ## 为什么 StatusText 也搬进来
///
/// 不是顺手合并，是**清除陈旧建议的唯一可靠时机**。
/// `RecoveryText` 只由 <see cref="ReportFailure"/> 写入；如果不管清除，
/// 一次失败之后作者保存成功，界面会是「已保存」+「请检查网络连接后重试。」
/// —— 建议还挂在那里，指向一个已经不存在的问题。
/// 把 StatusText 收进同一个类，就能在「状态被改成非失败文案」那一刻清掉建议，
/// 而不必在每个页面的每条成功路径上手工加一行 <c>RecoveryText = string.Empty</c>
/// （那种做法必然漏，而且漏了没有任何征兆）。
/// </summary>
public abstract class PageViewModelBase : ViewModelBase
{
    private string _statusText = string.Empty;
    private string _recoveryText = string.Empty;
    /// <summary>
    /// 当前建议**归属**的那句主文案。
    ///
    /// ⚠️ 这里刻意不是「正在上报失败」的布尔标志位 —— 我第一版就是那么写的，
    /// 而它**恒等于失效**：调用形状是 <c>StatusText = ReportFailure(ex, names)</c>，
    /// `ReportFailure` 早在赋值发生**之前**就返回了（标志位也随之复位），
    /// 于是紧接着那次 StatusText 赋值总被判为「非失败路径」，
    /// 刚算好的建议当场被清掉 —— 界面永远看不到建议。
    /// 守卫 `RecoveryHint_IsClearedWhenTheNextStatusIsNotAFailure` 实测抓到了这一条。
    ///
    /// 改记「归属的主文案」之后与赋值顺序无关：状态行还写着那次失败，建议就留着；
    /// 一旦被改成别的话（成功、加载中、下一次操作），建议即失效。
    /// </summary>
    private string _recoveryOwnerStatus = string.Empty;

    /// <summary>
    /// 页级状态行。改成**不是**当前建议所属的那句话时，建议一并失效。
    /// </summary>
    public string StatusText
    {
        get => _statusText;
        set
        {
            if (SetProperty(ref _statusText, value))
            {
                OnPropertyChanged(nameof(HasStatusText));
            }
            if (!string.Equals(value, _recoveryOwnerStatus, StringComparison.Ordinal))
            {
                RecoveryText = string.Empty;
            }
        }
    }

    public bool HasStatusText => !string.IsNullOrWhiteSpace(StatusText);

    /// <summary>
    /// 「下一步做什么」。空串 = 这次失败没有可操作的补救动作（例如作者自己按了停止），
    /// 此时 <see cref="HasRecoveryText"/> 为 false，渲染位整条隐藏——
    /// 不要为了「总有一行」去填一句「请重试」，那会训练作者忽略这一行。
    /// </summary>
    public string RecoveryText
    {
        get => _recoveryText;
        protected set
        {
            if (SetProperty(ref _recoveryText, value))
            {
                OnPropertyChanged(nameof(HasRecoveryText));
            }
        }
    }

    public bool HasRecoveryText => !string.IsNullOrWhiteSpace(RecoveryText);

    /// <summary>
    /// 失败 → 主文案 的唯一入口。返回值直接赋给 StatusText / ErrorText 等任意展示位，
    /// 副作用是把补救建议一并算好。
    ///
    /// 签名与 <see cref="UserFacingError.Format"/> 完全一致，是为了让既有的
    /// <c>StatusText = UserFacingError.Format(ex, _displayNames)</c> 能原地替换为
    /// <c>StatusText = ReportFailure(ex, _displayNames)</c> —— 一次机械替换，
    /// 编译器负责挑出放错位置的调用（静态方法、嵌套类里的同名调用）。
    /// </summary>
    protected string ReportFailure(Exception? ex, DisplayNameService names, string? contextKey = null)
    {
        var primary = UserFacingError.Format(ex, names, contextKey);
        // 先登记归属再写建议：顺序反了的话 RecoveryText 的 setter 还没有归属可比，
        // 而调用方随后那次 StatusText 赋值会把它判成「别人的话」清掉。
        _recoveryOwnerStatus = primary;
        RecoveryText = UserFacingError.Recovery(ex, names);
        return primary;
    }

    /// <summary>
    /// 直接给出一句建议（不经异常）。用于后端已经产出成文建议的场合，
    /// 例如工作流运行失败的 <c>WorkflowRunFailure.RecoverySuggestion</c>。
    ///
    /// 归属登记为**当前**状态行：此时状态行写着「已失败」，
    /// 作者下一步做任何事把它改掉，这条建议就该消失。
    /// </summary>
    protected void SetRecoverySuggestion(string? suggestion, DisplayNameService names)
    {
        _recoveryOwnerStatus = StatusText;
        RecoveryText = UserFacingError.RecoveryFromSuggestion(suggestion, names);
    }

    /// <summary>初始状态文案；构造期赋值不应触发「清建议」以外的任何副作用。</summary>
    protected void InitializeStatusText(string text) => StatusText = text;
}
