use serde_json::{json, Value};

use crate::contracts::{CoreError, CoreResult};
use crate::rag::line_patch::line_numbered_text;
use crate::rag::memory::MemoryWritingKnowledgeBase;
use crate::rag::models::{
    RegisterContent, RegisteredChangeStatus, WritingAgentKind, WritingContextBundle,
    WritingContextRequest, WritingContextSection,
};
use crate::rag::reference::{
    contains_content_reference, expand_content_references, ReferenceDocumentSource,
    ReferenceExpansionLimits, ReferenceWarning,
};

/// 写作节点上下文组装器；一个节点就是一个 agent。
pub struct WritingContextAssembler<'a> {
    knowledge: &'a MemoryWritingKnowledgeBase,
    /// 13-B：正文引用 `{{ref:...}}` 的文档来源。
    ///
    /// 没挂来源时，含引用的区块会 **fail-loud**（见 `expand_section_references`），
    /// 而不是把 `{{ref:...}}` 字面量送进 LLM 请求体。
    reference_documents: Option<&'a dyn ReferenceDocumentSource>,
    /// 引用展开的长度与条数护栏，恒为 `Default`。
    ///
    /// U116：曾有一个 `with_reference_limits(limits)` builder 可覆盖它，但从落地起
    /// 就零调用者——护栏值本身是**安全边界**（单条最大长度、单次最多条数），
    /// 让调用方随意放宽等于把边界交给最不该决定它的那一层。
    /// 真要按项目调，正确落点是配置层而不是 builder。所以直接留 `Default`，
    /// 需要时从配置读，不要再加回那个 setter。
    reference_limits: ReferenceExpansionLimits,
}

impl<'a> WritingContextAssembler<'a> {
    /// 创建上下文组装器。
    pub fn new(knowledge: &'a MemoryWritingKnowledgeBase) -> Self {
        Self {
            knowledge,
            reference_documents: None,
            reference_limits: ReferenceExpansionLimits::default(),
        }
    }

    /// 13-B：挂上正文引用的文档来源，使大纲里的 `{{ref:...}}` 就地展开为原文。
    ///
    /// 由调用方注入而非组装器自己读盘：解析 `document_id` 需要项目根、路径沙箱与
    /// 章节索引，那些都在 `documents` / `commands` 层，沙箱责任应留在已经持有
    /// `ensure_path_under_root` 的那一层。
    pub fn with_reference_documents(mut self, documents: &'a dyn ReferenceDocumentSource) -> Self {
        self.reference_documents = Some(documents);
        self
    }

    /// 按节点/agent 类型组装上下文。
    pub fn assemble(&self, request: WritingContextRequest) -> CoreResult<WritingContextBundle> {
        validate_chapter_id(&request.chapter_id)?;
        let mut sections = match request.agent {
            WritingAgentKind::Outliner => self.outliner_sections(&request)?,
            WritingAgentKind::Designer => self.designer_sections(&request)?,
            WritingAgentKind::Planner => self.planner_sections(&request)?,
            WritingAgentKind::Detail => self.detail_sections(&request)?,
            WritingAgentKind::Writer => self.writer_sections(&request)?,
            WritingAgentKind::Critic => self.critic_sections(&request)?,
            WritingAgentKind::Prudent => self.prudent_sections(&request)?,
            WritingAgentKind::Polisher => self.polisher_sections(&request)?,
            WritingAgentKind::Summarizer => self.summarizer_sections(&request)?,
        };

        // 13-B 约束 ③：展开在**唯一一处**收口，就在全部区块装配完之后。
        //
        // 刻意不放进各 `*_sections`：那样每新增一个 agent 就多一次「记得展开」的
        // 机会，而漏掉的那一条就是一个会把 `{{ref:...}}` 送进 LLM 请求体的安全
        // 缺口（约束 ②）。在这里收口意味着**没有任何 agent 路径能绕过它**。
        let warnings = self.expand_section_references(&mut sections)?;

        Ok(WritingContextBundle {
            agent: request.agent,
            chapter_id: request.chapter_id,
            sections,
            metadata: merge_reference_warnings(request.metadata, &warnings)?,
        })
    }

