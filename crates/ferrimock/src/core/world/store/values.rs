//! Turning a [`ValueSpec`] into a value, deterministically.
//!
//! Values are derived from `(seed, entity, ordinal, field path)` rather than
//! drawn in sequence. That is what makes a record identical no matter which
//! thread built it, in what order, or whether the ones before it were ever
//! materialised at all — and it is why the store can afford to leave most of
//! the world unbuilt until something asks for it.

use lean_string::LeanString;
use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::core::world::model::{Constraints, FieldDef, Scalar, ScalarKind, TextShape, ValueSpec};
use crate::core::world::store::bus;
use crate::core::world::store::clock;
use crate::core::world::store::distribution::{
    self, Ranking, Spread, falls_within, lopsided_chance,
};
use crate::core::world::store::pattern;
use crate::fake_data::{self, rng};
use crate::type_detector::FieldType;

/// How long a generated collection is when nothing constrains it.
///
/// Two, always, for every array in the world, was zero variance — the
/// cheapest thing in the engine for a client to notice, and no test statistic
/// needed. A real collection is mostly short, sometimes empty, occasionally
/// long.
const LIST_MEAN: f64 = 2.5;

/// A nested list stays short: the product grows fast and nothing reads it.
const NESTED_LIST_MEAN: f64 = 0.8;

const MAX_LIST_LEN: f64 = 12.0;

/// What an unbounded number's support is.
///
/// `1..=1000` was a single-feature giveaway needing no test statistic and
/// about five samples: the support itself was the tell. A real unconstrained
/// integer spans orders of magnitude, so the support does too and the spread
/// is log-uniform inside it — which is also what makes a leading-digit
/// profile read like a real one.
const OPEN_INT_CEILING: f64 = 1_000_000.0;
const OPEN_FLOAT_CEILING: f64 = 100_000.0;

/// How much of an unbounded number's mass sits on zero.
const MOST_ZEROS: f64 = 0.2;

/// Where equal-mass-per-decade starts to matter. Below two decades a declared
/// range is narrow enough that uniform is the honest answer, and Benford does
/// not apply to a rating or a percentage anyway.
const DECADES_WORTH_SPREADING: f64 = 100.0;

/// The derivation context for one field of one record.
#[derive(Debug, Clone, Copy)]
pub struct ValueSeed<'a> {
    pub seed: u64,
    pub entity: &'a str,
    pub ordinal: u64,
    /// When this record came into being.
    pub arrived: i64,
    /// Where it is, and everything that has to agree with that.
    pub place: &'static fake_data::Place,
}

impl<'a> ValueSeed<'a> {
    #[must_use]
    pub fn new(seed: u64, entity: &'a str, ordinal: u64) -> Self {
        Self {
            seed,
            entity,
            ordinal,
            arrived: clock::moment_of(seed, entity, ordinal),
            place: place_of(seed, entity, ordinal),
        }
    }

    /// The place one record's own draw lands in, before anything inherits it.
    #[must_use]
    pub fn place_for(seed: u64, entity: &str, ordinal: u64) -> &'static fake_data::Place {
        place_of(seed, entity, ordinal)
    }

    #[must_use]
    pub fn at_place(mut self, place: &'static fake_data::Place) -> Self {
        self.place = place;
        self
    }

    /// A record whose arrival the store already knows.
    ///
    /// A record a client just created came into being now, whatever ordinal
    /// it derives its values from — its ordinal sits past the census, which
    /// is where the *oldest* instances are.
    #[must_use]
    pub fn arriving(seed: u64, entity: &'a str, ordinal: u64, arrived: i64) -> Self {
        Self {
            seed,
            entity,
            ordinal,
            arrived,
            place: place_of(seed, entity, ordinal),
        }
    }

    /// The stream for one field path, so two fields of the same record never
    /// draw the same bytes.
    fn stream_for(&self, path: &str) -> u64 {
        rng::derive_seed_parts(self.seed, &[self.entity, "#", path], self.ordinal)
    }

    /// A stream beside the value's, for something else about the same field.
    ///
    /// Whether a field is present is not part of the value, and drawing it
    /// from the value's own bytes would tie the two together — a record whose
    /// `bio` happened to hash low would also be the record missing it.
    fn per_record(&self, path: &str, aspect: &str) -> u64 {
        rng::derive_seed_parts(
            self.seed,
            &[self.entity, "#", path, "#", aspect],
            self.ordinal,
        )
    }

    /// A stream for a fact about the field rather than about one record, so
    /// every instance reads the same answer.
    fn per_field(&self, path: &str, aspect: &str) -> u64 {
        rng::derive_seed_parts(
            self.seed,
            &[self.entity, "#", path, "#", aspect, "#field"],
            0,
        )
    }
}

fn place_of(seed: u64, entity: &str, ordinal: u64) -> &'static fake_data::Place {
    fake_data::place_of(rng::derive_seed_parts(seed, &[entity, "#place"], ordinal))
}

/// Whether one field of one record carries a value at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Presence {
    Value,
    Null,
    Absent,
}

/// How often a field the schema said may be missing actually is.
///
/// Drawn per field rather than per record, because that is how a real column
/// behaves: one is null a twentieth of the time and another half the time, and
/// neither is null never. The rate comes off the field's own stream, so it is
/// a property of the schema and the seed rather than of the order records
/// happened to be built in.
const MISSING_FLOOR: f64 = 0.05;
const MISSING_CEILING: f64 = 0.45;

/// A derived word as a uniform draw on `[0, 1)`.
fn unit(derived: u64) -> f64 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "53 bits, which is exactly the f64 mantissa"
    )]
    let scaled = (derived >> 11) as f64 / (1_u64 << 53) as f64;
    scaled
}

fn missing_rate(derived: u64) -> f64 {
    MISSING_CEILING
        .mul_add(unit(derived), MISSING_FLOOR)
        .min(MISSING_CEILING)
}

/// Whether a field appears, appears as null, or does not appear.
///
/// The two are separate answers because the schema gave two separate facts.
/// Omitting the key is what an optional property means; emitting `null` is
/// what a nullable one means, and a `type: string` that is merely optional
/// cannot be null without violating its own schema.
fn presence_of(field: &FieldDef, path: &str, seed: ValueSeed<'_>) -> Presence {
    if !field.may_be_missing() {
        return Presence::Value;
    }
    let drawn = unit(seed.per_record(path, "presence"));
    let mut below = 0.0;
    if !field.required {
        let rate = missing_rate(seed.per_field(path, "absent"));
        if drawn < rate {
            return Presence::Absent;
        }
        below = rate;
    }
    if field.nullable {
        let rate = missing_rate(seed.per_field(path, "null"));
        if drawn < below + rate {
            return Presence::Null;
        }
    }
    Presence::Value
}

/// Generate the value for one field.
pub fn generate(spec: &ValueSpec, path: &str, seed: ValueSeed<'_>) -> JsonValue {
    let derived = seed.stream_for(path);
    let _scope = rng::scope_seeded(derived);
    generate_in_scope(spec, path, seed, derived)
}

