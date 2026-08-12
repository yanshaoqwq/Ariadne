//! 时间换算与本地时区。
//!
//! 存在的理由是**消除重复**：`costs/budget.rs`（日预算切日）与 `llm/service.rs`
//! （月度用量切月）此前各自持有一半的民用历算法与时区读取，两处漂移会让
//! 「今天花了多少」和「这个月花了多少」按不同规则切分。
//!
//! 全部只依赖 std，不引第三方时间库。

/// 本地时区相对 UTC 的偏移（毫秒），东为正。
///
/// std 不提供时区格式化，因此偏移由环境变量 `ARIADNE_UTC_OFFSET_MINUTES` 显式声明，
/// 缺省按 UTC。取舍是**可预测优先于自动**：自动探测在容器/CI 里经常给出与用户预期
/// 不同的结果，而预算是会拦住运行的硬门禁，静默按错误时区切日比按 UTC 更难排查。
/// 桌面端在启动时把系统偏移写入该变量即可获得本地语义。
pub fn local_utc_offset_ms() -> i64 {
    const MS_PER_MINUTE: i64 = 60_000;
    // 合法范围 UTC-12:00 ~ UTC+14:00，超出视为配置错误并退化为 UTC。
    const MIN_OFFSET_MINUTES: i64 = -12 * 60;
    const MAX_OFFSET_MINUTES: i64 = 14 * 60;
    std::env::var("ARIADNE_UTC_OFFSET_MINUTES")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|minutes| (MIN_OFFSET_MINUTES..=MAX_OFFSET_MINUTES).contains(minutes))
        .map(|minutes| minutes * MS_PER_MINUTE)
        .unwrap_or(0)
}

/// days（自 1970-01-01 起的天数）转成 (year, month, day)。
///
/// Howard Hinnant 的民用历算法，以 0000-03-01 为纪元。
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// (year, month, day) 转成自 1970-01-01 起的天数。
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let m = month as i64;
    let d = day as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// 把毫秒时间戳格式化成**文件名安全**的本地时间：`20260812-143005`。
///
/// 用于导出文件名。刻意不含 `:` 与空格：Windows 禁止文件名含 `:`，
/// 空格会让命令行与脚本处理导出物时需要额外引号。
/// 按本地时区渲染——用户看到的文件名要对得上他按下导出键的墙钟时间。
pub fn format_local_timestamp_for_filename(now_ms: u64) -> String {
    const MS_PER_DAY: i64 = 86_400_000;
    let local = now_ms as i64 + local_utc_offset_ms();
    // rem_euclid 保证 1970 以前的时刻也落在 [0, MS_PER_DAY)，不会出现负的时分秒。
    let day_ms = local.rem_euclid(MS_PER_DAY);
    let days = (local - day_ms) / MS_PER_DAY;
    let (year, month, day) = civil_from_days(days);
    let seconds_of_day = day_ms / 1_000;
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_calendar_roundtrips_across_leap_boundaries() {
        for (year, month, day) in [
            (1970, 1, 1),
            (2000, 2, 29),
            (2026, 8, 12),
            (2100, 3, 1),
            (1969, 12, 31),
        ] {
            let days = days_from_civil(year, month, day);
            assert_eq!(civil_from_days(days), (year, month, day));
        }
    }

    #[test]
    fn filename_timestamp_has_no_characters_that_break_paths() {
        // 2026-08-12T14:30:05Z
        let formatted = format_local_timestamp_for_filename(1_786_631_405_000);
        assert_eq!(formatted, "20260812-143005");
        assert!(!formatted.contains(':'));
        assert!(!formatted.contains(' '));
        assert!(!formatted.contains('/'));
    }

    #[test]
    fn filename_timestamp_stays_wellformed_before_epoch() {
        // 负时间戳走 rem_euclid 分支；时分秒不能出现负号。
        let formatted = format_local_timestamp_for_filename(0);
        assert_eq!(formatted, "19700101-000000");
    }
}
