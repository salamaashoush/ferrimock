//! `world.*` — a script reading and writing the engine's entity world.
//!
//! The same store a schema-derived route serves and a template reads, so a
//! handler that creates a user is answering the same question the spec's
//! `users` query answers. That is the whole reason the world is an engine
//! concept and not something a spec owns.
//!
//! Every call is synchronous: it is a `DashMap` read behind an `Arc`, so it
//! stays on the handler fast path with nothing to await.

// rquickjs `Func` targets must take FromJs params owned and the injected `Ctx`
// by value.
#![allow(clippy::needless_pass_by_value)]

use rquickjs::function::{Func, Opt};
use rquickjs::{Ctx, Object, Value};
use serde_json::Value as JsonValue;

use crate::core::{EntityQuery, global_world};

fn throw(message: impl std::fmt::Display) -> rquickjs::Error {
    rquickjs::Error::new_from_js_message("ferrimock", "Error", message.to_string())
}

fn to_js<'js>(ctx: &Ctx<'js>, value: &JsonValue) -> rquickjs::Result<Value<'js>> {
    super::convert::json_to_js(ctx, value)
}

/// Read a JS value as JSON.
///
/// Straight through `rquickjs_serde`, unlike the outbound direction: the
/// private-number token is only ever produced by serde_json's *Serialize*
/// impl, so reading into Rust never meets it.
fn from_js(value: Option<Value<'_>>, what: &str) -> rquickjs::Result<JsonValue> {
    match value.filter(|v| !v.is_undefined() && !v.is_null()) {
        None => Ok(JsonValue::Object(serde_json::Map::new())),
        Some(v) => match rquickjs_serde::from_value(v).map_err(throw)? {
            object @ JsonValue::Object(_) => Ok(object),
            _ => Err(throw(format!("{what} has to be an object"))),
        },
    }
}

/// `world.list(type, { filter, sort, skip, limit })`
fn query_from(options: Option<Value<'_>>) -> rquickjs::Result<EntityQuery> {
    let JsonValue::Object(options) = from_js(options, "options")? else {
        return Ok(EntityQuery::default());
    };

    let filter = match options.get("filter") {
        Some(JsonValue::Object(map)) => map.clone(),
        None | Some(JsonValue::Null) => serde_json::Map::new(),
        Some(_) => return Err(throw("`filter` has to be an object of field to value")),
    };

    // A string or an array of them, so `sort: "-createdAt"` and
    // `sort: ["kind", "-createdAt"]` both read naturally.
    let sort = match options.get("sort") {
        Some(JsonValue::String(one)) => vec![one.clone()],
        Some(JsonValue::Array(many)) => many
            .iter()
            .filter_map(|v| v.as_str().map(ToString::to_string))
            .collect(),
        _ => Vec::new(),
    };

    Ok(EntityQuery {
        filter,
        sort,
        skip: options
            .get("skip")
            .and_then(JsonValue::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(0),
        limit: options
            .get("limit")
            .and_then(JsonValue::as_u64)
            .and_then(|n| usize::try_from(n).ok()),
    })
}

fn world_types(ctx: Ctx<'_>) -> rquickjs::Result<Value<'_>> {
    let names: Vec<String> = global_world()
        .entities()
        .into_iter()
        .map(|name| name.to_string())
        .collect();
    to_js(&ctx, &serde_json::json!(names))
}

fn world_count(entity: String) -> usize {
    global_world().count(&entity)
}

fn world_get(ctx: Ctx<'_>, entity: String, key: String) -> rquickjs::Result<Value<'_>> {
    // `undefined` rather than an error: a miss is an ordinary answer, and a
    // handler wants `if (!user) return ...`.
    match global_world().get(&entity, &key) {
        Some(record) => to_js(&ctx, &record),
        None => Ok(Value::new_undefined(ctx)),
    }
}

fn world_list<'js>(
    ctx: Ctx<'js>,
    entity: String,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let query = query_from(options.0)?;
    let page = global_world().list(&entity, &query).map_err(throw)?;
    to_js(
        &ctx,
        &serde_json::json!({
            "records": page.records,
            "total": page.total,
            "hasNext": page.has_next,
            "hasPrevious": page.has_previous,
        }),
    )
}

fn world_related<'js>(
    ctx: Ctx<'js>,
    entity: String,
    key: String,
    field: String,
    options: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let query = query_from(options.0)?;
    let page = global_world()
        .related(&entity, &key, &field, &query)
        .map_err(throw)?;
    to_js(
        &ctx,
        &serde_json::json!({
            "records": page.records,
            "total": page.total,
            "hasNext": page.has_next,
            "hasPrevious": page.has_previous,
        }),
    )
}

fn world_create<'js>(
    ctx: Ctx<'js>,
    entity: String,
    values: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let values = from_js(values.0, "values")?;
    let created = global_world().create(&entity, values).map_err(throw)?;
    to_js(&ctx, &created)
}

fn world_update<'js>(
    ctx: Ctx<'js>,
    entity: String,
    key: String,
    values: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let values = from_js(values.0, "values")?;
    let updated = global_world()
        .update(&entity, &key, values)
        .map_err(throw)?;
    to_js(&ctx, &updated)
}

fn world_replace<'js>(
    ctx: Ctx<'js>,
    entity: String,
    key: String,
    values: Opt<Value<'js>>,
) -> rquickjs::Result<Value<'js>> {
    let values = from_js(values.0, "values")?;
    let replaced = global_world()
        .replace(&entity, &key, values)
        .map_err(throw)?;
    to_js(&ctx, &replaced)
}

fn world_delete(entity: String, key: String) -> rquickjs::Result<()> {
    global_world().delete(&entity, &key).map_err(throw)
}

fn world_reset() {
    global_world().reset();
}

pub fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let world = Object::new(ctx.clone())?;
    world.set("types", Func::from(world_types))?;
    world.set("count", Func::from(world_count))?;
    world.set("get", Func::from(world_get))?;
    world.set("list", Func::from(world_list))?;
    world.set("related", Func::from(world_related))?;
    world.set("create", Func::from(world_create))?;
    world.set("update", Func::from(world_update))?;
    world.set("replace", Func::from(world_replace))?;
    world.set("delete", Func::from(world_delete))?;
    world.set("reset", Func::from(world_reset))?;
    ctx.globals().set("world", world)?;
    Ok(())
}
