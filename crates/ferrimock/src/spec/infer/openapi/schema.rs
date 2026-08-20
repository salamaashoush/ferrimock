//! A JSON Schema, read as the shape of a value.
//!
//! This is the OpenAPI counterpart of the GraphQL front end's `value_spec`: it
//! turns a declared schema into a [`ValueSpec`] the store can generate from,
//! and recognises the one case that is not a value at all — a `$ref` to a
//! schema that has identity, which is a link.

use lean_string::LeanString;
use rustc_hash::FxHashSet;

use super::document::{SchemaBook, SchemaKind, SchemaNode, SchemaRef};
use crate::core::world::model::{
    Cardinality, Carrier, Confidence, FieldDef, Provenance, Relation, Rule, Scalar, ScalarKind,
    ValueSpec,
};
use crate::profile::ConsolidationProfile;
use crate::spec::infer::descriptions::{DescriptionHint, hint};
use crate::spec::infer::semantics::{semantic_of, text_shape_of};

/// How deep value objects are inlined before the expansion stops.
///
/// Entities are links the store resolves on demand, so depth there is the
/// client's business. This bounds *value* objects, which have no identity to
/// stop at and which real documents nest in cycles.
const MAX_EMBED_DEPTH: usize = 4;

/// Everything reading a schema needs to know about the document around it.
pub struct Lens<'a> {
    pub book: &'a SchemaBook,
    /// Component names that have identity.
    pub entities: &'a FxHashSet<LeanString>,
    pub profile: &'a dyn ConsolidationProfile,
}

impl Lens<'_> {
    #[must_use]
    pub fn is_entity(&self, name: &str) -> bool {
        self.entities.contains(name)
    }

    /// The entity a reference points at, when it points at one.
    #[must_use]
    pub fn entity_of(&self, reference: &SchemaRef) -> Option<LeanString> {
        let name = reference.name()?;
        self.is_entity(name.as_str()).then(|| name.clone())
    }
}

/// What is currently being inlined, so a cycle among value objects ends.
#[derive(Default)]
struct Expansion {
    open: Vec<LeanString>,
}

impl Expansion {
    fn would_cycle(&self, name: &LeanString) -> bool {
        self.open.contains(name)
    }
}

/// The shape of a value declared by `reference`.
#[must_use]
pub fn value_spec_of(
    lens: &Lens<'_>,
    reference: &SchemaRef,
    field_name: &str,
    owner: &str,
) -> ValueSpec {
    value_spec(
        lens,
        reference,
        field_name,
        owner,
        &mut Expansion::default(),
    )
}

/// The fields of an object schema, as an entity's or an embedded value's.
#[must_use]
pub fn fields_of(lens: &Lens<'_>, node: &SchemaNode, owner: &str) -> Vec<FieldDef> {
    object_fields(lens, node, owner, &mut Expansion::default())
}

fn object_fields(
    lens: &Lens<'_>,
    node: &SchemaNode,
    owner: &str,
    expansion: &mut Expansion,
) -> Vec<FieldDef> {
    node.properties
        .iter()
        .map(|property| {
            let value = value_spec(lens, &property.schema, &property.name, owner, expansion);
            // Two facts, not one: `required` says the key is there, the
            // schema's own `nullable` says the value may be null. A property
            // left out of `required` is allowed to be absent — which is not
            // the same as being present and null, and answering a
            // `type: string` with null because it happened to be optional is
            // a schema violation.
            let nullable = lens
                .book
                .resolve(&property.schema)
                .is_some_and(|node| node.nullable);
            let field = FieldDef::new(property.name.as_str(), value, nullable);
            if property.required {
                field
            } else {
                field.optional()
            }
        })
        .collect()
}

