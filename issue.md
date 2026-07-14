##当前已知问题

> 2026-07-11 当前源码复核：本文件前半部分包含大量旧状态，不能再按原“确认存在”直接执行或删除。逐项结论如下（证据均来自当前工作树）：
>
> - 已解决：L2（`LocalFileSecretStore` 加密持久化及重载测试）、L4（`ConfirmationPolicySetting` 已只保留 normal/auto 两字段）、L5（数据边空 alias 现直接返回校验错误）、L6（start node 生成 schema 并传入工具定义）、L7（存在/目录/`.config` 分层校验）、L8（仅 http/https，拒绝 userinfo、私网/本机地址并有测试）、U1（标题栏已无 Auto Mode）、U2（`DialogScrim Grid.Row="1"`）、U3（缩放与可交互 MiniMap）、U4（`StatusIndicatorBrush`）、U5（右栏 `GridSplitter`）、U7（Undo 命令、快照栈和 Ctrl+Z）、U8（画布确认横幅/完整面板）、A4（`HeaderStatusText` 已拆分通知与后端状态）、A6（百分比属性，0 预算显示 unlimited）。
> - 部分解决：L1（公开 `run_workflow` 已是后台 `start_workflow` 别名，但 `project_ai_chat_impl` 注入的工具 runner 仍直接调用同步 `run_workflow_impl`）、U6（仍可见多处 `CurrentPage = item.PageFactory()`，导航状态丢失风险仍在）、A3（画布能力显著增强，但边配置是否完全摆脱 JSON/模板手填仍需交互验收）。
> - 仍存在：A1（EN/JA 资源仍是零翻译 stub）、A7（欢迎页仍未绑定 `IsLoading`/`StatusText`）、A8（模板空态和“加载更多”仍无状态可见性/可用性绑定）、A9（Git 页仍为 Canvas + 无尺寸 ScrollViewer + 780 固定宽）、A10（日志页仍只筛级别/关键词且不展示来源字段）、A11（外层 ScrollViewer + ItemsControl，无分页/虚拟化）、A12（分块 TextBox 方案需保留为未解决）。A5 的旧抑制标志实现已不存在，判为已解决。
> - 旧描述失效：A2（已由 A12 替代）；旧清单 #3/#6/#7/#11 保持已修复；#9/#10 现在也应从“部分修复”改为已解决；旧 #5“工具默认放行”已被 `tool_control_enabled(...).unwrap_or(false)` 修复；旧 #12/#14/#15/#17/#18/#19/#20/#21/#22/#24 已修复。
> - 尚不能删除：本文件至少仍承载 U6、A1、A7-A12 等有效未解决项；应在这些项修复并通过测试/人工交互验收后再删除。

### 2026-07-11 未解决项迁移完成

本文件中当前仍有效、尚未实现的问题已经全部迁移到 [发布前全量代码审查总索引](项目检验报告/发布前全量代码审查/README.md)：

| 本文件旧条目 | 主审查报告编号 |
|---|---|
| L1 同步运行残留 | N21、C9 |
| U6 页面 ViewModel 重建 | U39、C7 |
| A1 EN/JA 空壳 | U40 |
| A3 边/模板配置交互残留 | W1-W5 |
| A7 欢迎页状态不可见 | U30 |
| A8 模板状态机缺失 | U31、N20 |
| A9 Git Canvas 布局 | U32 |
| A10 日志来源与筛选缺失 | U33 |
| A11 日志无分页/虚拟化 | C20 |
| A12 分块 TextBox 连续编辑问题 | U34 |

A5 的 `_suppressAutoModeSave` 旧实现已不存在，判为已解决，不迁移。自此以后本文件只作为历史证据，活动问题以拆分后的审查目录为唯一账本；待对应项全部验证关闭后，本文件可删除。