/// Generate a whole record's value fields.
///
/// A record's field paths share one buffer, appended and unwound per field
/// rather than formatted fresh: the path is the stream name every value
/// derives from, so it was allocated once per field of every record built.
pub fn generate_fields(
    fields: &[FieldDef],
    prefix: &str,
    seed: ValueSeed<'_>,
) -> JsonMap<String, JsonValue> {
    let mut record = JsonMap::new();
    let mut path = String::from(prefix);
    let mark = path.len();
    for field in fields {
        if field.relation().is_some() {
            continue;
        }
        if mark > 0 {
            path.push('.');
        }
        path.push_str(field.name.as_str());
        match presence_of(field, &path, seed) {
            // The key is simply not there, which is what optional means.
            Presence::Absent => {}
            Presence::Null => {
                record.insert(field.name.to_string(), JsonValue::Null);
            }
            Presence::Value => {
                record.insert(field.name.to_string(), generate(&field.value, &path, seed));
            }
        }
        path.truncate(mark);
    }
    wire(fields, &mut record, &[]);
    record
}

/// Settle the fields of a record that are functions of the others.
///
/// Separate from generating them, and re-runnable, because the store writes a
/// record's key and its links after the values are drawn — an avatar URL
/// ending in the id has to be settled once the id is the one the record is
/// actually filed under.
pub fn wire(fields: &[FieldDef], record: &mut JsonMap<String, JsonValue>, stated: &[String]) {
    order_lifecycle(fields, record);
    bus::wire(fields, record, stated);
    empty_by_state(fields, record);
}

/// Clear the fields a record's own state says it cannot have.
///
/// Last, because it is an implication rather than a value: whatever the
/// generators and the bus decided, an order that has not shipped has no
/// `shipped_at`, and a client reading one would be reading a contradiction.
fn empty_by_state(fields: &[FieldDef], record: &mut JsonMap<String, JsonValue>) {
    for field in fields {
        let ValueSpec::Lifecycle(lifecycle) = &field.value else {
            continue;
        };
        let Some(state) = record
            .get(field.name.as_str())
            .and_then(JsonValue::as_str)
            .and_then(|held| lifecycle.get(held))
        else {
            continue;
        };
        for name in &state.empty {
            if record.contains_key(name.as_str()) {
                record.insert(name.to_string(), JsonValue::Null);
            }
        }
    }
}

/// Where a timestamp sits in a record's life.
///
/// Drawing each field independently is what makes a record derivable without
/// its neighbours, and it is also why `updated_at` lands before `created_at`
/// half the time. Nothing about the *draws* has to change to fix that: the
/// values a record already has can be dealt back out in the order the field
/// names say they happened.
fn lifecycle_rank(field: &str) -> Option<u8> {
    const OPENED: [&str; 12] = [
        "created",
        "registered",
        "added",
        "opened",
        "started",
        "issued",
        "submitted",
        "requested",
        "validfrom",
        "effectivefrom",
        "begins",
        "since",
    ];
    const TOUCHED: [&str; 10] = [
        "updated",
        "modified",
        "changed",
        "edited",
        "synced",
        "published",
        "approved",
        "reviewed",
        "confirmed",
        "lastseen",
    ];
    const CLOSED: [&str; 14] = [
        "completed",
        "finished",
        "resolved",
        "shipped",
        "delivered",
        "paid",
        "closed",
        "ended",
        "expires",
        "expired",
        "deleted",
        "removed",
        "archived",
        "validuntil",
    ];

    let lowered = field.to_ascii_lowercase().replace(['_', '-'], "");
    let has = |stems: &[&str]| stems.iter().any(|stem| lowered.contains(stem));
    if has(&OPENED) {
        return Some(0);
    }
    if has(&TOUCHED) {
        return Some(1);
    }
    if has(&CLOSED) {
        return Some(2);
    }
    None
}

/// How a timestamp is written, so only values that can trade places do.
///
/// The reordering deals one field's value into another, so two fields have to
/// be written the same way or a `Tue, 05 Mar 2024 …` lands in a field the
/// schema said holds an RFC 3339 instant. Grouping by the exact format is what
/// keeps that from happening — `"instant"` covering every timestamp variant
/// meant a date could be dealt against an epoch and an offset against a UTC
/// stamp.
fn timestamp_shape(field: &FieldDef) -> Option<&'static str> {
    let ValueSpec::Scalar(scalar) = &field.value else {
        return None;
    };
    match scalar.semantic.as_ref()? {
        FieldType::Timestamp { format } => Some(format.name()),
        FieldType::IsoDate { format } => Some(format.name()),
        _ => None,
    }
}

/// Deal a record's lifecycle timestamps back out in the order they happened.
///
/// Ordered by the instant each value names rather than by its text. For most
/// of the formats those are different orders: `…T00:00:00+09:00` sorts before
/// `…T00:00:00-05:00` and is fourteen hours later, `9.5` sorts after `10.2`,
/// and an RFC 2822 date leads with a weekday.
fn order_lifecycle(fields: &[FieldDef], record: &mut JsonMap<String, JsonValue>) {
    let mut ordered: Vec<(&'static str, u8, &str)> = fields
        .iter()
        .filter_map(|field| {
            let shape = timestamp_shape(field)?;
            let rank = lifecycle_rank(field.name.as_str())?;
            record
                .get(field.name.as_str())
                .filter(|value| value.is_string())
                .map(|_| (shape, rank, field.name.as_str()))
        })
        .collect();
    if ordered.len() < 2 {
        return;
    }
    // Name breaks a tie, so two fields of the same rank land the same way on
    // every run.
    ordered.sort_unstable();

    let mut shapes: Vec<&'static str> = ordered.iter().map(|(shape, _, _)| *shape).collect();
    shapes.dedup();
    for shape in shapes {
        let group: Vec<&str> = ordered
            .iter()
            .filter(|(held, _, _)| *held == shape)
            .map(|(_, _, name)| *name)
            .collect();
        if group.len() < 2 {
            continue;
        }
        let mut values: Vec<String> = group
            .iter()
            .filter_map(|name| record.get(*name)?.as_str().map(ToString::to_string))
            .collect();
        // A value nothing can read keeps its place rather than being dealt to
        // the front of the record.
        values.sort_by_key(|value| fake_data::instant_of(value).unwrap_or(i64::MAX));
        for (name, value) in group.iter().zip(values) {
            record.insert((*name).to_string(), JsonValue::String(value));
        }
    }
}

fn generate_in_scope(spec: &ValueSpec, path: &str, seed: ValueSeed<'_>, derived: u64) -> JsonValue {
    match spec {
        ValueSpec::Scalar(scalar) => scalar_value(scalar, path, seed, derived),
        ValueSpec::Enum(options) => {
            let ranking = Ranking::of(options.len(), seed.per_field(path, "ranking"));
            options
                .get(ranking.pick(derived))
                .map_or(JsonValue::Null, |v| JsonValue::String(v.to_string()))
        }
        ValueSpec::Lifecycle(lifecycle) => lifecycle
            .weighted(derived)
            .map_or(JsonValue::Null, |state| {
                JsonValue::String(state.name.to_string())
            }),
        ValueSpec::List(inner) => {
            let len = list_len(inner, derived);
            JsonValue::Array(
                (0..len)
                    .map(|i| {
                        let item_path = format!("{path}[{i}]");
                        generate(inner, &item_path, seed)
                    })
                    .collect(),
            )
        }
        ValueSpec::Embedded(fields) => JsonValue::Object(generate_fields(fields, path, seed)),
        // The escape hatch, rendered inside the scope this field already
        // installed — so a template is as reproducible as everything around it,
        // and `{{ fake_email() }}` draws from the same stream the built-in
        // generators do.
        ValueSpec::Template(template) => render(template),
        // A relation is resolved by the store against the entity graph, not
        // invented here; a caller reaching this has asked for the wrong thing.
        ValueSpec::Relation(_) => JsonValue::Null,
    }
}

/// Render an override's template into whatever JSON value it produced.
///
/// A template that yields `42` or `true` is that, not the string of it: the
/// point of the escape hatch is to produce the value the API actually returns.
/// A template that fails to render answers null rather than poisoning the
/// record with its own error text.
fn render(template: &str) -> JsonValue {
    let Ok(rendered) =
        crate::template::render_template(template, &crate::types::RequestContext::new())
    else {
        return JsonValue::Null;
    };
    serde_json::from_str(&rendered).unwrap_or(JsonValue::String(rendered))
}

fn list_len(inner: &ValueSpec, derived: u64) -> usize {
    let mean = match inner {
        ValueSpec::List(_) => NESTED_LIST_MEAN,
        _ => LIST_MEAN,
    };
    // A word of its own: the length of a collection is not a fact about the
    // first element of it.
    let drawn = Spread::Geometric { mean }.draw(derived.rotate_left(41));
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to a collection length either side of the cast"
    )]
    let clamped = drawn.clamp(0.0, MAX_LIST_LEN) as usize;
    clamped
}

