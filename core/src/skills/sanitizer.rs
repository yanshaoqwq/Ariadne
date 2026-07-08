/// 日志中用于替代敏感值的固定文本。
pub const REDACTED: &str = "[REDACTED]";

/// 对 Skill 日志做保守脱敏，不记录密钥、认证头或敏感环境变量值。
pub fn sanitize_skill_log(line: &str) -> String {
    let mut sanitized = line.trim().to_owned();
    sanitized = redact_authorization_header(&sanitized);
    sanitized = redact_bearer_token(&sanitized);
    for key in [
        "api_key",
        "api-key",
        "apikey",
        "x-api-key",
        "token",
        "access_token",
        "refresh_token",
        "secret",
        "password",
    ] {
        sanitized = redact_key_value(&sanitized, key);
    }
    sanitized
}

/// 批量脱敏 Skill 日志。
pub fn sanitize_skill_logs(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| sanitize_skill_log(line)).collect()
}

fn redact_authorization_header(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let Some(index) = lower.find("authorization") else {
        return input.to_owned();
    };
    let Some((delimiter_start, delimiter_end)) = delimiter_after_key(input, index, "authorization")
    else {
        return input.to_owned();
    };
    format!(
        "{}{}{}",
        &input[..delimiter_start],
        &input[delimiter_start..delimiter_end],
        REDACTED
    )
}

fn redact_bearer_token(input: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let Some(index) = lower.find("bearer ") else {
        return input.to_owned();
    };
    let value_start = index + "bearer ".len();
    let value_end = value_end(input, value_start, None);
    format!(
        "{}{}{}{}",
        &input[..value_start],
        REDACTED,
        quote_suffix(input, value_end, None),
        &input[value_end..]
    )
}

fn redact_key_value(input: &str, key: &str) -> String {
    let lower = input.to_ascii_lowercase();
    let mut output = String::with_capacity(input.len());
    let mut search_start = 0;
    while let Some(relative_index) = lower[search_start..].find(key) {
        let index = search_start + relative_index;
        let Some((_delimiter_start, delimiter_end)) = delimiter_after_key(input, index, key) else {
            let key_end = index + key.len();
            output.push_str(&input[search_start..key_end]);
            search_start = key_end;
            continue;
        };
        let value_start = skip_value_prefix(input, delimiter_end);
        let quote = input[value_start..]
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'');
        let actual_start = value_start + quote.map(char::len_utf8).unwrap_or(0);
        let value_end = value_end(input, actual_start, quote);
        let suffix = quote_suffix(input, value_end, quote);
        output.push_str(&input[search_start..actual_start]);
        output.push_str(REDACTED);
        output.push_str(suffix);
        search_start = value_end + suffix.len();
    }
    output.push_str(&input[search_start..]);
    output
}

fn delimiter_after_key(input: &str, key_start: usize, key: &str) -> Option<(usize, usize)> {
    let mut cursor = key_start + key.len();
    if input[cursor..]
        .chars()
        .next()
        .is_some_and(|ch| ch == '"' || ch == '\'')
    {
        cursor += 1;
    }
    cursor += input[cursor..]
        .chars()
        .take_while(|c| c.is_whitespace())
        .map(char::len_utf8)
        .sum::<usize>();
    let delimiter = input[cursor..].chars().next()?;
    if delimiter != ':' && delimiter != '=' {
        return None;
    }
    let delimiter_start = cursor;
    cursor += delimiter.len_utf8();
    cursor += input[cursor..]
        .chars()
        .take_while(|c| c.is_whitespace())
        .map(char::len_utf8)
        .sum::<usize>();
    Some((delimiter_start, cursor))
}

fn skip_value_prefix(input: &str, mut cursor: usize) -> usize {
    cursor += input[cursor..]
        .chars()
        .take_while(|c| c.is_whitespace())
        .map(char::len_utf8)
        .sum::<usize>();
    cursor
}

fn value_end(input: &str, start: usize, quote: Option<char>) -> usize {
    let mut cursor = start;
    for ch in input[start..].chars() {
        if quote.is_some_and(|quote| ch == quote)
            || (quote.is_none() && (ch.is_whitespace() || matches!(ch, ',' | ';' | '&')))
        {
            break;
        }
        cursor += ch.len_utf8();
    }
    cursor
}

fn quote_suffix(input: &str, value_end: usize, quote: Option<char>) -> &str {
    if quote.is_some_and(|quote| input[value_end..].starts_with(quote)) {
        &input[value_end..value_end + quote.unwrap().len_utf8()]
    } else {
        ""
    }
}
