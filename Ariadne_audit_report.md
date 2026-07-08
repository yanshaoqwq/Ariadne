# Ariadne 代码审计报告（逻辑 / UI / 反人类设计）

审计对象：`yanshaoqwq/Ariadne` 当前 `main` 分支，HEAD `15032cbcf7fbdc964d76d777f4541b1ff92344e1`（GitHub 显示 2026-07-08）。  
审计方式：静态源码审计，重点阅读 Rust `core` 与 Avalonia/C# `desktop`。本报告不声称已完成动态编译、端到端运行或真实 LLM 调用测试。  
重点文件：

- `core/src/commands.rs`
- `core/src/ipc.rs`
- `core/src/workflow/runtime.rs`
- `core/src/contracts/workflow.rs`
- `core/src/llm/service.rs`
- `core/src/providers/executor.rs`
- `core/src/config/secrets.rs`
- `desktop/Ariadne.Desktop/Backend/JsonLineBackendClient.cs`
- `desktop/Ariadne.Desktop/ViewModels/MainWindowViewModel.cs`
- `desktop/Ariadne.Desktop/ViewModels/WorkspacePageViewModel.cs`
- `desktop/Ariadne.Desktop/Views/MainWindow.axaml`
- `desktop/Ariadne.Desktop/Views/WorkspacePageView.axaml`

---

## 0. 总结：当前最大问题

这个仓库已经修掉了一批明显问题，例如：`run_id` 加了随机后缀、模板仓库默认 URL 改成空、模板 URL 做了 http/https 校验、工具控制默认改成 deny、keychain service 名称改成 `ariadne`、`reachable_nodes_from_start` 改成 `HashSet` 去重、Loop 重跑开始清理 communication 状态等。

但是当前代码仍存在几类会直接伤害可用性的结构性问题：

1. **运行/暂停/恢复模型是断裂的**：桌面端每个命令启动一个新的 `ariadne-ipc` 进程；`run_workflow_impl` 同步跑完整个流程；`pause/resume/resolve_confirmation` 只改数据库状态，没有后台任务继续执行旧 run。结果是：长流程无法真正停止，暂停后的工作流也无法真正继续。
2. **默认密钥存储在桌面端几乎不可用**：Rust core 默认 feature 为空，`default_secret_store()` 在未启用 `system-keychain` 时是内存存储；桌面端又是“每个命令一个进程”，保存 API Key 后进程退出，下一次命令就读不到。
3. **工作流 UI 名义上是流程图，实际无法像流程图一样编辑**：可以添加节点，但当前 UI 没有明显的“创建连线/拖线连接”入口，也没有画布连线可视化；边只能在右侧列表里编辑已有数据。对一个“GUI 编辑节点流程图”的产品，这是 P0 级缺陷。
4. **数据边不参与调度依赖**：runtime 只等 control 边；有 data 边但没有 control 边时，下游节点可能先运行，然后静默拿不到输入。
5. **预算控制是“花完再检查”**：Provider 调用已经发生、账本已记录后才做 budget check；这不能阻止真实花费。
6. **Auto Mode / 确认策略有高危 UX 与映射问题**：标题栏开关无二次确认；旧策略 `manual` 无法被双字段模型正确表达，可能在保存后变成 Auto Mode 自动审批策略。
7. **多个 UI 设计对用户极不友好**：页面切换重建 ViewModel、无 Undo/Redo、右侧面板固定 280px、对话框遮住标题栏、确认项入口隐藏、节点状态只有文字无颜色反馈、预算 `$0/$0` 表达误导。

---

## 1. P0 / P1 逻辑问题

### L-01：工作流运行、暂停、停止、恢复的架构断裂

**严重级别：P0 / blocker**  
**位置：**

- `desktop/Ariadne.Desktop/Backend/JsonLineBackendClient.cs`
- `core/src/ipc.rs`
- `core/src/commands.rs::run_workflow_impl`
- `core/src/commands.rs::pause_workflow / stop_workflow / resume_workflow / resolve_confirmation_impl`
- `core/src/workflow/runtime.rs::run_inner`
- `desktop/Ariadne.Desktop/ViewModels/WorkspacePageViewModel.cs::RunNodeAsync / PauseWorkflowAsync / ResumeWorkflowAsync`

**问题：**

桌面端的 `JsonLineBackendClient` 不是维护一个长生命周期后端连接，而是每次调用都：