1. 严重程度汇总
级别	数量	代表问题
🔴 严重（阻断/数据丢失/安全）	4	IPC 同步阻塞致前端假死、密钥不持久化、EN/JA 国际化为空壳、预算 0.0 footgun
🟠 重要（体验/正确性）	7	导航重建 ViewModel、画布无 Undo、确认项入口隐藏、节点无状态色、手写 JSON 配置
🟡 一般	6	画布无缩放/小地图、右栏不可调宽、DialogScrim 盖标题栏、BackendStatus 语义过载
🟢 已修复/失效	5	旧 Issue #3/#6/#7/#11 已修；#9/#10 部分修
2. 逻辑问题（后端 Rust）
L1 🔴 工作流执行同步阻塞 IPC 主线程，运行期间前端完全假死
状态：确认存在 ｜ core/src/ipc.rs（run_json_line_stdio + dispatch_request） / core/src/workflow/runtime.rs（run_inner）

run_json_line_stdio 是一个 for line in stdin.lock().lines() 同步循环，run_workflow 命令在循环内内联调用 runtime.run_persisted → run_inner，而 run_inner 是一个同步 loop{} 里逐节点 executor.execute()（含 LLM HTTP 调用、文档读写、多轮通信）。

后果（已验证链路）：

工作流运行期间，整个进程卡在读“下一条 stdin”之前的同步执行里，期间无法响应任何其他 IPC 请求。
前端发出的 pause_workflow / stop_workflow / get_workflow_run_state / 确认项审批，全部排在 stdin 缓冲区里直到本次运行结束才被处理 → “停止运行”按钮形同虚设，确认项无法实时审批。
这是产品级的“假死”，不是偶发卡顿。
建议：将执行移入独立线程/tokio::task，IPC 只返回 run_id；前端用轮询或事件流取状态；引入 CancellationToken 支持中途打断。

L2 🔴 非 keychain 构建下密钥完全不持久化，重启即丢
状态：确认存在（高度可能） ｜ core/src/config/secrets.rs / core/src/commands.rs

MemorySecretStore 用内存 BTreeMap<RwLock> 存密钥；default_for_process() → default_secret_store() 在未启用 system-keychain feature 时返回它。Linux 桌面大多无可用 keyring 服务，于是每次重启都要重输所有 API Key。这对“首次使用信任度”是毁灭性的。
（注：SystemKeychainSecretStore 分支本身实现是好的，见 L3。）

建议：提供“本地文件 + OS 密钥派生/主密码”降级持久化方案，而非裸内存。

L3 🔴 预算 0.0 footgun + “显示预算”与“执行预算”是两套不一致的数值
状态：确认存在 ｜ core/src/costs/budget.rs / core/src/commands.rs / MainWindowViewModel.ApplyBudgetStatus

BudgetConfigFile::default() = budget_usd: 0.0（这是 UI 上显示的“预算”）。
但真正执行的是 BudgetLimits，其 single/daily/monthly_usd 默认 None；exceeds(None, _) 恒为 false。
结果：UI 预算条显示 $0.0/$0.0（看着像“没钱不能用”），实际却默认可调用（单次 >$1 才需确认）。
反向 footgun：用户若把“日/月限额/预授权额度”手动填成 0.0（误以为 0=无限制），exceeds(Some(0.0), 任意正数)=true，瞬间封死所有 LLM 调用。
Rust

fn exceeds(limit: Option<f64>, value: f64) -> bool { limit.is_some_and(|l| value > l) }
建议：UI 在 budget_usd==0 时显示“未设置限额（默认无限制）”；validate_money 把限额字段的 0.0 解释为 None，或明确标注“0=禁止”。

L4 🟠 确认策略存在 legacy policy ↔ 双策略(normal/auto_mode) 的有损双表示
状态：确认存在 ｜ core/src/commands.rs（ConfirmationPolicySetting）

ConfirmationPolicySetting 同时序列化了 normal_policy、auto_mode_policy 和 一个遗留的 policy: String 字段。旧 Issue #8 指出二者双向映射会丢信息（AllowByDefault+AutoApproval 与 ManualReview+AutoApproval 都被压成同一 legacy 串）。当前结构里这个冗余字段仍然存在，意味着保存→读取仍可能静默篡改审批策略。

建议：彻底废弃 policy 字段，只保留双字段序列化，删除两个转换函数。

