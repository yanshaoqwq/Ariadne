use std::cmp::Ordering;

use crate::core::{CoreError, CoreResult};

/// 解析后的故事段编号，用于稳定排序和中点生成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentNumber {
    integer: Vec<u8>,
    fractional: Vec<u8>,
}

/// 解析故事段编号，支持非负十进制字符串。
pub fn parse_segment_number(value: &str) -> CoreResult<SegmentNumber> {
    if value.trim() != value || value.is_empty() {
        return Err(CoreError::validation("segment number cannot be empty"));
    }

    let mut parts = value.split('.');
    let integer_raw = parts.next().unwrap_or_default();
    let fractional_raw = parts.next();
    if parts.next().is_some() {
        return Err(CoreError::validation(
            "segment number can contain only one decimal point",
        ));
    }
    if integer_raw.is_empty() || !integer_raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CoreError::validation(
            "segment number integer part must be digits",
        ));
    }
    let fractional = match fractional_raw {
        Some(raw) => {
            if raw.is_empty() || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(CoreError::validation(
                    "segment number fractional part must be digits",
                ));
            }
            trim_trailing_zeroes(raw.as_bytes().to_vec())
        }
        None => Vec::new(),
    };

    Ok(SegmentNumber {
        integer: trim_leading_zeroes(integer_raw.as_bytes().to_vec()),
        fractional,
    })
}

/// 比较两个故事段编号。
pub fn compare_segment_numbers(left: &str, right: &str) -> CoreResult<Ordering> {
    let left = parse_segment_number(left)?;
    let right = parse_segment_number(right)?;
    Ok(left.cmp(&right))
}

/// 在两个编号之间生成中点编号。
pub fn midpoint_segment_number(left: &str, right: &str) -> CoreResult<String> {
    if compare_segment_numbers(left, right)? != Ordering::Less {
        return Err(CoreError::validation(
            "left segment number must be smaller than right segment number",
        ));
    }

    let left_scaled = scale_to_millionths(left)?;
    let right_scaled = scale_to_millionths(right)?;
    let midpoint = (left_scaled + right_scaled) / 2;
    if midpoint == left_scaled || midpoint == right_scaled {
        return Err(CoreError::validation(
            "segment numbers are too close to create midpoint",
        ));
    }

    Ok(format_scaled(midpoint))
}

impl Ord for SegmentNumber {
    /// 按十进制数值比较编号。
    fn cmp(&self, other: &Self) -> Ordering {
        match compare_digits(&self.integer, &other.integer) {
            Ordering::Equal => compare_fractional(&self.fractional, &other.fractional),
            ordering => ordering,
        }
    }
}

impl PartialOrd for SegmentNumber {
    /// 提供与 Ord 一致的部分排序。
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 去掉整数部分前导零，但保留单个 0。
fn trim_leading_zeroes(mut digits: Vec<u8>) -> Vec<u8> {
    while digits.len() > 1 && digits.first() == Some(&b'0') {
        digits.remove(0);
    }
    digits
}

/// 去掉小数部分尾随零，保证 1.50 与 1.5 排序一致。
fn trim_trailing_zeroes(mut digits: Vec<u8>) -> Vec<u8> {
    while digits.last() == Some(&b'0') {
        digits.pop();
    }
    digits
}

/// 比较无前导零的数字串。
fn compare_digits(left: &[u8], right: &[u8]) -> Ordering {
    match left.len().cmp(&right.len()) {
        Ordering::Equal => left.cmp(right),
        ordering => ordering,
    }
}

/// 比较小数部分，不足位补零。
fn compare_fractional(left: &[u8], right: &[u8]) -> Ordering {
    let len = left.len().max(right.len());
    for index in 0..len {
        let left_digit = left.get(index).copied().unwrap_or(b'0');
        let right_digit = right.get(index).copied().unwrap_or(b'0');
        match left_digit.cmp(&right_digit) {
            Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    Ordering::Equal
}

/// 将编号放大到百万分位，避免使用浮点数生成常见中点。
fn scale_to_millionths(value: &str) -> CoreResult<u128> {
    let parsed = parse_segment_number(value)?;
    let integer = digits_to_u128(&parsed.integer)?;
    let mut fractional = parsed.fractional;
    if fractional.len() > 6 {
        return Err(CoreError::validation(
            "segment number supports at most 6 decimal places",
        ));
    }
    while fractional.len() < 6 {
        fractional.push(b'0');
    }
    Ok(integer
        .checked_mul(1_000_000)
        .and_then(|base| base.checked_add(digits_to_u128(&fractional).ok()?))
        .ok_or_else(|| CoreError::validation("segment number is too large"))?)
}

/// 将数字串转成整数。
fn digits_to_u128(digits: &[u8]) -> CoreResult<u128> {
    let mut value = 0u128;
    for digit in digits {
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u128::from(digit - b'0')))
            .ok_or_else(|| CoreError::validation("segment number is too large"))?;
    }
    Ok(value)
}

/// 将百万分位整数格式化回十进制字符串。
fn format_scaled(value: u128) -> String {
    let integer = value / 1_000_000;
    let fractional = value % 1_000_000;
    if fractional == 0 {
        return integer.to_string();
    }
    let mut fractional_text = format!("{fractional:06}");
    while fractional_text.ends_with('0') {
        fractional_text.pop();
    }
    format!("{integer}.{fractional_text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_segment_numbers_sort_without_float() {
        assert_eq!(compare_segment_numbers("1", "1.5").unwrap(), Ordering::Less);
        assert_eq!(
            compare_segment_numbers("1.50", "1.5").unwrap(),
            Ordering::Equal
        );
        assert_eq!(compare_segment_numbers("2", "10").unwrap(), Ordering::Less);
    }

    #[test]
    fn midpoint_generates_readable_decimal() {
        assert_eq!(midpoint_segment_number("1", "2").unwrap(), "1.5");
        assert_eq!(midpoint_segment_number("1", "1.5").unwrap(), "1.25");
    }
}
