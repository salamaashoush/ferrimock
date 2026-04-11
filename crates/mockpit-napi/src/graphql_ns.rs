//! GraphQL namespace bindings: `graphql.query()`, `graphql.mutation()`, etc.

use crate::handler_bridge::js_to_handler_bridge;
use crate::http_ns::JsHandler;
use crate::request_context::MockpitRequest;
use crate::types::JsHandlerResponse;
use mockpit::handler;
use napi::bindgen_prelude::*;
use napi_derive::napi;

/// Create a handler mock for a GraphQL query operation.
///
/// @param operationName - The GraphQL operation name to match (e.g., `"GetUser"`).
/// @param handler - Async function receiving request context, returning response or null.
#[napi(namespace = "graphql")]
pub fn query(
    operation_name: String,
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let bridge = js_to_handler_bridge(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::graphql::query(&operation_name, bridge.handler_fn)),
        fn_ref: Some(bridge.fn_ref),
    })
}

/// Create a handler mock for a GraphQL mutation operation.
///
/// @param operationName - The GraphQL operation name to match.
/// @param handler - Async function receiving request context, returning response or null.
#[napi(namespace = "graphql")]
pub fn mutation(
    operation_name: String,
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let bridge = js_to_handler_bridge(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::graphql::mutation(&operation_name, bridge.handler_fn)),
        fn_ref: Some(bridge.fn_ref),
    })
}

/// Create a handler mock matching any GraphQL operation.
///
/// @param handler - Async function receiving request context, returning response or null.
#[napi(namespace = "graphql")]
pub fn operation(
    handler_fn: Function<'_, MockpitRequest, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let bridge = js_to_handler_bridge(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::graphql::operation(bridge.handler_fn)),
        fn_ref: Some(bridge.fn_ref),
    })
}