L5 🟡 数据边缺少 alias 时，警告只打到 stderr，运行日志页看不到
状态：部分修复（🟡） ｜ core/src/workflow/runtime.rs（collect_data_inputs）

旧 Issue #9 说“无 alias 静默跳过、节点拿不到输入却无报错”。当前代码已加了警告，但写的是：

Rust

eprintln!("[ariadne] warning: data edge {} to node {} has no alias; input will be skipped", ...);
问题：eprintln! 只进后端进程的 stderr，前端“运行日志”页读的是 structured_events，用户在 UI 里依然看不到这条警告 → “排错极困难”的体验问题并未真正解决。

建议：把该警告作为 WorkflowRuntimeEvent（如新增 NodeInputSkipped 类型）写入 structured_events，让 RunLog 页能展示；并在 validate_topology 阶段对数据边要求必须有 alias（治本）。

L6 🟠 项目空间 AI 工具 input_schema 此前为空对象（正在修复）
状态：部分修复（🟡） ｜ core/src/commands.rs（ProjectWorkflowTool）

旧 Issue #10 指出所有项目 AI 工具的 input_schema 都是 {"type":"object","properties":{},"additionalProperties":false}，导致 start 节点参数无法传给 LLM。当前 ProjectWorkflowTool 结构体已新增 input_schema: Value 字段（提交 补齐审计分支工作流工具），说明正在修。需确认 project_ai_tool_definitions 是否真的从 start 节点 ports 生成 schema，否则字段仍可能是空对象。

L7 🟡 validate_project_root 校验过弱
状态：确认存在（高度可能） ｜ core/src/commands.rs

set_project_root 调用了 validate_project_root，但旧 Issue #12 指出它仅判断路径非空，不校验“目录是否存在 / 是目录而非文件 / 是否已初始化（有无 .config/）”。传入不存在的路径会静默当成新项目。本轮确认该函数仍被调用且未见增强。建议补充存在性/类型/初始化状态校验。

L8 🟡 模板仓库 URL 协议不校验（潜在 SSRF）
状态：未在本轮独立复核 ｜ core/src/commands.rs

旧 Issue #14：base_url 仅校验非空，不校验 http/https，可被注入 file:///、http://169.254.169.254/...。该函数在 commands.rs 深处，本轮未直接读到，但因默认值已改为空串（见第四节），触发面收窄。建议保留复核并加 scheme 白名单校验。

3. UI 设计问题（Avalonia / desktop）
U1 🔴 标题栏放 Auto Mode 总开关——高风险操作紧贴窗口控制按钮
状态：确认存在 ｜ MainWindow.axaml

Auto Mode 开启后会自动审批所有写回正文操作，属高风险。但其 ToggleSwitch 放在自定义标题栏（Grid.Row="0"）的“预算条 + Auto Mode”区，与最小化/最大化/关闭按钮同一行、紧邻，且无二次确认。鼠标点窗口控件时极易误触开启。

建议：移入“自动化”设置页；或保留但加二次确认弹窗 + 风险文案。

U2 🟡 DialogScrim 遮罩盖住整个标题栏，弹窗期间无法最小化/移动窗口
状态：确认存在（且是有意为之） ｜ MainWindow.axaml

DialogScrim 放在最外层 Panel，源码注释直书：半透明 scrim 覆盖整个客户区（含标题栏）；点击遮罩=取消。由于窗口是无原生边框的自绘标题栏（WindowDecorations="None"），任何弹窗显示时，最小化/最大化/移动/关闭全部失效。开发者把“点遮罩取消”当特性，但牺牲了基本窗口可操作性。

建议：scrim 只覆盖内容区（Grid.Row="1"），标题栏（Row="0"）保持可交互。

U3 🟢 画布缺缩放控件与小地图，大工作流难导航
状态：确认存在 ｜ WorkspacePageView.axaml

画布顶部浮动工具栏只有：导入/导出/保存（图标）+ 暂停/停止/恢复（文字）。“适应视图”只藏在右键菜单里；无缩放滑块、无百分比、无 minimimap。20+ 节点时定位困难。