    /// 就地展开全部区块里的正文引用，并把引用坐标记进 `section.sources`。
    ///
    /// 返回全部警告，交由调用方落进运行日志与确认项 metadata。
    ///
    /// **没挂文档来源却遇到引用 → fail-loud。** 三个候选做法里只有这个是对的：
    /// - 原样放过 → `{{ref:...}}` 进 LLM 请求体，Auto Mode 的审计 LLM 会在「审的
    ///   是占位符」的前提下给出虚假通过（约束 ②），这是安全倒退；
    /// - 静默删掉 → Writer 以为那条指示后面本来就没有原文，人也看不出少了什么；
    /// - fail-loud → 报错点名是哪个区块、缺什么，用户当场知道该配什么。
    fn expand_section_references(
        &self,
        sections: &mut [WritingContextSection],
    ) -> CoreResult<Vec<ReferenceWarning>> {
        let mut warnings = Vec::new();
        for section in sections.iter_mut() {
            if !section_expands_content_references(&section.section_id) {
                continue;
            }
            if !contains_content_reference(&section.content) {
                continue;
            }
            let Some(documents) = self.reference_documents else {
                return Err(CoreError::validation(format!(
                    "writing context section `{}` contains {{{{ref:...}}}} content references but \
                     no reference document source is configured; wire \
                     WritingContextAssembler::with_reference_documents so the excerpts are \
                     expanded before the text reaches any model",
                    section.section_id
                )));
            };
            let mut expansion =
                expand_content_references(&section.content, documents, &self.reference_limits)?;
            // ⚠️ 正文用 `take` 换出来、不做 `expansion.text` 的字段移动：
            // `source_spans(&self)` 与 `merge_section_expansion_metadata(&expansion)`
            // 后面都还要借用整个 `expansion`，而字段移动会让它**部分移动**，
            // 之后任何借用都编译失败（E0382）。
            // 用 `take` 而不是 `clone`：正文可能是整章数千字，克隆正是本项目
            // 「引用式数据流」要避免的拷贝。
            section.content = std::mem::take(&mut expansion.text);
            // 展开后置条件：这个区块里**不能**再有 `{{ref:` 残留。
            //
            // 这条断言不是防御性冗余，它钉住的是约束 ②「任何送进 LLM 请求体的路径
            // 都必须展开」。展开器内部有若干「无法展开」的分支（文档缺失、越权、
            // 超条数、嵌套），每一条都必须把占位符换成可诊断标记；哪天有人新加一
            // 条分支忘了替换，占位符就会一路进到请求体，而 Auto Mode 的审计 LLM
            // 会在「审的是占位符」的前提下给出虚假通过。在这里 fail-loud，比在
            // 生产里静默批准变更好得多。
            if contains_content_reference(&section.content) {
                return Err(CoreError::validation(format!(
                    "writing context section `{}` still contains {{{{ref:...}}}} after expansion; \
                     refusing to hand unexpanded content references to a model",
                    section.section_id
                )));
            }
            // 引用坐标进 `sources`：这是既有字段（`rag/models.rs`），正好用来记录
            // 本区块展开了哪些引用，供审计溯源「Writer 看到的这段来自哪里」。
            section.sources.extend(expansion.source_spans());
            section.metadata = merge_section_expansion_metadata(
                std::mem::take(&mut section.metadata),
                &expansion,
            )?;
            warnings.extend(expansion.warnings);
        }
        Ok(warnings)
    }

