# Ariadne 用户指南

Ariadne 是一款面向长篇小说创作的桌面 AI 工作台。它把作品正文、项目记忆、写作流程、模型配置、模板集市和版本存档放在同一个应用里，帮助作者规划、撰写、修订和管理长篇项目。

## 适合做什么

- 管理小说项目：维护章节、正文、项目记忆、人物和主题线索。
- 搭建写作流程：把大纲、设计、写作、审稿、润色、总结等步骤编排成可运行的工作流。
- 使用多种模型：配置常用模型供应商，选择默认写作模型和向量检索模型。
- 复用模板：从模板集市导入工作流或预设，减少重复搭建。
- 控制自动化：设置预算、Auto Mode、确认项策略和工具权限。
- 保留版本：通过版本历史查看存档点，必要时回到指定版本。

## 开始使用

1. 打开 Ariadne 后，新建一个小说项目，或打开已有项目目录。
2. 进入“配置”页面，先完成模型和密钥设置。
3. 在“作品”页面导入或创建章节；项目记忆在“配置 > 通用”中维护。
4. 在“智能体编排”页面搭建工作流，拖入需要的节点并连接它们。
5. 运行工作流前，确认预算、权限和需要人工审批的确认项。
6. 写作过程中定期保存，并在“版本历史”中创建关键存档。

## 主要页面

### 智能体编排

这里用于搭建和运行工作流。左侧是节点库，中间是画布，右侧可以查看项目 AI、节点细节、连线配置和确认项。

常见操作：

- 从节点库添加写作 Agent 或通用节点。
- 拖动节点组织流程，连接节点间的数据或控制关系。
- 选中节点后配置提示词模板、模型、预算、超时和断点。
- 运行当前工作流，或从起始节点开始运行。
- 审阅确认项，通过或拒绝会影响后续运行。

### 作品

这里用于阅读、修改和导入章节正文。右侧面板包含目录、大纲入口和项目 AI；项目记忆在配置页维护，项目 AI 可以读取它。

常见操作：

- 在目录中打开章节。
- 在阅读/修改模式之间切换。
- 保存正文。
- 使用快捷 AI 对选中文本进行改写。
- 导入外部稿件，或合并导出 Markdown、EPUB、PDF。

### 模板集市

这里用于搜索和导入工作流模板。模板如果声明了额外权限，导入前会要求确认。

### 配置

配置页面按用途分为几个模块：

- 通用：项目名称、作品目录、工作流目录等基础信息。
- 模型：模型供应商、可用模型、默认模型、向量检索模型和服务密钥。
- 预设：不同节点类型的新建默认模型、预算和超时，以及模板来源入口。
- 自动化：预算、Auto Mode、确认项策略和运行限制。
- 权限控制：网络、工具、路径读写等权限。
- 个性化：主题、颜色、工作台显示偏好。
- 杂项：检索参数、版本存档、语言、诊断状态。

### 运行日志

这里用于查看节点、工具、模型、成本、确认项和诊断事件。遇到运行异常时，先在这里查看最近日志。

## 给其它程序调用

Ariadne 可以作为命令式工具或本地 REST 服务被其它程序、脚本和 agent 调用。所有入口都会复用同一套后端 command，不需要绕过桌面 UI 直接读写数据库。

### 命令式调用

先设置项目根目录，再使用 `ariadne-ipc call` 发起单次 JSON 调用：

```bash
ARIADNE_PROJECT_ROOT=/path/to/project ariadne-ipc call get_current_project
```

启动工作流会立即返回运行编号，不会等待整条流程结束：

```bash
ARIADNE_PROJECT_ROOT=/path/to/project ariadne-ipc call run_workflow '{"workflow_id":"default","start_node_id":"start-main"}'
```

随后可以按事件序号增量轮询结构化运行事件：

```bash
ARIADNE_PROJECT_ROOT=/path/to/project ariadne-ipc call get_workflow_events '{"workflow_id":"default","run_id":"run-xxx","after_sequence":0,"limit":50}'
```

也可以直接订阅运行事件，命令会持续输出 JSONL，直到该 run 停止、成功或失败：

```bash
ARIADNE_PROJECT_ROOT=/path/to/project ariadne-ipc watch-events default run-xxx 0
```

命令输出为统一 JSON：成功时 `ok=true` 且结果在 `data` 中；失败时 `ok=false` 且错误在 `error` 中。无参数运行 `ariadne-ipc` 或显式使用 `ariadne-ipc stdio` 时，会进入长驻 JSONL 模式，适合桌面端或需要复用同一后端进程的集成。

### 本地 REST 服务

需要跨语言或常驻调用时，可以启动本地 REST 服务。默认只建议绑定 `127.0.0.1`，并且必须设置 bearer token：

```bash
ariadne-server --project /path/to/project --bind 127.0.0.1:4817 --token <token>
```

常用接口：

- `GET /health`
- `GET /v1/tools/workflows`
- `POST /v1/workflows/{workflow_id}/runs`
- `GET /v1/workflows/{workflow_id}/runs/{run_id}`
- `GET /v1/workflows/{workflow_id}/runs/{run_id}/events?after_sequence=0&limit=50`
- `POST /v1/workflows/{workflow_id}/runs/{run_id}/pause`
- `POST /v1/workflows/{workflow_id}/runs/{run_id}/resume`
- `POST /v1/workflows/{workflow_id}/runs/{run_id}/stop`

除 `/health` 外，请求需要带 `Authorization: Bearer <token>`。也兼容 `/v1/projects/current/...` 形式的路径；首版 server 是单项目服务，`project_id` 只作为路由占位，不用于传任意本地路径。

### 版本历史

这里用于查看手动存档和自动存档。建议在重要章节完成、结构大改、批量润色前创建手动存档。

## 使用建议

- 模型配置完成前，不要直接运行需要 AI 生成内容的工作流。
- 对会写回正文、消耗预算或申请高权限的操作，建议保留人工确认。
- Auto Mode 适合稳定流程；新工作流第一次运行时建议手动审批。
- 项目记忆适合放长期设定，不适合放临时草稿。
- 修改大量章节前先创建版本存档。
- 模板来自外部来源时，先看权限声明再导入。

## 安全与确认

Ariadne 会在离开有未保存更改的页面前提醒保存、丢弃或取消。删除节点、剪切节点、停止运行、拒绝确认项、对全部节点执行批量操作等高影响动作也会要求再次确认，避免误触造成难以察觉的改动。

## 显示语言

当前正式界面语言以中文为基底。需要适配新语言时，可复制 `core/resources/display_name.json` 为 `core/resources/display_name.<语言代码>.json`，交给 AI 或翻译工具翻译同名键；桌面端会自动发现包含实际文案键的覆盖文件，缺失键回退中文。外部 agent 也可通过 IPC 调用 `get_display_name_language_pack_template` 导出翻译模板，并用 `validate_display_name_language_pack` 检查缺失、空值和多余 key。