U4 🟠 节点卡片无运行状态颜色/图标编码
状态：确认存在 ｜ WorkspacePageView.axaml

节点 Border 的 BorderBrush 绑定的是静态资源 Ariadne.NodeBorder，没有绑定运行状态；节点内只有一行 TextBlock Text="{Binding StatusText}"（纯文字）。10+ 节点时无法一眼看出“哪个在跑/哪个失败/哪个暂停”。

建议：边框/角标按 running(蓝脉冲)/succeeded(绿)/failed(红)/paused(橙) 着色。

U5 🟡 右侧配置面板固定宽度、不可拖拽调宽
状态：确认存在 ｜ WorkspacePageView.axaml

布局为 Grid ColumnDefinitions="*,Auto"，右栏是 Auto 固定宽，仅提供“折叠/展开”浮动按钮（panel-float），没有 GridSplitter。而该栏要塞下：提示词模板、模型 ID、预算、超时、断点，以及整组通信边配置（forward/reverse alias、模板、次数）和数据边 JSON。窄列 + 大量滚动。

U6 🟠 导航切换每次 new 全新 ViewModel，页面状态全丢
状态：确认存在 ｜ MainWindowViewModel.cs（CreatePage / SelectNavigationItemAsync）

csharp

CurrentPage = item.PageFactory();   // PageFactory 每次 new 一个新 VM
无任何缓存。在“工作空间”里编辑了提示词没保存、在“日志”页滚动到某处、在“设置”里改了一半表单——切走再切回，全部丢失。配合画布无 Undo（见 U8），误操作代价极高。

建议：用 Dictionary<string,object> 缓存 VM，仅在切换项目时清空。

U7 🔴 画布操作无 Undo/Redo，误删不可逆
状态：确认存在 ｜ WorkspacePageView.axaml（右键菜单）

画布右键菜单只有：添加/复制/剪切/粘贴/适应视图/删除。无 Undo 入口，代码里也未见 undo 栈。精心配置的节点被误删后只能手工重建。

建议：Command Pattern + undo/redo 栈，Ctrl+Z/Ctrl+Shift+Z，工具栏按钮。

U8 🟠 待审确认项入口埋在右侧面板深处
状态：确认存在 ｜ WorkspacePageView.axaml

确认项嵌在“右栏 → 项目 AI 标签页 → 下方列表”。侧栏 badge 只显示数字，画布上没有任何“有待审项”的视觉提示。新用户根本不知道有东西等着批。

4. 反人类 / 可用性问题
A1 🆕🔴 英文/日文界面是空壳——README 虚假宣传“三语支持”
状态：新发现·确认存在 ｜ core/resources/display_name.en.json、display_name.ja.json

中文主文件 display_name.json = 37 KB。
display_name.en.json 全文：
JSON

{ "_comment": "English translation overlay ... fall back to zh ...",
  "_status": "stub — pending translation" }
display_name.ja.json 同理："_status": "スタブ — 翻訳待ち"。
零条实际翻译。README 却写“应用支持中文、英文和日文界面。可以在‘配置 > 杂项 > 语言’中切换”。用户切到 EN/JA 后，整个界面回退成中文（或裸 key）。这是对目标用户的直接误导。

建议：在 README/语言下拉里明确标注 EN/JA 为“未完成/预览”；或先移除这两个选项直到翻译就绪。

A2 🟢 已失效：作品正文已不再使用单个 TextBox
状态：旧结论已删除 ｜ 当前问题由 A12“存储分块直接暴露为多个独立 TextBox”替代

当前实现已经改为分块编辑，因此“整章塞进单个 TextBox”的原证据不再成立。新的真实问题不是缺少分块，而是把内部存储分块直接暴露成多个互不连续的编辑控件，破坏跨块选择、边界退格、光标和滚动稳定性；详见 A12。

A3 🆕🟠 数据边/通信边配置要求小说作者手写 JSON 与模板占位符
状态：新发现·确认存在 ｜ WorkspacePageView.axaml（节点详情/边配置区）

