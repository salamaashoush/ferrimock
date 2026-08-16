//! The seeded world the bindings serve.
//!
//! Three layers, exactly one of them mutable:
//!
//! - **Census** — eager and tiny. Per entity: how many instances exist and what
//!   their keys are. Pagination totals come from here without building a single
//!   record.
//! - **Base** — lazy and pure. A field's value is derived from the seed, the
//!   entity, the instance ordinal and the field path, so it is the same value
//!   however and whenever it is asked for. Relations are derived the same way,
//!   into the target's census range, which is why a foreign key cannot dangle.
//! - **Delta** — the only mutable state. Creations, patches and tombstones.
//!
//! So the determinism claim is precise: the world is deterministic given the
//! seed, and the state is deterministic given the seed plus the sequence of
//! writes.

pub mod values;

use dashmap::DashMap;
use lean_string::LeanString;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::algebra::{Cursor, Mutation, Page, Predicate, PredicateOp, Selection, SortKey};
use super::model::{
    Cardinality, EntityGraph, EntityKey, EntityType, Relation, Scalar, ScalarKind, ValueSpec,
};
use crate::fake_data::rng;
use values::ValueSeed;

/// How many instances an entity gets when neither it nor the caller says.
pub const DEFAULT_SEED_COUNT: usize = 12;

/// One materialised instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub entity: LeanString,
    pub key: EntityKey,
    pub fields: JsonMap<String, JsonValue>,
}

impl Record {
    #[must_use]
    pub fn get(&self, field: &str) -> Option<&JsonValue> {
        self.fields.get(field)
    }
}

/// A slice of a list result, with the total the slice came from.
#[derive(Debug, Clone)]
pub struct PageResult {
    pub records: Vec<Record>,
    pub total: usize,
    pub has_previous: bool,
    pub has_next: bool,
    pub start_cursor: Option<Cursor>,
    pub end_cursor: Option<Cursor>,
}

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub seed: u64,
    /// Instances per entity when the entity does not specify.
    pub default_count: usize,
    /// Per-entity overrides, by entity name.
    pub counts: FxHashMap<LeanString, usize>,
    /// Whether removing a record also removes what points at it. Without it a
    /// delete that would orphan children is refused.
    pub cascade_delete: bool,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            default_count: DEFAULT_SEED_COUNT,
            counts: FxHashMap::default(),
            cascade_delete: true,
        }
    }
}

impl StoreConfig {
    #[must_use]
    pub fn seeded(seed: u64) -> Self {
        Self {
            seed,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_count(mut self, entity: impl Into<LeanString>, count: usize) -> Self {
        self.counts.insert(entity.into(), count);
        self
    }
}

/// What a write did, so a binding can answer with the right status.
#[derive(Debug, Clone)]
pub enum Written {
    Created(Record),
    Updated(Record),
    Removed(EntityKey),
}

/// The keys of one entity, without any of their fields.
#[derive(Debug, Clone, Default)]
struct Census {
    /// Derived keys, indexed by ordinal.
    derived: Vec<EntityKey>,
    ordinal_of: FxHashMap<EntityKey, u64>,
}

/// A change laid over the base world.
#[derive(Debug, Clone)]
enum Delta {
    Created(JsonMap<String, JsonValue>),
    Patched(JsonMap<String, JsonValue>),
    Tombstone,
}

pub struct EntityStore {
    graph: Arc<EntityGraph>,
    config: StoreConfig,
    census: FxHashMap<LeanString, Census>,
    delta: DashMap<(LeanString, EntityKey), Delta>,
    /// Keys created at runtime, in creation order, per entity. Their namespace
    /// is disjoint from the derived one so a creation can never collide with a
    /// key the base layer would hand out.
    created: RwLock<FxHashMap<LeanString, Vec<EntityKey>>>,
    next_created: AtomicU64,
}

impl std::fmt::Debug for EntityStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntityStore")
            .field("entities", &self.graph.len())
            .field("seed", &self.config.seed)
            .finish_non_exhaustive()
    }
}

