//! The client the proxy reaches upstreams with.

use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use std::sync::Arc;
use std::time::Duration;

use super::config::UpstreamConfig;
use super::tls::ensure_crypto_provider;

/// A pooled HTTP client for upstream requests.
#[derive(Clone)]
pub struct UpstreamClient {
    inner: Client<hyper_rustls::HttpsConnector<HttpConnector>, axum::body::Body>,
    header_timeout: Option<Duration>,
}

impl std::fmt::Debug for UpstreamClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpstreamClient")
            .field("header_timeout", &self.header_timeout)
            .finish_non_exhaustive()
    }
}

impl UpstreamClient {
    /// Build a client from an upstream configuration.
    ///
    /// # Errors
    /// Fails when the platform certificate store cannot be read, which only
    /// matters for HTTPS upstreams.
    pub fn new(config: &UpstreamConfig) -> crate::Result<Self> {
        ensure_crypto_provider();

        let mut http = HttpConnector::new();
        // The proxy hands the connector an absolute URI whose scheme may be
        // https; without this the connector rejects it before TLS is reached.
        http.enforce_http(false);
        // Nagle batches a small write against the next one, which on a proxy
        // means holding a request until the following frame shows up.
        http.set_nodelay(true);
        http.set_keepalive(Some(Duration::from_secs(60)));
        http.set_connect_timeout(config.connect_timeout);
        // A dual-stack `localhost` resolves to ::1 and 127.0.0.1, and a dev
        // server bound to one of them refuses the other. Racing the two costs
        // nothing and turns a hard failure into a connection.
        http.set_happy_eyeballs_timeout(Some(Duration::from_millis(200)));
        http.set_reuse_address(true);

        let builder = hyper_rustls::HttpsConnectorBuilder::new();
        let with_tls = if config.accept_invalid_certs {
            builder.with_tls_config(accept_any_certificate())
        } else {
            builder
                .with_native_roots()
                .map_err(|e| crate::mp_err!("cannot read the platform certificate store: {e}"))?
        };

        let schemes = with_tls.https_or_http();
        let connector = if config.http2 {
            schemes.enable_all_versions().wrap_connector(http)
        } else {
            schemes.enable_http1().wrap_connector(http)
        };

        let mut client = Client::builder(TokioExecutor::new());
        client
            // Without a timer the idle timeout never fires, so the pool grows
            // to `pool_max_idle_per_host` connections per upstream and holds
            // them until the process exits. This is the line that makes the
            // two settings below mean anything.
            .pool_timer(hyper_util::rt::TokioTimer::new())
            .pool_idle_timeout(config.pool_idle_timeout)
            .pool_max_idle_per_host(config.pool_max_idle_per_host)
            // Every response the proxy forwards is framed by hyper from the
            // body it is handed, so an upstream's own title-case header
            // spellings are not worth the per-header allocation to preserve.
            .http1_preserve_header_case(false)
            .http1_max_buf_size(config.http1_max_buf_size)
            // A dev server sends bundles over one HTTP/2 connection. A fixed
            // 64KB stream window stalls each of those until the proxy acks;
            // sizing from observed bandwidth-delay keeps them at line rate.
            .http2_adaptive_window(true)
            .http2_max_frame_size(Some(config.http2_max_frame_size))
            // A pooled connection to an upstream that went away silently is
            // discovered on the next request otherwise, which spends a whole
            // request to learn it. A ping finds it in the background.
            .http2_keep_alive_interval(Some(Duration::from_secs(30)))
            .http2_keep_alive_timeout(Duration::from_secs(10))
            .http2_keep_alive_while_idle(false)
            // A request the proxy has already begun forwarding cannot be
            // replayed: the body may be a stream that has been consumed, and
            // a non-idempotent method must not run twice.
            .retry_canceled_requests(false);

        Ok(Self {
            inner: client.build(connector),
            header_timeout: config.timeout,
        })
    }