1. `Process.Start` 启动一个 `ariadne-ipc` 进程；
2. 向 stdin 写一条 JSON；
3. 关闭 stdin；
4. `ReadToEndAsync` 等 stdout；
5. 等进程退出。

而 Rust 的 `run_workflow_impl` 会同步调用：

```rust
runtime.run_persisted(&workflow, &mut executor, &store)
```

这意味着一次 `run_workflow` IPC 调用会一直占用该子进程直到流程运行到成功、失败或暂停。

`pause_workflow` / `stop_workflow` / `resume_workflow` 的实现只是打开 `runtime.db`，加载状态，修改 `runtime.state.control/status`，再保存回去。它们**不会**唤醒或继续正在运行的 `run_workflow_impl`，也不会让已暂停 run 重新进入 `run_persisted`。

更严重的是：

- `WorkspacePageViewModel.RunNodeAsync` 在 `await _backend.RunWorkflowAsync(...)` 返回之后才设置 `CurrentRunId`。
- `PauseWorkflowCommand` / `StopWorkflowCommand` / `ResumeWorkflowCommand` 的可用性依赖 `HasCurrentRun()`。
- 所以当真正的长任务正在跑时，UI 还拿不到 run id，暂停/停止按钮基本不可用。
- 即便用户通过其他方式知道 run id，另一个进程写入 `runtime.db` 的 pause/stop 状态，正在运行的进程也不会持续轮询这个外部状态。

**影响：**

- 用户点击“运行”后，长流程无法可靠中断。
- 断点、确认项、预算暂停、communication 次数耗尽后的“Resume”只是把状态改成 Queued，不会继续跑。
- 自动重试退避也没有后台 scheduler。`run_inner` 遇到 `has_pending_retry_backoff` 后会返回 Queued，但没有进程在退避到期后继续。
- UI 上的 Pause/Stop/Resume 是“看起来有”，实际上核心语义不成立。

**建议修复：**

P0 重构运行架构：

1. 引入长生命周期后端进程或 in-process service，不要每个命令启动一次后端。
2. `run_workflow` 应立即创建 run，返回 `run_id`，后台任务执行。
3. runtime 执行循环需持有 `CancellationToken` / `RunControl`，并定期检查外部控制状态。
4. 增加 `resume_existing_run(workflow_id, run_id)`：加载旧 state 后重新进入 `run_persisted`。
5. `resolve_confirmation` 在确认全部解决后可以选择自动继续，或明确要求用户点击继续，但继续必须真的执行旧 run。
6. 用事件流、WebSocket、SSE、named pipe 或至少轮询接口推送 `workflow_status_update`。

---

### L-02：默认 SecretStore 与桌面“单命令单进程”模型冲突，API Key 保存后丢失

**严重级别：P0 / blocker**  
**位置：**

- `core/Cargo.toml`
- `core/src/commands.rs::default_secret_store`
- `core/src/config/secrets.rs::MemorySecretStore`
- `desktop/Ariadne.Desktop/Backend/JsonLineBackendClient.cs`

**问题：**

`core/Cargo.toml`：

```toml
[features]
default = []
system-keychain = ["dep:keyring"]
```

默认不启用 `system-keychain`。对应 `commands.rs`：

```rust
#[cfg(not(feature = "system-keychain"))]
pub fn default_secret_store() -> Arc<dyn SecretStore> {
    Arc::new(MemorySecretStore::default())
}
```

这在“长期运行的单进程后端”里已经只是临时方案；但桌面端当前每个命令都新开一个进程，问题变成致命：

1. 用户在设置页保存 API Key。
2. `save_provider_key` 把 key 写入该进程的 `MemorySecretStore`。
3. 进程退出。
4. 下一次 `get_provider_config` 或 `run_workflow` 是新进程，MemorySecretStore 为空。
5. UI 看到 key 未配置，LLM 调用提示 missing secret。

**影响：**

- 默认构建下，桌面应用的模型配置不可持久化。
- 用户会感觉“我刚刚保存了 Key，但软件说没保存”。这是信任灾难。

**建议修复：**

1. 桌面构建强制启用 `system-keychain`，并在 `run-ui.sh` / csproj / README 中明确。
2. 非 keychain 环境提供加密文件 fallback，而不是内存 fallback。
3. 如果继续每命令进程模型，secret store 必须跨进程持久化。
4. 更推荐先修 L-01：后端变成长生命周期服务，再配合系统 keychain。

