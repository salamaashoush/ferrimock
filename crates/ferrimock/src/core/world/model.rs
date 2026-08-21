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
        self.by_name
            .insert(entity.name.clone(), self.entities.len());
        self.entities.push(entity);
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&EntityType> {
        self.by_name
            .get(name)
            .and_then(|&idx| self.entities.get(idx))
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
        let mut order: Vec<&str> = Vec::with_capacity(self.entities.len());
        let mut broken = Vec::new();

        for entity in &self.entities {
            self.visit(entity, &mut state, &mut order, &mut broken);
        }

        SeedOrder {
            order: order.into_iter().map(LeanString::from).collect(),
            broken_cycles: broken,
        }
    }

    /// Depth-first, over an explicit stack.
    ///
    /// The obvious recursion is one frame per link in the longest chain of
    /// entities, and a production schema is exactly where that chain is long —
    /// the same shape that already had to be capped when *value* objects were
    /// inlined. A stack costs nothing here and cannot overflow.
    fn visit<'a>(
        &'a self,
        start: &'a EntityType,
        state: &mut FxHashMap<&'a str, VisitState>,
        order: &mut Vec<&'a str>,
        broken: &mut Vec<BrokenCycle>,
    ) {
        if state.contains_key(start.name.as_str()) {
            return;
        }

        let mut stack: Vec<(&'a EntityType, usize)> = vec![(start, 0)];
        state.insert(start.name.as_str(), VisitState::InProgress);

        while let Some((entity, cursor)) = stack.pop() {
            let Some(field) = entity.fields.get(cursor) else {
                state.insert(entity.name.as_str(), VisitState::Done);
                order.push(entity.name.as_str());
                continue;
            };
            stack.push((entity, cursor + 1));

            let Some(relation) = field.relation() else {
                continue;
            };
            let Some(target) = self.get(relation.target.as_str()) else {
                continue;
            };
            // A self edge is a cycle of length one, and skipping it hid the
            // only cut that matters: `parent: Folder!` asks for a root that has
            // a parent, which no finite world can give. The `InProgress` arm
            // records it and does not descend, so nothing recurses.
            match state.get(target.name.as_str()) {
                Some(VisitState::InProgress) => broken.push(BrokenCycle {
                    from: entity.name.clone(),
                    field: field.name.clone(),
                    to: relation.target.clone(),
                    nullable: field.nullable,
                    cardinality: relation.cardinality,
                }),
                Some(VisitState::Done) => {}
                None => {
                    state.insert(target.name.as_str(), VisitState::InProgress);
                    stack.push((target, 0));
                }
            }
        }
    }
}