impl EntityStore {
    /// Build the census for every entity. Records stay unbuilt until read.
    #[must_use]
    pub fn new(graph: Arc<EntityGraph>, config: StoreConfig) -> Self {
        let mut census = FxHashMap::default();
        for name in &graph.seed_order().order {
            let Some(entity) = graph.get(name) else {
                continue;
            };
            let count = config
                .counts
                .get(name)
                .copied()
                .or(entity.seed_count)
                .unwrap_or(config.default_count);
            census.insert(name.clone(), build_census(config.seed, entity, count));
        }

        Self {
            graph,
            config,
            census,
            delta: DashMap::new(),
            created: RwLock::new(FxHashMap::default()),
            next_created: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn graph(&self) -> &EntityGraph {
        &self.graph
    }

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.config.seed
    }

    /// How many instances of an entity are visible: derived, minus tombstones,
    /// plus creations. Arithmetic — nothing is materialised.
    #[must_use]
    pub fn count(&self, entity: &str) -> usize {
        let derived = self.census.get(entity).map_or(0, |c| c.derived.len());
        let created = self
            .created
            .read()
            .get(entity)
            .map_or(0, Vec::len);
        let tombstoned = self
            .delta
            .iter()
            .filter(|e| e.key().0 == entity && matches!(e.value(), Delta::Tombstone))
            .count();
        (derived + created).saturating_sub(tombstoned)
    }

    /// Every visible key of an entity, derived first then created, in a stable
    /// order.
    #[must_use]
    pub fn keys(&self, entity: &str) -> Vec<EntityKey> {
        let mut keys: Vec<EntityKey> = self
            .census
            .get(entity)
            .map(|c| c.derived.clone())
            .unwrap_or_default();
        if let Some(created) = self.created.read().get(entity) {
            keys.extend(created.iter().cloned());
        }
        keys.retain(|key| !self.is_tombstoned(entity, key));
        keys
    }

    /// Read one instance. `None` means it never existed or was removed.
    #[must_use]
    pub fn get(&self, entity: &str, key: &EntityKey) -> Option<Record> {
        if self.is_tombstoned(entity, key) {
            return None;
        }
        let entity_type = self.graph.get(entity)?;

        let fields = match self.delta.get(&(entity.into(), key.clone())).as_deref() {
            Some(Delta::Created(values)) => values.clone(),
            Some(Delta::Patched(patch)) => {
                let mut base = self.base_fields(entity_type, key)?;
                for (k, v) in patch {
                    base.insert(k.clone(), v.clone());
                }
                base
            }
            Some(Delta::Tombstone) => return None,
            None => self.base_fields(entity_type, key)?,
        };

        Some(Record {
            entity: entity_type.name.clone(),
            key: key.clone(),
            fields,
        })
    }

    /// Read one instance of whichever of `entities` holds it.
    ///
    /// An interface or union has no storage of its own; the instance lives in
    /// exactly one of the concrete types behind it.
    #[must_use]
    pub fn get_any(&self, entities: &[LeanString], key: &EntityKey) -> Option<Record> {
        entities
            .iter()
            .find_map(|entity| self.get(entity.as_str(), key))
    }

    /// Read a slice across several entities, as one list.
    pub fn list_any(
        &self,
        entities: &[LeanString],
        selection: &Selection,
    ) -> crate::Result<PageResult> {
        if let [only] = entities {
            return self.list(only.as_str(), selection);
        }

        let mut records = Vec::new();
        for entity in entities {
            if !self.graph.contains(entity.as_str()) {
                continue;
            }
            records.extend(
                self.keys(entity.as_str())
                    .into_iter()
                    .filter_map(|key| self.get(entity.as_str(), &key))
                    .filter(|record| selection.filters.iter().all(|p| matches(record, p))),
            );
        }
        // Ordered by key so a page boundary does not depend on which concrete
        // type happened to be walked first.
        sort_records(&mut records, &selection.sort);
        Ok(paginate(&records, &selection.page))
    }

    /// Read a slice of an entity's instances.
    pub fn list(&self, entity: &str, selection: &Selection) -> crate::Result<PageResult> {
        if !self.graph.contains(entity) {
            return Err(crate::mp_err!("Unknown entity `{entity}`"));
        }

        let mut records: Vec<Record> = self
            .keys(entity)
            .into_iter()
            .filter_map(|key| self.get(entity, &key))
            .filter(|record| selection.filters.iter().all(|p| matches(record, p)))
            .collect();

        sort_records(&mut records, &selection.sort);
        Ok(paginate(&records, &selection.page))
    }

    /// Follow a relation from one instance.
    pub fn related(
        &self,
        entity: &str,
        key: &EntityKey,
        field: &str,
        selection: &Selection,
    ) -> crate::Result<PageResult> {
        let entity_type = self
            .graph
            .get(entity)
            .ok_or_else(|| crate::mp_err!("Unknown entity `{entity}`"))?;
        let field_def = entity_type
            .field(field)
            .ok_or_else(|| crate::mp_err!("`{entity}` has no field `{field}`"))?;
        let relation = field_def
            .relation()
            .ok_or_else(|| crate::mp_err!("`{entity}.{field}` is not a relation"))?;

        let mut records = match relation.cardinality {
            Cardinality::One => self
                .relation_target(entity, key, field, relation)
                .into_iter()
                .collect::<Vec<_>>(),
            Cardinality::Many => self.relation_children(entity, key, relation),
        };

        records.retain(|record| selection.filters.iter().all(|p| matches(record, p)));
        sort_records(&mut records, &selection.sort);
        Ok(paginate(&records, &selection.page))
    }

    /// The single instance a to-one relation points at.
    #[must_use]
    pub fn relation_target(
        &self,
        entity: &str,
        key: &EntityKey,
        field: &str,
        relation: &Relation,
    ) -> Option<Record> {
        // A write can retarget a relation, and an explicit foreign key wins
        // over the derived one.
        if let Some(record) = self.get(entity, key)
            && let Some(JsonValue::String(target_key)) = record.fields.get(field)
        {
            let candidate = EntityKey::single(target_key.as_str());
            if let Some(found) = self.get(relation.target.as_str(), &candidate) {
                return Some(found);
            }
        }

        let ordinal = self.ordinal_of(entity, key)?;
        // An abstract target resolves to one of its members, chosen per
        // instance so a polymorphic field is stable but not uniform.
        let target = self.chosen_member(entity, ordinal, field, relation)?;
        let target_keys = self.census.get(target.as_str())?;
        let owner = owner_ordinal(
            self.config.seed,
            entity,
            ordinal,
            target.as_str(),
            field,
            target_keys.derived.len(),
        )?;
        let target_key = target_keys.derived.get(usize::try_from(owner).ok()?)?;
        self.get(target.as_str(), target_key)
    }

    /// Every instance of the target whose owning link resolves back to `key`.
    ///
    /// The inverse of the to-one derivation, so the two sides of one relation
    /// always agree: `user.posts` contains exactly the posts whose `author`
    /// resolves to that user.
    fn relation_children(&self, entity: &str, key: &EntityKey, relation: &Relation) -> Vec<Record> {
        let Some(parent_ordinal) = self.ordinal_of(entity, key) else {
            return Vec::new();
        };
        let Some(parent_count) = self.census.get(entity).map(|c| c.derived.len()) else {
            return Vec::new();
        };

        // An abstract collection draws from every concrete member, so a
        // polymorphic list contains a mixture the way the real one does.
        let mut children = Vec::new();
        for member in relation.concrete_targets() {
            let Some(target) = self.graph.get(member.as_str()) else {
                continue;
            };

            // When both sides are collections there is no owner to derive
            // from — a file belongs to several collections and a collection
            // holds several files. Membership has to be computed the same way
            // from either end, or the two directions disagree.
            if points_back_many(target, entity) {
                children.extend(self.keys(member.as_str()).into_iter().filter_map(
                    |child_key| {
                        let child_ordinal = self.ordinal_of(member.as_str(), &child_key)?;
                        self.shares_membership(
                            entity,
                            parent_ordinal,
                            member.as_str(),
                            child_ordinal,
                            parent_count,
                        )
                        .then(|| self.get(member.as_str(), &child_key))?
                    },
                ));
                continue;
            }

            let role = reciprocal_field(target, entity).unwrap_or_default();
            children.extend(self.keys(member.as_str()).into_iter().filter_map(|child_key| {
                let child_ordinal = self.ordinal_of(member.as_str(), &child_key)?;
                let owner = owner_ordinal(
                    self.config.seed,
                    member.as_str(),
                    child_ordinal,
                    entity,
                    &role,
                    parent_count,
                )?;
                (owner == parent_ordinal).then(|| self.get(member.as_str(), &child_key))?
            }));
        }
        children
    }

    /// Whether two instances are on opposite ends of the same many-to-many
    /// membership.
    ///
    /// Both ends compute it from the same anchored function, so
    /// `collection.items` and `file.collections` cannot contradict each other.
    fn shares_membership(
        &self,
        left: &str,
        left_ordinal: u64,
        right: &str,
        right_ordinal: u64,
        left_count: usize,
    ) -> bool {
        let (anchor, anchor_count, member, member_ordinal) = if left < right {
            (left, left_count, right, right_ordinal)
        } else {
            let right_count = self.census.get(right).map_or(0, |c| c.derived.len());
            (right, right_count, left, left_ordinal)
        };

        let wanted = if left < right {
            left_ordinal
        } else {
            right_ordinal
        };
        membership_of(self.config.seed, anchor, anchor_count, member, member_ordinal)
            .contains(&wanted)
    }

    /// Which concrete entity an abstract link resolves to for one instance.
    fn chosen_member(
        &self,
        entity: &str,
        ordinal: u64,
        field: &str,
        relation: &Relation,
    ) -> Option<LeanString> {
        let members = relation.concrete_targets();
        match members {
            [] => None,
            [only] => Some(only.clone()),
            many => {
                let stream = format!("{entity}.{field}#member");
                let derived = rng::derive_seed(self.config.seed, &stream, ordinal);
                let index = usize::try_from(derived % many.len() as u64).ok()?;
                many.get(index).cloned()
            }
        }
    }

    /// Apply a write.
    pub fn apply(&self, entity: &str, mutation: Mutation) -> crate::Result<Written> {
        let entity_type = self
            .graph
            .get(entity)
            .ok_or_else(|| crate::mp_err!("Unknown entity `{entity}`"))?;

        match mutation {
            Mutation::Insert { values } => self.insert(entity_type, values),
            Mutation::Patch { key, values } => self.patch(entity_type, &key, values, false),
            Mutation::Replace { key, values } => self.patch(entity_type, &key, values, true),
            Mutation::Remove { key } => self.remove(entity_type, &key),
        }
    }

    fn insert(&self, entity: &EntityType, values: JsonValue) -> crate::Result<Written> {
        let JsonValue::Object(provided) = values else {
            return Err(crate::mp_err!(
                "`{}` can only be created from an object",
                entity.name
            ));
        };

        let ordinal = self.next_created.fetch_add(1, Ordering::Relaxed);
        let key = match key_from_values(entity, &provided) {
            Some(key) => key,
            None => self.created_key(entity, ordinal),
        };

        if self.get(entity.name.as_str(), &key).is_some() {
            return Err(crate::mp_err!(
                "`{}` with key `{key}` already exists",
                entity.name
            ));
        }

        // Fields the caller left out still have to exist: the response is
        // validated against the same schema a real one would be.
        let mut fields = values::generate_fields(
            &entity.fields,
            "",
            ValueSeed::new(self.config.seed, entity.name.as_str(), u64::MAX - ordinal),
        );
        for (name, value) in provided {
            fields.insert(name, value);
        }
        write_key_fields(entity, &key, &mut fields);

        self.delta.insert(
            (entity.name.clone(), key.clone()),
            Delta::Created(fields.clone()),
        );
        self.created
            .write()
            .entry(entity.name.clone())
            .or_default()
            .push(key.clone());

        Ok(Written::Created(Record {
            entity: entity.name.clone(),
            key,
            fields,
        }))
    }

    fn patch(
        &self,
        entity: &EntityType,
        key: &EntityKey,
        values: JsonValue,
        replace: bool,
    ) -> crate::Result<Written> {
        let JsonValue::Object(provided) = values else {
            return Err(crate::mp_err!(
                "`{}` can only be updated from an object",
                entity.name
            ));
        };
        let existing = self
            .get(entity.name.as_str(), key)
            .ok_or_else(|| crate::mp_err!("`{}` with key `{key}` not found", entity.name))?;

        let mut fields = if replace {
            JsonMap::new()
        } else {
            existing.fields
        };
        for (name, value) in provided {
            fields.insert(name, value);
        }
        write_key_fields(entity, key, &mut fields);

        let slot = (entity.name.clone(), key.clone());
        let created = matches!(self.delta.get(&slot).as_deref(), Some(Delta::Created(_)));
        self.delta.insert(
            slot,
            if created {
                Delta::Created(fields.clone())
            } else {
                Delta::Patched(fields.clone())
            },
        );

        Ok(Written::Updated(Record {
            entity: entity.name.clone(),
            key: key.clone(),
            fields,
        }))
    }

    fn remove(&self, entity: &EntityType, key: &EntityKey) -> crate::Result<Written> {
        if self.get(entity.name.as_str(), key).is_none() {
            return Err(crate::mp_err!(
                "`{}` with key `{key}` not found",
                entity.name
            ));
        }

        let dependents = self.dependents_of(entity.name.as_str(), key);
        if !dependents.is_empty() {
            if !self.config.cascade_delete {
                return Err(crate::mp_err!(
                    "`{}` `{key}` still has {} dependent record(s)",
                    entity.name,
                    dependents.len()
                ));
            }
            for (child_entity, child_key) in dependents {
                self.delta
                    .insert((child_entity, child_key), Delta::Tombstone);
            }
        }

        self.delta
            .insert((entity.name.clone(), key.clone()), Delta::Tombstone);
        Ok(Written::Removed(key.clone()))
    }

    /// Records that point at `key` through a to-one relation.
    ///
    /// Costs a scan of each referencing entity's census, which is the honest
    /// price of deriving relations rather than storing an index for them.
    fn dependents_of(&self, entity: &str, key: &EntityKey) -> Vec<(LeanString, EntityKey)> {
        let mut found = Vec::new();
        for candidate in self.graph.entities() {
            for (field, relation) in candidate.relations() {
                if relation.cardinality != Cardinality::One
                    || !relation.concrete_targets().iter().any(|t| t == entity)
                {
                    continue;
                }
                for child_key in self.keys(candidate.name.as_str()) {
                    let points_at = self
                        .relation_target(candidate.name.as_str(), &child_key, &field.name, relation)
                        .is_some_and(|target| &target.key == key);
                    if points_at {
                        found.push((candidate.name.clone(), child_key));
                    }
                }
            }
        }
        found
    }

    /// A key for a record created at runtime.
    ///
    /// Derived the same way the base layer derives keys, but from an ordinal
    /// past the end of the census — so it looks like every other key of that
    /// entity (a uuid stays a uuid, an integer keeps counting) while being
    /// unable to collide with one the base layer would hand out.
    fn created_key(&self, entity: &EntityType, ordinal: u64) -> EntityKey {
        let base = self
            .census
            .get(entity.name.as_str())
            .map_or(0, |census| census.derived.len() as u64);
        EntityKey::single(values::derive_key_value(
            self.config.seed,
            entity.name.as_str(),
            &key_scalar(entity),
            base.saturating_add(ordinal),
        ))
    }

    fn is_tombstoned(&self, entity: &str, key: &EntityKey) -> bool {
        matches!(
            self.delta.get(&(entity.into(), key.clone())).as_deref(),
            Some(Delta::Tombstone)
        )
    }

    fn ordinal_of(&self, entity: &str, key: &EntityKey) -> Option<u64> {
        self.census.get(entity)?.ordinal_of.get(key).copied()
    }

    /// The base layer: derived values plus derived relation keys. Pure, so it
    /// never has to be stored.
    fn base_fields(
        &self,
        entity: &EntityType,
        key: &EntityKey,
    ) -> Option<JsonMap<String, JsonValue>> {
        let ordinal = self.ordinal_of(entity.name.as_str(), key)?;
        let seed = ValueSeed::new(self.config.seed, entity.name.as_str(), ordinal);
        let mut fields = values::generate_fields(&entity.fields, "", seed);

        for (field, relation) in entity.relations() {
            if relation.cardinality != Cardinality::One {
                continue;
            }
            let Some(target) =
                self.chosen_member(entity.name.as_str(), ordinal, &field.name, relation)
            else {
                continue;
            };
            let Some(target_census) = self.census.get(target.as_str()) else {
                continue;
            };
            let value = owner_ordinal(
                self.config.seed,
                entity.name.as_str(),
                ordinal,
                target.as_str(),
                &field.name,
                target_census.derived.len(),
            )
            .and_then(|owner| target_census.derived.get(usize::try_from(owner).ok()?))
            .map_or(JsonValue::Null, |k| JsonValue::String(k.to_string()));
            fields.insert(field.name.to_string(), value);
        }

        write_key_fields(entity, key, &mut fields);
        Some(fields)
    }
}

fn build_census(seed: u64, entity: &EntityType, count: usize) -> Census {
    let key_scalar = key_scalar(entity);
    let mut derived = Vec::with_capacity(count);
    let mut ordinal_of = FxHashMap::default();

    for ordinal in 0..count as u64 {
        let key = EntityKey::single(values::derive_key_value(
            seed,
            entity.name.as_str(),
            &key_scalar,
            ordinal,
        ));
        ordinal_of.insert(key.clone(), ordinal);
        derived.push(key);
    }

    Census {
        derived,
        ordinal_of,
    }
}

/// The scalar describing an entity's key field, so keys look like what the
/// spec said they are (a uuid, an integer, an opaque string).
fn key_scalar(entity: &EntityType) -> Scalar {
    entity
        .key
        .as_single()
        .and_then(|name| entity.field(name))
        .and_then(|field| match &field.value {
            ValueSpec::Scalar(scalar) => Some(scalar.clone()),
            _ => None,
        })
        .unwrap_or_else(|| Scalar::new(ScalarKind::Id))
}

/// Which instance of `parent` the `ordinal`th instance of `child` belongs to.
///
/// A pure function into the parent's census range, which is what makes
/// referential integrity structural: a derived foreign key cannot point
/// outside the set of parents that exist.
fn owner_ordinal(
    seed: u64,
    child: &str,
    child_ordinal: u64,
    parent: &str,
    role: &str,
    parent_count: usize,
) -> Option<u64> {
    if parent_count == 0 {
        return None;
    }
    let stream = format!("{child}->{parent}#{role}");
    let derived = rng::derive_seed(seed, &stream, child_ordinal);
    Some(derived % parent_count as u64)
}

/// The anchor instances one member belongs to.
///
/// Keyed by the lexicographically smaller entity name so the answer does not
/// depend on which side is asking, which is what keeps the two directions of a
/// many-to-many agreeing.
fn membership_of(
    seed: u64,
    anchor: &str,
    anchor_count: usize,
    member: &str,
    member_ordinal: u64,
) -> Vec<u64> {
    if anchor_count == 0 {
        return Vec::new();
    }
    let stream = format!("{member}<->{anchor}");
    let base = rng::derive_seed(seed, &stream, member_ordinal);
    // One or two anchors each: enough that a collection holds several members
    // and a member appears in more than one collection, without every pair
    // being related to every other.
    let degree = 1 + (base % 2);
    (0..degree)
        .map(|i| {
            rng::derive_seed(seed, &stream, member_ordinal.wrapping_add(i * 0x9E37_79B9))
                % anchor_count as u64
        })
        .collect()
}

/// Whether `entity` has a to-many link back to `other`, which is what makes a
/// relation a many-to-many rather than a parent-child.
fn points_back_many(entity: &EntityType, other: &str) -> bool {
    entity.relations().any(|(_, relation)| {
        relation.cardinality == Cardinality::Many
            && relation.concrete_targets().iter().any(|t| t == other)
    })
}

/// The field on `entity` that links back to `target`, naming the relation so
/// both directions derive the same pairing.
fn reciprocal_field(entity: &EntityType, target: &str) -> Option<String> {
    entity
        .relations()
        .find(|(_, relation)| {
            relation.target == target && relation.cardinality == Cardinality::One
        })
        .map(|(field, _)| field.name.to_string())
}

fn key_from_values(entity: &EntityType, values: &JsonMap<String, JsonValue>) -> Option<EntityKey> {
    let mut parts = Vec::with_capacity(entity.key.len());
    for part in entity.key.iter() {
        let value = values.get(part.field.as_str())?;
        parts.push(LeanString::from(json_to_key_part(value)?));
    }
    (!parts.is_empty()).then(|| EntityKey::from_parts(parts))
}

fn json_to_key_part(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(s) => Some(s.clone()),
        JsonValue::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Keep the key fields in the payload agreeing with the key the record is
/// filed under; a record whose `id` disagrees with its own address is the
/// fastest way to lose a client's trust.
fn write_key_fields(entity: &EntityType, key: &EntityKey, fields: &mut JsonMap<String, JsonValue>) {
    for (part, value) in entity.key.iter().zip(key.parts()) {
        fields.insert(
            part.field.to_string(),
            JsonValue::String(value.to_string()),
        );
    }
}

fn matches(record: &Record, predicate: &Predicate) -> bool {
    let Some(actual) = lookup(&record.fields, &predicate.field) else {
        return matches!(predicate.op, PredicateOp::Ne);
    };
    match predicate.op {
        PredicateOp::Eq => actual == &predicate.value,
        PredicateOp::Ne => actual != &predicate.value,
        PredicateOp::In => predicate
            .value
            .as_array()
            .is_some_and(|options| options.contains(actual)),
        PredicateOp::Contains => match (actual, &predicate.value) {
            (JsonValue::String(haystack), JsonValue::String(needle)) => haystack.contains(needle),
            (JsonValue::Array(items), needle) => items.contains(needle),
            _ => false,
        },
        PredicateOp::Gt | PredicateOp::Gte | PredicateOp::Lt | PredicateOp::Lte => {
            match compare(actual, &predicate.value) {
                Some(ordering) => match predicate.op {
                    PredicateOp::Gt => ordering.is_gt(),
                    PredicateOp::Gte => ordering.is_ge(),
                    PredicateOp::Lt => ordering.is_lt(),
                    _ => ordering.is_le(),
                },
                None => false,
            }
        }
    }
}

/// Dotted lookup, so a filter can reach into an embedded value.
fn lookup<'a>(fields: &'a JsonMap<String, JsonValue>, path: &str) -> Option<&'a JsonValue> {
    let mut current = fields.get(path.split('.').next()?)?;
    for segment in path.split('.').skip(1) {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

fn compare(a: &JsonValue, b: &JsonValue) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (JsonValue::Number(x), JsonValue::Number(y)) => x.as_f64()?.partial_cmp(&y.as_f64()?),
        (JsonValue::String(x), JsonValue::String(y)) => Some(x.cmp(y)),
        (JsonValue::Bool(x), JsonValue::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

fn sort_records(records: &mut [Record], keys: &[SortKey]) {
    if keys.is_empty() {
        return;
    }
    records.sort_by(|a, b| {
        for key in keys {
            let left = lookup(&a.fields, &key.field);
            let right = lookup(&b.fields, &key.field);
            let ordering = match (left, right) {
                (Some(l), Some(r)) => compare(l, r).unwrap_or(std::cmp::Ordering::Equal),
                (Some(_), None) => std::cmp::Ordering::Greater,
                (None, Some(_)) => std::cmp::Ordering::Less,
                (None, None) => std::cmp::Ordering::Equal,
            };
            let ordering = if key.descending {
                ordering.reverse()
            } else {
                ordering
            };
            if ordering != std::cmp::Ordering::Equal {
                return ordering;
            }
        }
        // Ties break on the key, so a page boundary never depends on sort
        // stability across two runs.
        a.key.cmp(&b.key)
    });
}

/// Cursors are the record's key: stable for a given world, and meaningful
/// across a re-sort in a way an index would not be.
fn cursor_of(record: &Record) -> Cursor {
    Cursor::new(record.key.to_string())
}

fn paginate(records: &[Record], page: &Page) -> PageResult {
    let total = records.len();
    let (start, end) = match page {
        Page::All => (0, total),
        Page::Offset { skip, take } => {
            let start = (*skip).min(total);
            (start, (start + take).min(total))
        }
        Page::After { cursor, first } => {
            let start = cursor
                .as_ref()
                .and_then(|c| position_of(records, c).map(|i| i + 1))
                .unwrap_or(0)
                .min(total);
            (start, (start + first).min(total))
        }
        Page::Before { cursor, last } => {
            let end = cursor
                .as_ref()
                .and_then(|c| position_of(records, c))
                .unwrap_or(total)
                .min(total);
            (end.saturating_sub(*last), end)
        }
    };

    let slice = records.get(start..end).unwrap_or_default().to_vec();
    PageResult {
        has_previous: start > 0,
        has_next: end < total,
        start_cursor: slice.first().map(cursor_of),
        end_cursor: slice.last().map(cursor_of),
        records: slice,
        total,
    }
}

fn position_of(records: &[Record], cursor: &Cursor) -> Option<usize> {
    records
        .iter()
        .position(|record| record.key.to_string() == cursor.as_str())
}

#[cfg(test)]
mod tests;