---

### L-03：暂停/确认后的 run 没有真正的继续执行路径

**严重级别：P0 / blocker**  
**位置：**

- `core/src/commands.rs::resume_workflow`
- `core/src/commands.rs::resolve_confirmation_impl`
- `core/src/commands.rs::resume_from_node`
- `core/src/commands.rs::override_confirmation_output`

**问题：**

这些命令都只是：

1. 从 `SqliteWorkflowRuntimeStore` 加载 `WorkflowRunState`；
2. 构造 `WorkflowRuntime::from_state(state)`；
3. 修改状态；
4. 保存状态；
5. 返回状态 label。

没有任何地方重新调用：

```rust
runtime.run_persisted(...)
```

`run_workflow_impl` 也不会接受已有 `run_id`，而是总是新建 `WorkflowRuntime::new(&workflow, run_id)`。

**影响：**

- 确认项通过后，旧 run 只是 Queued，不会继续。
- `Resume` 按钮只是改状态，不执行。
- breakpoint、budget pause、communication max pause 的恢复机制都停留在状态机层，没有调度层闭环。

**建议修复：**

新增命令：

```rust
continue_workflow_run(project_root, workflow_id, run_id)
```

语义：加载 workflow + existing runtime state，构造 executor，然后调用 `run_persisted`。桌面端 Resume / 确认通过后可调用它。

---

### L-04：数据边不参与 readiness，节点可能在输入不存在时运行

**严重级别：P0 / high**  
**位置：**

- `core/src/workflow/runtime.rs::ready_nodes`
- `core/src/workflow/runtime.rs::control_dependencies_satisfied`
- `core/src/workflow/runtime.rs::collect_data_inputs`
- `core/src/contracts/workflow.rs::validate_topology`

**问题：**

调度 readiness 只考虑 control 边：

```rust
control_dependencies_satisfied(workflow, state, &node.id)
```

`control_dependencies_satisfied` 只 filter `WorkflowEdgeKind::Control`。data 边不会让目标节点等待 source 成功。

随后 `collect_data_inputs` 遇到 source 不存在时：

```rust
let Some(source) = state.nodes.get(&edge.from.node_id) else {
    continue;
};
```

这会静默跳过输入。

**影响：**

- 用户画了一条数据边，以为表达“B 使用 A 的输出”。但如果没有额外 control 边，B 可能先运行。
- B 的 prompt/input 缺失，轻则运行失败，重则 LLM 在缺上下文下产出错误内容。
- 这是“图形工作流”的典型直觉违背。

**建议修复：**

至少二选一：

1. data 边也作为 readiness 依赖：所有 data source 成功后目标才可运行。
2. 强制要求 data 边必须伴随 control path，并在 `validate_topology` 检查。

更推荐：data edge 默认也是依赖；如果要非阻塞输入，增加显式 `optional_data` 或 edge config。

---

### L-05：data 边缺 alias 仍是合法拓扑，运行时只 stderr 警告

**严重级别：P1 / medium-high**  
**位置：**

- `core/src/contracts/workflow.rs::validate_topology`
- `core/src/workflow/runtime.rs::collect_data_inputs`

**问题：**

`validate_topology` 只在 alias 存在时检查非空、重复、只允许 data edge 使用；但 data edge alias 缺失不会报错。

运行时：

```rust
let Some(alias) = &edge.alias else {
    eprintln!("[ariadne] warning: data edge ... has no alias; input will be skipped");
    continue;
};
```

在桌面端，这个 stderr 警告不会变成 UI 可见诊断，也不会进入 structured events。

**影响：**

- 用户的边存在，但节点拿不到输入。
- UI 没有明确报错，排错困难。

**建议修复：**

- `validate_topology` 中要求 data edge 必须有 alias。
- 或至少写入 `WorkflowRuntimeEventType`，并在运行日志/节点详情中显示。

---

### L-06：communication reset 不完整，重跑后可能从错误一方继续发言

**严重级别：P1 / high**  
**位置：**

- `core/src/workflow/runtime.rs::reset_communication_edges_for_nodes`
- `core/src/workflow/runtime.rs::advance_loop`
- `core/src/workflow/runtime.rs::resume_from_node`

**问题：**

当前 `reset_communication_edges_for_nodes` 会重置：

