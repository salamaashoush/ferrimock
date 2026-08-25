//! A reverse proxy that answers from mocks first and forwards everything else.
//!
//! Put it in front of a dev server (vite, rspack, webpack) or a backend and
//! point the browser at it instead: every request that matches a mock is
//! answered locally, and every request that does not reaches the real thing.
//! Nothing in the application changes, and one origin covers both, so there
//! is no CORS to configure.
//!
//! ```no_run
//! use ferrimock::proxy::{ProxyConfig, RouteConfig, Target};
//!
//! # async fn example() -> ferrimock::Result<()> {
//! let mut config = ProxyConfig {
//!     routes: vec![
//!         RouteConfig::parse("/api=http://localhost:8080")?,
//!         RouteConfig::parse("/=http://localhost:5173")?,
//!     ],
//!     ..ProxyConfig::default()
//! };
//! config.compile();
//!
//! let proxy = ferrimock::proxy::start(config, None).await?;
//! println!("listening on {}", proxy.url());
//! proxy.wait().await;
//! # Ok(())
//! # }
//! ```
//!
//! ## What it costs
//!
//! The forwarding path never collects a body. A request is read into memory
//! only when some registered mock matches on request bodies, and a response
//! only when a `patch:` mock is rewriting it. Everything else moves frame by
//! frame, so an upload, a bundle and an event stream all cost one frame of
//! memory rather than their own size. Recording keeps that property: the body
//! is teed as it streams rather than collected first, which is what lets an
//! event stream be both recorded and delivered.
//!
//! ## What speaks what
//!
//! Downstream (browser to proxy): HTTP/1.1, HTTP/2 over TLS via ALPN, h2c by
//! prior knowledge, and WebSocket over either. Upstream (proxy to origin):
//! HTTP/1.1 and HTTP/2 by ALPN, TLS with optional certificate validation, and
//! WebSocket. Server-Sent Events need no special handling in either
//! direction; they are an ordinary streamed body and the proxy never
//! collects one.

mod client;
mod config;
mod forward;
mod headers;
mod pending;
mod route;
mod state;
mod tee;
mod tls;
mod websocket;

pub use client::{UpstreamClient, UpstreamError};
pub use config::{HostMatch, ProxyConfig, RouteConfig, Target, TlsConfig, UpstreamConfig};
pub use forward::ClientInfo;
pub use state::ProxyState;

use axum::Router;
use axum::extract::{ConnectInfo, State};
use axum::routing::any;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// What the one handler needs: the proxy, and whether this listener is TLS.
#[derive(Clone)]
struct ProxyContext {
    state: Arc<ProxyState>,
    tls: bool,
}

/// Every request is a mock lookup or a forward, so there is nothing to route
/// on: one fallback over the whole path space.
fn router(state: Arc<ProxyState>, tls: bool) -> Router {
    Router::new()
        .fallback(any(proxy_handler))
        .with_state(ProxyContext { state, tls })
}

async fn proxy_handler(
    State(context): State<ProxyContext>,
    // Always present: both serve arms below mount the router with
    // `into_make_service_with_connect_info`, which is what puts it there.
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
) -> axum::response::Response {
    let client = ClientInfo {
        ip: Some(peer.ip()),
        tls: context.tls,
    };

    if websocket::is_upgrade(&request) {
        websocket::handle(&context.state, request).await
    } else {
        forward::handle(&context.state, request, client).await
    }
}

/// A running proxy.
pub struct ProxyHandle {
    state: Arc<ProxyState>,
    local_addr: SocketAddr,
    tls: bool,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl ProxyHandle {
    /// The address actually bound, which is what to read after asking for
    /// port 0.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// The origin to point a browser at.
    pub fn url(&self) -> String {
        let scheme = if self.tls { "https" } else { "http" };
        format!("{scheme}://{}", self.local_addr)
    }

    /// The shared state, for starting a recording or reaching the registry
    /// while the proxy runs.
    pub fn state(&self) -> &Arc<ProxyState> {
        &self.state
    }

    /// Ask the proxy to stop accepting and drain what is in flight.
    pub fn shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }

    /// Wait for the proxy to finish, shutting it down first.
    pub async fn wait(mut self) {
        self.shutdown();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for ProxyHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Bind and start a proxy.
///
/// Pass a [`crate::engine::MockMatcher`] to answer from mocks before
/// forwarding, or `None` to run as a plain gateway.
///
/// # Errors
/// Fails when the address is already in use, when TLS material cannot be
/// read, or when the upstream client cannot be built.
pub async fn start(
    mut config: ProxyConfig,
    matcher: Option<crate::engine::MockMatcher>,
) -> crate::Result<ProxyHandle> {
    config.compile();

    let tls = match config.tls.as_ref() {
        Some(tls) => Some(tls::axum_config(tls)?),
        None => None,
    };

    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    let local_addr = listener.local_addr()?;
    let has_tls = tls.is_some();

    let state = Arc::new(ProxyState::new(config, matcher)?);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    let app = router(Arc::clone(&state), has_tls);
    let shutdown = async move {
        let _ = shutdown_rx.await;
    };

    let task = match tls {
        // axum::serve does not terminate TLS, so the TLS arm goes through
        // axum-server, which does and keeps the same Router.
        Some(tls) => {
            let std_listener = listener.into_std()?;
            std_listener.set_nonblocking(true)?;
            tokio::spawn(async move {
                let handle = axum_server::Handle::new();
                tokio::spawn({
                    let handle = handle.clone();
                    async move {
                        shutdown.await;
                        handle.graceful_shutdown(None);
                    }
                });
                if let Err(error) = axum_server::from_tcp_rustls(std_listener, tls)
                    .handle(handle)
                    .serve(app.into_make_service_with_connect_info::<SocketAddr>())
                    .await
                {
                    tracing::error!("proxy TLS server error: {error}");
                }
            })
        }
        None => tokio::spawn(async move {
            // Nagle holds a small write back waiting for the next one, which
            // on a proxy means holding a complete response until the following
            // request arrives.
            let listener =
                axum::serve::ListenerExt::tap_io(listener, |stream: &mut tokio::net::TcpStream| {
                    let _ = stream.set_nodelay(true);
                });
            if let Err(error) = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown)
            .await
            {
                tracing::error!("proxy server error: {error}");
            }
        }),
    };

    Ok(ProxyHandle {
        state,
        local_addr,
        tls: has_tls,
        shutdown: Some(shutdown_tx),
        task: Some(task),
    })
}
