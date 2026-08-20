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

pub mod pattern;
pub mod values;

use dashmap::DashMap;
use lean_string::LeanString;
use parking_lot::RwLock;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::algebra::{Cursor, Mutation, Page, Predicate, PredicateOp, Selection, SortKey};
use super::model::{
    Cardinality, Carrier, EntityGraph, EntityKey, EntityType, FieldDef, Relation, Scalar,
    ScalarKind, ValueSpec,
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

/// Every write applied to a store, lifted out so a rebuilt store can take them
/// back on. The base layer is pure, so this is the entire mutable state.
#[derive(Debug, Clone, Default)]
pub struct DeltaSnapshot {
    entries: Vec<(LeanString, EntityKey, Delta)>,
    created: FxHashMap<LeanString, Vec<EntityKey>>,
    next_created: u64,
}

impl DeltaSnapshot {
    /// The keys created records hold, which a rebuilt census must not hand out
    /// again. Without this a grown count re-derives a key a live record already
    /// owns, and the entity ends up with two records under one key.
    #[must_use]
    pub fn reserved_keys(&self) -> &FxHashMap<LeanString, Vec<EntityKey>> {
        &self.created
    }

    /// How many writes it carries. Zero means the world is exactly its seed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A write that could not be carried onto a rebuilt store.
///
/// Only ever a patch: a creation carries its own fields and a tombstone on a
/// key that no longer exists has already got what it wanted.
#[derive(Debug, Clone)]
pub struct DeltaConflict {
    pub entity: LeanString,
    pub key: EntityKey,
    pub reason: &'static str,
}

impl std::fmt::Display for DeltaConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` `{}`: {}", self.entity, self.key, self.reason)
    }
}

/// Keys created at runtime.
///
/// Creation order is what gives a created record its ordinal, so the order is
/// kept — but every question asked of it (does this key exist, what ordinal is
/// it) is asked per read and per write, so the position is indexed rather than
/// scanned for. Walking the list instead made creating the *n*th record cost
/// `O(n)`.
#[derive(Debug, Clone, Default)]
struct CreatedKeys {
    order: FxHashMap<LeanString, Vec<EntityKey>>,
    position: FxHashMap<(LeanString, EntityKey), u64>,
}

impl CreatedKeys {
    fn keys_of(&self, entity: &str) -> Option<&Vec<EntityKey>> {
        self.order.get(entity)
    }

    fn count(&self, entity: &str) -> usize {
        self.order.get(entity).map_or(0, Vec::len)
    }

    fn position_of(&self, entity: &str, key: &EntityKey) -> Option<u64> {
        self.position.get(&(entity.into(), key.clone())).copied()
    }

    fn contains(&self, entity: &str, key: &EntityKey) -> bool {
        self.position.contains_key(&(entity.into(), key.clone()))
    }

    /// Append a key, returning the ordinal it takes.
    fn push(&mut self, entity: &LeanString, key: EntityKey) -> u64 {
        let slot = self.order.entry(entity.clone()).or_default();
        let position = slot.len() as u64;
        slot.push(key.clone());
        self.position.insert((entity.clone(), key), position);
        position
    }

    fn clear(&mut self) {
        self.order.clear();
        self.position.clear();
    }

    fn replace(&mut self, order: FxHashMap<LeanString, Vec<EntityKey>>) {
        self.position = order
            .iter()
            .flat_map(|(entity, keys)| {
                keys.iter()
                    .enumerate()
                    .map(move |(at, key)| ((entity.clone(), key.clone()), at as u64))
            })
            .collect();
        self.order = order;
    }
}

/// The keys of one entity, without any of their fields.
#[derive(Debug, Clone, Default)]
struct Census {
    /// Derived keys, in position order.
    derived: Vec<EntityKey>,
    slots: FxHashMap<EntityKey, Slot>,
}

/// Where one derived instance sits.
///
/// The two are usually the same number and are not the same thing: `ordinal` is
/// what its values derive from, `index` is where it sits among its siblings.
/// They diverge when the census had to step over a key a created record owns,
/// and everything that pairs two instances works in `index` — the partition
/// that decides who owns whom is over positions, not over draws.
#[derive(Debug, Clone, Copy)]
struct Slot {
    ordinal: u64,
    index: u32,
}

/// Which parent owns which children, as boundaries over the child positions.
///
/// Hashing each child to a parent independently spreads them evenly, and real
/// data is never even: one folder holds four hundred files and forty hold none.
/// Drawing a weight per parent and cutting the child range in proportion gives
/// that shape — and because the answer is a *range*, reading one parent's
/// children stops being a scan of every child, and counting them stops being a
/// scan at all.
#[derive(Debug)]
struct Partition {
    /// `parents + 1` rising offsets into the child positions.
    boundaries: Vec<u32>,
}

/// How lopsided a relation is. Weights are `u^EXPONENT` over a uniform draw, so
/// most parents get few children and a few get many.
const SKEW_EXPONENT: f64 = 2.5;

/// Where a running share of the weight falls in the child range.
///
/// The clamps are the whole reason this is a function: the value is a
/// proportion of a count, so it is neither negative nor past the end, and
/// saying that once is better than convincing the lint of it at every call.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn scaled_edge(carried: f64, total: f64, children: u32) -> u32 {
    if total <= 0.0 {
        return 0;
    }
    let scaled = (carried / total * f64::from(children)).round();
    if scaled <= 0.0 {
        return 0;
    }
    if scaled >= f64::from(children) {
        return children;
    }
    scaled as u32
}

impl Partition {
    fn of(seed: u64, child: &str, parent: &str, role: &str, children: u32, parents: u32) -> Self {
        if parents == 0 {
            return Self {
                boundaries: vec![0],
            };
        }

        let stream = format!("{child}->{parent}#{role}");
        let mut weights = Vec::with_capacity(parents as usize);
        let mut total = 0.0_f64;
        for parent_index in 0..parents {
            let drawn = rng::derive_seed(seed, &stream, u64::from(parent_index));
            // Uniform in [0, 1), then bent toward zero so the weights are
            // lopsided rather than flat.
            #[allow(clippy::cast_precision_loss)]
            let uniform = (drawn >> 11) as f64 / (1_u64 << 53) as f64;
            // Never exactly zero: a parent with no weight at all would be
            // unreachable, and "some parents have none" should come out of the
            // rounding rather than being baked in.
            let weight = uniform.powf(SKEW_EXPONENT) + f64::EPSILON;
            total += weight;
            weights.push(weight);
        }

        let mut boundaries = Vec::with_capacity(parents as usize + 1);
        boundaries.push(0);
        let mut carried = 0.0_f64;
        for weight in &weights {
            carried += weight;
            // Monotone and never past the end, whatever the arithmetic did.
            let previous = boundaries.last().copied().unwrap_or(0);
            boundaries.push(scaled_edge(carried, total, children).clamp(previous, children));
        }
        // The last edge is the end of the range by definition, not by rounding.
        if let Some(last) = boundaries.last_mut() {
            *last = children;
        }

        Self { boundaries }
    }

    /// Which parent a child position belongs to.
    fn owner_of(&self, child_index: u32) -> Option<u32> {
        let at = self.boundaries.partition_point(|edge| *edge <= child_index);
        // `partition_point` counts edges at or before the child, and the first
        // edge is the start of the first parent's range.
        let parent = u32::try_from(at.checked_sub(1)?).ok()?;
        (usize::try_from(parent).ok()? + 1 < self.boundaries.len()).then_some(parent)
    }

    /// The child positions one parent owns.
    fn range_of(&self, parent_index: u32) -> std::ops::Range<u32> {
        let at = parent_index as usize;
        let start = self.boundaries.get(at).copied().unwrap_or(0);
        let end = self.boundaries.get(at + 1).copied().unwrap_or(start);
        start..end
    }
}

/// How many instances sit at the top of a hierarchy.
///
/// Root-ish of the total: one root is a chain, a root per record is a flat
/// list, and a real tree is neither. Halving the square root leaves room for
/// three or four levels below at any size worth generating.
fn root_count(total: u32) -> u32 {
    (total.isqrt() / 2).clamp(1, total.max(1))
}

/// A relation from an entity to itself, laid out in levels.
///
/// Partitioning a census against itself has a fixed point for every seed and
/// every count. `owner_of` is monotone non-decreasing over a rising boundary
/// vector, so `owner_of(i) - i` starts at or above zero, ends at or below it
/// and moves by at most one per step: a discrete intermediate value theorem
/// guarantees it crosses. A third of a twelve-record hierarchy is its own
/// parent, and no client can walk a breadcrumb chain through that.
///
/// Levels remove it structurally rather than by rejection. Positions are cut
/// into contiguous levels, each level is partitioned across the level above
/// it, and level zero has nothing above it — which is where the world's roots
/// come from. A parent is always at a lower level than its child, so a cycle
/// of any length is impossible. The range property survives: one parent still
/// owns one contiguous run, so reading its children is a range read and
/// counting them is still arithmetic.
#[derive(Debug)]
struct Hierarchy {
    /// Rising offsets: `levels[i]` is where level `i` starts.
    levels: Vec<u32>,
    /// `cuts[i]` divides level `i + 1` across the parents of level `i`.
    cuts: Vec<Partition>,
}

impl Hierarchy {
    fn of(seed: u64, entity: &str, role: &str, total: u32) -> Self {
        let mut levels = vec![0_u32];
        if total == 0 {
            levels.push(0);
            return Self {
                levels,
                cuts: Vec::new(),
            };
        }

        let stream = format!("{entity}^{entity}#{role}#levels");
        let mut placed = root_count(total).min(total);
        let mut width = placed;
        levels.push(placed);
        while placed < total {
            let drawn = rng::derive_seed(seed, &stream, u64::try_from(levels.len()).unwrap_or(0));
            let factor = u32::try_from(drawn % 3).unwrap_or(0).saturating_add(2);
            let next = width
                .saturating_mul(factor)
                .clamp(1, total.saturating_sub(placed));
            placed = placed.saturating_add(next);
            width = next;
            levels.push(placed);
        }

        let mut cuts = Vec::with_capacity(levels.len().saturating_sub(2));
        for depth in 0..levels.len().saturating_sub(2) {
            let parents = span(&levels, depth);
            let children = span(&levels, depth + 1);
            cuts.push(Partition::of(
                seed,
                entity,
                entity,
                &format!("{role}#level{depth}"),
                children,
                parents,
            ));
        }
        Self { levels, cuts }
    }

    /// Which level a position sits in.
    fn depth_of(&self, index: u32) -> Option<usize> {
        let at = self.levels.partition_point(|edge| *edge <= index);
        let depth = at.checked_sub(1)?;
        (depth + 1 < self.levels.len()).then_some(depth)
    }

    fn owner_of(&self, child_index: u32) -> Option<u32> {
        let depth = self.depth_of(child_index)?;
        // Level zero is a root, which is the whole point of having levels.
        let above = depth.checked_sub(1)?;
        let start = *self.levels.get(depth)?;
        let parent_start = *self.levels.get(above)?;
        let local = self
            .cuts
            .get(above)?
            .owner_of(child_index.checked_sub(start)?)?;
        parent_start.checked_add(local)
    }

    fn range_of(&self, parent_index: u32) -> std::ops::Range<u32> {
        let Some(depth) = self.depth_of(parent_index) else {
            return 0..0;
        };
        let (Some(cut), Some(parent_start), Some(child_start)) = (
            self.cuts.get(depth),
            self.levels.get(depth).copied(),
            self.levels.get(depth + 1).copied(),
        ) else {
            return 0..0;
        };
        let range = cut.range_of(parent_index.saturating_sub(parent_start));
        child_start.saturating_add(range.start)..child_start.saturating_add(range.end)
    }
}

/// The width of one level.
fn span(levels: &[u32], depth: usize) -> u32 {
    let start = levels.get(depth).copied().unwrap_or(0);
    let end = levels.get(depth + 1).copied().unwrap_or(start);
    end.saturating_sub(start)
}

/// Who owns whom for one relation.
///
/// Two entities are cut flat: each parent takes one contiguous run of the
/// child census. An entity against itself cannot be, so it is levelled. Both
/// answer the same two questions, which is what lets every reader of a
/// relation stay on one code path.
#[derive(Debug)]
enum Ownership {
    Flat(Partition),
    Levelled(Hierarchy),
}

impl Ownership {
    fn owner_of(&self, child_index: u32) -> Option<u32> {
        match self {
            Self::Flat(partition) => partition.owner_of(child_index),
            Self::Levelled(hierarchy) => hierarchy.owner_of(child_index),
        }
    }

    fn range_of(&self, parent_index: u32) -> std::ops::Range<u32> {
        match self {
            Self::Flat(partition) => partition.range_of(parent_index),
            Self::Levelled(hierarchy) => hierarchy.range_of(parent_index),
        }
    }
}

/// Which instances of two collections hold each other, as positions.
///
/// A relation whose both ends are collections has no owner to derive from, so
/// membership is drawn per member and inverted once. Anchoring on the
/// lexicographically smaller entity name is what makes the two directions read
/// one table rather than two functions that have to be kept agreeing — and the
/// inversion is what keeps the anchor's side off a scan of every member.
#[derive(Debug)]
struct Membership {
    /// Anchor position -> the member positions holding it.
    by_anchor: Vec<Vec<u32>>,
    /// Member position -> the anchor positions it holds.
    by_member: Vec<Vec<u32>>,
}

impl Membership {
    fn of(seed: u64, anchor: &str, anchors: usize, member: &str, ordinals: &[u64]) -> Self {
        let mut by_anchor = vec![Vec::new(); anchors];
        let mut by_member = Vec::with_capacity(ordinals.len());
        for (slot, ordinal) in ordinals.iter().enumerate() {
            let Ok(position) = u32::try_from(slot) else {
                break;
            };
            let mut held: Vec<u32> = membership_of(seed, anchor, anchors, member, *ordinal)
                .into_iter()
                .filter_map(|index| u32::try_from(index).ok())
                .collect();
            // The draw can land twice on the same anchor, and a member belongs
            // to a collection once however many times it was drawn into it.
            held.sort_unstable();
            held.dedup();
            for index in &held {
                if let Some(bucket) = usize::try_from(*index)
                    .ok()
                    .and_then(|at| by_anchor.get_mut(at))
                {
                    bucket.push(position);
                }
            }
            by_member.push(held);
        }
        Self {
            by_anchor,
            by_member,
        }
    }
}

/// Which of two collections anchors their shared membership.
fn membership_sides<'a>(left: &'a str, right: &'a str) -> (&'a str, &'a str) {
    if left <= right {
        (left, right)
    } else {
        (right, left)
    }
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
    created: RwLock<CreatedKeys>,
    next_created: AtomicU64,
    /// How many records of each entity are tombstoned. Counted as they are
    /// written rather than scanned for: `count` is on the request path of every
    /// paginated list, and walking the whole delta to answer it made the cost
    /// of a read grow with the number of writes that had ever happened.
    tombstones: RwLock<FxHashMap<LeanString, usize>>,
    /// One ownership per relation, built on first use. It depends only on the
    /// seed and the two census sizes, all of which are fixed for the life of a
    /// store, so it is computed once and never invalidated.
    ownership: RwLock<FxHashMap<(LeanString, LeanString, LeanString), Arc<Ownership>>>,
    /// One membership table per many-to-many, on the same terms as
    /// `partitions`: built on first use, never invalidated.
    memberships: RwLock<FxHashMap<(LeanString, LeanString), Arc<Membership>>>,
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
        Self::new_reserving(graph, config, &FxHashMap::default())
    }

    /// [`Self::new`], leaving the given keys for records that already hold
    /// them — the ones a previous store's creations own.
    #[must_use]
    pub fn new_reserving(
        graph: Arc<EntityGraph>,
        config: StoreConfig,
        reserved: &FxHashMap<LeanString, Vec<EntityKey>>,
    ) -> Self {
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
            census.insert(
                name.clone(),
                build_census(config.seed, entity, count, reserved.get(name)),
            );
        }

        Self {
            graph,
            config,
            census,
            delta: DashMap::new(),
            created: RwLock::new(CreatedKeys::default()),
            next_created: AtomicU64::new(0),
            tombstones: RwLock::new(FxHashMap::default()),
            ownership: RwLock::new(FxHashMap::default()),
            memberships: RwLock::new(FxHashMap::default()),
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
        let created = self.created.read().count(entity);
        let tombstoned = self.tombstones.read().get(entity).copied().unwrap_or(0);
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
        if let Some(created) = self.created.read().keys_of(entity) {
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

        // A page with nothing to filter or sort by is answerable from the
        // census alone: keys are already in their final order, so only the
        // requested window has to be derived. Without this a `limit: 25` on a
        // large entity builds every record and throws all but 25 away —
        // measurably the difference between microseconds and tens of
        // milliseconds on the request path.
        if selection.filters.is_empty() && selection.sort.is_empty() {
            return Ok(self.page_from_keys(entity, &selection.page));
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

    /// Slice a page out of an entity's keys, deriving only that window.
    ///
    /// Sound only when nothing filters or sorts: cursors are the record's key
    /// and `keys` is already the order `paginate` would have produced, so the
    /// answer is identical to materialising everything first.
    fn page_from_keys(&self, entity: &str, page: &Page) -> PageResult {
        let keys = self.keys(entity);
        let total = keys.len();

        let (start, end) = match page {
            Page::All => (0, total),
            Page::Offset { skip, take } => {
                let start = (*skip).min(total);
                (start, start.saturating_add(*take).min(total))
            }
            Page::After { cursor, first } => {
                let start = cursor
                    .as_ref()
                    .and_then(|c| key_position(&keys, c).map(|i| i + 1))
                    .unwrap_or(0)
                    .min(total);
                (start, start.saturating_add(*first).min(total))
            }
            Page::Before { cursor, last } => {
                let end = cursor
                    .as_ref()
                    .and_then(|c| key_position(&keys, c))
                    .unwrap_or(total)
                    .min(total);
                (end.saturating_sub(*last), end)
            }
        };

        let window = keys.get(start..end).unwrap_or_default();
        let records: Vec<Record> = window
            .iter()
            .filter_map(|key| self.get(entity, key))
            .collect();

        PageResult {
            has_previous: start > 0,
            has_next: end < total,
            start_cursor: window.first().map(|key| Cursor::new(key.to_string())),
            end_cursor: window.last().map(|key| Cursor::new(key.to_string())),
            records,
            total,
        }
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
        let (target, target_key) = self.link_key(entity, key, field, relation)?;
        self.get(target.as_str(), &target_key)
    }

    /// Which instance a to-one link points at, as an entity and a key.
    ///
    /// Answering with the key rather than the record is what lets a delete
    /// check every referencing record without building one: the derived case
    /// is pure arithmetic over the census, and only a record that has actually
    /// been written has to be read.
    fn link_key(
        &self,
        entity: &str,
        key: &EntityKey,
        field: &str,
        relation: &Relation,
    ) -> Option<(LeanString, EntityKey)> {
        // A write can retarget a relation, and an explicit foreign key wins
        // over the derived one. The key may be carried by a sibling field, so
        // it is the carrier that is read rather than the link's own field.
        if self.is_written(entity, key) {
            let carrier = relation.carrier.key_field(&LeanString::from(field)).clone();
            if let Some(record) = self.get(entity, key)
                && let Some(stated) = record
                    .fields
                    .get(carrier.as_str())
                    .and_then(values::key_text)
            {
                let candidate = self.entity_key_of(relation.target.as_str(), &stated);
                if let Some(found) = relation
                    .concrete_targets()
                    .iter()
                    .find(|target| self.get(target.as_str(), &candidate).is_some())
                {
                    return Some((found.clone(), candidate));
                }
            }
        }

        let ordinal = self.ordinal_of(entity, key)?;
        // An abstract target resolves to one of its members, chosen per
        // instance so a polymorphic field is stable but not uniform.
        let target = self.chosen_member(entity, ordinal, field, relation)?;
        let target_key =
            self.owner_key(entity, self.index_of(entity, key)?, field, target.as_str())?;
        Some((target, target_key))
    }

    /// The key an entity is filed under, read from one string.
    ///
    /// A composite key is written the way it prints: its parts separated by
    /// `/`, which is what a path addressing it looks like anyway.
    #[must_use]
    pub fn entity_key_of(&self, entity: &str, key: &str) -> EntityKey {
        let parts = self
            .graph
            .get(entity)
            .map_or(1, |entity| entity.key.len().max(1));
        if parts <= 1 {
            return EntityKey::single(key);
        }
        EntityKey::from_parts(key.splitn(parts, '/').map(LeanString::from))
    }

    /// Every instance of the target whose owning link resolves back to `key`.
    ///
    /// The inverse of the to-one derivation, so the two sides of one relation
    /// always agree: `user.posts` contains exactly the posts whose `author`
    /// resolves to that user.
    fn relation_children(&self, entity: &str, key: &EntityKey, relation: &Relation) -> Vec<Record> {
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
            if is_membership(target, entity) {
                children.extend(
                    self.membership_positions(entity, key, member.as_str())
                        .into_iter()
                        .filter_map(|position| {
                            self.census
                                .get(member.as_str())?
                                .derived
                                .get(usize::try_from(position).ok()?)
                                .cloned()
                        })
                        .filter_map(|child_key| self.get(member.as_str(), &child_key)),
                );
                // Members created at runtime are not in the table. Bounded by
                // the number of creations rather than by the size of the world.
                children.extend(
                    self.created_keys(member.as_str())
                        .into_iter()
                        .filter(|child_key| {
                            self.shares_membership(member.as_str(), child_key, entity, key)
                        })
                        .filter_map(|child_key| self.get(member.as_str(), &child_key)),
                );
                continue;
            }

            let back = reciprocal_link(target, entity);
            let role = back.map_or_else(String::new, |(field, _)| field.name.to_string());
            let carrier =
                back.map(|(field, relation)| relation.carrier.key_field(&field.name).clone());

            // The derived children are a contiguous run of the member's
            // positions, so this reads only that run rather than filtering
            // every child of every parent.
            if let Some(parent_index) = self.index_of(entity, key) {
                let partition = self.ownership(member.as_str(), entity, &role);
                let range = partition.range_of(parent_index);
                children.extend(range.filter_map(|position| {
                    let child_key = self
                        .census
                        .get(member.as_str())?
                        .derived
                        .get(usize::try_from(position).ok()?)?
                        .clone();
                    // A child that has been written answers from its own
                    // fields, and may have been pointed somewhere else since.
                    if self.is_written(member.as_str(), &child_key) {
                        return None;
                    }
                    self.get(member.as_str(), &child_key)
                }));
            }

            // Everything written: records created here, and derived records
            // whose link was changed. Bounded by the number of writes rather
            // than by the size of the world.
            let Some(carrier) = &carrier else {
                continue;
            };
            children.extend(self.written_keys(member.as_str()).into_iter().filter_map(
                |child_key| {
                    let record = self.get(member.as_str(), &child_key)?;
                    let stated = record
                        .fields
                        .get(carrier.as_str())
                        .and_then(values::key_text)?;
                    (self.entity_key_of(entity, &stated) == *key).then_some(record)
                },
            ));
        }
        children
    }

    /// Every key of an entity that carries a write.
    ///
    /// Its size is the number of writes, not the size of the world, which is
    /// what lets a collection reconcile writes without walking every record.
    fn written_keys(&self, entity: &str) -> Vec<EntityKey> {
        self.delta
            .iter()
            .filter(|entry| entry.key().0 == entity)
            .filter(|entry| !matches!(entry.value(), Delta::Tombstone))
            .map(|entry| entry.key().1.clone())
            .collect()
    }

    /// Every key of an entity that a write brought into being.
    ///
    /// Its size is the number of creations, not the size of the world.
    fn created_keys(&self, entity: &str) -> Vec<EntityKey> {
        self.created
            .read()
            .keys_of(entity)
            .cloned()
            .unwrap_or_default()
    }

    /// The positions of `target` sharing a membership with one instance.
    ///
    /// A derived instance reads the inverted table; one created at runtime is
    /// not in it, so its own side is drawn from the function the table was
    /// built from. Both ends of a membership therefore answer from one
    /// derivation, which is what stops `collection.items` and
    /// `doc.collections` contradicting each other.
    fn membership_positions(&self, entity: &str, key: &EntityKey, target: &str) -> Vec<u32> {
        let table = self.membership(entity, target);
        let index = self.index_of(entity, key);

        // A membership an entity has with itself has one side, so it is only
        // symmetric if both are read: two instances are related when either
        // one drew the other.
        if entity == target {
            let Some(index) = index.and_then(|index| usize::try_from(index).ok()) else {
                return Vec::new();
            };
            let mut both = table.by_anchor.get(index).cloned().unwrap_or_default();
            both.extend(table.by_member.get(index).into_iter().flatten().copied());
            both.sort_unstable();
            both.dedup();
            return both;
        }

        let (anchor, _) = membership_sides(entity, target);
        if let Some(index) = index.and_then(|index| usize::try_from(index).ok()) {
            let side = if anchor == entity {
                &table.by_anchor
            } else {
                &table.by_member
            };
            return side.get(index).cloned().unwrap_or_default();
        }

        // Nothing draws an anchor created at runtime: the draw lands inside
        // the anchor's census, which a created key sits past the end of.
        if anchor == entity {
            return Vec::new();
        }
        let Some(ordinal) = self.ordinal_of(entity, key) else {
            return Vec::new();
        };
        let anchors = self.census.get(target).map_or(0, |c| c.derived.len());
        let mut drawn: Vec<u32> = membership_of(self.config.seed, target, anchors, entity, ordinal)
            .into_iter()
            .filter_map(|index| u32::try_from(index).ok())
            .collect();
        drawn.sort_unstable();
        drawn.dedup();
        drawn
    }

    /// Whether two instances are on opposite ends of the same many-to-many
    /// membership.
    fn shares_membership(
        &self,
        left: &str,
        left_key: &EntityKey,
        right: &str,
        right_key: &EntityKey,
    ) -> bool {
        let Some(index) = self.index_of(right, right_key) else {
            return false;
        };
        self.membership_positions(left, left_key, right)
            .contains(&index)
    }

    /// How many instances of `target` share a membership with one instance.
    ///
    /// The same enumeration `relation_children` walks, counted rather than
    /// built, so a count field and the collection it names cannot disagree.
    fn membership_count(&self, entity: &str, key: &EntityKey, target: &str) -> usize {
        let derived = self
            .membership_positions(entity, key, target)
            .into_iter()
            .filter_map(|position| {
                self.census
                    .get(target)?
                    .derived
                    .get(usize::try_from(position).ok()?)
            })
            .filter(|child_key| !self.is_tombstoned(target, child_key))
            .count();
        let created = self
            .created_keys(target)
            .into_iter()
            .filter(|child_key| !self.is_tombstoned(target, child_key))
            .filter(|child_key| self.shares_membership(target, child_key, entity, key))
            .count();
        derived + created
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
        let provided = self.read_links(entity, provided);

        let sequence = self.next_created.fetch_add(1, Ordering::Relaxed);
        let key = match key_from_values(entity, &provided) {
            Some(key) => key,
            None => self.created_key(entity, sequence),
        };

        // Read before locking: `get` reaches the created list itself, so
        // holding the write lock across it deadlocks.
        if self.get(entity.name.as_str(), &key).is_some() {
            return Err(crate::mp_err!(
                "`{}` with key `{key}` already exists",
                entity.name
            ));
        }

        // The ordinal a created record derives from sits past the census, so
        // every ordinal-keyed derivation — its values, its links, which
        // collections it belongs to — works for it exactly as for a seeded one.
        // Claimed under the same lock that publishes the record, so two
        // concurrent creations cannot both take the slot and leave one of them
        // deriving from an ordinal another record already owns.
        let mut created = self.created.write();
        if created.contains(entity.name.as_str(), &key) {
            return Err(crate::mp_err!(
                "`{}` with key `{key}` already exists",
                entity.name
            ));
        }
        let base = self
            .census
            .get(entity.name.as_str())
            .map_or(0, |census| census.derived.len() as u64);
        let ordinal = base.saturating_add(created.push(&entity.name, key.clone()));
        drop(created);

        // Fields the caller left out still have to exist: the response is
        // validated against the same schema a real one would be.
        let mut fields = values::generate_fields(
            &entity.fields,
            "",
            ValueSeed::new(self.config.seed, entity.name.as_str(), ordinal),
        );
        // A created record is not part of the derived partition, so its links
        // are drawn from the child position its ordinal would have had — enough
        // to give it a plausible owner, with anything the caller stated
        // overriding it below.
        let children = self
            .census
            .get(entity.name.as_str())
            .map_or(0, |census| census.derived.len());
        let index = (children > 0).then(|| u32::try_from(ordinal % children as u64).unwrap_or(0));
        self.write_links(entity, ordinal, index, &mut fields);
        self.write_counts(entity, &key, &mut fields);
        for (name, value) in provided {
            fields.insert(name, value);
        }
        write_key_fields(entity, &key, &mut fields);

        self.delta.insert(
            (entity.name.clone(), key.clone()),
            Delta::Created(fields.clone()),
        );

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
        let provided = self.read_links(entity, provided);
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
        if !dependents.is_empty() && !self.config.cascade_delete {
            return Err(crate::mp_err!(
                "`{}` `{key}` still has {} dependent record(s)",
                entity.name,
                dependents.len()
            ));
        }

        // To a fixpoint, not one generation. Over a real hierarchy a single
        // level of cascade tombstones the children and leaves every generation
        // below them pointing at a record that is gone — the dangling key this
        // store exists to make impossible. `seen` is what stops a link a write
        // retargeted from walking in a circle.
        let mut seen: FxHashSet<(LeanString, EntityKey)> = FxHashSet::default();
        seen.insert((entity.name.clone(), key.clone()));
        let mut pending = dependents;
        while let Some((child_entity, child_key)) = pending.pop() {
            if !seen.insert((child_entity.clone(), child_key.clone())) {
                continue;
            }
            pending.extend(self.dependents_of(child_entity.as_str(), &child_key));
            self.tombstone(child_entity, child_key);
        }

        self.tombstone(entity.name.clone(), key.clone());
        Ok(Written::Removed(key.clone()))
    }

    /// Mark a record removed, keeping the per-entity tally `count` reads.
    fn tombstone(&self, entity: LeanString, key: EntityKey) {
        let replaced = self.delta.insert((entity.clone(), key), Delta::Tombstone);
        // Removing something already removed must not count twice.
        if matches!(replaced, Some(Delta::Tombstone)) {
            return;
        }
        *self.tombstones.write().entry(entity).or_default() += 1;
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
                        .link_key(candidate.name.as_str(), &child_key, &field.name, relation)
                        .is_some_and(|(target, target_key)| target == entity && &target_key == key);
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
    fn created_key(&self, entity: &EntityType, sequence: u64) -> EntityKey {
        let census = self.census.get(entity.name.as_str());
        let base = census.map_or(0, |census| census.derived.len() as u64);
        let scalars = key_scalars(entity);
        let created = self.created.read();

        // Probing rather than trusting the arithmetic: `base + sequence` is
        // free the moment it is computed, but a rebuild that grew the entity
        // can have handed the same ordinal to the census since, and two records
        // under one key is worse than a gap in the numbering.
        let mut ordinal = base.saturating_add(sequence);
        loop {
            let key = derive_key(self.config.seed, entity.name.as_str(), &scalars, ordinal);
            let clashes = census.is_some_and(|census| census.slots.contains_key(&key))
                || created.contains(entity.name.as_str(), &key);
            if !clashes {
                return key;
            }
            ordinal = ordinal.saturating_add(1);
        }
    }

    /// Lift every write off the store.
    ///
    /// The base layer is derived from the seed and never stored, so a snapshot
    /// plus the seed is the whole world.
    #[must_use]
    pub fn export_delta(&self) -> DeltaSnapshot {
        let mut entries: Vec<(LeanString, EntityKey, Delta)> = self
            .delta
            .iter()
            .map(|e| {
                let (entity, key) = e.key();
                (entity.clone(), key.clone(), e.value().clone())
            })
            .collect();
        // DashMap iterates arbitrarily; a snapshot that reorders writes would
        // make a rebuild depend on hash order.
        entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

        DeltaSnapshot {
            entries,
            created: self.created.read().order.clone(),
            next_created: self.next_created.load(Ordering::Relaxed),
        }
    }

    /// Put a snapshot's writes back on, reporting the ones that no longer fit.
    ///
    /// A creation carries its own fields, so it survives any rebuild. A patch
    /// is a layer over a derived record, so it survives only while the record
    /// it layers over still exists — shrinking an entity's count can take that
    /// away, and the caller is told rather than left with a silent hole.
    pub fn import_delta(&self, snapshot: DeltaSnapshot) -> Vec<DeltaConflict> {
        let mut conflicts = Vec::new();

        for (entity, key, delta) in snapshot.entries {
            if matches!(delta, Delta::Patched(_))
                && self.ordinal_of(entity.as_str(), &key).is_none()
            {
                conflicts.push(DeltaConflict {
                    entity,
                    key,
                    reason: "the record this patch layered over no longer exists",
                });
                continue;
            }
            if matches!(delta, Delta::Tombstone) {
                self.tombstone(entity, key);
                continue;
            }
            self.delta.insert((entity, key), delta);
        }

        self.created.write().replace(snapshot.created);
        self.next_created
            .store(snapshot.next_created, Ordering::Relaxed);

        conflicts
    }

    /// Drop every write, leaving exactly what the seed derives.
    pub fn reset(&self) {
        self.delta.clear();
        self.created.write().clear();
        self.tombstones.write().clear();
        self.next_created.store(0, Ordering::Relaxed);
    }

    fn is_tombstoned(&self, entity: &str, key: &EntityKey) -> bool {
        matches!(
            self.delta.get(&(entity.into(), key.clone())).as_deref(),
            Some(Delta::Tombstone)
        )
    }

    /// The ordinal an instance derives from: its census ordinal, or — for a
    /// record created at runtime — its position past the end of the census.
    fn ordinal_of(&self, entity: &str, key: &EntityKey) -> Option<u64> {
        if let Some(slot) = self
            .census
            .get(entity)
            .and_then(|census| census.slots.get(key))
        {
            return Some(slot.ordinal);
        }
        let base = self
            .census
            .get(entity)
            .map_or(0, |census| census.derived.len() as u64);
        let position = self.created.read().position_of(entity, key)?;
        Some(base.saturating_add(position))
    }

    /// Where a derived instance sits among its siblings. `None` for a record
    /// created at runtime: those are not part of the derived partition, and
    /// their links are read from what they were written with.
    fn index_of(&self, entity: &str, key: &EntityKey) -> Option<u32> {
        Some(self.census.get(entity)?.slots.get(key)?.index)
    }

    /// Who owns whom for one relation.
    fn ownership(&self, child: &str, parent: &str, role: &str) -> Arc<Ownership> {
        let slot = (
            LeanString::from(child),
            LeanString::from(parent),
            LeanString::from(role),
        );
        if let Some(held) = self.ownership.read().get(&slot) {
            return Arc::clone(held);
        }
        let children = u32::try_from(self.census.get(child).map_or(0, |c| c.derived.len()))
            .unwrap_or(u32::MAX);
        let parents = u32::try_from(self.census.get(parent).map_or(0, |c| c.derived.len()))
            .unwrap_or(u32::MAX);
        let built = Arc::new(if child == parent {
            Ownership::Levelled(Hierarchy::of(self.config.seed, child, role, children))
        } else {
            Ownership::Flat(Partition::of(
                self.config.seed,
                child,
                parent,
                role,
                children,
                parents,
            ))
        });
        self.ownership.write().insert(slot, Arc::clone(&built));
        built
    }

    /// The membership table two collections share.
    fn membership(&self, left: &str, right: &str) -> Arc<Membership> {
        let (anchor, member) = membership_sides(left, right);
        let slot = (LeanString::from(anchor), LeanString::from(member));
        if let Some(held) = self.memberships.read().get(&slot) {
            return Arc::clone(held);
        }
        let anchors = self.census.get(anchor).map_or(0, |c| c.derived.len());
        let ordinals: Vec<u64> = self.census.get(member).map_or_else(Vec::new, |census| {
            census
                .derived
                .iter()
                .filter_map(|key| census.slots.get(key).map(|slot| slot.ordinal))
                .collect()
        });
        let built = Arc::new(Membership::of(
            self.config.seed,
            anchor,
            anchors,
            member,
            &ordinals,
        ));
        self.memberships.write().insert(slot, Arc::clone(&built));
        built
    }

    /// Whether a record has been written to, so its own fields are the answer
    /// rather than what its ordinal derives.
    fn is_written(&self, entity: &str, key: &EntityKey) -> bool {
        self.delta.contains_key(&(entity.into(), key.clone()))
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
        let index = self.index_of(entity.name.as_str(), key);
        self.write_links(entity, ordinal, index, &mut fields);
        self.write_counts(entity, key, &mut fields);
        write_key_fields(entity, key, &mut fields);
        Some(fields)
    }

    /// Write every to-one link an instance carries, into the field the schema
    /// said carries it.
    ///
    /// Shared by the base layer and by creation, because a record a client made
    /// has to carry the same links a seeded one does — a `POST` that answered
    /// with a null where every other instance has an object is the difference
    /// between a mock a client can develop against and one it cannot.
    fn write_links(
        &self,
        entity: &EntityType,
        ordinal: u64,
        index: Option<u32>,
        fields: &mut JsonMap<String, JsonValue>,
    ) {
        for (field, relation) in entity.relations() {
            if relation.cardinality != Cardinality::One {
                continue;
            }
            let carrier = relation.carrier.key_field(&field.name);
            match self.derived_link(entity.name.as_str(), ordinal, index, &field.name, relation) {
                Some(value) => {
                    fields.insert(carrier.to_string(), value);
                }
                // Nothing to point at — a root of a hierarchy, or a target
                // with no instances. A link with a field of its own says so,
                // and so does one the schema said may be absent. What is left
                // is a declared scalar the spec marked required, where the
                // relation was usually inferred from the field's name: nulling
                // that on the strength of a name match is worse than leaving
                // the value its own type generated, and `world explain` reports
                // the hole either way.
                None if field.nullable
                    || relation.carrier.is_inline_key(&field.name)
                    || matches!(relation.carrier, Carrier::Embedded | Carrier::Connection(_)) =>
                {
                    fields.insert(carrier.to_string(), JsonValue::Null);
                }
                None => {}
            }
        }
    }

    /// Read the links a write states, wherever the caller put them.
    ///
    /// A client names a link the way its own schema does — `{"author": "<id>"}`,
    /// `{"author": {"id": "<id>"}}`, `{"authorId": "<id>"}` or the carrier field
    /// outright — and all four mean the same thing. Landing them all in the
    /// carrier is what makes a write actually retarget the relation instead of
    /// being kept as a field nothing reads.
    fn read_links(
        &self,
        entity: &EntityType,
        mut provided: JsonMap<String, JsonValue>,
    ) -> JsonMap<String, JsonValue> {
        // Nothing was stated, so there is nothing to read. Worth the check:
        // a bare create is the common one, and walking the relations to build
        // the names a caller *might* have used is pure waste when it sent none.
        if provided.is_empty() {
            return provided;
        }

        for (field, relation) in entity.relations() {
            if relation.cardinality != Cardinality::One {
                continue;
            }
            let carrier = relation.carrier.key_field(&field.name).clone();
            // Already a usable key. An *object* sitting there is not one: a
            // caller who sent `{"author": {"id": …}}` named the link, and
            // keeping the object would store a field nothing resolves.
            if provided
                .get(carrier.as_str())
                .is_some_and(|value| values::key_text(value).is_some())
            {
                continue;
            }

            let Some(target) = self.graph.get(relation.target.as_str()) else {
                continue;
            };
            let target_key = target
                .key
                .as_single()
                .cloned()
                .unwrap_or_else(|| LeanString::from("id"));

            // The link's own field, then the spellings an input object uses
            // for it. The aliases are only spelled out when the caller did not
            // use the field's own name, so the common write allocates nothing.
            let mut stated = provided
                .get(field.name.as_str())
                .and_then(|value| link_key_in(value, target_key.as_str()))
                .map(|key| (field.name.to_string(), key));

            if stated.is_none() {
                for alias in [format!("{}Id", field.name), format!("{}_id", field.name)] {
                    // An alias is only consumed when the entity has no field of
                    // that name, so a declared `user_id` is never mistaken for
                    // one.
                    if entity.field(&alias).is_some() {
                        continue;
                    }
                    if let Some(key) = provided
                        .get(&alias)
                        .and_then(|value| link_key_in(value, target_key.as_str()))
                    {
                        stated = Some((alias, key));
                        break;
                    }
                }
            }

            let Some((source, key)) = stated else {
                continue;
            };
            if source != carrier {
                provided.remove(&source);
            }
            provided.insert(carrier.to_string(), key_value_json(target, &key));
        }
        provided
    }

    /// The key a to-one link derives to for one instance, as the target's
    /// declared key kind.
    fn derived_link(
        &self,
        entity: &str,
        ordinal: u64,
        index: Option<u32>,
        field: &LeanString,
        relation: &Relation,
    ) -> Option<JsonValue> {
        let target = self.chosen_member(entity, ordinal, field.as_str(), relation)?;
        let target_type = self.graph.get(target.as_str())?;
        let key = self.owner_key(entity, index?, field.as_str(), target.as_str())?;
        Some(key_value_json(target_type, &key.to_string()))
    }

    /// How many children a parent has, without building any of them.
    ///
    /// The derived answer is the width of the partition's range. Writes move
    /// the edges: a child pointed somewhere else leaves, one pointed here
    /// arrives, and a removed one is gone — all read straight off the delta,
    /// so a count never materialises a record and can never recurse back into
    /// the record it is being written into.
    fn child_count(
        &self,
        entity: &str,
        key: &EntityKey,
        field: &FieldDef,
        relation: &Relation,
    ) -> usize {
        let Some(parent_index) = self.index_of(entity, key) else {
            return 0;
        };
        let mut total: i64 = 0;

        for member in relation.concrete_targets() {
            let Some(target) = self.graph.get(member.as_str()) else {
                continue;
            };

            // The same question `relation_children` asks, so the count and the
            // collection cannot land on different mechanisms.
            if is_membership(target, entity) {
                total += i64::try_from(self.membership_count(entity, key, member.as_str()))
                    .unwrap_or(i64::MAX);
                continue;
            }

            let back = reciprocal_link(target, entity);
            let role = back.map_or_else(String::new, |(field, _)| field.name.to_string());
            let carrier =
                back.map(|(field, relation)| relation.carrier.key_field(&field.name).clone());

            let partition = self.ownership(member.as_str(), entity, &role);
            let range = partition.range_of(parent_index);
            total += i64::from(range.end - range.start);

            for (child_key, stated) in self.stated_owners(member.as_str(), carrier.as_ref()) {
                let derived_here = self
                    .census
                    .get(member.as_str())
                    .and_then(|census| census.slots.get(&child_key))
                    .is_some_and(|slot| range.contains(&slot.index));
                let stated_here = stated
                    .as_ref()
                    .is_some_and(|stated| self.entity_key_of(entity, stated) == *key);
                match (derived_here, stated_here) {
                    (true, false) => total -= 1,
                    (false, true) => total += 1,
                    _ => {}
                }
            }
            let _ = field;
        }
        usize::try_from(total.max(0)).unwrap_or(0)
    }

    /// What every written record of an entity says its owner is.
    ///
    /// Read from the delta rather than through `get`, so nothing is
    /// materialised. A tombstone reports no owner, which is what removes it
    /// from whichever collection it was in — and so does a relation with no
    /// carrier to read, where a written record has left the derived collection
    /// with nothing to say where it went.
    fn stated_owners(
        &self,
        entity: &str,
        carrier: Option<&LeanString>,
    ) -> Vec<(EntityKey, Option<String>)> {
        self.delta
            .iter()
            .filter(|entry| entry.key().0 == entity)
            .map(|entry| {
                let stated = match entry.value() {
                    Delta::Created(fields) | Delta::Patched(fields) => carrier
                        .and_then(|carrier| fields.get(carrier.as_str()))
                        .and_then(values::key_text),
                    Delta::Tombstone => None,
                };
                (entry.key().1.clone(), stated)
            })
            .collect()
    }

    /// Write the fields that count a relation rather than holding one.
    ///
    /// `item_count` beside an `items` link is a thing real payloads carry and
    /// clients assert on, and a generated number that disagrees with the list
    /// endpoint is worse than no field at all.
    fn write_counts(
        &self,
        entity: &EntityType,
        key: &EntityKey,
        fields: &mut JsonMap<String, JsonValue>,
    ) {
        for field in &entity.fields {
            if field.relation().is_some() {
                continue;
            }
            let Some((counted, relation)) = counted_relation(entity, field.name.as_str()) else {
                continue;
            };
            let total = self.child_count(entity.name.as_str(), key, counted, relation);
            fields.insert(field.name.to_string(), JsonValue::from(total));
        }
    }

    /// The parent a child position belongs to, as that parent's key.
    ///
    /// The same partition both directions read, which is what makes them agree:
    /// `user.posts` is the range this function maps back into.
    fn owner_key(
        &self,
        child: &str,
        child_index: u32,
        role: &str,
        parent: &str,
    ) -> Option<EntityKey> {
        let partition = self.ownership(child, parent, role);
        let owner = partition.owner_of(child_index)?;
        self.census
            .get(parent)?
            .derived
            .get(usize::try_from(owner).ok()?)
            .cloned()
    }
}

/// Derive `count` keys for an entity, stepping over any a created record
/// already owns.
///
/// Skipping shifts the ordinal a slot derives from, which is why an instance's
/// ordinal is recorded rather than assumed to equal its position — everything
/// that pairs two instances compares keys, so a gap in the ordinals costs
/// nothing.
fn build_census(
    seed: u64,
    entity: &EntityType,
    count: usize,
    reserved: Option<&Vec<EntityKey>>,
) -> Census {
    let scalars = key_scalars(entity);
    let mut derived = Vec::with_capacity(count);
    let mut slots: FxHashMap<EntityKey, Slot> = FxHashMap::default();

    // With nothing reserved there is nothing to step over, and distinct
    // ordinals derive distinct keys — so the ordinary case pays none of the
    // probing, which is a hash of every key at load.
    let Some(reserved) = reserved else {
        // A key of one part is what almost every entity has, and this loop runs
        // once per instance of every entity in the world: the part is resolved
        // before it rather than per key.
        if let [(field, scalar)] = scalars.as_slice() {
            let field = field.as_deref();
            for ordinal in 0..count as u64 {
                let key = EntityKey::single(values::derive_key_value(
                    seed,
                    entity.name.as_str(),
                    field,
                    scalar,
                    ordinal,
                ));
                slots.insert(key.clone(), slot_at(ordinal, derived.len()));
                derived.push(key);
            }
        } else {
            for ordinal in 0..count as u64 {
                let key = derive_key(seed, entity.name.as_str(), &scalars, ordinal);
                slots.insert(key.clone(), slot_at(ordinal, derived.len()));
                derived.push(key);
            }
        }
        return Census { derived, slots };
    };

    let mut ordinal = 0_u64;
    while derived.len() < count {
        let key = derive_key(seed, entity.name.as_str(), &scalars, ordinal);
        if !reserved.contains(&key) {
            slots.insert(key.clone(), slot_at(ordinal, derived.len()));
            derived.push(key);
        }
        let Some(next) = ordinal.checked_add(1) else {
            break;
        };
        ordinal = next;
    }

    Census { derived, slots }
}

fn slot_at(ordinal: u64, index: usize) -> Slot {
    Slot {
        ordinal,
        index: u32::try_from(index).unwrap_or(u32::MAX),
    }
}

/// The scalar describing an entity's key field, so keys look like what the
/// spec said they are (a uuid, an integer, an opaque string).
fn key_scalar(entity: &EntityType) -> Scalar {
    entity.key.as_single().map_or_else(
        || Scalar::new(ScalarKind::Id),
        |name| field_scalar(entity, name),
    )
}

/// One scalar per part of an entity's key, each with the field naming it.
///
/// A single-part key carries no field name: it is derived from the entity, the
/// way it always has been, and naming it would change every key in every world
/// that already exists.
fn key_scalars(entity: &EntityType) -> Vec<(Option<LeanString>, Scalar)> {
    if entity.key.is_empty() {
        return vec![(None, Scalar::new(ScalarKind::Id))];
    }
    let composite = entity.key.len() > 1;
    entity
        .key
        .iter()
        .map(|part| {
            let field = composite.then(|| part.field.clone());
            (field, field_scalar(entity, part.field.as_str()))
        })
        .collect()
}

/// A whole key for one instance: every part, each derived as its own field.
fn derive_key(
    seed: u64,
    entity: &str,
    scalars: &[(Option<LeanString>, Scalar)],
    ordinal: u64,
) -> EntityKey {
    EntityKey::from_parts(scalars.iter().map(|(field, scalar)| {
        values::derive_key_value(seed, entity, field.as_deref(), scalar, ordinal)
    }))
}

/// The scalar describing one named field, defaulting to an opaque identifier.
fn field_scalar(entity: &EntityType, field: &str) -> Scalar {
    entity
        .field(field)
        .and_then(|field| match &field.value {
            ValueSpec::Scalar(scalar) => Some(scalar.clone()),
            _ => None,
        })
        .unwrap_or_else(|| Scalar::new(ScalarKind::Id))
}

/// An entity's key, rendered as the kind its schema declared.
///
/// Only meaningful for a single-part key: a link to a composite-keyed entity
/// has no one field to write, so the text is kept as it stands.
fn key_value_json(entity: &EntityType, value: &str) -> JsonValue {
    if entity.key.len() > 1 {
        return JsonValue::String(value.to_string());
    }
    values::key_json(&key_scalar(entity).kind, value)
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

/// Whether a to-many onto `target` is a membership rather than an ownership.
///
/// Both ends being collections leaves no owner to derive from. A to-many
/// pointing back does not settle that on its own — a folder's `children` is
/// itself a to-many at `Folder`, and an unrelated `liked_by` is one at `User` —
/// so it is the absence of a functional carrier that decides. Everything
/// reading the relation has to ask this same question or the readers answer
/// from different mechanisms.
pub(crate) fn is_membership(target: &EntityType, entity: &str) -> bool {
    reciprocal_link(target, entity).is_none()
        && target.relations().any(|(_, relation)| {
            relation.cardinality == Cardinality::Many
                && relation.concrete_targets().iter().any(|t| t == entity)
        })
}

/// The to-many relation a `*_count` field is counting, if it is counting one.
///
/// Matched on the name with the suffix removed: `item_count` counts `items`,
/// `commentCount` counts `comments`. A field whose stem names nothing is an
/// ordinary number.
pub(crate) fn counted_relation<'a>(
    entity: &'a EntityType,
    field: &str,
) -> Option<(&'a FieldDef, &'a Relation)> {
    // Cheap first: this is asked of every field of every record that is read,
    // and almost none of them are counting anything. Only a name that actually
    // ends in `count` is worth normalising.
    let tail = field.len().checked_sub(5).filter(|at| *at > 0);
    if !tail
        .and_then(|at| field.get(at..))
        .is_some_and(|tail| tail.eq_ignore_ascii_case("count"))
    {
        return None;
    }
    let lowered = field.to_ascii_lowercase().replace(['_', '-'], "");
    let stem = lowered.strip_suffix("count")?;
    if stem.is_empty() {
        return None;
    }
    entity.relations().find(|(candidate, relation)| {
        if relation.cardinality != Cardinality::Many {
            return false;
        }
        let name = candidate.name.to_ascii_lowercase().replace(['_', '-'], "");
        name == stem || singular(&name) == singular(stem)
    })
}

/// A crude singular, only ever compared against another crude singular.
fn singular(name: &str) -> String {
    name.strip_suffix('s').unwrap_or(name).to_string()
}

/// The field on `entity` that links back to `target`, naming the relation so
/// both directions derive the same pairing, and carrying it so the field
/// actually holding the key can be read.
fn reciprocal_link<'a>(
    entity: &'a EntityType,
    target: &str,
) -> Option<(&'a FieldDef, &'a Relation)> {
    entity
        .relations()
        .find(|(_, relation)| relation.target == target && relation.cardinality == Cardinality::One)
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
    values::key_text(value)
}

/// The key a written value states, whether it is the key itself or an object
/// carrying it.
fn link_key_in(value: &JsonValue, key_field: &str) -> Option<String> {
    match value {
        JsonValue::Object(object) => object.get(key_field).and_then(values::key_text),
        other => values::key_text(other),
    }
}

/// Keep the key fields in the payload agreeing with the key the record is
/// filed under; a record whose `id` disagrees with its own address is the
/// fastest way to lose a client's trust.
fn write_key_fields(entity: &EntityType, key: &EntityKey, fields: &mut JsonMap<String, JsonValue>) {
    for (part, value) in entity.key.iter().zip(key.parts()) {
        let kind = entity
            .field(part.field.as_str())
            .and_then(|field| match &field.value {
                ValueSpec::Scalar(scalar) => Some(&scalar.kind),
                _ => None,
            })
            .unwrap_or(&ScalarKind::Id);
        fields.insert(
            part.field.to_string(),
            values::key_json(kind, value.as_str()),
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
            (start, start.saturating_add(*take).min(total))
        }
        Page::After { cursor, first } => {
            let start = cursor
                .as_ref()
                .and_then(|c| position_of(records, c).map(|i| i + 1))
                .unwrap_or(0)
                .min(total);
            (start, start.saturating_add(*first).min(total))
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

/// Where a cursor sits among keys — the key-level twin of [`position_of`],
/// which needs materialised records to ask the same question.
fn key_position(keys: &[EntityKey], cursor: &Cursor) -> Option<usize> {
    keys.iter()
        .position(|key| key.to_string() == cursor.as_str())
}

fn position_of(records: &[Record], cursor: &Cursor) -> Option<usize> {
    records
        .iter()
        .position(|record| record.key.to_string() == cursor.as_str())
}

#[cfg(test)]
mod tests;