    /// Outliner 上下文用于全局开局规划，重点接收用户初始意图和已有长期知识。
    fn outliner_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        // U175：两个区块无条件产出。`node_template.outliner.default` 无条件引用
        // `{{用户初始意图}}` 与 `{{已有全局总纲}}`——首次开局时二者天然都没有
        // （总纲正是这个节点要写出来的东西），缺席即渲染失败，
        // 恰好把「新项目开第一篇」这条最该顺畅的路径堵死。
        let mut sections = vec![
            section_or_absent(
                "user_intent",
                "用户初始意图",
                non_empty_optional(&request.user_intent),
                "（作者没有另外交代初始构想：请按本节点提示词自行拟定，\
                 或在上游用数据边把创作意图传进来）",
                Value::Null,
            ),
            section_or_absent(
                "global_outline",
                "已有全局总纲",
                non_empty_optional(&request.global_outline),
                "（暂无全局总纲：这是开篇，总纲由你从零拟定）",
                Value::Null,
            ),
        ];
        let character_state =
            current_character_and_relationship_state(&self.knowledge.registered_changes()?);
        if !character_state.is_empty() {
            sections.push(section(
                "character_state",
                "人物与关系当前状态",
                character_state,
                Value::Null,
            ));
        }
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }

    /// Designer 上下文用于阶段粒度规划，包含全局总纲、既有阶段总纲和章节概括。
    fn designer_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        // U175：四个区块无条件产出，`node_template.designer.default` 全都无条件引用。
        let mut sections = vec![
            section_or_absent(
                "global_outline",
                "全局总纲",
                non_empty_optional(&request.global_outline),
                "（暂无全局总纲：请先让全局纲领节点写出总纲，或按本节点提示词自行把握全局）",
                Value::Null,
            ),
            section_or_absent(
                "previous_stage_outline",
                "之前阶段总纲",
                non_empty_optional(&request.previous_stage_outline),
                "（这是第一个阶段，没有需要承接的上一阶段）",
                Value::Null,
            ),
            section_or_absent(
                "stage_outline",
                "既有阶段总纲",
                non_empty_optional(&request.stage_outline),
                "（本阶段尚无总纲：由你从零拟定）",
                Value::Null,
            ),
            section_or_absent(
                "chapter_summaries",
                "章节概括",
                non_empty_optional(&request.chapter_summaries),
                "（暂无章节概括：本阶段还没有已写就的章节）",
                Value::Null,
            ),
        ];
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }

    /// Planner 上下文包含前文总结、人物当前状态、未回收伏笔和上一章正文。
    fn planner_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        // U175：三个 outline 类区块也要无条件产出。
        // 此前只有下面那三个「知识库派生」的是无条件的，这三个仍是条件产出——
        // 而 `node_template.planner.default` 对 `{{全局总纲}}`
        // `{{当前阶段总纲}}` 同样是无条件引用。
        let mut sections = vec![
            section_or_absent(
                "global_outline",
                "全局总纲",
                non_empty_optional(&request.global_outline),
                "（暂无全局总纲）",
                Value::Null,
            ),
            section_or_absent(
                "stage_outline",
                "当前阶段总纲",
                non_empty_optional(&request.stage_outline),
                "（暂无本阶段总纲：请按全局总纲与前文自行判断本章的位置）",
                Value::Null,
            ),
            section_or_absent(
                "chapter_summaries",
                "当前阶段章节概括",
                non_empty_optional(&request.chapter_summaries),
                "（暂无本阶段章节概括）",
                Value::Null,
            ),
        ];
        // ⚠️ 下面三个区块是**知识库派生**的，必须**无条件产出**，空时给明确空态文本。
        //
        // 原实现是 `if !xxx.is_empty()`：空知识库下整个区块不入 sections，
        // 而 `node_template.planner.default` **无条件**引用 `{{前文总结}}`
        // `{{人物与关系当前状态}}` `{{未回收伏笔}}` 三者，于是
        // **新项目写第一章时 Planner 节点必然渲染失败**（渲染器对未知变量 fail-loud）——
        // 恰好是最该顺畅的那条路径。这不是 U149 引入的，从更早就有，
        // 只因当时的用例只渲染 writer 模板而没被发现。
        //
        // 为什么修装配层而不是把模板改成条件引用：渲染器没有条件语法，
        // 而「让每个模板作者自己记住哪几个变量可能缺席」是把陷阱留在原地。
        // 空态文本也比缺席更好——它明确告诉模型「这里查过了，确实没有」，
        // 而缺席会让模型以为是自己没拿到资料。
        let chapter_summaries = self.knowledge.chapter_summaries()?;
        sections.push(section(
            "previous_summaries",
            "前文总结",
            if chapter_summaries.is_empty() {
                "（暂无前文总结：这是开篇，没有需要承接的既有情节）".to_owned()
            } else {
                format_ordered_map(&chapter_summaries)
            },
            Value::Null,
        ));

        let character_state =
            current_character_and_relationship_state(&self.knowledge.registered_changes()?);
        sections.push(section(
            "character_state",
            "人物与关系当前状态",
            if character_state.is_empty() {
                "（暂无已登记的人物与关系：人物可在本章首次登场）".to_owned()
            } else {
                character_state
            },
            Value::Null,
        ));

        let foreshadowing = self.knowledge.unresolved_foreshadowing()?;
        sections.push(section(
            "unresolved_foreshadowing",
            "未回收伏笔",
            if foreshadowing.is_empty() {
                "（暂无未回收伏笔）".to_owned()
            } else {
                foreshadowing
                    .iter()
                    .map(|record| format!("- {}: {}", record.title, record.description))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            Value::Null,
        ));

        // U175 / 「上一章原文」的处置：**降级为空态说明，不接线读盘**。
        //
        // 取「章节 id → 上一章文档」需要一套章节顺序与命名约定（chapter-02 的上一章
        // 是 chapter-01？chapter-1？序号带补零吗？分卷时跨卷怎么算？），
        // 那套约定在本项目尚未确立——CLAUDE.md 记着「故本次不猜测」这个决定。
        // 猜错的代价是**把错的章节当上一章喂给模型**，比没有更糟：作者要在成稿里
        // 才发现文风承接错了对象。
        //
        // 但「不猜测」不能等于「节点必然失败」（那正是 U175）。所以这里明确告知
        // 模型这项材料当前不供给、以及作者可以怎么手动给（数据边）。
        // 真要接线，正确落点是先确立章节目录约定，再在 `commands.rs` 装配处
        // （持有项目根与路径沙箱的那一层）读盘填 `previous_chapter_text`。
        sections.push(section_or_absent(
            "previous_chapter_text",
            "上一章全文",
            non_empty_optional(&request.previous_chapter_text),
            "（未提供上一章正文：本项目尚未确立章节→文档的目录约定，系统不会自行猜测是哪一篇。\
             需要承接上一章时，请用数据边把上一章正文传进本节点）",
            Value::Null,
        ));
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }

    /// Detail 上下文聚焦当前章节大纲和已有总结，不直接塞 Writer 草稿。
    fn detail_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let mut sections = Vec::new();
        // 同 planner_sections 的理由：`node_template.detail.default` 无条件引用
        // `{{章节总结}}`，而这个区块原先只在知识库里已有该章总结时才产出——
        // 于是「先写细节、还没做总结」这条正常顺序下 Detail 节点必然渲染失败。
        // 空态文本比缺席好：它明确告诉模型「这章还没有总结」。
        sections.push(section(
            "chapter_summary",
            "章节总结",
            self.knowledge
                .chapter_summary(&request.chapter_id)?
                .unwrap_or_else(|| "（本章暂无总结：尚未写作或尚未生成总结）".to_owned()),
            Value::Null,
        ));
        // U175：`本章大纲` 无条件产出。detail 的工作方式是「随大纲每一步备料」
        // （见 agent_prompt.detail），没有大纲时必须明说，否则模型会凭空造素材。
        sections.push(section_or_absent(
            "outline",
            "本章大纲",
            non_empty_optional(&request.outline),
            "（本章暂无大纲：请把章节大纲节点用数据边连到本节点，或在本节点提示词里\
             直接交代要为哪些场景备料）",
            Value::Null,
        ));
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }

    /// Writer 上下文包含大纲、细节、上一章和带行号草稿；不默认塞未回收伏笔。
    fn writer_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        // U175：五个区块无条件产出。`node_template.writer.default` 引用了
        // `{{上一章原文}}` `{{本章大纲}}` `{{本章细节}}` `{{返修上下文}}` 四个，
        // 生产装配处只填得上 `current_draft_text`（而模板还偏偏没引用它）
        // ⇒ 拖一个「执笔」节点上画布直接运行，四个变量全解析不出来。
        let mut sections = vec![
            // 「上一章原文」同 planner：不猜测章节→文档映射，见那里的长注释。
            section_or_absent(
                "previous_chapter_text",
                "上一章全文",
                non_empty_optional(&request.previous_chapter_text),
                "（未提供上一章正文：本项目尚未确立章节→文档的目录约定，系统不会自行猜测是哪一篇。\
                 需要沿用上一章文风时，请用数据边把上一章正文传进本节点）",
                Value::Null,
            ),
            section_or_absent(
                "outline",
                "本章大纲",
                non_empty_optional(&request.outline),
                "（本章暂无大纲：请把章节大纲节点用数据边连到本节点，\
                 或在本节点提示词里直接交代这一章要写什么）",
                Value::Null,
            ),
            section_or_absent(
                "details",
                "本章细节",
                non_empty_optional(&request.details),
                "（没有备好的细节素材：需要时自行落实到可感知的具体事物）",
                Value::Null,
            ),
        ];
        // 带行号正文：空态不编行号（同 polisher_sections 的理由——
        // 给空串编上 `1: ` 会让模型对着不存在的正文调行号修改工具）。
        sections.push(match non_empty_optional(&request.current_draft_text) {
            Some(draft) => section(
                "line_numbered_draft",
                "带行号正文",
                line_numbered_text(draft),
                json!({ "line_numbered": true }),
            ),
            None => section(
                "line_numbered_draft",
                "带行号正文",
                "（本章还没有正文：这是从零起笔，不要调用行号修改工具——行号无所指）".to_owned(),
                Value::Null,
            ),
        });
        sections.push(section_or_absent(
            "revision_context",
            "审慎者返修上下文",
            non_empty_optional(&request.revision_context),
            "（不是返修：这是初稿，没有需要照着改的意见）",
            Value::Null,
        ));
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }

    /// Critic 上下文用于评价正文，可接入待评价文本、章节/阶段规划和上游 alias。
    fn critic_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        // U175：三个区块一律无条件产出，缺席时给空态文本。
        //
        // `node_template.critic.default` 无条件引用 `{{待评价文本}}`
        // `{{本章大纲}}` `{{阶段总纲}}`，而生产装配处（`integration.rs` 的
        // `render_writing_node_prompt`）对 critic 只可能填到 `target_text`
        // ——于是拖一个「意见者」节点上画布、用预填提示词运行必然渲染失败。
        let mut sections = vec![
            section_or_absent(
                "target_text",
                "待评价文本",
                non_empty_optional(&request.target_text),
                "（没有收到待评价的正文：请把产出正文的节点用数据边连到本节点）",
                Value::Null,
            ),
            section_or_absent(
                "outline",
                "本章大纲",
                non_empty_optional(&request.outline),
                "（本章暂无大纲：请按正文自身的完成度评价，不要臆测大纲要求）",
                Value::Null,
            ),
            section_or_absent(
                "stage_outline",
                "阶段总纲",
                non_empty_optional(&request.stage_outline),
                "（暂无阶段总纲）",
                Value::Null,
            ),
        ];
        append_template_inputs(&mut sections, &request.template_inputs)?;
        // ⚠️ 原先这里是 `if sections.is_empty() { return Err("requires target_text") }`。
        // 那道守卫**从来没拦住过真正的问题**，只拦住了产品自己：`target_text` 在生产
        // 恒为 `None`（装配处没填），所以它把「拖一个意见者节点直接运行」判成错误。
        // 「没有正文可评」这件事现在由空态文本明确告知模型，比整个节点 failed 好——
        // 后者让用户完全无从判断是自己配错了还是产品坏了。
        Ok(sections)
    }

    /// Prudent 上下文接收一个或多个 Critic 输出，并形成返修判断依据。
    fn prudent_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        // U175：同 critic_sections 的理由，三个区块无条件产出。
        // `node_template.prudent.default` 无条件引用 `{{评审意见}}`
        // `{{待评价文本}}` `{{本章大纲}}`，而生产装配处一个都填不上。
        let mut sections = vec![
            section_or_absent(
                "critic_outputs",
                "意见者输出",
                non_empty_optional(&request.critic_outputs),
                "（没有收到评审意见：请把产出评审意见的节点用数据边连到本节点）",
                Value::Null,
            ),
            section_or_absent(
                "target_text",
                "待评价文本",
                non_empty_optional(&request.target_text),
                "（没有收到被评的正文：请把产出正文的节点用数据边连到本节点）",
                Value::Null,
            ),
            section_or_absent(
                "outline",
                "本章大纲",
                non_empty_optional(&request.outline),
                "（本章暂无大纲）",
                Value::Null,
            ),
        ];
        append_template_inputs(&mut sections, &request.template_inputs)?;
        // ⚠️ 原先这里 `return Err("prudent context requires critic_outputs")`。
        // 与 critic 同一个病：`critic_outputs` 生产恒为 `None`，这道守卫的唯一
        // 实际效果是让「拖一个审稿裁断节点直接运行」必然失败。
        Ok(sections)
    }

    /// Polisher 上下文必须包含当前正文，并至少包含 Critic 或 Prudent 返修依据之一。
    fn polisher_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        let critic_outputs = non_empty_optional(&request.critic_outputs);
        let revision_context = non_empty_optional(&request.revision_context);
        // 带行号正文是 polisher 的工作对象，也是行号 patch 工具的坐标系。
        //
        // ⚠️ 空态时**不能**走 `line_numbered_text`：那会给空串编上一个 `1: ` 行号，
        // 模型可能据此调用 `polisher-replace-lines(1, 1, ...)` 去改一段不存在的正文。
        // 空态文本必须是明摆着不像正文的说明。
        let mut sections = vec![match non_empty_optional(&request.current_draft_text) {
            Some(draft) => section(
                "line_numbered_draft",
                "带行号正文",
                line_numbered_text(draft),
                json!({ "line_numbered": true }),
            ),
            None => section(
                "line_numbered_draft",
                "带行号正文",
                "（没有拿到正文：请在本节点上指定要润色的文档，或把产出正文的节点用数据边连过来。                 没有正文时不要调用行号修改工具——行号无所指）"
                    .to_owned(),
                Value::Null,
            ),
        }];
        // U175：`意见者输出` / `审慎者返修上下文` / `返修依据` 三者都无条件产出。
        // `node_template.polisher.default` 引用的是 `{{返修依据}}`（两者的合并），
        // 而它此前只在至少一个来源非空时才产出。
        sections.push(section_or_absent(
            "critic_outputs",
            "意见者输出",
            critic_outputs,
            "（没有收到评审意见）",
            Value::Null,
        ));
        sections.push(section_or_absent(
            "revision_context",
            "审慎者返修上下文",
            revision_context,
            "（没有收到返修要求）",
            Value::Null,
        ));
        let revision_basis = [critic_outputs, revision_context]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");
        sections.push(section_or_absent(
            "revision_basis",
            "返修依据",
            non_empty_str(&revision_basis),
            "（没有收到具体的返修依据：请把意见者或审稿裁断节点用数据边连到本节点。             缺少依据时不要自行大改，只做明显的文字修正）",
            Value::Null,
        ));
        sections.push(section_or_absent(
            "outline",
            "本章大纲",
            non_empty_optional(&request.outline),
            "（本章暂无大纲）",
            Value::Null,
        ));
        append_template_inputs(&mut sections, &request.template_inputs)?;
        // ⚠️ 原先这里有两道 `return Err(...)`（要求 current_draft_text、
        // 要求 critic_outputs 或 revision_context）。两道守的字段在生产装配处
        // 都填不上（只有 `current_draft_text` 在节点指名了 document_id 时有值），
        // 所以它们的实际效果就是让「拖一个润色节点直接运行」必然失败。
        // 现在这些前提由空态文本明确告知模型，节点本身可运行。
        Ok(sections)
    }

    /// Summarizer 上下文接收当前正文草稿，后续由流水线执行器消费结构化结果。
    fn summarizer_sections(
        &self,
        request: &WritingContextRequest,
    ) -> CoreResult<Vec<WritingContextSection>> {
        // U175：`当前章节正文` 无条件产出。
        //
        // ⚠️ 这一支在**当前**产品里走不到：summarizer 节点走的是四步总结生产链
        // （`execute_summarizer_node_*`），它的 prompt 经 `author_template_prefix()`
        // 原文拼接，**不过 `render_prompt_template`**，所以不会调用本函数。
        // 但装配器是公开契约、`WritingAgentKind::Summarizer` 是合法入参，
        // 留一个「凑不齐上下文就整个失败」的分支只会在将来接线时重演 U175。
        let mut sections = vec![section_or_absent(
            "chapter_text",
            "当前章节正文",
            non_empty_optional(&request.current_draft_text),
            "（没有拿到章节正文：请把产出正文的节点用数据边连到本节点）",
            Value::Null,
        )];
        append_template_inputs(&mut sections, &request.template_inputs)?;
        Ok(sections)
    }
}

