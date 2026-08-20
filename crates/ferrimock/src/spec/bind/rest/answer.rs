//! Answering one REST operation from the world.
//!
//! Everything an operation needs is worked out once, at build time, and kept on
//! a [`BoundOperation`]: which entity, which path capture holds its key, which
//! query parameters filter it, what its envelope looks like. A request then
//! costs a store read and a serialization, with no schema walking on the path.

use bytes::Bytes;
use http::StatusCode;
use lean_string::LeanString;
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::core::world::algebra::{Page, Selection};
use crate::core::world::model::{
    Cardinality, Carrier, EntityKey, EntityType, KeySource, ScalarKind, ValueSpec,
};
use crate::core::world::store::EntityStore;
use crate::core::world::store::Record;
use crate::core::world::store::values::{self, ValueSeed, generate};
use crate::core::{EntityQuery, World};
use crate::spec::bind::plan::RootPlan;
use crate::types::{DynamicResponse, RequestContext};

/// The window a list answers with when the request asked for none.
use crate::core::world::algebra::DEFAULT_PAGE_SIZE;

/// How many members of a to-many link are written into a payload.
///
/// A schema saying a folder has items means the payload carries some, not all
/// of them — and a mock that materialised every one would turn a 200-instance
/// world into a megabyte of JSON per request.
const EMBEDDED_LIST_LEN: usize = 3;

/// How deep links are followed when writing a payload.
///
/// One level is what a real payload does: a folder carries its parent, and the
/// parent carries a key rather than another folder. Without a cap, `parent`
/// pointing at `Folder` never stops.
const MAX_EXPAND_DEPTH: usize = 1;

/// How much of an API is answered from the store, and how much is invented.
#[derive(Debug, Default)]
pub struct Coverage {
    classified: Vec<String>,
    unclassified: Vec<String>,
    fallback_hits: AtomicU64,
}

impl Coverage {
    /// Operations answered from the store.
    #[must_use]
    pub fn classified(&self) -> &[String] {
        &self.classified
    }

    /// Operations answered from their declared response shape alone.
    #[must_use]
    pub fn unclassified(&self) -> &[String] {
        &self.unclassified
    }

    /// How many requests have been answered by the fallback so far.
    #[must_use]
    pub fn fallback_hits(&self) -> u64 {
        self.fallback_hits.load(Ordering::Relaxed)
    }

    /// The share of operations backed by the store, 0.0 to 1.0.
    #[must_use]
    pub fn ratio(&self) -> f64 {
        let total = self.classified.len() + self.unclassified.len();
        if total == 0 {
            return 1.0;
        }
        f64::from(u32::try_from(self.classified.len()).unwrap_or(u32::MAX))
            / f64::from(u32::try_from(total).unwrap_or(u32::MAX))
    }

    /// Count one operation as backed by the store or not.
    ///
    /// A viewer with nothing bound to it is not backed: the schema says a
    /// `User` comes back and nothing says which, so it answers from its
    /// declared shape like any unclassified operation. Counting it as
    /// classified would report a backend that answers `/me` when it does not.
    pub(super) fn record(&mut self, id: &str, plan: &RootPlan, viewer: Option<&LeanString>) {
        let answerable = match plan {
            RootPlan::Viewer { entity, members } => {
                viewer.is_some_and(|bound| bound == entity || members.contains(bound))
            }
            other => other.is_classified(),
        };
        if answerable {
            self.classified.push(id.to_string());
        } else {
            self.unclassified.push(id.to_string());
        }
    }
}

/// The parent a sub-resource list hangs off.
#[derive(Debug, Clone)]
pub struct ParentLink {
    pub entity: LeanString,
    /// The path parameter holding the parent's key.
    pub param: LeanString,
    /// The relation field on the parent holding the children.
    pub field: LeanString,
}

/// What one property of a list envelope holds.
#[derive(Debug, Clone)]
pub enum EnvelopeSlot {
    Records,
    Total,
    Limit,
    Offset,
    /// Anything else the envelope declared, generated from its shape.
    Declared(Arc<ValueSpec>),
}

