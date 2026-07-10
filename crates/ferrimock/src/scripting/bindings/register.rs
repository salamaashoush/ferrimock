//! `http.*` / `graphql.*` — the MSW-style registration surface scripts
//! call at module evaluation time.
//!
//! Each call persists the handler function into the VM's
//! [`crate::scripting::slots`] registry and records a spec; the loader
//! drains the specs after evaluation and builds `MockDefinition`s.

// rquickjs `Func` targets must take FromJs params owned and the
// injected `Ctx` by value.
#![allow(clippy::needless_pass_by_value)]

use rquickjs::function::{Func, Opt, This};
use rquickjs::{Ctx, Function, Object, Persistent, Value};

use crate::scripting::slots::{
    ScriptGraphQLName, ScriptGraphQLOp, ScriptMockKind, ScriptMockSpec, ScriptPath, with_slots,
};

pub(super) fn parse_once(options: Option<Object<'_>>) -> rquickjs::Result<bool> {
    match options {
        Some(obj) => Ok(obj.get::<_, Option<bool>>("once")?.unwrap_or(false)),
        None => Ok(false),
    }
}

/// JS RegExp flags filtered to the set the regex crate honors inline.
fn matching_flags(flags: &str) -> String {
    flags.chars().filter(|c| "ims".contains(*c)).collect()
}

/// Extract `(source, flags)` when the value is a JS RegExp (detected by
/// its string `source` property, same as the NAPI binding).
fn as_regexp(value: &Value<'_>) -> rquickjs::Result<Option<(String, String)>> {
    let Some(obj) = value.as_object() else {
        return Ok(None);
    };
    let Ok(source) = obj.get::<_, String>("source") else {
        return Ok(None);
    };
    let flags: String = obj.get::<_, Option<String>>("flags")?.unwrap_or_default();
    Ok(Some((source, matching_flags(&flags))))
}

/// Accepts a pattern string or a JS RegExp.
pub(super) fn parse_path(path: Value<'_>) -> rquickjs::Result<ScriptPath> {
    if let Some(s) = path.as_string() {
        return Ok(ScriptPath::Pattern(s.to_string()?));
    }
    if let Some((source, flags)) = as_regexp(&path)? {
        return Ok(ScriptPath::Regex { source, flags });
    }
    Err(rquickjs::Error::new_from_js_message(
        "ferrimock",
        "TypeError",
        "path must be a string or RegExp".to_string(),
    ))
}

/// Whether the resolver is a (async) generator function, detected from
/// its constructor name. Recorded at registration; the bridge drives the
/// iterator per request (MSW generator-resolver semantics).
fn is_generator_fn(handler: &Function<'_>) -> bool {
    handler.as_object().is_some_and(|obj| {
        obj.get::<_, Object<'_>>("constructor")
            .and_then(|ctor| ctor.get::<_, String>("name"))
            .is_ok_and(|name| name == "GeneratorFunction" || name == "AsyncGeneratorFunction")
    })
}

fn register_http<'js>(
    ctx: &Ctx<'js>,
    method: Option<&str>,
    path: Value<'js>,
    handler: Function<'js>,
    options: Option<Object<'js>>,
) -> rquickjs::Result<()> {
    let path = parse_path(path)?;
    let once = parse_once(options)?;
    let is_generator = is_generator_fn(&handler);
    let persistent = Persistent::save(ctx, handler);
    with_slots(ctx, |slots| {
        let slot = slots.insert(persistent);
        slots.push_spec(ScriptMockSpec {
            kind: ScriptMockKind::Http {
                method: method.map(str::to_string),
                path,
            },
            slot,
            once,
            is_generator,
        });
    })
}

macro_rules! http_method_fn {
    ($name:ident, $method:expr) => {
        fn $name<'js>(
            ctx: Ctx<'js>,
            path: Value<'js>,
            handler: Function<'js>,
            options: Opt<Object<'js>>,
        ) -> rquickjs::Result<()> {
            register_http(&ctx, $method, path, handler, options.0)
        }
    };
}

http_method_fn!(http_get, Some("GET"));
http_method_fn!(http_post, Some("POST"));
http_method_fn!(http_put, Some("PUT"));
http_method_fn!(http_delete, Some("DELETE"));
http_method_fn!(http_patch, Some("PATCH"));
http_method_fn!(http_head, Some("HEAD"));
http_method_fn!(http_options, Some("OPTIONS"));
http_method_fn!(http_all, None);

/// GraphQL operation predicate: a name string or a JS RegExp.
fn parse_operation_name(name: Value<'_>) -> rquickjs::Result<ScriptGraphQLName> {
    if let Some(s) = name.as_string() {
        return Ok(ScriptGraphQLName::Exact(s.to_string()?));
    }
    if let Some((source, flags)) = as_regexp(&name)? {
        return Ok(ScriptGraphQLName::Regex { source, flags });
    }
    Err(rquickjs::Error::new_from_js_message(
        "ferrimock",
        "TypeError",
        "operation name must be a string or RegExp".to_string(),
    ))
}

