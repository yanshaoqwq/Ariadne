# Ariadne Issue 清单

> 以下 Issue 按 `gh issue create` 格式整理，可直接在 `yanshaoqwq/Ariadne` 仓库创建。

---

## Issue 1：默认 SecretStore 不持久化，API Key 在进程重启后丢失

**Title:** 🔴 默认 SecretStore 不持久化 — API Key 在进程重启后丢失

**Labels:** bug, security, priority:high

**Body:**

## 问题描述

`default_secret_store()` 在未启用 `system-keychain` feature 时返回 `MemorySecretStore`，所有密钥存储在内存 `BTreeMap` 中。进程退出后密钥全部丢失。

**文件**: `core/src/commands.rs:88-93`

```rust
#[cfg(not(feature = "system-keychain"))]
pub fn default_secret_store() -> Arc<dyn SecretStore> {
    Arc::new(MemorySecretStore::default())
}
```

## 影响

- 用户每次重启桌面应用都要重新输入所有 API Key
- 大多数 Linux 桌面环境没有 keyring 服务，`system-keychain` feature 无法正常工作
- 严重影响用户信任和首次使用体验

## 建议修复

在非 keychain 环境下提供基于本地文件 + 应用级加密的 fallback 持久化方案：

1. 密钥写入 `~/.ariadne/secrets/` 下的加密文件
2. 使用 OS 提供的密钥派生（如 Linux `keyctl`、Windows DPAPI）或用户主密码
3. `system-keychain` feature 仍然作为首选，文件加密方案作为降级 fallback

---

## Issue 2：工作流执行同步阻塞 IPC 主线程

**Title:** 🔴 run_workflow_impl 同步阻塞 — 运行期间前端无法交互

**Labels:** bug, architecture, priority:high

**Body:**

## 问题描述

`run_workflow_impl`（`core/src/commands.rs:689-746`）在 IPC 主线程中同步执行整个工作流，包含 LLM API 调用、document 读写和多轮 communication。执行期间无法响应任何其他前端请求。

```rust
pub fn run_workflow_impl(
    project_root: &Path,
    secrets: &dyn SecretStore,
    request: RunWorkflowRequest,
) -> CommandResult<WorkflowRunStarted> {
    // ... 整个工作流执行在这里同步完成
    let status = runtime.run_persisted(&workflow, &mut executor, &store)?;
    // ...
}
```

## 影响

- 长时间运行工作流时前端完全"假死"
- 用户无法通过 pause/stop 命令中断正在运行的工作流
- 无法查询运行状态或处理确认项
- IPC 行协议是单线程处理，阻塞期间所有命令排队

## 建议修复

1. 将工作流执行移入独立线程或 tokio task
2. IPC 层仅返回 `run_id`，前端通过轮询或事件流获取状态
3. 引入 `CancellationToken` 供前端通过 IPC 中断执行
4. 长远考虑 WebSocket 或 SSE 替代 stdin/stdout 行协议

---

## Issue 3：默认模板仓库 URL 不可达

**Title:** 🟡 默认模板仓库 URL `templates.ariadne.local` 不可达

**Labels:** bug, ux

**Body:**

## 问题描述

```rust
// core/src/commands.rs:32
const DEFAULT_TEMPLATE_REPOSITORY_URL: &str = "https://templates.ariadne.local";
```

`.local` 域名仅在 mDNS 局域网环境下解析，公网不存在此域名。用户首次使用"模板集市"搜索必定失败，且错误信息不够友好。

## 影响

- 新用户看到"模板集市"功能完全不可用，但不知道原因
- 错误信息是底层网络超时，没有提示"模板仓库未配置"

## 建议修复

1. 改为真实可访问的 URL（如果已有模板服务）
2. 若尚无模板服务，默认 URL 留空，UI 中明确提示"模板仓库尚未配置"
3. 在 `search_templates` / `get_template_detail` 网络错误时返回更友好的中文提示

---

## Issue 4：run_id 基于毫秒时间戳，并发运行时存在碰撞风险

