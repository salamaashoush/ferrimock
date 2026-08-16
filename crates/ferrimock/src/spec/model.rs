//! The entity graph: what a spec means, with no trace of how it was written.
//!
//! Both front ends compile to this, and the store answers queries against it.
//! Nothing here knows about paths, status codes, selection sets or wire
//! formats — that is what keeps one model serving two protocols instead of
//! becoming the union of both.

use lean_string::LeanString;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use std::fmt;

use crate::type_detector::FieldType;

/// A complete set of entity types and the relations between them.
#[derive(Debug, Clone, Default)]
pub struct EntityGraph {
    entities: Vec<EntityType>,
    by_name: FxHashMap<LeanString, usize>,
}

impl EntityGraph {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entity, replacing any existing one with the same name.
    pub fn insert(&mut self, entity: EntityType) {
        if let Some(existing) = self
            .by_name
            .get(&entity.name)
            .copied()
            .and_then(|idx| self.entities.get_mut(idx))
        {
            *existing = entity;
            return;
        }
        self.by_name.insert(entity.name.clone(), self.entities.len());
        self.entities.push(entity);
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&EntityType> {
        self.by_name.get(name).and_then(|&idx| self.entities.get(idx))
    }

    #[must_use]
    pub fn get_mut(&mut self, name: &str) -> Option<&mut EntityType> {
        let idx = *self.by_name.get(name)?;
        self.entities.get_mut(idx)
    }

    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    /// Entities in insertion order, which is the order the front end produced
    /// them — stable across runs, so seeding is reproducible.
    pub fn entities(&self) -> impl Iterator<Item = &EntityType> {
        self.entities.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Entity names ordered so that every relation target precedes the entity
    /// referencing it, which is the order the store seeds in. Cycles are broken
    /// at the first nullable relation encountered; if a cycle has no nullable
    /// edge it is broken arbitrarily and reported.
    #[must_use]
    pub fn seed_order(&self) -> SeedOrder {
        let mut state: FxHashMap<&str, VisitState> = FxHashMap::default();
        let mut order = Vec::with_capacity(self.entities.len());
        let mut broken = Vec::new();

        for entity in &self.entities {
            self.visit(entity, &mut state, &mut order, &mut broken);
        }

        SeedOrder {
            order: order.into_iter().map(LeanString::from).collect(),
            broken_cycles: broken,
        }
    }

    fn visit<'a>(
        &'a self,
        entity: &'a EntityType,
        state: &mut FxHashMap<&'a str, VisitState>,
        order: &mut Vec<&'a str>,
        broken: &mut Vec<BrokenCycle>,
    ) {
        if state.contains_key(entity.name.as_str()) {
            return;
        }
        state.insert(entity.name.as_str(), VisitState::InProgress);

        for field in &entity.fields {
            let Some(relation) = field.relation() else {
                continue;
            };
            let Some(target) = self.get(relation.target.as_str()) else {
                continue;
            };
            if target.name == entity.name {
                continue;
            }
            if state.get(target.name.as_str()) == Some(&VisitState::InProgress) {
                broken.push(BrokenCycle {
                    from: entity.name.clone(),
                    field: field.name.clone(),
                    to: relation.target.clone(),
                    nullable: field.nullable,
                    cardinality: relation.cardinality,
                });
                continue;
            }
            self.visit(target, state, order, broken);
        }

        state.insert(entity.name.as_str(), VisitState::Done);
        order.push(entity.name.as_str());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisitState {
    InProgress,
    Done,
}

/// The order entities are seeded in, plus every relation edge that had to be
/// cut to get there.
#[derive(Debug, Clone, Default)]
pub struct SeedOrder {
    pub order: Vec<LeanString>,
    pub broken_cycles: Vec<BrokenCycle>,
}

/// A relation edge cut to break a cycle.
///
/// Cutting a to-many edge costs nothing — a collection can be empty, and its
/// members are derived from the other side anyway. Cutting a *non-nullable
/// to-one* edge is the case worth surfacing: the spec asked for something no
/// finite world can give.
#[derive(Debug, Clone)]
pub struct BrokenCycle {
    pub from: LeanString,
    pub field: LeanString,
    pub to: LeanString,
    pub nullable: bool,
    pub cardinality: Cardinality,
}

impl BrokenCycle {
    /// Whether this cut leaves a hole the spec said could not exist.
    #[must_use]
    pub fn is_unsatisfiable(&self) -> bool {
        !self.nullable && self.cardinality == Cardinality::One
    }
}

/// One entity type: a name, how instances are addressed, and their fields.
#[derive(Debug, Clone)]
pub struct EntityType {
    pub name: LeanString,
    pub key: CompositeKey,
    pub fields: Vec<FieldDef>,
    /// Concrete `__typename` for entities reached through an interface or
    /// union. Stored rather than derived so abstract-type resolution is stable
    /// across requests.
    pub typename: Option<LeanString>,
    /// How many instances to seed. `None` takes the store's default.
    pub seed_count: Option<usize>,
    pub provenance: Provenance,
}

impl EntityType {
    #[must_use]
    pub fn new(name: impl Into<LeanString>, key: CompositeKey, provenance: Provenance) -> Self {
        Self {
            name: name.into(),
            key,
            fields: Vec::new(),
            typename: None,
            seed_count: None,
            provenance,
        }
    }

    #[must_use]
    pub fn with_field(mut self, field: FieldDef) -> Self {
        self.fields.push(field);
        self
    }

    #[must_use]
    pub fn field(&self, name: &str) -> Option<&FieldDef> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Fields that carry a relation, paired with it.
    pub fn relations(&self) -> impl Iterator<Item = (&FieldDef, &Relation)> {
        self.fields
            .iter()
            .filter_map(|f| f.relation().map(|r| (f, r)))
    }

    /// Fields that hold a value rather than a link.
    pub fn value_fields(&self) -> impl Iterator<Item = &FieldDef> {
        self.fields.iter().filter(|f| f.relation().is_none())
    }
}

/// How an instance is addressed. Usually one field, but a spec keyed by
/// `/repos/{owner}/{repo}/issues/{issue_number}` needs all three, and a scalar
/// key cannot express that at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositeKey(SmallVec<[KeyPart; 2]>);

impl CompositeKey {
    #[must_use]
    pub fn single(field: impl Into<LeanString>) -> Self {
        let field = field.into();
        Self(SmallVec::from_elem(
            KeyPart {
                source: KeySource::Field(field.clone()),
                field,
            },
            1,
        ))
    }

    #[must_use]
    pub fn parts(parts: impl IntoIterator<Item = KeyPart>) -> Self {
        Self(parts.into_iter().collect())
    }

    pub fn iter(&self) -> impl Iterator<Item = &KeyPart> {
        self.0.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The field name when the key is a single part.
    #[must_use]
    pub fn as_single(&self) -> Option<&LeanString> {
        match self.0.as_slice() {
            [part] => Some(&part.field),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyPart {
    /// The field on the entity holding this part of the key.
    pub field: LeanString,
    pub source: KeySource,
}

/// Where a key part is read from on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    /// A field of the entity's own payload.
    Field(LeanString),
    /// A path parameter, which for a scoped resource is also the parent link.
    PathParam(LeanString),
}

/// A concrete instance key: the key fields' values, in key order.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityKey(SmallVec<[LeanString; 1]>);

impl EntityKey {
    #[must_use]
    pub fn single(value: impl Into<LeanString>) -> Self {
        Self(SmallVec::from_elem(value.into(), 1))
    }

    #[must_use]
    pub fn from_parts(parts: impl IntoIterator<Item = LeanString>) -> Self {
        Self(parts.into_iter().collect())
    }

    pub fn parts(&self) -> impl Iterator<Item = &LeanString> {
        self.0.iter()
    }

    #[must_use]
    pub fn as_single(&self) -> Option<&LeanString> {
        match self.0.as_slice() {
            [value] => Some(value),
            _ => None,
        }
    }
}

impl fmt::Display for EntityKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, part) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            f.write_str(part)?;
        }
        Ok(())
    }
}

/// A field: a name, what it holds, and whether it may be absent.
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: LeanString,
    pub value: ValueSpec,
    pub nullable: bool,
}

impl FieldDef {
    #[must_use]
    pub fn new(name: impl Into<LeanString>, value: ValueSpec, nullable: bool) -> Self {
        Self {
            name: name.into(),
            value,
            nullable,
        }
    }

