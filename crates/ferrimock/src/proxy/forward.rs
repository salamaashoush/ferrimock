//! The request path: match a mock, or forward to an upstream.

use axum::body::Body;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use http::{HeaderMap, Method, Request, StatusCode};
use http_body_util::BodyExt;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::client::UpstreamError;
use super::config::{RouteConfig, Target};
use super::headers;
use super::pending::PendingBody;
use super::route;
use super::state::ProxyState;
use super::tee::TeeBody;
use crate::engine::MockAction;
use crate::recorder::MockRecorder;

/// Where a request came from, as far as the forwarding identity is concerned.
#[derive(Debug, Clone, Copy)]
pub struct ClientInfo {
    /// Peer address, for `X-Forwarded-For`.
    pub ip: Option<IpAddr>,
    /// Whether the client reached this listener over TLS.
    pub tls: bool,
}

/// Answer one request.
///
/// The order is load-bearing. A mock is consulted before a route is resolved,
/// so a mock can answer for a host no route forwards; a route is resolved
/// before the body is read, so a request nothing needs to inspect never has
/// its body touched.
pub async fn handle(
    state: &Arc<ProxyState>,
    request: Request<Body>,
    client: ClientInfo,
) -> Response {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().map(str::to_string);
    let mut pending = PendingBody::new(body);

    let action = if state.mocks_enabled() {
        match_mock(
            state,
            &parts.method,
            &path,
            query.as_deref(),
            &parts.headers,
            &mut pending,
        )
        .await
    } else {
        None
    };

    match action {
        Some(MockAction::FullMock(response)) => {
            if state.config.verbose {
                tracing::info!(
                    method = %parts.method,
                    path = %path,
                    status = response.status().as_u16(),
                    "mocked"
                );
            }
            let (response_parts, bytes) = response.into_parts();
            http::Response::from_parts(response_parts, Body::from(bytes))
        }
        Some(MockAction::PatchUpstream {
            response_patches,
            request_patches,
            pre_delay,
            post_delay,
            status_override,
            upstream_options,
            rewrite_path,
            mock_id,
            captures,
            vars,
        }) => {
            let patch = PatchPlan {
                response_patches,
                request_patches,
                pre_delay,
                post_delay,
                status_override,
                forward_to: upstream_options.forward_to,
                request_timeout: upstream_options.timeout,
                rewrite_path,
                mock_id: mock_id.to_string(),
                captures,
                vars,
            };
            patch_upstream(state, parts, path, query, pending, patch).await
        }
        None => forward(state, parts, path, query, pending, client).await,
    }
}

/// Ask the matcher what to do with this request, reading the body only if
/// some registered mock actually matches on one.
///
/// `needs_request_body` is a registry-wide fact, so a setup with no
/// body-matching mock never buffers anything, and an upload passes through
/// the proxy at streaming cost.
async fn match_mock(
    state: &Arc<ProxyState>,
    method: &Method,
    path: &str,
    query: Option<&str>,
    request_headers: &HeaderMap,
    pending: &mut PendingBody,
) -> Option<MockAction> {
    let matcher = state.matcher.as_ref()?;

    let body = if matcher.registry().needs_request_body() {
        pending.bytes(state.config.max_buffered_request).await
    } else {
        None
    };

    matcher
        .try_match_parts(method, path, query, request_headers, body.as_deref())
        .await
}

