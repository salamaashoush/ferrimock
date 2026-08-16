//! The house styles real APIs are written in.
//!
//! A corpus that draws every field the same way teaches a model one API. The
//! dimension that matters is not how many rows there are -- it is how many
//! *conventions* they cover, because a convention is what a model meets when it
//! is pointed at a service nobody trained it on.
//!
//! So generation is organised around families. Each names its fields a
//! particular way, mints ids in a handful of shapes, writes dates in a handful
//! of formats, and serves a particular part of the world. The families are
//! modelled on real, widely-copied APIs, and are deliberately not labelled with
//! the vendors they resemble anywhere a user can see: what is being reproduced
//! is a convention, not a service.
//!
//! The family an example came from is recorded on it, which is what makes the
//! only honest test of "does this work on an API it has never seen" possible:
//! hold a whole family out of training and score on it. See
//! [`crate::eval::held_out`].

/// How a field name is spelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameStyle {
    /// `created_at`
    Snake,
    /// `createdAt`
    Camel,
    /// `CreatedAt`
    Pascal,
    /// `created-at`
    Kebab,
    /// `created.at`
    Dotted,
    /// `CREATED_AT`
    Screaming,
    /// `createdat` -- older services that never separated words at all
    Flat,
}

impl NameStyle {
    /// Render already-lowercased words in this style.
    pub fn render(self, words: &[&str]) -> String {
        let joined_with = |separator: &str| words.join(separator);
        match self {
            Self::Snake => joined_with("_"),
            Self::Kebab => joined_with("-"),
            Self::Dotted => joined_with("."),
            Self::Screaming => joined_with("_").to_uppercase(),
            Self::Flat => joined_with(""),
            Self::Camel => words
                .iter()
                .enumerate()
                .map(|(index, word)| {
                    if index == 0 {
                        (*word).to_string()
                    } else {
                        capitalise(word)
                    }
                })
                .collect(),
            Self::Pascal => words.iter().map(|word| capitalise(word)).collect(),
        }
    }
}

fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// The shape an API mints its identifiers in.
///
/// This is the axis that has grown most since the built-in detector was
/// written. A detector that knows a UUID and a long digit string covers two of
/// the twenty below, and answers `RandomString` to the rest -- which is exactly
/// the residual a model is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdStyle {
    /// `550e8400-e29b-41d4-a716-446655440000`
    UuidV4,
    /// The same, upper case.
    UuidUpper,
    /// The same, with the dashes taken out: 32 hex characters.
    UuidCompact,
    /// `urn:uuid:550e8400-...`
    UrnUuid,
    /// A long digit string, kept as text so it survives a 53-bit float.
    NumericString,
    /// A small integer, as JSON's own number type.
    SmallInt,
    /// A 19-digit time-ordered integer.
    Snowflake,
    /// 24 hex characters.
    ObjectId,
    /// 26 Crockford base32 characters, time-ordered.
    Ulid,
    /// 27 base62 characters, time-ordered.
    Ksuid,
    /// 21 URL-safe characters.
    Nanoid,
    /// `c` followed by base36.
    Cuid,
    /// A type prefix, an underscore, then base62: `cus_9s2Kf3...`
    PrefixedBase62,
    /// A type prefix, a dash, then a hex body: `evt-8fa21c...`
    PrefixedHex,
    /// Base64 of a type and a number, the way relay-style global ids are built.
    OpaqueBase64,
    /// A hex digest.
    HashHex,
    /// A slash-separated resource name: `projects/p/locations/l/instances/i`
    ResourceName,
    /// `arn:cloud:service:region:account:resource/name`
    Arn,
    /// A project key and a counter: `PROJ-1423`
    KeyedCounter,
    /// Two identifiers joined: `file_9931:v4`
    Composite,
    /// A human-readable slug.
    Slug,
}