```rust
comm.completed = false;
comm.completed_reason = None;
comm.pause_reason = None;
comm.message_count = 0;
comm.messages.clear();
```

但没有重置：

- `comm.next_sender_node_id`
- `comm.last_message_hash`

如果上一轮 communication 已经把 `next_sender_node_id` 推到了接收方，Loop 重跑或路径 A 注入后，这条边虽然消息清空、次数归零，但“下一发言人”仍可能是上一轮状态的接收方，而不是 `initiator_node_id`。

**影响：**

- 返修循环第二轮可能从错误节点开始。
- UI 上看起来像“循环重跑了”，实际 communication 语义漂移。

**建议修复：**

```rust
comm.next_sender_node_id = comm.initiator_node_id.clone();
comm.last_message_hash = None;
```

并增加回归测试：Writer → Critic → Loop → Writer，验证第二轮第一条 communication 仍由 initiator 发起。

---

### L-07：预算控制发生在 Provider 调用之后，不能阻止真实花费

**严重级别：P1 / high**  
**位置：**

- `core/src/providers/executor.rs::complete_llm`
- `core/src/llm/service.rs::call_provider`
- `core/src/llm/service.rs::check_after_provider_response`
- `core/src/llm/service.rs::check_budget`

**问题：**

调用顺序是：

1. `ProviderExecutor::complete_llm` 调用真实 provider；
2. provider 返回 response；
3. `record_optional_cost` 写成本账本；
4. `LlmService::check_after_provider_response` 再执行 budget check。

这意味着预算超限只能在钱已经花完后暂停。

**影响：**

- `single_call_usd` 或每日/月度预算无法作为“预授权上限”。
- 用户以为设置了预算保护，实际第一笔超额调用已经发生。

**建议修复：**

1. 调用前基于模型价格、输入 token、`max_output_tokens` 做 worst-case 预估。
2. 如果无法估算价格，要求用户确认或视为高风险。
3. 做“预算预留 reservation”：调用前占用额度，调用后用真实成本结算，多退少补。
4. Auto Mode 下也不能绕过硬预算。

---

### L-08：确认策略 `manual` 无法被当前双字段模型表达，保存后可能被改写

**严重级别：P1 / high**  
**位置：**

- `core/src/commands.rs::policy_for_kind`
- `core/src/commands.rs::policies_from_legacy_policy`
- `core/src/commands.rs::legacy_policy_from_dual_policy`
- `core/src/commands.rs::approval_policy_from_ui`

**问题：**

`policy_for_kind` 在没有 prompt 或 `allow_auto_approval=false` 时返回：

```rust
"manual"
```

但 `policies_from_legacy_policy` 没有显式处理 `manual`，未知值走默认：

```rust
_ => (ManualReview, AutoApproval)
```

更深层的问题是：`ConfirmationAutoModePolicy` 只有：

- `AllowByDefault`
- `AutoApproval`

没有“Auto Mode 下仍手动审批 / 不自动审批”的状态。因此 legacy 的 `manual`（Auto Mode 也不自动通过）无法无损表达。

**影响：**

- 默认 `ApprovalPolicy::default()` 是不允许自动审批；但 UI 读取后可能展示成 Auto Mode 自动审批。
- 保存后可能写成 `auto_audit`，改变安全策略。
- 用户以为“手动”，实际 Auto Mode 打开后可能自动审查/审批。

**建议修复：**

1. 给 `ConfirmationAutoModePolicy` 增加 `ManualReview` 或 `Disabled`。
2. `policies_from_legacy_policy("manual")` 必须映射到 `(ManualReview, ManualReview)`。
3. 不要再用 `policy` 字符串作为优先来源覆盖双字段；做一次迁移后删除 legacy 字段。
4. 为四种策略加 round-trip 测试。

---

### L-09：`open_project` 只检查路径非空，不验证项目存在或已初始化

**严重级别：P1 / medium-high**  
**位置：**

- `core/src/commands.rs::open_project`
- `core/src/commands.rs::validate_project_root`
- `core/src/commands.rs::validate_existing_project_root`

**问题：**

`validate_existing_project_root` 已存在，但 `open_project` 调用的是弱校验：

```rust
validate_project_root(&project_root)?;
```

它只检查路径非空。

**影响：**

