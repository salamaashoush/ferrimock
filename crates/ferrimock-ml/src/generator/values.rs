//! Synthesising the values a field was seen holding.
//!
//! Every value here is produced by something that *decided* what it was making,
//! which is the only reason the label attached to it means anything. A value is
//! drawn from three things at once: the label, the family whose conventions the
//! field belongs to, and the locale its text is written in.
//!
//! ## Where a label stops being able to say enough
//!
//! A label is worth attaching when a generator for it produces a value that
//! could have appeared in the field. That test is what decides the awkward
//! cases, and it fails today in three places, which are recorded here rather
//! than smoothed over:
//!
//! - A date written as `Sun, 17 Mar 2024 09:41:22 GMT` is a `timestamp`, but the
//!   template a `timestamp` generates is ISO 8601. The class is right and the
//!   format is not; fixing it means teaching generation to keep the format it
//!   observed, not moving the label.
//! - Every modern identifier shape past a UUID -- ULID, KSUID, nanoid, a
//!   type-prefixed base62 key, a resource name, an ARN -- lands in `opaque`,
//!   because there is no field type that regenerates one. This is the residual
//!   the classifier is supposed to be useful on, and today the most it can
//!   honestly say is "I have no shape for this".
//! - An IPv6 address is `opaque` for the same reason: the address generator only
//!   writes v4.
//!
//! Each is a gap in what the engine can *generate*, and the corpus is written so
//! that closing one is a change to [`label_of_id`] or [`label_of_date`] rather
//! than a regeneration of everything.

use super::dialect::{ApiDialect, DateStyle, IdStyle, Locale};
use super::lexicon;
use super::rng::Rng;
use crate::label::FieldLabel;

/// The conventions one field's values are drawn under.
///
/// Drawn once per field rather than once per value: a field whose samples each
/// picked their own identifier shape would teach a model that agreement between
/// samples means nothing, and agreement is most of what the value features say.
#[derive(Debug, Clone, Copy)]
pub struct FieldStyle {
    pub dialect: ApiDialect,
    pub locale: Locale,
    pub id_style: IdStyle,
    pub date_style: DateStyle,
    /// A per-field draw, so the samples of one field agree on the choices a
    /// real field would not vary.
    ///
    /// A hash column holds hashes of one width, not an eight-character CRC next
    /// to a forty-character SHA-1. Drawing the width per value made every such
    /// field disagree with itself, which is both unrealistic and unfair to a
    /// detector that reasonably asks its samples to look alike.
    pub salt: u64,
    /// Whether the field's name says what it holds.
    ///
    /// It decides whether an irreducibly ambiguous value may be drawn: `1` and
    /// `0` are a real spelling of a boolean, but only a field called
    /// `is_enabled` makes them readable as one. Behind a field called `value`
    /// they cannot be told from a count, and a corpus carrying both readings is
    /// teaching a contradiction rather than a domain.
    pub name_is_informative: bool,
}

impl FieldStyle {
    /// Draw the conventions for one field of `label` in `dialect`.
    ///
    /// The identifier and date shapes are constrained by the label: a family
    /// that mints ARNs still returns UUIDs from the middleware in front of it,
    /// so when the family's own shapes cannot spell the label, one that can is
    /// drawn instead.
    pub fn draw(
        dialect: ApiDialect,
        label: FieldLabel,
        name_is_informative: bool,
        rng: &mut Rng,
    ) -> Self {
        let locale = *rng.choose(dialect.locales()).unwrap_or(&Locale::EnUs);

        let family_ids: Vec<IdStyle> = dialect
            .id_styles()
            .iter()
            .copied()
            .filter(|style| label_of_id(*style) == label)
            .collect();
        let id_pool = if family_ids.is_empty() {
            id_styles_for(label)
        } else {
            family_ids
        };
        let id_style = *rng.choose(&id_pool).unwrap_or(&IdStyle::UuidV4);

        let family_dates: Vec<DateStyle> = dialect
            .date_styles()
            .iter()
            .copied()
            .filter(|style| label_of_date(*style) == label)
            .collect();
        let date_pool = if family_dates.is_empty() {
            date_styles_for(label)
        } else {
            family_dates
        };
        let date_style = *rng.choose(&date_pool).unwrap_or(&DateStyle::Rfc3339Utc);

        Self {
            dialect,
            locale,
            id_style,
            date_style,
            salt: rng.next_u64(),
            name_is_informative,
        }
    }

    /// One of `weights`, drawn once for the field rather than once per value.
    fn stable_choice(self, weights: &[u32]) -> usize {
        let total: u32 = weights.iter().sum();
        if total == 0 {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation)] // bounded by `total`, a u32
        let mut point = (self.salt % u64::from(total)) as u32;
        for (index, weight) in weights.iter().enumerate() {
            if point < *weight {
                return index;
            }
            point -= *weight;
        }
        weights.len() - 1
    }

    /// A length drawn once for the field, inside `low..=high`.
    fn stable_length(self, low: usize, high: usize) -> usize {
        if high <= low {
            return low;
        }
        let span = high - low + 1;
        #[allow(clippy::cast_possible_truncation)] // reduced modulo `span`, a usize already
        let offset = ((self.salt >> 17) % span as u64) as usize;
        low + offset
    }
}

/// The label an identifier of this shape truthfully carries.
///
/// This table decides how much of the identifier space a model can say anything
/// useful about. Everything answering `Opaque` is a shape the engine cannot
/// regenerate; widening [`FieldLabel`] and the field types behind it is what
/// moves an entry off that answer.
pub fn label_of_id(style: IdStyle) -> FieldLabel {
    match style {
        IdStyle::UuidV4 | IdStyle::UuidUpper => FieldLabel::Uuid,
        IdStyle::UuidCompact | IdStyle::ObjectId | IdStyle::HashHex => FieldLabel::HexString,
        IdStyle::NumericString | IdStyle::Snowflake => FieldLabel::NumericStringId,
        IdStyle::SmallInt => FieldLabel::Number,
        IdStyle::OpaqueBase64 => FieldLabel::Base64,
        IdStyle::UrnUuid
        | IdStyle::Ulid
        | IdStyle::Ksuid
        | IdStyle::Nanoid
        | IdStyle::Cuid
        | IdStyle::PrefixedBase62
        | IdStyle::PrefixedHex
        | IdStyle::ResourceName
        | IdStyle::Arn
        | IdStyle::KeyedCounter
        | IdStyle::Composite
        | IdStyle::Slug => FieldLabel::Opaque,
    }
}

