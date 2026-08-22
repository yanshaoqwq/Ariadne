using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Text.Json;
using System.Threading.Tasks;
using Ariadne.Desktop.Backend;

namespace Ariadne.Desktop.ViewModels;

/// <summary>
/// U206-B：跨章知识查询面板 —— 给 `@知识:<关键词>` 引用补上前端入口。
///
/// # 这个面板要解决的具体问题
///
/// 写到第 40 章，作者想问「阿青的性格是在哪一章定下的」。这个能力后端**早就齐备**：
/// `resolve_project_reference` 的 `知识` 前缀会扫全部 10 个 `FindScope`
/// （角色档案 / 角色规划 / 性格路径 / 关系路径 / 事件分段 / 分段正文 / 伏笔 /
/// 主题锚点 / 章节摘要 / 阶段摘要），命中后返回 `source`（出处，也就是「哪一章」）、
/// `title`、`spans`、`text`。IPC 也通、前端边界 `ResolveProjectReferenceAsync` 也实现了
/// —— 唯独 ViewModel/View 里**一个调用点都没有**，于是整个应用没有任何入口能发起这个查询。
///
/// 全前端此前发出去的引用只有一种：审阅面板硬编码的 `@确认项:<id>`（U139④）。
/// 6 种前缀里 5 种从未被用过。本面板接的是其中最贵的一种（知识库是百万字项目
/// 唯一能回答「这个设定最早在哪定的」的地方）。
///
/// # 为什么直接调 resolve_project_reference，而不是把问题丢给项目 AI
///
/// 「在哪一章定下的」这个问题的答案**就在 payload.source 里**，是一次本地检索，
/// 零 token、零等待、零预算记账。走 LLM 反而更差：要花钱、要等，而且模型只能
/// 复述我们本来就要传给它的那段检索结果 —— 中间多一道转述只会多一处失真。
///
/// 面板同时给出「问 AI」出口（`RequestAskAi`），用于「查到了，但想让 AI 就这段设定
/// 做一致性判断」。那条路复用既有 `project_ai_chat` + `references`，
/// 也就是把同一个引用串交给后端展开 —— 引用式数据流，不把 text 拼进 message。
///
/// # 为什么单独一个 VM 而不是往 WorkspacePageViewModel 里加
///
/// 与 <see cref="VariableFillPanelViewModel"/> 同一个理由：这几条状态
/// （关键词 / 查询中 / 上次结果）彼此闭合，不依赖画布、运行态、确认项中的任何一个。
/// 塞进那个 6600 行的页面 VM 只会让它更难读，而这里没有任何东西需要页面上下文。
/// </summary>
public sealed class KnowledgeLookupPanelViewModel : ViewModelBase
{
    /// <summary>引用前缀。与后端 `frontend/service.rs` 的 `"知识"` 分派臂同值。</summary>
    public const string ReferencePrefix = "@知识:";

    private readonly Func<string, string> _text;
    private readonly Action<string> _report;
    private readonly Func<Exception, string> _describeError;
    private string _term = string.Empty;
    private bool _isLooking;
    private bool _isPanelOpen;
    private ProjectReference? _result;
    private string _resultSource = string.Empty;
    private string _resultTitle = string.Empty;
    private string _resultText = string.Empty;

    public KnowledgeLookupPanelViewModel(
        Func<string, string> text,
        Action<string> report,
        Func<Exception, string> describeError)
    {
        _text = text;
        _report = report;
        _describeError = describeError;
        LookupCommand = new RelayCommand(() => _ = LookupAsync(), CanLookup);
        AskAiCommand = new RelayCommand(() => _ = AskAiAsync(), CanAskAi);
        TogglePanelCommand = new RelayCommand(() => IsPanelOpen = !IsPanelOpen);
        ClosePanelCommand = new RelayCommand(() => IsPanelOpen = false);
    }