fn register_graphql<'js>(
    ctx: &Ctx<'js>,
    operation_type: ScriptGraphQLOp,
    operation_name: Option<ScriptGraphQLName>,
    endpoint: Option<String>,
    handler: Function<'js>,
    options: Option<Object<'js>>,
) -> rquickjs::Result<()> {
    let once = parse_once(options)?;
    let is_generator = is_generator_fn(&handler);
    let persistent = Persistent::save(ctx, handler);
    with_slots(ctx, |slots| {
        let slot = slots.insert(persistent);
        slots.push_spec(ScriptMockSpec {
            kind: ScriptMockKind::GraphQL {
                operation_type,
                operation_name,
                endpoint,
            },
            slot,
            once,
            is_generator,
        });
    })
}

fn graphql_query<'js>(
    ctx: Ctx<'js>,
    name: Value<'js>,
    handler: Function<'js>,
    options: Opt<Object<'js>>,
) -> rquickjs::Result<()> {
    let name = parse_operation_name(name)?;
    register_graphql(
        &ctx,
        ScriptGraphQLOp::Query,
        Some(name),
        None,
        handler,
        options.0,
    )
}

fn graphql_mutation<'js>(
    ctx: Ctx<'js>,
    name: Value<'js>,
    handler: Function<'js>,
    options: Opt<Object<'js>>,
) -> rquickjs::Result<()> {
    let name = parse_operation_name(name)?;
    register_graphql(
        &ctx,
        ScriptGraphQLOp::Mutation,
        Some(name),
        None,
        handler,
        options.0,
    )
}

fn graphql_operation<'js>(
    ctx: Ctx<'js>,
    handler: Function<'js>,
    options: Opt<Object<'js>>,
) -> rquickjs::Result<()> {
    register_graphql(&ctx, ScriptGraphQLOp::Any, None, None, handler, options.0)
}

/// `graphql.link(url)` — endpoint-scoped registration. The endpoint
/// becomes a real matcher (host + path) on the built mock, so scoping
/// happens in the Rust matcher, not in the resolver. The scoped methods
/// read the endpoint from `this`, so call them on the link object
/// (`const api = graphql.link(url); api.query(...)`).
fn link_endpoint(ctx: &Ctx<'_>, this: &Object<'_>) -> rquickjs::Result<String> {
    this.get::<_, Option<String>>("__ferrimockEndpoint")?
        .ok_or_else(|| {
            rquickjs::Exception::throw_type(
                ctx,
                "graphql.link handlers must be called on the link object (api.query(...), not a detached reference)",
            )
        })
}

fn link_query<'js>(
    ctx: Ctx<'js>,
    this: This<Object<'js>>,
    name: Value<'js>,
    handler: Function<'js>,
    options: Opt<Object<'js>>,
) -> rquickjs::Result<()> {
    let endpoint = link_endpoint(&ctx, &this.0)?;
    let name = parse_operation_name(name)?;
    register_graphql(
        &ctx,
        ScriptGraphQLOp::Query,
        Some(name),
        Some(endpoint),
        handler,
        options.0,
    )
}

fn link_mutation<'js>(
    ctx: Ctx<'js>,
    this: This<Object<'js>>,
    name: Value<'js>,
    handler: Function<'js>,
    options: Opt<Object<'js>>,
) -> rquickjs::Result<()> {
    let endpoint = link_endpoint(&ctx, &this.0)?;
    let name = parse_operation_name(name)?;
    register_graphql(
        &ctx,
        ScriptGraphQLOp::Mutation,
        Some(name),
        Some(endpoint),
        handler,
        options.0,
    )
}

fn link_operation<'js>(
    ctx: Ctx<'js>,
    this: This<Object<'js>>,
    handler: Function<'js>,
    options: Opt<Object<'js>>,
) -> rquickjs::Result<()> {
    let endpoint = link_endpoint(&ctx, &this.0)?;
    register_graphql(
        &ctx,
        ScriptGraphQLOp::Any,
        None,
        Some(endpoint),
        handler,
        options.0,
    )
}

fn graphql_link(ctx: Ctx<'_>, url: String) -> rquickjs::Result<Object<'_>> {
    let scoped = Object::new(ctx)?;
    scoped.set("__ferrimockEndpoint", url)?;
    scoped.set("query", Func::from(link_query))?;
    scoped.set("mutation", Func::from(link_mutation))?;
    scoped.set("operation", Func::from(link_operation))?;
    Ok(scoped)
}

pub fn install(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let http = Object::new(ctx.clone())?;
    http.set("get", Func::from(http_get))?;
    http.set("post", Func::from(http_post))?;
    http.set("put", Func::from(http_put))?;
    http.set("delete", Func::from(http_delete))?;
    http.set("patch", Func::from(http_patch))?;
    http.set("head", Func::from(http_head))?;
    http.set("options", Func::from(http_options))?;
    http.set("all", Func::from(http_all))?;
    ctx.globals().set("http", http)?;

    let graphql = Object::new(ctx.clone())?;
    graphql.set("query", Func::from(graphql_query))?;
    graphql.set("mutation", Func::from(graphql_mutation))?;
    graphql.set("operation", Func::from(graphql_operation))?;
    graphql.set("link", Func::from(graphql_link))?;
    ctx.globals().set("graphql", graphql)?;
    Ok(())
}