- 打开不存在目录、文件路径、未初始化目录时，UI 可能记录最近项目并进入异常状态。
- 后续 `load_or_create` 类操作可能静默创建配置，使“打开项目”变成“半创建项目”。

**建议修复：**

- `open_project` 使用 `validate_existing_project_root`。
- 进一步检查 `.config/`、`workflows/`、`documents/` 等初始化标记。
- `create_project` 才允许创建新目录。

---

### L-10：加载不存在 workflow_id 时返回空 default workflow，容易掩盖错误

**严重级别：P1 / medium**  
**位置：**

- `core/src/commands.rs::load_workflow_definition`
- `core/src/commands.rs::workflow_path`

**问题：**

`load_workflow_definition(project_root, Some(workflow_id))` 在文件不存在时返回一个空的 `Default Workflow`，而不是报错。

**影响：**

- workflow id 拼写错误不会被发现。
- 用户可能保存空图覆盖预期文件。
- API 调用方难以区分“新建 default”与“加载失败”。

**建议修复：**

- 仅在 `workflow_id == None` 或显式 create 时创建默认 workflow。
- `Some(id)` 缺失应返回 `workflow not found: id`。

---

### L-11：Project AI workflow tool schema 仍不可用/不可发现

**严重级别：P1 / medium**  
**位置：**

- `core/src/commands.rs::project_ai_workflow_tools`
- `core/src/commands.rs::start_node_tool_input_schema`
- `desktop/Ariadne.Desktop/ViewModels/WorkspacePageViewModel.cs`

**问题：**

当前已不再强制所有工具 schema 都是空对象，而是从 start node config 读取：

- `tool_input_schema`
- `input_schema`

但 UI 只提供 `ExposeAsTool`、`WorkDir`、`Name` 等字段，没有 schema 编辑器，也没有从 start 节点端口自动生成 schema。

**影响：**

- 普通用户勾选“Expose as Tool”后，Project AI 看到的工具多数仍是空参数工具。
- 如果工作流入口需要参数，LLM 无法知道该传什么。

**建议修复：**

1. 从 start node 的输入端口定义生成 JSON Schema。
2. UI 提供工具参数 schema 编辑器/预览。
3. 没有 schema 但存在 `initial_inputs` 需求时，禁止暴露或给出警告。

---

### L-12：Project AI 只执行第一个 tool call，其余 tool call 被忽略

**严重级别：P2 / medium**  
**位置：**

- `core/src/commands.rs::project_ai_answer`

**问题：**

代码使用：

```rust
report.response.tool_calls.iter().find_map(...)
```

只找第一个匹配的 workflow tool 并运行一次。

**影响：**

- 如果模型一次返回多个 tool call，只有第一个执行。
- UI/回答没有告诉用户其他 tool call 被忽略。

**建议修复：**

- 明确限制 tool_choice 为最多一个，或循环执行所有 tool call。
- 将被忽略的 tool call 写入审计日志和 UI。

---

### L-13：Provider base_url 缺少 URL 级校验

**严重级别：P2 / medium**  
**位置：**

- `core/src/config/models.rs::ProviderConfig::validate`
- `core/src/providers/protocol.rs::resolve_base_url`

**问题：**

模板仓库 URL 已校验 http/https，但 provider `base_url` 只检查 OpenAI-compatible 非空，没有解析 URL、scheme、host。

**影响：**

- 错误配置要到真实调用才失败。
- 本地桌面软件场景 SSRF 风险低于服务端，但仍可能访问内网地址或奇怪 endpoint。

**建议修复：**

- 使用 `url::Url::parse`。
- 限制 scheme 为 http/https。
- UI 上对 localhost / 内网地址给风险提示。

---

### L-14：构建/启动脚本没有构建 Rust IPC 后端

**严重级别：P2 / medium**  
**位置：**

- `desktop/run-ui.sh`
- `desktop/Ariadne.Desktop/Backend/JsonLineBackendClient.cs::DiscoverBackendCommand`

**问题：**

`run-ui.sh build/run` 只执行：

```bash
dotnet build "$CSPROJ/Ariadne.Desktop.csproj"
```

但桌面端通过 `DiscoverBackendCommand()` 查找：

- `core/target/debug/ariadne-ipc`
- `target/debug/ariadne-ipc`

脚本没有 `cargo build --bin ariadne-ipc`。

**影响：**

- 新用户按脚本启动 UI，后端可能显示不可用。
- 这与 README “打开应用后配置模型/新建项目”不一致。