    /// <summary>
    /// 折叠/展开。
    ///
    /// U213-C：这个面板原先**常驻**在对话流之上（右栏 Row 0），六行竖排
    /// （标题 / 说明 / 输入框 / 示例 / 查询键 / 结果区）。用户否掉的不只是
    /// 「太占地」，而是**位置本身**：「不应该塞在顶端，应该在输入框下面
    /// 悬浮着做工具栏」。
    ///
    /// ⇒ 现在折叠态是输入框下方工具栏里的一个搜索小图标，展开态是向上弹的浮层。
    ///
    /// ⚠️ **不要因此把它塞回对话流的 ScrollViewer 里**。原注释里那条理由仍然成立：
    /// 聊了十几轮之后要先滚回顶部才能查一次设定，而「这个设定当初在哪定的」
    /// 恰恰是对话进行中最常要查的东西。搬到底部工具栏**同时**满足了
    /// 「不随对话滚动」（它在 ScrollViewer 外）与「不占顶端」——
    /// 这是本方案比「留在顶端只做折叠」更好的地方。
    /// </summary>
    public bool IsPanelOpen
    {
        get => _isPanelOpen;
        set => SetProperty(ref _isPanelOpen, value);
    }

    /// <summary>发起 `resolve_project_reference` 的通道，由页面 VM 注入（这里够不到后端客户端）。</summary>
    public Func<string, Task<ProjectReference>>? RequestLookup { get; set; }

    /// <summary>把查到的引用串交给项目 AI；同样由页面 VM 注入。</summary>
    public Func<string, Task>? RequestAskAi { get; set; }

    public RelayCommand LookupCommand { get; }
    public RelayCommand AskAiCommand { get; }

    /// <summary>工具栏上那个搜索小图标：开合浮层。</summary>
    public RelayCommand TogglePanelCommand { get; }

    /// <summary>浮层内的关闭键。开合各有独立命令而不是共用一个 Toggle：
    /// 关闭键的语义是「关」，绑 Toggle 会让它在某些路径下反而把面板开回来。</summary>
    public RelayCommand ClosePanelCommand { get; }

    /// <summary>
    /// 查过的关键词，供输入框做候选。
    ///
    /// 候选源刻意是**历史查询词**而不是「知识库全部实体名」：后端没有任何
    /// 列举实体的命令（前端能调的 90 个方法里没有一个 knowledge list），
    /// 硬造一条新 IPC 只为填下拉框不成比例。而作者的实际用法是反复查同几个角色，
    /// 历史词已经覆盖了绝大部分重复输入。
    /// </summary>
    public ObservableCollection<string> TermCandidates { get; } = new();

    /// <summary>要查的关键词。后端做的是模糊匹配，所以「阿青」这类自然词就够。</summary>
    public string Term
    {
        get => _term;
        set
        {
            if (SetProperty(ref _term, value))
            {
                LookupCommand.NotifyCanExecuteChanged();
            }
        }
    }

    public bool IsLooking
    {
        get => _isLooking;
        private set
        {
            if (SetProperty(ref _isLooking, value))
            {
                LookupCommand.NotifyCanExecuteChanged();
                AskAiCommand.NotifyCanExecuteChanged();
                OnPropertyChanged(nameof(LookingText));
            }
        }
    }

    /// <summary>上次命中的引用。null = 还没查过或没命中，此时结果区整块不渲染。</summary>
    public ProjectReference? Result => _result;

    public bool HasResult => _result is not null;

    /// <summary>出处 —— 也就是作者问的「在哪一章定下的」那个答案。</summary>
    public string ResultSource => _resultSource;

    public string ResultTitle => _resultTitle;

    /// <summary>命中条目的正文片段；知识库条目通常只有一两句，直接摊开显示。</summary>
    public string ResultText => _resultText;

    public bool HasResultText => !string.IsNullOrWhiteSpace(_resultText);