/// The query parameters a list understands.
#[derive(Debug, Default, Clone)]
pub struct Pagination {
    pub limit: Vec<LeanString>,
    pub offset: Vec<LeanString>,
    pub page: Vec<LeanString>,
    pub sort: Vec<LeanString>,
}

impl Pagination {
    fn value_of<'a>(names: &[LeanString], query: &'a QueryMap) -> Option<&'a str> {
        names
            .iter()
            .find_map(|name| query.get(name.as_str()).map(String::as_str))
    }
}

/// A query value as the client meant it.
///
/// `RequestContext` keeps the query string raw, because a matcher comparing
/// raw values is what the rest of the engine does. A *filter* is different: it
/// is compared against a stored value, and `?name=Ada%20Lovelace` matching
/// nothing is the kind of bug that reads as "filtering is broken".
///
/// `+` is a space, the way `URLSearchParams` and every form-urlencoded parser
/// read it — which is also what `curl --data-urlencode` and most client
/// libraries emit. A value that means a literal plus sends `%2B`, and the
/// substitution runs first so that still decodes to one.
fn decoded(value: &str) -> String {
    let spaced = value.replace('+', " ");
    match urlencoding::decode(&spaced) {
        Ok(decoded) => decoded.into_owned(),
        Err(_) => spaced,
    }
}

type QueryMap = rustc_hash::FxHashMap<String, String>;

/// One operation, ready to answer.
pub struct BoundOperation {
    pub id: LeanString,
    pub method: http::Method,
    pub path: LeanString,
    pub summary: Option<LeanString>,
    pub plan: RootPlan,
    pub status: StatusCode,
    pub content_type: LeanString,
    pub(super) world: Arc<World>,
    pub(super) coverage: Arc<Coverage>,
    /// The response shape, for the rung that answers from it alone.
    pub(super) declared: Option<Arc<ValueSpec>>,
    /// Property name to what it holds, when a list response is wrapped.
    pub(super) envelope: Vec<(LeanString, EnvelopeSlot)>,
    pub(super) parent: Option<ParentLink>,
    pub(super) pagination: Pagination,
    /// Query parameters naming a field of the entity, with the kind to read
    /// their values as — a query string is all strings, and a store comparing
    /// `"5"` against `5` matches nothing.
    pub(super) filterable: Vec<(LeanString, ScalarKind)>,
}

impl BoundOperation {
    /// Answer one request.
    pub fn answer(&self, ctx: &RequestContext) -> DynamicResponse {
        let world = &self.world;
        match &self.plan {
            RootPlan::Get {
                entity,
                members,
                key_arg,
            } => self.answer_get(entity, members, key_arg, ctx),

            RootPlan::Viewer { entity, members } => self.answer_viewer(entity, members, ctx),

            RootPlan::List { entity, .. } => self.answer_list(entity, ctx),

            RootPlan::Create { entity, .. } => {
                let values = self.input_values(ctx);
                match world.create(entity.as_str(), values) {
                    Ok(record) => self.ok(&self.wrap_one(entity, record)),
                    Err(error) => Self::failed(StatusCode::BAD_REQUEST, &error.to_string()),
                }
            }

            RootPlan::Update {
                entity, key_arg, ..
            } => {
                let Some(key) = self.addressed_text(entity, key_arg, ctx) else {
                    return Self::failed(
                        StatusCode::BAD_REQUEST,
                        &format!("`{key_arg}` is missing from the path"),
                    );
                };
                let key = &key;
                let values = self.input_values(ctx);
                // PUT replaces, PATCH merges. The two are the same call with a
                // different mutation, which is the whole difference.
                let written = if self.method == http::Method::PUT {
                    world.replace(entity.as_str(), key, values)
                } else {
                    world.update(entity.as_str(), key, values)
                };
                match written {
                    Ok(record) => self.ok(&self.wrap_one(entity, record)),
                    // A write the world's own state refuses is a conflict, not
                    // a missing record: the record is right there, and what it
                    // holds is the reason the write cannot land.
                    Err(crate::FerrimockError::Conflict(why)) => {
                        Self::failed(StatusCode::CONFLICT, &why)
                    }
                    Err(_) => Self::not_found(entity, key),
                }
            }

            RootPlan::Delete {
                entity, key_arg, ..
            } => {
                let Some(key) = self.addressed_text(entity, key_arg, ctx) else {
                    return Self::failed(
                        StatusCode::BAD_REQUEST,
                        &format!("`{key_arg}` is missing from the path"),
                    );
                };
                let key = &key;
                // The removed record is the useful answer, so read it before it
                // stops being readable.
                let removed = world.get(entity.as_str(), key);
                if removed.is_none() {
                    return Self::not_found(entity, key);
                }
                match world.delete(entity.as_str(), key) {
                    Ok(()) if self.status == StatusCode::NO_CONTENT => DynamicResponse {
                        status: Some(self.status),
                        body: Bytes::new(),
                        ..DynamicResponse::default()
                    },
                    Ok(()) => {
                        let expanded = removed
                            .map_or(JsonValue::Null, |record| self.expand(entity, record, 0));
                        self.ok(&self.wrap_payload(expanded))
                    }
                    Err(error) => Self::failed(StatusCode::CONFLICT, &error.to_string()),
                }
            }

            RootPlan::Unclassified => {
                self.coverage.fallback_hits.fetch_add(1, Ordering::Relaxed);
                self.ok(&self.declared_value(ctx))
            }
        }
    }