**建议修复：**

- `run-ui.sh build` 同时构建 Rust core。
- 桌面 csproj 增加 pre-build 或 README 明确步骤。
- 发布包中捆绑后端二进制。

---

## 2. UI / 交互 / “反人类”设计问题

### U-01：工作流画布没有可用的连线创建与可视化，核心卖点失效

**严重级别：P0 / blocker**  
**位置：**

- `desktop/Ariadne.Desktop/Views/WorkspacePageView.axaml`
- `desktop/Ariadne.Desktop/ViewModels/WorkspacePageViewModel.cs`

**问题：**

当前 Workspace UI 可以添加节点、拖动节点、编辑已有 `Edges` 列表，但没有看到：

- 从端口拖拽创建边；
- “添加边”命令；
- 节点之间的线条绘制；
- 控制边/数据边/communication 边的颜色区分；
- 端口 handle 的可交互入口。

`WorkspacePageViewModel` 也没有 `AddEdgeCommand` 之类的命令；`Edges` 主要来自加载已有 workflow。

**影响：**

这对一个“GUI 编辑节点流程图来构建工作流”的应用是致命 UX：用户能摆节点，却不能直观连接节点。

**建议修复：**

1. 节点显示端口：exec/data/communication 分区。
2. 鼠标从端口拖线到端口创建 edge。
3. 画布绘制 Bezier/orthogonal edge。
4. 不同 edge kind 用不同颜色/线型：control 实线、data 蓝线、communication 橙色双向线。
5. 选中边后右侧编辑配置。

---

### U-02：没有 Undo/Redo，误操作成本过高

**严重级别：P1 / high**  
**位置：**

- `WorkspacePageViewModel.cs`

**问题：**

删除、剪切、移动节点、修改边配置、批量打包等操作没有 Undo/Redo。

虽然部分危险动作有确认，但确认不能代替撤销。用户一旦误删复杂节点，只能手工恢复。

**建议修复：**

- Command Pattern：每个画布操作封装为可逆 command。
- 快捷键：`Ctrl+Z` / `Ctrl+Shift+Z`。
- 工具栏加 Undo/Redo 按钮。
- 保存前的 snapshot 不能替代细粒度 undo。

---

### U-03：页面切换每次重建 ViewModel，状态丢失

**严重级别：P1 / high**  
**位置：**

- `MainWindowViewModel.cs::CreatePage`
- `MainWindowViewModel.cs::SelectNavigationItemAsync`

**问题：**

每次切换导航都会：

```csharp
"workspace" => new WorkspacePageViewModel(...)
"settings" => new SettingsPageViewModel(...)
```

未缓存页面实例。

**影响：**

- 工作区选择状态、Project AI 输入、日志筛选/滚动、设置页当前 section 都会丢。
- 即使有 `IUnsavedChangesGuard`，保存后的临时 UI 状态也会消失。

**建议修复：**

- 为每个项目维护 `_pageCache`。
- 切换项目/离开项目时清空 cache。
- 对运行日志等页面保留滚动/筛选状态。

---

### U-04：Auto Mode 放标题栏且无二次确认，是高危误触设计

**严重级别：P1 / high**  
**位置：**

- `MainWindow.axaml`
- `MainWindowViewModel.cs::AutoModeEnabled / SetAutoModeAsync`

**问题：**

Auto Mode 开关在标题栏预算条旁边，切换后直接：

```csharp
_ = SetAutoModeAsync(value);
```

无二次确认，无风险说明。

**影响：**

Auto Mode 会影响确认项策略、预算预授权、写回流程。放在标题栏很容易被误触，尤其靠近窗口控制区。

**建议修复：**

- 移入“设置 > 自动化”。
- 如保留标题栏，开启时必须弹出确认：说明会自动审批哪些操作、预算风险、如何关闭。
- 开关旁显示状态与风险 badge。

---

### U-05：Pause/Stop/Resume 按钮给出虚假可控感

**严重级别：P1 / high**  
**位置：**

- `WorkspacePageView.axaml`
- `WorkspacePageViewModel.cs`
- 见 L-01 / L-03

**问题：**

UI 顶部有 Pause/Stop/Resume，但受当前架构限制，它们不能中断正在执行的 run，也不能真正继续已暂停的 run。

**影响：**