fn scalar_value(scalar: &Scalar, path: &str, seed: ValueSeed<'_>, derived: u64) -> JsonValue {
    let value = scalar
        .semantic
        .as_ref()
        .and_then(|semantic| semantic_value(semantic, scalar, path, seed, derived))
        .unwrap_or_else(|| kind_value(scalar, path, seed, derived));

    // A declared pattern is the strictest thing the spec said, so a value that
    // does not satisfy it is wrong however realistic it reads. The realistic
    // value is still preferred when it happens to satisfy the pattern, which
    // is what keeps a permissive `^.+$` from replacing a name with noise.
    let Some(declared) = &scalar.constraints.pattern else {
        return value;
    };
    match &value {
        JsonValue::String(text) if pattern::matches(declared, text) => value,
        JsonValue::String(_) => pattern::generate(
            declared,
            scalar.constraints.min_length,
            scalar.constraints.max_length,
        )
        .map_or(value, JsonValue::String),
        _ => value,
    }
}

/// A value for a field whose meaning the detector recognised.
///
/// Only the variants a spec can actually produce are handled; the rest fall
/// back to the declared kind, which is always safe because the kind is what
/// the schema will be validated against.
fn semantic_value(
    field_type: &FieldType,
    scalar: &Scalar,
    path: &str,
    seed: ValueSeed<'_>,
    derived: u64,
) -> Option<JsonValue> {
    let constraints = &scalar.constraints;
    let ordinal = seed.ordinal;
    let value = match field_type {
        FieldType::Uuid => JsonValue::String(fake_data::fake_uuid()),
        FieldType::Email => JsonValue::String(fake_data::fake_email()),
        FieldType::Username => JsonValue::String(fake_data::fake_username()),
        // Everything a place decides, so a record does not hold a French name
        // beside a `+44` phone and an `America/Bogota` timezone.
        FieldType::Name => JsonValue::String(seed.place.person()),
        // Composed rather than lorem: a title is read by a person, and
        // `perferendis non adipisci asperiores` is the thing that makes a
        // mocked screen look mocked.
        FieldType::Sentence => JsonValue::String(fake_data::fake_headline()),
        FieldType::Paragraph => JsonValue::String(fake_data::fake_prose(2)),
        FieldType::Url | FieldType::ImageUrl => JsonValue::String(fake_data::fake_url()),
        FieldType::IpAddress => JsonValue::String(fake_data::fake_ipv4()),
        FieldType::PhoneNumber => JsonValue::String(seed.place.phone()),
        FieldType::FileName => JsonValue::String(fake_data::fake_filename()),
        FieldType::MimeType => JsonValue::String(fake_data::fake_mime_type()),
        FieldType::CountryCode => JsonValue::String(seed.place.country_code.to_string()),
        FieldType::CurrencyCode => JsonValue::String(seed.place.currency.to_string()),
        FieldType::Timezone => JsonValue::String(seed.place.timezone.to_string()),
        FieldType::LocaleCode => JsonValue::String(seed.place.locale.to_string()),
        FieldType::PostalCode => JsonValue::String(seed.place.postal_code()),
        FieldType::Token => JsonValue::String(fake_data::fake_token()),
        FieldType::ETag => JsonValue::String(fake_data::fake_etag()),
        FieldType::NumericStringId => JsonValue::String(fake_data::fake_numeric_id()),
        FieldType::ApiEndpoint => JsonValue::String(fake_data::fake_api_endpoint()),
        // The world's clock, not a fresh draw: a record's timestamps are a
        // fact about when it arrived, and every field of it moves together.
        FieldType::Timestamp { format } => JsonValue::String(fake_data::write_timestamp(
            moment(seed.arrived, derived, path),
            *format,
        )),
        FieldType::IsoDate { format } => JsonValue::String(fake_data::write_date(
            moment(seed.arrived, derived, path),
            *format,
        )),
        FieldType::Boolean { spelling } => {
            let flag = falls_within(lopsided_chance(seed.per_field(path, "flag")), derived);
            let (falsy, truthy) = spelling.pair();
            if matches!(spelling, crate::type_detector::BooleanSpelling::TrueFalse) {
                JsonValue::Bool(flag)
            } else {
                JsonValue::String(if flag { truthy } else { falsy }.to_string())
            }
        }
        FieldType::Constant(value) => value.clone(),
        FieldType::SequentialNumber { start, step } => {
            // Sequential means sequential *across instances*: the ordinal is
            // the position in the sequence, so the field counts the way the
            // recording it was detected from counted.
            let offset = step.saturating_mul(i64::try_from(ordinal).unwrap_or(i64::MAX));
            JsonValue::Number((*start).saturating_add(offset).into())
        }
        // Both ends were observed in a recording, so they are as good as
        // declared — the spread inside them is what was missing.
        FieldType::RandomNumber { min, max } => {
            let low = constraints.min.map_or(min.unwrap_or(1), clamp_to_i64);
            let high = constraints.max.map_or(max.unwrap_or(1000), clamp_to_i64);
            let spread = bounded_spread(as_f64(low), as_f64(high));
            JsonValue::Number(spread.whole(derived, low, high).into())
        }
        FieldType::RandomFloat { min, max } => {
            let low = constraints.min.or(*min).unwrap_or(0.0);
            let high = constraints.max.or(*max).unwrap_or(1000.0);
            json_float(bounded_spread(low, high).draw(derived), low, high)
        }
        _ => return None,
    };
    Some(value)
}

