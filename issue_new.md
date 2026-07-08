# Ariadne 当前已知问题（Issue 清单 · 终版）

> **审计日期**：2026-07-08（Asia/Shanghai）
> **对应 commit**：`main` 分支 HEAD `700e9bd29332aea8527a556596b26b0493960dbe`
> **审计来源（四份交叉核对）**：
>
> 1. 仓库自带 `Ariadne_audit_report.md`（31 KB，14 条 L/U 问题）
> 2. 仓库自带旧 `issue.md`（约 25 条）
> 3. 我方二次深度静态审计（通读 core/src 的 ipc/commands/workflow/runtime/config/costs/providers/llm/contracts/secrets + desktop 的 AXAML/ViewModel/Backend/Theme）
> 4. 第三方独立全量审计（4.8 万行扫读，重点在 Skill/WASM/Patch/Git/SQLite/安全，是本次最主要的新增来源）
>
> **条目符号约定**：
> - 🔥 = 第三方审计发现但前三份报告漏掉的**关键**问题
> - 🆕 = 我方二次审计新增
> - 🟡部分修复 = 旧问题修了一半仍残留
> - ⚠️版本差异 = 第三方审计观察的是较早版本，HEAD `700e9bd` 已修，仍建议回归验证
> - **未做的事**：未本地 cargo/dotnet build，未跑真实 LLM 端到端；所有结论以静态代码证据为准。

---

## 0. 一句话结论

core 里对「权限 / 预算 / 确认项 / 版本 / 通信边 / 文档 patch（UTF-8 字节范围+is_char_boundary 检查）」这些严肃概念的设计方向是对的，色板 token 也认真定义过，但当前版本存在三个致命的、让产品"跑不起来"的断裂：

1. **IPC 架构断裂**：one-shot CLI 进程 vs 桌面端期望的"长驻服务"完全错位，导致 Pause/Resume/Stop/实时状态全是假按钮，密钥保存即丢，DB 多进程并发；
2. **前后端 DTO 手写错位**：`run_workflow` Rust 返回裸字符串，C# 端期待 `{run_id,status}` 对象，反序列化必失败，Run 按钮实际上拿不到 run_id；
3. **画布核心能力缺失**：没有边的渲染、没有端口、没有拖拽连线——一个主打"GUI 编辑节点流程图"的产品，流程图最基本的"线"不存在。

修完这三件事之前，RAG、多粒度、长篇伏笔回收都是上层故事。

---

## 1. 严重程度汇总

| 级别 | 数量 | 代表问题 |
|---|---|---|
| 🔴 **P0 阻断** | **11** | IPC 同步阻塞+stdin Close；run_workflow 返回类型不匹配；密钥内存丢；pause/resume 不驱动 runtime；画布无连线；侧栏折叠按钮死代码；data 边不参与调度；EN/JA 空壳；正文单 TextBox；DialogScrim 盖标题栏；XAML `Classes.selected` 语法错导致选中态全失效 |
| 🟠 **P1 高风险** | **28** | 预算事后检查；manual 策略映射丢；communication reset 不完整；open_project 弱校验+无沙箱；workflow_id 缺省返回空图；Patch 行号 off-by-one；单 hunk diff；diagnostics 永远 Healthy；默认模型 id 未验证；HTTP Skill .no_proxy()+无大小限制+SSRF；日志脱敏漏判过删；WASM memory.grow 突破上限；SQLite 无 WAL/锁；ledger 多连接并发写；canonicalize/symlink 可能逃逸；project_root 锁粒度不对；attempts 计数错；communication 不收敛；AutoMode 语义混用默认值/当前态；最近项目双路径；文档保存无并发检测；Quick Edit 一键毁章；冷启动 env 缺失静默失败；Project AI 假聊天；base_url 无 URL 校验；Settings 控件堆叠；关闭按钮未接未保存确认；Confirmation 按钮无禁用态；实时事件空壳 |
| 🟡 **P2 体验/视觉/工程** | **29** | 右栏固定 280px；画布无缩放/小地图；确认项埋太深；BackendStatus 过载；$0/$0 误导；手写 JSON/模板；预算边界 off-by-one；f64 货币；孤立节点未检测；run-ui.sh 不构建 Rust 后端；枚举序列化风险；eprintln! 不进事件流；节点卡表单化；两三角图标歧义；色板未接线；三套宽度并存；Diff 等宽 TextBox；无 Dark Mode；图标粗细不一；chip 无下拉箭头；预算条太细；选中竖条 3px 太弱；ToggleSwitch 无文字；Badge 无级别色；右键绑 Canvas 点节点不出菜单；节点长名撑高重叠；错误类型打平成 String；reqwest::blocking 无 per-call 取消；缺集成测试；global.json 锁 .NET 10 preview；README 残留 Tauri 路径；data edge alias 只走 stderr；节点无 Min/MaxHeight；SecretStore Clone 语义不一致 |
| 🟢 **已修复** | 9 | 默认模板 URL 为空（⚠️第三方 #47 说 .local 是旧版本）；模板 URL http/https 校验；run_id 加随机后缀；工具默认 deny；keychain service 名改为 ariadne（⚠️第三方 #20/#46 说 literature-agent 是旧版本）；reachable_nodes 改 HashSet；Loop 部分清 communication（L-13 残留 next_sender/hash）；preauthorized=0 映射 None；Project AI tool schema 允许自定义字段 |

