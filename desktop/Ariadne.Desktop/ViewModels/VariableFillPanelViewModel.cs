using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Linq;
using System.Threading.Tasks;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 执行页 Ctrl+K 的「AI 填变量值」面板。
///
/// 结构与作品页快捷改写**逐段对齐**（标题 + 说明框 + 生成 / diff / 撤销 + 应用），
/// 快捷键与焦点行为也一致：Ctrl+K 开面板并把光标送进说明框，生成中不许关窗
/// （请求已发出，关窗只会让结果无处落地），关窗即丢弃未应用的建议
/// （留着会在下次开窗给出一条与当前取值早已不同步的旧 diff，而应用又会被守卫拒绝，
/// 等于摆一个死按钮）。差别只有一处：diff 区显示的是变量新旧值，不是正文行。
///
/// **不复用 <see cref="WorksPageViewModel"/> 那份实现**：那份的每一条状态都锚在
/// 「当前文档 id / 版本 / 正文 / 编辑器选区」上（守卫、撤销、失效时机全由
/// OnEditorDocumentTextChanged 之类的正文事件驱动），执行页一件都没有。
/// 抽公共基类只能把这些字段抽象成占位参数，守卫随之退化成永远成立的比较——
/// 那比重写一份更危险。这里共用的是**判据形状**与 diff 行视图
/// （<see cref="QuickEditDiffLineViewModel"/> 解析同一套 -/+ 前缀）。
/// </summary>
public sealed class VariableFillPanelViewModel : ViewModelBase
{
    private readonly Func<string, string> _text;
    private readonly Func<string, IReadOnlyDictionary<string, string>, string> _format;
    private readonly Action<string> _report;
    private readonly Func<Exception, string> _describeError;
    private WorkflowVariableGroupViewModel? _target;
    private string _instruction = string.Empty;
    private string _diff = string.Empty;
    private bool _isOpen;
    private bool _isGenerating;
    private VariableFillSession? _pending;
    private VariableFillUndoState? _undo;

    public VariableFillPanelViewModel(
        Func<string, string> text,
        Func<string, IReadOnlyDictionary<string, string>, string> format,
        Action<string> report,
        Func<Exception, string> describeError)
    {
        _text = text;
        _format = format;
        _report = report;
        _describeError = describeError;
        CloseCommand = new RelayCommand(Close, () => !IsGenerating);
        GenerateCommand = new RelayCommand(() => _ = GenerateAsync(), CanGenerate);
        ApplyCommand = new RelayCommand(Apply, CanApply);
        UndoCommand = new RelayCommand(Undo, CanUndo);
    }

    public RelayCommand CloseCommand { get; }
    public RelayCommand GenerateCommand { get; }
    public RelayCommand ApplyCommand { get; }
    public RelayCommand UndoCommand { get; }

    /// <summary>
    /// 发请求的通道，由页面 VM 注入（这里够不到后端客户端）。
    /// 未注入时生成命令不可用，而不是点了没反应。
    /// </summary>
    public Func<string, Task<string>>? RequestFill { get; set; }

    /// <summary>开面板后把光标送进说明框；由视图注入（同作品页 Ctrl+K）。</summary>
    public Action? RequestFocusInstruction { get; set; }

    /// <summary>本次要填的变量组。null = 画布上没有带变量的起始节点，此时 Ctrl+K 无处可填。</summary>
    public WorkflowVariableGroupViewModel? Target => _target;

    public ObservableCollection<QuickEditDiffLineViewModel> DiffLines { get; } = new();

    /// <summary>diff 为空时整块不渲染，避免留下一个空白框。</summary>
    public bool HasDiff => DiffLines.Count > 0;

    public string TitleText => _text("ui.workspace.variable_fill.title");
    public string PlaceholderText => _text("ui.workspace.variable_fill.placeholder");
    public string DiffText => _text("ui.workspace.variable_fill.diff");
    public string ApplyText => _text("ui.workspace.variable_fill.apply");
    public string UndoText => _text("ui.workspace.variable_fill.undo");
    public string CloseText => _text("ui.common.close");