    public string TitleText => _text("ui.workspace.knowledge_lookup.title");
    /// <summary>
    /// 说明文案的新归属：**tooltip**，不是浮层里的一行字（U213-C）。
    ///
    /// # 为什么不合进输入框水印（我先做了这个，被实机截图否掉）
    ///
    /// 第一版把 `PlaceholderText` 与 `HintText` 拼成一行当水印
    /// （`"例如：阿青 — 输入人物、地点或设定关键词，直接查它在知识库里的出处，比如……"`）。
    /// 渲染出来是**一行截断的长句**：水印不折行，后半截直接看不见——
    /// 既没起到说明作用，又正是用户说的「做得丑」。
    /// ⇒ 水印只放短示例词（`PlaceholderText`），这段解释挂 tooltip：
    /// 需要的人 hover 就有，不需要的人不必每次读一遍，而且**不占任何版面**。
    ///
    /// 这条是「视觉改动必须真的渲染出来看」的又一个实例：
    /// 三条布局守卫全绿，而水印被截断这件事只有截图能看见。
    /// </summary>
    public string HintText => _text("ui.workspace.knowledge_lookup.hint");
    public string PlaceholderText => _text("ui.workspace.knowledge_lookup.placeholder");
    public string SourceLabelText => _text("ui.workspace.knowledge_lookup.source");
    public string AskAiText => _text("ui.workspace.knowledge_lookup.ask_ai");
    public string CloseText => _text("ui.common.close");


    /// <summary>
    /// 折叠态那颗小图标的可及名称 = **动作**（「查出处」）。
    ///
    /// U213-C 删掉满宽查询按钮后，这个 key 差点变成「代码里还引用着、界面上
    /// 永远不显示」的半死键：原 `LookupButtonText` 有两支，而查询中那一支
    /// 才是唯一会露面的文案 ⇒ 「查出处」这一支不再有任何呈现位置。
    /// 图标按钮本来就需要一个描述动作的可及名称（图标本身对读屏软件不可读），
    /// 两个需求正好互补，于是它在这里落脚而不是被删。
    /// </summary>
    public string ActionText => _text("ui.workspace.knowledge_lookup.lookup");

    /// <summary>
    /// 查询中的状态文案。
    ///
    /// 只有这一支，**没有「空闲时显示什么」的对偶**：满宽查询按钮已删（U213-C），
    /// 这行字只在等结果的那几秒出现。原先它是按钮文案的一支
    /// （「查询中换文案而不是只灰掉」——灰按钮说明「不能点」，说不出「已经在查了」），
    /// 按钮没了以后同一条理由落在这行独立状态字上：
    /// 按下 Enter 后浮层几秒钟毫无变化会被读作「没反应」。
    /// </summary>
    public string LookingText => _text("ui.workspace.knowledge_lookup.looking");

    /// <summary>语言切换后刷新本面板的静态文案（页面 VM 统一转发）。</summary>
    public void RefreshLocalizedText()
    {
        OnPropertyChanged(nameof(TitleText));
        OnPropertyChanged(nameof(HintText));
        OnPropertyChanged(nameof(PlaceholderText));
        OnPropertyChanged(nameof(CloseText));
        OnPropertyChanged(nameof(SourceLabelText));
        OnPropertyChanged(nameof(AskAiText));
        OnPropertyChanged(nameof(ActionText));
        OnPropertyChanged(nameof(LookingText));
    }

    /// <summary>关键词为空或正在查时不可点，而不是点了发一次注定被后端拒的空引用。</summary>
    private bool CanLookup() => !_isLooking
        && RequestLookup is not null
        && !string.IsNullOrWhiteSpace(_term);

    private bool CanAskAi() => !_isLooking && RequestAskAi is not null && _result is not null;

    /// <summary>
    /// 拼引用串。
    ///
    /// **前缀不是装饰**：后端顶层 `parse_project_reference` 要求引用里含 ':' 或 '/'，
    /// 裸关键词会被判成非法引用（"project reference must contain ':' or '/'"）而不是
    /// 「查不到」——用户会看见一条完全不相干的错误。U139④ 在确认项那条路上踩过同一处。
    /// </summary>
    public static string ComposeReference(string term) => ReferencePrefix + term.Trim();