/// Every identifier shape that produces `label`.
fn id_styles_for(label: FieldLabel) -> Vec<IdStyle> {
    let matching: Vec<IdStyle> = IdStyle::ALL
        .into_iter()
        .filter(|style| label_of_id(*style) == label)
        .collect();
    if matching.is_empty() {
        vec![IdStyle::UuidV4]
    } else {
        matching
    }
}

/// The label a date written this way truthfully carries.
pub fn label_of_date(style: DateStyle) -> FieldLabel {
    if style.is_numeric() {
        FieldLabel::UnixTimestamp
    } else if style.has_time() {
        FieldLabel::Timestamp
    } else {
        FieldLabel::IsoDate
    }
}

/// Every date format that produces `label`.
fn date_styles_for(label: FieldLabel) -> Vec<DateStyle> {
    let matching: Vec<DateStyle> = DateStyle::ALL
        .into_iter()
        .filter(|style| label_of_date(*style) == label)
        .collect();
    if matching.is_empty() {
        vec![DateStyle::Rfc3339Utc]
    } else {
        matching
    }
}

/// Seconds since the epoch that the corpus's dates are drawn around.
///
/// A fixed window rather than the wall clock: a corpus has to be reproducible,
/// and one regenerated next year would otherwise hold different values.
const EPOCH_WINDOW_START: i64 = 1_546_300_800; // 2019-01-01T00:00:00Z
const EPOCH_WINDOW_SECONDS: i64 = 220_000_000; // a little under seven years

/// One value of `label`, drawn under `style`.
pub fn value(label: FieldLabel, style: FieldStyle, rng: &mut Rng) -> String {
    match label {
        FieldLabel::Uuid => uuid(style.id_style, rng),
        FieldLabel::HexString => hex_identifier(&style, rng),
        FieldLabel::NumericStringId => numeric_identifier(&style, rng),
        FieldLabel::Base64 => base64_value(&style, rng),
        FieldLabel::Opaque => opaque(style, rng),

        FieldLabel::Email => email(style, rng),
        FieldLabel::Url => url(style, rng),
        FieldLabel::ImageUrl => image_url(style, rng),

        FieldLabel::IsoDate | FieldLabel::Timestamp | FieldLabel::UnixTimestamp => {
            render_date(style.date_style, rng)
        }

        FieldLabel::PhoneNumber => phone(style, rng),
        FieldLabel::IpAddress => ipv4(rng),
        FieldLabel::Semver => semver(rng),

        FieldLabel::CountryCode => country_code(style, rng),
        FieldLabel::CurrencyCode => currency_code(style, rng),
        FieldLabel::LocaleCode => locale_code(style, rng),
        FieldLabel::Timezone => timezone(style, rng),
        FieldLabel::PostalCode => lexicon::data(style.locale).postal.render(rng),
        FieldLabel::MimeType => mime_type(rng),

        FieldLabel::FileName => file_name(style, rng),
        FieldLabel::FilePath => file_path(style, rng),

        FieldLabel::Username => username(style, rng),
        FieldLabel::PersonName => person_name(style, rng),
        FieldLabel::Sentence => sentence(style, rng),

        FieldLabel::Token => token(&style, rng),
        FieldLabel::ETag => etag(&style, rng),
        FieldLabel::Boolean => boolean(style, rng),
        FieldLabel::Number => number(&style, rng),
    }
}

fn uuid(style: IdStyle, rng: &mut Rng) -> String {
    let body = format!(
        "{}-{}-4{}-{}{}-{}",
        rng.hex(8),
        rng.hex(4),
        rng.hex(3),
        rng.pick(&["8", "9", "a", "b"]),
        rng.hex(3),
        rng.hex(12)
    );
    if style == IdStyle::UuidUpper {
        body.to_uppercase()
    } else {
        body
    }
}

fn hex_identifier(style: &FieldStyle, rng: &mut Rng) -> String {
    const DIGEST_WIDTHS: [usize; 7] = [8, 16, 32, 40, 56, 64, 128];

    // Width and case are the field's, not the value's: a hash column holds
    // hashes of one width, in one case.
    let length = match style.id_style {
        IdStyle::ObjectId => 24,
        IdStyle::UuidCompact => 32,
        _ => DIGEST_WIDTHS
            .get(style.stable_choice(&[2, 2, 4, 4, 1, 3, 1]))
            .copied()
            .unwrap_or(32),
    };
    if style.stable_choice(&[5, 1]) == 1 {
        rng.hex_upper(length)
    } else {
        rng.hex(length)
    }
}

fn numeric_identifier(style: &FieldStyle, rng: &mut Rng) -> String {
    if style.id_style == IdStyle::Snowflake {
        return rng.digits_no_leading_zero(19);
    }
    // The lengths that collide with a unix timestamp are drawn on purpose:
    // telling a ten-digit id from a ten-digit epoch is the hardest call in this
    // label space, and a corpus that avoids it produces a model that has never
    // had to make it.
    // An id column holds ids of one width.
    match style.stable_choice(&[4, 3, 2, 1, 1]) {
        0 => rng.digits_no_leading_zero(11),
        1 => rng.digits_no_leading_zero(style.stable_length(12, 14)),
        2 => rng.digits_no_leading_zero(10),
        3 => rng.digits_no_leading_zero(style.stable_length(15, 18)),
        // Leading zeros are why these are text rather than numbers.
        _ => format!("0{}", rng.digits(style.stable_length(6, 10))),
    }
}

