using System.Reflection;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Headless;
using Avalonia.Input;
using Avalonia.Threading;
using Avalonia.VisualTree;
using Ariadne.Desktop;
using Ariadne.Desktop.Backend;
using Ariadne.Desktop.Controls;
using Ariadne.Desktop.Localization;
using Ariadne.Desktop.ViewModels;
using Ariadne.Desktop.Views;
using Xunit;

namespace Ariadne.Desktop.Tests;

/// <summary>
/// U9999（用户亲报，数据破坏级）：**在提示词里按 Delete 想删一个字，画布弹出「删除节点」确认框。**
///
/// 用户原话：
/// > 编辑提示词等时删除被理解为删除节点弹确认了！
/// > 复制等也有这个问题！
///
/// # 缺陷形态：闸门早就存在，注释也写对了，但实现无效
///
/// `WorkspacePageView.axaml.cs` 的画布键盘处理里，Delete/Backspace 删节点、
/// Ctrl+C/X/V 复制剪切粘贴节点这一批**破坏性**快捷键之前，早就有一道闸：
/// 原名 `IsTextInputFocused()`，注释写着「输入框内不劫持复制、剪切、粘贴、退格或删除」。
///
/// **判断完全正确，实现无效** —— 它的白名单只有 `TextBox` 和 `ComboBox`，
/// 而提示词编辑器是 `AvaloniaEdit`，焦点实际落在它模板里的 `TextArea` 上，
/// **既不是 `TextBox` 也不是它的子类** ⇒ 闸门放行 ⇒ 画布把 Delete 当成「删节点」。
///
/// ⚠️ 这与同批的 U213-A2 / U211-A 是**同一个形态**：那两条是
/// 「`IsEnabled` 我没在自己身上绑」而祖先绑了；这条是「白名单我写了」而漏了实际类型。
/// 三条的共同点：**代码里那段注释会让人以为已经防住了，而缺陷不在那段代码里。**
///
/// # 判据为什么必须走真实按键路径
///
/// ⚠️ **不能**直接调那个闸门函数断言它返回 true —— 那只证明「函数认得这个类型」，
/// 证不上「真按下 Delete 时节点还在」。中间隔着焦点系统（`GetFocusedElement()`
/// 返回谁）、事件路由（谁先拿到 KeyDown）两层，而**缺陷正出在焦点系统这一层**：
/// 焦点落在的是模板内层的 `TextArea`，不是我们在 XAML 里写的那个控件。
///
/// 所以每条用例都：headless 起真窗口 → 把焦点真正设到目标控件 →
/// 走 `RaiseEvent` 发真实 `KeyEventArgs` → 断言**节点数**（用户可见结果）。
///
/// # 判据为什么取「确认框有没有弹出」而不是「节点数变没变」
///
/// 删节点要过一个 Danger 确认框（`DeleteSelectedNodeAsync` → `ConfirmDangerAsync`），
/// 所以**节点数在两种情况下都不变**：闸门生效（压根没触发删除）、
/// 以及闸门失效但确认框还挂在屏上等人点。
/// ⇒ 用节点数做判据，正向那条会因为「确认框没人点」而**假绿**。
///
/// 我第一版就是这么写的，反向判据当场红了（焦点在画布上按 Delete，节点数 1 → 1），
/// 排查后才发现观测点取错了 —— **而那一红恰好救了正向那条**：
/// 若只写正向、看它绿了就收工，会留下一条什么都没验的用例。
///
/// 用户报告的原话是「**弹确认了**」，那就是他看到的现象，判据也该落在那里。
///
/// # 反向判据不可省
///
/// 只钉「输入框里按 Delete 不弹确认框」的话，最省事的假修法是**让画布再也不处理 Delete**
/// —— 那会把「选中节点按 Delete 删掉」这个正当功能一起弄没。
/// 所以每条正向判据都配一条「焦点在画布上时同一个按键**确实**弹出了确认框」。
/// </summary>
[Collection("GlobalDialogService")]
public sealed class CanvasShortcutsYieldToTextEditingTests
{
    /// <summary>
    /// 主判据：**焦点在提示词编辑器里按 Delete，不许弹「删除节点」确认框。**
    ///
    /// 这是用户亲报的那一幕。焦点刻意设到 `PromptTemplateEditor.TextArea`
    /// （而不是 `PromptTemplateEditor` 本身）—— `TextEditor` 自身 `Focusable=false`，
    /// 真实点击后焦点必然落在 `TextArea` 上，那正是旧白名单漏掉的类型。
    /// </summary>
    [Fact]
    public async Task DeleteInsidePromptEditor_DoesNotAskToDeleteTheNode()
    {
        await RunAsync(async harness =>
        {
            var area = harness.PromptEditor.TextArea;
            Assert.True(area.Focus(), "提示词编辑器的 TextArea 拿不到焦点，本用例没走到缺陷现场");
            await Drain();

            harness.PressKey(Key.Delete);
            await Drain();

            Assert.Null(DialogService.Current.ActiveDialog);
            Assert.Equal(harness.NodeCountAtStart, harness.ViewModel.Nodes.Count);
        });
    }

