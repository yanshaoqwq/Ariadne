using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Linq;
using System.Threading.Tasks;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// 执行页上的一个工作流变量行。
///
/// 所有非 hidden 变量一律可编辑，没有只读态——章节号通常由循环递增，
/// 但作者随时可以改成第 0 章或第 3.5 章。
/// </summary>
public sealed class WorkflowVariableViewModel : ViewModelBase
{
    private string _text;
    private bool _hasParseError;

    public WorkflowVariableViewModel(
        string name,
        string kind,
        object? initialValue,
        bool required,
        Action onChanged)
    {
        Name = name;
        Kind = kind;
        Required = required;
        _text = WorkflowVariableRules.FormatValue(initialValue);
        OnChanged = onChanged;
    }

    public string Name { get; }
    public string Kind { get; }
    public bool Required { get; }
    private Action OnChanged { get; }

    /// <summary>数字右对齐并走等宽：数值就该读作数值。</summary>
    public bool IsNumber => Kind == WorkflowVariableRules.KindNumber;
    public bool IsBoolean => Kind == WorkflowVariableRules.KindBoolean;
    public bool IsText => !IsNumber && !IsBoolean;

    /// <summary>
    /// 布尔变量的勾选态。
    ///
    /// 单独开一个 bool 属性而不是给 CheckBox 套 string↔bool 转换器：
    /// 取值的权威表示仍是 Text（与其它类型同构，启动载荷只走 TryBuildValue），
    /// 这里只是把它翻译给 CheckBox。多一个 converter 反而多一处口径。
    /// </summary>
    public bool IsChecked
    {
        get => bool.TryParse(Text.Trim(), out var flag) && flag;
        set => Text = value ? "true" : "false";
    }

    public string Text
    {
        get => _text;
        set
        {
            if (SetProperty(ref _text, value ?? string.Empty))
            {
                HasParseError = !WorkflowVariableRules.TryParse(Kind, _text, out _);
                OnPropertyChanged(nameof(IsBlank));
                OnPropertyChanged(nameof(IsChecked));
                OnChanged();
            }
        }
    }

    /// <summary>文本不是该类型的合法值。不静默塞默认值——那会让作者以为填进去了。</summary>
    public bool HasParseError
    {
        get => _hasParseError;
        private set => SetProperty(ref _hasParseError, value);
    }

    /// <summary>当前取值是否为空白（null / 空白串；0 与 false 不算空）。</summary>
    public bool IsBlank =>
        WorkflowVariableRules.TryParse(Kind, Text, out var value)
        && WorkflowVariableRules.IsBlank(value);

    /// <summary>转成启动载荷里的取值；解析失败时返回 false。</summary>
    public bool TryBuildValue(out object? value) =>
        WorkflowVariableRules.TryParse(Kind, Text, out value);
}

/// <summary>
/// 起始节点的变量组：折叠句 + 展开表单 + 启动前校验。
///
/// 折叠态只显示一句话（summary_template 渲染结果），因为变量最终会成为稿子里的
/// 句子——把它读作稿签比读作一堆标签配输入框更贴近它的实际作用。
/// </summary>
public sealed class WorkflowVariableGroupViewModel : ViewModelBase
{
    private readonly Func<string, string> _text;
    private readonly Func<string, IReadOnlyDictionary<string, string>, string> _format;
    private readonly Action? _markDirty;
    private string _summaryTemplate;
    private bool _isExpanded;
    private bool _loading;
    private bool _isGeneratingSummary;

    public WorkflowVariableGroupViewModel(
        Func<string, string> text,
        Func<string, IReadOnlyDictionary<string, string>, string> format,
        Action? markDirty = null)
    {
        _text = text;
        _format = format;
        _markDirty = markDirty;
        _summaryTemplate = string.Empty;
        Variables = new ObservableCollection<WorkflowVariableViewModel>();
        ToggleCommand = new RelayCommand(() => IsExpanded = !IsExpanded);
        // 未注入生成通道（或正在生成、或提示词缺失）时按钮不可用——
        // 比点了没反应诚实。
        GenerateSummaryCommand = new RelayCommand(
            () => _ = RunGenerateSummaryAsync(),
            () => RequestGenerateSummary is not null
                  && !IsGeneratingSummary
                  && Localization.PromptCatalog.ResolveWorkflowVariableSummaryPrompt().Length > 0);
    }

