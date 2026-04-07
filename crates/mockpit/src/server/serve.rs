//! HTTP server with graceful shutdown

use axum::Router;
use axum::serve::ListenerExt;
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// Serve an Axum router with graceful shutdown
///
/// Starts an HTTP server on the given listener and shuts down gracefully
/// when the shutdown signal resolves.
pub fn serve_with_graceful_shutdown(
    listener: TcpListener,
    app: Router,
    shutdown_signal: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<JoinHandle<()>> {
    let server_handle = tokio::spawn(async move {
        // Enable TCP_NODELAY on every accepted connection to disable Nagle's
        // algorithm and eliminate latency on the final TCP segment.
        let listener = listener.tap_io(|tcp_stream: &mut tokio::net::TcpStream| {
            let _ = tcp_stream.set_nodelay(true);
        });

        if let Err(e) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal)
        .await
        {
            tracing::error!("Server error: {e}");
        }
    });

    Ok(server_handle)
}
