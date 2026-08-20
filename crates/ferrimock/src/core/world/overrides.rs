//! Saying what a field should hold, when the schema does not.
//!
//! A schema types a field as `String` and stops there. Which string is a
//! product decision — `Order.status` is one of three words, `Money` is a
//! bounded float, a badge is whatever that team's convention says. This is
//! where that knowledge goes.
//!
//! Rules are resolved **once, into the entity graph**, not consulted per read.
//! That is what keeps them compatible with the store: values still derive from
//! `(seed, entity, ordinal, field path)`, a record still builds without its
//! neighbours, and a record a client created goes through the same
//! `generate_fields` as a seeded one — so an override applies to both without
//! knowing either exists.
//!
//! Two things a rule may never touch, because the store owns their values: a
//! key field, and a field carrying a relation. Overriding either produces a
//! world whose keys do not address it or whose links resolve to nothing, so
//! both are refused by name rather than half-applied.

use lean_string::LeanString;
use rustc_hash::FxHashMap;
use serde_json::Value as JsonValue;

use super::model::{
    Constraints, EntityGraph, EntityType, Lifecycle, Scalar, ScalarKind, TextShape, ValueSpec,
};
use crate::type_detector::FieldType;

/// What a field should hold instead of what was inferred.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldRule {
    /// A generator named by the caller, resolved to the same `FieldType` the
    /// detector would have produced.
    Semantic(FieldType),
    /// How a string-shaped value reads, when the meaning is right and only the
    /// spelling is wrong.
    Shape(TextShape),
    /// One of a fixed set.
    OneOf(Vec<LeanString>),
    /// A position in a lifecycle, with what each state implies about the rest
    /// of the record.
    Lifecycle(Lifecycle),
    /// Always this.
    Constant(JsonValue),
    /// A number in a range.
    Number {
        float: bool,
        min: Option<f64>,
        max: Option<f64>,
    },
    /// Anything the pattern accepts.
    Pattern(LeanString),
    /// A template rendered per value, in the field's own seeded stream.
    ///
    /// The escape hatch: it costs a render per value, and only fields that ask
    /// for it pay.
    Template(LeanString),
}

/// Which fields a rule applies to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RuleKey {
    /// `User.email` — one field of one entity.
    Field {
        entity: LeanString,
        field: LeanString,
    },
    /// `*.slug` — a field name, whichever entity has it.
    AnyEntity(LeanString),
    /// A declared type: a GraphQL custom scalar (`Money`) or an OpenAPI
    /// `format` (`date-time`). Both name a kind of value rather than a place.
    Declared(LeanString),
}

/// Every rule a collection stated, in the order they are consulted.
#[derive(Debug, Clone, Default)]
pub struct FieldRules {
    rules: FxHashMap<RuleKey, FieldRule>,
}

impl FieldRules {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn insert(&mut self, key: RuleKey, rule: FieldRule) {
        self.rules.insert(key, rule);
    }

    pub fn extend(&mut self, other: &Self) {
        for (key, rule) in &other.rules {
            self.rules.insert(key.clone(), rule.clone());
        }
    }

    /// The rule for one field, most specific first.
    ///
    /// `User.email` beats `*.email` beats whatever `Money` says, because the
    /// more precisely a rule names its target the more likely it is to be the
    /// one that was meant.
    fn resolve(
        &self,
        entity: &str,
        field: &str,
        declared: &[LeanString],
    ) -> Option<(RuleKey, &FieldRule)> {
        let exact = RuleKey::Field {
            entity: LeanString::from(entity),
            field: LeanString::from(field),
        };
        if let Some(rule) = self.rules.get(&exact) {
            return Some((exact, rule));
        }
        let any = RuleKey::AnyEntity(LeanString::from(field));
        if let Some(rule) = self.rules.get(&any) {
            return Some((any, rule));
        }
        declared.iter().find_map(|name| {
            let key = RuleKey::Declared(name.clone());
            self.rules.get(&key).map(|rule| (key, rule))
        })
    }
}

/// A rule that named something the world does not have, or may not change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedRule {
    pub target: String,
    pub reason: &'static str,
}