---

## 2. P0 阻断级问题（必须先修）

### P0-1 🔥 运行/暂停/停止/恢复架构断裂 + stdin 主动 Close 埋坑
- **位置**：`desktop/.../Backend/JsonLineBackendClient.cs::{InvokeRequiredAsync,InvokeCommandAsync,InvokeOrDefaultAsync}`、`core/src/ipc.rs::run_json_line_stdio`、`core/src/commands.rs::run_workflow_impl`、`core/src/workflow/runtime.rs::run_inner`
- **问题**：每次 IPC 调用都 `Process.Start` 新进程 → WriteLine 一行 JSON → **`process.StandardInput.Close()`** → `ReadToEnd` → `WaitForExit`。`run_workflow` 内联调 `runtime.run_persisted` 同步跑完整个流程，堵住 stdin；`pause/stop/resume/get_workflow_run_state/resolve_confirmation` 全部排队等运行结束。Pause/Stop 在 UI 层是 `await RunWorkflowAsync` 返回后才赋值 `CurrentRunId`，运行中按钮 `CanExecute=false`。
- **更致命**：`Close stdin` 意味着即便将来改成 while 循环长连接，第一条请求后管道也被前端主动切断，confirmation/retry backoff/多轮对话根本没有通道回灌原进程，只能新起进程读写 SQLite → 多进程并发+无 WAL（P1-15）= SQLITE_BUSY。
- **影响**：长流程无法中断；四种"暂停"都是假暂停；retry backoff 没 scheduler 续跑；密钥/最近项目/配置全是进程内状态，每次调用冷启动。
- **建议**：长生命周期后端 + job manager + event stream（WebSocket/SSE/named pipe）+ CancellationToken；stdin 常开按 request/response id 协议；`run_workflow` 立即返回 `run_id` 后台执行。

### P0-2 🔥 `run_workflow` 前后端 DTO 类型对不上，Run 按钮必反序列化失败
- **位置**：`core/src/commands.rs` `run_workflow_impl` 返回 `CommandResult<String>`（裸字符串 run_id）；`desktop/.../AriadneBackendModels.cs` 中 `RunWorkflowAsync` 声明 `Task<WorkflowRunStarted>`（期待 `{run_id, status}` 对象）
- **问题**：后端返回 `"run-xxx"` 纯字符串，`System.Text.Json` 抛 JsonException；Run/单节点运行拿不到 run_id，后续 Pause/Stop/Resume 因 `CurrentRunId` 为空显示"无"。同类问题：`pause/stop/resume` 后端返回 `WorkflowActionResult{workflow_id,run_id,status}`，C# 端也声明为 `Task<WorkflowRunStarted>`，字段侥幸对齐但语义错。
- **建议**：用 typeshare/schemars 从 Rust 类型生成 C# DTO，避免手写 record 漂移；所有 run 相关命令统一返回 `WorkflowRunStarted{run_id, status}`。

### P0-3 确认/断点恢复后旧 run 没有继续执行路径
- **位置**：`commands.rs::{resume_workflow, resolve_confirmation_impl, resume_from_node, override_confirmation_output, update_workflow_run_control}`
- **问题**：这些命令全部 load state→改 state→save 返回 label，**完全没再调 `runtime.run_persisted()`**。`run_workflow_impl` 也不接收已有 run_id，总是 `WorkflowRuntime::new(...)`。结合 P0-1，新进程写 DB 里 control=Resume，正在跑的原进程不 reload DB，新进程也不自动进入 run_inner——工作流永久停摆。
- **建议**：长驻后端下新增 `continue_workflow_run(project_root, workflow_id, run_id)`，加载已有 state 后真正进入 `run_persisted`；`update_workflow_run_control` 改完状态后由 scheduler 触发 continue 或向运行中的 runtime 发 signal。

### P0-4 默认 SecretStore 是纯内存，叠加"每命令一进程"=API Key 保存即丢
- **位置**：`core/Cargo.toml`（`default=[]`）、`config/secrets.rs::MemorySecretStore`、`commands.rs::default_secret_store`
- **问题**：默认 feature 未启用 system-keychain，返回进程内 `Arc<RwLock<BTreeMap>>`；Linux 桌面大多无 keyring daemon；每次 IPC 新进程，"保存 Key→进程退→下次读不到→LLM 提示 missing secret"。⚠️第三方审计 #20/#46 说 SystemKeychainSecretStore 默认 service 名是 `literature-agent`——直接读 HEAD `config/secrets.rs::SystemKeychainSecretStore::default()` 是 `Self::new("ariadne")`，已修。
- **建议**：桌面构建强制启用 `system-keychain`；非 keychain 环境提供"主密码+本地加密文件"fallback；长期先解决 P0-1。

