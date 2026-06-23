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
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            ranges.push((start, index + 1));
            start = index + 1;
        }
    }
    if start < text.len() {
        ranges.push((start, text.len()));
    }
    ranges
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
}
