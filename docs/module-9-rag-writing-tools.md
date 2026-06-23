# Module 9 RAG 与创作工具

Module 9 按 `创作总结机制(不可删除).md` 优先落地创作 RAG 基础。当前实现先固定工具契约、内置提示词资源、创作知识模型、内存索引和 Writer 行号 patch，真实工作流编排与 UI 审批后续模块继续接入。

## 已实现文件

- `src-tauri/resources/prompt_list.json`: 内置工具提示词、自动审计提示词和 Summarizer 提示词。
- `src-tauri/resources/display_name.json`: agent、tool、operation 和 confirmation item 显示名称。
- `src-tauri/src/rag/models.rs`: 故事段、事件、注册项、伏笔、四类写作节点定义、问题队列、确认项、find/search 响应和总结流水线报告模型。
- `src-tauri/src/rag/numbering.rs`: 故事段十进制字符串编号排序和中点生成，避免使用浮点数。
- `src-tauri/src/rag/resources.rs`: 内置资源加载和必需 key 校验。
- `src-tauri/src/rag/memory.rs`: 内存版创作知识库和双向索引。
- `src-tauri/src/rag/context.rs`: Planner、Detail、Writer、Summarizer 节点上下文组装。
- `src-tauri/src/rag/pipeline.rs`: Summarizer 结构化草稿应用、确认项和未落地问题队列处理。
- `src-tauri/src/rag/tools.rs`: Planner、Detail、Writer 的特化工具定义和执行器。
- `src-tauri/src/rag/line_patch.rs`: Writer 1-based 行号操作到 `DocumentPatch` 的转换。
- `src-tauri/tests/rag_contracts.rs`: Module 9 契约集成测试。

## 契约规则

- 提示词正文和 UI 显示名从资源 JSON 读取，代码不硬编码中文 prompt/display name。
- 故事段只保存 `SourceSpan`，默认 find 结果不复制正文。
- 显式 `include_text=true` 且执行器持有当前文档上下文时，才按 `SourceSpan` 回填正文片段。
- 故事段编号使用字符串小数，例如 `1`、`1.5`、`1.25`。
- 事件状态为 `ongoing`、`paused`、`completed`。
- Planner register 的内容使用强类型 JSON schema，覆盖人物性格、人物关系和伏笔。
- 注册项生命周期为 `planned -> realized`；删除错误操作结果使用 `deleted`，不表达角色死亡。
- 伏笔状态为 `planned`、`planted`、`recovered`、`abandoned`；Planner 默认可查询未回收伏笔。
- Planner、Detail、Writer 的 find/search 工具使用不同工具名和资源描述，底层执行共享同一套安全逻辑。
- Planner、Detail、Writer、Summarizer 都是独立节点；一个节点就是一个 agent。
- 本地 find 与外部 search 分离；SearchProvider 结果返回 `persisted_to_knowledge=false`，不自动入库。
- Writer 暴露 `writer-find`、`writer-search`、`writer-insert-lines` 和 `writer-replace-lines`。
- Writer 的 search 描述明确限定为“当需要知道这一处现实中的情况时调用”，且结果只用于当前写作判断，不允许自动注册。
- Writer 不具备 `planner-register`，不能注册新设定或伏笔。
- Writer 行号工具按 1-based 行号生成 UTF-8 byte-range patch。
- Planner 上下文默认包含前文总结、当前人物/关系状态、未回收伏笔和上一章正文。
- Detail 上下文聚焦当前章节大纲和已有章节总结。
- Writer 上下文包含上一章、本章大纲、本章细节和带行号草稿，不默认塞未回收伏笔。
- Summarizer 流水线按故事段、事件、章节、阶段顺序应用结构化草稿。
- 普通模式确认项默认 `pending`；Auto Mode 默认进入 `auto_audited`。
- 未落地的 `planned` 注册项会进入 Planner 问题队列。

## 当前边界

- 当前实现是内存版创作知识库，用于固定契约和测试；持久化存储、索引重建和真实混合召回仍在后续模块接入。
- Planner、Detail、Writer 和 Summarizer 已建模为四个内置写作节点。
- Summarizer 是独立节点，引用四段总结提示词和固定确认项顺序，但不暴露普通工具集合。
- 当前流水线执行器消费结构化草稿，不直接调用 LLM；真实 LLM 调度、审批 UI、失败暂停恢复和受影响步骤重跑放到工作流模块。
- `planner-search` / `detail-search` 支持可选 `SearchProvider` adapter；默认未配置 provider 时返回明确错误。

## 验证

- `cargo fmt`
- `cargo test --quiet`
- `cargo test --test rag_contracts --quiet`