impl IdStyle {
    /// Every style, so a test can assert the table covers them all.
    pub const ALL: [Self; 21] = [
        Self::UuidV4,
        Self::UuidUpper,
        Self::UuidCompact,
        Self::UrnUuid,
        Self::NumericString,
        Self::SmallInt,
        Self::Snowflake,
        Self::ObjectId,
        Self::Ulid,
        Self::Ksuid,
        Self::Nanoid,
        Self::Cuid,
        Self::PrefixedBase62,
        Self::PrefixedHex,
        Self::OpaqueBase64,
        Self::HashHex,
        Self::ResourceName,
        Self::Arn,
        Self::KeyedCounter,
        Self::Composite,
        Self::Slug,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::UuidV4 => "uuid_v4",
            Self::UuidUpper => "uuid_upper",
            Self::UuidCompact => "uuid_compact",
            Self::UrnUuid => "urn_uuid",
            Self::NumericString => "numeric_string",
            Self::SmallInt => "small_int",
            Self::Snowflake => "snowflake",
            Self::ObjectId => "object_id",
            Self::Ulid => "ulid",
            Self::Ksuid => "ksuid",
            Self::Nanoid => "nanoid",
            Self::Cuid => "cuid",
            Self::PrefixedBase62 => "prefixed_base62",
            Self::PrefixedHex => "prefixed_hex",
            Self::OpaqueBase64 => "opaque_base64",
            Self::HashHex => "hash_hex",
            Self::ResourceName => "resource_name",
            Self::Arn => "arn",
            Self::KeyedCounter => "keyed_counter",
            Self::Composite => "composite",
            Self::Slug => "slug",
        }
    }
}

/// How an API writes a moment in time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DateStyle {
    /// `2024-03-17T09:41:22Z`
    Rfc3339Utc,
    /// `2024-03-17T09:41:22+02:00`
    Rfc3339Offset,
    /// `2024-03-17T09:41:22.481Z`
    Rfc3339Millis,
    /// `2024-03-17T09:41:22.481920374Z`
    Rfc3339Nanos,
    /// `2024-03-17 09:41:22` -- a database column that reached the wire
    SqlDateTime,
    /// `Sun, 17 Mar 2024 09:41:22 +0000`
    Rfc2822,
    /// `Sun, 17 Mar 2024 09:41:22 GMT`
    HttpDate,
    /// Seconds since the epoch.
    EpochSeconds,
    /// Milliseconds since the epoch.
    EpochMillis,
    /// Microseconds since the epoch.
    EpochMicros,
    /// Seconds with a fractional part, as text: `1710668482.000100`
    EpochFractional,
    /// `2024-03-17`
    DateOnly,
    /// `17/03/2024`
    SlashDate,
    /// `17.03.2024`
    DottedDate,
    /// `20240317`
    CompactDate,
    /// `/Date(1710668482000)/`
    WrappedEpoch,
}

impl DateStyle {
    pub const ALL: [Self; 16] = [
        Self::Rfc3339Utc,
        Self::Rfc3339Offset,
        Self::Rfc3339Millis,
        Self::Rfc3339Nanos,
        Self::SqlDateTime,
        Self::Rfc2822,
        Self::HttpDate,
        Self::EpochSeconds,
        Self::EpochMillis,
        Self::EpochMicros,
        Self::EpochFractional,
        Self::DateOnly,
        Self::SlashDate,
        Self::DottedDate,
        Self::CompactDate,
        Self::WrappedEpoch,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Rfc3339Utc => "rfc3339_utc",
            Self::Rfc3339Offset => "rfc3339_offset",
            Self::Rfc3339Millis => "rfc3339_millis",
            Self::Rfc3339Nanos => "rfc3339_nanos",
            Self::SqlDateTime => "sql_datetime",
            Self::Rfc2822 => "rfc2822",
            Self::HttpDate => "http_date",
            Self::EpochSeconds => "epoch_seconds",
            Self::EpochMillis => "epoch_millis",
            Self::EpochMicros => "epoch_micros",
            Self::EpochFractional => "epoch_fractional",
            Self::DateOnly => "date_only",
            Self::SlashDate => "slash_date",
            Self::DottedDate => "dotted_date",
            Self::CompactDate => "compact_date",
            Self::WrappedEpoch => "wrapped_epoch",
        }
    }

    /// Whether this style writes a date as a bare number.
    ///
    /// It decides which label the value truthfully carries: an epoch is a
    /// `unix_timestamp`, a formatted string is a `timestamp`, and generating one
    /// under the other's label would teach the model something false.
    pub fn is_numeric(self) -> bool {
        matches!(
            self,
            Self::EpochSeconds | Self::EpochMillis | Self::EpochMicros
        )
    }

    /// Whether this style carries a time as well as a date.
    pub fn has_time(self) -> bool {
        !matches!(
            self,
            Self::DateOnly | Self::SlashDate | Self::DottedDate | Self::CompactDate
        )
    }
}

