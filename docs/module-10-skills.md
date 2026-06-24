# Module 10 PromptTemplate / Workflow / ExecutorAdapter

Module 10 的主轴调整为三类资源：

- `PromptTemplate`: 可复用、可参数化、可内联的提示词模板。
- `Workflow`: 完整流程模板，导入后复制展开为普通节点和连线。
- `ExecutorAdapter`: 旧 LLM / HTTP / WASM Skill 执行契约的迁移名称，后续作为节点执行后端。

## 已实现文件

- `src-tauri/src/skills/models.rs`: ExecutorAdapter 兼容模型、PromptTemplate manifest、Workflow manifest、SemVer、hash 和 trace 模型。
- `src-tauri/src/skills/loader.rs`: ExecutorAdapter、PromptTemplate、Workflow 三类资源加载器。
- `src-tauri/src/skills/executor.rs`: ExecutorAdapter 执行器契约、权限、预算、墙钟超时、输出限制和日志脱敏。
- `src-tauri/src/rag/prompt_template.rs`: 节点提示词模板渲染、`{{template.xxx(...)}}` 内联、参数校验和 trace 生成。
- `src-tauri/tests/skill_contracts.rs`: Module 10 契约测试。

## PromptTemplate

- manifest 文件名固定为 `prompt_template.json`。
- 默认目录为 `prompt_templates/`，加载器允许配置全局和项目根目录。
- 固定版本 key 为 `template_id@version`。
- 项目同 id 同版本模板覆盖全局模板。
- 版本号使用 SemVer `major.minor.patch`。
- 节点引用必须锁定 `template_id`、`version` 和 `content_hash`。
- 新版本只返回可更新状态，不自动改变旧节点行为。
- 模板内联使用 `{{template.文风约束(风格="克制")}}`。
- `skill` 命名空间已废弃；工程名统一使用 PromptTemplate。

## Workflow

- manifest 文件名固定为 `workflow.json`。
- 默认目录为 `workflows/`。
- Workflow manifest 声明 workflow id、version、节点、边、PromptTemplate 依赖、节点类型、工具和权限。
- 导入 Workflow 时返回普通 `WorkflowDefinition` 副本。
- 导入后的节点和连线可编辑，不与源模板自动同步。
- 工作流边支持 `data`、`control`、`feedback` 三类。
- `control` 边必须从 `exec_out` 连接到 `exec_in`。
- `feedback` 边必须携带有限 `max_communication_count`，用于 Prudent -> Writer 等返修通信。

## ExecutorAdapter

- 旧 `skill.json` 继续兼容读取，类型别名为 `ExecutorAdapterManifest`。
- 项目 ExecutorAdapter 覆盖全局同 id manifest 时仍按项目优先生效，并可通过加载器覆盖诊断列表提示用户。
- LLM ExecutorAdapter 复用 Module 7 `LlmService` 和 provider/cost 链路。
- HTTP ExecutorAdapter 执行前必须通过 `PermissionRequest::HttpSkill`。
- WASM ExecutorAdapter 默认无网络；声明网络访问时必须逐 host 通过 `PermissionRequest::WasmNetwork`。
- manifest 必须设置非零 timeout 和 max output。
- 执行前按 `estimated_cost_usd` 走 Module 2 预算评估。
- 后端自报耗时和客户端墙钟耗时任一超过 timeout 都会被拒绝。
- 日志会脱敏 Authorization、API key、token、secret、password 等敏感值。

## 当前边界

- HTTP 和 WASM 当前是 trait 后端，测试使用 mock；真实网络客户端和 WASM runtime 后续接入这些 trait。
- WASM 文件系统能力尚未开放；后续接真实 runtime 时必须继续通过 Module 0 文件权限沙箱。
- PromptTemplate 参数校验实现 JSON schema 子集：`required` 与 `properties`。
- 运行 trace 不保存展开后的完整 prompt，只保存 hash、依赖和输入来源映射。

## 验证

- `cargo fmt`
- `cargo test --test skill_contracts --quiet`
- `cargo test --quiet`