**Title:** 🟡 run_id 基于毫秒时间戳存在碰撞风险

**Labels:** bug, data-integrity

**Body:**

## 问题描述

```rust
// core/src/commands.rs:697
let run_id = RunId::from(format!("run-{}", now_timestamp_ms()?));
```

如果用户在 1ms 内连续点击运行（或程序化调用），两次运行会产生相同的 `run_id`，后一次会覆盖 `SqliteWorkflowRuntimeStore` 中前一次的记录。

## 影响

- 并发启动同一工作流时运行记录被覆盖
- 数据丢失，审计不完整

## 建议修复

加入随机后缀或 UUID：

```rust
let run_id = RunId::from(format!("run-{}-{:04x}", now_timestamp_ms()?, rand::random::<u16>()));
```

---

## Issue 5：工具控制默认策略过于宽松 — 未配置的工具默认放行

**Title:** 🟡 工具控制默认放行未配置的工具，违背最小权限原则

**Labels:** security, config

**Body:**

## 问题描述

```rust
// core/src/commands.rs:947-950
fn tool_control_enabled(
    controls: &BTreeMap<String, BTreeMap<String, bool>>,
    scope: &str,
    tool: &str,
) -> bool {
    controls
        .get(scope)
        .and_then(|scope_controls| scope_controls.get(tool).copied())
        .unwrap_or(true)  // 未配置的工具默认放行
}
```

未在 `tool_controls` 中显式列出的工具自动返回 `true`。新增的 agent 类型或自定义工具会在用户不知情时获得所有权限。

## 影响

- 新增 agent 类型的工具（如未来新增 `reviewer` agent）会自动获得执行权限
- 自定义 skill 的工具也默认放行
- 违背最小权限原则

## 建议修复

默认值改为 `false`，新工具需显式启用：

```rust
.unwrap_or(false)
```

同时在 `default_permission_tool_controls()` 中为已知工具提供默认启用配置，确保升级后现有工具不受影响。

---

## Issue 6：Loop 重跑不重置 communication 边状态

**Title:** 🔴 advance_loop 不重置 communication 边状态，返修循环静默失效

**Labels:** bug, workflow-runtime, priority:high

**Body:**

## 问题描述

`advance_loop`（`core/src/workflow/runtime.rs:826-838`）在重跑时清理了 control 闭包内节点的 `nodes` 快照，但没有清理 `communication_edges` 中涉及这些节点的状态（`message_count`、`completed` 等）。

如果上一轮 communication 已完成（`completed = true`），重跑后下游节点不会再次触发 communication，返修循环静默失效。

## 复现场景

1. 工作流包含 Writer → Critic → Prudent → Writer 的通信边
2. 第一轮 communication 完成后 Loop 节点决定继续
3. `advance_loop` 清理了 Writer/Critic/Prudent 的节点快照
4. 但 communication 边的 `completed = true` 仍然保留
5. 第二轮 Writer 运行后 `advance_communication` 检测到 `completed = true`，跳过通信
6. Critic 和 Prudent 不会再收到消息，返修循环失效

## 建议修复

在 `advance_loop` 清理 control 闭包时，同步重置涉及被清理节点的 communication 边状态：

```rust
for edge_id in affected_edges {
    if let Some(comm) = self.state.communication_edges.get_mut(&edge_id) {
        comm.completed = false;
        comm.message_count = 0;
        // 保留 initiator 设置，其余重置
    }
}
```

---

## Issue 7：resume_from_node 不重置 loop 迭代计数

**Title:** 🟡 resume_from_node 不重置 loop_iterations，路径 A 恢复后立即再次暂停

**Labels:** bug, workflow-runtime

**Body:**

## 问题描述

`resume_from_node`（`core/src/workflow/runtime.rs:659-695`）在路径 A 注入正文后，清理了下游节点快照，但 `loop_iterations` 中的计数未清零。

如果之前循环已耗尽 `max_iterations`，注入后工作流恢复时会立即再次触发迭代上限暂停，无法真正继续。

