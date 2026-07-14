use std::collections::BTreeSet;

use crate::contracts::{CoreError, CoreResult};

/// 将作者输入拆成自然语言片段。中英文标点统一作为词边界，高级查询语法不隐式开放。
fn natural_language_segments(query: &str) -> CoreResult<Vec<String>> {
    let segments = normalized_segments(query);
    if segments.is_empty() {
        return Err(CoreError::validation(
            "natural language search query cannot be empty",
        ));
    }
    Ok(segments)
}

fn normalized_segments(query: &str) -> Vec<String> {
    let normalized = query
        .chars()
        .map(|character| {
            if character.is_whitespace() || is_query_punctuation(character) {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let segments = normalized
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    segments
}

fn ngram_tokens(segments: &[String]) -> Vec<String> {
    let mut tokens = Vec::new();
    for segment in segments {
        let characters = segment.chars().collect::<Vec<_>>();
        for start in 0..characters.len() {
            for width in 1..=3.min(characters.len() - start) {
                tokens.push(characters[start..start + width].iter().collect());
            }
        }
    }
    tokens
}

fn is_query_punctuation(character: char) -> bool {
    matches!(
        character,
        ':' | '：'
            | ';'
            | '；'
            | ','
            | '，'
            | '.'
            | '。'
            | '!'
            | '！'
            | '?'
            | '？'
            | '('
            | '（'
            | ')'
            | '）'
            | '['
            | '【'
            | ']'
            | '】'
            | '{'
            | '}'
            | '<'
            | '《'
            | '>'
            | '》'
            | '"'
            | '“'
            | '”'
            | '\''
            | '‘'
            | '’'
            | '-'
            | '—'
            | '–'
            | '/'
            | '\\'
            | '|'
    )
}

/// 将自然语言输入编码为 Tantivy 字面量查询，禁止冒号、括号、减号等被解释为语法。
pub(crate) fn tantivy_literal_query(query: &str) -> CoreResult<String> {
    natural_language_segments(query).map(|segments| {
        segments
            .into_iter()
            .map(|segment| {
                let escaped = segment.replace('\\', "\\\\").replace('"', "\\\"");
                format!("\"{escaped}\"")
            })
            .collect::<Vec<_>>()
            .join(" ")
    })
}

/// 将同一自然语言片段编码为 SQLite FTS5 字面量查询。
pub(crate) fn sqlite_fts_literal_query(query: &str) -> CoreResult<String> {
    natural_language_segments(query).map(|segments| {
        let mut seen = BTreeSet::new();
        ngram_tokens(&segments)
            .into_iter()
            .filter(|token| seen.insert(token.clone()))
            .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ")
    })
}

/// SQLite FTS5 使用 unicode61 读取预分词后的 1–3 gram；原文仍保存在主表。
pub(crate) fn sqlite_fts_index_text(text: &str) -> CoreResult<String> {
    Ok(ngram_tokens(&normalized_segments(text)).join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_language_query_encoders_quote_backend_syntax() {
        let tantivy = tantivy_literal_query("角色:张三 (旧城) \"线索").unwrap();
        assert_eq!(tantivy, "\"角色\" \"张三\" \"旧城\" \"线索\"");
        let sqlite = sqlite_fts_literal_query("角色:张三 (旧城) \"线索").unwrap();
        assert!(sqlite.contains("\"角色\""));
        assert!(sqlite.contains("\"张三\""));
        assert!(sqlite.contains("\"旧城\""));
        assert!(sqlite.contains("\"线索\""));
        assert!(!sqlite.contains(':'));
    }
}
