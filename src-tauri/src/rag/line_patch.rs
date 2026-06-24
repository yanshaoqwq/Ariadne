use serde::{Deserialize, Serialize};

use crate::core::{CoreError, CoreResult, DocumentPatch, PatchHunk, TextRange};

/// Writer 插入正文的行号参数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriterInsertLines {
    pub document_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_version: Option<String>,
    pub after_line: u64,
    pub text: String,
}

/// Writer 替换正文的行号参数，`start_line..=end_line` 是闭区间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriterReplaceLines {
    pub document_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_version: Option<String>,
    pub start_line: u64,
    pub end_line: u64,
    pub text: String,
}

/// Patch 会话中的单次行号操作；后续操作基于模拟文本继续计算行号。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinePatchOperation {
    Insert {
        after_line: u64,
        text: String,
    },
    Replace {
        start_line: u64,
        end_line: u64,
        text: String,
    },
}

/// Writer/规划节点的行号 patch 会话，节点运行结束时才提交最终 patch。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchSession {
    pub document_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_version: Option<String>,
    pub base_content_hash: String,
    pub snapshot: String,
    pub simulated: String,
    #[serde(default)]
    pub pending_ops: Vec<LinePatchOperation>,
}

/// PatchSession 提交结果；只暴露一个最终 DocumentPatch 和审计用 hash。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchSessionCommit {
    pub document_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_version: Option<String>,
    pub base_content_hash: String,
    pub final_content_hash: String,
    #[serde(default)]
    pub operations: Vec<LinePatchOperation>,
    pub patch: DocumentPatch,
}

impl PatchSession {
    /// 从节点运行开始时的正文/纲领快照创建 patch 会话。
    pub fn new(
        document_id: impl Into<String>,
        base_version: Option<String>,
        snapshot: impl Into<String>,
    ) -> CoreResult<Self> {
        let document_id = document_id.into();
        validate_document_id(&document_id)?;
        let snapshot = snapshot.into();
        Ok(Self {
            document_id,
            base_version,
            base_content_hash: content_hash(&snapshot),
            simulated: snapshot.clone(),
            snapshot,
            pending_ops: Vec::new(),
        })
    }

    /// 记录一次插入操作，并立即应用到模拟文本，后续行号基于模拟结果。
    pub fn insert_lines(&mut self, after_line: u64, text: impl Into<String>) -> CoreResult<()> {
        let text = text.into();
        self.simulated = apply_line_operation(
            &self.simulated,
            &LinePatchOperation::Insert {
                after_line,
                text: text.clone(),
            },
        )?;
        self.pending_ops
            .push(LinePatchOperation::Insert { after_line, text });
        Ok(())
    }

    /// 记录一次替换操作，并立即应用到模拟文本，避免同一节点内行号漂移。
    pub fn replace_lines(
        &mut self,
        start_line: u64,
        end_line: u64,
        text: impl Into<String>,
    ) -> CoreResult<()> {
        let text = text.into();
        self.simulated = apply_line_operation(
            &self.simulated,
            &LinePatchOperation::Replace {
                start_line,
                end_line,
                text: text.clone(),
            },
        )?;
        self.pending_ops.push(LinePatchOperation::Replace {
            start_line,
            end_line,
            text,
        });
        Ok(())
    }

    /// 提交会话，生成从原始快照到模拟文本的单个最终 DocumentPatch。
    pub fn commit(&self) -> CoreResult<PatchSessionCommit> {
        let patch = diff_texts_to_patch(
            self.document_id.clone(),
            self.base_version.clone(),
            &self.snapshot,
            &self.simulated,
        )?;
        Ok(PatchSessionCommit {
            document_id: self.document_id.clone(),
            base_version: self.base_version.clone(),
            base_content_hash: self.base_content_hash.clone(),
            final_content_hash: content_hash(&self.simulated),
            operations: self.pending_ops.clone(),
            patch,
        })
    }
}