/// 生成上下文区块。
fn section(
    section_id: impl Into<String>,
    title: impl Into<String>,
    content: impl Into<String>,
    metadata: Value,
) -> WritingContextSection {
    WritingContextSection {
        section_id: section_id.into(),
        title: title.into(),
        content: content.into(),
        sources: Vec::new(),
        metadata,
    }
}

/// 产出一个上下文区块；值缺席时用**明确的空态文本**代替，而不是不产出区块。
///
/// U175：这是本文件的核心不变量。产品自带的 `node_template.{agent}.default`
/// **无条件**引用它那几个变量，而 `render_prompt_template` 对未知变量 fail-loud
/// ⇒ 区块缺席 = 别名不登记 = 用户拖一个该类型节点上画布、用预填提示词点运行
/// **必然报错**（实测 9 个自带模板里 8 个如此，见 U175 报告）。
///
/// 三个候选做法里只有空态文本是对的：
/// - **区块缺席** → 渲染器 fail-loud，节点 failed、零次出站请求。产品自带的
///   默认值成了一个必然失败的配置，这是 U175 本身。
/// - **静默替换成空串** → 比留字面量更糟：模型会以为「这一章本来就没有大纲」，
///   于是自由发挥，而人看不出少喂了材料。
/// - **空态文本** → 明确告诉模型「这项查过了，确实没有」，模型据此调整行为
///   （比如没有上一章就不必强行承接），人读请求体也一眼看得出缺什么。
///
/// ⚠️ 这**不会**放过拼错的变量名：空态只给产品自己认识的那些 section
/// （即各 `*_sections` 里显式列出的），`{{这个变量根本不存在}}` 依旧
/// fail-loud（`unknown_prompt_placeholder_fails_loudly` 守的就是这条）。
fn section_or_absent(
    section_id: &str,
    title: &str,
    value: Option<&str>,
    absent_note: &str,
    metadata: Value,
) -> WritingContextSection {
    match value {
        Some(value) => section(section_id, title, value.to_owned(), metadata),
        // 空态文本刻意不带 metadata：它不是真实材料，不该被当作可溯源的内容。
        None => section(section_id, title, absent_note.to_owned(), Value::Null),
    }
}