impl std::fmt::Display for RejectedRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}`: {}", self.target, self.reason)
    }
}

/// Apply every rule to a graph, reporting the ones that could not be.
///
/// Reported rather than silently dropped: a rule that matches nothing is
/// almost always a typo or a field that was renamed out from under it, and
/// finding that out from a payload is finding it out too late.
pub fn apply(graph: &mut EntityGraph, rules: &FieldRules) -> Vec<RejectedRule> {
    if rules.is_empty() {
        return Vec::new();
    }

    let mut matched: FxHashMap<RuleKey, bool> =
        rules.rules.keys().map(|key| (key.clone(), false)).collect();
    let mut rejected = Vec::new();

    let names: Vec<LeanString> = graph.entities().map(|entity| entity.name.clone()).collect();
    for name in names {
        let Some(entity) = graph.get_mut(name.as_str()) else {
            continue;
        };
        apply_to_entity(entity, rules, &mut matched, &mut rejected);
    }

    for (key, hit) in matched {
        if !hit {
            rejected.push(RejectedRule {
                target: describe(&key),
                reason: "nothing in the world has this field",
            });
        }
    }
    rejected.sort_by(|a, b| a.target.cmp(&b.target));
    rejected
}

fn apply_to_entity(
    entity: &mut EntityType,
    rules: &FieldRules,
    matched: &mut FxHashMap<RuleKey, bool>,
    rejected: &mut Vec<RejectedRule>,
) {
    let owner = entity.name.clone();
    let keyed: Vec<LeanString> = entity.key.iter().map(|part| part.field.clone()).collect();
    // A carrier is the field a link's key is written into, which the store
    // writes and nobody else may.
    let carriers: Vec<LeanString> = entity
        .relations()
        .map(|(field, relation)| relation.carrier.key_field(&field.name).clone())
        .collect();

    for field in &mut entity.fields {
        let declared = declared_names(&field.value);
        let Some((key, rule)) = rules.resolve(owner.as_str(), field.name.as_str(), &declared)
        else {
            continue;
        };
        if let Some(hit) = matched.get_mut(&key) {
            *hit = true;
        }

        // A rule that *named* this field and cannot have it is worth saying so.
        // One that swept in by naming a type is not: `scalars: { String: … }`
        // reaches every string in the schema, and the keys among them were
        // never what it was aimed at.
        let aimed = !matches!(key, RuleKey::Declared(_));
        let target = format!("{owner}.{}", field.name);
        if keyed.contains(&field.name) {
            if aimed {
                rejected.push(RejectedRule {
                    target,
                    reason: "a key field is derived by the store so that it addresses its record",
                });
            }
            continue;
        }
        if field.relation().is_some() || carriers.contains(&field.name) {
            if aimed {
                rejected.push(RejectedRule {
                    target,
                    reason:
                        "this field carries a relation, and the store writes the target's key here",
                });
            }
            continue;
        }

        rewrite(&mut field.value, rule);
    }
}

/// The type names a field was declared as, for a rule keyed on a kind of value
/// rather than a place.
///
/// A GraphQL custom scalar arrives as `ScalarKind::Custom("Money")`; an OpenAPI
/// `format` is kept verbatim on the constraints. Both are how a document says
/// "this is a Money" without saying which field.
fn declared_names(value: &ValueSpec) -> Vec<LeanString> {
    let ValueSpec::Scalar(scalar) = value else {
        return Vec::new();
    };
    let mut names = Vec::new();
    if let Some(format) = &scalar.constraints.format {
        names.push(format.clone());
    }
    match &scalar.kind {
        ScalarKind::Custom(name) => names.push(name.clone()),
        // A builtin is nameable too — mapping every `String` in a schema is
        // blunt, but it is what someone writing `scalars: { String: … }` asked
        // for, and a rule naming the field still beats it. Both spellings,
        // because one front end says `String` and the other says `string`.
        ScalarKind::String => {
            names.extend([LeanString::from("String"), LeanString::from("string")]);
        }
        ScalarKind::Int => names.extend([LeanString::from("Int"), LeanString::from("integer")]),
        ScalarKind::Float => names.extend([LeanString::from("Float"), LeanString::from("number")]),
        ScalarKind::Boolean => {
            names.extend([LeanString::from("Boolean"), LeanString::from("boolean")]);
        }
        ScalarKind::Id => names.push(LeanString::from("ID")),
    }
    names
}

/// Rewrite a field's shape, keeping whatever wrapper it already had.
///
/// A list of strings whose element is overridden stays a list — the rule is
/// about the value, not about how many of them there are.
fn rewrite(value: &mut ValueSpec, rule: &FieldRule) {
    if let ValueSpec::List(inner) = value {
        rewrite(inner, rule);
        return;
    }

    let existing = match value {
        ValueSpec::Scalar(scalar) => scalar.clone(),
        _ => Scalar::new(ScalarKind::String),
    };

    *value = match rule {
        FieldRule::Semantic(semantic) => {
            ValueSpec::Scalar(existing.with_semantic(semantic.clone()))
        }
        FieldRule::Shape(shape) => ValueSpec::Scalar(Scalar {
            semantic: None,
            ..existing.with_shape(*shape)
        }),
        FieldRule::OneOf(options) => ValueSpec::Enum(options.clone()),
        FieldRule::Lifecycle(lifecycle) => ValueSpec::Lifecycle(Box::new(lifecycle.clone())),
        FieldRule::Constant(constant) => {
            ValueSpec::Scalar(existing.with_semantic(FieldType::Constant(constant.clone())))
        }
        FieldRule::Number { float, min, max } => {
            let kind = if *float {
                ScalarKind::Float
            } else {
                ScalarKind::Int
            };
            let constraints = Constraints {
                min: *min,
                max: *max,
                ..existing.constraints
            };
            ValueSpec::Scalar(Scalar::new(kind).with_constraints(constraints))
        }
        FieldRule::Pattern(pattern) => {
            let constraints = Constraints {
                pattern: Some(pattern.clone()),
                ..existing.constraints.clone()
            };
            ValueSpec::Scalar(existing.with_constraints(constraints))
        }
        FieldRule::Template(template) => ValueSpec::Template(template.clone()),
    };
}

fn describe(key: &RuleKey) -> String {
    match key {
        RuleKey::Field { entity, field } => format!("{entity}.{field}"),
        RuleKey::AnyEntity(field) => format!("*.{field}"),
        RuleKey::Declared(name) => name.to_string(),
    }
}

/// The generator a name refers to.
///
/// Named for what the value *is*, not for the function that makes it, so the
/// same name keeps working when the generator behind it improves.
#[must_use]
pub fn generator_named(name: &str) -> Option<FieldRule> {
    use crate::type_detector::{DateFormat, TimestampFormat};

    let semantic = |field_type: FieldType| Some(FieldRule::Semantic(field_type));
    match name.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
        "uuid" | "guid" => semantic(FieldType::Uuid),
        "email" => semantic(FieldType::Email),
        "username" | "login" => semantic(FieldType::Username),
        "person_name" | "full_name" => semantic(FieldType::Name),
        "headline" | "title" | "sentence" => semantic(FieldType::Sentence),
        "paragraph" | "prose" | "description" => semantic(FieldType::Paragraph),
        "url" | "uri" => semantic(FieldType::Url),
        "image_url" | "avatar" => semantic(FieldType::ImageUrl),
        "ip" | "ip_address" => semantic(FieldType::IpAddress),
        "phone" => semantic(FieldType::PhoneNumber),
        "filename" | "file_name" => semantic(FieldType::FileName),
        "mime_type" | "content_type" => semantic(FieldType::MimeType),
        "token" | "api_key" => semantic(FieldType::Token),
        "etag" => semantic(FieldType::ETag),
        "numeric_id" => semantic(FieldType::NumericStringId),
        "api_endpoint" => semantic(FieldType::ApiEndpoint),
        "timestamp" | "datetime" | "date_time" => semantic(FieldType::Timestamp {
            format: TimestampFormat::Rfc3339Utc,
        }),
        "date" => semantic(FieldType::IsoDate {
            format: DateFormat::Iso,
        }),
        "boolean" | "bool" => semantic(FieldType::Boolean {
            spelling: crate::type_detector::BooleanSpelling::TrueFalse,
        }),
        "word" | "token_word" => Some(FieldRule::Shape(TextShape::Word)),
        "slug" => Some(FieldRule::Shape(TextShape::Slug)),
        _ => None,
    }
}

/// Every name [`generator_named`] answers to, for an error message that can
/// suggest one.
#[must_use]
pub fn generator_names() -> &'static [&'static str] {
    &[
        "uuid",
        "email",
        "username",
        "person_name",
        "headline",
        "paragraph",
        "url",
        "image_url",
        "ip",
        "phone",
        "filename",
        "mime_type",
        "token",
        "etag",
        "numeric_id",
        "api_endpoint",
        "timestamp",
        "date",
        "boolean",
        "word",
        "slug",
    ]
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::core::world::model::{
        Cardinality, Carrier, CompositeKey, Confidence, FieldDef, Provenance, Relation, Rule,
    };

    fn scalar(name: &str) -> FieldDef {
        FieldDef::new(
            name,
            ValueSpec::Scalar(Scalar::new(ScalarKind::String)),
            true,
        )
    }

    fn entity(name: &str) -> EntityType {
        EntityType::new(
            name,
            CompositeKey::single("id"),
            Provenance::new(Rule::Explicit, "test"),
        )
        .with_field(scalar("id"))
    }

    fn graph_of(entities: Vec<EntityType>) -> EntityGraph {
        let mut graph = EntityGraph::new();
        for entity in entities {
            graph.insert(entity);
        }
        graph
    }

    fn rules(pairs: Vec<(RuleKey, FieldRule)>) -> FieldRules {
        let mut rules = FieldRules::default();
        for (key, rule) in pairs {
            rules.insert(key, rule);
        }
        rules
    }

    fn field_of<'a>(graph: &'a EntityGraph, entity: &str, field: &str) -> &'a ValueSpec {
        &graph.get(entity).unwrap().field(field).unwrap().value
    }

    #[test]
    fn a_rule_names_one_field_of_one_entity() {
        let mut graph = graph_of(vec![
            entity("User").with_field(scalar("email")),
            entity("Order").with_field(scalar("email")),
        ]);
        let rejected = apply(
            &mut graph,
            &rules(vec![(
                RuleKey::Field {
                    entity: "User".into(),
                    field: "email".into(),
                },
                FieldRule::Semantic(FieldType::Email),
            )]),
        );

        assert!(rejected.is_empty(), "{rejected:?}");
        let ValueSpec::Scalar(user) = field_of(&graph, "User", "email") else {
            panic!("expected a scalar")
        };
        assert!(matches!(user.semantic, Some(FieldType::Email)));
        let ValueSpec::Scalar(order) = field_of(&graph, "Order", "email") else {
            panic!("expected a scalar")
        };
        assert!(order.semantic.is_none(), "only the named entity changes");
    }

    #[test]
    fn a_starred_rule_reaches_every_entity() {
        let mut graph = graph_of(vec![
            entity("User").with_field(scalar("slug")),
            entity("Post").with_field(scalar("slug")),
        ]);
        apply(
            &mut graph,
            &rules(vec![(
                RuleKey::AnyEntity("slug".into()),
                FieldRule::Shape(TextShape::Slug),
            )]),
        );

        for name in ["User", "Post"] {
            let ValueSpec::Scalar(scalar) = field_of(&graph, name, "slug") else {
                panic!("expected a scalar")
            };
            assert_eq!(scalar.shape, TextShape::Slug, "{name}");
        }
    }

    #[test]
    fn the_more_specific_rule_wins() {
        let mut graph = graph_of(vec![entity("User").with_field(scalar("slug"))]);
        apply(
            &mut graph,
            &rules(vec![
                (
                    RuleKey::AnyEntity("slug".into()),
                    FieldRule::Shape(TextShape::Word),
                ),
                (
                    RuleKey::Field {
                        entity: "User".into(),
                        field: "slug".into(),
                    },
                    FieldRule::Shape(TextShape::Slug),
                ),
            ]),
        );

        let ValueSpec::Scalar(scalar) = field_of(&graph, "User", "slug") else {
            panic!("expected a scalar")
        };
        assert_eq!(scalar.shape, TextShape::Slug);
    }

    #[test]
    fn a_declared_type_is_matched_by_name_and_by_format() {
        let money = FieldDef::new(
            "total",
            ValueSpec::Scalar(Scalar::new(ScalarKind::Custom("Money".into()))),
            true,
        );
        let stamped = FieldDef::new(
            "seen_at",
            ValueSpec::Scalar(
                Scalar::new(ScalarKind::String).with_constraints(Constraints {
                    format: Some("date-time".into()),
                    ..Constraints::default()
                }),
            ),
            true,
        );
        let mut graph = graph_of(vec![entity("Order").with_field(money).with_field(stamped)]);

        let rejected = apply(
            &mut graph,
            &rules(vec![
                (
                    RuleKey::Declared("Money".into()),
                    FieldRule::Number {
                        float: true,
                        min: Some(1.0),
                        max: Some(99.0),
                    },
                ),
                (
                    RuleKey::Declared("date-time".into()),
                    FieldRule::Semantic(FieldType::Uuid),
                ),
            ]),
        );
        assert!(rejected.is_empty(), "{rejected:?}");

        let ValueSpec::Scalar(total) = field_of(&graph, "Order", "total") else {
            panic!("expected a scalar")
        };
        assert_eq!(total.kind, ScalarKind::Float);
        assert_eq!(total.constraints.max, Some(99.0));

        let ValueSpec::Scalar(seen) = field_of(&graph, "Order", "seen_at") else {
            panic!("expected a scalar")
        };
        assert!(
            matches!(seen.semantic, Some(FieldType::Uuid)),
            "an OpenAPI `format` names a kind of value the same way a scalar does"
        );
    }

    #[test]
    fn a_builtin_scalar_is_nameable_the_way_a_custom_one_is() {
        let mut graph = graph_of(vec![entity("User").with_field(scalar("email"))]);
        let rejected = apply(
            &mut graph,
            &rules(vec![(
                RuleKey::Declared("String".into()),
                FieldRule::Shape(TextShape::Word),
            )]),
        );
        assert!(rejected.is_empty(), "{rejected:?}");

        let ValueSpec::Scalar(email) = field_of(&graph, "User", "email") else {
            panic!("expected a scalar")
        };
        assert_eq!(
            email.shape,
            TextShape::Word,
            "`scalars: {{ String: … }}` is what the old --type-mappings files say"
        );

        // And the key is still a key: a rule that reached it would be refused,
        // but a rule on `String` must not even try — `id` is an `ID`.
        let ValueSpec::Scalar(id) = field_of(&graph, "User", "id") else {
            panic!("expected a scalar")
        };
        assert_eq!(id.shape, TextShape::Prose, "the key was left alone");
    }

    #[test]
    fn a_key_field_is_refused() {
        let mut graph = graph_of(vec![entity("User")]);
        let rejected = apply(
            &mut graph,
            &rules(vec![(
                RuleKey::Field {
                    entity: "User".into(),
                    field: "id".into(),
                },
                FieldRule::Semantic(FieldType::Email),
            )]),
        );
        assert_eq!(rejected.len(), 1);
        assert!(rejected[0].reason.contains("key field"), "{rejected:?}");

        let ValueSpec::Scalar(id) = field_of(&graph, "User", "id") else {
            panic!("expected a scalar")
        };
        assert!(
            id.semantic.is_none(),
            "the key is left as the store made it"
        );
    }

    #[test]
    fn a_relation_carrier_is_refused() {
        let link = FieldDef::new(
            "customer",
            ValueSpec::Relation(Box::new(Relation::new(
                "User",
                Cardinality::One,
                Carrier::ForeignKey("user_id".into()),
                Confidence::STRUCTURAL,
                Provenance::new(Rule::SchemaRef, "Order.customer"),
            ))),
            true,
        );
        let mut graph = graph_of(vec![
            entity("User"),
            entity("Order")
                .with_field(scalar("user_id"))
                .with_field(link),
        ]);

        let rejected = apply(
            &mut graph,
            &rules(vec![(
                RuleKey::Field {
                    entity: "Order".into(),
                    field: "user_id".into(),
                },
                FieldRule::Semantic(FieldType::Uuid),
            )]),
        );
        assert_eq!(rejected.len(), 1, "{rejected:?}");
        assert!(rejected[0].reason.contains("relation"), "{rejected:?}");
    }

    #[test]
    fn a_rule_matching_nothing_is_reported() {
        let mut graph = graph_of(vec![entity("User")]);
        let rejected = apply(
            &mut graph,
            &rules(vec![(
                RuleKey::Field {
                    entity: "User".into(),
                    field: "nickname".into(),
                },
                FieldRule::Semantic(FieldType::Username),
            )]),
        );
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].target, "User.nickname");
        assert!(rejected[0].reason.contains("nothing in the world"));
    }

    #[test]
    fn a_list_keeps_being_a_list() {
        let tags = FieldDef::new(
            "tags",
            ValueSpec::List(Box::new(ValueSpec::Scalar(Scalar::new(ScalarKind::String)))),
            true,
        );
        let mut graph = graph_of(vec![entity("Post").with_field(tags)]);
        apply(
            &mut graph,
            &rules(vec![(
                RuleKey::AnyEntity("tags".into()),
                FieldRule::Shape(TextShape::Slug),
            )]),
        );

        let ValueSpec::List(inner) = field_of(&graph, "Post", "tags") else {
            panic!("a rule about the value must not change how many there are")
        };
        let ValueSpec::Scalar(scalar) = inner.as_ref() else {
            panic!("expected a scalar element")
        };
        assert_eq!(scalar.shape, TextShape::Slug);
    }

    #[test]
    fn a_generator_name_is_case_and_punctuation_insensitive() {
        for spelling in ["image_url", "image-url", "IMAGE URL", "Image_Url"] {
            assert_eq!(
                generator_named(spelling),
                Some(FieldRule::Semantic(FieldType::ImageUrl)),
                "`{spelling}`"
            );
        }
        assert!(generator_named("not_a_generator").is_none());
    }
}