fn kind_value(scalar: &Scalar, path: &str, seed: ValueSeed<'_>, derived: u64) -> JsonValue {
    let constraints = &scalar.constraints;
    match &scalar.kind {
        ScalarKind::Boolean => JsonValue::Bool(falls_within(
            lopsided_chance(seed.per_field(path, "flag")),
            derived,
        )),
        ScalarKind::Int => {
            let bounded = constraints.min.is_some() || constraints.max.is_some();
            let low = constraints.min.map_or(0, clamp_to_i64);
            let high = constraints.max.unwrap_or(OPEN_INT_CEILING).max(1.0);
            let spread = if bounded {
                bounded_spread(as_f64(low), high)
            } else {
                open_spread(1.0, OPEN_INT_CEILING, path, seed)
            };
            JsonValue::Number(spread.whole(derived, low, clamp_to_i64(high)).into())
        }
        ScalarKind::Float => {
            let bounded = constraints.min.is_some() || constraints.max.is_some();
            let low = constraints.min.unwrap_or(0.0);
            let high = constraints.max.unwrap_or(OPEN_FLOAT_CEILING);
            let spread = if bounded {
                bounded_spread(low, high)
            } else {
                open_spread(0.01, OPEN_FLOAT_CEILING, path, seed)
            };
            json_float(spread.draw(derived), low, high)
        }
        ScalarKind::Id => JsonValue::String(fake_data::fake_uuid()),
        ScalarKind::String | ScalarKind::Custom(_) => JsonValue::String(match scalar.shape {
            TextShape::Prose => bounded_string(constraints, path),
            // A closed set the field name actually implies, drawn the way an
            // enum is: `"status": "perferendis"` is not a value a distribution
            // can fix, because it does not mean what the field says.
            TextShape::Word => {
                let vocabulary = fake_data::token_vocabulary(path);
                let ranking = Ranking::of(vocabulary.len(), seed.per_field(path, "ranking"));
                vocabulary
                    .get(ranking.pick(derived))
                    .map_or_else(String::new, |token| (*token).to_string())
            }
            TextShape::Slug => fake_data::fake_slug(),
        }),
    }
}

/// A string that satisfies whatever length bounds the spec set.
///
/// A composed phrase is the readable default — a `String` a schema said nothing
/// else about is still going to be read by someone. Bounds are met by
/// truncating or padding, because a spec that says `maxLength: 8` means it.
fn bounded_string(constraints: &Constraints, path: &str) -> String {
    let mut text = fake_data::fake_headline();
    if let Some(min) = constraints.min_length {
        while text.chars().count() < min {
            text.push(' ');
            text.push_str(&fake_data::fake_label());
        }
    }
    if let Some(max) = constraints.max_length
        && text.chars().count() > max
    {
        text = text.chars().take(max).collect();
    }
    if text.is_empty() {
        // A zero-length bound is legal and a caller still needs something
        // stable to key on; the path is the only stable thing available.
        text = path
            .chars()
            .take(constraints.max_length.unwrap_or(0))
            .collect();
    }
    text
}

/// The instant one timestamp field of a record names.
///
/// A record was created, then touched, then closed, so each field sits its own
/// distance past the record's arrival and `order_lifecycle` deals the results
/// back out in the order the names imply.
fn moment(arrived: i64, derived: u64, path: &str) -> chrono::DateTime<chrono::Utc> {
    let leaf = path.rsplit('.').next().unwrap_or(path);
    let stage = lifecycle_rank(leaf).unwrap_or(0);
    chrono::DateTime::from_timestamp(clock::field_moment(arrived, derived, stage), 0)
        .unwrap_or_else(chrono::Utc::now)
}

/// How a number inside a declared range is spread.
///
/// Two decades or more and equal-mass-per-decade is the realistic answer;
/// below that the range is narrow enough that uniform is, and the digit
/// profile a wider field would have does not apply to a rating or a
/// percentage.
fn bounded_spread(low: f64, high: f64) -> Spread {
    if low > 0.0 && high / low >= DECADES_WORTH_SPREADING {
        Spread::LogUniform { low, high }
    } else {
        Spread::Uniform { low, high }
    }
}

/// How a number nothing bounded is spread.
///
/// Orders of magnitude, and sometimes zero: an unconstrained integer in a real
/// payload is a count, a size or a balance, and none of those are uniform on a
/// three-decade window that never reaches a fourth.
fn open_spread(low: f64, high: f64, path: &str, seed: ValueSeed<'_>) -> Spread {
    Spread::ZeroInflated {
        zero: distribution::unit(seed.per_field(path, "zeroes")) * MOST_ZEROS,
        inner: Box::new(Spread::LogUniform { low, high }),
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "a declared bound, compared against another as a magnitude"
)]
fn as_f64(bound: i64) -> f64 {
    bound as f64
}

/// A declared bound is a float even when the field is an integer; the whole
/// number inside it is what the bound actually means.
fn clamp_to_i64(bound: f64) -> i64 {
    if bound.is_nan() {
        return 0;
    }
    if bound >= 9_223_372_036_854_775_000.0 {
        return i64::MAX;
    }
    if bound <= -9_223_372_036_854_775_000.0 {
        return i64::MIN;
    }
    #[allow(clippy::cast_possible_truncation)]
    let truncated = bound as i64;
    truncated
}

/// JSON numbers cannot be NaN or infinite; a bound that produces one is a spec
/// error, and null is the honest answer rather than a silently clamped value.
///
/// Two decimals is what a payload actually carries: `706.3558368819936` is a
/// legal answer for a `number`, but no API writes one, and the extra digits are
/// the first thing that makes a mocked payload read as generated. Bounds
/// narrower than a hundredth keep the full value, because there the precision
/// is the point.
fn json_float(value: f64, low: f64, high: f64) -> JsonValue {
    let rounded = (value * 100.0).round() / 100.0;
    let kept = if rounded >= low && rounded <= high {
        rounded
    } else {
        value
    };
    serde_json::Number::from_f64(kept).map_or(JsonValue::Null, JsonValue::Number)
}

