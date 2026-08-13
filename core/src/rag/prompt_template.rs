use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::contracts::{CoreError, CoreResult};
use crate::rag::models::{WritingAgentKind, WritingContextBundle, WritingContextSection};
use crate::rag::resources::PromptResources;
use crate::skills::PromptTemplateManifest;

/// 模板内联最大递归深度，防止 PromptTemplate 相互引用造成无限展开。
const MAX_TEMPLATE_RENDER_DEPTH: usize = 8;

// U116：曾有 `PromptTemplateBackup` / `NodePromptConfig`（含 `default_for_agent`
// 与 `replace_template`）这一整簇「节点提示词配置 + 编辑历史备份」模型，已删。
// 生产从不构造 `NodePromptConfig`：默认提示词由前端 `PromptCatalog.ResolveNodePrompt`
// 解析，用户编辑走 Avalonia 双向绑定直接改字符串，随节点 config 的 `prompt_template`
// 键保存；Rust 侧只消费那个裸字符串（见 `workflow/integration.rs` 调
// `render_prompt_template`）。备份历史这个功能在生产不存在，也没有对应 IPC 命令。
// 连带删除只服务于该类型的 `render_node_prompt` 与 `render_node_prompt_with_trace`。

/// 模板渲染上下文；输入来自节点上下文包和上游数据边 alias。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PromptTemplateContext {
    pub prompt_text: String,
    pub inputs: BTreeMap<String, String>,
    pub system: BTreeMap<String, String>,
    pub parameters: BTreeMap<String, String>,
    pub templates: BTreeMap<String, PromptTemplateManifest>,
    pub input_sources: BTreeMap<String, String>,
    /// 工作流变量的已渲染文本，对应 `{{var.名字}}`。
    ///
    /// 运行期由 runtime 从变量作用域链求值后灌入；这里只保存渲染成字符串的
    /// 结果，类型校验在 runtime 侧完成，模板层不做隐式转换。
    pub variables: BTreeMap<String, String>,
}

impl PromptTemplateContext {
    /// 根据 agent 默认提示词和上下文区块构造模板变量表。
    pub fn from_bundle(
        agent: WritingAgentKind,
        prompts: &PromptResources,
        bundle: &WritingContextBundle,
    ) -> CoreResult<Self> {
        let mut context = Self {
            prompt_text: prompt_text(prompts, agent.prompt_key())?,
            inputs: BTreeMap::new(),
            system: BTreeMap::new(),
            parameters: BTreeMap::new(),
            templates: BTreeMap::new(),
            input_sources: BTreeMap::new(),
            variables: BTreeMap::new(),
        };
        context
            .system
            .insert("当前章节号".to_owned(), bundle.chapter_id.clone());
        context
            .system
            .insert("agent".to_owned(), format!("{:?}", bundle.agent));

        for section in &bundle.sections {
            insert_section_aliases(&mut context.inputs, section);
        }
        Ok(context)
    }

    /// 注册可内联的 PromptTemplate，通常来自节点固定版本依赖。
    pub fn with_prompt_template(mut self, manifest: PromptTemplateManifest) -> CoreResult<Self> {
        manifest.validate()?;
        self.templates
            .insert(manifest.template_id.clone(), manifest);
        Ok(self)
    }

    /// 记录 input 变量来源，供运行 trace 审计使用。
    pub fn with_input_source(
        mut self,
        variable: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        self.input_sources.insert(variable.into(), source.into());
        self
    }

    /// 注入 `{{var.名字}}` 可读的工作流变量当前值。
    ///
    /// 取值来自 runtime 的变量作用域，已按「默认值 → AI 注入 → 人工输入 →
    /// 循环写回」的顺序结算完毕；这里只负责渲染，不再做覆盖判断。
    /// 数字与布尔按 JSON 字面量转文本（`3`、`true`），字符串取原文而非带引号的
    /// JSON 形式，避免模板里出现 `"第二章"`。
    pub fn with_variables(
        mut self,
        variables: impl IntoIterator<Item = (String, serde_json::Value)>,
    ) -> Self {
        for (name, value) in variables {
            let rendered = match value {
                serde_json::Value::String(text) => text,
                other => other.to_string(),
            };
            self.variables.insert(name, rendered);
        }
        self
    }
}

/// 渲染任意提示词模板；未知变量会报错，避免静默替换为空字符串。
pub fn render_prompt_template(
    template: &str,
    context: &PromptTemplateContext,
) -> CoreResult<String> {
    render_prompt_template_at_depth(template, context, 0)
}