/// Which part of the world a value reads as.
///
/// Names, sentences, postal codes, phone numbers and file names all change with
/// it, and several stop being ASCII. A model that has only met Latin text calls
/// a Japanese display name opaque, which is the field that most needed placing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    EnUs,
    EnGb,
    DeDe,
    FrFr,
    EsEs,
    PtBr,
    ItIt,
    NlNl,
    SvSe,
    PlPl,
    TrTr,
    RuRu,
    ElGr,
    ArEg,
    HeIl,
    HiIn,
    ThTh,
    JaJp,
    ZhCn,
    KoKr,
}

impl Locale {
    pub const ALL: [Self; 20] = [
        Self::EnUs,
        Self::EnGb,
        Self::DeDe,
        Self::FrFr,
        Self::EsEs,
        Self::PtBr,
        Self::ItIt,
        Self::NlNl,
        Self::SvSe,
        Self::PlPl,
        Self::TrTr,
        Self::RuRu,
        Self::ElGr,
        Self::ArEg,
        Self::HeIl,
        Self::HiIn,
        Self::ThTh,
        Self::JaJp,
        Self::ZhCn,
        Self::KoKr,
    ];

    pub fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::EnGb => "en-GB",
            Self::DeDe => "de-DE",
            Self::FrFr => "fr-FR",
            Self::EsEs => "es-ES",
            Self::PtBr => "pt-BR",
            Self::ItIt => "it-IT",
            Self::NlNl => "nl-NL",
            Self::SvSe => "sv-SE",
            Self::PlPl => "pl-PL",
            Self::TrTr => "tr-TR",
            Self::RuRu => "ru-RU",
            Self::ElGr => "el-GR",
            Self::ArEg => "ar-EG",
            Self::HeIl => "he-IL",
            Self::HiIn => "hi-IN",
            Self::ThTh => "th-TH",
            Self::JaJp => "ja-JP",
            Self::ZhCn => "zh-CN",
            Self::KoKr => "ko-KR",
        }
    }

    /// The two-letter country this locale sits in.
    pub fn country(self) -> &'static str {
        self.tag().rsplit('-').next().unwrap_or("US")
    }

    /// Whether text in this locale is written outside the Latin alphabet.
    pub fn is_non_latin(self) -> bool {
        matches!(
            self,
            Self::RuRu
                | Self::ElGr
                | Self::ArEg
                | Self::HeIl
                | Self::HiIn
                | Self::ThTh
                | Self::JaJp
                | Self::ZhCn
                | Self::KoKr
        )
    }
}

/// How much of a recording's mess a family carries.
///
/// Real traffic is not clean: fields come back empty, redacted, truncated by a
/// proxy, or filled with a placeholder that means nothing. A corpus without any
/// of it produces a model that has never met the values it will actually be
/// asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoiseLevel {
    /// A carefully-maintained public API.
    Clean,
    /// The common case.
    Typical,
    /// An internal service, or one behind a redacting proxy.
    Messy,
}

impl NoiseLevel {
    /// Chance in sixteen that any one sample is disturbed.
    pub fn per_sample_chance(self) -> u32 {
        match self {
            Self::Clean => 1,
            Self::Typical => 3,
            Self::Messy => 6,
        }
    }
}

/// A family of API conventions.
///
/// Every variant is modelled on the house style of a widely-copied real service
/// -- the naming, the identifier shapes, the date formats -- because those are
/// the conventions a model will actually be pointed at. None of them reproduces
/// a service's data, and the names below describe the convention rather than
/// the vendor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ApiDialect {
    /// snake_case, long numeric string ids, offset timestamps, `entries` and
    /// `total_count` paging.
    ContentPlatform,
    /// snake_case, small integer ids alongside opaque base64 node ids, `Z`
    /// timestamps, `login` and `html_url`.
    DeveloperPlatform,
    /// snake_case, type-prefixed base62 ids, epoch seconds everywhere.
    PaymentsPlatform,
    /// camelCase, slash-separated resource names, nanosecond timestamps,
    /// `nextPageToken`.
    CloudResource,
    /// PascalCase, ARNs, mixed epoch and ISO, `RequestId` envelopes.
    CloudInfrastructure,
    /// camelCase, project keys and counters, millisecond offsets.
    IssueTracker,
    /// PascalCase with custom-field suffixes, 18-character base62 ids.
    EnterpriseCrm,
    /// snake_case, global ids in a URI form, offset timestamps.
    CommercePlatform,
    /// PascalCase, 34-character prefixed hex SIDs, RFC 2822 dates.
    TelephonyPlatform,
    /// snake_case, short prefixed ids, fractional epoch seconds.
    MessagingPlatform,
    /// kebab-case attributes, UUID ids, a typed resource envelope.
    JsonApiService,
    /// PascalCase, `@`-prefixed metadata keys, wrapped epoch dates.
    ODataService,
    /// camelCase, relay-style base64 global ids, `Z` timestamps.
    GraphQlService,
    /// camelCase, ObjectId `_id`, millisecond timestamps.
    DocumentStore,
    /// SCREAMING_SNAKE, sequential integers, compact dates.
    LegacyEnterprise,
    /// snake_case, time-ordered opaque ids, epoch milliseconds.
    InternalMicroservice,
    /// Deliberately inconsistent: a service assembled from several others, which
    /// is what most private APIs actually look like.
    MixedLegacy,
}