/// The key value for the `ordinal`th derived instance of an entity.
///
/// Keys must be derivable without materialising the record: the census hands
/// out keys, and only a read that actually happens builds the fields.
/// When an instance came into being.
///
/// An id has to carry this or it cannot sort. A sequential number already
/// does, because ordinal and age rise together; an opaque one has to embed it.
#[derive(Debug, Clone, Copy)]
pub struct Arrival {
    pub moment: i64,
}

impl Arrival {
    #[must_use]
    pub fn seeded(seed: u64, entity: &str, ordinal: u64) -> Self {
        Self {
            moment: clock::moment_of(seed, entity, ordinal),
        }
    }

    /// A record a client made, which is newer than anything the seed derived.
    #[must_use]
    pub const fn created(moment: i64) -> Self {
        Self { moment }
    }
}

pub fn derive_key_value(
    seed: u64,
    entity: &str,
    field: Option<&str>,
    key_field: &Scalar,
    ordinal: u64,
    arrival: Arrival,
) -> LeanString {
    // A key of one part keeps the stream and the wording it has always had, so
    // adding composite keys did not renumber every world that already exists —
    // `derive_seed` is documented as stable across releases, and a key is the
    // most visible thing it derives. Each part of a composite key is named by
    // its own field instead, or `/repos/{owner}/{repo}` answers with a repo
    // whose owner is called `repo-3`.
    let label = field.unwrap_or(entity);
    let stream = match field {
        None => format!("{entity}#key"),
        Some(field) => format!("{entity}#key:{field}"),
    };
    let derived = rng::derive_seed(seed, &stream, ordinal);
    let _scope = rng::scope_seeded(derived);

    // The declared kind is checked before the detected meaning, and that order
    // is the whole point: a field named `id` reads as a uuid to the detector,
    // so a document declaring `id: { type: integer }` used to be keyed by uuids
    // — which made `GET /users/1` a 404 on every integer-keyed API there is.
    // Ordinal and age rise together, so counting from one already counts the
    // way creation time does — and `GET /orders/1` resolving is what makes an
    // integer-keyed document usable by hand.
    let numbered = || LeanString::from((ordinal + 1).to_string());
    // A `format: uuid` is the document's own answer and stands. Everything
    // else was a guess from a field name, and a v4 uuid is the one family that
    // carries neither a count nor a clock — so sorting a collection by id put
    // it in an order unrelated to anything, which no real API does.
    let declared_uuid = key_field
        .constraints
        .format
        .as_deref()
        .is_some_and(|format| format.eq_ignore_ascii_case("uuid"));

    match &key_field.kind {
        ScalarKind::Int | ScalarKind::Float => numbered(),
        _ if declared_uuid => fake_data::fake_uuid().into(),
        _ => match &key_field.semantic {
            Some(FieldType::NumericStringId) => fake_data::fake_numeric_id().into(),
            Some(FieldType::SequentialNumber { .. }) => numbered(),
            Some(FieldType::Uuid) => stamped_id(arrival.moment, derived).into(),
            _ if key_field.kind == ScalarKind::Id => stamped_id(arrival.moment, derived).into(),
            _ => LeanString::from(format!(
                "{}_{}",
                fake_data::id_prefix(label),
                stamped_id(arrival.moment, derived)
            )),
        },
    }
}

/// An opaque id that sorts the way its record was created.
fn stamped_id(moment: i64, derived: u64) -> String {
    fake_data::ulid_at(
        moment.saturating_mul(1000),
        derived,
        derived.rotate_left(31) ^ 0x9E37_79B9_7F4A_7C15,
    )
}

/// A key rendered as the kind the schema declared it.
///
/// Keys are held as text because that is what a path segment and a cursor are,
/// but a payload has to carry the declared type: a client that POSTs
/// `{"id": 42}` against `id: { type: integer }` and reads back `{"id": "42"}`
/// has watched the mock change the type under it.
#[must_use]
pub fn key_json(kind: &ScalarKind, value: &str) -> JsonValue {
    match kind {
        ScalarKind::Int => value
            .parse::<i64>()
            .map_or_else(|_| JsonValue::String(value.to_string()), JsonValue::from),
        ScalarKind::Float => value
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(|| JsonValue::String(value.to_string()), JsonValue::Number),
        _ => JsonValue::String(value.to_string()),
    }
}