/// 带递归深度的模板渲染实现。
fn render_prompt_template_at_depth(
    template: &str,
    context: &PromptTemplateContext,
    depth: usize,
) -> CoreResult<String> {
    if depth > MAX_TEMPLATE_RENDER_DEPTH {
        return Err(CoreError::validation(
            "prompt template expansion exceeded maximum depth",
        ));
    }
    let mut rendered = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        rendered.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            return Err(CoreError::validation(
                "prompt template has an unclosed variable",
            ));
        };
        let variable = after_start[..end].trim();
        if variable.is_empty() {
            return Err(CoreError::validation(
                "prompt template variable cannot be empty",
            ));
        }
        rendered.push_str(&resolve_variable(variable, context, depth)?);
        rest = &after_start[end + 2..];
    }
    rendered.push_str(rest);
    Ok(rendered)
}

/// 从资源表读取提示词正文。
fn prompt_text(prompts: &PromptResources, key: &str) -> CoreResult<String> {
    prompts
        .get(key)
        .map(|resource| resource.prompt.clone())
        .ok_or_else(|| CoreError::validation(format!("missing prompt resource: {key}")))
}

/// 解析单个模板变量，兼容短名和命名空间写法。
fn resolve_variable(
    variable: &str,
    context: &PromptTemplateContext,
    depth: usize,
) -> CoreResult<String> {
    // U149：`角色设定` 是新名，`节点提示词` 是**必须保留的兼容别名**。
    // 「节点」是画布上的方块、是我们的实现概念——模型读到「你是一个节点」
    // 既得不到水准锚点也得不到身份代入，所以模板里改叫「角色设定」。
    // 但存量工作流的节点 config 里存的是**渲染前的模板字符串**：
    // 只认新名会让所有已保存的工作流在下一次运行时 fail-loud
    // （`unknown_prompt_placeholder_fails_loudly` 那条保护正是为此存在的）。
    // 两个名字指向同一份正文，直到确认线上再无旧模板为止。
    if matches!(
        variable,
        "角色设定" | "prompt.角色设定" | "节点提示词" | "prompt.节点提示词"
    ) {
        return Ok(context.prompt_text.clone());
    }
    if let Some(name) = variable.strip_prefix("input.") {
        return context
            .inputs
            .get(name)
            .cloned()
            .ok_or_else(|| missing_variable(variable));
    }
    if let Some(name) = variable.strip_prefix("system.") {
        return context
            .system
            .get(name)
            .cloned()
            .ok_or_else(|| missing_variable(variable));
    }
    if let Some(name) = variable.strip_prefix("param.") {
        return context
            .parameters
            .get(name)
            .cloned()
            .ok_or_else(|| missing_variable(variable));
    }
    // 工作流变量：未声明的引用报缺失，不静默替换成空串，
    // 否则「第二章」写成空标题也能一路跑到底。
    if let Some(name) = variable.strip_prefix("var.") {
        return context
            .variables
            .get(name)
            .cloned()
            .ok_or_else(|| missing_variable(variable));
    }
    if variable.starts_with("skill.") {
        return Err(CoreError::validation(
            "prompt template namespace `skill` is deprecated; use `template`",
        ));
    }
    if let Some(reference) = variable.strip_prefix("template.") {
        return render_inline_prompt_template(reference, context, depth + 1);
    }
    context
        .inputs
        .get(variable)
        .cloned()
        .ok_or_else(|| missing_variable(variable))
}

/// 构造缺失变量错误。
fn missing_variable(variable: &str) -> CoreError {
    CoreError::validation(format!(
        "prompt template variable is unresolved: {variable}"
    ))
}

/// 渲染 `{{template.xxx(...)}}` 引用。
fn render_inline_prompt_template(
    reference: &str,
    context: &PromptTemplateContext,
    depth: usize,
) -> CoreResult<String> {
    let call = parse_template_call(reference)?;
    let manifest = context.templates.get(&call.template_id).ok_or_else(|| {
        CoreError::validation(format!(
            "prompt template is not loaded: {}",
            call.template_id
        ))
    })?;
    manifest.validate()?;
    validate_template_parameters(manifest, &call.arguments)?;

    let mut nested = context.clone();
    nested.parameters = call.arguments;
    render_prompt_template_at_depth(&manifest.template, &nested, depth)
}

/// 解析模板内联调用。
fn parse_template_call(reference: &str) -> CoreResult<TemplateCall> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return Err(CoreError::validation(
            "prompt template reference cannot be empty",
        ));
    }
    let Some(open) = trimmed.find('(') else {
        return Ok(TemplateCall {
            template_id: trimmed.to_owned(),
            arguments: BTreeMap::new(),
        });
    };
    if !trimmed.ends_with(')') {
        return Err(CoreError::validation(
            "prompt template call must close with `)`",
        ));
    }
    let template_id = trimmed[..open].trim();
    if template_id.is_empty() {
        return Err(CoreError::validation(
            "prompt template reference cannot be empty",
        ));
    }
    let body = &trimmed[open + 1..trimmed.len() - 1];
    Ok(TemplateCall {
        template_id: template_id.to_owned(),
        arguments: parse_template_arguments(body)?,
    })
}

