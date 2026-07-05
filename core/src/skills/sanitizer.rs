/// 日志中用于替代敏感值的固定文本。
pub const REDACTED: &str = "[REDACTED]";

/// 对 Skill 日志做保守脱敏，不记录密钥、认证头或敏感环境变量值。
pub fn sanitize_skill_log(line: &str) -> String {
    let trimmed = line.trim();
    let lower = trimmed.to_lowercase();
    for marker in [
        "authorization:",
        "api_key=",
        "api-key=",
        "apikey=",
        "token=",
        "secret=",
        "password=",
        "bearer ",
    ] {
        if let Some(index) = lower.find(marker) {
            let prefix = &trimmed[..index + marker.len()];
            return format!("{prefix}{REDACTED}");
        }
    }
    trimmed.to_owned()
}

/// 批量脱敏 Skill 日志。
pub fn sanitize_skill_logs(lines: &[String]) -> Vec<String> {
    lines.iter().map(|line| sanitize_skill_log(line)).collect()
}