/// 将上游数据边 alias 展开为模板可引用的上下文区块。
fn append_template_inputs(
    sections: &mut Vec<WritingContextSection>,
    inputs: &std::collections::BTreeMap<String, String>,
) -> CoreResult<()> {
    for (alias, content) in inputs {
        if alias.trim().is_empty() {
            return Err(CoreError::validation(
                "template input alias cannot be empty",
            ));
        }
        if content.trim().is_empty() {
            continue;
        }
        sections.push(section(
            format!("input.{alias}"),
            alias.clone(),
            content.clone(),
            json!({ "from_template_input": true }),
        ));
    }
    Ok(())
}

/// 格式化有序摘要表。
fn format_ordered_map(values: &std::collections::BTreeMap<String, String>) -> String {
    values
        .iter()
        .map(|(key, value)| format!("- {key}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 从已落地注册项中生成当前人物和关系状态。
fn current_character_and_relationship_state(
    changes: &[crate::rag::models::RegisteredChange],
) -> String {
    changes
        .iter()
        .filter(|change| change.status == RegisteredChangeStatus::Realized)
        .filter_map(|change| match &change.content {
            RegisterContent::CharacterProfile(content) => Some(format!(
                "- 人物 {}({}): {}; 初始状态 {}",
                content.name, content.character_id, content.narrative_role, content.initial_state
            )),
            RegisterContent::CharacterPlan(content) => Some(format!(
                "- 人物计划 {}: {} 于 {} 承担 {}; 目标 {}",
                content.plan_id,
                content.character_id,
                match (&content.stage_id, &content.chapter_id) {
                    (Some(stage_id), Some(chapter_id)) =>
                        format!("阶段 {stage_id} / 章节 {chapter_id}"),
                    (Some(stage_id), None) => format!("阶段 {stage_id}"),
                    (None, Some(chapter_id)) => format!("章节 {chapter_id}"),
                    (None, None) => "未指定范围".to_owned(),
                },
                content.narrative_function,
                content.appearance_goal
            )),
            RegisterContent::CharacterTrait(content) => Some(format!(
                "- {} / {}: {}",
                content.character, content.trait_name, content.to_value
            )),
            RegisterContent::Relationship(content) => Some(format!(
                "- {} 与 {} / {}: {}",
                content.character_a,
                content.character_b,
                content.relationship_name,
                content.to_value
            )),
            RegisterContent::ThemeAnchor(content) => {
                Some(format!("- 主题 {}: {}", content.title, content.statement))
            }
            RegisterContent::Foreshadowing(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 返回非空字符串切片；全空白视作缺席。
///
/// 与 `non_empty_optional` 的区别只是入参不是 `Option`——`revision_basis`
/// 是就地拼出来的 `String`，没有 `Option` 外壳。
fn non_empty_str(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// 返回非空可选字符串。
fn non_empty_optional(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// 校验章节 id。
fn validate_chapter_id(chapter_id: &str) -> CoreResult<()> {
    if chapter_id.trim().is_empty() {
        return Err(CoreError::validation("chapter_id cannot be empty"));
    }
    Ok(())
}

/// 哪些区块允许含正文引用。
///
/// 13-B 的引用是 **Planner 写进大纲**的，所以放行的是大纲类与规划类区块，以及
/// 数据边灌进来的 `input.*`（Planner 的产出常经数据边传给 Writer 节点）。
///
/// **正文类区块刻意不放行**（`previous_chapter_text` / `line_numbered_draft` /
/// `chapter_text` / `target_text`）：那些是小说正文本身。若正文里恰好出现
/// `{{ref:` 字样（作者写了一段讲模板语法的对话、或粘了段配置），展开它等于按
/// 用户正文里的字符串去读别的文件——把内容当指令，属注入面。放行清单是白名单
/// 而非黑名单，正是为了让「新增区块默认不可注入」。
///
/// 带行号正文尤其不能展开：行号是 patch 工具的坐标系，展开会插入若干行，
/// 使模型数出来的 `after_line` 与真实文档全部错位。
fn section_expands_content_references(section_id: &str) -> bool {
    section_id.starts_with("input.")
        || matches!(
            section_id,
            "outline"
                | "details"
                | "global_outline"
                | "stage_outline"
                | "previous_stage_outline"
                | "chapter_summaries"
                | "revision_context"
                | "revision_basis"
                | "critic_outputs"
        )
}

/// 把本区块的展开结果记进区块 metadata。
///
/// 人类 UI 靠这份结构化列表做折叠渲染（约束 ③：上游只产出展开态，折叠是纯前端
/// 的显示变换）。`expanded_range` 给出展开块在**展开后文本**里的位置，正是折叠时
/// 要替换掉的那一段。
fn merge_section_expansion_metadata(
    metadata: Value,
    expansion: &crate::rag::reference::ExpandedOutline,
) -> CoreResult<Value> {
    let mut map = match metadata {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("previous_metadata".to_owned(), other);
            map
        }
    };
    map.insert(
        "expanded_content_references".to_owned(),
        serde_json::to_value(&expansion.expanded)?,
    );
    Ok(Value::Object(map))
}

/// 把展开警告汇总进 bundle metadata。
///
/// 警告必须随 bundle 一起往上走，否则「引用失效 / 被截断」只能靠肉眼在正文里找
/// 失效标记——而确认项 payload 与运行日志都读 metadata。无警告时不插入这个键，
/// 免得每个 bundle 都多一个空数组。
fn merge_reference_warnings(
    metadata: Value,
    warnings: &[crate::rag::reference::ReferenceWarning],
) -> CoreResult<Value> {
    if warnings.is_empty() {
        return Ok(metadata);
    }
    let mut map = match metadata {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("previous_metadata".to_owned(), other);
            map
        }
    };
    map.insert(
        "content_reference_warnings".to_owned(),
        serde_json::to_value(warnings)?,
    );
    Ok(Value::Object(map))
}