fn value_spec(
    lens: &Lens<'_>,
    reference: &SchemaRef,
    field_name: &str,
    owner: &str,
    expansion: &mut Expansion,
) -> ValueSpec {
    if let Some(entity) = lens.entity_of(reference) {
        return ValueSpec::Relation(Box::new(Relation::new(
            entity,
            Cardinality::One,
            Carrier::Embedded,
            Confidence::STRUCTURAL,
            Provenance::new(Rule::SchemaRef, format!("{owner}.{field_name}")),
        )));
    }

    let Some(node) = lens.book.effective(reference) else {
        return ValueSpec::Scalar(Scalar::new(ScalarKind::Custom(LeanString::from("any"))));
    };

    // A polymorphic value whose members all have identity is a link that
    // resolves to one of them per instance, exactly like a GraphQL union.
    if !node.one_of.is_empty() {
        let members: Vec<LeanString> = node
            .one_of
            .iter()
            .filter_map(|m| lens.entity_of(m))
            .collect();
        if members.len() == node.one_of.len() {
            let Some(first) = members.first().cloned() else {
                return ValueSpec::Scalar(Scalar::new(ScalarKind::Custom(LeanString::from("any"))));
            };
            return ValueSpec::Relation(Box::new(
                Relation::new(
                    first,
                    Cardinality::One,
                    Carrier::Embedded,
                    Confidence::STRUCTURAL,
                    Provenance::new(Rule::SchemaRef, format!("{owner}.{field_name}")),
                )
                .abstract_target(members),
            ));
        }
        // Otherwise the first member is a truthful sample of one of the shapes.
        if let Some(member) = node.one_of.first() {
            return value_spec(lens, member, field_name, owner, expansion);
        }
    }

    match node.effective_kind() {
        SchemaKind::Array => {
            let Some(items) = &node.items else {
                return ValueSpec::List(Box::new(ValueSpec::Scalar(Scalar::new(
                    ScalarKind::Custom(LeanString::from("any")),
                ))));
            };
            if let Some(entity) = lens.entity_of(items) {
                return ValueSpec::List(Box::new(ValueSpec::Relation(Box::new(Relation::new(
                    entity,
                    Cardinality::Many,
                    Carrier::Embedded,
                    Confidence::STRUCTURAL,
                    Provenance::new(Rule::SchemaRef, format!("{owner}.{field_name}")),
                )))));
            }
            ValueSpec::List(Box::new(value_spec(
                lens, items, field_name, owner, expansion,
            )))
        }
        SchemaKind::Object => {
            let name = reference
                .name()
                .cloned()
                .unwrap_or_else(|| LeanString::from(format!("{owner}.{field_name}")));
            if expansion.would_cycle(&name) || expansion.open.len() >= MAX_EMBED_DEPTH {
                return ValueSpec::Embedded(Vec::new());
            }
            expansion.open.push(name);
            let fields = object_fields(lens, &node, owner, expansion);
            expansion.open.pop();
            ValueSpec::Embedded(fields)
        }
        kind => scalar_spec(lens, &node, kind, field_name, owner),
    }
}

fn scalar_spec(
    lens: &Lens<'_>,
    node: &SchemaNode,
    kind: SchemaKind,
    field_name: &str,
    owner: &str,
) -> ValueSpec {
    if !node.enum_values.is_empty() {
        return ValueSpec::Enum(node.enum_values.clone());
    }

    // The description is the only place a spec-only pipeline can learn a domain
    // vocabulary, so it is consulted before anything is guessed from the name.
    let mined = node.description.as_deref().and_then(hint);
    match mined {
        Some(DescriptionHint::Constant(value)) => {
            return ValueSpec::Scalar(Scalar::new(ScalarKind::String).with_semantic(
                crate::type_detector::FieldType::Constant(serde_json::Value::String(
                    value.to_string(),
                )),
            ));
        }
        Some(DescriptionHint::OneOf(values)) => return ValueSpec::Enum(values),
        _ => {}
    }

    let mut scalar = Scalar::new(scalar_kind(kind))
        .with_shape(text_shape_of(field_name))
        .with_constraints(node.constraints.clone());

    // Domain knowledge answers ahead of the built-in detector, which is the
    // whole point of a profile: `continuation` is a cursor at one company and
    // an ordinary string everywhere else.
    let declared_type = node.title.as_deref().unwrap_or_else(|| kind_name(kind));
    if let Some((field_type, _)) = lens.profile.classify_field(field_name, &[]) {
        scalar = scalar.with_semantic(field_type);
    } else if let Some(field_type) = semantic_of(
        field_name,
        declared_type,
        node.format.as_deref(),
        owner,
        &node.examples,
    ) {
        scalar = scalar.with_semantic(field_type);
    } else if let Some(DescriptionHint::Semantic(field_type)) = mined {
        scalar = scalar.with_semantic(field_type);
    }

    ValueSpec::Scalar(scalar)
}

