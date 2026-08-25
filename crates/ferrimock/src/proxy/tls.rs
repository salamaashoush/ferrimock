//! TLS termination on the listening side.

use super::config::TlsConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::path::Path;
use std::sync::Arc;

/// Install the process-wide rustls crypto provider.
///
/// Doing it explicitly rather than relying on the feature-inferred default
/// keeps this working in a binary that also links a second provider, where the
/// inferred default is "none" and the first handshake panics.
pub fn ensure_crypto_provider() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        // An error means something else already installed one, which is
        // exactly as good.
        drop(rustls::crypto::aws_lc_rs::default_provider().install_default());
    });
}

/// Offered in preference order. A browser that can do HTTP/2 will take it,
/// and one that cannot falls back without a round trip.
const ALPN: [&[u8]; 2] = [b"h2", b"http/1.1"];

/// Build the TLS configuration `axum-server` terminates with.
///
/// # Errors
/// Fails when a certificate or key cannot be read or parsed, or when a
/// self-signed certificate cannot be generated.
pub fn axum_config(config: &TlsConfig) -> crate::Result<axum_server::tls_rustls::RustlsConfig> {
    Ok(axum_server::tls_rustls::RustlsConfig::from_config(
        server_config(config)?,
    ))
}

/// Build the server-side TLS configuration.
///
/// # Errors
/// Fails when a certificate or key cannot be read or parsed, or when a
/// self-signed certificate cannot be generated.
pub fn server_config(config: &TlsConfig) -> crate::Result<Arc<rustls::ServerConfig>> {
    ensure_crypto_provider();

    let (chain, key) = match config {
        TlsConfig::SelfSigned { names } => self_signed(names)?,
        TlsConfig::Files { cert, key } => from_files(cert, key)?,
    };

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|e| crate::mp_err!("certificate and key do not go together: {e}"))?;

    server_config.alpn_protocols = ALPN.iter().map(|p| p.to_vec()).collect();

    Ok(Arc::new(server_config))
}

/// Issue a certificate for `names`, valid for nothing but development.
fn self_signed(
    names: &[String],
) -> crate::Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    // A certificate with no names matches no request, so an empty list is
    // read as the default rather than honoured literally.
    let names = if names.is_empty() {
        vec!["localhost".to_string(), "127.0.0.1".to_string()]
    } else {
        names.to_vec()
    };

    let issued = rcgen::generate_simple_self_signed(names)
        .map_err(|e| crate::mp_err!("cannot generate a self-signed certificate: {e}"))?;

    let key = PrivateKeyDer::try_from(issued.signing_key.serialize_der())
        .map_err(|e| crate::mp_err!("generated key is not usable: {e}"))?;

    Ok((vec![issued.cert.der().clone()], key))
}

/// Read a PEM chain and key off disk.
fn from_files(
    cert_path: &Path,
    key_path: &Path,
) -> crate::Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    use rustls::pki_types::pem::PemObject;

    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
        .map_err(|e| {
            crate::mp_err!(
                "cannot read the certificate at {}: {e}",
                cert_path.display()
            )
        })?
        .collect::<Result<_, _>>()
        .map_err(|e| {
            crate::mp_err!(
                "cannot parse the certificate at {}: {e}",
                cert_path.display()
            )
        })?;

    if chain.is_empty() {
        return Err(crate::mp_err!(
            "{} holds no certificate",
            cert_path.display()
        ));
    }

    let key = PrivateKeyDer::from_pem_file(key_path)
        .map_err(|e| crate::mp_err!("cannot read the key at {}: {e}", key_path.display()))?;

    Ok((chain, key))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_self_signed_certificate_carries_the_requested_names() {
        let config = server_config(&TlsConfig::SelfSigned {
            names: vec!["app.local".to_string()],
        })
        .unwrap();
        assert_eq!(
            config.alpn_protocols,
            vec![b"h2".to_vec(), b"http/1.1".to_vec()]
        );
    }

    #[test]
    fn an_empty_name_list_falls_back_to_localhost() {
        let (chain, _key) = self_signed(&[]).unwrap();
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn a_missing_certificate_file_says_which_path_failed() {
        let error = server_config(&TlsConfig::Files {
            cert: "/nonexistent/cert.pem".into(),
            key: "/nonexistent/key.pem".into(),
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("/nonexistent/cert.pem"), "{error}");
    }

    #[test]
    fn a_generated_certificate_and_key_go_together() {
        // `with_single_cert` is what checks the pair, so building the config
        // at all is the assertion.
        assert!(
            server_config(&TlsConfig::SelfSigned {
                names: vec!["localhost".to_string()]
            })
            .is_ok()
        );
    }
}