    /// The relation this field carries, looking through list wrappers.
    #[must_use]
    pub fn relation(&self) -> Option<&Relation> {
        self.value.relation()
    }
}

/// What a field holds.
///
/// Relations live here rather than in a parallel list on the entity: a field
/// and its relation cannot drift apart if there is only one of them.
#[derive(Debug, Clone)]
pub enum ValueSpec {
    Scalar(Scalar),
    /// One of a fixed set of strings.
    Enum(Vec<LeanString>),
    List(Box<ValueSpec>),
    /// A structured value with no identity of its own — inlined, never stored.
    Embedded(Vec<FieldDef>),
    /// A link to another entity.
    Relation(Box<Relation>),
}

impl ValueSpec {
    #[must_use]
    pub fn relation(&self) -> Option<&Relation> {
        match self {
            ValueSpec::Relation(r) => Some(r),
            ValueSpec::List(inner) => inner.relation(),
            _ => None,
        }
    }

    /// Whether the outermost wrapper is a list.
    #[must_use]
    pub fn is_list(&self) -> bool {
        matches!(self, ValueSpec::List(_))
    }
}

/// A scalar field, with whatever the spec said about its values.
#[derive(Debug, Clone)]
pub struct Scalar {
    pub kind: ScalarKind,
    /// How a string-shaped value should read. A schema cannot say whether a
    /// `String` holds a sentence or a token, but the field's name usually can,
    /// and answering `collectionType` with a lorem sentence is wrong in a way
    /// a client notices.
    pub shape: TextShape,
    /// What the field *means*, from the same detector the recording path uses.
    /// A spec gives this three inputs instead of two: the declared type, the
    /// field name, and any format or example it carries.
    pub semantic: Option<FieldType>,
    pub constraints: Constraints,
}

impl Scalar {
    #[must_use]
    pub fn new(kind: ScalarKind) -> Self {
        Self {
            kind,
            shape: TextShape::Prose,
            semantic: None,
            constraints: Constraints::default(),
        }
    }

