# Module 10 Skill 系统

Module 10 建立可复用 Skill 的基础契约。当前实现固定 manifest、typed ports、加载优先级、三类 executor 边界、权限检查、预算预估、超时、输出大小限制和日志脱敏。真实 HTTP 客户端和 WASM runtime 通过后端 trait 接入，避免默认执行不受控本机脚本。

## 已实现文件

- `src-tauri/src/skills/models.rs`: Skill manifest、输入输出 schema、运行限制、executor 配置、运行请求和输出模型。
- `src-tauri/src/skills/loader.rs`: 从全局目录和项目目录加载 `skill.json`，项目 Skill 覆盖同 id 全局 Skill。
- `src-tauri/src/skills/executor.rs`: LLM/HTTP/WASM Skill 执行器契约和统一安全检查。
- `src-tauri/src/skills/sanitizer.rs`: Skill 日志脱敏。
- `src-tauri/tests/skill_contracts.rs`: Module 10 契约测试。

## 契约规则

- Skill manifest 文件名固定为 `skill.json`。
- 项目 Skill 优先于全局 Skill。
- Skill schema 生成核心 `PortDefinition`，输出可连接到下游 typed ports。
- LLM Skill 复用 Module 7 `LlmService` 和现有 provider/cost 链路。
- HTTP Skill 执行前必须通过 `PermissionRequest::HttpSkill`。
- WASM Skill 默认无网络；声明网络访问时必须逐 host 通过 `PermissionRequest::WasmNetwork`。
- Skill manifest 必须设置非零 timeout 和 max output。
- Skill 执行前按 `estimated_cost_usd` 走 Module 2 预算评估。
- 后端输出超过 timeout 或 max output 会被拒绝。
- Skill 日志会脱敏 Authorization、API key、token、secret、password 等敏感值。

## 当前边界

- HTTP 和 WASM 当前是 trait 后端，测试使用 mock；真实网络客户端和 WASM runtime 后续接入这些 trait。
- WASM 文件系统能力尚未开放；后续接真实 runtime 时必须继续通过 Module 0 文件权限沙箱。
- 真实执行成本写账依赖后端或 provider 返回费用；当前先实现执行前预算预估阻断。

## 验证

- `cargo fmt`
- `cargo test --test skill_contracts --quiet`
- `cargo test --quiet`
- `cargo test --features system-keychain --no-run`
