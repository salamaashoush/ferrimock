//! Date and time generators

use std::sync::OnceLock;

use super::rng::rng;
use chrono::{DateTime, Datelike, FixedOffset, Timelike, Utc};
use fake::Fake;
use fake::faker::chrono::en::Time;
use rand::RngExt;
use rand::seq::IndexedRandom;

/// How far back a generated moment can fall.
const WINDOW_YEARS: i64 = 5;

const SECONDS_PER_YEAR: i64 = 365 * 24 * 60 * 60;

/// The instant every generated date is measured back from.
///
/// A hardcoded span goes stale the day after it is written: `2020..=2025` put
/// every record months in the past, and the gap widened every day the constant
/// sat there — a stronger tell than any flat histogram, and one that decays
/// with wall-clock time rather than staying fixed. Read once per process
/// rather than per call, so two records built a second apart still agree with
/// each other. The world clock replaces this outright when there is one.
#[must_use]
pub fn anchor() -> DateTime<Utc> {
    static ANCHOR: OnceLock<DateTime<Utc>> = OnceLock::new();
    *ANCHOR.get_or_init(Utc::now)
}

/// A moment inside the window, drawn as an instant rather than as separate
/// fields.
///
/// Drawing a year, a month and a day independently cannot produce a valid
/// date, which is why the day used to stop at the 28th — roughly a tenth of
/// real dates fall outside that, and month-end logic could never be exercised
/// against this mock at all. Drawing the instant makes every calendar
/// question, leap years included, someone else's problem.
fn drawn_moment() -> DateTime<Utc> {
    let back = rng().random_range(0..=(WINDOW_YEARS * SECONDS_PER_YEAR));
    anchor() - chrono::TimeDelta::seconds(back)
}

/// Generate a random date in RFC3339 format
pub fn fake_date() -> String {
    drawn_moment().to_rfc3339()
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
    write_date(drawn_moment(), format)
}

/// Write a moment the world already decided on as a date.
#[must_use]
pub fn write_date(moment: DateTime<Utc>, format: crate::type_detector::DateFormat) -> String {
    use crate::type_detector::DateFormat;

    let (year, month, day) = (moment.year(), moment.month(), moment.day());

    match format {
        DateFormat::Iso => format!("{year:04}-{month:02}-{day:02}"),
        DateFormat::Slash => format!("{day:02}/{month:02}/{year:04}"),
        DateFormat::Dotted => format!("{day:02}.{month:02}.{year:04}"),
        DateFormat::Compact => format!("{year:04}{month:02}{day:02}"),
    }
}

/// Generate a moment in time, written the way the field wrote it.
pub fn fake_timestamp_in(format: crate::type_detector::TimestampFormat) -> String {
    write_timestamp(drawn_moment(), format)
}

/// Write a moment the world already decided on, in the field's own format.
#[must_use]
pub fn write_timestamp(
    moment: DateTime<Utc>,
    format: crate::type_detector::TimestampFormat,
) -> String {
    use crate::type_detector::TimestampFormat;

    const OFFSETS: [i32; 6] = [3600, 7200, -18_000, -28_800, 19_800, 32_400];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let date = format!(
        "{:04}-{:02}-{:02}",
        moment.year(),
        moment.month(),
        moment.day()
    );
    let time = format!(
        "{:02}:{:02}:{:02}",
        moment.hour(),
        moment.minute(),
        moment.second()
    );

    match format {
        TimestampFormat::Rfc3339Utc => format!("{date}T{time}Z"),
        // Rendered in the zone rather than stamped with it, so the wall clock
        // and the offset describe the same instant. A field that carried the
        // offset but not the shift was fourteen hours out and still sorted as
        // if it were not.
        TimestampFormat::Rfc3339Offset => {
            let seconds = OFFSETS.choose(&mut rng()).copied().unwrap_or(0);
            FixedOffset::east_opt(seconds).map_or_else(
                || format!("{date}T{time}Z"),
                |zone| {
                    moment
                        .with_timezone(&zone)
                        .format("%Y-%m-%dT%H:%M:%S%:z")
                        .to_string()
                },
            )
        }
        TimestampFormat::Rfc3339Millis => {
            format!("{date}T{time}.{:03}Z", moment.timestamp_subsec_millis())
        }
        TimestampFormat::Rfc3339Nanos => {
            format!(
                "{date}T{time}.{:09}Z",
                rng().random_range(0..1_000_000_000_u32)
            )
        }
        TimestampFormat::SqlDateTime => format!("{date} {time}"),
        // The weekday is a fact about the date, not a seventh choice: picked
        // independently it was wrong six times in seven, and a strict HTTP-date
        // parser rejects it outright.
        TimestampFormat::Rfc2822 | TimestampFormat::HttpDate => {
            let weekday = moment.weekday();
            let month_name = MONTHS
                .get(moment.month0() as usize)
                .copied()
                .unwrap_or("Jan");
            let zone = if matches!(format, TimestampFormat::HttpDate) {
                "GMT"
            } else {
                "+0000"
            };
            format!(
                "{weekday}, {:02} {month_name} {:04} {time} {zone}",
                moment.day(),
                moment.year()
            )
        }
        TimestampFormat::EpochFractional => {
            format!(
                "{}.{:06}",
                moment.timestamp(),
                rng().random_range(0..1_000_000)
            )
        }
    }
}

/// Generate a Unix timestamp
pub fn fake_unix_timestamp() -> i64 {
    drawn_moment().timestamp()
}