    /// <summary>生成中改文案而不是只禁用：按钮变灰但字不变，看起来像卡住了。</summary>
    public string GenerateText => _text(IsGenerating
        ? "ui.workspace.variable_fill.generating"
        : "ui.workspace.variable_fill.generate");

    public bool IsOpen
    {
        get => _isOpen;
        private set
        {
            if (SetProperty(ref _isOpen, value))
            {
                OnPropertyChanged(nameof(IsCloseEnabled));
            }
        }
    }

    public bool IsGenerating
    {
        get => _isGenerating;
        private set
        {
            if (SetProperty(ref _isGenerating, value))
            {
                OnPropertyChanged(nameof(GenerateText));
                OnPropertyChanged(nameof(IsCloseEnabled));
                GenerateCommand.NotifyCanExecuteChanged();
                ApplyCommand.NotifyCanExecuteChanged();
                CloseCommand.NotifyCanExecuteChanged();
            }
        }
    }

    /// <summary>生成中禁用关闭：请求已经发出去了（钱已经花了）。</summary>
    public bool IsCloseEnabled => !IsGenerating;

    public string Instruction
    {
        get => _instruction;
        set
        {
            if (SetProperty(ref _instruction, value ?? string.Empty))
            {
                // 改了说明就等于要重新生成：留着上一条 diff 会让「应用」落到
                // 与眼前说明不符的改动上。
                Invalidate();
                GenerateCommand.NotifyCanExecuteChanged();
            }
        }
    }

    /// <summary>
    /// 打开面板并指定本次的变量组。
    ///
    /// 换目标（切到另一个起始节点）必须清空待应用建议与撤销态：
    /// 那些取值属于上一个节点，套到这个节点上是张冠李戴。
    /// </summary>
    public bool Open(WorkflowVariableGroupViewModel? target)
    {
        if (target is null)
        {
            _report(_text("ui.workspace.variable_fill.no_target"));
            return false;
        }

        if (!ReferenceEquals(_target, target))
        {
            _target = target;
            OnPropertyChanged(nameof(Target));
            Invalidate();
            ClearUndo();
        }

        IsOpen = true;
        GenerateCommand.NotifyCanExecuteChanged();
        RequestFocusInstruction?.Invoke();
        return true;
    }

    private void Close()
    {
        if (IsGenerating)
        {
            return;
        }

        IsOpen = false;
        Invalidate();
    }

    private bool CanGenerate() =>
        RequestFill is not null
        && _target is { HasVariables: true }
        && !IsGenerating
        && !string.IsNullOrWhiteSpace(Instruction);

    private async Task GenerateAsync()
    {
        if (_target is not { } target || RequestFill is null)
        {
            return;
        }

        var variables = target.Variables.ToList();
        var snapshot = VariableFillProtocol.CaptureValues(variables);
        var message = VariableFillProtocol.BuildMessage(Instruction, variables);
        IsGenerating = true;
        try
        {
            var answer = await RequestFill(message).ConfigureAwait(true);
            var parsed = VariableFillProtocol.Parse(answer, variables);
            // 等回话期间作者又改了取值：这条建议基于的快照已过期，整体作废。
            // 半套上去会得到一个「作者没写过、AI 也没提过」的第三种状态。
            var live = VariableFillProtocol.CaptureValues(target.Variables.ToList());
            if (!DictionaryEquals(snapshot, live))
            {
                _report(_text("ui.workspace.variable_fill.outdated"));
                return;
            }

            if (parsed.Changes.Count == 0)
            {
                _report(parsed.RejectedNames.Count > 0
                    ? FormatRejected(parsed.RejectedNames)
                    : _text("ui.workspace.variable_fill.no_change"));
                return;
            }

            _pending = new VariableFillSession(snapshot, parsed.Changes);
            Diff = _pending.BuildDiffText(_ => _text("ui.workspace.variable_fill.blank_value"));
            ApplyCommand.NotifyCanExecuteChanged();
            // 被拒条目要说出来：静默丢弃会让作者以为 AI 填了那几个变量。
            _report(parsed.RejectedNames.Count > 0
                ? FormatRejected(parsed.RejectedNames)
                : _text("ui.workspace.variable_fill.ready"));
        }
        catch (OperationCanceledException)
        {
            // 工作流已切换；迟到的建议不得落到新会话的变量上。
        }
        catch (Exception ex)
        {
            _report(_describeError(ex));
        }
        finally
        {
            IsGenerating = false;
        }
    }

