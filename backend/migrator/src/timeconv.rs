//! 旧库时间列转换：MySQL `datetime` 以字符串提取（避免 Zero Date 导致 sqlx 解码崩溃），
//! 按 `--source-offset` 指定的旧站时区解释，再归一化为 UTC `OffsetDateTime`。

use thiserror::Error;
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TimeConvError {
    #[error("invalid source offset `{0}` (expected like +08:00)")]
    InvalidOffset(String),
    #[error("invalid datetime format `{0}` (expected `YYYY-MM-DD HH:MM:SS[.ffffff]`)")]
    InvalidFormat(String),
    #[error("zero date `{0}` is not representable")]
    ZeroDate(String),
    #[error("datetime `{0}` has out-of-range components")]
    OutOfRange(String),
}

/// 解析 `+08:00` / `-0530` / `+08` 形式的时区偏移。
pub fn parse_source_offset(raw: &str) -> Result<UtcOffset, TimeConvError> {
    let raw = raw.trim();
    let (sign, rest) = match raw.as_bytes().first() {
        Some(b'+') => (1, &raw[1..]),
        Some(b'-') => (-1, &raw[1..]),
        _ => return Err(TimeConvError::InvalidOffset(raw.to_owned())),
    };
    let rest = rest.replace(':', "");
    if rest.len() != 2 && rest.len() != 4 || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return Err(TimeConvError::InvalidOffset(raw.to_owned()));
    }
    let (hours, minutes) = rest.split_at(2);
    let hours = hours
        .parse::<i8>()
        .map_err(|_| TimeConvError::InvalidOffset(raw.to_owned()))?;
    let minutes = if minutes.is_empty() {
        0
    } else {
        minutes
            .parse::<i8>()
            .map_err(|_| TimeConvError::InvalidOffset(raw.to_owned()))?
    };
    if hours > 23 || minutes > 59 {
        return Err(TimeConvError::InvalidOffset(raw.to_owned()));
    }
    UtcOffset::from_hms(sign * hours, sign * minutes, 0)
        .map_err(|_| TimeConvError::InvalidOffset(raw.to_owned()))
}

/// 把 `YYYY-MM-DD HH:MM:SS[.fraction]` 按 `offset` 解释并转换为 UTC。
/// Zero Date（`0000-00-00 ...`）与越界分量返回错误，由 Preflight 拦截。
pub fn legacy_datetime_to_utc(
    raw: &str,
    offset: UtcOffset,
) -> Result<OffsetDateTime, TimeConvError> {
    let raw = raw.trim();
    let (date_part, time_part) = raw
        .split_once([' ', 'T'])
        .ok_or_else(|| TimeConvError::InvalidFormat(raw.to_owned()))?;

    let date_numbers: Vec<&str> = date_part.split('-').collect();
    let [year, month, day] = date_numbers.as_slice() else {
        return Err(TimeConvError::InvalidFormat(raw.to_owned()));
    };
    let year = parse_number::<i32>(year, raw)?;
    let month = parse_number::<u8>(month, raw)?;
    let day = parse_number::<u8>(day, raw)?;
    if year == 0 || month == 0 || day == 0 {
        return Err(TimeConvError::ZeroDate(raw.to_owned()));
    }

    let (hms, fraction) = match time_part.split_once('.') {
        Some((hms, fraction)) => (hms, fraction),
        None => (time_part, "0"),
    };
    let time_numbers: Vec<&str> = hms.split(':').collect();
    let [hour, minute, second] = time_numbers.as_slice() else {
        return Err(TimeConvError::InvalidFormat(raw.to_owned()));
    };
    let hour = parse_number::<u8>(hour, raw)?;
    let minute = parse_number::<u8>(minute, raw)?;
    let second = parse_number::<u8>(second, raw)?;

    // MySQL 小数秒最多 6 位（微秒）；右补零到 9 位纳秒。
    if fraction.len() > 9 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return Err(TimeConvError::InvalidFormat(raw.to_owned()));
    }
    let nanos_text = format!("{fraction:0<9}");
    let nanos = nanos_text
        .parse::<u32>()
        .map_err(|_| TimeConvError::InvalidFormat(raw.to_owned()))?;

    let month = Month::try_from(month).map_err(|_| TimeConvError::OutOfRange(raw.to_owned()))?;
    let date = Date::from_calendar_date(year, month, day)
        .map_err(|_| TimeConvError::OutOfRange(raw.to_owned()))?;
    let time = Time::from_hms_nano(hour, minute, second, nanos)
        .map_err(|_| TimeConvError::OutOfRange(raw.to_owned()))?;

    Ok(PrimitiveDateTime::new(date, time)
        .assume_offset(offset)
        .to_offset(UtcOffset::UTC))
}

fn parse_number<T: std::str::FromStr>(raw: &str, whole: &str) -> Result<T, TimeConvError> {
    raw.parse::<T>()
        .map_err(|_| TimeConvError::InvalidFormat(whole.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shanghai() -> UtcOffset {
        parse_source_offset("+08:00").unwrap()
    }

    #[test]
    fn parses_offset_variants() {
        assert_eq!(parse_source_offset("+08:00").unwrap().whole_hours(), 8);
        assert_eq!(parse_source_offset("+0800").unwrap().whole_hours(), 8);
        assert_eq!(parse_source_offset("+08").unwrap().whole_hours(), 8);
        assert_eq!(
            parse_source_offset("-05:30").unwrap(),
            UtcOffset::from_hms(-5, -30, 0).unwrap()
        );
        assert!(parse_source_offset("08:00").is_err());
        assert!(parse_source_offset("+25:00").is_err());
        assert!(parse_source_offset("+ab:cd").is_err());
    }

    #[test]
    fn converts_legacy_datetime_to_utc() {
        let utc = legacy_datetime_to_utc("2020-01-02 08:00:00", shanghai()).unwrap();
        assert_eq!(
            utc,
            OffsetDateTime::from_unix_timestamp(1_577_923_200).unwrap()
        );
    }

    #[test]
    fn converts_fractional_seconds() {
        let utc = legacy_datetime_to_utc("2020-01-02 08:00:00.5", shanghai()).unwrap();
        assert_eq!(utc.nanosecond(), 500_000_000);
        let utc = legacy_datetime_to_utc("2020-01-02 08:00:00.123456", shanghai()).unwrap();
        assert_eq!(utc.nanosecond(), 123_456_000);
    }

    #[test]
    fn rejects_zero_date() {
        let error = legacy_datetime_to_utc("0000-00-00 00:00:00", shanghai()).unwrap_err();
        assert_eq!(
            error,
            TimeConvError::ZeroDate("0000-00-00 00:00:00".to_owned())
        );
        assert!(legacy_datetime_to_utc("2020-00-10 00:00:00", shanghai()).is_err());
        assert!(legacy_datetime_to_utc("2020-01-00 00:00:00", shanghai()).is_err());
    }

    #[test]
    fn rejects_out_of_range_and_garbage() {
        assert!(legacy_datetime_to_utc("2020-13-01 00:00:00", shanghai()).is_err());
        assert!(legacy_datetime_to_utc("2020-02-30 00:00:00", shanghai()).is_err());
        assert!(legacy_datetime_to_utc("2020-01-02 25:00:00", shanghai()).is_err());
        assert!(legacy_datetime_to_utc("not a date", shanghai()).is_err());
        assert!(legacy_datetime_to_utc("2020-01-02", shanghai()).is_err());
    }
}
