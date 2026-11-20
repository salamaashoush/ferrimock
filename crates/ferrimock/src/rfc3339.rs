//! RFC 3339 text for a [`Timestamp`], in the shape chrono wrote it.
//!
//! Recordings, HAR files and API responses from earlier releases carry whole
//! milli-, micro- or nanoseconds; jiff alone trims the fraction to its last
//! non-zero digit, which would change every re-recorded file.

use jiff::Timestamp;
use jiff::tz::Offset;

fn precision(at: Timestamp) -> usize {
    match at.subsec_nanosecond() {
        0 => 0,
        nanos if nanos % 1_000_000 == 0 => 3,
        nanos if nanos % 1_000 == 0 => 6,
        _ => 9,
    }
}

/// `2024-03-17T05:00:00.120+00:00`
pub fn with_utc_offset(at: Timestamp) -> String {
    format!("{:.*}", precision(at), at.display_with_offset(Offset::UTC))
}

/// `2024-03-17T05:00:00.120Z`
#[cfg(feature = "engine")]
pub fn zulu(at: Timestamp) -> String {
    format!("{:.*}", precision(at), at)
}

#[cfg(feature = "engine")]
pub trait Zulu {
    fn serialize_zulu<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error>;
}

#[cfg(feature = "engine")]
impl Zulu for Timestamp {
    fn serialize_zulu<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&zulu(*self))
    }
}

#[cfg(feature = "engine")]
impl Zulu for Option<Timestamp> {
    fn serialize_zulu<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Some(at) => serializer.serialize_some(&zulu(*at)),
            None => serializer.serialize_none(),
        }
    }
}

#[cfg(feature = "engine")]
pub fn serialize<T: Zulu, S: serde::Serializer>(at: &T, serializer: S) -> Result<S::Ok, S::Error> {
    at.serialize_zulu(serializer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fraction_is_whole_milli_micro_or_nanoseconds() -> Result<(), jiff::Error> {
        for (written, expected) in [
            ("2024-03-17T05:00:00Z", "2024-03-17T05:00:00+00:00"),
            ("2024-03-17T05:00:00.12Z", "2024-03-17T05:00:00.120+00:00"),
            (
                "2024-03-17T05:00:00.1234Z",
                "2024-03-17T05:00:00.123400+00:00",
            ),
            (
                "2024-03-17T05:00:00.1234567Z",
                "2024-03-17T05:00:00.123456700+00:00",
            ),
        ] {
            assert_eq!(with_utc_offset(written.parse()?), expected);
        }
        Ok(())
    }

    #[cfg(feature = "engine")]
    #[test]
    fn zulu_is_the_same_instant_with_a_z() -> Result<(), jiff::Error> {
        assert_eq!(
            zulu("2024-03-17T05:00:00Z".parse()?),
            "2024-03-17T05:00:00Z"
        );
        assert_eq!(
            zulu("2024-03-17T14:00:00.5+09:00".parse()?),
            "2024-03-17T05:00:00.500Z"
        );
        Ok(())
    }
}