    #[must_use]
    pub fn with_shape(mut self, shape: TextShape) -> Self {
        self.shape = shape;
        self
    }

    #[must_use]
    pub fn with_semantic(mut self, semantic: FieldType) -> Self {
        self.semantic = Some(semantic);
        self
    }

    #[must_use]
    pub fn with_constraints(mut self, constraints: Constraints) -> Self {
        self.constraints = constraints;
        self
    }
}

/// How a textual value reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TextShape {
    /// Free text: a title, a description, a name.
    #[default]
    Prose,
    /// A single lowercase word — what a `*Type`/`*State` field holds.
    Word,
    /// A hyphenated identifier — what a `*Slug`/`*Key` field holds.
    Slug,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalarKind {
    String,
    Int,
    Float,
    Boolean,
    Id,
    /// A scalar the spec named but did not define (`DateTime`, `JSON`, …).
    Custom(LeanString),
}

/// Bounds a generated value has to respect.
#[derive(Debug, Clone, Default)]
pub struct Constraints {
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub pattern: Option<LeanString>,
    /// The spec's `format` (`uuid`, `date-time`, `email`, …), kept verbatim so
    /// nonstandard ones can still be mapped by a profile.
    pub format: Option<LeanString>,
}

impl Constraints {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min.is_none()
            && self.max.is_none()
            && self.min_length.is_none()
            && self.max_length.is_none()
            && self.pattern.is_none()
            && self.format.is_none()
    }
}

/// A link from one entity to another.
#[derive(Debug, Clone)]
pub struct Relation {
    /// The declared type of the link, which may be an interface or a union.
    pub target: LeanString,
    /// The concrete entities `target` can actually be, when it is abstract.
    /// Empty when `target` is itself a concrete entity.
    pub members: Vec<LeanString>,
    pub cardinality: Cardinality,
    pub carrier: Carrier,
    pub confidence: Confidence,
    pub provenance: Provenance,
}