## 建议修复

对被清理的 loop 节点同时重置 `loop_iterations`：

```rust
for downstream in &closure {
    if downstream != node_id {
        self.state.nodes.remove(downstream);
        self.state.loop_iterations.remove(downstream);  // 新增
    }
}
```

---

## Issue 8：确认策略双表示映射有损

**Title:** 🟡 确认策略 legacy_policy ↔ dual_policy 映射有损，保存后策略被静默篡改

**Labels:** bug, config

**Body:**

## 问题描述

`legacy_policy_from_dual_policy`（`core/src/commands.rs:1253-1261`）将 `(AllowByDefault, AutoApproval)` 和 `(ManualReview, AutoApproval)` 都映射为 `"auto_audit"`，反向解析时丢失了 `normal_policy` 的差异。

```rust
fn legacy_policy_from_dual_policy(
    normal_policy: ConfirmationNormalPolicy,
    auto_mode_policy: ConfirmationAutoModePolicy,
) -> String {
    match (normal_policy, auto_mode_policy) {
        (ConfirmationNormalPolicy::AllowByDefault, ConfirmationAutoModePolicy::AllowByDefault) => "auto_skip",
        (_, ConfirmationAutoModePolicy::AllowByDefault) => "auto_skip",  // ← AllowByDefault + AutoApproval 也映射为 auto_skip
        (_, ConfirmationAutoModePolicy::AutoApproval) => "auto_audit",
    }
}
```

保存设置后读取，普通模式策略被静默篡改。

## 影响

- 用户在 UI 中设置"普通模式=手动审批，Auto Mode=自动审批"
- 保存后读取变成"普通模式=默认放行"（因为被映射为 `auto_skip`）
- 策略配置不可靠

## 建议修复

1. 完全废弃 legacy `policy` 字段
2. 只保留 `normal_policy` + `auto_mode_policy` 双字段序列化
3. 删除 `legacy_policy_from_dual_policy` 和 `policies_from_legacy_policy` 的双向转换逻辑

---

## Issue 9：collect_data_inputs 静默跳过无 alias 的数据边

**Title:** 🟡 数据边缺少 alias 时静默跳过，节点拿不到输入但无报错

**Labels:** bug, ux, workflow-runtime

**Body:**

## 问题描述

```rust
// core/src/workflow/runtime.rs:1010-1014
for edge in workflow.edges.iter()
    .filter(|edge| edge.kind == WorkflowEdgeKind::Data && edge.to.node_id == *node_id)
{
    let Some(alias) = &edge.alias else { continue; };  // ← 静默跳过
```

数据边如果用户忘了设 alias，节点拿不到该输入但没有任何错误或警告。排错极其困难。

## 建议修复

二选一：

1. **运行时保护**：在 `validate_topology` 中要求数据边必须有 alias
2. **降级警告**：运行时对无 alias 的数据边产生 warning 事件，写入 `structured_events`

---

## Issue 10：工作流工具定义 schema 为空 — LLM 无法传参

**Title:** 🟡 项目空间 AI 工具 input_schema 为空对象，start 节点参数无法传递

**Labels:** bug, feature

**Body:**

## 问题描述

```rust
// core/src/commands.rs:1048-1055
fn project_ai_tool_definitions(workflow_tools: &[ProjectWorkflowTool]) -> Vec<ToolDefinition> {
    workflow_tools.iter().map(|tool| ToolDefinition {
        input_schema: json!({"type": "object", "properties": {}, "additionalProperties": false}),
        ...
    }).collect()
}
```

所有项目空间 AI 工具的 `input_schema` 都是空对象。如果 start 节点定义了需要参数的端口，LLM 无法通过 tool call 传递参数。

## 建议修复

从 start 节点的 input_ports / config 定义自动生成 `input_schema`，将端口名映射为 JSON Schema properties。

---

## Issue 11：SystemKeychainSecretStore 默认 service 名过时

