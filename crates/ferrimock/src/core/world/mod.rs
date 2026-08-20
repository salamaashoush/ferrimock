//! The entity world: what a mocked API pretends to have.
//!
//! Two halves, deliberately not owned by whatever populated them:
//!
//! - the **graph** — entity types, keys, relations — merged from every schema
//!   loaded into this process, so two schemas that both mention `User` mean
//!   the same `User`;
//! - the **store** — the seeded instances and every write against them.
//!
//! It sits beside [`crate::core::PersistenceStore`] on the registry and is
//! reached the same way: from templates, from scripts, and over HTTP. That is
//! the point. A spec *populates* the world; it does not own it, so a
//! declarative template and a JS handler read and write the same entities the
//! spec-derived routes serve.

pub mod algebra;
pub mod doctor;
pub mod model;
pub mod overrides;
pub mod store;

use lean_string::LeanString;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use algebra::{Mutation, Page, Predicate, PredicateOp, Selection, SortKey};
use model::{EntityGraph, EntityKey, EntityType};
use overrides::{FieldRules, RejectedRule};
use store::{DeltaConflict, EntityStore, Record, StoreConfig};

/// Seed, sizes, and who asked for them.
///
/// The source paths are kept so a second collection setting a different seed
/// can be refused by name rather than by silently winning.
#[derive(Debug, Clone, Default)]
struct Settings {
    seed: Option<u64>,
    seed_source: Option<PathBuf>,
    default_count: Option<usize>,
    default_count_source: Option<PathBuf>,
    scale: Option<f64>,
    counts: FxHashMap<LeanString, usize>,
    cascade_delete: Option<bool>,
    overrides: FieldRules,
}

impl Settings {
    fn to_store_config(&self) -> StoreConfig {
        let mut config = StoreConfig::seeded(
            self.seed
                .or_else(crate::fake_data::rng::global_seed)
                .unwrap_or(0),
        );
        config.default_count = self.default_count;
        if let Some(scale) = self.scale {
            config.scale = scale;
        }
        if let Some(cascade) = self.cascade_delete {
            config.cascade_delete = cascade;
        }
        config.counts.clone_from(&self.counts);
        config
    }
}

/// The file that already set a single-valued setting to something else.
///
/// `None` when nothing is contested: no previous value, the same value, or the
/// same file setting it again — that last one is an edit being reloaded, not
/// two collections disagreeing.
fn contested<'a, T: PartialEq + Copy>(
    current: Option<T>,
    current_source: Option<&'a Path>,
    wanted: &T,
    source: &Path,
) -> Option<&'a Path> {
    let existing = current?;
    let existing_source = current_source?;
    (existing != *wanted && existing_source != source).then_some(existing_source)
}

/// What a collection's `world:` block asks for.
#[derive(Debug, Clone, Default)]
pub struct WorldSettings {
    pub seed: Option<u64>,
    pub default_count: Option<usize>,
    /// Multiplies whatever the default resolves to, so a mount can ask for a
    /// bigger world without naming every entity in it.
    pub scale: Option<f64>,
    pub counts: FxHashMap<LeanString, usize>,
    /// Whether removing a record also removes what points at it. `None` keeps
    /// whatever the world already had.
    pub cascade_delete: Option<bool>,
    /// What a field should hold, where the schema does not say. Collections
    /// accumulate these rather than contesting them: two files naming the same
    /// field is the last one loaded winning, which is the same rule `counts`
    /// already follows.
    pub overrides: FieldRules,
}

/// How a loaded schema is served, kept per schema because the entity graph
/// merges but an operation table does not.
#[cfg(feature = "graphql")]
#[derive(Clone)]
pub enum Binding {
    /// A GraphQL schema, bound by building an executable schema over the world.
    GraphQL(Arc<crate::graphql::introspection::ParsedSchema>),
    /// An OpenAPI document, bound by mounting one mock per operation.
    #[cfg(feature = "spec")]
    OpenApi(Arc<crate::spec::infer::openapi::OperationTable>),
}

