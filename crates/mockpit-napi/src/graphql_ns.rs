//! GraphQL namespace bindings: `graphql.query()`, `graphql.mutation()`, etc.

use crate::handler_bridge::js_to_handler_fn;
use crate::http_ns::JsHandler;
use crate::types::{JsHandlerResponse, JsRequestContext};
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
    handler_fn: Function<'_, JsRequestContext, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let rust_handler = js_to_handler_fn(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::graphql::query(&operation_name, rust_handler)),
    })
}

/// Create a handler mock for a GraphQL mutation operation.
///
/// @param operationName - The GraphQL operation name to match.
/// @param handler - Async function receiving request context, returning response or null.
#[napi(namespace = "graphql")]
pub fn mutation(
    operation_name: String,
    handler_fn: Function<'_, JsRequestContext, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let rust_handler = js_to_handler_fn(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::graphql::mutation(&operation_name, rust_handler)),
    })
}

/// Create a handler mock matching any GraphQL operation.
///
/// @param handler - Async function receiving request context, returning response or null.
#[napi(namespace = "graphql")]
pub fn operation(
    handler_fn: Function<'_, JsRequestContext, Promise<Option<JsHandlerResponse>>>,
) -> Result<JsHandler> {
    let rust_handler = js_to_handler_fn(handler_fn)?;
    Ok(JsHandler {
        inner: Some(handler::graphql::operation(rust_handler)),
    })
}