impl Relation {
    #[must_use]
    pub fn new(
        target: impl Into<LeanString>,
        cardinality: Cardinality,
        carrier: Carrier,
        confidence: Confidence,
        provenance: Provenance,
    ) -> Self {
        Self {
            target: target.into(),
            members: Vec::new(),
            cardinality,
            carrier,
            confidence,
            provenance,
        }
    }

    /// Link to an interface or union, listing the concrete entities behind it.
    #[must_use]
    pub fn abstract_target(mut self, members: Vec<LeanString>) -> Self {
        self.members = members;
        self
    }

    /// Whether the declared target is an interface or union.
    #[must_use]
    pub fn is_abstract(&self) -> bool {
        !self.members.is_empty()
    }

    /// The concrete entities this link can resolve to.
    #[must_use]
    pub fn concrete_targets(&self) -> &[LeanString] {
        if self.members.is_empty() {
            std::slice::from_ref(&self.target)
        } else {
            &self.members
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cardinality {
    One,
    Many,
}

/// How the link is physically carried on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Carrier {
    /// The related entity is nested in the payload.
    Embedded,
    /// A scalar field on this entity holds the target's key.
    ForeignKey(LeanString),
    /// A sub-path (`/users/{id}/posts`), so the child carries the parent key.
    Subresource(LeanString),
    /// A Relay connection: `edges { node cursor }` plus `pageInfo`.
    Connection(ConnectionShape),
}

/// The type names making up a recognised Relay connection, so the binding can
/// rebuild the exact shape the schema declared instead of a generic one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionShape {
    pub connection_type: LeanString,
    pub edge_type: LeanString,
    pub page_info_type: LeanString,
}

/// How much to trust an inferred fact. Declared facts are certain; structural
/// ones are near-certain; name matches are guesses that must be reviewable.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Confidence(f32);

impl Confidence {
    /// The spec said so outright.
    pub const DECLARED: Self = Self(1.0);
    /// The spec's structure implies it and cannot mean much else.
    pub const STRUCTURAL: Self = Self(0.9);
    /// A heuristic that holds often enough to act on, and must be reported.
    pub const HEURISTIC: Self = Self(0.6);
    /// A guess offered for review, never acted on silently.
    pub const CANDIDATE: Self = Self(0.3);

    #[must_use]
    pub fn new(value: f32) -> Self {
        Self(value.clamp(0.0, 1.0))
    }

    #[must_use]
    pub fn value(self) -> f32 {
        self.0
    }

    #[must_use]
    pub fn is_actionable(self) -> bool {
        self.0 >= Self::HEURISTIC.0
    }
}

/// Which rule produced a fact, and from what. Inference that cannot explain
/// itself is not usable on a real spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub rule: Rule,
    pub detail: LeanString,
}

