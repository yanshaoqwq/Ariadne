##当前已知问题

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

A2 🆕🟠 作品正文用单个 TextBox 双向绑定——“理论无限长度写作”的架构隐患
状态：新发现·确认存在 ｜ WorksPageView.axaml

本项目核心卖点是“利用多粒度 RAG 实现理论无限长度写作”。但章节正文编辑器是一个 TextBox Text="{Binding DocumentContent, Mode=TwoWay}"（MinHeight="520"），整章文本塞进一个 Avalonia TextBox 做整体双向绑定，无虚拟化、无分块渲染。超长章节下，渲染/输入/序列化都会劣化；且大文本每次属性变更触发绑定，内存与卡顿风险高。对一个“写超长小说”的工具，这是基础体验短板。

建议：长正文改用分块/虚拟化编辑器，或基于行区间的增量保存，避免整篇大字符串双向绑定。

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