这是反人类设计：用户以为可以控制成本和风险，实际上点击后只是改了数据库状态或甚至按钮不可用。

**建议修复：**

先修运行架构，再让 UI 反映真实状态。运行期间：

- 立即显示 run id；
- Pause/Stop 可用；
- 显示“正在停止/已停止/停止失败”；
- Resume 调用真实继续执行。

---

### U-06：右侧配置面板固定 280px，编辑长 prompt/edge template 很痛苦

**严重级别：P2 / medium**  
**位置：**

- `WorkspacePageView.axaml`

**问题：**

右侧面板：

```xml
<Border Width="280" ...>
```

prompt template、communication template、edge data JSON 都在 280px 宽里编辑。

**建议修复：**

- 使用 `GridSplitter` 调整宽度。
- 设置 `MinWidth=260`、`MaxWidth=560`。
- 记住用户偏好。
- 长文本编辑用弹出式大编辑器。

---

### U-07：画布没有缩放、小地图、平移体系

**严重级别：P2 / medium**  
**位置：**

- `WorkspacePageView.axaml.cs::FitViewToNodes`

**问题：**

当前只有 context menu 里的 Fit View，而且 `FitViewToNodes` 本质上只是把最小 x/y 平移到 48；没有 zoom、minimap、滚轮缩放、无限画布平移。

节点拖动还被 clamp 到当前视口范围内，复杂工作流很难展开。

**建议修复：**

- 画布 transform 支持 scale + translate。
- 鼠标滚轮缩放，空格/中键拖拽平移。
- 工具栏显示 `+ / - / 100% / fit`。
- 右上角 minimap。

---

### U-08：节点运行状态只有文字，没有颜色/图标/实时进度

**严重级别：P2 / medium**  
**位置：**

- `WorkspacePageView.axaml`
- `WorkflowNodeViewModel.StatusText`

**问题：**

节点卡片边框是固定 `Ariadne.NodeBorder`，状态只靠 `StatusText`。没有 running/succeeded/failed/paused 的颜色编码。

更关键的是，当前 UI 也没有持续读取 `get_workflow_run_state` 来刷新每个节点的 runtime 状态。

**建议修复：**

- 节点状态色：Queued 灰、Running 蓝色脉冲、Succeeded 绿、Failed 红、Paused 橙。
- 节点右上角状态图标。
- run event stream 推动实时刷新。

---

### U-09：确认项入口隐藏，且没有画布级提示

**严重级别：P2 / medium**  
**位置：**

- `WorkspacePageView.axaml`
- `WorkspacePageViewModel.cs::LoadConfirmationsAsync`

**问题：**

确认项在 Workspace 右侧面板 Project AI 区域里，用户不一定能发现。badge 也不是实时事件流更新。

**建议修复：**

- 有 pending confirmation 时画布顶部显示 banner。
- 点击 banner 打开右侧确认项。
- 自动刷新确认项列表。
- pending 节点在画布上高亮。

---

### U-10：DialogScrim 覆盖整个窗口，包括标题栏

**严重级别：P2 / low-medium**  
**位置：**

- `MainWindow.axaml`

**问题：**

全局弹窗遮罩是 `Panel` 内最后一层，覆盖整个客户区，包括自定义标题栏。弹窗期间用户不能移动窗口、最小化或关闭窗口，只能先处理弹窗。

**建议修复：**

- 遮罩只覆盖内容区，标题栏保留可交互。
- 或至少窗口控制按钮浮在遮罩之上。

---

### U-11：设置页混合“即时保存”和“需要保存”，撤销语义混乱

**严重级别：P2 / medium**  
**位置：**

- `SettingsPageViewModel.cs::SelectedLanguage`
- `SettingsPageViewModel.cs::ConfirmLeaveIfNeededAsync`
- `SettingsPageViewModel.cs::SelectTabAsync`

**问题：**

语言切换会立即：

```csharp
_displayNames.SwitchLanguage(value);
_ = PersistLanguageAsync(value);
```

但设置页整体又有 `HasUnsavedChanges` 和离开确认。用户会以为取消/丢弃可以撤销所有设置，但语言可能已经写入。

此外，设置页内部 tab 切换也会触发未保存离开确认，用户想在多个 tab 间对照配置会很烦。

**建议修复：**

- 明确区分“立即生效”与“待保存”字段。
- 立即生效字段旁标注“已自动保存”。
- 设置页内部 tab 切换不应等同于离开整个页面。

