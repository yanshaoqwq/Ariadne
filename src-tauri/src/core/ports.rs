use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::errors::{CoreError, CoreResult};

/// 文本范围，使用半开区间 `[start, end)` 表示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRange {
    pub start: u64,
    pub end: u64,
}

impl TextRange {
    /// 创建文本范围，并拒绝起点大于终点的非法区间。
    pub fn new(start: u64, end: u64) -> CoreResult<Self> {
        if start > end {
            return Err(CoreError::validation(format!(
                "text range start {start} is greater than end {end}"
            )));
        }

        Ok(Self { start, end })
    }

    /// 返回范围长度。
    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    /// 判断范围是否为空。
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// 判断 offset 是否落在半开区间内。
    pub fn contains(&self, offset: u64) -> bool {
        offset >= self.start && offset < self.end
    }
}

/// 指向源文档中一段文本的位置，用于检索结果和 artifact 溯源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub document_id: String,
    pub range: TextRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PortValue {
    /// 小型结构化数据，允许直接内联。
    Inline { value: Value },
    /// 大文档引用，避免在工作流状态里复制全文。
    DocumentRef {
        document_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        range: Option<TextRange>,
    },
    /// 已分块内容引用。
    ChunkRef { chunk_id: String },
    /// 生成结果、patch、导出文件等 artifact 引用。
    ArtifactRef { artifact_id: String },
}

impl PortValue {
    /// 创建内联端口值。
    pub fn inline(value: impl Into<Value>) -> Self {
        Self::Inline {
            value: value.into(),
        }
    }

    /// 创建文档引用端口值。
    pub fn document_ref(document_id: impl Into<String>, range: Option<TextRange>) -> Self {
        Self::DocumentRef {
            document_id: document_id.into(),
            range,
        }
    }

    /// 创建 chunk 引用端口值。
    pub fn chunk_ref(chunk_id: impl Into<String>) -> Self {
        Self::ChunkRef {
            chunk_id: chunk_id.into(),
        }
    }

    /// 创建 artifact 引用端口值。
    pub fn artifact_ref(artifact_id: impl Into<String>) -> Self {
        Self::ArtifactRef {
            artifact_id: artifact_id.into(),
        }
    }

    /// 判断端口值是否是引用类型。
    pub fn is_reference(&self) -> bool {
        !matches!(self, Self::Inline { .. })
    }

    /// 返回用于端口契约校验和 UI 展示的类型名。
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Inline { .. } => "inline",
            Self::DocumentRef { .. } => "document_ref",
            Self::ChunkRef { .. } => "chunk_ref",
            Self::ArtifactRef { .. } => "artifact_ref",
        }
    }
}

/// 节点输入或输出端口定义。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortDefinition {
    pub name: String,
    pub type_name: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl PortDefinition {
    /// 创建端口定义。
    pub fn new(name: impl Into<String>, type_name: impl Into<String>, required: bool) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
            required,
            description: None,
        }
    }

    /// 为端口补充说明文本。
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

pub type PortMap = BTreeMap<String, PortValue>;

/// 校验所有 required 端口是否都已经提供值。
pub fn validate_required_ports(definitions: &[PortDefinition], values: &PortMap) -> CoreResult<()> {
    for definition in definitions {
        if definition.required && !values.contains_key(&definition.name) {
            return Err(CoreError::PortMissing {
                port: definition.name.clone(),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn text_range_rejects_inverted_offsets() {
        assert!(TextRange::new(10, 2).is_err());
    }

    #[test]
    fn port_value_serializes_as_tagged_reference() {
        let value = PortValue::document_ref("doc-1", Some(TextRange::new(1, 4).unwrap()));
        let json = serde_json::to_value(value).unwrap();

        assert_eq!(
            json,
            json!({
                "kind": "document_ref",
                "document_id": "doc-1",
                "range": { "start": 1, "end": 4 }
            })
        );
    }

    #[test]
    fn required_port_validation_reports_missing_port() {
        let definitions = vec![PortDefinition::new("prompt", "string", true)];
        let values = PortMap::new();

        assert!(matches!(
            validate_required_ports(&definitions, &values),
            Err(CoreError::PortMissing { port }) if port == "prompt"
        ));
    }
}