#[cfg(feature = "graphql")]
impl Binding {
    /// What `serve:` calls this protocol.
    #[must_use]
    pub const fn protocol(&self) -> &'static str {
        match self {
            Self::GraphQL(_) => "graphql",
            #[cfg(feature = "spec")]
            Self::OpenApi(_) => "rest",
        }
    }

    /// How many endpoints this binding mounts, when it knows in advance.
    ///
    /// GraphQL never does — the client chooses the operation name, so the
    /// schema mounts as one endpoint. A document designs its endpoints, so it
    /// can say.
    #[must_use]
    pub fn endpoints(&self) -> Option<usize> {
        match self {
            Self::GraphQL(_) => None,
            #[cfg(feature = "spec")]
            Self::OpenApi(table) => Some(table.operations.len()),
        }
    }
}

#[cfg(feature = "graphql")]
#[derive(Clone)]
pub struct LoadedSchema {
    pub path: PathBuf,
    pub binding: Binding,
    /// Entities this schema contributed, before merging.
    pub entities: Vec<LeanString>,
}

/// An entity name contributed by more than one schema.
///
/// Merging is usually what you want — one company's REST and GraphQL surfaces
/// describe one `User` — but it is never what you want silently.
#[derive(Debug, Clone)]
pub struct EntityCollision {
    pub entity: LeanString,
    pub sources: Vec<PathBuf>,
}

