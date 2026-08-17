//! `world.*` — the engine's entity world, from Node.
//!
//! The same store a schema-derived route serves, a Tera template reads and a
//! QuickJS handler writes. A spec populates the world; it does not own it, so
//! a handler that creates a user is answering the same question the spec's
//! `users` query answers.
//!
//! ```ts
//! import { world } from 'ferrimock'
//!
//! const user = world.create('User', { name: 'Ada' })
//! world.list('Folder', { filter: { owner: user.id }, sort: '-createdAt' })
//! ```
//!
//! Every call is synchronous: a `DashMap` read behind an `Arc`, so it stays on
//! the handler fast path with nothing to await.
//!
//! `serde_json::Value` crosses the NAPI boundary by matching on the enum
//! directly (see `napi`'s `ToNapiValue for &Value`), not through the serde
//! data model — so the `arbitrary_precision` number tagging that the QuickJS
//! lane has to work around does not arise here.

use napi::bindgen_prelude::{Either, Undefined};
use napi::{Error, Result, Status};
use napi_derive::napi;

use ferrimock::core::{EntityQuery, global_world};

/// A slice of one entity's instances.
#[napi(object, namespace = "world")]
pub struct WorldPage {
    pub records: Vec<serde_json::Value>,
    pub total: u32,
    pub has_next: bool,
    pub has_previous: bool,
}

/// What to read. Every field optional, so `world.list('User')` is the whole set.
#[napi(object, namespace = "world")]
pub struct WorldQuery {
    /// Field to value. A value matches for equality unless it is an object
    /// carrying one operator key: `{ age: { gt: 30 } }`, `{ id: { in: [...] } }`.
    pub filter: Option<serde_json::Value>,
    /// A field name, or several. `-name` sorts descending.
    pub sort: Option<Either<String, Vec<String>>>,
    pub skip: Option<u32>,
    pub limit: Option<u32>,
}

fn failure(e: impl std::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, e.to_string())
}

impl WorldQuery {
    fn into_query(self) -> Result<EntityQuery> {
        let filter = match self.filter {
            None | Some(serde_json::Value::Null) => serde_json::Map::new(),
            Some(serde_json::Value::Object(map)) => map,
            Some(_) => {
                return Err(failure("`filter` has to be an object of field to value"));
            }
        };

        Ok(EntityQuery {
            filter,
            sort: match self.sort {
                None => Vec::new(),
                Some(Either::A(one)) => vec![one],
                Some(Either::B(many)) => many,
            },
            skip: self.skip.unwrap_or(0) as usize,
            limit: self.limit.map(|n| n as usize),
        })
    }
}

fn query_of(options: Option<WorldQuery>) -> Result<EntityQuery> {
    options.map_or_else(|| Ok(EntityQuery::default()), WorldQuery::into_query)
}

fn page_of(page: ferrimock::core::EntityPage) -> WorldPage {
    WorldPage {
        records: page.records,
        total: u32::try_from(page.total).unwrap_or(u32::MAX),
        has_next: page.has_next,
        has_previous: page.has_previous,
    }
}

fn object_or_empty(values: Option<serde_json::Value>) -> Result<serde_json::Value> {
    match values {
        None | Some(serde_json::Value::Null) => {
            Ok(serde_json::Value::Object(serde_json::Map::new()))
        }
        Some(object @ serde_json::Value::Object(_)) => Ok(object),
        Some(_) => Err(failure("values have to be an object")),
    }
}

/// Entity types the world knows.
#[napi(namespace = "world")]
pub fn types() -> Vec<String> {
    global_world()
        .entities()
        .into_iter()
        .map(|name| name.to_string())
        .collect()
}

/// How many instances of an entity exist.
#[napi(namespace = "world")]
pub fn count(entity: String) -> u32 {
    u32::try_from(global_world().count(&entity)).unwrap_or(u32::MAX)
}

/// Read one instance by key.
///
/// `undefined` when it never existed or was removed — a miss is an ordinary
/// answer, and a handler wants `if (!user) return ...`.
///
/// Explicitly `undefined` rather than `null`: `Option` would surface as
/// `null` here while the QuickJS lane hands back `undefined`, and a handler
/// has to behave the same on both.
#[napi(namespace = "world")]
pub fn get(entity: String, key: String) -> Either<serde_json::Value, Undefined> {
    global_world()
        .get(&entity, &key)
        .map_or(Either::B(()), Either::A)
}

/// Read a slice of an entity's instances.
#[napi(namespace = "world")]
pub fn list(entity: String, options: Option<WorldQuery>) -> Result<WorldPage> {
    global_world()
        .list(&entity, &query_of(options)?)
        .map(page_of)
        .map_err(failure)
}

/// Follow a relation from one instance.
#[napi(namespace = "world")]
pub fn related(
    entity: String,
    key: String,
    field: String,
    options: Option<WorldQuery>,
) -> Result<WorldPage> {
    global_world()
        .related(&entity, &key, &field, &query_of(options)?)
        .map(page_of)
        .map_err(failure)
}

/// Create an instance.
///
/// Fields left out are generated from the seed, so the result validates
/// against the same schema a real one would.
#[napi(namespace = "world")]
pub fn create(entity: String, values: Option<serde_json::Value>) -> Result<serde_json::Value> {
    global_world()
        .create(&entity, object_or_empty(values)?)
        .map_err(failure)
}

/// Merge fields into an existing instance.
#[napi(namespace = "world")]
pub fn update(
    entity: String,
    key: String,
    values: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    global_world()
        .update(&entity, &key, object_or_empty(values)?)
        .map_err(failure)
}

/// Replace an instance wholesale, keeping its key.
#[napi(namespace = "world")]
pub fn replace(
    entity: String,
    key: String,
    values: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    global_world()
        .replace(&entity, &key, object_or_empty(values)?)
        .map_err(failure)
}

/// Remove an instance.
#[napi(namespace = "world", js_name = "delete")]
pub fn delete_entity(entity: String, key: String) -> Result<()> {
    global_world().delete(&entity, &key).map_err(failure)
}

/// Drop every write, leaving exactly what the seed derives.
///
/// The typed counterpart of clearing the persistence store — call it between
/// tests, or state leaks from one into the next.
#[napi(namespace = "world")]
pub fn reset() {
    global_world().reset();
}

/// How many writes are laid over the seeded world.
#[napi(namespace = "world")]
pub fn pending_writes() -> u32 {
    u32::try_from(global_world().pending_writes()).unwrap_or(u32::MAX)
}