    public ObservableCollection<WorkflowVariableViewModel> Variables { get; }
    public RelayCommand ToggleCommand { get; }
    public RelayCommand GenerateSummaryCommand { get; }

    /// <summary>
    /// 跑一次生成。异常在页面 VM 里翻译成状态栏文案，这里只保证
    /// 无论成败都解除进行态——否则一次失败会把按钮永久禁用。
    /// </summary>
    private async Task RunGenerateSummaryAsync()
    {
        if (RequestGenerateSummary is null)
        {
            return;
        }

        IsGeneratingSummary = true;
        try
        {
            await RequestGenerateSummary().ConfigureAwait(true);
        }
        finally
        {
            IsGeneratingSummary = false;
        }
    }

    /// <summary>没有可见变量时整块不渲染——空的变量区只是噪点。</summary>
    public bool HasVariables => Variables.Count > 0;

    public bool IsExpanded
    {
        get => _isExpanded;
        set
        {
            if (SetProperty(ref _isExpanded, value))
            {
                OnPropertyChanged(nameof(ToggleText));
            }
        }
    }

    public string ToggleText => _text(IsExpanded
        ? "ui.workspace.variables_collapse"
        : "ui.workspace.variables_expand");

    public string SummaryLabelText => _text("ui.workspace.variable_summary_label");
    public string SummaryHintText => _text("ui.workspace.variable_summary_hint");
    public string FillWithAiText => _text("ui.workspace.variable_fill_with_ai");
    public string GenerateSummaryText => _text("ui.workspace.variable_summary_generate");

    /// <summary>
    /// 让项目空间 AI 起一句句式。由页面 VM 注入（这里够不到后端客户端）。
    /// 未注入时命令不可用，而不是静默什么都不做。
    /// </summary>
    public Func<Task>? RequestGenerateSummary { get; set; }

    /// <summary>生成中：按钮禁用并显示进行态，避免连点发出多次请求。</summary>
    public bool IsGeneratingSummary
    {
        get => _isGeneratingSummary;
        set
        {
            if (SetProperty(ref _isGeneratingSummary, value))
            {
                GenerateSummaryCommand.NotifyCanExecuteChanged();
            }
        }
    }

    /// <summary>
    /// 组装给 AI 的提示：模板取自 <c>prompt_list.json</c> 的
    /// <c>workflow.variable_summary</c>，这里只填 <c>{{variables}}</c> 与
    /// <c>{{current}}</c> 两个槽。
    ///
    /// 提示词不写在 C# 里：全项目的提示词都归 prompt_list.json 管，
    /// 硬编码一份会让它绕过那套集中维护（改措辞得改代码、也无从审阅）。
    /// </summary>
    public string BuildGenerateSummaryPrompt()
    {
        var lines = Variables
            .Select(variable => $"- {variable.Name}（{variable.Kind}）当前值：{variable.Text}")
            .ToList();
        var known = lines.Count == 0 ? "（暂无变量）" : string.Join("\n", lines);
        var current = string.IsNullOrWhiteSpace(SummaryTemplate)
            ? "（尚未拟定）"
            : SummaryTemplate;

        return Localization.PromptCatalog.ResolveWorkflowVariableSummaryPrompt()
            .Replace("{{variables}}", known, StringComparison.Ordinal)
            .Replace("{{current}}", current, StringComparison.Ordinal);
    }