数据边 payload：TextBox Text="{Binding SelectedEdge.DataJson, Mode=TwoWay}"（MinHeight="90"）——让用户手写 JSON。
通信边：要求填 forward/reverse alias、forward/reverse template（含 {{input.xxx}} 占位符）、max communication count。
目标用户是小说作者，却被要求在窄文本框里手写 JSON 和模板语法。排错几乎不可能，属典型反人类设计。

建议：JSON 用结构化表单/键值对编辑器；模板用“变量插入”按钮 + 实时预览；提供可视化校验。

A4 🆕🟡 BackendStatus 字段语义严重过载
状态：新发现·确认存在 ｜ MainWindowViewModel.cs

同一个 BackendStatus 字符串属性，既显示“项目标题”、又被 ShowVersionAsync 写成版本号、被 ShowFeedbackAsync 写成反馈文案、被 SetAutoModeAsync 的 catch 写成后端异常原文。用户在状态栏一会看到项目名、一会看到版本号、一会看到一段英文报错，含义反复横跳。

建议：拆分为独立字段（项目标题/连接状态/临时通知/错误），各自绑定。

A5 🆕🟡 Auto Mode 异常回滚用 _suppressAutoModeSave 布尔标志位，脆弱
状态：新发现·确认存在 ｜ MainWindowViewModel.cs

AutoModeEnabled 的 setter 在 _suppressAutoModeSave==false 时触发异步 SetAutoModeAsync；失败时又置 _suppressAutoModeSave=true 回拨开关再置回 false。这种“标志位抑制自身回调”的模式容易在并发/快速点击下漏抑制或重入，是隐性 bug 温床。建议改用显式命令 + 结果驱动 UI（服务端状态回灌）。

A6 🟢 预算条魔法宽度 * 92 + $0.0/$0.0 空进度条
状态：确认存在 ｜ MainWindowViewModel.ApplyBudgetStatus

BudgetUsageWidth = total * 92;（92px 硬编码条宽），且 budget_usd<=0 时 total=0 → 空条 + $0.0/$0.0，与 L3 叠加，进一步强化“没钱不能用”的错觉。

5. 对仓库已有 Ariadne_issues.md 的勘误（高价值）
旧清单共 25 条，以下按当前 main 代码复核结果分类。直接套用旧清单会误报。

🟢 已修复 / 失效（建议从 Issue 清单移除或关闭）
旧 Issue	旧描述	当前真相
#3 模板 URL templates.ariadne.local 不可达	默认指向 mDNS .local	DEFAULT_TEMPLATE_REPOSITORY_URL = ""（空串），TemplateRepositorySettings 默认 base_url=""。不可达域名已移除。
#6 advance_loop 不重置 communication 边	返修循环静默失效	advance_loop 现在对每个 rerun target 调 collect_control_closure 清节点后，显式调用 reset_communication_edges_for_nodes 重置 completed/message_count/messages。已修复。
#7 resume_from_node 不重置 loop_iterations	路径 A 恢复后立即再暂停	现在同时 loop_iterations.remove(downstream) 和 reset_communication_edges_for_nodes。注释还把旧 bug 描述原样写进了代码。已修复。
#11 keychain service 名仍为 literature-agent	与项目名不符	SystemKeychainSecretStore::default() 现为 Self::new("ariadne")，并注明旧名迁移。已修复。
🟡 部分修复（仍有残留，建议保留并更新描述）
旧 Issue	当前状态
#9 数据边无 alias 静默跳过	已加 eprintln! 警告，但未进 structured_events，UI 运行日志页仍看不到（见 L5）。
#10 项目 AI 工具 input_schema 为空	ProjectWorkflowTool 已新增 input_schema 字段；需确认是否真从 start 节点 ports 生成（见 L6）。
🔴 仍确认存在（旧清单正确，当前代码未动）
#1(密钥不持久化, 见 L2)、#2(IPC 同步阻塞, 见 L1)、#5(工具默认放行, 未独立复核)、#8(策略双表示, 见 L4)、#12(project_root 校验弱, 见 L7)、#13(IPC 明文)、#14(URL SSRF, 见 L8)、#15(预算, 见 L3)、#16(无 VM 缓存, 见 U6)、#17(无 Undo, 见 U7)、#18(确认项隐藏, 见 U8)、#19(Auto Mode 标题栏, 见 U1)、#20(无缩放/小地图, 见 U3)、#21(右栏不可调宽, 见 U5)、#22(节点无状态色, 见 U4)、#24(scrim 盖标题栏, 见 U2)、#25(Vec 去重 O(n²))。