fn scalar_kind(kind: SchemaKind) -> ScalarKind {
    match kind {
        SchemaKind::String => ScalarKind::String,
        SchemaKind::Integer => ScalarKind::Int,
        SchemaKind::Number => ScalarKind::Float,
        SchemaKind::Boolean => ScalarKind::Boolean,
        SchemaKind::Array | SchemaKind::Object | SchemaKind::Any => {
            ScalarKind::Custom(LeanString::from("any"))
        }
    }
}

fn kind_name(kind: SchemaKind) -> &'static str {
    match kind {
        SchemaKind::Object => "object",
        SchemaKind::Array => "array",
        SchemaKind::String => "string",
        SchemaKind::Integer => "integer",
        SchemaKind::Number => "number",
        SchemaKind::Boolean => "boolean",
        SchemaKind::Any => "any",
    }
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
    use crate::profile::DefaultProfile;
    use crate::spec::infer::openapi::document::{OperationTable, parse_openapi};

    const DOC: &str = r"
openapi: 3.0.3
info: { title: t }
paths: {}
components:
  schemas:
    Folder:
      type: object
      required: [id]
      properties:
        id: { type: string }
        name: { type: string }
        size: { type: integer, minimum: 0, maximum: 10 }
        created_at: { type: string, format: date-time }
        state: { type: string, enum: [active, trashed] }
        owner: { $ref: '#/components/schemas/User' }
        items:
          type: array
          items: { $ref: '#/components/schemas/File' }
        path:
          type: object
          properties:
            total: { type: integer }
        tags:
          type: array
          items: { type: string }
    User:
      type: object
      properties:
        id: { type: string }
    File:
      type: object
      properties:
        id: { type: string }
    Ring:
      type: object
      properties:
        note: { type: string }
        next: { $ref: '#/components/schemas/Ring' }
