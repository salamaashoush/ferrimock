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
use crate::core::world::store::pattern;
use crate::fake_data::{self, rng};
use crate::type_detector::FieldType;

/// How many elements a generated list holds when nothing constrains it.
const DEFAULT_LIST_LEN: usize = 2;

/// The derivation context for one field of one record.
#[derive(Debug, Clone, Copy)]
pub struct ValueSeed<'a> {
    pub seed: u64,
    pub entity: &'a str,
    pub ordinal: u64,
}

impl<'a> ValueSeed<'a> {
    #[must_use]
    pub fn new(seed: u64, entity: &'a str, ordinal: u64) -> Self {
        Self {
            seed,
            entity,
            ordinal,
        }
    }

    /// The stream for one field path, so two fields of the same record never
    /// draw the same bytes.
    fn stream_for(&self, path: &str) -> u64 {
        let stream = format!("{}#{}", self.entity, path);
        rng::derive_seed(self.seed, &stream, self.ordinal)
    }
}

/// Generate the value for one field.
pub fn generate(spec: &ValueSpec, path: &str, seed: ValueSeed<'_>) -> JsonValue {
    let derived = seed.stream_for(path);
    let _scope = rng::scope_seeded(derived);
    generate_in_scope(spec, path, seed, derived)
}

/// Generate a whole record's value fields.
pub fn generate_fields(
    fields: &[FieldDef],
    prefix: &str,
    seed: ValueSeed<'_>,
) -> JsonMap<String, JsonValue> {
    let mut record = JsonMap::new();
    for field in fields {
        if field.relation().is_some() {
            continue;
        }
        let path = if prefix.is_empty() {
            field.name.to_string()
        } else {
            format!("{prefix}.{}", field.name)
        };
        record.insert(field.name.to_string(), generate(&field.value, &path, seed));
    }
    order_lifecycle(fields, &mut record);
    record
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
        ValueSpec::Scalar(scalar) => scalar_value(scalar, path, seed.ordinal),
        ValueSpec::Enum(options) => options
            .get(pick_index(derived, options.len()))
            .map_or(JsonValue::Null, |v| JsonValue::String(v.to_string())),
        ValueSpec::List(inner) => {
            let len = list_len(inner);
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

fn list_len(inner: &ValueSpec) -> usize {
    match inner {
        // Nested lists stay short: the product grows fast and nothing reads it.
        ValueSpec::List(_) => 1,
        _ => DEFAULT_LIST_LEN,
    }
}

fn pick_index(derived: u64, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    usize::try_from(derived % len as u64).unwrap_or(0)
}

fn scalar_value(scalar: &Scalar, path: &str, ordinal: u64) -> JsonValue {
    let value = scalar
        .semantic
        .as_ref()
        .and_then(|semantic| semantic_value(semantic, &scalar.constraints, ordinal))
        .unwrap_or_else(|| kind_value(scalar, path));

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
    constraints: &Constraints,
    ordinal: u64,
) -> Option<JsonValue> {
    let value = match field_type {
        FieldType::Uuid => JsonValue::String(fake_data::fake_uuid()),
        FieldType::Email => JsonValue::String(fake_data::fake_email()),
        FieldType::Username => JsonValue::String(fake_data::fake_username()),
        FieldType::Name => JsonValue::String(fake_data::fake_name()),
        // Composed rather than lorem: a title is read by a person, and
        // `perferendis non adipisci asperiores` is the thing that makes a
        // mocked screen look mocked.
        FieldType::Sentence => JsonValue::String(fake_data::fake_headline()),
        FieldType::Paragraph => JsonValue::String(fake_data::fake_prose(2)),
        FieldType::Url | FieldType::ImageUrl => JsonValue::String(fake_data::fake_url()),
        FieldType::IpAddress => JsonValue::String(fake_data::fake_ipv4()),
        FieldType::PhoneNumber => JsonValue::String(fake_data::fake_phone()),
        FieldType::FileName => JsonValue::String(fake_data::fake_filename()),
        FieldType::MimeType => JsonValue::String(fake_data::fake_mime_type()),
        FieldType::Token => JsonValue::String(fake_data::fake_token()),
        FieldType::ETag => JsonValue::String(fake_data::fake_etag()),
        FieldType::NumericStringId => JsonValue::String(fake_data::fake_numeric_id()),
        FieldType::ApiEndpoint => JsonValue::String(fake_data::fake_api_endpoint()),
        FieldType::Timestamp { format } => JsonValue::String(fake_data::fake_timestamp_in(*format)),
        FieldType::IsoDate { format } => JsonValue::String(fake_data::fake_date_in(*format)),
        FieldType::Boolean { spelling } => {
            let flag = fake_data::fake_boolean();
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
        FieldType::RandomNumber { min, max } => {
            let low = constraints.min.map_or(min.unwrap_or(1), clamp_to_i64);
            let high = constraints.max.map_or(max.unwrap_or(1000), clamp_to_i64);
            JsonValue::Number(int_between(low, high).into())
        }
        FieldType::RandomFloat { min, max } => {
            let low = constraints.min.or(*min).unwrap_or(0.0);
            let high = constraints.max.or(*max).unwrap_or(1000.0);
            json_float(fake_data::fake_price(low, high), low, high)
        }
        _ => return None,
    };
    Some(value)
}

fn kind_value(scalar: &Scalar, path: &str) -> JsonValue {
    let constraints = &scalar.constraints;
    match &scalar.kind {
        ScalarKind::Boolean => JsonValue::Bool(fake_data::fake_boolean()),
        ScalarKind::Int => {
            let low = constraints.min.map_or(1, clamp_to_i64);
            let high = constraints.max.map_or(1000, clamp_to_i64);
            JsonValue::Number(int_between(low, high).into())
        }
        ScalarKind::Float => {
            let low = constraints.min.unwrap_or(0.0);
            let high = constraints.max.unwrap_or(1000.0);
            json_float(fake_data::fake_price(low, high), low, high)
        }
        ScalarKind::Id => JsonValue::String(fake_data::fake_uuid()),
        ScalarKind::String | ScalarKind::Custom(_) => JsonValue::String(match scalar.shape {
            TextShape::Prose => bounded_string(constraints, path),
            TextShape::Word => fake_data::fake_word().to_lowercase(),
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

fn int_between(low: i64, high: i64) -> i64 {
    use rand::RngExt as _;
    if low >= high {
        return low;
    }
    rng::rng().random_range(low..=high)
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
pub fn derive_key_value(
    seed: u64,
    entity: &str,
    field: Option<&str>,
    key_field: &Scalar,
    ordinal: u64,
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
    match &key_field.kind {
        ScalarKind::Int | ScalarKind::Float => LeanString::from((ordinal + 1).to_string()),
        _ => match &key_field.semantic {
            Some(FieldType::NumericStringId) => fake_data::fake_numeric_id().into(),
            Some(FieldType::SequentialNumber { .. }) => LeanString::from((ordinal + 1).to_string()),
            Some(FieldType::Uuid) => fake_data::fake_uuid().into(),
            _ if key_field.kind == ScalarKind::Id => fake_data::fake_uuid().into(),
            _ => LeanString::from(format!("{}-{}", slug(label), ordinal + 1)),
        },
    }
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

fn slug(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn scalar(kind: ScalarKind) -> ValueSpec {
        ValueSpec::Scalar(Scalar::new(kind))
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

    #[test]
    fn keys_are_unique_and_stable_per_ordinal() {
        let key = Scalar::new(ScalarKind::Id);
        let first: Vec<_> = (0..50)
            .map(|i| derive_key_value(11, "User", None, &key, i))
            .collect();
        let again: Vec<_> = (0..50)
            .map(|i| derive_key_value(11, "User", None, &key, i))
            .collect();
        assert_eq!(first, again);
        let unique: std::collections::BTreeSet<_> = first.iter().collect();
        assert_eq!(unique.len(), first.len(), "derived keys must not collide");
    }

    #[test]
    fn integer_keys_read_like_integers() {
        let key = Scalar::new(ScalarKind::Int);
        assert_eq!(derive_key_value(11, "User", None, &key, 0).as_str(), "1");
        assert_eq!(derive_key_value(11, "User", None, &key, 41).as_str(), "42");
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