    /// Answer `/me` as the caller rather than as record zero.
    ///
    /// Which instance a credential is comes from the credential, so the same
    /// token is the same person on every request and two tokens are two
    /// people. With no viewer declared the endpoint is not answerable at all:
    /// the schema says a `User` comes back and nothing says which, so it is
    /// counted as unclassified rather than answered wrongly.
    fn answer_viewer(
        &self,
        entity: &LeanString,
        members: &[LeanString],
        ctx: &RequestContext,
    ) -> DynamicResponse {
        use crate::core::world::viewer::{Credential, challenge};

        let store = self.world.store();
        let targets = concrete_or(entity, members);
        let Some(bound) = self.world.viewer().filter(|held| targets.contains(held)) else {
            self.coverage.fallback_hits.fetch_add(1, Ordering::Relaxed);
            return self.ok(&self.declared_value(ctx));
        };

        let credential = Credential::read(&ctx.headers);
        if credential == Credential::Absent {
            let mut refused = Self::failed(StatusCode::UNAUTHORIZED, "no credential was presented");
            refused.headers = Some(
                std::iter::once(("www-authenticate".to_string(), challenge().to_string()))
                    .collect(),
            );
            return refused;
        }

        let keys = store.keys(bound.as_str());
        let record = credential
            .bound_to(store.seed(), bound.as_str(), &keys)
            .and_then(|key| store.get(bound.as_str(), &key));
        match record {
            Some(record) => {
                let held = record.entity.clone();
                let value = self.expand_with(&store, &held, record, 0);
                self.ok(&self.wrap_payload(value))
            }
            None => Self::failed(StatusCode::NOT_FOUND, "the credential names nobody"),
        }
    }

    fn answer_get(
        &self,
        entity: &LeanString,
        members: &[LeanString],
        key_arg: &LeanString,
        ctx: &RequestContext,
    ) -> DynamicResponse {
        let store = self.world.store();
        let targets = concrete_or(entity, members);

        let record = if key_arg.is_empty() {
            targets.iter().find_map(|target| {
                store
                    .keys(target.as_str())
                    .first()
                    .and_then(|key| store.get(target.as_str(), key))
            })
        } else {
            if !ctx.captures.contains_key(key_arg.as_str()) {
                return Self::failed(
                    StatusCode::BAD_REQUEST,
                    &format!("`{key_arg}` is missing from the path"),
                );
            }
            targets.iter().find_map(|target| {
                let key = Self::addressed_key(&store, target.as_str(), key_arg, ctx)?;
                store.get(target.as_str(), &key)
            })
        };

        match record {
            Some(record) => {
                let entity = record.entity.clone();
                let value = self.expand_with(&store, &entity, record, 0);
                self.ok(&self.wrap_payload(value))
            }
            None => Self::not_found(
                entity,
                &self
                    .addressed_text(entity, key_arg, ctx)
                    .unwrap_or_default(),
            ),
        }
    }