### P0-5 🔥 侧栏折叠按钮绑了个寂寞
- **位置**：`MainWindow.axaml`（侧栏 Border 写死 `Width="204"`）、`MainWindowViewModel.cs`（`ToggleSidebarCommand => new(...)` 每次访问 new 一个新 RelayCommand）
- **问题**：Width 没绑到 `SidebarExpanded`；Command 每次 new 是新实例不会触发 CanExecuteChanged；点按钮只翻转 bool，UI 完全没变化。
- **建议**：Width 双向绑定到 SidebarExpanded（BoolToWidth converter 或 Visual State）；Command 在构造函数里 new 一次复用。

### P0-6 画布无连线、无端口、无拖拽——核心卖点缺失
- **位置**：`Views/WorkspacePageView.axaml`、`ViewModels/WorkspacePageViewModel.cs`
- **问题**：只有 `ItemsControl ItemsSource="{Binding Nodes}"` 渲染矩形节点；**没有** Edges ItemsControl/Path/Line、input/output port 圆点、端口拖拽手势、Add Edge 命令、边类型颜色区分；ViewModel 虽然维护 `Edges`/`ToCanvasEdge()` 但 XAML 完全不消费。边编辑只能靠右侧面板手填 SourceHandle/TargetHandle 字符串 ID。
- **建议**：节点显示端口圆点；端口拖线→端口创建边；画布渲染 Bezier/orthogonal 线；control/data/communication 边分色。

### P0-7 调度只认 control 边，data 边不参与 readiness
- **位置**：`workflow/runtime.rs::{ready_nodes, control_dependencies_satisfied, collect_data_inputs}`、`contracts/workflow.rs::validate_topology`
- **问题**：data 边不阻塞目标节点，`collect_data_inputs` 遇 source 不存在直接 `continue` 静默跳过——B 可能比 A 先跑，prompt 缺上下文。
- **建议**：data edge 默认也是依赖；非阻塞输入加显式 `optional_data` 边类型；或 `validate_topology` 强制 data 边伴随 control path。

### P0-8 EN/JA 国际化是空壳，README 宣称三语
- **位置**：`core/resources/display_name.en.json`（201 字节 stub）、`display_name.ja.json`（283 字节 stub）、中文主文件 37.4 KB
- **建议**：README/语言下拉标注"未完成/预览"，或暂时下线 EN/JA。

### P0-9 "理论无限长度写作"正文编辑器是单个 TextBox 整篇双向绑定
- **位置**：`WorksPageView.axaml`
```xml
<TextBox Text="{Binding DocumentContent, Mode=TwoWay}" AcceptsReturn="True" MinHeight="620" .../>
```
单 TextBox 装整章文本，无虚拟化/分块/增量保存；TwoWay 每次按键触发 Setter；几十万字必卡顿。
- **建议**：分块/虚拟化编辑器或基于行区间增量保存。

### P0-10 🔥 `Classes.selected="{Binding IsSelected}"` 不是 Avalonia 合法语法 → 全应用选中态从未生效
- **位置**：多个 AXAML（导航项、tab 按钮、节点）
- **问题**：Avalonia 不支持"点属性"语法 `Classes.selected="..."`，应该用 pseudo-class `:selected` 或 code-behind 增删类；结果导航/节点/tab 的"选中高亮"样式大概率从未生效。这解释了之前看到"导航选中只有一个 3px 细竖条、节点选中无加粗"的现象——其实那 3px 竖条是靠 `IsVisible="{Binding IsSelected}"` 单独绑的一个 Border，并非真·选中样式。
- **建议**：全部改成 Avalonia 支持的写法（SelectingItemsControl + `:selected` 伪类，或 code-behind 在 IsSelected 变化时 `Classes.Add/Remove("selected")`）。

### P0-11 DialogScrim 盖掉整个标题栏，弹窗期间窗口不能移动/最小化/关闭
- **位置**：`MainWindow.axaml`（`DialogScrim` 在 Panel 最后层，`Background="#66000000"` 覆盖整个客户区含 40px 自绘标题栏）
- **问题**：配合 `WindowDecorations="None"`，任何弹窗出来后不能拖动/最小化/最大化/关闭；源码注释直书"半透明 scrim 覆盖整个客户区（含标题栏）；点击遮罩=取消"。
- **建议**：scrim 只覆盖内容区（Row=1），标题栏（Row=0）保持可交互；或至少让窗口控制按钮浮在遮罩之上。