impl ApiDialect {
    pub const ALL: [Self; 17] = [
        Self::ContentPlatform,
        Self::DeveloperPlatform,
        Self::PaymentsPlatform,
        Self::CloudResource,
        Self::CloudInfrastructure,
        Self::IssueTracker,
        Self::EnterpriseCrm,
        Self::CommercePlatform,
        Self::TelephonyPlatform,
        Self::MessagingPlatform,
        Self::JsonApiService,
        Self::ODataService,
        Self::GraphQlService,
        Self::DocumentStore,
        Self::LegacyEnterprise,
        Self::InternalMicroservice,
        Self::MixedLegacy,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::ContentPlatform => "content-platform",
            Self::DeveloperPlatform => "developer-platform",
            Self::PaymentsPlatform => "payments-platform",
            Self::CloudResource => "cloud-resource",
            Self::CloudInfrastructure => "cloud-infrastructure",
            Self::IssueTracker => "issue-tracker",
            Self::EnterpriseCrm => "enterprise-crm",
            Self::CommercePlatform => "commerce-platform",
            Self::TelephonyPlatform => "telephony-platform",
            Self::MessagingPlatform => "messaging-platform",
            Self::JsonApiService => "json-api-service",
            Self::ODataService => "odata-service",
            Self::GraphQlService => "graphql-service",
            Self::DocumentStore => "document-store",
            Self::LegacyEnterprise => "legacy-enterprise",
            Self::InternalMicroservice => "internal-microservice",
            Self::MixedLegacy => "mixed-legacy",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|dialect| dialect.name() == name)
    }

    /// How this family spells a field name.
    ///
    /// More than one for the families that never settled on a convention, which
    /// is most private APIs and several public ones.
    pub fn name_styles(self) -> &'static [NameStyle] {
        match self {
            Self::ContentPlatform
            | Self::PaymentsPlatform
            | Self::CommercePlatform
            | Self::MessagingPlatform
            | Self::InternalMicroservice => &[NameStyle::Snake],
            Self::DeveloperPlatform => &[NameStyle::Snake, NameStyle::Snake, NameStyle::Camel],
            Self::CloudResource | Self::GraphQlService | Self::DocumentStore => &[NameStyle::Camel],
            Self::IssueTracker => &[NameStyle::Camel, NameStyle::Camel, NameStyle::Snake],
            Self::CloudInfrastructure | Self::EnterpriseCrm | Self::TelephonyPlatform => {
                &[NameStyle::Pascal]
            }
            Self::JsonApiService => &[NameStyle::Kebab, NameStyle::Kebab, NameStyle::Snake],
            Self::ODataService => &[NameStyle::Pascal, NameStyle::Dotted],
            Self::LegacyEnterprise => &[NameStyle::Screaming, NameStyle::Flat],
            Self::MixedLegacy => &[
                NameStyle::Snake,
                NameStyle::Camel,
                NameStyle::Pascal,
                NameStyle::Kebab,
                NameStyle::Flat,
                NameStyle::Screaming,
            ],
        }
    }

    /// The identifier shapes this family mints, most common first.
    pub fn id_styles(self) -> &'static [IdStyle] {
        match self {
            Self::ContentPlatform => &[
                IdStyle::NumericString,
                IdStyle::NumericString,
                IdStyle::UuidV4,
                IdStyle::Composite,
            ],
            Self::DeveloperPlatform => &[
                IdStyle::SmallInt,
                IdStyle::OpaqueBase64,
                IdStyle::HashHex,
                IdStyle::Slug,
            ],
            Self::PaymentsPlatform => &[
                IdStyle::PrefixedBase62,
                IdStyle::PrefixedBase62,
                IdStyle::PrefixedHex,
            ],
            Self::CloudResource => &[
                IdStyle::ResourceName,
                IdStyle::UuidV4,
                IdStyle::NumericString,
                IdStyle::Slug,
            ],
            Self::CloudInfrastructure => &[
                IdStyle::Arn,
                IdStyle::PrefixedHex,
                IdStyle::UuidV4,
                IdStyle::HashHex,
            ],
            Self::IssueTracker => &[
                IdStyle::KeyedCounter,
                IdStyle::NumericString,
                IdStyle::UuidV4,
            ],
            Self::EnterpriseCrm => &[
                IdStyle::PrefixedBase62,
                IdStyle::UuidUpper,
                IdStyle::SmallInt,
            ],
            Self::CommercePlatform => &[
                IdStyle::Composite,
                IdStyle::Snowflake,
                IdStyle::NumericString,
            ],
            Self::TelephonyPlatform => &[IdStyle::PrefixedHex, IdStyle::UuidCompact],
            Self::MessagingPlatform => {
                &[IdStyle::PrefixedBase62, IdStyle::Composite, IdStyle::Slug]
            }
            Self::JsonApiService => &[IdStyle::UuidV4, IdStyle::UrnUuid, IdStyle::NumericString],
            Self::ODataService => &[IdStyle::UuidUpper, IdStyle::SmallInt, IdStyle::Slug],
            Self::GraphQlService => &[IdStyle::OpaqueBase64, IdStyle::UuidV4, IdStyle::Cuid],
            Self::DocumentStore => &[IdStyle::ObjectId, IdStyle::UuidV4, IdStyle::Nanoid],
            Self::LegacyEnterprise => &[
                IdStyle::SmallInt,
                IdStyle::NumericString,
                IdStyle::KeyedCounter,
            ],
            Self::InternalMicroservice => &[
                IdStyle::Ulid,
                IdStyle::Ksuid,
                IdStyle::Nanoid,
                IdStyle::UuidV4,
                IdStyle::Cuid,
            ],
            Self::MixedLegacy => &IdStyle::ALL,
        }
    }

    /// The date formats this family writes, most common first.
    pub fn date_styles(self) -> &'static [DateStyle] {
        match self {
            Self::ContentPlatform => &[DateStyle::Rfc3339Offset, DateStyle::Rfc3339Utc],
            Self::DeveloperPlatform => &[DateStyle::Rfc3339Utc, DateStyle::EpochSeconds],
            Self::PaymentsPlatform => &[DateStyle::EpochSeconds, DateStyle::EpochSeconds],
            Self::CloudResource => &[
                DateStyle::Rfc3339Nanos,
                DateStyle::Rfc3339Utc,
                DateStyle::DateOnly,
            ],
            Self::CloudInfrastructure => &[
                DateStyle::EpochSeconds,
                DateStyle::Rfc3339Utc,
                DateStyle::HttpDate,
            ],
            Self::IssueTracker => &[DateStyle::Rfc3339Millis, DateStyle::DateOnly],
            Self::EnterpriseCrm => &[DateStyle::Rfc3339Utc, DateStyle::DateOnly],
            Self::CommercePlatform => &[DateStyle::Rfc3339Offset, DateStyle::EpochMillis],
            Self::TelephonyPlatform => &[DateStyle::Rfc2822, DateStyle::Rfc3339Utc],
            Self::MessagingPlatform => &[DateStyle::EpochFractional, DateStyle::EpochSeconds],
            Self::JsonApiService => &[DateStyle::Rfc3339Utc, DateStyle::Rfc3339Millis],
            Self::ODataService => &[DateStyle::WrappedEpoch, DateStyle::Rfc3339Utc],
            Self::GraphQlService => &[DateStyle::Rfc3339Utc, DateStyle::EpochMillis],
            Self::DocumentStore => &[DateStyle::Rfc3339Millis, DateStyle::EpochMillis],
            Self::LegacyEnterprise => &[
                DateStyle::CompactDate,
                DateStyle::SqlDateTime,
                DateStyle::SlashDate,
                DateStyle::DottedDate,
            ],
            Self::InternalMicroservice => &[
                DateStyle::EpochMillis,
                DateStyle::EpochMicros,
                DateStyle::Rfc3339Utc,
            ],
            Self::MixedLegacy => &DateStyle::ALL,
        }
    }

    /// The locales this family's text comes back in, most common first.
    pub fn locales(self) -> &'static [Locale] {
        match self {
            Self::ContentPlatform | Self::DeveloperPlatform | Self::CloudResource => &[
                Locale::EnUs,
                Locale::EnUs,
                Locale::EnGb,
                Locale::DeDe,
                Locale::FrFr,
                Locale::JaJp,
                Locale::ZhCn,
                Locale::PtBr,
            ],
            Self::PaymentsPlatform | Self::CommercePlatform => &[
                Locale::EnUs,
                Locale::EnGb,
                Locale::DeDe,
                Locale::FrFr,
                Locale::EsEs,
                Locale::ItIt,
                Locale::NlNl,
                Locale::SvSe,
                Locale::PtBr,
                Locale::JaJp,
            ],
            Self::CloudInfrastructure | Self::InternalMicroservice | Self::LegacyEnterprise => {
                &[Locale::EnUs, Locale::EnUs, Locale::EnGb]
            }
            Self::IssueTracker | Self::EnterpriseCrm | Self::ODataService => &[
                Locale::EnUs,
                Locale::EnGb,
                Locale::DeDe,
                Locale::FrFr,
                Locale::PlPl,
                Locale::TrTr,
            ],
            Self::TelephonyPlatform => &[
                Locale::EnUs,
                Locale::EnGb,
                Locale::EsEs,
                Locale::PtBr,
                Locale::HiIn,
            ],
            Self::MessagingPlatform | Self::GraphQlService | Self::DocumentStore => &[
                Locale::EnUs,
                Locale::EnGb,
                Locale::JaJp,
                Locale::KoKr,
                Locale::ZhCn,
                Locale::RuRu,
                Locale::ElGr,
            ],
            Self::JsonApiService => &[
                Locale::EnUs,
                Locale::DeDe,
                Locale::FrFr,
                Locale::EsEs,
                Locale::ArEg,
                Locale::HeIl,
            ],
            // The point of this family is that nothing about it is predictable.
            Self::MixedLegacy => &Locale::ALL,
        }
    }

    pub fn noise(self) -> NoiseLevel {
        match self {
            Self::PaymentsPlatform | Self::CloudResource | Self::JsonApiService => {
                NoiseLevel::Clean
            }
            Self::LegacyEnterprise | Self::MixedLegacy | Self::InternalMicroservice => {
                NoiseLevel::Messy
            }
            _ => NoiseLevel::Typical,
        }
    }

    /// A prefix this family puts in front of some field names, if it has one.
    ///
    /// `@odata.etag`, `X-Request-Id` flattened into a body, `attributes.` --
    /// every one of them shifts the name features a model reads.
    pub fn name_prefixes(self) -> &'static [&'static str] {
        match self {
            Self::ODataService => &["@odata", "@"],
            Self::JsonApiService => &["attributes", "meta"],
            Self::DocumentStore => &["_"],
            Self::CloudInfrastructure => &["x", "aws"],
            Self::LegacyEnterprise => &["fld", "col", "sys"],
            Self::MixedLegacy => &["_", "x", "internal", "tmp"],
            _ => &[],
        }
    }

    /// A suffix this family puts after some field names, if it has one.
    pub fn name_suffixes(self) -> &'static [&'static str] {
        match self {
            Self::EnterpriseCrm => &["c", "pc", "r"],
            Self::LegacyEnterprise => &["fld", "val", "txt", "num"],
            Self::MixedLegacy => &["v2", "new", "old", "raw"],
            _ => &[],
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use rustc_hash::FxHashSet;

    #[test]
    fn dialect_names_are_unique_and_round_trip() {
        let mut names: Vec<&str> = ApiDialect::ALL.iter().map(|d| d.name()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total);

        for dialect in ApiDialect::ALL {
            assert_eq!(ApiDialect::from_name(dialect.name()), Some(dialect));
        }
        assert_eq!(ApiDialect::from_name("not-a-dialect"), None);
    }

    #[test]
    fn every_dialect_can_name_a_field_mint_an_id_and_write_a_date() {
        // An empty table would silently narrow the corpus to whatever the
        // fallback happened to be.
        for dialect in ApiDialect::ALL {
            assert!(!dialect.name_styles().is_empty(), "{}", dialect.name());
            assert!(!dialect.id_styles().is_empty(), "{}", dialect.name());
            assert!(!dialect.date_styles().is_empty(), "{}", dialect.name());
            assert!(!dialect.locales().is_empty(), "{}", dialect.name());
        }
    }

    #[test]
    fn every_id_style_and_date_style_is_reachable_from_some_dialect() {
        // A style nothing draws is a style the model never meets, which makes
        // the table a lie about what the corpus covers.
        let ids: FxHashSet<IdStyle> = ApiDialect::ALL
            .iter()
            .flat_map(|dialect| dialect.id_styles().iter().copied())
            .collect();
        for style in IdStyle::ALL {
            assert!(ids.contains(&style), "no dialect mints {}", style.name());
        }

        let dates: FxHashSet<DateStyle> = ApiDialect::ALL
            .iter()
            .flat_map(|dialect| dialect.date_styles().iter().copied())
            .collect();
        for style in DateStyle::ALL {
            assert!(dates.contains(&style), "no dialect writes {}", style.name());
        }
    }

    #[test]
    fn every_locale_is_reachable() {
        let seen: FxHashSet<&str> = ApiDialect::ALL
            .iter()
            .flat_map(|dialect| dialect.locales().iter().map(|locale| locale.tag()))
            .collect();
        for locale in Locale::ALL {
            assert!(
                seen.contains(locale.tag()),
                "no dialect serves {}",
                locale.tag()
            );
        }
    }

    #[test]
    fn a_name_is_spelled_the_way_its_style_says() {
        let words = ["created", "at"];
        assert_eq!(NameStyle::Snake.render(&words), "created_at");
        assert_eq!(NameStyle::Camel.render(&words), "createdAt");
        assert_eq!(NameStyle::Pascal.render(&words), "CreatedAt");
        assert_eq!(NameStyle::Kebab.render(&words), "created-at");
        assert_eq!(NameStyle::Dotted.render(&words), "created.at");
        assert_eq!(NameStyle::Screaming.render(&words), "CREATED_AT");
        assert_eq!(NameStyle::Flat.render(&words), "createdat");
    }

    #[test]
    fn a_single_word_name_survives_every_style() {
        for style in [
            NameStyle::Snake,
            NameStyle::Camel,
            NameStyle::Pascal,
            NameStyle::Kebab,
            NameStyle::Dotted,
            NameStyle::Screaming,
            NameStyle::Flat,
        ] {
            assert!(!style.render(&["email"]).is_empty());
        }
        assert_eq!(NameStyle::Camel.render(&[]), "");
    }

    #[test]
    fn numeric_date_styles_are_the_ones_that_write_a_bare_number() {
        assert!(DateStyle::EpochSeconds.is_numeric());
        assert!(DateStyle::EpochMillis.is_numeric());
        assert!(!DateStyle::Rfc3339Utc.is_numeric());
        assert!(
            !DateStyle::EpochFractional.is_numeric(),
            "a fractional epoch is written as text, and reads as one"
        );
        assert!(!DateStyle::WrappedEpoch.is_numeric());
    }

    #[test]
    fn a_locale_knows_its_country_and_its_script() {
        assert_eq!(Locale::JaJp.country(), "JP");
        assert_eq!(Locale::PtBr.country(), "BR");
        assert!(Locale::ZhCn.is_non_latin());
        assert!(!Locale::SvSe.is_non_latin());
    }

    #[test]
    fn locale_tags_are_unique() {
        let mut tags: Vec<&str> = Locale::ALL.iter().map(|l| l.tag()).collect();
        let total = tags.len();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), total);
    }

    #[test]
    fn the_mixed_family_really_does_cover_everything() {
        // It exists so a corpus always contains one service that refuses to be
        // consistent, which is what a private API usually is.
        assert_eq!(
            ApiDialect::MixedLegacy.id_styles().len(),
            IdStyle::ALL.len()
        );
        assert_eq!(
            ApiDialect::MixedLegacy.date_styles().len(),
            DateStyle::ALL.len()
        );
        assert_eq!(ApiDialect::MixedLegacy.locales().len(), Locale::ALL.len());
    }
}