/// Forward a request to its route's upstream and stream the answer back.
async fn forward(
    state: &Arc<ProxyState>,
    parts: http::request::Parts,
    path: String,
    query: Option<String>,
    pending: PendingBody,
    client: ClientInfo,
) -> Response {
    let host = parts
        .headers
        .get(http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let Some(route) = route::resolve(&state.config.routes, host.as_deref(), &path) else {
        return unroutable(state, &parts.method, &path, host.as_deref());
    };

    let uri = match route::upstream_uri(route, &path, query.as_deref()) {
        Ok(uri) => uri,
        Err(error) => {
            return error_response(StatusCode::BAD_REQUEST, &error.to_string());
        }
    };

    // Recording has to decide before the request goes out, because it forces
    // an unencoded response and that is a request header.
    let recorder = state.recorder();
    let request_body = recorder
        .is_some()
        .then(|| match &pending {
            PendingBody::Buffered(bytes) => Some(bytes.clone()),
            PendingBody::Stream(_) | PendingBody::Chained { .. } => None,
        })
        .flatten();

    let mut upstream_headers = parts.headers;
    headers::strip_hop_by_hop(&mut upstream_headers);
    let client_host = upstream_headers.get(http::header::HOST).cloned();
    headers::apply_forwarding_identity(
        &mut upstream_headers,
        route,
        client_host.as_ref(),
        headers::client_scheme(client.tls),
        client.ip,
    );
    if recorder.is_some() {
        headers::request_identity_encoding(&mut upstream_headers);
    }

    let recorded_request_headers = recorder.is_some().then(|| upstream_headers.clone());

    let mut request = Request::new(pending.into_request_body());
    *request.method_mut() = parts.method.clone();
    *request.uri_mut() = uri;
    *request.version_mut() = http::Version::HTTP_11;
    *request.headers_mut() = upstream_headers;

    let started = Instant::now();
    let response = match state.client.send(request, route.timeout).await {
        Ok(response) => response,
        Err(error) => return upstream_failure(state, &error, &path),
    };

    if state.config.verbose {
        tracing::info!(
            method = %parts.method,
            path = %path,
            status = response.status().as_u16(),
            upstream = %route.target,
            elapsed_ms = started.elapsed().as_millis(),
            "forwarded"
        );
    }

    let (mut response_parts, body) = response.into_parts();
    headers::clean_response_headers(&mut response_parts.headers, false);

    // An event stream never ends, so teeing one accumulates forever and
    // records nothing. Streaming content types are forwarded and not recorded.
    let record = recorder.filter(|_| !headers::is_streaming_response(&response_parts.headers));

    let body = match (record, recorded_request_headers) {
        (Some(recorder), Some(request_headers)) => {
            let tap = RecordingTap {
                recorder,
                method: parts.method,
                path,
                query,
                request_headers,
                request_body,
                status: response_parts.status,
                response_headers: response_parts.headers.clone(),
                started,
            };
            Body::new(TeeBody::new(
                Body::new(body),
                state.config.max_buffered_response,
                Box::new(move |bytes| tap.commit(bytes)),
            ))
        }
        _ => Body::new(body),
    };

    Response::from_parts(response_parts, body)
}

/// Everything a completed response needs in order to be recorded.
struct RecordingTap {
    recorder: Arc<MockRecorder>,
    method: Method,
    path: String,
    query: Option<String>,
    request_headers: HeaderMap,
    request_body: Option<Bytes>,
    status: StatusCode,
    response_headers: HeaderMap,
    started: Instant,
}

impl RecordingTap {
    /// Hand the finished interaction to the recorder.
    ///
    /// Off the response path entirely: the last frame has already gone to the
    /// browser by the time this runs, so a slow disk cannot hold a response
    /// open.
    fn commit(self, response_body: Bytes) {
        let elapsed = self.started.elapsed();
        tokio::spawn(async move {
            if let Err(error) = self
                .recorder
                .record(
                    &self.method,
                    &self.path,
                    self.query.as_deref(),
                    &self.request_headers,
                    self.request_body.as_ref(),
                    self.status,
                    &self.response_headers,
                    &response_body,
                    elapsed,
                )
                .await
            {
                tracing::warn!("recording {} {} failed: {error}", self.method, self.path);
            }
        });
    }
}

/// What a matched `patch:` mock asked for.
struct PatchPlan {
    response_patches: Vec<crate::types::PatchOperation>,
    request_patches: Vec<crate::types::RequestPatch>,
    pre_delay: Option<Duration>,
    post_delay: Option<Duration>,
    status_override: Option<StatusCode>,
    forward_to: Option<String>,
    request_timeout: Option<Duration>,
    rewrite_path: Option<String>,
    mock_id: String,
    captures: rustc_hash::FxHashMap<String, String>,
    vars: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Fetch from upstream, then rewrite the answer the way the mock asked.
///
/// This is the one path that must collect the response: a JSONPath patch
/// cannot be applied to a stream. It is also the one path that forces an
/// unencoded upstream response, since a patch operates on JSON and gzip is
/// not JSON.
async fn patch_upstream(
    state: &Arc<ProxyState>,
    parts: http::request::Parts,
    path: String,
    query: Option<String>,
    mut pending: PendingBody,
    plan: PatchPlan,
) -> Response {
    let host = parts
        .headers
        .get(http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    // `forward_to` retargets one mock without touching the route table, which
    // is how a single endpoint gets pointed at staging while everything else
    // stays local.
    let override_target = match plan.forward_to.as_deref().map(Target::parse).transpose() {
        Ok(target) => target,
        Err(error) => return error_response(StatusCode::BAD_GATEWAY, &error.to_string()),
    };

    let matched = route::resolve(&state.config.routes, host.as_deref(), &path);
    let owned_route;
    let route = if let Some(target) = override_target {
        // Keep whatever the matching route said about prefixes and host
        // handling; only the destination changes. With no matching route
        // there is nothing to keep, and the mock's own target is the whole
        // rule.
        owned_route = match matched.cloned() {
            Some(route) => RouteConfig { target, ..route },
            None => RouteConfig::new("/", target),
        };
        &owned_route
    } else if let Some(existing) = matched {
        existing
    } else {
        return unroutable(state, &parts.method, &path, host.as_deref());
    };

    let forwarded_path = plan.rewrite_path.as_deref().unwrap_or(&path);

    let mut upstream_headers = parts.headers.clone();
    headers::strip_hop_by_hop(&mut upstream_headers);

    let body_for_context = pending.bytes(state.config.max_buffered_request).await;
    let (mut upstream_headers, patched_body, patched_query) =
        match crate::engine::RequestPatcher::new(plan.request_patches).apply(
            upstream_headers,
            body_for_context.clone(),
            query.as_deref(),
        ) {
            Ok(applied) => applied,
            Err(error) => {
                return error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("request patch for mock '{}' failed: {error}", plan.mock_id),
                );
            }
        };

    if let Some(body) = patched_body {
        pending.replace(body);
    }
    let effective_query = patched_query.or(query.clone());

    let uri = match route::upstream_uri(route, forwarded_path, effective_query.as_deref()) {
        Ok(uri) => uri,
        Err(error) => return error_response(StatusCode::BAD_REQUEST, &error.to_string()),
    };

    let client_host = upstream_headers.get(http::header::HOST).cloned();
    headers::apply_forwarding_identity(
        &mut upstream_headers,
        route,
        client_host.as_ref(),
        headers::client_scheme(false),
        None,
    );
    headers::request_identity_encoding(&mut upstream_headers);

    if let Some(delay) = plan.pre_delay {
        tokio::time::sleep(delay).await;
    }

    let mut request = Request::new(pending.into_request_body());
    *request.method_mut() = parts.method.clone();
    *request.uri_mut() = uri;
    *request.headers_mut() = upstream_headers;

    let response = match state
        .client
        .send(request, plan.request_timeout.or(route.timeout))
        .await
    {
        Ok(response) => response,
        Err(error) => return upstream_failure(state, &error, &path),
    };

    let (mut response_parts, body) = response.into_parts();
    let collected = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(error) => {
            return error_response(
                StatusCode::BAD_GATEWAY,
                &format!("reading the upstream body failed: {error}"),
            );
        }
    };

    if let Some(delay) = plan.post_delay {
        tokio::time::sleep(delay).await;
    }

    if let Some(status) = plan.status_override {
        response_parts.status = status;
    }
    headers::clean_response_headers(&mut response_parts.headers, true);

    let mut request_context = crate::types::RequestContext::from_request(
        parts.method.as_str(),
        &path,
        effective_query.as_deref(),
        &parts.headers,
        body_for_context.as_deref(),
    );
    request_context.captures = plan.captures;
    request_context.vars = plan.vars;

    let upstream = http::Response::from_parts(response_parts, collected);
    match crate::engine::MockMatcher::apply_patches(
        plan.response_patches,
        &plan.mock_id,
        upstream,
        Some(request_context),
    ) {
        Ok(patched) => {
            let (patched_parts, bytes) = patched.into_parts();
            http::Response::from_parts(patched_parts, Body::from(bytes))
        }
        Err(error) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("response patch for mock '{}' failed: {error}", plan.mock_id),
        ),
    }
}