**Title:** 🟡 SystemKeychainSecretStore 默认 service 名为 "literature-agent"，与项目名不一致

**Labels:** bug, config

**Body:**

## 问题描述

```rust
// core/src/config/secrets.rs:119-121
impl Default for SystemKeychainSecretStore {
    fn default() -> Self {
        Self::new("literature-agent")
    }
}
```

项目已更名为 Ariadne，但 keychain service 名仍为旧名 "literature-agent"。已有的旧密钥无法被新版本读取。

## 建议修复

1. 将默认 service 名改为 `"ariadne"`
2. 在迁移逻辑中尝试从 `"literature-agent"` 读取旧密钥并迁移到 `"ariadne"`

---

## Issue 12：validate_project_root 仅检查路径非空

**Title:** 🟡 validate_project_root 仅检查路径非空，不验证目录存在性和初始化状态

**Labels:** bug, robustness

**Body:**

## 问题描述

```rust
// core/src/commands.rs:1089-1093
fn validate_project_root(project_root: &Path) -> CommandResult<()> {
    if project_root.as_os_str().is_empty() {
        return Err("project_root cannot be empty".to_owned());
    }
    Ok(())
}
```

不检查目录是否存在、是否已初始化（有无 `.config/`）、是否为文件而非目录。传入不存在的路径会静默创建空配置，降级为"新项目"。

## 建议修复

对 `open_project` 操作增加：
1. 目录存在性检查
2. 是否为目录（而非文件）
3. 是否包含 `.config/` 目录（已初始化标志）

`create_project` 可跳过已存在性检查（因为会创建新目录）。

---

## Issue 13：IPC 传输 API Key 为明文

**Title:** 🟡 IPC 行协议中 API Key 明文传输，存在泄露风险

**Labels:** security

**Body:**

## 问题描述

`save_provider_key`（`core/src/ipc.rs`）将 API Key 作为 JSON 明文字段通过 stdin/stdout 行协议传输。

如果其他进程读取 `/proc/{pid}/fd/0` 或用户终端有回显，密钥可能泄露。

## 建议修复

1. 桌面环境中通过环境变量传递密钥
2. 或使用 Unix domain socket + 文件权限保护
3. 至少确保 stdin 不被回显到日志或 stdout

---

## Issue 14：模板仓库 URL 不校验协议，存在 SSRF 风险

**Title:** 🟡 模板仓库 base_url 不校验协议，可被注入 file:/// 或内网地址

**Labels:** security

**Body:**

## 问题描述

```rust
// core/src/commands.rs:646-649
if settings.base_url.trim().is_empty() {
    return Err("template repository base_url cannot be empty".to_owned());
}
```

仅检查非空，不校验是否为合法 `http://` 或 `https://` URL。用户填入 `file:///etc/passwd`、`http://169.254.169.254/latest/meta-data/` 等会直接传给 `reqwest`。

## 建议修复

```rust
fn validate_template_url(url: &str) -> CommandResult<()> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(()),
        other => Err(format!("template URL must use http or https, got {other}")),
    }
}
```

---

## Issue 15：预算默认 0 但无 UI 警告，新用户无法运行 LLM 节点

Title: 🟡 预算显示 $0.0/$0.0 易引误解；手动设为 0.0 会阻断所有 LLM 调用

Labels: ux, config

问题描述：

BudgetConfigFile 的 budget_usd 默认为 0.0，UI 预算条显示 $0.0/$0.0。但这并不阻断 LLM 调用——实际预算执行使用 BudgetLimits，其 daily_usd/monthly_usd/single_call_usd 默认均为 None，exceeds(None, value) 返回 false，即默认无日/月限额。

存在两个问题：

UX 误导：$0.0/$0.0 让用户以为无法运行 LLM，但实际默认可以运行（单次 >$1 需确认，<$1 直接放行）
0.0 footgun：如果用户在设置中手动将日限额/月限额/预授权额度设为 0.0（误以为 0 = 无限制），exceeds(Some(0.0), any_positive) = true 会阻断所有 LLM 调用
建议修复：