    fn answer_list(&self, entity: &LeanString, ctx: &RequestContext) -> DynamicResponse {
        let query = self.query_of(ctx);

        let page = match &self.parent {
            Some(parent) => {
                let Some(key) = ctx.captures.get(parent.param.as_str()) else {
                    return Self::failed(
                        StatusCode::BAD_REQUEST,
                        &format!("`{}` is missing from the path", parent.param),
                    );
                };
                self.world
                    .related(parent.entity.as_str(), key, parent.field.as_str(), &query)
            }
            None => self.world.list(entity.as_str(), &query),
        };

        let page = match page {
            Ok(page) => page,
            Err(error) => return Self::failed(StatusCode::BAD_REQUEST, &error.to_string()),
        };

        let store = self.world.store();
        let records: Vec<JsonValue> = page
            .records
            .iter()
            .map(|value| self.expand_object(&store, entity, value.clone(), 0))
            .collect();

        if self.envelope.is_empty() {
            return self.ok(&JsonValue::Array(records));
        }

        let mut wrapper = JsonMap::new();
        let mut records = Some(records);
        for (name, slot) in &self.envelope {
            let value = match slot {
                EnvelopeSlot::Records => records
                    .take()
                    .map_or_else(|| JsonValue::Array(Vec::new()), JsonValue::Array),
                EnvelopeSlot::Total => JsonValue::from(page.total),
                EnvelopeSlot::Limit => JsonValue::from(query.limit.unwrap_or(DEFAULT_PAGE_SIZE)),
                EnvelopeSlot::Offset => JsonValue::from(query.skip),
                EnvelopeSlot::Declared(spec) => generate(
                    spec,
                    name.as_str(),
                    ValueSeed::new(store.seed(), self.id.as_str(), 0),
                ),
            };
            wrapper.insert(name.to_string(), value);
        }
        self.ok(&JsonValue::Object(wrapper))
    }

    /// The key a request addresses, assembled from every path parameter the
    /// entity's key is made of.
    ///
    /// A key of one part is the ordinary case and reads straight off its
    /// capture. A key of several — `/repos/{owner}/{repo}` — is only fully
    /// addressed by all of them, and taking the last one alone would answer
    /// with whichever repo happened to be first.
    fn addressed_key(
        store: &Arc<EntityStore>,
        entity: &str,
        key_arg: &LeanString,
        ctx: &RequestContext,
    ) -> Option<EntityKey> {
        let graph = store.graph();
        let Some(definition) = graph.get(entity) else {
            return ctx
                .captures
                .get(key_arg.as_str())
                .map(|key| EntityKey::single(key.as_str()));
        };
        if definition.key.len() <= 1 {
            return ctx
                .captures
                .get(key_arg.as_str())
                .map(|key| EntityKey::single(key.as_str()));
        }

        let mut parts = Vec::with_capacity(definition.key.len());
        for part in definition.key.iter() {
            let capture = match &part.source {
                KeySource::PathParam(name) => ctx.captures.get(name.as_str()),
                KeySource::Field(name) => ctx
                    .captures
                    .get(name.as_str())
                    .or_else(|| ctx.captures.get(key_arg.as_str())),
            }?;
            parts.push(LeanString::from(capture.as_str()));
        }
        Some(EntityKey::from_parts(parts))
    }

    /// The key a write addresses, in the text form the world reads.
    fn addressed_text(
        &self,
        entity: &LeanString,
        key_arg: &LeanString,
        ctx: &RequestContext,
    ) -> Option<String> {
        let store = self.world.store();
        Self::addressed_key(&store, entity.as_str(), key_arg, ctx).map(|key| key.to_string())
    }

    /// What a request body says to write.
    fn input_values(&self, ctx: &RequestContext) -> JsonValue {
        let Some(body) = ctx.body.as_deref() else {
            return JsonValue::Object(JsonMap::new());
        };
        let parsed: JsonValue = match serde_json::from_str(body) {
            Ok(value) => value,
            Err(_) => return JsonValue::Object(JsonMap::new()),
        };

        // A body that wraps the entity (`{ "folder": { … } }`) means the
        // wrapper's contents, not the wrapper.
        let envelope = match &self.plan {
            RootPlan::Create { input_arg, .. } | RootPlan::Update { input_arg, .. } => {
                input_arg.as_deref()
            }
            _ => None,
        };
        match envelope {
            Some(field) => parsed.get(field).cloned().unwrap_or(parsed),
            None => parsed,
        }
    }