    private bool CanApply() =>
        !IsGenerating
        && _pending is not null
        && _target is { } target
        && _pending.MatchesCurrent(VariableFillProtocol.CaptureValues(target.Variables.ToList()));

    private void Apply()
    {
        if (_pending is not { } session || _target is not { } target)
        {
            return;
        }

        var live = VariableFillProtocol.CaptureValues(target.Variables.ToList());
        if (!session.MatchesCurrent(live))
        {
            Invalidate();
            _report(_text("ui.workspace.variable_fill.outdated"));
            return;
        }

        foreach (var change in session.Changes)
        {
            var variable = target.Variables.FirstOrDefault(item =>
                string.Equals(item.Name, change.Name, StringComparison.Ordinal));
            if (variable is not null)
            {
                variable.Text = change.NewText;
            }
        }

        _undo = new VariableFillUndoState(
            VariableFillProtocol.CaptureValues(target.Variables.ToList()),
            live);
        UndoCommand.NotifyCanExecuteChanged();
        Invalidate();
        _report(_text("ui.workspace.variable_fill.applied"));
    }

    private bool CanUndo() =>
        _undo is not null
        && _target is { } target
        && _undo.CanUndo(VariableFillProtocol.CaptureValues(target.Variables.ToList()));

    private void Undo()
    {
        if (_undo is not { } undo || _target is not { } target)
        {
            return;
        }

        if (!undo.CanUndo(VariableFillProtocol.CaptureValues(target.Variables.ToList())))
        {
            ClearUndo();
            _report(_text("ui.workspace.variable_fill.undo_unavailable"));
            return;
        }

        foreach (var variable in target.Variables)
        {
            if (undo.PreviousValues.TryGetValue(variable.Name, out var previous))
            {
                variable.Text = previous;
            }
        }

        ClearUndo();
        _report(_text("ui.workspace.variable_fill.undone"));
    }

    /// <summary>丢弃待应用建议。撤销态**不动**：应用完的那次仍该可撤。</summary>
    private void Invalidate()
    {
        if (_pending is null && Diff.Length == 0)
        {
            return;
        }

        _pending = null;
        Diff = string.Empty;
        ApplyCommand.NotifyCanExecuteChanged();
    }

    private void ClearUndo()
    {
        if (_undo is null)
        {
            return;
        }

        _undo = null;
        UndoCommand.NotifyCanExecuteChanged();
    }

    private string FormatRejected(IReadOnlyList<string> names) =>
        _format("ui.workspace.variable_fill.rejected", new Dictionary<string, string>
        {
            ["names"] = string.Join("、", names),
        });

    private string Diff
    {
        get => _diff;
        set
        {
            if (SetProperty(ref _diff, value))
            {
                RebuildDiffLines();
            }
        }
    }

    /// <summary>
    /// 把 diff 文本翻成分行视图。解析器与作品页、确认项审阅是同一个
    /// （<see cref="QuickEditDiffLineViewModel"/>）：各写一份迟早漂移。
    /// </summary>
    private void RebuildDiffLines()
    {
        DiffLines.Clear();
        if (_diff.Length > 0)
        {
            foreach (var line in _diff.Split('\n'))
            {
                // 末尾换行会切出一个空串，跳过它，否则视图底部多一条空行。
                if (line.Length == 0)
                {
                    continue;
                }

                DiffLines.Add(new QuickEditDiffLineViewModel(line));
            }
        }

        OnPropertyChanged(nameof(HasDiff));
    }

    private static bool DictionaryEquals(
        IReadOnlyDictionary<string, string> left,
        IReadOnlyDictionary<string, string> right)
    {
        if (left.Count != right.Count)
        {
            return false;
        }

        foreach (var (name, value) in left)
        {
            if (!right.TryGetValue(name, out var other)
                || !string.Equals(other, value, StringComparison.Ordinal))
            {
                return false;
            }
        }

        return true;
    }
}