UI 在 budget_usd = 0 时显示"预算未设置，LLM 调用默认无限制"提示
validate_money 对限额字段将 0.0 解释为"无限制"（映射为 None），或在 UI 中明确标注"0 表示禁止"
预算条在 budget_usd = 0 时使用灰色+提示文案替代空进度条

---

## Issue 16：导航切换每次重建 ViewModel，页面状态全部丢失

**Title:** 🔴 导航切换每次重建 ViewModel — 页面状态全部丢失

**Labels:** ux, priority:high, frontend

**Body:**

## 问题描述

```csharp
// desktop/…/ViewModels/MainWindowViewModel.cs:207-218
private object CreatePage(string id, string key) {
    return id switch {
        "workspace" => new WorkspacePageViewModel(_displayNames, _backend),
        ...
    };
}
```

每次切换侧栏页签都 `new` 一个全新 ViewModel。用户在工作空间编辑了提示词但未保存、在日志页滚动了位置、在设置页修改了表单——切换后再切回，一切丢失。

## 影响

- 操作连续性极差
- 用户养成"不敢切换页签"的习惯
- 配合缺少 Undo 功能，误操作代价极高

## 建议修复

缓存 ViewModel 实例：

```csharp
private readonly Dictionary<string, object> _pageCache = new();

private object CreatePage(string id, string key) {
    if (_pageCache.TryGetValue(id, out var cached)) return cached;
    var page = id switch { ... };
    _pageCache[id] = page;
    return page;
}
```

切换项目时清空缓存。

---

## Issue 17：画布无撤销/重做功能

**Title:** 🔴 画布操作无 Undo/Redo，误操作不可逆

**Labels:** ux, feature, priority:high, frontend

**Body:**

## 问题描述

画布上删除节点、修改连线、拖拽位置都不可逆。没有 Undo 栈。用户误删一个精心配置的节点后只能手动重建。

## 建议修复

1. 实现 Command Pattern 维护 undo/redo 栈
2. 每次画布操作（增删改节点/边/位置）封装为可逆 Command
3. 快捷键 `Ctrl+Z` / `Ctrl+Shift+Z`
4. 工具栏显示 undo/redo 按钮

---

## Issue 18：确认项审阅入口隐藏太深

**Title:** 🟡 确认项审阅入口隐藏在右侧面板深处，新用户难以发现

**Labels:** ux, frontend

**Body:**

## 问题描述

确认项嵌套在"工作空间 → 右侧面板 → 项目 AI 标签页"下方。用户不知道有待审项，且 sidebar badge 只显示数字，没有画布级的视觉提示。

## 建议修复

1. 待审项出现时在画布上方弹出横幅（toast/banner），点击直达审阅面板
2. 右侧面板自动展开并切换到确认项区域
3. badge 增加脉冲动画吸引注意

---

## Issue 19：Auto Mode 开关放在标题栏过于突出，容易误触

**Title:** 🟡 Auto Mode 开关放在标题栏，危险操作缺少保护

**Labels:** ux, security, frontend

**Body:**

## 问题描述

Auto Mode 开关位于标题栏预算条旁边，与窗口控制按钮同层。开启后系统会自动审批所有写回操作，是高风险操作，但开关位置容易误触且无二次确认。

## 建议修复

1. 将 Auto Mode 开关移入自动化设置页
2. 或保留当前位置但增加二次确认弹窗："开启 Auto Mode 后系统将自动审批写回操作，是否确认？"
3. 开关旁增加简短风险说明文字

---

## Issue 20：画布缺少缩放控件和小地图

**Title:** 🟢 画布缺少缩放控件和小地图，大工作流导航困难

**Labels:** ux, feature, frontend

**Body:**

## 问题描述

大工作流（20+ 节点）在固定视口中无法快速定位。缺少缩放滑块和适应视图按钮（仅右键菜单有"适应视图"），无小地图。

## 建议修复