    /// Read the request's query string as a store query.
    fn query_of(&self, ctx: &RequestContext) -> EntityQuery {
        let mut query = EntityQuery {
            limit: Some(DEFAULT_PAGE_SIZE),
            ..EntityQuery::default()
        };

        if let Some(limit) = Pagination::value_of(&self.pagination.limit, &ctx.query)
            && let Ok(limit) = limit.parse::<usize>()
        {
            query.limit = Some(limit);
        }
        if let Some(offset) = Pagination::value_of(&self.pagination.offset, &ctx.query)
            && let Ok(offset) = offset.parse::<usize>()
        {
            query.skip = offset;
        } else if let Some(page) = Pagination::value_of(&self.pagination.page, &ctx.query)
            && let Ok(page) = page.parse::<usize>()
        {
            // Pages are 1-based wherever they are offered. A page number past
            // what the offset can hold saturates rather than wrapping: the
            // request is nonsense either way, and a client must not be able to
            // decide whether a worker panics.
            query.skip = page
                .saturating_sub(1)
                .saturating_mul(query.limit.unwrap_or(DEFAULT_PAGE_SIZE));
        }
        if let Some(sort) = Pagination::value_of(&self.pagination.sort, &ctx.query) {
            query.sort = decoded(sort)
                .split(',')
                .map(str::trim)
                .map(sort_key)
                .collect();
        }

        for (name, value) in &ctx.query {
            // `size[gt]=5` is a spelling of the operator syntax the world
            // already takes, not a filter language of its own.
            let (field, operator) = match name.split_once('[') {
                Some((field, rest)) => (field, rest.strip_suffix(']')),
                None => (name.as_str(), None),
            };
            let Some((_, kind)) = self
                .filterable
                .iter()
                .find(|(candidate, _)| candidate == field)
            else {
                continue;
            };
            let typed = typed_value(&decoded(value), kind);
            let entry = match operator {
                Some(operator) => {
                    let mut wrapper = JsonMap::new();
                    wrapper.insert(operator.to_string(), typed);
                    JsonValue::Object(wrapper)
                }
                None => typed,
            };
            query.filter.insert(field.to_string(), entry);
        }

        query
    }

    /// A record as a payload, with the links the schema declared written out.
    fn expand(&self, entity: &str, record: JsonValue, depth: usize) -> JsonValue {
        let store = self.world.store();
        self.expand_object(&store, entity, record, depth)
    }

    fn expand_with(
        &self,
        store: &Arc<EntityStore>,
        entity: &str,
        record: Record,
        depth: usize,
    ) -> JsonValue {
        self.expand_object(store, entity, JsonValue::Object(record.fields), depth)
    }