/// Whether a later declaration of a field says strictly more than the one held.
fn supersedes(incoming: &FieldDef, held: &FieldDef) -> bool {
    incoming.relation().is_some() && held.relation().is_none()
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

    /// Fold another declaration of the same entity into this one.
    ///
    /// Two schemas describing one `User` describe one `User`: the REST document
    /// knows about `email`, the GraphQL schema knows about `karma`, and a store
    /// serving both has to hold every field or one of the two surfaces starts
    /// answering with payloads its own schema rejects. So fields union.
    ///
    /// The first declaration wins a conflict, because *something* has to and
    /// load order is the only tiebreak available — except where the later one
    /// says strictly more: a link is more than the scalar key it is carried by,
    /// and a declared field is more than one that was only guessed at.
    pub fn absorb(&mut self, other: &Self) {
        for field in &other.fields {
            match self.fields.iter_mut().find(|held| held.name == field.name) {
                Some(held) if supersedes(field, held) => *held = field.clone(),
                Some(_) => {}
                None => self.fields.push(field.clone()),
            }
        }
        if self.typename.is_none() {
            self.typename.clone_from(&other.typename);
        }
        if self.seed_count.is_none() {
            self.seed_count = other.seed_count;
        }
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
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
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
    /// Whether the key is always in the payload.
    ///
    /// Separate from `nullable`, because they are separate facts and a schema
    /// says both. A GraphQL field that was selected is always in the response
    /// and may be null; an OpenAPI property left out of `required` may not be
    /// in the object at all. Folding them together made every optional field
    /// present, and emitting `null` for a merely-optional `type: string` is a
    /// schema violation rather than a realistic value.
    pub required: bool,
    /// Whether the value may be null.
    pub nullable: bool,
}

impl FieldDef {
    #[must_use]
    pub fn new(name: impl Into<LeanString>, value: ValueSpec, nullable: bool) -> Self {
        Self {
            name: name.into(),
            value,
            required: true,
            nullable,
        }
    }

    /// The key may be missing from the payload entirely.
    #[must_use]
    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    /// Whether a payload is allowed not to carry a usable value here.
    #[must_use]
    pub const fn may_be_missing(&self) -> bool {
        self.nullable || !self.required
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
    /// A position in a lifecycle, which is not the same thing as one of a set.
    Lifecycle(Box<Lifecycle>),
    List(Box<ValueSpec>),
    /// A structured value with no identity of its own — inlined, never stored.
    Embedded(Vec<FieldDef>),
    /// A link to another entity.
    Relation(Box<Relation>),
    /// A template rendered per value, in that value's own seeded stream.
    ///
    /// Only a `world.fields` override produces one: it is the escape hatch for
    /// a value no generator names, and it costs a render per value, so only the
    /// fields that ask for it pay.
    Template(LeanString),
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

    pub fn relation_mut(&mut self) -> Option<&mut Relation> {
        match self {
            ValueSpec::Relation(r) => Some(r),
            ValueSpec::List(inner) => inner.relation_mut(),
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

/// A field whose value is a position in a lifecycle.
///
/// A status is not a categorical draw. `shipped` *means* `shipped_at` holds a
/// value and `delivered_at` does not — a logical implication, and no
/// correlation reproduces one: a latent gives a probability where the schema
/// needs a certainty. Ordering is the other half: a delivered order cannot go
/// back to draft, and a service that let it would be broken.
#[derive(Debug, Clone, PartialEq)]
pub struct Lifecycle {
    /// In order. A record moves to a later state, never to an earlier one.
    pub states: Vec<LifecycleState>,
}

impl Lifecycle {
    #[must_use]
    pub fn position_of(&self, state: &str) -> Option<usize> {
        self.states.iter().position(|held| held.name == state)
    }

    #[must_use]
    pub fn state(&self, at: usize) -> Option<&LifecycleState> {
        self.states.get(at)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LifecycleState {
    pub name: LeanString,
    /// How much of the population sits here.
    pub weight: f64,
    /// Fields that hold nothing while a record is in this state.
    pub empty: Vec<LeanString>,
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

    /// Whether a separate scalar field already carries this link's key.
    #[must_use]
    pub fn is_carried(&self) -> bool {
        matches!(self.carrier, Carrier::ForeignKey(_))
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

impl Carrier {
    /// The field holding the target's key, given the field holding the link.
    ///
    /// Usually the same field: a link with no separate carrier writes the key
    /// where the link is. A document that declares both — `user_id` beside an
    /// embedded `customer` — has one link written twice, and naming the carrier
    /// is what keeps the two spellings pointing at the same instance.
    #[must_use]
    pub fn key_field<'a>(&'a self, field: &'a LeanString) -> &'a LeanString {
        match self {
            Carrier::ForeignKey(name) => name,
            _ => field,
        }
    }

    /// Whether the link's own field holds the key rather than an object.
    #[must_use]
    pub fn is_inline_key(&self, field: &LeanString) -> bool {
        matches!(self, Carrier::ForeignKey(name) if name == field)
    }
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

    /// A self relation is a cycle of length one. Skipping it hid the only cut
    /// worth reporting: a hierarchy has roots, and a root that must have a
    /// parent is a hole no finite world can fill.
    #[test]
    fn a_self_relation_is_a_cycle_of_length_one() {
        let mut graph = EntityGraph::new();
        graph.insert(entity("Comment").with_field(relation_field("parent", "Comment", true)));
        let order = graph.seed_order();
        assert_eq!(order.order.len(), 1);
        assert_eq!(order.broken_cycles.len(), 1);
        assert!(
            !order.broken_cycles[0].is_unsatisfiable(),
            "a comment may have no parent, so the root of the thread fills it with null"
        );
    }

    #[test]
    fn a_self_relation_that_cannot_be_null_is_unsatisfiable() {
        let mut graph = EntityGraph::new();
        graph.insert(entity("Folder").with_field(relation_field("parent", "Folder", false)));
        let order = graph.seed_order();
        assert_eq!(order.broken_cycles.len(), 1);
        assert!(
            order.broken_cycles[0].is_unsatisfiable(),
            "every hierarchy has a root, and this one says it does not"
        );
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