1. 画布工具栏增加缩放控件（+/-/fit/百分比显示）
2. 右上角悬浮小地图（minimap），可点击快速跳转
3. 支持鼠标滚轮缩放和 Ctrl+拖拽平移

---

## Issue 21：右侧面板固定 280px 不可调

**Title:** 🟢 右侧面板固定 280px 不可调，编辑长提示词空间不足

**Labels:** ux, frontend

**Body:**

## 问题描述

```xml
<Border Width="280" ...>
```

编辑长提示词模板时 280px 宽度严重不足，需要大量滚动。用户无法拖拽调整宽度。

## 建议修复

改用 `GridSplitter` 允许手动拖拽，设置 `MinWidth="240" MaxWidth="500"`，并记住用户偏好宽度。

---

## Issue 22：画布节点无运行状态视觉反馈

**Title:** 🟢 画布节点无运行状态颜色编码，无法快速定位运行/失败节点

**Labels:** ux, frontend

**Body:**

## 问题描述

节点卡片上仅有文字 `StatusText`，没有颜色编码或图标表示 running/succeeded/failed/paused。在 10+ 节点的工作流中，用户无法一眼看出哪个节点正在运行、哪个失败了。

## 建议修复

节点卡片边框颜色映射运行状态：
- 排队中 → 灰色
- 运行中 → 蓝色脉冲动画
- 已成功 → 绿色
- 已失败 → 红色
- 已暂停 → 橙色

---

## Issue 23：侧栏折叠后完全消失，无图标模式

**Title:** 🟢 侧栏折叠后只剩一个按钮，无图标中间态

**Labels:** ux, frontend

**Body:**

## 问题描述

折叠侧栏后只剩一个展开按钮，无法看到导航图标和 badge。现代桌面应用通常有"仅图标"中间态（~48px 宽）。

## 建议修复

实现三态：完整（204px）→ 图标（48px）→ 隐藏。图标态仍显示导航图标和 badge 数字。

---

## Issue 24：弹窗遮罩覆盖标题栏，无法最小化/移动窗口

**Title:** 🟢 DialogScrim 覆盖标题栏，弹窗期间无法操作窗口控制

**Labels:** ux, frontend

**Body:**

## 问题描述

```xml
<Border x:Name="DialogScrim" Background="#66000000" IsVisible="{Binding Dialog.IsOpen}" ...>
```

`DialogScrim` 覆盖整个客户区包括标题栏。弹窗显示期间无法最小化或移动窗口。

## 建议修复

遮罩仅覆盖内容区域（Grid.Row="1"），标题栏（Grid.Row="0"）保持可交互。

---

## Issue 25：reachable_nodes_from_start 使用 Vec 做去重，O(n²) 复杂度

**Title:** 🟢 reachable_nodes_from_start 使用 Vec::contains 去重，大工作流性能劣化

**Labels:** performance

**Body:**

## 问题描述

```rust
fn reachable_nodes_from_start(workflow: &WorkflowDefinition, start_node_id: &NodeId) -> Vec<NodeId> {
    let mut reachable = Vec::new();
    // ...
    if reachable.contains(&node_id) { continue; }  // O(n) 查找
```

`Vec::contains` 是 O(n) 查找，整体复杂度 O(n²)。大型工作流（数百节点）时性能劣化。

## 建议修复

改用 `HashSet<NodeId>` 做 O(1) 去重查找。

---

## 快速创建脚本

将以上 Issue 保存后，可使用以下 shell 脚本批量创建：

```bash
#!/bin/bash
# 在 yanshaoqwq/Ariadne 仓库根目录运行
# 需要 gh CLI 已登录

REPO="yanshaoqwq/Ariadne"

gh issue create --repo "$REPO" \
  --title "🔴 默认 SecretStore 不持久化 — API Key 在进程重启后丢失" \
  --label "bug,security" \
  --body-file /dev/stdin <<'EOF'
（粘贴 Issue 1 Body 内容）
EOF

# 重复以上模式...
```