impl Provenance {
    #[must_use]
    pub fn new(rule: Rule, detail: impl Into<LeanString>) -> Self {
        Self {
            rule,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// A GraphQL schema stated the type and its relations.
    GraphQLSchema,
    /// A GraphQL type whose shape matches the Relay connection spec.
    RelayConnection,
    /// A collection path and an item path returning the same schema.
    CollectionItemPair,
    /// A nested resource path.
    PathNesting,
    /// A `$ref` from one entity schema into another.
    SchemaRef,
    /// A field named like a key of another entity.
    ForeignKeyName,
    /// An OpenAPI `links` object.
    SpecLink,
    /// A vendor extension a profile knows how to read.
    VendorExtension,
    /// Stated by the user, in a spec extension or an overrides file.
    Explicit,
}

impl Rule {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Rule::GraphQLSchema => "graphql-schema",
            Rule::RelayConnection => "relay-connection",
            Rule::CollectionItemPair => "collection-item-pair",
            Rule::PathNesting => "path-nesting",
            Rule::SchemaRef => "schema-ref",
            Rule::ForeignKeyName => "foreign-key-name",
            Rule::SpecLink => "spec-link",
            Rule::VendorExtension => "vendor-extension",
            Rule::Explicit => "explicit",
        }
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn entity(name: &str) -> EntityType {
        EntityType::new(
            name,
            CompositeKey::single("id"),
            Provenance::new(Rule::GraphQLSchema, name),
        )
    }

    fn relation_field(name: &str, target: &str, nullable: bool) -> FieldDef {
        FieldDef::new(
            name,
            ValueSpec::Relation(Box::new(Relation::new(
                target,
                Cardinality::One,
                Carrier::Embedded,
                Confidence::STRUCTURAL,
                Provenance::new(Rule::GraphQLSchema, name),
            ))),
            nullable,
        )
    }

    #[test]
    fn seed_order_puts_targets_first() {
        let mut graph = EntityGraph::new();
        graph.insert(entity("Post").with_field(relation_field("author", "User", false)));
        graph.insert(entity("User"));

        let order = graph.seed_order();
        let names: Vec<&str> = order.order.iter().map(LeanString::as_str).collect();
        assert_eq!(names, ["User", "Post"]);
        assert!(order.broken_cycles.is_empty());
    }

    #[test]
    fn seed_order_breaks_cycles_and_reports_them() {
        let mut graph = EntityGraph::new();
        graph.insert(entity("User").with_field(relation_field("posts", "Post", true)));
        graph.insert(entity("Post").with_field(relation_field("author", "User", false)));

        let order = graph.seed_order();
        assert_eq!(order.order.len(), 2);
        assert_eq!(order.broken_cycles.len(), 1);
        let cut = &order.broken_cycles[0];
        assert_eq!(cut.from.as_str(), "Post");
        assert_eq!(cut.to.as_str(), "User");
        assert!(
            cut.is_unsatisfiable(),
            "a cut non-nullable to-one edge is the case worth reporting"
        );
    }

    #[test]
    fn cutting_a_to_many_edge_is_not_a_problem() {
        let mut graph = EntityGraph::new();
        let many = FieldDef::new(
            "posts",
            ValueSpec::List(Box::new(ValueSpec::Relation(Box::new(Relation::new(
                "Post",
                Cardinality::Many,
                Carrier::Embedded,
                Confidence::STRUCTURAL,
                Provenance::new(Rule::GraphQLSchema, "posts"),
            ))))),
            false,
        );
        graph.insert(entity("User").with_field(many));
        graph.insert(entity("Post").with_field(relation_field("author", "User", true)));

        let order = graph.seed_order();
        assert_eq!(order.broken_cycles.len(), 1);
        assert!(
            !order.broken_cycles[0].is_unsatisfiable(),
            "a collection can be empty, so cutting it costs nothing"
        );
    }

    #[test]
    fn a_self_relation_is_not_a_cycle() {
        let mut graph = EntityGraph::new();
        graph.insert(entity("Comment").with_field(relation_field("parent", "Comment", true)));
        let order = graph.seed_order();
        assert_eq!(order.order.len(), 1);
        assert!(order.broken_cycles.is_empty());
    }

    #[test]
    fn relations_are_found_through_list_wrappers() {
        let field = FieldDef::new(
            "posts",
            ValueSpec::List(Box::new(ValueSpec::Relation(Box::new(Relation::new(
                "Post",
                Cardinality::Many,
                Carrier::Embedded,
                Confidence::STRUCTURAL,
                Provenance::new(Rule::GraphQLSchema, "posts"),
            ))))),
            false,
        );
        assert_eq!(field.relation().map(|r| r.target.as_str()), Some("Post"));
    }

    #[test]
    fn insert_replaces_rather_than_duplicates() {
        let mut graph = EntityGraph::new();
        graph.insert(entity("User"));
        graph.insert(entity("User").with_field(relation_field("best", "User", true)));
        assert_eq!(graph.len(), 1);
        assert_eq!(graph.get("User").unwrap().fields.len(), 1);
    }

    #[test]
    fn a_missing_relation_target_does_not_break_ordering() {
        let mut graph = EntityGraph::new();
        graph.insert(entity("Post").with_field(relation_field("author", "Ghost", true)));
        let order = graph.seed_order();
        assert_eq!(order.order.len(), 1);
    }
}
