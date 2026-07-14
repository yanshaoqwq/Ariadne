/// 计算可持久化的内容版本。
///
/// 文档服务、检索索引与 SourceSpan 必须使用同一算法，否则同一正文会产生
/// 不可比较的版本标识。当前沿用既有 FNV-1a 64-bit 合同以保持磁盘兼容。
pub fn content_version_for_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_version_is_stable_for_utf8_bytes() {
        assert_eq!(
            content_version_for_bytes("甲乙".as_bytes()),
            content_version_for_bytes("甲乙".as_bytes())
        );
        assert_ne!(
            content_version_for_bytes("甲乙".as_bytes()),
            content_version_for_bytes("乙甲".as_bytes())
        );
    }
}