    /// Send a request upstream and wait for its response headers.
    ///
    /// The timeout covers headers only. A body is allowed to take as long as
    /// it likes: an event stream that is still open after a minute is working,
    /// not stuck, and timing it out is how a proxy breaks SSE.
    ///
    /// # Errors
    /// Fails when the upstream cannot be reached, speaks something that is not
    /// HTTP, or does not answer within `timeout`.
    pub async fn send(
        &self,
        request: http::Request<axum::body::Body>,
        timeout: Option<Duration>,
    ) -> Result<http::Response<hyper::body::Incoming>, UpstreamError> {
        let pending = self.inner.request(request);

        match timeout.or(self.header_timeout) {
            Some(limit) => tokio::time::timeout(limit, pending)
                .await
                .map_err(|_| UpstreamError::Timeout(limit))?
                .map_err(|e| UpstreamError::Transport(e.to_string())),
            None => pending
                .await
                .map_err(|e| UpstreamError::Transport(e.to_string())),
        }
    }
}

/// Why an upstream request did not produce a response.
#[derive(Debug, thiserror::Error)]
pub enum UpstreamError {
    /// The upstream did not send response headers in time.
    #[error("upstream did not respond within {0:?}")]
    Timeout(Duration),
    /// The connection failed, or what came back was not HTTP.
    #[error("upstream request failed: {0}")]
    Transport(String),
}

impl UpstreamError {
    /// The status a client should see for this failure.
    ///
    /// The distinction is worth keeping: 504 says the upstream is slow and a
    /// retry may work, 502 says it answered with something unusable.
    pub fn status(&self) -> http::StatusCode {
        match self {
            Self::Timeout(_) => http::StatusCode::GATEWAY_TIMEOUT,
            Self::Transport(_) => http::StatusCode::BAD_GATEWAY,
        }
    }
}

/// A TLS configuration that validates nothing.
///
/// Only reachable through `accept_invalid_certs`, which is off by default and
/// exists for the one case that cannot be solved another way: a dev backend
/// behind a certificate no store will ever trust.
fn accept_any_certificate() -> rustls::ClientConfig {
    #[derive(Debug)]
    struct AcceptAnyServerCert(Arc<rustls::crypto::CryptoProvider>);

    impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
        fn verify_server_cert(
            &self,
            _end_entity: &rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[rustls::pki_types::CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls12_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            rustls::crypto::verify_tls13_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }

    ensure_crypto_provider();
    let provider = rustls::crypto::CryptoProvider::get_default().map_or_else(
        || Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
        Arc::clone,
    );

    // ALPN is deliberately left empty. `HttpsConnectorBuilder` fills it from
    // whichever of `enable_all_versions` / `enable_http1` the caller picks,
    // and asserts that nobody set it first.
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert(provider)))
        .with_no_client_auth()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_client_builds_from_the_default_configuration() {
        assert!(UpstreamClient::new(&UpstreamConfig::default()).is_ok());
    }

    #[test]
    fn a_client_builds_with_certificate_validation_disabled() {
        let config = UpstreamConfig {
            accept_invalid_certs: true,
            ..UpstreamConfig::default()
        };
        assert!(UpstreamClient::new(&config).is_ok());
    }

    #[test]
    fn a_timeout_is_a_gateway_timeout_and_a_transport_failure_a_bad_gateway() {
        assert_eq!(
            UpstreamError::Timeout(Duration::from_secs(1)).status(),
            http::StatusCode::GATEWAY_TIMEOUT
        );
        assert_eq!(
            UpstreamError::Transport("refused".into()).status(),
            http::StatusCode::BAD_GATEWAY
        );
    }

    #[test]
    fn the_dangerous_configuration_leaves_alpn_to_the_connector() {
        // `HttpsConnectorBuilder::with_tls_config` panics on a config that
        // already names protocols, so this emptiness is a requirement rather
        // than an oversight.
        assert!(accept_any_certificate().alpn_protocols.is_empty());
    }
}