/// A key as text, however the payload happened to write it.
#[must_use]
pub fn key_text(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn scalar(kind: ScalarKind) -> ValueSpec {
        ValueSpec::Scalar(Scalar::new(kind))
    }

    fn drawn_values(field: FieldDef, count: u64) -> Vec<JsonValue> {
        let name = field.name.to_string();
        let fields = vec![field];
        (0..count)
            .filter_map(|ordinal| {
                generate_fields(&fields, "", ValueSeed::new(11, "Doc", ordinal))
                    .get(&name)
                    .cloned()
            })
            .collect()
    }

    fn bounded(kind: ScalarKind, min: Option<f64>, max: Option<f64>) -> FieldDef {
        let mut inner = Scalar::new(kind);
        inner.constraints.min = min;
        inner.constraints.max = max;
        FieldDef::new("n", ValueSpec::Scalar(inner), false)
    }

    /// The support was the tell: no test statistic needed and about five
    /// samples enough.
    #[test]
    fn an_unconstrained_number_is_not_capped_at_a_round_thousand() {
        let drawn: Vec<f64> = drawn_values(bounded(ScalarKind::Int, None, None), 2000)
            .into_iter()
            .filter_map(|value| value.as_f64())
            .collect();
        assert_eq!(drawn.len(), 2000);

        let largest = drawn.iter().copied().fold(f64::MIN, f64::max);
        assert!(largest > 1000.0, "largest was {largest}");
        assert!(drawn.contains(&0.0), "never zero");

        // Equal mass per decade, which is what a leading-digit profile is made
        // of: a flat draw over the same support would put a thousandth here.
        let small = drawn.iter().filter(|value| **value < 1000.0).count();
        assert!(
            small > drawn.len() / 4,
            "only {small} of {} fell in the lower three decades",
            drawn.len()
        );
    }

    #[test]
    fn a_declared_range_is_still_the_range() {
        let drawn: Vec<f64> = drawn_values(bounded(ScalarKind::Int, Some(10.0), Some(20.0)), 500)
            .into_iter()
            .filter_map(|value| value.as_f64())
            .collect();
        assert!(drawn.iter().all(|value| (10.0..=20.0).contains(value)));

        let mut distinct: Vec<String> = drawn.iter().map(f64::to_string).collect();
        distinct.sort_unstable();
        distinct.dedup();
        assert!(distinct.len() > 5, "a range should be used: {distinct:?}");
    }

    /// A collection of exactly two, every time, is zero variance.
    #[test]
    fn a_list_is_not_always_two_long() {
        let field = FieldDef::new(
            "tags",
            ValueSpec::List(Box::new(scalar(ScalarKind::String))),
            false,
        );
        let lengths: Vec<usize> = drawn_values(field, 500)
            .into_iter()
            .filter_map(|value| value.as_array().map(Vec::len))
            .collect();

        let mut distinct = lengths.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert!(distinct.len() > 3, "lengths were {distinct:?}");
        assert!(lengths.contains(&0), "never empty");
        assert!(lengths.iter().any(|len| *len > 4), "never long");
    }

    /// Half the users of an API are not administrators.
    #[test]
    fn a_boolean_is_not_a_fair_coin() {
        let field = FieldDef::new("flag", scalar(ScalarKind::Boolean), false);
        let set = drawn_values(field, 1000)
            .into_iter()
            .filter(|value| value == &JsonValue::Bool(true))
            .count();
        assert!(
            !(450..=550).contains(&set),
            "a boolean read as a coin flip: {set} of 1000"
        );
    }

    #[test]
    fn an_enum_is_skewed_rather_than_flat() {
        let members = ["draft", "review", "live", "archived"];
        let field = FieldDef::new(
            "status",
            ValueSpec::Enum(members.iter().map(|m| (*m).into()).collect()),
            false,
        );
        let drawn = drawn_values(field, 1200);
        let counts: Vec<usize> = members
            .iter()
            .map(|member| {
                drawn
                    .iter()
                    .filter(|value| value.as_str() == Some(*member))
                    .count()
            })
            .collect();
        assert_eq!(counts.iter().sum::<usize>(), 1200);
        assert!(counts.iter().all(|count| *count > 0), "{counts:?}");

        let most = counts.iter().copied().max().unwrap_or(0);
        let least = counts.iter().copied().min().unwrap_or(0);
        assert!(most > least * 2, "a flat enum: {counts:?}");
    }

    fn shaped(name: &str, shape: TextShape) -> FieldDef {
        let inner = Scalar::new(ScalarKind::String).with_shape(shape);
        FieldDef::new(name, ValueSpec::Scalar(inner), false)
    }

    /// `"status": "perferendis"` is not a value a distribution can fix: it does
    /// not mean what the field name says, and a client switching on it breaks
    /// on the first record.
    #[test]
    fn a_short_token_field_holds_a_word_its_own_name_implies() {
        for (name, expected) in [
            ("status", "active"),
            ("sync_state", "pending"),
            ("collection_type", "standard"),
            ("member_role", "owner"),
            ("log_level", "critical"),
        ] {
            let vocabulary = fake_data::token_vocabulary(name);
            assert!(
                vocabulary.contains(&expected),
                "`{name}` should draw from a set holding `{expected}`"
            );
            let drawn = drawn_values(shaped(name, TextShape::Word), 300);
            for value in &drawn {
                let held = value.as_str().unwrap_or_default();
                assert!(
                    vocabulary.contains(&held),
                    "`{name}` answered `{held}`, which is not one of its own tokens"
                );
            }
            let mut distinct: Vec<&str> = drawn.iter().filter_map(|value| value.as_str()).collect();
            distinct.sort_unstable();
            distinct.dedup();
            assert!(distinct.len() > 1, "`{name}` only ever answers one thing");
        }
    }

    #[test]
    fn a_slug_is_built_from_words_a_person_could_have_written() {
        let stems = fake_data::slug_stems();
        for value in drawn_values(shaped("share_slug", TextShape::Slug), 300) {
            let held = value.as_str().unwrap_or_default();
            assert!(!held.is_empty());
            let words: Vec<&str> = held.split('-').collect();
            assert!((2..=4).contains(&words.len()), "{held}");
            for word in words {
                assert!(
                    stems.contains(&word) || word.chars().all(|c| c.is_ascii_digit()),
                    "`{held}` holds `{word}`, which is not a word"
                );
            }
        }
    }

    /// `required` and `nullable` are separate answers, so an optional field
    /// loses its key and a nullable one keeps it holding null. Emitting null
    /// for a merely-optional `type: string` violates the schema that declared
    /// it.
    #[test]
    fn an_optional_field_goes_missing_and_a_nullable_one_goes_null() {
        let fields = vec![
            FieldDef::new("id", scalar(ScalarKind::Id), false),
            FieldDef::new("subtitle", scalar(ScalarKind::String), false).optional(),
            FieldDef::new("bio", scalar(ScalarKind::String), true),
        ];

        let mut absent = 0;
        let mut nulled = 0;
        for ordinal in 0..400 {
            let record = generate_fields(&fields, "", ValueSeed::new(4, "Doc", ordinal));
            assert!(
                record.contains_key("id"),
                "a required field is always there"
            );
            assert!(
                !record.get("subtitle").is_some_and(JsonValue::is_null),
                "an optional field that is not nullable is absent, never null"
            );
            if !record.contains_key("subtitle") {
                absent += 1;
            }
            assert!(record.contains_key("bio"), "a nullable field keeps its key");
            if record["bio"].is_null() {
                nulled += 1;
            }
        }
        assert!((20..380).contains(&absent), "absent {absent} of 400");
        assert!((20..380).contains(&nulled), "null {nulled} of 400");
    }

    /// The rate belongs to the field, not to the record: one column is null a
    /// twentieth of the time and another half the time.
    #[test]
    fn two_optional_fields_go_missing_at_different_rates() {
        let rates: Vec<usize> = ["alpha", "beta", "gamma", "delta"]
            .into_iter()
            .map(|name| {
                let fields = vec![FieldDef::new(name, scalar(ScalarKind::String), true)];
                (0..400)
                    .filter(|ordinal| {
                        generate_fields(&fields, "", ValueSeed::new(4, "Doc", *ordinal))[name]
                            .is_null()
                    })
                    .count()
            })
            .collect();
        let mut distinct = rates.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), rates.len(), "{rates:?}");
    }

    fn stamped(name: &str, format: crate::type_detector::TimestampFormat) -> FieldDef {
        let mut inner = Scalar::new(ScalarKind::String);
        inner.semantic = Some(FieldType::Timestamp { format });
        FieldDef::new(name, ValueSpec::Scalar(inner), false)
    }

    fn held(record: &JsonMap<String, JsonValue>, name: &str) -> String {
        record
            .get(name)
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_string()
    }

    /// `created` before `updated` has to mean the moment, not the text.
    ///
    /// Two instants an hour apart in opposite zones sort by their local wall
    /// clocks, which is not the order they happened in: `20:00+09:00` is
    /// 11:00Z and `10:00-05:00` is 15:00Z, so the text puts the later one
    /// first.
    #[test]
    fn a_lifecycle_orders_by_the_moment_rather_than_by_the_text() {
        use crate::type_detector::TimestampFormat;

        let fields = vec![
            stamped("created_at", TimestampFormat::Rfc3339Offset),
            stamped("updated_at", TimestampFormat::Rfc3339Offset),
        ];
        let mut record = JsonMap::new();
        record.insert(
            "created_at".to_string(),
            JsonValue::from("2024-03-17T10:00:00-05:00"),
        );
        record.insert(
            "updated_at".to_string(),
            JsonValue::from("2024-03-17T20:00:00+09:00"),
        );
        order_lifecycle(&fields, &mut record);

        let created = fake_data::instant_of(&held(&record, "created_at"));
        let updated = fake_data::instant_of(&held(&record, "updated_at"));
        assert!(created <= updated, "updated before created: {record:?}");
    }

    /// Two lifecycle fields written differently cannot trade places: dealing a
    /// `Sun, 17 Mar 2024 …` into a field the schema said holds an RFC 3339
    /// instant changes its type, and no client survives a field whose format
    /// depends on which record it is reading.
    #[test]
    fn a_lifecycle_only_reorders_fields_written_the_same_way() {
        use crate::type_detector::TimestampFormat;

        let fields = vec![
            stamped("created_at", TimestampFormat::HttpDate),
            stamped("updated_at", TimestampFormat::Rfc3339Utc),
        ];
        for ordinal in 0..200 {
            let record = generate_fields(&fields, "", ValueSeed::new(7, "Doc", ordinal));
            assert!(held(&record, "created_at").ends_with("GMT"), "{record:?}");
            assert!(held(&record, "updated_at").ends_with('Z'), "{record:?}");
        }
    }

    #[test]
    fn the_same_field_of_the_same_record_is_always_the_same() {
        let seed = ValueSeed::new(42, "User", 7);
        let a = generate(&scalar(ScalarKind::String), "name", seed);
        let b = generate(&scalar(ScalarKind::String), "name", seed);
        assert_eq!(a, b);
    }

    #[test]
    fn ordering_does_not_change_a_record() {
        let spec = scalar(ScalarKind::String);
        let forwards: Vec<_> = (0..5)
            .map(|i| generate(&spec, "name", ValueSeed::new(42, "User", i)))
            .collect();
        let backwards: Vec<_> = (0..5)
            .rev()
            .map(|i| generate(&spec, "name", ValueSeed::new(42, "User", i)))
            .rev()
            .collect();
        assert_eq!(
            forwards, backwards,
            "a record must not depend on which records were built before it"
        );
    }

    #[test]
    fn different_fields_of_one_record_differ() {
        let seed = ValueSeed::new(42, "User", 1);
        let name = generate(&scalar(ScalarKind::String), "name", seed);
        let bio = generate(&scalar(ScalarKind::String), "bio", seed);
        assert_ne!(name, bio);
    }

    #[test]
    fn different_seeds_give_different_worlds() {
        let spec = scalar(ScalarKind::String);
        let a = generate(&spec, "name", ValueSeed::new(1, "User", 0));
        let b = generate(&spec, "name", ValueSeed::new(2, "User", 0));
        assert_ne!(a, b);
    }

    #[test]
    fn declared_bounds_are_respected() {
        let spec = ValueSpec::Scalar(Scalar::new(ScalarKind::Int).with_constraints(Constraints {
            min: Some(10.0),
            max: Some(12.0),
            ..Constraints::default()
        }));
        for ordinal in 0..25 {
            let value = generate(&spec, "count", ValueSeed::new(9, "Thing", ordinal));
            let n = value.as_i64().unwrap();
            assert!((10..=12).contains(&n), "{n} outside declared bounds");
        }
    }

    #[test]
    fn string_length_bounds_are_respected() {
        let spec = ValueSpec::Scalar(Scalar::new(ScalarKind::String).with_constraints(
            Constraints {
                min_length: Some(20),
                max_length: Some(24),
                ..Constraints::default()
            },
        ));
        for ordinal in 0..25 {
            let value = generate(&spec, "title", ValueSeed::new(9, "Thing", ordinal));
            let len = value.as_str().unwrap().chars().count();
            assert!((20..=24).contains(&len), "length {len} outside bounds");
        }
    }

    #[test]
    fn an_enum_only_yields_declared_options() {
        let spec = ValueSpec::Enum(vec!["draft".into(), "live".into()]);
        for ordinal in 0..20 {
            let value = generate(&spec, "status", ValueSeed::new(3, "Post", ordinal));
            let s = value.as_str().unwrap();
            assert!(s == "draft" || s == "live", "unexpected enum value {s}");
        }
    }

    #[test]
    fn a_declared_pattern_is_satisfied() {
        let spec = ValueSpec::Scalar(Scalar::new(ScalarKind::String).with_constraints(
            Constraints {
                pattern: Some(LeanString::from("^[A-Z]{3}-[0-9]{4}$")),
                ..Constraints::default()
            },
        ));
        let matcher = regex::Regex::new("^[A-Z]{3}-[0-9]{4}$").unwrap();
        for ordinal in 0..20 {
            let value = generate(&spec, "sku", ValueSeed::new(4, "Thing", ordinal));
            let text = value.as_str().unwrap();
            assert!(
                matcher.is_match(text),
                "{text} does not satisfy the pattern"
            );
        }
    }

    #[test]
    fn a_realistic_value_that_already_satisfies_the_pattern_is_kept() {
        let spec = ValueSpec::Scalar(
            Scalar::new(ScalarKind::String)
                .with_semantic(FieldType::Email)
                .with_constraints(Constraints {
                    pattern: Some(LeanString::from("@")),
                    ..Constraints::default()
                }),
        );
        let value = generate(&spec, "contact", ValueSeed::new(4, "User", 0));
        let text = value.as_str().unwrap();
        assert!(
            text.contains('@') && text.contains('.'),
            "a permissive pattern must not replace a real email, got {text}"
        );
    }

    #[test]
    fn a_sequential_number_advances_with_the_instance() {
        let spec = ValueSpec::Scalar(
            Scalar::new(ScalarKind::Int)
                .with_semantic(FieldType::SequentialNumber { start: 10, step: 5 }),
        );
        let values: Vec<i64> = (0..4)
            .map(|ordinal| {
                generate(&spec, "position", ValueSeed::new(1, "Row", ordinal))
                    .as_i64()
                    .unwrap()
            })
            .collect();
        assert_eq!(
            values,
            [10, 15, 20, 25],
            "sequential means across instances"
        );
    }

    #[test]
    fn a_generated_float_reads_like_a_payload_writes_one() {
        let spec = ValueSpec::Scalar(Scalar::new(ScalarKind::Float));
        for ordinal in 0..20 {
            let value = generate(&spec, "total", ValueSeed::new(6, "Order", ordinal));
            let text = value.to_string();
            let decimals = text.split_once('.').map_or(0, |(_, rest)| rest.len());
            assert!(
                decimals <= 2,
                "{text} carries more precision than an API does"
            );
        }
    }

    #[test]
    fn a_record_s_timestamps_happen_in_the_order_their_names_say() {
        let stamp = |name: &str| {
            FieldDef::new(
                name,
                ValueSpec::Scalar(Scalar::new(ScalarKind::String).with_semantic(
                    FieldType::Timestamp {
                        format: crate::type_detector::TimestampFormat::Rfc3339Utc,
                    },
                )),
                false,
            )
        };
        let fields = vec![
            stamp("created_at"),
            stamp("updated_at"),
            stamp("deleted_at"),
        ];

        for ordinal in 0..40 {
            let record = generate_fields(&fields, "", ValueSeed::new(5, "Doc", ordinal));
            let at = |name: &str| record.get(name).unwrap().as_str().unwrap().to_string();
            assert!(
                at("created_at") <= at("updated_at"),
                "created {} after updated {}",
                at("created_at"),
                at("updated_at")
            );
            assert!(
                at("updated_at") <= at("deleted_at"),
                "updated {} after deleted {}",
                at("updated_at"),
                at("deleted_at")
            );
        }
    }

    #[test]
    fn a_date_is_not_ordered_against_an_instant() {
        let date = FieldDef::new(
            "start_date",
            ValueSpec::Scalar(
                Scalar::new(ScalarKind::String).with_semantic(FieldType::IsoDate {
                    format: crate::type_detector::DateFormat::Iso,
                }),
            ),
            false,
        );
        let instant = FieldDef::new(
            "updated_at",
            ValueSpec::Scalar(Scalar::new(ScalarKind::String).with_semantic(
                FieldType::Timestamp {
                    format: crate::type_detector::TimestampFormat::Rfc3339Utc,
                },
            )),
            false,
        );
        let record = generate_fields(&[date, instant], "", ValueSeed::new(5, "Doc", 0));
        assert!(
            !record
                .get("start_date")
                .unwrap()
                .as_str()
                .unwrap()
                .contains('T')
        );
        assert!(
            record
                .get("updated_at")
                .unwrap()
                .as_str()
                .unwrap()
                .contains('T')
        );
    }

    #[test]
    fn ordering_does_not_disturb_a_lone_timestamp() {
        let only = FieldDef::new(
            "created_at",
            ValueSpec::Scalar(Scalar::new(ScalarKind::String).with_semantic(
                FieldType::Timestamp {
                    format: crate::type_detector::TimestampFormat::Rfc3339Utc,
                },
            )),
            false,
        );
        let with_pass = generate_fields(std::slice::from_ref(&only), "", ValueSeed::new(9, "D", 3));
        let raw = generate(&only.value, "created_at", ValueSeed::new(9, "D", 3));
        assert_eq!(with_pass.get("created_at"), Some(&raw));
    }

    #[test]
    fn semantic_types_beat_the_declared_kind() {
        let spec =
            ValueSpec::Scalar(Scalar::new(ScalarKind::String).with_semantic(FieldType::Email));
        let value = generate(&spec, "contact", ValueSeed::new(5, "User", 0));
        assert!(value.as_str().unwrap().contains('@'));
    }

    fn keyed(scalar: &Scalar, ordinal: u64) -> LeanString {
        derive_key_value(
            11,
            "User",
            None,
            scalar,
            ordinal,
            Arrival::seeded(11, "User", ordinal),
        )
    }

    #[test]
    fn keys_are_unique_and_stable_per_ordinal() {
        let key = Scalar::new(ScalarKind::Id);
        let first: Vec<_> = (0..50).map(|i| keyed(&key, i)).collect();
        let again: Vec<_> = (0..50).map(|i| keyed(&key, i)).collect();
        assert_eq!(first, again);
        let unique: std::collections::BTreeSet<_> = first.iter().collect();
        assert_eq!(unique.len(), first.len(), "derived keys must not collide");
    }

    #[test]
    fn integer_keys_read_like_integers() {
        let key = Scalar::new(ScalarKind::Int);
        for ordinal in 0..50 {
            assert!(
                keyed(&key, ordinal).parse::<i64>().is_ok(),
                "an integer key has to reach the wire as an integer"
            );
        }
    }

    /// A sequential id counts the way creation time does, which it can only do
    /// because the ordinal and the age rise together.
    #[test]
    fn a_sequential_id_counts_the_way_time_does() {
        let key = Scalar::new(ScalarKind::Int);
        let number = |ordinal: u64| keyed(&key, ordinal).parse::<i64>().unwrap();
        for ordinal in 1..50 {
            assert!(number(ordinal) > number(ordinal - 1));
            assert!(
                clock::moment_of(11, "User", ordinal) > clock::moment_of(11, "User", ordinal - 1),
                "a higher id has to be a later record"
            );
        }
    }

    /// A `format: uuid` is the document's own answer. Everything else was a
    /// guess from a field name, and a v4 uuid is the one family carrying
    /// neither a count nor a clock — so sorting a collection by id put it in
    /// an order unrelated to anything that happened.
    #[test]
    fn an_opaque_id_sorts_the_way_its_record_was_created() {
        let key = Scalar::new(ScalarKind::Id);
        let mut held: Vec<(i64, String)> = (0..80)
            .map(|i| (clock::moment_of(11, "User", i), keyed(&key, i).to_string()))
            .collect();
        assert!(held.iter().all(|(_, id)| id.len() == 26), "{held:?}");

        held.sort_by_key(|(moment, _)| *moment);
        let by_time: Vec<&String> = held.iter().map(|(_, id)| id).collect();
        let mut by_id = by_time.clone();
        by_id.sort();
        assert_eq!(by_id, by_time, "sorting by id has to sort by time");
    }

    #[test]
    fn a_numeric_string_id_still_reads_as_a_numeric_string() {
        let key = Scalar::new(ScalarKind::String).with_semantic(FieldType::NumericStringId);
        let held = keyed(&key, 0);
        assert!(held.parse::<u64>().is_ok(), "{held}");
    }

    #[test]
    fn a_declared_uuid_stays_a_uuid() {
        let mut key = Scalar::new(ScalarKind::String);
        key.constraints.format = Some("uuid".into());
        let held = keyed(&key, 0);
        assert_eq!(held.len(), 36, "{held}");
        assert_eq!(held.matches('-').count(), 4, "{held}");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod shape_tests {
    use super::*;
    use crate::core::world::model::TextShape;

    #[test]
    fn a_word_shaped_field_is_a_word_not_a_sentence() {
        let spec = ValueSpec::Scalar(Scalar::new(ScalarKind::String).with_shape(TextShape::Word));
        let value = generate(&spec, "collectionType", ValueSeed::new(1, "Collection", 0));
        let text = value.as_str().unwrap();
        assert!(
            !text.contains(' '),
            "a `*Type` field should read as a token, got {text:?}"
        );
        assert_eq!(text, text.to_lowercase());
    }

    #[test]
    fn a_slug_shaped_field_has_no_spaces() {
        let spec = ValueSpec::Scalar(Scalar::new(ScalarKind::String).with_shape(TextShape::Slug));
        let value = generate(&spec, "slug", ValueSeed::new(1, "Post", 0));
        assert!(!value.as_str().unwrap().contains(' '));
    }

    #[test]
    fn prose_is_still_prose() {
        let spec = ValueSpec::Scalar(Scalar::new(ScalarKind::String));
        let value = generate(&spec, "title", ValueSeed::new(1, "Post", 0));
        assert!(value.as_str().unwrap().contains(' '));
    }
}