    /// <summary>
    /// 反向判据：**焦点在画布上时，Delete 确实发起了删除**（弹出确认框）。
    ///
    /// 缺了这条，「让画布再也不处理 Delete」这种假修法能让上面那条全绿，
    /// 代价是把正当功能弄没 —— 本仓已记「做一半的功能会掩盖没做的一半」。
    /// </summary>
    [Fact]
    public async Task DeleteWithCanvasFocused_StillAsksToDeleteTheNode()
    {
        await RunAsync(async harness =>
        {
            // 焦点给画布宿主本身（不是任何输入控件）。
            harness.View.Focus();
            await Drain();

            harness.PressKey(Key.Delete);
            await Drain();

            Assert.True(
                DialogService.Current.ActiveDialog is not null,
                "焦点在画布上按 Delete 却没发起删除 ⇒ 闸门做成了「一律不处理」，"
                + "把正当功能一起关掉了。");
        });
    }

    /// <summary>
    /// Ctrl+V：用户原话「复制等也有这个问题」。
    ///
    /// 这条比 Delete 更隐蔽：粘贴节点**不过确认框**，所以作者在提示词里按 Ctrl+V
    /// 想粘一段文字，画布会**直接往画布上多出一个节点**，而他要粘的文字没进去。
    /// ⇒ 判据取节点数（这里它是有效判据，因为没有确认框这一层）。
    /// </summary>
    [Fact]
    public async Task PasteInsidePromptEditor_DoesNotPasteANode()
    {
        await RunAsync(async harness =>
        {
            // 先复制一个节点进内部剪贴板，否则「粘贴不出节点」可能只是因为没东西可粘。
            harness.View.Focus();
            await Drain();
            harness.PressKey(Key.C, KeyModifiers.Control);
            await Drain();

            var before = harness.ViewModel.Nodes.Count;
            Assert.True(
                harness.ViewModel.PasteNodeCommand.CanExecute(null),
                "内部剪贴板是空的 ⇒ 「粘贴没产生节点」这个判据无效（前提自检）");

            harness.PromptEditor.TextArea.Focus();
            await Drain();
            harness.PressKey(Key.V, KeyModifiers.Control);
            await Drain();

            Assert.Equal(before, harness.ViewModel.Nodes.Count);
        });
    }

    // ── 脚手架 ───────────────────────────────────────────────

    private sealed class Harness
    {
        public required WorkspacePageView View { get; init; }
        public required WorkspacePageViewModel ViewModel { get; init; }
        public required Window Window { get; init; }
        public required PromptTemplateEditor PromptEditor { get; init; }
        public required int NodeCountAtStart { get; init; }