fn base64_value(style: &FieldStyle, rng: &mut Rng) -> String {
    if style.id_style == IdStyle::OpaqueBase64 {
        // Relay-style global ids: short, and almost always padded.
        return rng.base64(style.stable_length(8, 24));
    }
    // The encoding a field uses is the field's, not the value's.
    match style.stable_choice(&[4, 3, 2, 1]) {
        0 => rng.base64(style.stable_length(24, 64)),
        1 => rng.url_safe(style.stable_length(22, 48)),
        2 => rng.base64(style.stable_length(88, 256)),
        _ => {
            let mime = rng.pick(&["image/png", "application/pdf", "image/jpeg"]);
            let length = rng.between(40, 120);
            format!("data:{mime};base64,{}", rng.base64(length))
        }
    }
}

/// The residual: everything with no shape the engine can regenerate.
///
/// Deliberately the widest arm here. A corpus that fills `opaque` with short
/// random handles teaches a model that opaque means short and random, and then
/// every ULID, ARN and resource name in a real recording is read as something
/// else.
fn opaque(style: FieldStyle, rng: &mut Rng) -> String {
    let locale = lexicon::data(style.locale);
    match style.id_style {
        IdStyle::Ulid => rng.crockford(26),
        IdStyle::Ksuid => rng.base62(27),
        IdStyle::Nanoid => rng.url_safe(21),
        IdStyle::Cuid => format!("c{}", rng.base36(24)),
        IdStyle::PrefixedBase62 => {
            let prefix = rng.pick(&[
                "cus", "acct", "sub", "inv", "txn", "usr", "org", "wrk", "evt", "req", "sess",
                "tok", "job", "run",
            ]);
            let live = if rng.chance(1, 3) { "_live" } else { "" };
            let length = rng.between(14, 24);
            format!("{prefix}{live}_{}", rng.base62(length))
        }
        IdStyle::PrefixedHex => {
            let prefix = rng.pick(&["AC", "SM", "MG", "CA", "evt", "req", "trace", "span"]);
            format!("{prefix}{}", rng.hex(32))
        }
        IdStyle::ResourceName => {
            let project = locale.ascii_word(rng);
            let scope = rng.pick(&["locations", "regions", "zones"]);
            let region = rng.pick(&["us-central1", "europe-west4", "asia-east1"]);
            let kind = rng.pick(&["instances", "buckets", "topics", "datasets"]);
            let length = rng.between(6, 12);
            format!(
                "projects/{project}/{scope}/{region}/{kind}/{}",
                rng.lower_alnum(length)
            )
        }
        IdStyle::Arn => {
            let service = rng.pick(&["s3", "iam", "lambda", "sqs", "dynamodb"]);
            let region = rng.pick(&["us-east-1", "eu-west-2", "ap-southeast-1", ""]);
            let account = rng.digits(12);
            let kind = rng.pick(&["bucket", "role", "function", "queue", "table"]);
            format!(
                "arn:cloud:{service}:{region}:{account}:{kind}/{}",
                locale.ascii_word(rng)
            )
        }
        IdStyle::KeyedCounter => {
            let key_length = rng.between(2, 5);
            let key = rng.from_alphabet(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ", key_length);
            let counter_length = rng.between(1, 5);
            format!("{key}-{}", rng.digits_no_leading_zero(counter_length))
        }
        IdStyle::Composite => {
            let word = locale.ascii_word(rng);
            let length = rng.between(4, 9);
            let number = rng.digits_no_leading_zero(length);
            let version = rng.pick(&["v1", "v2", "v3", "latest", "draft"]);
            format!("{word}_{number}:{version}")
        }
        IdStyle::Slug => {
            let count = rng.between(2, 4);
            let words: Vec<String> = (0..count).map(|_| locale.ascii_word(rng)).collect();
            words.join("-")
        }
        IdStyle::UrnUuid => format!("urn:uuid:{}", uuid(IdStyle::UuidV4, rng)),
        // Shapes with a label of their own still turn up here, because a field
        // can hold something no shape covers at all: a cursor, an address, a
        // colour, a namespaced key.
        _ => match rng.weighted(&[4, 3, 2, 2, 2, 1, 1]) {
            0 => {
                let length = rng.between(5, 12);
                rng.alnum(length)
            }
            1 => {
                let length = rng.between(16, 40);
                rng.url_safe(length)
            }
            2 => format!("#{}", rng.hex(6)),
            3 => ipv6(rng),
            4 => {
                let namespace_length = rng.between(4, 8);
                let namespace = rng.lower_alnum(namespace_length);
                let body_length = rng.between(8, 16);
                format!("{namespace}:{}", rng.base62(body_length))
            }
            5 => {
                let word = locale.ascii_word(rng);
                let length = rng.between(3, 8);
                format!("{word}::{}", rng.digits_no_leading_zero(length))
            }
            _ => {
                let length = rng.between(6, 10);
                rng.from_alphabet(b"ABCDEFGHIJKLMNPQRSTUVWXYZ23456789", length)
            }
        },
    }
}

fn ipv6(rng: &mut Rng) -> String {
    let groups: Vec<String> = (0..8)
        .map(|_| {
            let length = rng.between(1, 4);
            rng.hex(length)
        })
        .collect();
    if rng.chance(1, 3) {
        // The compressed form, which shares almost nothing with the full one.
        let head = groups.first().cloned().unwrap_or_default();
        let tail = groups.last().cloned().unwrap_or_default();
        format!("{head}::{tail}")
    } else {
        groups.join(":")
    }
}

fn email(style: FieldStyle, rng: &mut Rng) -> String {
    let locale = lexicon::data(style.locale);
    let first = locale.ascii_word(rng);
    let second = locale.ascii_word(rng);
    let local = match rng.weighted(&[4, 3, 2, 2, 1, 1]) {
        0 => format!("{first}.{second}"),
        1 => {
            let length = rng.between(1, 3);
            format!("{first}{}", rng.digits(length))
        }
        2 => format!("{first}_{second}"),
        3 => format!("{}{second}", first.chars().next().unwrap_or('a')),
        4 => format!("{first}+{}", rng.lower_alnum(6)),
        _ => {
            let length = rng.between(8, 16);
            rng.lower_alnum(length)
        }
    };
    let domain = rng.pick(locale.domains);
    let address = format!("{local}@{domain}");
    // Addresses come back upper-cased more often than anyone expects.
    if rng.chance(1, 12) {
        address.to_uppercase()
    } else {
        address
    }
}

fn url(style: FieldStyle, rng: &mut Rng) -> String {
    let locale = lexicon::data(style.locale);
    let scheme = if rng.chance(1, 10) { "http" } else { "https" };
    let host = rng.pick(locale.domains);
    let subdomain = match rng.weighted(&[5, 3, 2]) {
        0 => String::new(),
        1 => format!("{}.", rng.pick(&["api", "www", "app", "cdn", "static"])),
        _ => format!("{}.", locale.ascii_word(rng)),
    };
    let port = if rng.chance(1, 20) {
        format!(":{}", rng.pick(&["8080", "8443", "3000", "9000"]))
    } else {
        String::new()
    };

    let depth = rng.between(1, 4);
    let mut path = String::new();
    for _ in 0..depth {
        path.push('/');
        if rng.chance(1, 4) {
            let length = rng.between(3, 8);
            path.push_str(&rng.digits_no_leading_zero(length));
        } else {
            path.push_str(&locale.ascii_word(rng));
        }
    }

    let query = match rng.weighted(&[6, 2, 1, 1]) {
        0 => String::new(),
        1 => {
            let key = rng.pick(&["page", "offset", "limit"]);
            format!("?{key}={}", rng.digits(2))
        }
        2 => {
            let search_key = rng.pick(&["q", "search", "filter"]);
            let term = locale.ascii_word(rng);
            let sort_key = rng.pick(&["sort", "order"]);
            let direction = rng.pick(&["asc", "desc"]);
            format!("?{search_key}={term}&{sort_key}={direction}")
        }
        _ => {
            let length = rng.between(20, 60);
            format!("?token={}", rng.url_safe(length))
        }
    };
    let fragment = if rng.chance(1, 25) {
        format!("#{}", locale.ascii_word(rng))
    } else {
        String::new()
    };

    format!("{scheme}://{subdomain}{host}{port}{path}{query}{fragment}")
}

fn image_url(style: FieldStyle, rng: &mut Rng) -> String {
    let locale = lexicon::data(style.locale);
    let host = match rng.weighted(&[4, 3, 2]) {
        0 => format!("cdn.{}", rng.pick(locale.domains)),
        1 => format!("images.{}", rng.pick(locale.domains)),
        _ => format!("{}.imagecdn.net", rng.lower_alnum(6)),
    };
    let folder = rng.pick(&[
        "avatars",
        "thumbnails",
        "images",
        "assets",
        "media",
        "profile",
    ]);
    let name = match rng.weighted(&[4, 3, 2]) {
        0 => {
            let length = rng.between(16, 32);
            rng.hex(length)
        }
        1 => locale.ascii_word(rng),
        _ => {
            let length = rng.between(4, 10);
            rng.digits_no_leading_zero(length)
        }
    };
    let extension = rng.pick(&["png", "jpg", "jpeg", "webp", "gif", "svg", "avif"]);
    let sizing = match rng.weighted(&[5, 3, 2]) {
        0 => String::new(),
        1 => format!("?s={}", rng.pick(&["64", "128", "256", "512"])),
        _ => {
            let width = rng.pick(&["100", "320", "640"]);
            let height = rng.pick(&["100", "320", "640"]);
            format!("?w={width}&h={height}&fit=crop")
        }
    };
    format!("https://{host}/{folder}/{name}.{extension}{sizing}")
}

/// Days since the epoch, turned into a civil date.
///
/// The standard shift-to-March conversion: moving the year boundary to the start
/// of March makes the leap day the last day of the year, so no month's length
/// depends on whether the year is a leap year.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let march_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * march_month + 2) / 5 + 1;
    let month = if march_month < 10 {
        march_month + 3
    } else {
        march_month - 9
    };

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    // months and days are small and positive
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

/// A moment inside the corpus's fixed window, as its parts.
struct Moment {
    epoch_seconds: i64,
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    /// 0 is Sunday.
    weekday: usize,
}

fn draw_moment(rng: &mut Rng) -> Moment {
    #[allow(clippy::cast_possible_wrap)] // the window is far below i64's range
    let offset = (rng.next_u64() % EPOCH_WINDOW_SECONDS as u64) as i64;
    let epoch_seconds = EPOCH_WINDOW_START + offset;
    let days = epoch_seconds.div_euclid(86_400);
    let time_of_day = epoch_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // bounded by one day
    let (hour, minute, second) = (
        (time_of_day / 3600) as u32,
        ((time_of_day % 3600) / 60) as u32,
        (time_of_day % 60) as u32,
    );
    // 1970-01-01 was a Thursday, which is weekday 4 counting from Sunday.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // modulo seven
    let weekday = (days + 4).rem_euclid(7) as usize;

    Moment {
        epoch_seconds,
        year,
        month,
        day,
        hour,
        minute,
        second,
        weekday,
    }
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn render_date(style: DateStyle, rng: &mut Rng) -> String {
    let at = draw_moment(rng);
    let date = format!("{:04}-{:02}-{:02}", at.year, at.month, at.day);
    let time = format!("{:02}:{:02}:{:02}", at.hour, at.minute, at.second);
    let weekday = WEEKDAYS.get(at.weekday).copied().unwrap_or("Mon");
    let month = MONTHS
        .get((at.month as usize).saturating_sub(1))
        .copied()
        .unwrap_or("Jan");

    match style {
        DateStyle::Rfc3339Utc => format!("{date}T{time}Z"),
        DateStyle::Rfc3339Offset => {
            let offset = rng.pick(&[
                "+01:00", "+02:00", "-05:00", "-08:00", "+05:30", "+09:00", "-03:00", "+00:00",
            ]);
            format!("{date}T{time}{offset}")
        }
        DateStyle::Rfc3339Millis => format!("{date}T{time}.{:03}Z", rng.below(1000)),
        DateStyle::Rfc3339Nanos => format!("{date}T{time}.{:09}Z", rng.below(1_000_000_000)),
        DateStyle::SqlDateTime => format!("{date} {time}"),
        DateStyle::Rfc2822 => {
            format!("{weekday}, {:02} {month} {} {time} +0000", at.day, at.year)
        }
        DateStyle::HttpDate => format!("{weekday}, {:02} {month} {} {time} GMT", at.day, at.year),
        DateStyle::EpochSeconds => at.epoch_seconds.to_string(),
        DateStyle::EpochMillis => {
            let fraction = rng.below(1000);
            #[allow(clippy::cast_possible_wrap)] // bounded by 1000
            let stamp = at.epoch_seconds * 1_000 + fraction as i64;
            stamp.to_string()
        }
        DateStyle::EpochMicros => {
            let fraction = rng.below(1_000_000);
            #[allow(clippy::cast_possible_wrap)] // bounded by a million
            let stamp = at.epoch_seconds * 1_000_000 + fraction as i64;
            stamp.to_string()
        }
        DateStyle::EpochFractional => format!("{}.{:06}", at.epoch_seconds, rng.below(1_000_000)),
        DateStyle::DateOnly => date,
        DateStyle::SlashDate => format!("{:02}/{:02}/{:04}", at.day, at.month, at.year),
        DateStyle::DottedDate => format!("{:02}.{:02}.{:04}", at.day, at.month, at.year),
        DateStyle::CompactDate => format!("{:04}{:02}{:02}", at.year, at.month, at.day),
        DateStyle::WrappedEpoch => format!("/Date({}000)/", at.epoch_seconds),
    }
}

fn phone(style: FieldStyle, rng: &mut Rng) -> String {
    let number = lexicon::data(style.locale).phone.render(rng);
    if rng.chance(1, 14) {
        let length = rng.between(2, 4);
        format!("{number} x{}", rng.digits(length))
    } else {
        number
    }
}

fn ipv4(rng: &mut Rng) -> String {
    match rng.weighted(&[6, 3, 1]) {
        0 => format!(
            "{}.{}.{}.{}",
            rng.between(1, 223),
            rng.below(256),
            rng.below(256),
            rng.between(1, 254)
        ),
        // Private ranges, which is most of what an internal service records.
        1 => format!(
            "10.{}.{}.{}",
            rng.below(256),
            rng.below(256),
            rng.between(1, 254)
        ),
        _ => format!("192.168.{}.{}", rng.below(256), rng.between(1, 254)),
    }
}

fn semver(rng: &mut Rng) -> String {
    let core = format!("{}.{}.{}", rng.below(20), rng.below(40), rng.below(30));
    match rng.weighted(&[6, 2, 1, 1]) {
        0 => core,
        // A leading `v` is everywhere and is not part of the grammar.
        1 => format!("v{core}"),
        2 => {
            let channel = rng.pick(&["alpha", "beta", "rc", "next"]);
            format!("{core}-{channel}.{}", rng.below(10))
        }
        _ => {
            let length = rng.between(3, 6);
            format!("{core}+build.{}", rng.digits(length))
        }
    }
}

fn country_code(style: FieldStyle, rng: &mut Rng) -> String {
    const CODES: [&str; 24] = [
        "US", "GB", "DE", "FR", "ES", "IT", "NL", "SE", "PL", "TR", "RU", "GR", "EG", "IL", "IN",
        "TH", "JP", "CN", "KR", "BR", "CA", "AU", "MX", "ZA",
    ];
    // The field's own country turns up more often than a random one.
    let code = if rng.chance(1, 3) {
        style.locale.country().to_string()
    } else {
        rng.pick(&CODES).to_string()
    };
    if rng.chance(1, 10) {
        code.to_lowercase()
    } else {
        code
    }
}

fn currency_code(style: FieldStyle, rng: &mut Rng) -> String {
    const CODES: [&str; 16] = [
        "USD", "EUR", "GBP", "JPY", "CHF", "AUD", "CAD", "SEK", "PLN", "TRY", "RUB", "BRL", "INR",
        "CNY", "KRW", "ILS",
    ];
    if rng.chance(1, 3) {
        lexicon::data(style.locale).currency.to_string()
    } else {
        rng.pick(&CODES).to_string()
    }
}

fn locale_code(style: FieldStyle, rng: &mut Rng) -> String {
    let tag = style.locale.tag();
    match rng.weighted(&[5, 3, 2, 1]) {
        0 => tag.to_string(),
        1 => tag.replace('-', "_"),
        2 => tag.split('-').next().unwrap_or("en").to_string(),
        // Script subtags and encodings, which nothing shaped like a two-letter
        // code expects to meet.
        _ => match style.locale {
            Locale::ZhCn => rng.pick(&["zh-Hans-CN", "zh-Hant-TW"]).to_string(),
            Locale::PtBr => "pt-BR".to_string(),
            _ => format!("{tag}.UTF-8"),
        },
    }
}

fn timezone(style: FieldStyle, rng: &mut Rng) -> String {
    let zones = lexicon::data(style.locale).timezones;
    match rng.weighted(&[7, 2, 1]) {
        0 => rng.pick(zones).to_string(),
        1 => rng
            .pick(&[
                "UTC",
                "America/Sao_Paulo",
                "Australia/Sydney",
                "Africa/Nairobi",
                "Pacific/Auckland",
            ])
            .to_string(),
        _ => "GMT".to_string(),
    }
}

fn mime_type(rng: &mut Rng) -> String {
    const TYPES: [&str; 20] = [
        "application/json",
        "application/xml",
        "application/pdf",
        "application/zip",
        "application/octet-stream",
        "application/vnd.ms-excel",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "text/plain",
        "text/html",
        "text/csv",
        "text/markdown",
        "image/png",
        "image/jpeg",
        "image/svg+xml",
        "image/heic",
        "video/mp4",
        "audio/mpeg",
        "font/woff2",
        "multipart/form-data",
        "application/graphql",
    ];
    let base = rng.pick(&TYPES).to_string();
    if rng.chance(1, 8) {
        format!("{base}; charset=utf-8")
    } else {
        base
    }
}

fn file_name(style: FieldStyle, rng: &mut Rng) -> String {
    let locale = lexicon::data(style.locale);
    // Names come back in the language the file was named in, which is why the
    // stem is drawn from the locale rather than from ASCII.
    let stem = match rng.weighted(&[4, 3, 2, 2, 1]) {
        0 => rng.pick(locale.words).to_string(),
        1 => format!("{} {}", rng.pick(locale.words), rng.pick(locale.words)),
        2 => {
            let word = locale.ascii_word(rng);
            let length = rng.between(2, 4);
            format!("{word}_{}", rng.digits(length))
        }
        3 => {
            let word = locale.ascii_word(rng);
            let marker = rng.pick(&["final", "v2", "draft", "signed", "copy"]);
            format!("{word}-{marker}")
        }
        _ => {
            let length = rng.between(8, 20);
            rng.hex(length)
        }
    };
    let extension = rng.pick(&[
        "pdf", "docx", "xlsx", "pptx", "png", "jpg", "csv", "zip", "txt", "json", "mp4", "heic",
    ]);
    match rng.weighted(&[8, 1, 1]) {
        0 => format!("{stem}.{extension}"),
        // Double extensions and upper-cased ones both turn up.
        1 => format!("{stem}.tar.gz"),
        _ => format!("{stem}.{}", extension.to_uppercase()),
    }
}

fn file_path(style: FieldStyle, rng: &mut Rng) -> String {
    let locale = lexicon::data(style.locale);
    let leaf = file_name(style, rng);
    match rng.weighted(&[5, 3, 2, 1, 1]) {
        0 => {
            let depth = rng.between(1, 4);
            let parts: Vec<String> = (0..depth).map(|_| locale.ascii_word(rng)).collect();
            format!("/{}/{leaf}", parts.join("/"))
        }
        // A directory, with no file at the end of it.
        1 => {
            let depth = rng.between(2, 5);
            let parts: Vec<String> = (0..depth).map(|_| locale.ascii_word(rng)).collect();
            format!("/{}", parts.join("/"))
        }
        2 => {
            let root = rng.pick(&["Users", "Program Files", "Data"]);
            format!("C:\\{root}\\{}\\{leaf}", locale.ascii_word(rng))
        }
        3 => format!("./{}/{leaf}", locale.ascii_word(rng)),
        _ => {
            let host = rng.lower_alnum(6);
            format!("\\\\{host}\\{}\\{leaf}", locale.ascii_word(rng))
        }
    }
}

fn username(style: FieldStyle, rng: &mut Rng) -> String {
    let locale = lexicon::data(style.locale);
    let first = locale.ascii_word(rng);
    match rng.weighted(&[4, 3, 2, 2, 1]) {
        0 => {
            let length = rng.between(1, 4);
            format!("{first}{}", rng.digits(length))
        }
        1 => format!("{first}.{}", locale.ascii_word(rng)),
        2 => format!("{first}_{}", locale.ascii_word(rng)),
        3 => format!("{first}-{}", locale.ascii_word(rng)),
        _ => {
            let length = rng.between(6, 14);
            rng.lower_alnum(length)
        }
    }
}

fn person_name(style: FieldStyle, rng: &mut Rng) -> String {
    let locale = lexicon::data(style.locale);
    let base = locale.person_name(style.locale, rng);
    match rng.weighted(&[7, 1, 1, 1]) {
        0 => base,
        1 => format!("{} {base}", rng.pick(locale.given_names)),
        2 => format!("{} {base}", rng.pick(&["Dr.", "Prof.", "Mr.", "Ms."])),
        _ => format!("{base}-{}", rng.pick(locale.family_names)),
    }
}

fn sentence(style: FieldStyle, rng: &mut Rng) -> String {
    let locale = lexicon::data(style.locale);
    match rng.weighted(&[5, 3, 2, 1]) {
        0 => {
            let words = rng.between(4, 10);
            locale.sentence(rng, words)
        }
        // Several sentences: a description field rather than a summary.
        1 => {
            let count = rng.between(2, 4);
            let sentences: Vec<String> = (0..count)
                .map(|_| {
                    let words = rng.between(4, 9);
                    locale.sentence(rng, words)
                })
                .collect();
            sentences.join(if locale.spaced { " " } else { "" })
        }
        2 => {
            let words = rng.between(3, 7);
            let body = locale.sentence(rng, words);
            let mark = rng.pick(&["\u{26a0}", "\u{2713}", "\u{2192}"]);
            format!("{mark} {body}")
        }
        _ => {
            let words = rng.between(20, 40);
            locale.sentence(rng, words)
        }
    }
}

fn token(style: &FieldStyle, rng: &mut Rng) -> String {
    // A token column holds one kind of token.
    match style.stable_choice(&[4, 3, 2, 1]) {
        // A JWT, which is three base64url segments and nothing else.
        0 => {
            let header = rng.between(20, 36);
            let payload = rng.between(60, 200);
            let signature = rng.between(40, 64);
            format!(
                "{}.{}.{}",
                rng.url_safe(header),
                rng.url_safe(payload),
                rng.url_safe(signature)
            )
        }
        1 => {
            let length = rng.between(32, 64);
            rng.base62(length)
        }
        2 => {
            let length = rng.between(32, 64);
            rng.hex(length)
        }
        _ => {
            let prefix = rng.pick(&["sk", "pk", "rk", "ghp", "xoxb", "api"]);
            let length = rng.between(24, 40);
            format!("{prefix}_{}", rng.base62(length))
        }
    }
}

fn etag(style: &FieldStyle, rng: &mut Rng) -> String {
    let body = match style.stable_choice(&[4, 3, 2, 1]) {
        0 => {
            let length = rng.between(1, 3);
            rng.digits(length)
        }
        1 => rng.hex(32),
        2 => {
            let length = rng.between(8, 22);
            rng.base62(length)
        }
        _ => {
            let length = rng.between(1, 4);
            format!("{}-{}", rng.hex(16), rng.digits(length))
        }
    };
    // Quoting is a property of the endpoint, not of the individual response.
    match style.stable_choice(&[5, 3, 2]) {
        0 => format!("\"{body}\""),
        1 => body,
        _ => format!("W/\"{body}\""),
    }
}

fn boolean(style: FieldStyle, rng: &mut Rng) -> String {
    // `1` and `0` are only drawn behind a name that says the field is a flag.
    // Behind an uninformative name they cannot be told from a count, and a
    // corpus carrying both readings teaches a contradiction.
    let spellings: &[&str] = if style.name_is_informative {
        &[
            "true", "false", "true", "false", "1", "0", "yes", "no", "Y", "N", "True", "False",
        ]
    } else {
        &["true", "false", "True", "False", "TRUE", "FALSE"]
    };
    rng.pick(spellings).to_string()
}

fn number(style: &FieldStyle, rng: &mut Rng) -> String {
    // A count column holds counts, and a price column holds prices.
    match style.stable_choice(&[5, 4, 3, 2, 2, 1, 1]) {
        0 => rng.between(0, 1000).to_string(),
        1 => rng.between(1000, 100_000_000).to_string(),
        // A size in bytes, which is what most large numbers in an API are.
        2 => rng.between(1024, 8_000_000_000).to_string(),
        3 => format!("{}.{:02}", rng.between(0, 5000), rng.below(100)),
        4 => format!("-{}", rng.between(1, 5000)),
        5 => {
            let degrees = rng.between(0, 180);
            let length = rng.between(4, 7);
            format!("{degrees}.{}", rng.digits(length))
        }
        _ => {
            let exponent = rng.between(1, 12);
            format!("{}.{}e{exponent}", rng.below(10), rng.digits(3))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use rustc_hash::FxHashSet;

    fn drawn(dialect: ApiDialect, label: FieldLabel, seed: u64) -> (FieldStyle, Rng) {
        let mut rng = Rng::seeded(seed);
        let style = FieldStyle::draw(dialect, label, true, &mut rng);
        (style, rng)
    }

    fn sample(label: FieldLabel, dialect: ApiDialect, count: u64) -> Vec<String> {
        (0..count)
            .map(|seed| {
                let (style, mut rng) = drawn(dialect, label, seed + 1);
                value(label, style, &mut rng)
            })
            .collect()
    }

    #[test]
    fn every_label_produces_something_under_every_dialect() {
        for dialect in ApiDialect::ALL {
            for label in FieldLabel::ALL {
                for produced in sample(label, dialect, 4) {
                    assert!(
                        !produced.is_empty(),
                        "{} produced nothing in {}",
                        label.name(),
                        dialect.name()
                    );
                }
            }
        }
    }

    #[test]
    fn a_drawn_style_can_always_spell_the_label_it_was_drawn_for() {
        // The guard against a family whose own identifier shapes cannot spell the
        // label: it has to borrow one that can, or the value carries a label that
        // is simply false.
        for dialect in ApiDialect::ALL {
            for label in [
                FieldLabel::Uuid,
                FieldLabel::HexString,
                FieldLabel::NumericStringId,
                FieldLabel::Base64,
                FieldLabel::Opaque,
            ] {
                for seed in 0..8 {
                    let (style, _) = drawn(dialect, label, seed);
                    assert_eq!(
                        label_of_id(style.id_style),
                        label,
                        "{} drew {} for {}",
                        dialect.name(),
                        style.id_style.name(),
                        label.name()
                    );
                }
            }
            for label in [
                FieldLabel::IsoDate,
                FieldLabel::Timestamp,
                FieldLabel::UnixTimestamp,
            ] {
                for seed in 0..8 {
                    let (style, _) = drawn(dialect, label, seed);
                    assert_eq!(
                        label_of_date(style.date_style),
                        label,
                        "{} drew {} for {}",
                        dialect.name(),
                        style.date_style.name(),
                        label.name()
                    );
                }
            }
        }
    }

    #[test]
    fn a_uuid_is_a_uuid_whatever_its_case() {
        for produced in sample(FieldLabel::Uuid, ApiDialect::JsonApiService, 40) {
            assert_eq!(produced.len(), 36, "{produced}");
            assert_eq!(produced.matches('-').count(), 4, "{produced}");
            assert!(
                produced.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
                "{produced}"
            );
        }
    }

    #[test]
    fn a_timestamp_is_never_written_as_a_bare_number() {
        // The one confusion that would be the corpus's own fault rather than the
        // domain's: an epoch drawn under the `timestamp` label.
        for dialect in ApiDialect::ALL {
            for produced in sample(FieldLabel::Timestamp, dialect, 6) {
                assert!(
                    !produced.chars().all(|c| c.is_ascii_digit()),
                    "{} wrote a timestamp as an epoch: {produced}",
                    dialect.name()
                );
            }
        }
    }

    #[test]
    fn a_unix_timestamp_is_always_a_bare_number() {
        for dialect in ApiDialect::ALL {
            for produced in sample(FieldLabel::UnixTimestamp, dialect, 6) {
                assert!(
                    produced.chars().all(|c| c.is_ascii_digit()),
                    "{} wrote an epoch as text: {produced}",
                    dialect.name()
                );
            }
        }
    }

    #[test]
    fn an_iso_date_never_carries_a_time() {
        for dialect in ApiDialect::ALL {
            for produced in sample(FieldLabel::IsoDate, dialect, 6) {
                assert!(
                    !produced.contains(':'),
                    "{} wrote a time: {produced}",
                    dialect.name()
                );
            }
        }
    }

    #[test]
    fn dates_fall_inside_the_window_the_corpus_fixed() {
        // A corpus regenerated next year has to hold the same values, so nothing
        // here may read the wall clock.
        let mut rng = Rng::seeded(3);
        for _ in 0..500 {
            let at = draw_moment(&mut rng);
            assert!((2019..=2026).contains(&at.year), "{}", at.year);
            assert!((1..=12).contains(&at.month));
            assert!((1..=31).contains(&at.day));
            assert!(at.hour < 24 && at.minute < 60 && at.second < 60);
            assert!(at.weekday < 7);
        }
    }

    #[test]
    fn the_calendar_conversion_agrees_with_dates_that_are_known() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        // A leap day, which is the only thing this algorithm exists to get right.
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn an_epoch_and_a_numeric_id_collide_at_the_lengths_they_really_do() {
        // If they never collide, the corpus has quietly made the hardest call in
        // the label space easy, and the model's score stops meaning anything.
        let ids: FxHashSet<usize> =
            sample(FieldLabel::NumericStringId, ApiDialect::MixedLegacy, 200)
                .iter()
                .map(String::len)
                .collect();
        assert!(ids.contains(&10), "no ten-digit ids: {ids:?}");
        assert!(ids.contains(&11));

        let epochs: FxHashSet<usize> =
            sample(FieldLabel::UnixTimestamp, ApiDialect::MixedLegacy, 200)
                .iter()
                .map(String::len)
                .collect();
        assert!(epochs.contains(&10), "no ten-digit epochs: {epochs:?}");
    }

    #[test]
    fn the_modern_identifier_shapes_are_actually_generated() {
        // The residual is the point of the whole crate, so a regression that
        // stopped drawing these would be invisible in every accuracy number.
        let mut rng = Rng::seeded(13);
        let of = |style: IdStyle, rng: &mut Rng| {
            let field = FieldStyle {
                dialect: ApiDialect::InternalMicroservice,
                locale: Locale::EnUs,
                id_style: style,
                date_style: DateStyle::Rfc3339Utc,
                salt: 0,
                name_is_informative: false,
            };
            value(FieldLabel::Opaque, field, rng)
        };

        assert_eq!(of(IdStyle::Ulid, &mut rng).len(), 26);
        assert_eq!(of(IdStyle::Ksuid, &mut rng).len(), 27);
        assert_eq!(of(IdStyle::Nanoid, &mut rng).len(), 21);
        assert!(of(IdStyle::Cuid, &mut rng).starts_with('c'));
        assert!(of(IdStyle::PrefixedBase62, &mut rng).contains('_'));
        assert!(of(IdStyle::Arn, &mut rng).starts_with("arn:"));
        assert!(of(IdStyle::ResourceName, &mut rng).starts_with("projects/"));
        assert!(of(IdStyle::UrnUuid, &mut rng).starts_with("urn:uuid:"));
    }

    #[test]
    fn every_identifier_shape_produces_a_value_under_the_label_it_carries() {
        let mut rng = Rng::seeded(11);
        for style in IdStyle::ALL {
            let field = FieldStyle {
                dialect: ApiDialect::MixedLegacy,
                locale: Locale::EnUs,
                id_style: style,
                date_style: DateStyle::Rfc3339Utc,
                salt: 0,
                name_is_informative: true,
            };
            let produced = value(label_of_id(style), field, &mut rng);
            assert!(!produced.is_empty(), "{} produced nothing", style.name());
        }
    }

    #[test]
    fn an_email_has_exactly_one_at_and_a_dot_after_it() {
        for dialect in ApiDialect::ALL {
            for produced in sample(FieldLabel::Email, dialect, 8) {
                assert_eq!(produced.matches('@').count(), 1, "{produced}");
                let host = produced.split('@').next_back().unwrap_or("");
                assert!(host.contains('.'), "{produced}");
            }
        }
    }

    #[test]
    fn a_url_always_carries_a_scheme() {
        for dialect in ApiDialect::ALL {
            for produced in sample(FieldLabel::Url, dialect, 6) {
                assert!(
                    produced.starts_with("http://") || produced.starts_with("https://"),
                    "{produced}"
                );
            }
            for produced in sample(FieldLabel::ImageUrl, dialect, 6) {
                assert!(produced.starts_with("https://"), "{produced}");
            }
        }
    }

    #[test]
    fn text_is_written_in_the_script_of_the_locale_that_produced_it() {
        // The failure this guards is subtle: a corpus that draws every locale but
        // renders every value in ASCII has all its diversity in the metadata and
        // none in the data.
        let mut non_ascii = 0;
        for seed in 0..200 {
            let (style, mut rng) = drawn(ApiDialect::MixedLegacy, FieldLabel::PersonName, seed);
            if !value(FieldLabel::PersonName, style, &mut rng).is_ascii() {
                non_ascii += 1;
            }
        }
        assert!(non_ascii > 20, "only {non_ascii} of 200 names left ASCII");
    }

    #[test]
    fn a_flag_is_only_spelled_as_one_and_zero_when_its_name_says_it_is_a_flag() {
        let spellings = |informative: bool| -> FxHashSet<String> {
            (0..300)
                .map(|seed| {
                    let mut rng = Rng::seeded(seed);
                    let style = FieldStyle::draw(
                        ApiDialect::MixedLegacy,
                        FieldLabel::Boolean,
                        informative,
                        &mut rng,
                    );
                    value(FieldLabel::Boolean, style, &mut rng)
                })
                .collect()
        };

        let anonymous = spellings(false);
        assert!(
            !anonymous.contains("1") && !anonymous.contains("0"),
            "an unnamed flag was spelled as a number, which no reader could tell from a count"
        );
        assert!(
            spellings(true).contains("1"),
            "a named flag never met its numeric spelling"
        );
    }

    #[test]
    fn a_value_is_a_function_of_its_seed() {
        for label in FieldLabel::ALL {
            let (style, _) = drawn(ApiDialect::ContentPlatform, label, 99);
            let mut first = Rng::seeded(1234);
            let mut second = Rng::seeded(1234);
            assert_eq!(
                value(label, style, &mut first),
                value(label, style, &mut second),
                "{label} is not reproducible"
            );
        }
    }

    #[test]
    fn one_label_covers_many_shapes_rather_than_one() {
        // A label whose values all look alike is a label the model learns from a
        // single regex, and then meets something else entirely in a recording.
        //
        // Shape count is the wrong measure for a closed vocabulary -- every
        // currency code in the world is three upper-case letters, and covering
        // one shape covers all of it. So each label has to be varied *somehow*,
        // and most of them have to be varied in shape.
        let mut wide = 0;
        for label in FieldLabel::ALL {
            let drawn = sample(label, ApiDialect::MixedLegacy, 120);
            let distinct: FxHashSet<&String> = drawn.iter().collect();
            assert!(
                distinct.len() >= 8,
                "{} produced only {} distinct values in 120 draws",
                label.name(),
                distinct.len()
            );

            let shapes: FxHashSet<String> = drawn
                .iter()
                .map(|produced| crate::generator::census::shape_signature(produced))
                .collect();
            if shapes.len() >= 8 {
                wide += 1;
            }
        }
        assert!(
            wide >= 20,
            "only {wide} of {} labels cover eight shapes or more",
            FieldLabel::ALL.len()
        );
    }
}
