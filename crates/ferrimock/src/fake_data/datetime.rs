//! Date and time generators

use super::rng::rng;
use fake::Fake;
use fake::faker::chrono::en::*;
use rand::RngExt;
use rand::seq::IndexedRandom;

/// Generate a random date in RFC3339 format
pub fn fake_date() -> String {
    DateTime()
        .fake_with_rng::<chrono::DateTime<chrono::Utc>, _>(&mut rng())
        .to_rfc3339()
}

/// Generate a random time string
pub fn fake_time() -> String {
    Time().fake_with_rng(&mut rng())
}

/// Generate an ISO date (date only, no time)
pub fn fake_iso_date() -> String {
    fake_date_in(crate::type_detector::DateFormat::Iso)
}

/// Generate a date without a time, written the way the field wrote it.
///
/// The format is carried rather than assumed because answering a `17/03/2024`
/// field with `2024-03-17` changes the value's shape, and anything parsing it
/// breaks on the reply.
pub fn fake_date_in(format: crate::type_detector::DateFormat) -> String {
    use crate::type_detector::DateFormat;

    let year = rng().random_range(2020..=2025);
    let month = rng().random_range(1..=12);
    let day = rng().random_range(1..=28);

    match format {
        DateFormat::Iso => format!("{year:04}-{month:02}-{day:02}"),
        DateFormat::Slash => format!("{day:02}/{month:02}/{year:04}"),
        DateFormat::Dotted => format!("{day:02}.{month:02}.{year:04}"),
        DateFormat::Compact => format!("{year:04}{month:02}{day:02}"),
    }
}

/// Generate a moment in time, written the way the field wrote it.
pub fn fake_timestamp_in(format: crate::type_detector::TimestampFormat) -> String {
    use crate::type_detector::TimestampFormat;

    const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let year = rng().random_range(2020..=2025);
    let month: usize = rng().random_range(1..=12);
    let day = rng().random_range(1..=28);
    let (hour, minute, second) = (
        rng().random_range(0..24),
        rng().random_range(0..60),
        rng().random_range(0..60),
    );
    let date = format!("{year:04}-{month:02}-{day:02}");
    let time = format!("{hour:02}:{minute:02}:{second:02}");

    match format {
        TimestampFormat::Rfc3339Utc => format!("{date}T{time}Z"),
        TimestampFormat::Rfc3339Offset => {
            let offsets = ["+01:00", "+02:00", "-05:00", "-08:00", "+05:30", "+09:00"];
            let offset = offsets.choose(&mut rng()).copied().unwrap_or("+00:00");
            format!("{date}T{time}{offset}")
        }
        TimestampFormat::Rfc3339Millis => {
            format!("{date}T{time}.{:03}Z", rng().random_range(0..1000))
        }
        TimestampFormat::Rfc3339Nanos => {
            format!(
                "{date}T{time}.{:09}Z",
                rng().random_range(0..1_000_000_000_u32)
            )
        }
        TimestampFormat::SqlDateTime => format!("{date} {time}"),
        TimestampFormat::Rfc2822 | TimestampFormat::HttpDate => {
            let weekday = WEEKDAYS.choose(&mut rng()).copied().unwrap_or("Mon");
            let month_name = MONTHS.get(month - 1).copied().unwrap_or("Jan");
            let zone = if matches!(format, TimestampFormat::HttpDate) {
                "GMT"
            } else {
                "+0000"
            };
            format!("{weekday}, {day:02} {month_name} {year:04} {time} {zone}")
        }
        TimestampFormat::EpochFractional => {
            format!(
                "{}.{:06}",
                fake_unix_timestamp(),
                rng().random_range(0..1_000_000)
            )
        }
    }
}

/// Generate a Unix timestamp
pub fn fake_unix_timestamp() -> i64 {
    rng().random_range(1_640_000_000..=1_900_000_000)
}

/// Generate a relative time string
pub fn fake_relative_time() -> String {
    let times = [
        "2 hours ago",
        "1 day ago",
        "3 days ago",
        "1 week ago",
        "2 weeks ago",
        "1 month ago",
    ];
    times
        .choose(&mut rng())
        .copied()
        .unwrap_or("1 day ago")
        .to_string()
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_fake_date() {
        let date = fake_date();
        assert!(!date.is_empty());
        assert!(date.contains('T'));
    }

    #[test]
    fn test_fake_time() {
        let time = fake_time();
        assert!(!time.is_empty());
    }

    #[test]
    fn test_fake_iso_date() {
        let date = fake_iso_date();
        assert!(date.contains('-'));
        let parts: Vec<&str> = date.split('-').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].len(), 4);
        assert_eq!(parts[1].len(), 2);
        assert_eq!(parts[2].len(), 2);
    }

    #[test]
    fn test_fake_unix_timestamp() {
        let timestamp = fake_unix_timestamp();
        assert!(timestamp >= 1_640_000_000);
        assert!(timestamp <= 1_900_000_000);
    }

    #[test]
    fn test_fake_relative_time() {
        let time = fake_relative_time();
        assert!(!time.is_empty());
        assert!(time.contains("ago"));
    }
}