";

    fn table() -> OperationTable {
        parse_openapi(DOC).unwrap().0
    }

    fn entities() -> FxHashSet<LeanString> {
        ["Folder", "User", "File"]
            .into_iter()
            .map(LeanString::from)
            .collect()
    }

    fn folder_fields(table: &OperationTable, names: &FxHashSet<LeanString>) -> Vec<FieldDef> {
        let lens = Lens {
            book: &table.schemas,
            entities: names,
            profile: &DefaultProfile,
        };
        let folder = table.schemas.get("Folder").unwrap();
        fields_of(&lens, folder, "Folder")
    }

    fn field<'a>(fields: &'a [FieldDef], name: &str) -> &'a FieldDef {
        fields.iter().find(|f| f.name == name).unwrap()
    }

    #[test]
    fn a_ref_to_an_entity_is_a_relation() {
        let table = table();
        let fields = folder_fields(&table, &entities());
        let owner = field(&fields, "owner").relation().unwrap();
        assert_eq!(owner.target.as_str(), "User");
        assert_eq!(owner.cardinality, Cardinality::One);
        assert_eq!(owner.provenance.rule, Rule::SchemaRef);
    }

    #[test]
    fn an_array_of_refs_is_a_to_many_relation() {
        let table = table();
        let fields = folder_fields(&table, &entities());
        let items = field(&fields, "items");
        assert!(items.value.is_list());
        assert_eq!(items.relation().unwrap().cardinality, Cardinality::Many);
    }

    #[test]
    fn an_object_without_identity_is_inlined() {
        let table = table();
        let fields = folder_fields(&table, &entities());
        assert!(matches!(
            field(&fields, "path").value,
            ValueSpec::Embedded(_)
        ));
    }

    /// `required` and `nullable` are separate facts. A property left out of
    /// `required` may be absent; only a schema that says `nullable` may be
    /// present and null.
    #[test]
    fn required_and_nullable_are_read_apart() {
        let table = table();
        let fields = folder_fields(&table, &entities());

        let id = field(&fields, "id");
        assert!(id.required);
        assert!(!id.nullable);

        let name = field(&fields, "name");
        assert!(!name.required);
        assert!(
            !name.nullable,
            "optional is not the same as nullable, and null is a schema violation here"
        );
        assert!(name.may_be_missing());
    }

    /// A value the document wrote is the only evidence in a spec that is not
    /// an inference: `example: "2024-03-17T09:41:22Z"` on a field called
    /// `stamp` says what it holds, and nothing in the word `stamp` does.
    #[test]
    fn a_declared_example_answers_ahead_of_a_field_name() {
        let node = crate::spec::infer::openapi::document::SchemaNode {
            kind: Some(crate::spec::infer::openapi::document::SchemaKind::String),
            examples: vec![serde_json::json!("2024-03-17T09:41:22Z")],
            ..Default::default()
        };
        let detected = crate::spec::infer::semantics::semantic_of(
            "stamp",
            "String",
            None,
            "Thing",
            &node.examples,
        );
        assert!(
            matches!(
                detected,
                Some(crate::type_detector::FieldType::Timestamp { .. })
            ),
            "{detected:?}"
        );
    }

    /// A declared `format` is the document stating the answer outright, so it
    /// still wins over a value it happened to show.
    #[test]
    fn a_declared_format_still_beats_an_example() {
        let detected = crate::spec::infer::semantics::semantic_of(
            "reference",
            "String",
            Some("uuid"),
            "Thing",
            &[serde_json::json!("2024-03-17T09:41:22Z")],
        );
        assert_eq!(detected, Some(crate::type_detector::FieldType::Uuid));
    }

    #[test]
    fn an_enum_keeps_its_declared_options() {
        let table = table();
        let fields = folder_fields(&table, &entities());
        let ValueSpec::Enum(options) = &field(&fields, "state").value else {
            panic!("state should be an enum")
        };
        assert_eq!(options.len(), 2);
    }

    #[test]
    fn constraints_and_formats_reach_the_scalar() {
        let table = table();
        let fields = folder_fields(&table, &entities());

        let ValueSpec::Scalar(size) = &field(&fields, "size").value else {
            panic!("size should be a scalar")
        };
        assert_eq!(size.kind, ScalarKind::Int);
        assert_eq!(size.constraints.max, Some(10.0));

        let ValueSpec::Scalar(created) = &field(&fields, "created_at").value else {
            panic!("created_at should be a scalar")
        };
        assert!(
            matches!(
                created.semantic,
                Some(crate::type_detector::FieldType::Timestamp { .. })
            ),
            "a declared `date-time` format is the spec stating the answer"
        );
    }

    #[test]
    fn a_scalar_list_stays_a_list_of_scalars() {
        let table = table();
        let fields = folder_fields(&table, &entities());
        let tags = field(&fields, "tags");
        assert!(tags.value.is_list());
        assert!(tags.relation().is_none());
    }

    #[test]
    fn a_ref_to_a_schema_without_identity_is_a_value_not_a_link() {
        let table = table();
        // `User` is not an entity here, so `owner` has to inline rather than
        // become a link to nothing.
        let names: FxHashSet<LeanString> = std::iter::once(LeanString::from("Folder")).collect();
        let fields = folder_fields(&table, &names);
        assert!(matches!(
            field(&fields, "owner").value,
            ValueSpec::Embedded(_)
        ));
    }

    #[test]
    fn a_self_referencing_value_object_stops_expanding() {
        let table = table();
        let names = FxHashSet::default();
        let lens = Lens {
            book: &table.schemas,
            entities: &names,
            profile: &DefaultProfile,
        };
        let value = value_spec_of(
            &lens,
            &SchemaRef::Named(LeanString::from("Ring")),
            "ring",
            "Test",
        );
        let ValueSpec::Embedded(fields) = &value else {
            panic!("ring should be embedded, got {value:?}")
        };
        let next = fields.iter().find(|f| f.name == "next").unwrap();
        assert!(
            matches!(&next.value, ValueSpec::Embedded(inner) if inner.is_empty()),
            "the cycle stops rather than the field vanishing"
        );
        assert!(fields.iter().any(|f| f.name == "note"));
    }

    #[test]
    fn a_profile_types_a_field_ahead_of_the_built_in_detector() {
        struct Continuations;
        impl ConsolidationProfile for Continuations {
            fn name(&self) -> &'static str {
                "continuations"
            }
            fn classify_field(
                &self,
                field: &str,
                _values: &[&serde_json::Value],
            ) -> Option<(crate::type_detector::FieldType, f64)> {
                (field == "name").then_some((crate::type_detector::FieldType::Token, 0.9))
            }
        }

        let table = table();
        let names = entities();
        let lens = Lens {
            book: &table.schemas,
            entities: &names,
            profile: &Continuations,
        };
        let fields = fields_of(&lens, table.schemas.get("Folder").unwrap(), "Folder");
        let ValueSpec::Scalar(name) = &field(&fields, "name").value else {
            panic!("name should be a scalar")
        };
        assert!(matches!(
            name.semantic,
            Some(crate::type_detector::FieldType::Token)
        ));
    }
}