    private async Task LookupAsync()
    {
        var lookup = RequestLookup;
        var term = _term.Trim();
        if (lookup is null || string.IsNullOrEmpty(term))
        {
            return;
        }

        IsLooking = true;
        try
        {
            var reference = await lookup(ComposeReference(term)).ConfigureAwait(true);
            ApplyResult(reference);
            RememberTerm(term);
            _report(_text("ui.workspace.knowledge_lookup.found"));
        }
        catch (Exception ex)
        {
            // 查不到走的是后端 validation 错误（"knowledge item not found: X"）。
            // 清掉上一次结果：留着会让人以为这次也命中了，只是内容没变。
            ClearResult();
            _report(_describeError(ex));
        }
        finally
        {
            IsLooking = false;
            AskAiCommand.NotifyCanExecuteChanged();
        }
    }

    private async Task AskAiAsync()
    {
        var askAi = RequestAskAi;
        var result = _result;
        if (askAi is null || result is null)
        {
            return;
        }

        // 传**上次实际命中的那个引用串**而不是重新拼当前输入框：
        // 用户可能查到结果后又在框里敲了别的字，那时按原样重拼会把 AI 引到
        // 一个还没查过的关键词上，而结果区显示的仍是旧命中。
        await askAi(result.Reference).ConfigureAwait(true);
    }

    private void ApplyResult(ProjectReference reference)
    {
        _result = reference;
        // 出处 / 标题 / 正文都在 payload 里（后端 resolve_knowledge 组装的 FindResult 投影），
        // 顶层只有 summary（= snippet）。payload 缺字段时留空而不是显示 "null"。
        _resultSource = ReadPayloadString(reference.Payload, "source");
        _resultTitle = ReadPayloadString(reference.Payload, "title");
        _resultText = ReadPayloadString(reference.Payload, "text");
        if (string.IsNullOrWhiteSpace(_resultTitle))
        {
            _resultTitle = reference.Summary;
        }

        NotifyResultChanged();
    }

    private void ClearResult()
    {
        _result = null;
        _resultSource = string.Empty;
        _resultTitle = string.Empty;
        _resultText = string.Empty;
        NotifyResultChanged();
    }

    private void NotifyResultChanged()
    {
        OnPropertyChanged(nameof(Result));
        OnPropertyChanged(nameof(HasResult));
        OnPropertyChanged(nameof(ResultSource));
        OnPropertyChanged(nameof(ResultTitle));
        OnPropertyChanged(nameof(ResultText));
        OnPropertyChanged(nameof(HasResultText));
        AskAiCommand.NotifyCanExecuteChanged();
    }

    /// <summary>最近查过的词排在最前：作者的实际用法是反复查同几个角色。</summary>
    private void RememberTerm(string term)
    {
        // merged 是 Compose 拷出来的新 List，所以 Sync 内部的 Clear 不会反过来清空它。
        var merged = IdentifierCandidates.Compose(new string?[] { term }, TermCandidates);
        IdentifierCandidates.Sync(TermCandidates, merged);
    }

    /// <summary>
    /// 从 payload 里取一个字符串字段。
    ///
    /// payload 声明成 `object?`，走 `JsonSerializerDefaults.Web` 反序列化后实际是
    /// <see cref="JsonElement"/>；但测试里会直接塞匿名对象或字典，所以两种都认。
    /// 取不到一律返回空串——这里是展示路径，缺字段不该抛。
    /// </summary>
    private static string ReadPayloadString(object? payload, string field)
    {
        switch (payload)
        {
            case JsonElement element when element.ValueKind == JsonValueKind.Object:
                return element.TryGetProperty(field, out var value)
                    && value.ValueKind == JsonValueKind.String
                    ? value.GetString() ?? string.Empty
                    : string.Empty;
            case IReadOnlyDictionary<string, object?> map:
                return map.TryGetValue(field, out var raw) ? raw?.ToString() ?? string.Empty : string.Empty;
            default:
                return string.Empty;
        }
    }
}