/// Nothing matched and nothing forwards: say which of the two it was.
///
/// A bare 404 here is the single most confusing thing a dev proxy can answer,
/// because it looks like the upstream's own 404. Naming the host and listing
/// the routes that exist is the difference between a typo found in seconds and
/// one found in an hour.
fn unroutable(
    state: &Arc<ProxyState>,
    method: &Method,
    path: &str,
    host: Option<&str>,
) -> Response {
    let routes: Vec<String> = state
        .config
        .routes
        .iter()
        .map(|route| format!("{} -> {}", route.prefix, route.target))
        .collect();

    let body = serde_json::json!({
        "error": "no mock matched and no route forwards this request",
        "method": method.as_str(),
        "path": path,
        "host": host,
        "routes": routes,
    });

    error_json(StatusCode::NOT_FOUND, &body)
}

/// The upstream was reachable in configuration but not in fact.
fn upstream_failure(state: &Arc<ProxyState>, error: &UpstreamError, path: &str) -> Response {
    if state.config.verbose {
        tracing::warn!(path = %path, "{error}");
    }
    let body = serde_json::json!({
        "error": error.to_string(),
        "path": path,
    });
    error_json(error.status(), &body)
}

/// A JSON error response.
fn error_json(status: StatusCode, body: &serde_json::Value) -> Response {
    (
        status,
        [(http::header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// A plain-text error response, for failures that have nothing structured to say.
fn error_response(status: StatusCode, message: &str) -> Response {
    error_json(status, &serde_json::json!({ "error": message }))
}