    /// <summary>
    /// 句式本身可编辑（项目 AI 生成初稿后作者可改）。
    ///
    /// 改动要标记画布未保存：句式存在 start 节点 config 的 summary_template 键上，
    /// 不标脏就会出现「改完切走再回来没了」——而作者以为已经存了。
    /// Load 期间不标脏：那是把磁盘值灌进来，不是用户编辑。
    /// </summary>
    public string SummaryTemplate
    {
        get => _summaryTemplate;
        set
        {
            if (SetProperty(ref _summaryTemplate, value ?? string.Empty))
            {
                OnPropertyChanged(nameof(SummaryText));
                if (!_loading)
                {
                    _markDirty?.Invoke();
                }
            }
        }
    }

    /// <summary>
    /// 折叠行那句话。有句式按它渲染；缺省时退回按声明顺序拼接。
    /// 与 core 的 render_variable_summary 同口径。
    /// </summary>
    public string SummaryText
    {
        get
        {
            var values = CurrentValues();
            if (!string.IsNullOrWhiteSpace(SummaryTemplate))
            {
                return WorkflowVariableRules.RenderSummary(SummaryTemplate, values);
            }

            return string.Join(
                " · ",
                Variables.Select(variable => $"{variable.Name} {variable.Text}"));
        }
    }

    /// <summary>
    /// 启动前校验：required 变量不得为空，且所有取值必须能解析成声明类型。
    /// 返回 null 表示可以启动；否则是要显示在运行按钮旁的原因——
    /// 禁用按钮必须配文字说明，不能只灰掉。
    /// </summary>
    public string? BlockingReason()
    {
        foreach (var variable in Variables)
        {
            if (variable.HasParseError)
            {
                return _format("ui.workspace.variable_blank_blocks_run", new Dictionary<string, string>
                {
                    ["name"] = variable.Name,
                });
            }

            if (variable.Required && variable.IsBlank)
            {
                return _format("ui.workspace.variable_blank_blocks_run", new Dictionary<string, string>
                {
                    ["name"] = variable.Name,
                });
            }
        }

        return null;
    }

    /// <summary>组装启动载荷；解析失败的变量不写入，由 BlockingReason 拦在前面。</summary>
    public Dictionary<string, object?> BuildStartVariables()
    {
        var payload = new Dictionary<string, object?>(StringComparer.Ordinal);
        foreach (var variable in Variables)
        {
            if (variable.TryBuildValue(out var value) && value is not null)
            {
                payload[variable.Name] = value;
            }
        }

        return payload;
    }

    /// <summary>按 start 节点声明重建变量行；hidden 变量完全不出现。</summary>
    public void Load(IReadOnlyList<WorkflowVariableDeclaration> declarations, string? summaryTemplate)
    {
        // 灌入磁盘值不算用户编辑，期间不标脏——否则一打开画布就显示「有未保存改动」。
        _loading = true;
        try
        {
            Variables.Clear();
            foreach (var declaration in declarations.Where(item => !item.Hidden))
            {
                Variables.Add(new WorkflowVariableViewModel(
                    declaration.Name,
                    declaration.Kind,
                    declaration.Default,
                    declaration.Required,
                    OnVariableChanged));
            }

            SummaryTemplate = summaryTemplate ?? string.Empty;
        }
        finally
        {
            _loading = false;
        }

        OnPropertyChanged(nameof(HasVariables));
        OnPropertyChanged(nameof(SummaryText));
    }

    private void OnVariableChanged()
    {
        OnPropertyChanged(nameof(SummaryText));
    }

    private Dictionary<string, object?> CurrentValues()
    {
        var values = new Dictionary<string, object?>(StringComparer.Ordinal);
        foreach (var variable in Variables)
        {
            if (variable.TryBuildValue(out var value))
            {
                values[variable.Name] = value;
            }
        }

        return values;
    }
}

/// <summary>start 节点上的一条变量声明；与 core 的 WorkflowVariableDecl 同构。</summary>
public sealed record WorkflowVariableDeclaration(
    string Name,
    string Kind,
    object? Default,
    bool Required,
    bool Hidden);