##额外问题：
设置页面一些设置的填写逻辑反人类，且无法有效填写

A7 🆕🟠 欢迎页把加载、创建和打开项目的状态全部写入不可见属性，失败后界面可能只显示“还没有最近项目”
状态：新发现·确认存在 ｜ desktop/Ariadne.Desktop/ViewModels/WelcomeViewModel.cs:100-139,149-205,208-249；desktop/Ariadne.Desktop/Views/WelcomeView.axaml:36-247

`WelcomeViewModel` 在最近项目加载、创建项目和打开项目期间维护 `IsLoading`，并把取消、校验失败和后端异常写入 `StatusText`；但 `WelcomeView.axaml` 没有任何控件绑定 `IsLoading` 或 `StatusText`。最近项目读取失败时，集合仍为空，页面继续显示普通的“还没有最近项目”空态；创建/打开项目后端失败时，用户也看不到失败原因或忙碌反馈，按钮仍可继续点击。

用户场景：最近项目文件损坏或后端暂时不可用时，作者看到的是“还没有最近项目”，会误以为历史记录被清空；点击“新建项目”后请求失败，页面无可见提示，用户可能反复点击并触发并发流程。

建议：为欢迎页增加明确的 loading/error/empty 三态；忙碌时禁用新建、打开和最近项目项；错误态提供重试与诊断入口，不能退化成正常空态。

A8 🆕🟠 模板集市没有结果状态机：有结果时仍显示空态插画，“加载更多”在初始、失败和末页都永久可点
状态：新发现·确认存在 ｜ desktop/Ariadne.Desktop/Views/TemplateMarketPageView.axaml:56-95；desktop/Ariadne.Desktop/ViewModels/TemplateMarketPageViewModel.cs:101-139

模板卡片列表、空态 `Border` 和“加载更多”按钮都没有任何互斥可见性或可用性绑定。搜索成功且已经展示模板时，页面下方仍固定出现空态插画，只是把其中的文字改成模板数量；尚未搜索、仓库未配置、请求失败和确实没有结果也共用同一个区域。“加载更多”则不判断是否已有首屏结果、是否正在加载、是否到达末页或上次请求是否失败。

用户场景：作者搜到 6 个模板后，列表底部仍出现一幅代表“没有内容”的大插画和数字“6”，视觉语义自相矛盾；首次进入页面即可点击“加载更多”，失败后也能连续点击，用户无法判断系统是在加载、已结束还是出错。

建议：建立 idle/loading/results/empty/error/end-of-list 六态；空态仅在请求成功且结果为零时显示；加载更多仅在存在下一页且非忙碌时可用，并显示进度和重试。

A9 🆕🟠 Git 历史把 ScrollViewer 绝对定位在 Canvas 中且不给可用宽高，长历史可能被画布裁掉而不是形成可滚动视口
状态：新发现·确认存在 ｜ desktop/Ariadne.Desktop/Views/GitPageView.axaml:8-23,61-89；desktop/Ariadne.Desktop/Views/GitPageView.axaml.cs:38-54

Git 主区使用 `Canvas`，历史 `ScrollViewer` 只设置 `Canvas.Left/Top`，没有绑定画布剩余宽度和高度；其内部又固定为 `Width="780"`。Canvas 会按子元素期望尺寸布局而不把它约束到窗口剩余空间，外层还设置了 `ClipToBounds="True"`。代码只计算右侧开合 pill 的位置，没有为历史视口补尺寸。

用户场景：在最小窗口或展开 300px 详情栏时，780px 历史内容必然要求横向滚动；存档较多时，ScrollViewer 可能按全部内容高度展开后被 Canvas 底部裁切，用户看不到下方存档，也得不到正常的窗口内纵向滚动范围。

