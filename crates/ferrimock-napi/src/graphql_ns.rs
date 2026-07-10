//! GraphQL namespace bindings: `graphql.query()`, `graphql.mutation()`, etc.
//!
//! Handlers receive `GraphQLRequestInfo` (MSW's `{ query, variables,
//! operationName, cookies, request, requestId }`). Operation names accept
//! a string or a RegExp. `graphql.link(url)` is composed in ferrimock
//! (it returns a scoped namespace object); the endpoint scoping itself is
//! expressed through these factories via the `endpoint` parameter.

use crate::handler_bridge::js_to_handler_bridge;
use crate::http_ns::{
    RequestHandler, RequestHandlerOptions, as_regexp, compile_js_regex, finish_handler,
};
use crate::request_context::{GraphQLRequestInfo, HandlerKind};
use crate::types::HandlerResponse;
use ferrimock::handler;
use ferrimock::types::{GraphQLOperationType, MockDefinition};
use napi::bindgen_prelude::*;
use napi_derive::napi;

fn build_graphql(
    env: &Env,
    op_type: Option<GraphQLOperationType>,
    operation_name: Option<Unknown>,
    endpoint: Option<String>,
    handler_fn: Function<'_, GraphQLRequestInfo, Promise<Option<HandlerResponse>>>,
    options: Option<RequestHandlerOptions>,
) -> Result<RequestHandler> {
    let bridge = js_to_handler_bridge(handler_fn, HandlerKind::GraphQL)?;

    let (mut mock_def, display): (MockDefinition, String) = match (op_type, operation_name) {
        (Some(op_type), Some(name)) => {
            let label = match op_type {
                GraphQLOperationType::Mutation => "mutation",
                _ => "query",
            };
            match as_regexp(env, &name)? {
                Some((source, flags)) => {
                    let regex = compile_js_regex(&source, &flags)?;
                    let display = format!("{label} /{source}/{flags}");
                    let mock = match op_type {
                        GraphQLOperationType::Mutation => {
                            handler::graphql::mutation_regex(regex, bridge.handler_fn.clone())
                        }
                        _ => handler::graphql::query_regex(regex, bridge.handler_fn.clone()),
                    };
                    (mock, display)
                }
                None => {
                    #[allow(unsafe_code)]
                    let name_str: String =
                        unsafe { FromNapiValue::from_napi_value(env.raw(), name.raw())? };
                    let display = format!("{label} {name_str}");
                    let mock = match op_type {
                        GraphQLOperationType::Mutation => {
                            handler::graphql::mutation(&name_str, bridge.handler_fn.clone())
                        }
                        _ => handler::graphql::query(&name_str, bridge.handler_fn.clone()),
                    };
                    (mock, display)
                }
            }
        }
        _ => (
            handler::graphql::operation(bridge.handler_fn.clone()),
            "all".to_string(),
        ),
    };

    // MSW's GraphQLHandler header shape: "query GetUser (origin: *)".
    let origin = endpoint.as_deref().unwrap_or("*");
    let pattern = format!("{display} (origin: {origin})");

    if let Some(endpoint) = endpoint {
        apply_endpoint(&mut mock_def, &endpoint);
    }

    Ok(finish_handler(bridge, mock_def, options, Some(pattern)))
}

/// Scope a `graphql.link` mock to its endpoint: exact path pattern plus a
/// Host-header matcher for absolute URLs.
fn apply_endpoint(mock: &mut MockDefinition, endpoint: &str) {
    use ferrimock::types::{HeaderMatcher, UrlPattern};
    use smallvec::SmallVec;

    let (host, path) = match UrlPattern::split_absolute_url(endpoint) {
        Some((host, path)) => (Some(host), path),
        None => (None, endpoint),
    };
    mock.request.url_patterns = SmallVec::from_elem(UrlPattern::exact(path), 1);
    if let Some(host) = host {
        mock.request.header_matchers =
            SmallVec::from_elem(HeaderMatcher::exact(http::header::HOST, host), 1);
    }
}

/// Create a handler mock for a GraphQL query operation.
///
/// @param operationName - Operation name to match: string or RegExp.
/// @param handler - Function receiving GraphQL resolver info, returning a response,
///   or null/undefined to fall through.
/// @param options - Optional `{ once: true }`.
/// @param endpoint - Optional endpoint URL scope (used by `graphql.link`).
#[napi(namespace = "graphql")]
pub fn query(
    env: &Env,
    operation_name: Unknown,
    handler_fn: Function<'_, GraphQLRequestInfo, Promise<Option<HandlerResponse>>>,
    options: Option<RequestHandlerOptions>,
    endpoint: Option<String>,
) -> Result<RequestHandler> {
    build_graphql(
        env,
        Some(GraphQLOperationType::Query),
        Some(operation_name),
        endpoint,
        handler_fn,
        options,
    )
}

/// Create a handler mock for a GraphQL mutation operation.
///
/// @param operationName - Operation name to match: string or RegExp.
/// @param handler - Function receiving GraphQL resolver info, returning a response,
///   or null/undefined to fall through.
/// @param options - Optional `{ once: true }`.
/// @param endpoint - Optional endpoint URL scope (used by `graphql.link`).
#[napi(namespace = "graphql")]
pub fn mutation(
    env: &Env,
    operation_name: Unknown,
    handler_fn: Function<'_, GraphQLRequestInfo, Promise<Option<HandlerResponse>>>,
    options: Option<RequestHandlerOptions>,
    endpoint: Option<String>,
) -> Result<RequestHandler> {
    build_graphql(
        env,
        Some(GraphQLOperationType::Mutation),
        Some(operation_name),
        endpoint,
        handler_fn,
        options,
    )
}

/// Create a handler mock matching any GraphQL operation.
///
/// @param handler - Function receiving GraphQL resolver info, returning a response,
///   or null/undefined to fall through.
/// @param options - Optional `{ once: true }`.
/// @param endpoint - Optional endpoint URL scope (used by `graphql.link`).
#[napi(namespace = "graphql")]
pub fn operation(
    env: &Env,
    handler_fn: Function<'_, GraphQLRequestInfo, Promise<Option<HandlerResponse>>>,
    options: Option<RequestHandlerOptions>,
    endpoint: Option<String>,
) -> Result<RequestHandler> {
    build_graphql(env, None, None, endpoint, handler_fn, options)
}