    fn expand_object(
        &self,
        store: &Arc<EntityStore>,
        entity: &str,
        record: JsonValue,
        depth: usize,
    ) -> JsonValue {
        let JsonValue::Object(mut fields) = record else {
            return record;
        };
        let graph = store.graph();
        let Some(definition) = graph.get(entity) else {
            return JsonValue::Object(fields);
        };
        let Some(key) = key_of(definition, &fields) else {
            return JsonValue::Object(fields);
        };

        for (field, relation) in definition.relations() {
            match &relation.carrier {
                // The field already holds the target's key, which is what the
                // schema said it holds. A carrier naming a *different* field is
                // the other case: the key lives on the sibling, and this field
                // is the object the document declared.
                Carrier::ForeignKey(_) if relation.carrier.is_inline_key(&field.name) => continue,
                // The sub-path serves these; they were never in the payload.
                Carrier::Subresource(_) => {
                    fields.remove(field.name.as_str());
                    continue;
                }
                Carrier::ForeignKey(_) | Carrier::Embedded | Carrier::Connection(_) => {}
            }

            if depth >= MAX_EXPAND_DEPTH {
                // The schema said this field holds an object, so it holds one
                // whatever the depth cap decides — carrying only the key, the
                // way real APIs return a mini representation. Leaving the raw
                // key here would make the field a string at one depth and an
                // object at another, which is worse than either.
                let carrier = relation.carrier.key_field(&field.name);
                let value = fields
                    .get(carrier.as_str())
                    .and_then(key_text)
                    .map(|key| mini_representation(graph, relation, &key));
                match (value, relation.cardinality) {
                    (Some(value), Cardinality::One) => {
                        fields.insert(field.name.to_string(), value);
                    }
                    (_, Cardinality::Many) => {
                        fields.insert(field.name.to_string(), JsonValue::Array(Vec::new()));
                    }
                    (None, Cardinality::One) => {}
                }
                continue;
            }

            let take = if relation.cardinality == Cardinality::One {
                1
            } else {
                EMBEDDED_LIST_LEN
            };
            let selection = Selection {
                page: Page::Offset { skip: 0, take },
                ..Selection::new()
            };
            let Ok(page) = store.related(entity, &key, field.name.as_str(), &selection) else {
                continue;
            };

            let mut linked: Vec<JsonValue> = page
                .records
                .into_iter()
                .map(|record| {
                    let target = record.entity.clone();
                    self.expand_with(store, target.as_str(), record, depth + 1)
                })
                .collect();

            let value = if relation.cardinality == Cardinality::One {
                linked.pop().unwrap_or(JsonValue::Null)
            } else {
                JsonValue::Array(linked)
            };
            fields.insert(field.name.to_string(), value);
        }

        JsonValue::Object(fields)
    }

    fn wrap_one(&self, entity: &LeanString, record: JsonValue) -> JsonValue {
        self.wrap_payload(self.expand(entity.as_str(), record, 0))
    }

    /// Put a single answer back inside the envelope the response declared.
    fn wrap_payload(&self, value: JsonValue) -> JsonValue {
        let field = match &self.plan {
            RootPlan::Create { payload_field, .. }
            | RootPlan::Update { payload_field, .. }
            | RootPlan::Delete { payload_field, .. } => payload_field.as_deref(),
            RootPlan::Get { .. }
            | RootPlan::Viewer { .. }
            | RootPlan::List { .. }
            | RootPlan::Unclassified => None,
        };
        match field {
            Some(field) => {
                let mut wrapper = JsonMap::new();
                wrapper.insert(field.to_string(), value);
                JsonValue::Object(wrapper)
            }
            None => value,
        }
    }

    /// An operation nothing could be inferred about still has to answer, and
    /// it answers from the shape its own document declared.
    fn declared_value(&self, ctx: &RequestContext) -> JsonValue {
        let Some(declared) = &self.declared else {
            return JsonValue::Object(JsonMap::new());
        };
        // Seeded by the path actually requested, so the same call answers the
        // same way twice and two different ones do not collide.
        let ordinal = path_ordinal(&ctx.path);
        generate(
            declared,
            self.id.as_str(),
            ValueSeed::new(self.world.seed(), self.id.as_str(), ordinal),
        )
    }

    fn ok(&self, body: &JsonValue) -> DynamicResponse {
        DynamicResponse {
            status: Some(self.status),
            body: Bytes::from(body.to_string()),
            ..DynamicResponse::default()
        }
    }

    fn not_found(entity: &str, key: &str) -> DynamicResponse {
        Self::failed(
            StatusCode::NOT_FOUND,
            &format!("no {entity} with key `{key}`"),
        )
    }

    /// A failure the world produced, in a shape a client can parse.
    ///
    /// Deliberately generic: an API's own error envelope is its own, and the
    /// way to serve that one is an ordinary mock at a higher priority.
    fn failed(status: StatusCode, message: &str) -> DynamicResponse {
        let body = serde_json::json!({
            "error": { "status": status.as_u16(), "message": message }
        });
        DynamicResponse {
            status: Some(status),
            body: Bytes::from(body.to_string()),
            ..DynamicResponse::default()
        }
    }
}