建议：主区改用 Grid（标题行 + 状态行 + `*` 历史区），ScrollViewer 直接 Stretch 占满剩余区域；移除 780px 固定宽度，让存档卡片响应式布局。

A10 🆕🟠 运行记录丢弃最关键的来源上下文，无法按工作流、运行、节点或事件类型定位故障
状态：新发现·确认存在 ｜ desktop/Ariadne.Desktop/Views/RunLogPageView.axaml:43-60；desktop/Ariadne.Desktop/ViewModels/RunLogPageViewModel.cs:21-30,96-129,156-193；core/src/commands.rs:1756-1771

后端日志查询已经支持 `kind/workflow_id/run_id/node_id/level/query`，桌面模型也接收并保存了 `Kind`，但页面每条记录只显示时间、级别和消息，筛选器也只有级别与关键字。`Kind` 从未绑定，工作流、运行和节点过滤入口完全缺失。

用户场景：一次工作流产生数百条节点、工具、模型、成本和诊断日志后，作者只能在混合消息中肉眼翻找；两个运行出现相似错误文案时，页面无法判断是哪次运行、哪个节点、哪类事件，所谓“运行记录”不能承担排障职责。

建议：显示本地化的事件类型与来源（工作流/运行/节点），支持组合筛选和从画布节点跳转到对应日志；保留“仅看当前运行/仅看错误”的快捷筛选。

A11 🆕🟠 运行记录页无分页、无虚拟化，每次查询把全部日志逐条实例化为可视控件
状态：新发现·确认存在 ｜ desktop/Ariadne.Desktop/Views/RunLogPageView.axaml:43-69；desktop/Ariadne.Desktop/ViewModels/RunLogPageViewModel.cs:96-129；core/src/commands.rs:1756-1771

页面结构是外层 `ScrollViewer` 包住 `StackPanel + ItemsControl`，没有虚拟化列表和分页；刷新时后端返回完整 `Vec`，前端清空集合后逐条创建 ViewModel 与卡片控件。外层 ScrollViewer 让 ItemsControl 按全部内容高度测量，视口外记录也会进入视觉树。

复杂度与用户影响：L 条日志的一次刷新至少产生 O(L) 反序列化、集合通知、ViewModel 和控件实例；结合日志文件本身的全量查询，长期项目进入日志页或切换级别时会随历史长度持续变慢、占用大量内存，严重时界面冻结。

建议：后端提供游标分页/时间窗口；前端改用带 `VirtualizingStackPanel` 的列表，批量替换集合，并默认只加载最近一页。

A12 🆕🟠 作品编辑器把 4–6 千字符分块直接暴露成多个独立 TextBox，跨块选择、连续键盘编辑和光标稳定性被破坏
状态：新发现·确认存在 ｜ desktop/Ariadne.Desktop/Views/WorksPageView.axaml:102-148；desktop/Ariadne.Desktop/Views/WorksPageView.axaml.cs:24-41,61-69,138-164；desktop/Ariadne.Desktop/ViewModels/WorksPageViewModel.cs:11-16,452-472,505-587

当前实现已经不是 A2 所述的“单个 TextBox”，而是按约 4000 字符拆成多个独立 TextBox。上下块之间不存在编辑器级的连续选区、退格合并、方向键跨块移动或全篇 Ctrl+A；右键“全选”也只选择当前活动块。单块增长到 12000 字符以上时，`RebalanceDocumentBlocks` 会清空并重建全部块，正在编辑的 TextBox、焦点、选区和光标对象随之被替换。

用户场景：作者从一段中部拖选到下一分块无法形成连续选区；在分块边界按退格不会像普通编辑器一样自然合并；粘贴大段文本触发重分块后，光标和滚动位置可能跳走。界面还把“分块：N”写进文档信息，把内部实现细节直接甩给作者。

建议：保留分块存储/渲染，但提供一个逻辑连续的编辑表面和全局选区模型；重分块必须保留绝对光标、选区与滚动锚点。旧 A2 应标记为已失效并替换为本条，而不是继续报告“单 TextBox”。
