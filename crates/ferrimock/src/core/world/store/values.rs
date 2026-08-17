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
    record
}

fn generate_in_scope(spec: &ValueSpec, path: &str, seed: ValueSeed<'_>, derived: u64) -> JsonValue {
    match spec {
        ValueSpec::Scalar(scalar) => scalar_value(scalar, path),
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
        // A relation is resolved by the store against the entity graph, not
        // invented here; a caller reaching this has asked for the wrong thing.
        ValueSpec::Relation(_) => JsonValue::Null,
    }
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

fn scalar_value(scalar: &Scalar, path: &str) -> JsonValue {
    if let Some(semantic) = &scalar.semantic
        && let Some(value) = semantic_value(semantic, &scalar.constraints)
    {
        return value;
    }
    kind_value(scalar, path)
}

/// A value for a field whose meaning the detector recognised.
///
/// Only the variants a spec can actually produce are handled; the rest fall
/// back to the declared kind, which is always safe because the kind is what
/// the schema will be validated against.
fn semantic_value(field_type: &FieldType, constraints: &Constraints) -> Option<JsonValue> {
    let value = match field_type {
        FieldType::Uuid => JsonValue::String(fake_data::fake_uuid()),
        FieldType::Email => JsonValue::String(fake_data::fake_email()),
        FieldType::Username => JsonValue::String(fake_data::fake_username()),
        FieldType::Name => JsonValue::String(fake_data::fake_name()),
        FieldType::Sentence => JsonValue::String(fake_data::fake_sentence(8)),
        FieldType::Paragraph => JsonValue::String(fake_data::fake_paragraph(3)),
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
            JsonValue::Number((*start).saturating_add(*step).into())
        }
        FieldType::RandomNumber { min, max } => {
            let low = constraints.min.map_or(min.unwrap_or(1), clamp_to_i64);
            let high = constraints.max.map_or(max.unwrap_or(1000), clamp_to_i64);
            JsonValue::Number(int_between(low, high).into())
        }
        FieldType::RandomFloat { min, max } => {
            let low = constraints.min.or(*min).unwrap_or(0.0);
            let high = constraints.max.or(*max).unwrap_or(1000.0);
            json_float(fake_data::fake_price(low, high))
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
            json_float(fake_data::fake_price(low, high))
        }
        ScalarKind::Id => JsonValue::String(fake_data::fake_uuid()),
        ScalarKind::String | ScalarKind::Custom(_) => JsonValue::String(match scalar.shape {
            TextShape::Prose => bounded_string(constraints, path),
            TextShape::Word => fake_data::fake_word().to_lowercase(),
            TextShape::Slug => fake_data::fake_slug(),
        }),
    }
}

/// A string that satisfies whatever length bounds the spec set. Words are the
/// readable default; bounds are met by truncating or padding with more words,
/// because a spec that says `maxLength: 8` means it.
fn bounded_string(constraints: &Constraints, path: &str) -> String {
    let mut text = fake_data::fake_words(3);
    if let Some(min) = constraints.min_length {
        while text.chars().count() < min {
            text.push(' ');
            text.push_str(&fake_data::fake_word());
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
fn json_float(value: f64) -> JsonValue {
    serde_json::Number::from_f64(value).map_or(JsonValue::Null, JsonValue::Number)
}

/// The key value for the `ordinal`th derived instance of an entity.
///
/// Keys must be derivable without materialising the record: the census hands
/// out keys, and only a read that actually happens builds the fields.
pub fn derive_key_value(seed: u64, entity: &str, key_field: &Scalar, ordinal: u64) -> LeanString {
    let derived = rng::derive_seed(seed, &format!("{entity}#key"), ordinal);
    let _scope = rng::scope_seeded(derived);
    match (&key_field.kind, &key_field.semantic) {
        (_, Some(FieldType::Uuid)) | (ScalarKind::Id, None) => fake_data::fake_uuid().into(),
        (ScalarKind::Int, _) | (_, Some(FieldType::SequentialNumber { .. })) => {
            LeanString::from((ordinal + 1).to_string())
        }
        (_, Some(FieldType::NumericStringId)) => fake_data::fake_numeric_id().into(),
        _ => match &key_field.semantic {
            Some(FieldType::Uuid) => fake_data::fake_uuid().into(),
            _ => LeanString::from(format!("{}-{}", slug(entity), ordinal + 1)),
        },
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
            .map(|i| derive_key_value(11, "User", &key, i))
            .collect();
        let again: Vec<_> = (0..50)
            .map(|i| derive_key_value(11, "User", &key, i))
            .collect();
        assert_eq!(first, again);
        let unique: std::collections::BTreeSet<_> = first.iter().collect();
        assert_eq!(unique.len(), first.len(), "derived keys must not collide");
    }

    #[test]
    fn integer_keys_read_like_integers() {
        let key = Scalar::new(ScalarKind::Int);
        assert_eq!(derive_key_value(11, "User", &key, 0).as_str(), "1");
        assert_eq!(derive_key_value(11, "User", &key, 41).as_str(), "42");
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
