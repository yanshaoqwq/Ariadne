use crate::contracts::{CoreError, CoreResult};

/// 将作者输入拆成自然语言片段。高级查询语法不在此入口隐式开放。
fn natural_language_segments(query: &str) -> CoreResult<Vec<&str>> {
    let segments = query
        .split_whitespace()
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return Err(CoreError::validation(
            "natural language search query cannot be empty",
        ));
    }
    Ok(segments)
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
        segments
            .into_iter()
            .map(|segment| format!("\"{}\"", segment.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_language_query_encoders_quote_backend_syntax() {
        assert_eq!(
            tantivy_literal_query("角色:张三 (旧城) \"线索").unwrap(),
            "\"角色:张三\" \"(旧城)\" \"\\\"线索\""
        );
        assert_eq!(
            sqlite_fts_literal_query("角色:张三 (旧城) \"线索").unwrap(),
            "\"角色:张三\" AND \"(旧城)\" AND \"\"\"线索\""
        );
    }
}