/// 把正文转成带 1-based 行号的文本，供 Writer prompt 使用。
pub fn line_numbered_text(text: &str) -> String {
    text.lines()
        .enumerate()
        .map(|(index, line)| format!("{}: {}", index + 1, line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 将插入行号操作转换成 UTF-8 byte range patch。
pub fn insert_lines_to_patch(
    original: &str,
    request: WriterInsertLines,
) -> CoreResult<DocumentPatch> {
    validate_document_id(&request.document_id)?;
    let ranges = line_ranges(original);
    if request.after_line == 0 || request.after_line as usize > ranges.len() {
        return Err(CoreError::validation(format!(
            "after_line {} is outside document line range 1..={}",
            request.after_line,
            ranges.len()
        )));
    }

    let insert_at = ranges[(request.after_line - 1) as usize].1;
    patch_for_range(
        request.document_id,
        request.base_version,
        insert_at,
        insert_at,
        request.text,
    )
}

/// 将替换行号操作转换成 UTF-8 byte range patch。
pub fn replace_lines_to_patch(
    original: &str,
    request: WriterReplaceLines,
) -> CoreResult<DocumentPatch> {
    validate_document_id(&request.document_id)?;
    let ranges = line_ranges(original);
    if request.start_line == 0 || request.end_line == 0 || request.start_line > request.end_line {
        return Err(CoreError::validation(
            "replace line range must be a 1-based closed interval",
        ));
    }
    if request.end_line as usize > ranges.len() {
        return Err(CoreError::validation(format!(
            "end_line {} is outside document line range 1..={}",
            request.end_line,
            ranges.len()
        )));
    }

    let start = ranges[(request.start_line - 1) as usize].0;
    let end = ranges[(request.end_line - 1) as usize].1;
    patch_for_range(
        request.document_id,
        request.base_version,
        start,
        end,
        request.text,
    )
}

/// 计算每一行的 UTF-8 byte 半开区间；换行符属于该行。
fn line_ranges(text: &str) -> Vec<(usize, usize)> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut ranges = Vec::new();
    let mut start = 0usize;
    for line in text.split_inclusive('\n') {
        let end = start + line.len();
        ranges.push((start, end));
        start = end;
    }
    if start < text.len() {
        ranges.push((start, text.len()));
    }
    ranges
}

/// 在给定文本上应用单次行号操作，返回新的模拟文本。
fn apply_line_operation(original: &str, operation: &LinePatchOperation) -> CoreResult<String> {
    match operation {
        LinePatchOperation::Insert { after_line, text } => {
            let ranges = line_ranges(original);
            if *after_line == 0 || *after_line as usize > ranges.len() {
                return Err(CoreError::validation(format!(
                    "after_line {after_line} is outside document line range 1..={}",
                    ranges.len()
                )));
            }
            let insert_at = ranges[(*after_line - 1) as usize].1;
            let mut next = String::with_capacity(original.len() + text.len());
            next.push_str(&original[..insert_at]);
            next.push_str(text);
            next.push_str(&original[insert_at..]);
            Ok(next)
        }
        LinePatchOperation::Replace {
            start_line,
            end_line,
            text,
        } => {
            let ranges = line_ranges(original);
            validate_replace_lines(*start_line, *end_line, ranges.len())?;
            let start = ranges[(*start_line - 1) as usize].0;
            let end = ranges[(*end_line - 1) as usize].1;
            let mut next = String::with_capacity(original.len() + text.len());
            next.push_str(&original[..start]);
            next.push_str(text);
            next.push_str(&original[end..]);
            Ok(next)
        }
    }
}

/// 校验替换行号区间。
fn validate_replace_lines(start_line: u64, end_line: u64, line_count: usize) -> CoreResult<()> {
    if start_line == 0 || end_line == 0 || start_line > end_line {
        return Err(CoreError::validation(
            "replace line range must be a 1-based closed interval",
        ));
    }
    if end_line as usize > line_count {
        return Err(CoreError::validation(format!(
            "end_line {end_line} is outside document line range 1..={line_count}"
        )));
    }
    Ok(())
}

/// 通过公共前后缀压缩，生成一个从旧文本到新文本的最小连续替换 hunk。
fn diff_texts_to_patch(
    document_id: String,
    base_version: Option<String>,
    original: &str,
    updated: &str,
) -> CoreResult<DocumentPatch> {
    if original == updated {
        return Ok(DocumentPatch {
            document_id,
            base_version,
            hunks: Vec::new(),
        });
    }

    let prefix = common_prefix_len(original, updated);
    let suffix = common_suffix_len(&original[prefix..], &updated[prefix..]);
    let original_end = original.len() - suffix;
    let updated_end = updated.len() - suffix;
    patch_for_range(
        document_id,
        base_version,
        prefix,
        original_end,
        updated[prefix..updated_end].to_owned(),
    )
}

/// 计算 UTF-8 边界对齐的公共前缀字节长度。
fn common_prefix_len(left: &str, right: &str) -> usize {
    let mut len = 0usize;
    for (left_char, right_char) in left.chars().zip(right.chars()) {
        if left_char != right_char {
            break;
        }
        len += left_char.len_utf8();
    }
    len
}

/// 计算 UTF-8 边界对齐的公共后缀字节长度。
fn common_suffix_len(left: &str, right: &str) -> usize {
    let mut len = 0usize;
    let max_len = left.len().min(right.len());
    for (left_char, right_char) in left.chars().rev().zip(right.chars().rev()) {
        if left_char != right_char {
            break;
        }
        let char_len = left_char.len_utf8();
        if len + char_len > max_len {
            break;
        }
        len += char_len;
    }
    len
}

/// 构造单 hunk 文档 patch。
fn patch_for_range(
    document_id: String,
    base_version: Option<String>,
    start: usize,
    end: usize,
    replacement: String,
) -> CoreResult<DocumentPatch> {
    Ok(DocumentPatch {
        document_id,
        base_version,
        hunks: vec![PatchHunk {
            range: TextRange::new(start as u64, end as u64)?,
            replacement,
        }],
    })
}

/// 校验文档 id。
fn validate_document_id(document_id: &str) -> CoreResult<()> {
    if document_id.trim().is_empty() {
        return Err(CoreError::validation("document_id cannot be empty"));
    }
    Ok(())
}

/// 计算会话内容 hash，便于提交前后做并发和审计比对。
fn content_hash(text: &str) -> String {
    crate::skills::stable_text_hash(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_numbered_text_uses_one_based_lines() {
        assert_eq!(line_numbered_text("甲\n乙"), "1: 甲\n2: 乙");
    }

    #[test]
    fn replace_lines_uses_closed_line_interval() {
        let patch = replace_lines_to_patch(
            "甲\n乙\n丙",
            WriterReplaceLines {
                document_id: "doc-1".to_owned(),
                base_version: Some("v1".to_owned()),
                start_line: 2,
                end_line: 3,
                text: "新乙\n新丙".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(patch.hunks[0].range, TextRange { start: 4, end: 11 });
    }

    #[test]
    fn patch_session_applies_multiple_ops_against_simulated_text() {
        let mut session = PatchSession::new("doc-1", Some("v1".to_owned()), "甲\n乙\n丙").unwrap();
        session.insert_lines(1, "新行\n").unwrap();
        session.replace_lines(2, 2, "替换\n").unwrap();
        let commit = session.commit().unwrap();

        assert_eq!(session.simulated, "甲\n替换\n乙\n丙");
        assert_eq!(commit.operations.len(), 2);
        assert_eq!(commit.patch.document_id, "doc-1");
        assert_eq!(commit.patch.base_version.as_deref(), Some("v1"));
        assert!(!commit.patch.is_empty());
    }

    #[test]
    fn line_ranges_stay_on_utf8_boundaries() {
        let patch = replace_lines_to_patch(
            "甲\n乙",
            WriterReplaceLines {
                document_id: "doc-1".to_owned(),
                base_version: None,
                start_line: 2,
                end_line: 2,
                text: "丙".to_owned(),
            },
        )
        .unwrap();

        assert_eq!(patch.hunks[0].range, TextRange { start: 4, end: 7 });
    }
}