/// 解析 `key="value"` 形式的 PromptTemplate 参数。
fn parse_template_arguments(body: &str) -> CoreResult<BTreeMap<String, String>> {
    let mut arguments = BTreeMap::new();
    if body.trim().is_empty() {
        return Ok(arguments);
    }

    for raw_part in body.split(',') {
        let part = raw_part.trim();
        let Some((key, value)) = part.split_once('=') else {
            return Err(CoreError::validation(
                "prompt template argument must use key=\"value\" syntax",
            ));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(CoreError::validation(
                "prompt template argument key cannot be empty",
            ));
        }
        if arguments.contains_key(key) {
            return Err(CoreError::validation(format!(
                "duplicate prompt template argument: {key}"
            )));
        }
        arguments.insert(key.to_owned(), parse_quoted_argument(value.trim())?);
    }

    Ok(arguments)
}

/// 解析带引号的参数值。
fn parse_quoted_argument(value: &str) -> CoreResult<String> {
    let bytes = value.as_bytes();
    if bytes.len() < 2 {
        return Err(CoreError::validation(
            "prompt template argument value must be quoted",
        ));
    }
    let quote = bytes[0];
    if quote != b'"' && quote != b'\'' {
        return Err(CoreError::validation(
            "prompt template argument value must be quoted",
        ));
    }
    if bytes[bytes.len() - 1] != quote {
        return Err(CoreError::validation(
            "prompt template argument quote is not closed",
        ));
    }
    Ok(value[1..value.len() - 1].to_owned())
}

/// 根据 manifest 的 JSON schema 子集校验模板参数。
fn validate_template_parameters(
    manifest: &PromptTemplateManifest,
    arguments: &BTreeMap<String, String>,
) -> CoreResult<()> {
    let Some(schema) = manifest.parameter_schema.as_object() else {
        if arguments.is_empty() {
            return Ok(());
        }
        return Err(CoreError::validation(format!(
            "prompt template {} does not accept parameters",
            manifest.template_id
        )));
    };

    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|values| values.keys().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default();
    if !properties.is_empty() {
        for key in arguments.keys() {
            if !properties.contains(key) {
                return Err(CoreError::validation(format!(
                    "unknown prompt template argument: {key}"
                )));
            }
        }
    }

    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !arguments.contains_key(key) {
                return Err(CoreError::validation(format!(
                    "missing prompt template argument: {key}"
                )));
            }
        }
    }

    Ok(())
}

/// 内部表示一次 PromptTemplate 调用。
struct TemplateCall {
    template_id: String,
    arguments: BTreeMap<String, String>,
}

/// 将上下文区块注册为可内联变量，中文短名和 section_id 都可用。
fn insert_section_aliases(inputs: &mut BTreeMap<String, String>, section: &WritingContextSection) {
    inputs.insert(section.section_id.clone(), section.content.clone());
    inputs.insert(section.title.clone(), section.content.clone());
    if let Some(alias) = section.section_id.strip_prefix("input.") {
        inputs.insert(alias.to_owned(), section.content.clone());
    }
    for alias in known_section_aliases(&section.section_id) {
        inputs.insert((*alias).to_owned(), section.content.clone());
    }
}

/// 常用上下文区块的中文变量别名，匹配 GUI 中更自然的 `{{上一章原文}}` 写法。
fn known_section_aliases(section_id: &str) -> &'static [&'static str] {
    match section_id {
        "user_intent" => &["用户初始意图"],
        "global_outline" => &["全局总纲", "已有全局总纲"],
        "stage_outline" => &["阶段总纲", "当前阶段总纲", "既有阶段总纲"],
        "previous_stage_outline" => &["之前阶段总纲"],
        "chapter_summaries" => &["章节概括", "当前阶段章节概括"],
        "previous_summaries" => &["前文总结"],
        "character_state" => &["人物与关系当前状态"],
        "unresolved_foreshadowing" => &["未回收伏笔"],
        "previous_chapter_text" => &["上一章原文", "上一章全文"],
        "outline" => &["本章大纲"],
        "details" => &["本章细节"],
        "line_numbered_draft" => &["带行号正文"],
        "chapter_text" => &["当前章节正文"],
        "target_text" => &["待评价文本"],
        "critic_outputs" => &["意见者输出"],
        "revision_context" => &["审慎者返修上下文", "返修上下文"],
        "revision_basis" => &["返修依据"],
        _ => &[],
    }
}

// U116：曾有 `prompt_template_hash(template)` 转发到 `skills::stable_text_hash`，已删。
// 它自称「供测试和 GUI 比对使用」，但两处都没用；真正需要模板 hash 的地方
// （skills/models.rs 的 original_template_hash、summarizer 的 request_hash）
// 一律直接调 stable_text_hash。同名转发只会让人以为存在两套 hash 口径。