        /// <summary>
        /// 发一个**真实**的 KeyDown 事件。
        ///
        /// ⚠️ 刻意从 `View` 发起而不是直调 `OnWorkspaceKeyDown`：后者会绕过焦点系统，
        /// 而焦点系统正是缺陷所在（焦点落在模板内层的 TextArea 上）。
        /// </summary>
        public void PressKey(Key key, KeyModifiers modifiers = KeyModifiers.None)
        {
            View.RaiseEvent(new KeyEventArgs
            {
                RoutedEvent = InputElement.KeyDownEvent,
                Key = key,
                KeyModifiers = modifiers,
                Source = View,
            });
        }
    }

    private static async Task RunAsync(Func<Harness, Task> body)
    {
        using var session = HeadlessUnitTestSession.StartNew(
            typeof(HeadlessAppBuilder),
            AvaloniaTestIsolationLevel.PerTest);

        await session.Dispatch(
            async () =>
            {
                var names = DisplayNameService.LoadDefault();
                // 确认框走全局 DialogService 单例 ⇒ 必须初始化，否则 ActiveDialog 永远是 null，
                // 反向判据会红在一个与产品无关的原因上。
                DialogService.Initialize(names);
                var viewModel = new WorkspacePageViewModel(
                    names,
                    DispatchProxy.Create<IAriadneBackendClient, SoftBackendProxy>());
                var view = new WorkspacePageView { DataContext = viewModel };
                var window = new Window { Width = 1400, Height = 900, Content = view };
                window.Show();
                await Drain();

                // 右栏（检查器）必须打开，提示词编辑器才在视觉树里；
                // 折叠时那棵子树不挂树，焦点设不过去 —— 与 U213-A2 那条
                // 「Expander 折叠时 IsEffectivelyEnabled 不重算」同源。
                viewModel.IsRightPanelOpen = true;
                viewModel.AddNodeAt("summarizer", 200, 200);
                viewModel.SelectNode(viewModel.Nodes[0]);
                await Drain();

                var editor = view.GetVisualDescendants().OfType<PromptTemplateEditor>()
                    .FirstOrDefault(candidate => candidate.IsEffectivelyVisible);
                Assert.True(
                    editor is not null,
                    "检查器里找不到可见的提示词编辑器 ⇒ 本用例的前提没了，请重新定判据");

                try
                {
                    await body(new Harness
                    {
                        View = view,
                        ViewModel = viewModel,
                        Window = window,
                        PromptEditor = editor!,
                        NodeCountAtStart = viewModel.Nodes.Count,
                    });
                }
                finally
                {
                    window.Content = null;
                    window.Close();
                    await Drain();
                }

                return true;
            },
            CancellationToken.None);
    }

    private static async Task Drain()
    {
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.Loaded);
        await Dispatcher.UIThread.InvokeAsync(() => { }, DispatcherPriority.SystemIdle);
    }

    private static class HeadlessAppBuilder
    {
        public static AppBuilder BuildAvaloniaApp() =>
            AppBuilder.Configure<App>().UseHeadless(new AvaloniaHeadlessPlatformOptions());
    }

    /// <summary>DispatchProxy 的宿主类不能 sealed（运行时要派生它）。</summary>
    private class SoftBackendProxy : DispatchProxy
    {
        protected override object? Invoke(MethodInfo? targetMethod, object?[]? args)
        {
            if (targetMethod?.Name == "get_HasProjectRoot")
            {
                return true;
            }
            if (targetMethod?.ReturnType == typeof(Task))
            {
                return Task.CompletedTask;
            }
            if (targetMethod?.ReturnType.IsGenericType == true
                && targetMethod.ReturnType.GetGenericTypeDefinition() == typeof(Task<>))
            {
                var resultType = targetMethod.ReturnType.GetGenericArguments()[0];
                var value = resultType.IsValueType ? Activator.CreateInstance(resultType) : null;
                return typeof(Task).GetMethod(nameof(Task.FromResult))!
                    .MakeGenericMethod(resultType)
                    .Invoke(null, new[] { value });
            }
            return targetMethod?.ReturnType is { IsValueType: true } vt
                ? Activator.CreateInstance(vt)
                : null;
        }
    }
}
