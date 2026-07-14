using Ariadne.Desktop.Localization;

namespace Ariadne.Desktop.ViewModels;

/// 全局弹窗服务：宿主在 MainWindow 内叠层渲染 <see cref="ActiveDialog"/>。
/// - 仿 <see cref="DisplayNameService"/> 的静态单例范式，任意 VM 无需构造注入即可 await 调用。
/// - 同一时刻只承载一个弹窗；已有弹窗时新的请求直接返回其取消结果，避免叠层。
public sealed class DialogService : ViewModelBase
{
    private readonly DisplayNameService _names;
    private ConfirmDialogViewModel? _activeDialog;

    private DialogService(DisplayNameService names)
    {
        _names = names;
    }

    public static DialogService Current { get; private set; } = new(DisplayNameService.Current);

    public static void Initialize(DisplayNameService names)
    {
        Current = new DialogService(names);
    }

    /// 当前弹窗；null 表示无。视图据此与 <see cref="IsOpen"/> 控制遮罩显隐。
    public ConfirmDialogViewModel? ActiveDialog
    {
        get => _activeDialog;
        private set
        {
            if (SetProperty(ref _activeDialog, value))
            {
                OnPropertyChanged(nameof(IsOpen));
            }
        }
    }

    public bool IsOpen => _activeDialog is not null;

    /// 弹「未保存离开」确认，异步返回用户选择（保存/不保存/取消）。
    public async Task<UnsavedLeaveChoice> ConfirmUnsavedLeaveAsync(string? pageContext = null)
    {
        if (IsOpen)
        {
            return UnsavedLeaveChoice.Cancel;
        }

        var dialog = ConfirmDialogViewModel.UnsavedLeave(_names, pageContext);
        var result = await ShowAsync(dialog).ConfigureAwait(true);
        return (UnsavedLeaveChoice)result;
    }

    /// <summary>多页未保存：一次决策，不边问边执行（U65）。</summary>
    public async Task<UnsavedLeaveChoice> ConfirmUnsavedLeaveManyAsync(IReadOnlyList<string> pageTitles)
    {
        if (IsOpen)
        {
            return UnsavedLeaveChoice.Cancel;
        }

        if (pageTitles.Count == 0)
        {
            return UnsavedLeaveChoice.Save;
        }

        if (pageTitles.Count == 1)
        {
            return await ConfirmUnsavedLeaveAsync(pageTitles[0]).ConfigureAwait(true);
        }

        var dialog = ConfirmDialogViewModel.UnsavedLeaveMany(_names, pageTitles);
        var result = await ShowAsync(dialog).ConfigureAwait(true);
        return (UnsavedLeaveChoice)result;
    }

    /// <summary>键盘 Enter 路由到当前弹窗确认（危险语义会拒绝）。</summary>
    public void RequestConfirmActive()
    {
        _activeDialog?.RequestConfirm();
    }

    /// 通用确认：调用方自备标题/正文/按钮，返回被点击按钮的 ResultIndex（取消或未开=-1）。
    public async Task<int> ConfirmAsync(ConfirmDialogViewModel dialog)
    {
        if (IsOpen)
        {
            return -1;
        }

        return await ShowAsync(dialog).ConfigureAwait(true);
    }

    /// 由视图在按 Esc 或点击遮罩时调用：走弹窗自身的取消语义。
    public void RequestCancelActive()
    {
        _activeDialog?.Cancel();
    }

    private async Task<int> ShowAsync(ConfirmDialogViewModel dialog)
    {
        ActiveDialog = dialog;
        try
        {
            return await dialog.Completion.ConfigureAwait(true);
        }
        finally
        {
            ActiveDialog = null;
        }
    }
}