---

### U-12：预算 `$0/$0` 仍然容易误导

**严重级别：P2 / medium**  
**位置：**

- `MainWindowViewModel.cs::ApplyBudgetStatus`
- `SettingsPageViewModel.cs::ApplyAutomation`

**问题：**

后端已把 `preauthorized_usd = 0` 映射成 None，避免 0 阻断；但标题栏仍显示 spent/budget 形式。`BudgetUsd <= 0` 时进度条宽度 0，但文本仍像“预算为 0”。

**建议修复：**

当 budget 为 0 或未设置时显示：

> 未设置预算 / 不限制；建议设置预算上限

而不是 `$0 / $0`。

---

### U-13：高级概念直接暴露为内部字段名，对普通作者不友好

**严重级别：P2 / medium**  
**位置：**

- `WorkspacePageView.axaml` edge editor
- `WorkspacePageViewModel.WorkflowEdgeViewModel`

**问题：**

UI 直接让用户编辑：

- `SourceHandle`
- `TargetHandle`
- `Label`（实际 data alias）
- `ForwardAlias`
- `ReverseAlias`
- raw `DataJson`

这些是工程内部概念。对目标用户“长篇小说作者”来说过于底层。

**建议修复：**

- data edge 用“把 A 的【输出字段】传给 B 的【输入名】”。
- control edge 用“执行顺序”。
- communication edge 用“谁先发起 / 最多几轮 / 正向提示 / 反向提示”。
- 隐藏原始 JSON，提供高级模式。

---

## 3. 旧问题复核

以下问题在当前 HEAD 中已经基本修复或部分修复：

- 默认模板 URL 不可达：`DEFAULT_TEMPLATE_REPOSITORY_URL` 已为空，未配置时返回友好错误。
- 模板 URL 协议：已要求 `http://` 或 `https://`。
- `run_id` 毫秒碰撞：已加 `simple_random_u16()` 后缀。
- 工具权限默认放行：`tool_control_enabled` 已改为 `unwrap_or(false)`。
- System keychain service 旧名：已改为 `ariadne`。
- `reachable_nodes_from_start` O(n²)：已引入 `HashSet`。
- Loop / resume 清理 communication 状态：已部分实现，但见 L-06，reset 不完整。
- Project AI tool schema 固定空对象：已允许读取 `tool_input_schema/input_schema`，但 UI 与自动生成仍缺失，见 L-11。
- 预算 0 阻断：preauthorized 0 已映射 None，但预算显示仍误导，见 U-12。

---

## 4. 建议修复优先级

### P0：必须先修，否则产品闭环不成立

1. **后端运行架构**：长生命周期 backend + job manager + event stream + cancellation。
2. **继续旧 run**：`continue_workflow_run(workflow_id, run_id)`，确认/断点/预算暂停后能真正继续。
3. **SecretStore 持久化**：桌面默认启用系统 keychain 或加密文件 fallback。
4. **工作流连线 UI**：可拖拽创建边，画布显示边。
5. **data edge 调度语义**：data source 未完成时 target 不可运行。

### P1：高风险与高痛点

1. 预算 preflight / reservation。
2. 修复 confirmation policy `manual` 映射与双字段模型缺口。
3. `reset_communication_edges_for_nodes` 重置 `next_sender_node_id` 与 hash。
4. `open_project` 与显式 workflow id 缺失校验。
5. Undo/Redo 与页面 ViewModel 缓存。
6. Auto Mode 二次确认与风险说明。

### P2：体验完善

1. zoom/minimap/pan。
2. 节点状态颜色与实时进度。
3. 右侧面板可拖拽宽度。
4. 确认项 banner 与自动刷新。
5. 设置页保存语义统一。
6. 发布脚本同时构建 Rust backend。

---

## 5. 一句话结论

Ariadne 的 core 里已经有不少认真设计的模块（权限、运行快照、RAG、成本账本、确认项、通信边等），但当前 desktop 与 runtime 的交接还没有形成真正可用的“长任务工作台”。最大短板不是某个小 bug，而是：**后端进程生命周期、运行控制、恢复执行、密钥持久化、画布连线编辑这五个核心闭环没有打通**。先把这五个 P0 修掉，产品才会从“漂亮的配置壳”变成真正能让作者安全编排长篇写作流程的工具。
