//! The TLS client configuration of a connection.
//!
//! `rustls` on `aws-lc-rs`, the provider every TLS client of the workspace
//! uses: a second provider would be a second cryptography to ship and audit.
//! The roots are the Mozilla list `webpki-roots` carries, or — when the form
//! names a CA file — that file's certificates **only**: a private authority
//! is what a CA file is for, and adding the public roots beside it would let
//! any public certificate for the name pass.

use std::path::Path;
use std::sync::Arc;

use oxyn_core::{OxynError, Result};
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{self, CryptoProvider, WebPkiSupportedAlgorithms};
use tokio_rustls::rustls::pki_types::pem::PemObject as _;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme};

use crate::options::TlsMode;

/// The client configuration for a mode, or `None` in clear text.
///
/// Reads the CA file on the blocking pool: a file system that stalls must not
/// stall a runtime thread.
///
/// # Errors
/// [`OxynError::Config`] when the CA file cannot be read or holds no
/// certificate, [`OxynError::Internal`] if the provider refuses the default
/// protocol versions.
pub async fn client_config(
    mode: TlsMode,
    ca_file: Option<&Path>,
) -> Result<Option<Arc<ClientConfig>>> {
    let provider = Arc::new(crypto::aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .map_err(|error| OxynError::Internal(format!("TLS protocol versions refused: {error}")))?;
    let config = match mode {
        TlsMode::Disable => return Ok(None),
        TlsMode::VerifyFull => {
            let roots = match ca_file {
                Some(path) => roots_from(path.to_owned()).await?,
                None => RootCertStore {
                    roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
                },
            };
            builder.with_root_certificates(roots).with_no_client_auth()
        }
        TlsMode::Require => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Unverified(provider)))
            .with_no_client_auth(),
    };
    Ok(Some(Arc::new(config)))
}

/// The name a certificate must carry.
///
/// # Errors
/// [`OxynError::Config`] for a host that is neither a DNS name nor an address.
pub fn server_name(host: &str) -> Result<ServerName<'static>> {
    ServerName::try_from(host.to_owned()).map_err(|_| {
        OxynError::Config("the host is not a name a TLS certificate can carry".to_owned())
    })
}

async fn roots_from(path: std::path::PathBuf) -> Result<RootCertStore> {
    tokio::task::spawn_blocking(move || -> Result<RootCertStore> {
        let mut roots = RootCertStore::empty();
        let certificates = CertificateDer::pem_file_iter(&path)
            .map_err(|_| OxynError::Config("the CA file cannot be read as PEM".to_owned()))?;
        for certificate in certificates {
            let certificate = certificate.map_err(|_| {
                OxynError::Config("the CA file holds an unreadable certificate".to_owned())
            })?;
            roots.add(certificate).map_err(|_| {
                OxynError::Config("the CA file holds a certificate rustls refuses".to_owned())
            })?;
        }
        if roots.is_empty() {
            return Err(OxynError::Config(
                "the CA file holds no certificate".to_owned(),
            ));
        }
        Ok(roots)
    })
    .await
    .map_err(|_| OxynError::Internal("the CA file reader stopped".to_owned()))?
}

/// `require`: the traffic is encrypted, the server's certificate is not
/// checked against anything — only the handshake signatures are, so the
/// session keys are the server's.
#[derive(Debug)]
struct Unverified(Arc<CryptoProvider>);

impl Unverified {
    fn algorithms(&self) -> &WebPkiSupportedAlgorithms {
        &self.0.signature_verification_algorithms
    }
}

impl ServerCertVerifier for Unverified {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        crypto::verify_tls12_signature(message, cert, dss, self.algorithms())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        crypto::verify_tls13_signature(message, cert, dss, self.algorithms())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms().supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn clear_text_has_no_configuration_and_the_others_do() {
        assert!(
            client_config(TlsMode::Disable, None)
                .await
                .expect("built")
                .is_none()
        );
        assert!(
            client_config(TlsMode::VerifyFull, None)
                .await
                .expect("built")
                .is_some()
        );
        assert!(
            client_config(TlsMode::Require, None)
                .await
                .expect("built")
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_ca_file_that_is_not_pem_is_a_configuration_error() {
        let missing = Path::new("/nonexistent/oxyn-ca.pem");
        let error = client_config(TlsMode::VerifyFull, Some(missing))
            .await
            .expect_err("unreadable");
        assert!(matches!(error, OxynError::Config(_)), "{error:?}");
    }

    #[test]
    fn a_host_becomes_a_server_name() {
        assert!(server_name("cache.internal").is_ok());
        assert!(server_name("10.0.0.7").is_ok());
        assert!(server_name("bad host").is_err());
    }
}