/// The smallest honest object for a link the expansion stopped at: its key.
fn mini_representation(
    graph: &crate::core::world::model::EntityGraph,
    relation: &crate::core::world::model::Relation,
    key: &str,
) -> JsonValue {
    let target = graph.get(relation.target.as_str());
    let field = target
        .and_then(|entity| entity.key.as_single().cloned())
        .unwrap_or_else(|| LeanString::from("id"));
    let kind = target.and_then(key_kind_of).unwrap_or(ScalarKind::String);

    let mut object = JsonMap::new();
    object.insert(field.to_string(), values::key_json(&kind, key));
    JsonValue::Object(object)
}

/// The kind an entity's key field was declared as.
fn key_kind_of(entity: &EntityType) -> Option<ScalarKind> {
    let field = entity.key.as_single()?;
    match &entity.field(field.as_str())?.value {
        ValueSpec::Scalar(scalar) => Some(scalar.kind.clone()),
        _ => None,
    }
}

/// A key as text, however the payload wrote it.
fn key_text(value: &JsonValue) -> Option<String> {
    values::key_text(value)
}

fn concrete_or(entity: &LeanString, members: &[LeanString]) -> Vec<LeanString> {
    if members.is_empty() {
        vec![entity.clone()]
    } else {
        members.to_vec()
    }
}

fn key_of(entity: &EntityType, fields: &JsonMap<String, JsonValue>) -> Option<EntityKey> {
    let field = entity.key.as_single()?;
    match fields.get(field.as_str())? {
        JsonValue::String(value) => Some(EntityKey::single(value.as_str())),
        other => Some(EntityKey::single(other.to_string())),
    }
}

fn sort_key(field: &str) -> String {
    // `-name` and `name:desc` are both spellings people reach for; the world
    // takes the first, so the second is translated rather than rejected.
    match field.split_once(':') {
        Some((name, "desc" | "DESC")) => format!("-{name}"),
        Some((name, _)) => name.to_string(),
        None => field.to_string(),
    }
}

/// A query value read as the kind the field actually holds.
fn typed_value(value: &str, kind: &ScalarKind) -> JsonValue {
    match kind {
        ScalarKind::Int => value
            .parse::<i64>()
            .map_or_else(|_| JsonValue::String(value.to_string()), JsonValue::from),
        ScalarKind::Float => value
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(|| JsonValue::String(value.to_string()), JsonValue::Number),
        ScalarKind::Boolean => match value {
            "true" | "1" => JsonValue::Bool(true),
            "false" | "0" => JsonValue::Bool(false),
            other => JsonValue::String(other.to_string()),
        },
        ScalarKind::String | ScalarKind::Id | ScalarKind::Custom(_) => {
            JsonValue::String(value.to_string())
        }
    }
}

/// A stable ordinal for a request path, so an unclassified operation answers
/// the same way for the same path and differently for a different one.
fn path_ordinal(path: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    path.hash(&mut hasher);
    hasher.finish()
}

/// Whether an entity's field is worth offering as a filter.
///
/// A foreign key is compared against a stored key, so it is read as the kind
/// the *target's* key was declared as — filtering `?user_id=5` against a world
/// holding `5` and one holding `"5"` are different questions, and only the
/// schema knows which was asked.
#[must_use]
pub fn filterable_fields(
    entity: &EntityType,
    graph: &crate::core::world::model::EntityGraph,
) -> Vec<(LeanString, ScalarKind)> {
    entity
        .fields
        .iter()
        .filter_map(|field| match &field.value {
            ValueSpec::Scalar(scalar) => Some((field.name.clone(), scalar.kind.clone())),
            ValueSpec::Enum(_) => Some((field.name.clone(), ScalarKind::String)),
            ValueSpec::Relation(relation) => {
                // A link whose key is carried by a sibling is filterable
                // through that sibling, which is a scalar field in its own
                // right; offering the object's name as well would invite a
                // filter that can never match.
                if !relation.carrier.is_inline_key(&field.name) {
                    return None;
                }
                let kind = graph
                    .get(relation.target.as_str())
                    .and_then(key_kind_of)
                    .unwrap_or(ScalarKind::String);
                Some((field.name.clone(), kind))
            }
            _ => None,
        })
        .collect()
}