impl std::fmt::Display for EntityCollision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` is declared by ", self.entity)?;
        for (i, source) in self.sources.iter().enumerate() {
            if i > 0 {
                f.write_str(" and ")?;
            }
            write!(f, "{}", source.display())?;
        }
        Ok(())
    }
}

/// A slice of one entity's instances, in the shape templates, scripts and the
/// HTTP API all hand back.
#[derive(Debug, Clone)]
pub struct EntityPage {
    pub records: Vec<JsonValue>,
    pub total: usize,
    pub has_next: bool,
    pub has_previous: bool,
}

/// What to read, in terms every caller can build from its own syntax.
#[derive(Debug, Clone, Default)]
pub struct EntityQuery {
    /// Field to value. A value is matched for equality unless it is an object
    /// carrying one operator key (`{"gt": 5}`, `{"in": [...]}`).
    pub filter: JsonMap<String, JsonValue>,
    /// Field names, `-name` for descending.
    pub sort: Vec<String>,
    pub skip: usize,
    pub limit: Option<usize>,
}

impl EntityQuery {
    fn to_selection(&self) -> Selection {
        let mut selection = Selection::new();

        for (field, value) in &self.filter {
            selection.filters.push(predicate_of(field, value));
        }

        for key in &self.sort {
            selection.sort.push(match key.strip_prefix('-') {
                Some(field) => SortKey::desc(field),
                None => SortKey::asc(key.as_str()),
            });
        }

        if self.skip > 0 || self.limit.is_some() {
            selection.page = Page::Offset {
                skip: self.skip,
                take: self.limit.unwrap_or(usize::MAX),
            };
        }

        selection
    }
}

/// Read one filter entry. A bare value means equality; a single-key object
/// names the operator, which is what lets one syntax serve YAML, JS and a
/// query string without any of them growing a filter DSL.
fn predicate_of(field: &str, value: &JsonValue) -> Predicate {
    if let JsonValue::Object(map) = value
        && map.len() == 1
        && let Some((op, operand)) = map.iter().next()
        && let Some(op) = predicate_op(op)
    {
        return Predicate {
            field: field.into(),
            op,
            value: operand.clone(),
        };
    }
    Predicate::eq(field, value.clone())
}

fn predicate_op(name: &str) -> Option<PredicateOp> {
    Some(match name {
        "eq" => PredicateOp::Eq,
        "ne" => PredicateOp::Ne,
        "in" => PredicateOp::In,
        "gt" => PredicateOp::Gt,
        "gte" => PredicateOp::Gte,
        "lt" => PredicateOp::Lt,
        "lte" => PredicateOp::Lte,
        "contains" => PredicateOp::Contains,
        _ => return None,
    })
}

/// The entity graph and the store over it, shared by every lane.
pub struct World {
    graph: RwLock<Arc<EntityGraph>>,
    /// Swapped wholesale when the graph changes, so callers must read it
    /// through [`World::store`] rather than caching the `Arc` — a binding that
    /// held the old one would quietly serve a stale world.
    store: RwLock<Arc<EntityStore>>,
    settings: RwLock<Settings>,
    /// Serializes store rebuilds so two schemas loading at once cannot each
    /// snapshot the delta and then overwrite the other's swap.
    rebuilding: parking_lot::Mutex<()>,
    #[cfg(feature = "graphql")]
    schemas: RwLock<Vec<LoadedSchema>>,
    /// Which schema each entity name came from, for collision reporting.
    origins: RwLock<FxHashMap<LeanString, Vec<PathBuf>>>,
    /// What each source declares, kept apart so a reload can replace one
    /// source's contribution without guessing which fields were its.
    contributions: RwLock<Vec<(PathBuf, EntityGraph)>>,
    /// Overrides that matched nothing, or named something the store owns.
    rejected: RwLock<Vec<RejectedRule>>,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World")
            .field("entities", &self.graph.read().len())
            .field("seed", &self.store.read().seed())
            .finish_non_exhaustive()
    }
}

impl World {
    #[must_use]
    pub fn new() -> Self {
        let graph = Arc::new(EntityGraph::new());
        let store = Arc::new(EntityStore::new(
            Arc::clone(&graph),
            Settings::default().to_store_config(),
        ));
        Self {
            graph: RwLock::new(graph),
            store: RwLock::new(store),
            settings: RwLock::new(Settings::default()),
            rebuilding: parking_lot::Mutex::new(()),
            #[cfg(feature = "graphql")]
            schemas: RwLock::new(Vec::new()),
            origins: RwLock::new(FxHashMap::default()),
            contributions: RwLock::new(Vec::new()),
            rejected: RwLock::new(Vec::new()),
        }
    }

    /// The current store. Read it per use; a rebuild replaces it.
    #[must_use]
    pub fn store(&self) -> Arc<EntityStore> {
        Arc::clone(&self.store.read())
    }

    #[must_use]
    pub fn graph(&self) -> Arc<EntityGraph> {
        Arc::clone(&self.graph.read())
    }

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.store.read().seed()
    }

    /// Whether any schema has been loaded. An empty world serves nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.graph.read().is_empty()
    }

    /// Apply a collection's `world:` block.
    ///
    /// There is one world, so there is one seed. A second collection asking
    /// for a different one is a mistake worth failing on, named by file — but
    /// the *same* file asking for a different one is an edit, and a hot reload
    /// has to be able to apply it or the setting is only changeable by restart.
    pub fn configure(&self, settings: &WorldSettings, source: &Path) -> crate::Result<()> {
        {
            let mut current = self.settings.write();

            if let Some(seed) = settings.seed {
                if let Some(existing_source) =
                    contested(current.seed, current.seed_source.as_deref(), &seed, source)
                {
                    return Err(crate::mp_err!(
                        "the world has one seed: {} sets {}, {} sets {seed}",
                        existing_source.display(),
                        current.seed.unwrap_or_default(),
                        source.display()
                    ));
                }
                current.seed = Some(seed);
                current.seed_source = Some(source.to_path_buf());
            }

            if let Some(count) = settings.default_count {
                if let Some(existing_source) = contested(
                    current.default_count,
                    current.default_count_source.as_deref(),
                    &count,
                    source,
                ) {
                    return Err(crate::mp_err!(
                        "the world has one default count: {} sets {}, {} sets {count}",
                        existing_source.display(),
                        current.default_count.unwrap_or_default(),
                        source.display()
                    ));
                }
                current.default_count = Some(count);
                current.default_count_source = Some(source.to_path_buf());
            }

            if let Some(scale) = settings.scale {
                current.scale = Some(scale);
            }

            for (entity, count) in &settings.counts {
                current.counts.insert(entity.clone(), *count);
            }
            current.overrides.extend(&settings.overrides);

            if let Some(cascade) = settings.cascade_delete {
                current.cascade_delete = Some(cascade);
            }
        }

        // Rules may arrive after the schemas they describe — a second
        // collection adding overrides to a world the first one populated — so
        // the graph is recomposed rather than left as it was built.
        self.recompose();
        self.rebuild().map(|_| ())
    }

    /// Merge a schema's entities in and record how it is served.
    ///
    /// Returns the writes that could not be carried onto the rebuilt store.
    #[cfg(feature = "graphql")]
    pub fn add_schema(
        &self,
        path: &Path,
        binding: Binding,
        contribution: &EntityGraph,
    ) -> crate::Result<Vec<DeltaConflict>> {
        let entities: Vec<LeanString> = contribution.entities().map(|e| e.name.clone()).collect();

        {
            let mut origins = self.origins.write();
            for name in &entities {
                let sources = origins.entry(name.clone()).or_default();
                // A reload re-reads the same file; recording it twice would
                // report the schema as colliding with itself.
                if !sources.iter().any(|source| source == path) {
                    sources.push(path.to_path_buf());
                }
            }
        }

        {
            let entry = LoadedSchema {
                path: path.to_path_buf(),
                binding,
                entities,
            };
            let mut schemas = self.schemas.write();
            // Replace rather than push: a reload of the same path must not
            // leave two candidates behind, or `serve: graphql` would start
            // refusing to resolve a schema that has not changed.
            match schemas.iter_mut().find(|s| s.path == path) {
                Some(existing) => *existing = entry,
                None => schemas.push(entry),
            }
        }

        self.merge(path, contribution);
        self.rebuild()
    }

    /// Merge entities in without registering a way to serve them. For an
    /// embedder building a world by hand.
    pub fn add_entities(&self, contribution: &EntityGraph) -> crate::Result<Vec<DeltaConflict>> {
        self.merge(Path::new(EMBEDDED_SOURCE), contribution);
        self.rebuild()
    }

    /// Record what one source declares, and rebuild the merged graph from every
    /// source.
    ///
    /// Recomposing rather than folding into the existing graph is what makes a
    /// reload correct: a schema that dropped a field has to *lose* it, and
    /// there is no way to tell a dropped field from a field another schema
    /// contributed unless each source's declaration is kept apart.
    fn merge(&self, source: &Path, contribution: &EntityGraph) {
        // One lock across the read-modify-write. Cloning out from under a read
        // guard and writing back afterwards let two schemas loading at once
        // lose one of them, which showed up as an entity that had definitely
        // been loaded reading as unknown.
        let mut graph = self.graph.write();
        let mut contributions = self.contributions.write();

        let entry = (source.to_path_buf(), contribution.clone());
        match contributions.iter_mut().find(|(held, _)| held == source) {
            // An embedder has no file to key on, so its additions accumulate
            // rather than replacing each other.
            Some((_, held)) if source == Path::new(EMBEDDED_SOURCE) => {
                for entity in contribution.entities() {
                    absorb_into(held, entity);
                }
            }
            Some(held) => *held = entry,
            None => contributions.push(entry),
        }

        let rules = self.settings.read().overrides.clone();
        *graph = Arc::new(compose(&contributions, &rules, &self.rejected));
    }

    /// Rebuild the merged graph from what every source declared, applying the
    /// current rules to it.
    fn recompose(&self) {
        let mut graph = self.graph.write();
        let contributions = self.contributions.read();
        let rules = self.settings.read().overrides.clone();
        *graph = Arc::new(compose(&contributions, &rules, &self.rejected));
    }

    /// Rules that named something the world does not have, or may not change.
    #[must_use]
    pub fn rejected_overrides(&self) -> Vec<RejectedRule> {
        self.rejected.read().clone()
    }

    /// Rebuild the store from the current graph and settings, carrying every
    /// write across.
    ///
    /// The base layer is derived from the seed, so entity names and ordinals
    /// that already existed keep their exact values — adding a schema does not
    /// disturb a world already in use.
    ///
    /// Rebuilds are serialized against each other, but a write landing between
    /// the snapshot and the swap is lost. Loading schemas is a startup and
    /// hot-reload activity, so that window is not on the request path; a
    /// handler writing at the instant a schema reloads is the one case, and
    /// widening the lock to cover it would put a mutex on every read.
    pub fn rebuild(&self) -> crate::Result<Vec<DeltaConflict>> {
        let _serialized = self.rebuilding.lock();

        let graph = self.graph();
        let config = self.settings.read().to_store_config();

        let snapshot = self.store.read().export_delta();
        let rebuilt = EntityStore::new_reserving(graph, config, snapshot.reserved_keys());
        let conflicts = rebuilt.import_delta(snapshot);

        *self.store.write() = Arc::new(rebuilt);
        Ok(conflicts)
    }

    /// Drop every write, leaving exactly what the seed derives.
    pub fn reset(&self) {
        self.store.read().reset();
    }

    /// Entity names declared by more than one schema.
    #[must_use]
    pub fn collisions(&self) -> Vec<EntityCollision> {
        let mut collisions: Vec<EntityCollision> = self
            .origins
            .read()
            .iter()
            .filter(|(_, sources)| sources.len() > 1)
            .map(|(entity, sources)| EntityCollision {
                entity: entity.clone(),
                sources: sources.clone(),
            })
            .collect();
        collisions.sort_by(|a, b| a.entity.cmp(&b.entity));
        collisions
    }

    /// Every schema loaded into this world.
    #[cfg(feature = "graphql")]
    #[must_use]
    pub fn schemas(&self) -> Vec<LoadedSchema> {
        self.schemas.read().clone()
    }

    /// Resolve the schema a `serve:` refers to.
    ///
    /// With one candidate the protocol alone is unambiguous. With several the
    /// caller has to say which, because guessing would bind a route to a
    /// schema nobody named.
    #[cfg(feature = "graphql")]
    pub fn resolve_schema(
        &self,
        protocol: &str,
        selector: Option<&str>,
        mock_id: &str,
    ) -> crate::Result<LoadedSchema> {
        let schemas = self.schemas.read();
        let candidates: Vec<&LoadedSchema> = schemas
            .iter()
            .filter(|s| s.binding.protocol() == protocol)
            .collect();

        if let Some(selector) = selector {
            let wanted = Path::new(selector);
            return candidates
                .iter()
                .find(|s| s.path == wanted || s.path.ends_with(wanted))
                .map(|s| (*s).clone())
                .ok_or_else(|| {
                    crate::mp_err!(
                        "mock `{mock_id}`: no {protocol} schema matching `{selector}` is loaded \
                         into the world{}",
                        list_paths(&candidates)
                    )
                });
        }

        match candidates.as_slice() {
            [] => Err(crate::mp_err!(
                "mock `{mock_id}`: `serve: {protocol}` needs a {protocol} schema in the world; \
                 add one under `world.schemas`"
            )),
            [only] => Ok((*only).clone()),
            many => Err(crate::mp_err!(
                "mock `{mock_id}`: `serve: {protocol}` matches {} schemas{}. Name one with \
                 `serve: {{ protocol: {protocol}, schema: <path> }}`",
                many.len(),
                list_paths(many)
            )),
        }
    }

    // ===== Entity access, the surface templates / scripts / HTTP share =====

    /// How many instances of an entity exist.
    #[must_use]
    pub fn count(&self, entity: &str) -> usize {
        self.store().count(entity)
    }

    /// Entity names in the world.
    #[must_use]
    pub fn entities(&self) -> Vec<LeanString> {
        self.graph().entities().map(|e| e.name.clone()).collect()
    }

    /// The key an entity is addressed by, read from one string.
    ///
    /// A composite key is written the way it prints: its parts separated by
    /// `/`, which is what a path addressing it looks like anyway.
    #[must_use]
    pub fn entity_key(&self, entity: &str, key: &str) -> EntityKey {
        self.store().entity_key_of(entity, key)
    }

    /// Read one instance by key.
    #[must_use]
    pub fn get(&self, entity: &str, key: &str) -> Option<JsonValue> {
        self.store()
            .get(entity, &self.entity_key(entity, key))
            .map(record_json)
    }

    /// Read a slice of an entity's instances.
    pub fn list(&self, entity: &str, query: &EntityQuery) -> crate::Result<EntityPage> {
        let page = self.store().list(entity, &query.to_selection())?;
        Ok(EntityPage {
            records: page.records.into_iter().map(record_json).collect(),
            total: page.total,
            has_next: page.has_next,
            has_previous: page.has_previous,
        })
    }

    /// Follow a relation from one instance.
    pub fn related(
        &self,
        entity: &str,
        key: &str,
        field: &str,
        query: &EntityQuery,
    ) -> crate::Result<EntityPage> {
        let page = self.store().related(
            entity,
            &self.entity_key(entity, key),
            field,
            &query.to_selection(),
        )?;
        Ok(EntityPage {
            records: page.records.into_iter().map(record_json).collect(),
            total: page.total,
            has_next: page.has_next,
            has_previous: page.has_previous,
        })
    }

    /// Create an instance. Fields left out are generated from the seed, so the
    /// result validates against the same schema a real one would.
    pub fn create(&self, entity: &str, values: JsonValue) -> crate::Result<JsonValue> {
        match self.store().apply(entity, Mutation::Insert { values })? {
            store::Written::Created(record) => Ok(record_json(record)),
            other => Err(crate::mp_err!("expected a creation, got {other:?}")),
        }
    }

    /// Merge fields into an existing instance.
    pub fn update(&self, entity: &str, key: &str, values: JsonValue) -> crate::Result<JsonValue> {
        let mutation = Mutation::Patch {
            key: self.entity_key(entity, key),
            values,
        };
        match self.store().apply(entity, mutation)? {
            store::Written::Updated(record) => Ok(record_json(record)),
            other => Err(crate::mp_err!("expected an update, got {other:?}")),
        }
    }

    /// Replace an instance wholesale, keeping its key.
    pub fn replace(&self, entity: &str, key: &str, values: JsonValue) -> crate::Result<JsonValue> {
        let mutation = Mutation::Replace {
            key: self.entity_key(entity, key),
            values,
        };
        match self.store().apply(entity, mutation)? {
            store::Written::Updated(record) => Ok(record_json(record)),
            other => Err(crate::mp_err!("expected an update, got {other:?}")),
        }
    }

    /// Remove an instance.
    pub fn delete(&self, entity: &str, key: &str) -> crate::Result<()> {
        let mutation = Mutation::Remove {
            key: self.entity_key(entity, key),
        };
        self.store().apply(entity, mutation).map(|_| ())
    }

    /// How many writes are laid over the seeded world. Non-zero between tests
    /// means state is leaking from one into the next.
    #[must_use]
    pub fn pending_writes(&self) -> usize {
        self.store().export_delta().len()
    }
}

/// The source name entities added by an embedder are filed under.
const EMBEDDED_SOURCE: &str = "<embedded>";

/// Merge every source's declaration, then apply the rules to the result.
///
/// Rules are applied to the *merged* graph rather than to each contribution:
/// a rule about `User.email` should not care which of two schemas declared
/// `email`, and applying it per contribution would run it twice.
fn compose(
    contributions: &[(PathBuf, EntityGraph)],
    rules: &FieldRules,
    rejected: &RwLock<Vec<RejectedRule>>,
) -> EntityGraph {
    let mut merged = EntityGraph::new();
    for (_, declared) in contributions {
        for entity in declared.entities() {
            absorb_into(&mut merged, entity);
        }
    }
    *rejected.write() = overrides::apply(&mut merged, rules);
    merged
}

/// Insert an entity, folding it into any declaration already held.
fn absorb_into(graph: &mut EntityGraph, entity: &EntityType) {
    match graph.get_mut(entity.name.as_str()) {
        Some(held) => held.absorb(entity),
        None => graph.insert(entity.clone()),
    }
}

fn record_json(record: Record) -> JsonValue {
    JsonValue::Object(record.fields)
}

#[cfg(feature = "graphql")]
fn list_paths(schemas: &[&LoadedSchema]) -> String {
    if schemas.is_empty() {
        return String::new();
    }
    format!(
        " (loaded: {})",
        schemas
            .iter()
            .map(|s| s.path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Entity names the graph knows, for a "did you mean" on a typo.
#[must_use]
pub fn nearest_entity<'a>(graph: &'a EntityGraph, wanted: &str) -> Option<&'a EntityType> {
    graph
        .entities()
        .map(|entity| {
            (
                crate::core::levenshtein_distance(entity.name.as_str(), wanted),
                entity,
            )
        })
        .filter(|(distance, _)| *distance <= 3)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, entity)| entity)
}

// ============================================================================
// GLOBAL WORLD
// ============================================================================

// Tera's function registry is stateless, so templates cannot be handed a
// world per render. The persistence store solved this with a process global
// and the world follows it rather than inventing a second mechanism; see
// `template::store`. The cost is the same one that store already pays: two
// registries in one process share it.
static GLOBAL_WORLD: OnceLock<Arc<World>> = OnceLock::new();

/// The process-wide world, created on first use.
pub fn global_world() -> Arc<World> {
    Arc::clone(GLOBAL_WORLD.get_or_init(|| Arc::new(World::new())))
}

/// Install a world before anything reads one.
///
/// # Errors
/// Returns the passed world back when one is already installed.
pub fn set_global_world(world: Arc<World>) -> Result<(), Arc<World>> {
    GLOBAL_WORLD.set(world)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests;