/// The instant a written moment names, whatever wrote it.
///
/// Text order is not time order for most of the formats above — `+09:00`
/// sorts before `-05:00` and is fourteen hours later, `9.5` sorts after
/// `10.2`, and a weekday-first RFC 2822 date does not sort at all — so
/// anything comparing two generated moments has to read them first.
#[must_use]
pub fn instant_of(text: &str) -> Option<i64> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
        return Some(parsed.timestamp());
    }
    if let Ok(parsed) = DateTime::parse_from_rfc2822(text) {
        return Some(parsed.timestamp());
    }
    for pattern in ["%Y-%m-%d %H:%M:%S", "%a, %d %b %Y %H:%M:%S GMT"] {
        if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(text, pattern) {
            return Some(parsed.and_utc().timestamp());
        }
    }
    for pattern in ["%Y-%m-%d", "%d/%m/%Y", "%d.%m.%Y", "%Y%m%d"] {
        if let Ok(parsed) = chrono::NaiveDate::parse_from_str(text, pattern) {
            return parsed
                .and_hms_opt(0, 0, 0)
                .map(|at| at.and_utc().timestamp());
        }
    }
    // `1710668482.000100`, where the text sorts by digit count rather than by
    // when it happened.
    text.parse::<f64>()
        .ok()
        .filter(|seconds| seconds.is_finite())
        .map(|seconds| seconds.trunc() as i64)
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
#[allow(
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used
)]
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
        let now = anchor().timestamp();
        let timestamp = fake_unix_timestamp();
        assert!(timestamp <= now, "a record was created in the future");
        assert!(timestamp >= now - WINDOW_YEARS * SECONDS_PER_YEAR);
    }

    /// Roughly a tenth of real dates fall past the 28th, and a naive day draw
    /// could never produce one — so month-end logic could not be exercised
    /// against this mock at all.
    #[test]
    fn a_date_can_land_on_any_day_the_month_actually_has() {
        use crate::type_detector::DateFormat;

        let drawn: Vec<chrono::NaiveDate> = (0..4000)
            .map(|_| fake_date_in(DateFormat::Iso))
            .map(|text| {
                chrono::NaiveDate::parse_from_str(&text, "%Y-%m-%d")
                    .unwrap_or_else(|e| panic!("`{text}` is not a date: {e}"))
            })
            .collect();

        for day in [29, 30, 31] {
            assert!(
                drawn.iter().any(|date| date.day() == day),
                "no date ever fell on the {day}th"
            );
        }
        assert!(
            drawn
                .iter()
                .any(|date| date.month() == 2 && date.day() >= 28),
            "February never reached its own end"
        );
    }

    #[test]
    fn every_written_format_reads_back_as_the_moment_it_names() {
        use crate::type_detector::{DateFormat, TimestampFormat};

        for format in [
            TimestampFormat::Rfc3339Utc,
            TimestampFormat::Rfc3339Offset,
            TimestampFormat::Rfc3339Millis,
            TimestampFormat::Rfc3339Nanos,
            TimestampFormat::SqlDateTime,
            TimestampFormat::Rfc2822,
            TimestampFormat::HttpDate,
            TimestampFormat::EpochFractional,
        ] {
            let written = fake_timestamp_in(format);
            let read = instant_of(&written)
                .unwrap_or_else(|| panic!("{format:?} wrote `{written}`, which nothing can read"));
            let now = anchor().timestamp();
            assert!(
                read <= now && read >= now - WINDOW_YEARS * SECONDS_PER_YEAR,
                "{format:?} wrote `{written}`, outside the window"
            );
        }
        for format in [
            DateFormat::Iso,
            DateFormat::Slash,
            DateFormat::Dotted,
            DateFormat::Compact,
        ] {
            let written = fake_date_in(format);
            assert!(
                instant_of(&written).is_some(),
                "{format:?} wrote `{written}`, which nothing can read"
            );
        }
    }

    /// The weekday is a fact about the date. Chosen from a list of seven it was
    /// wrong six times in seven, and a strict HTTP-date parser rejects it.
    #[test]
    fn an_http_date_names_the_weekday_its_own_date_falls_on() {
        use crate::type_detector::TimestampFormat;

        for format in [TimestampFormat::HttpDate, TimestampFormat::Rfc2822] {
            for _ in 0..200 {
                let written = fake_timestamp_in(format);
                let stated = written.split(',').next().unwrap_or_default().to_string();
                let rest = written.split_once(", ").map(|(_, rest)| rest.to_string());
                let date = rest
                    .as_deref()
                    .and_then(|rest| rest.get(..11))
                    .and_then(|date| chrono::NaiveDate::parse_from_str(date, "%d %b %Y").ok())
                    .unwrap_or_else(|| panic!("`{written}` has no readable date"));
                assert_eq!(
                    stated,
                    date.weekday().to_string(),
                    "`{written}` names a weekday its own date does not fall on"
                );
            }
        }
    }

    #[test]
    fn text_order_is_not_time_order_and_instant_of_knows_it() {
        let east = instant_of("2024-03-17T00:00:00+09:00").unwrap();
        let west = instant_of("2024-03-17T00:00:00-05:00").unwrap();
        assert!(east < west, "the same wall clock further east is earlier");

        let early = instant_of("1710668400.500000").unwrap();
        let late = instant_of("1710668410.100000").unwrap();
        assert!(early < late, "an epoch string sorts by digit count as text");

        assert!(instant_of("Sun, 17 Mar 2024 05:00:00 GMT").is_some());
        assert!(
            instant_of("Tue, 17 Mar 2024 05:00:00 GMT").is_none(),
            "the 17th of March 2024 was a Sunday, and a strict parser knows it"
        );
        assert!(instant_of("2024-03-17 05:00:00").is_some());
        assert!(instant_of("17/03/2024").is_some());
        assert!(instant_of("not a moment").is_none());
    }

    #[test]
    fn test_fake_relative_time() {
        let time = fake_relative_time();
        assert!(!time.is_empty());
        assert!(time.contains("ago"));
    }
}
