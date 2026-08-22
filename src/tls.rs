//! TLS support for the HTTP API server.
//!
//! Provides certificate loading from files, self-signed generation,
//! and (future) ACME and Vault PKI integration.

use anyhow::{Context, Result, bail};
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;

/// TLS configuration for the HTTP API server
#[derive(Debug, Clone, Default)]
pub struct TlsConfig {
    /// Path to PEM certificate file
    pub cert_path: Option<String>,
    /// Path to PEM private key file
    pub key_path: Option<String>,
    /// Auto-generate self-signed cert if no cert/key provided
    pub self_signed: bool,
    /// Require TLS (reject plain HTTP connections)
    pub require_tls: bool,
}

impl TlsConfig {
    /// Load certificates from files or generate self-signed, then build a TlsAcceptor.
    pub fn load_or_generate(&self) -> Result<TlsAcceptor> {
        // Ensure a CryptoProvider is installed (idempotent, ignores error if already set)
        let _ = rustls::crypto::ring::default_provider().install_default();

        let (certs, key) =
            if let (Some(cert_path), Some(key_path)) = (&self.cert_path, &self.key_path) {
                let certs = load_certs_from_file(Path::new(cert_path))?;
                let key = load_key_from_file(Path::new(key_path))?;
                (certs, key)
            } else if self.self_signed || (self.cert_path.is_none() && self.key_path.is_none()) {
                if self.require_tls {
                    eprintln!(
                        "Warning: --require-tls is set with a self-signed certificate. \
                         Consider providing --tls-cert/--tls-key for production use."
                    );
                }
                let (cert_pem, key_pem) = generate_self_signed_cert("agentkernel")?;
                let certs = load_certs_from_pem(cert_pem.as_bytes())?;
                let key = load_key_from_pem(key_pem.as_bytes())?;
                (certs, key)
            } else {
                bail!("Both --tls-cert and --tls-key must be provided together");
            };

        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .context("Failed to build TLS server config")?;

        Ok(TlsAcceptor::from(Arc::new(server_config)))
    }
}

/// Generate a self-signed certificate for development use.
///
/// Returns (cert_pem, key_pem) strings.
pub fn generate_self_signed_cert(cn: &str) -> Result<(String, String)> {
    let subject_alt_names = vec![
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        cn.to_string(),
    ];

    let certified_key = rcgen::generate_simple_self_signed(subject_alt_names)
        .context("Failed to generate self-signed certificate")?;

    let cert_pem = certified_key.cert.pem();
    let key_pem = certified_key.signing_key.serialize_pem();

    eprintln!(
        "Warning: Using self-signed certificate. \
         For production, use --tls-cert/--tls-key or Vault PKI."
    );

    Ok((cert_pem, key_pem))
}

/// Load certificates from a PEM file on disk.
pub fn load_certs_from_file(
    path: &Path,
) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open certificate file: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("Failed to parse certificates from: {}", path.display()))?;
    if certs.is_empty() {
        bail!("No certificates found in file: {}", path.display());
    }
    Ok(certs)
}

/// Load a private key from a PEM file on disk.
pub fn load_key_from_file(path: &Path) -> Result<rustls::pki_types::PrivateKeyDer<'static>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open key file: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let key = rustls_pemfile::private_key(&mut reader)
        .with_context(|| format!("Failed to parse private key from: {}", path.display()))?
        .ok_or_else(|| anyhow::anyhow!("No private key found in file: {}", path.display()))?;
    Ok(key)
}

/// Load certificates from PEM bytes in memory.
fn load_certs_from_pem(pem: &[u8]) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let mut reader = BufReader::new(pem);
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("Failed to parse PEM certificates")?;
    Ok(certs)
}

/// Load a private key from PEM bytes in memory.
fn load_key_from_pem(pem: &[u8]) -> Result<rustls::pki_types::PrivateKeyDer<'static>> {
    let mut reader = BufReader::new(pem);
    let key = rustls_pemfile::private_key(&mut reader)
        .context("Failed to parse PEM private key")?
        .ok_or_else(|| anyhow::anyhow!("No private key found in PEM data"))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tls_config_defaults() {
        let config = TlsConfig::default();
        assert!(config.cert_path.is_none());
        assert!(config.key_path.is_none());
        assert!(!config.self_signed);
        assert!(!config.require_tls);
    }

    #[test]
    fn test_generate_self_signed_cert_produces_valid_pem() {
        let (cert_pem, key_pem) = generate_self_signed_cert("test-host").unwrap();

        // Verify PEM markers
        assert!(cert_pem.contains("-----BEGIN CERTIFICATE-----"));
        assert!(cert_pem.contains("-----END CERTIFICATE-----"));
        assert!(key_pem.contains("-----BEGIN PRIVATE KEY-----"));
        assert!(key_pem.contains("-----END PRIVATE KEY-----"));

        // Verify we can parse them back
        let certs = load_certs_from_pem(cert_pem.as_bytes()).unwrap();
        assert_eq!(certs.len(), 1);

        let _key = load_key_from_pem(key_pem.as_bytes()).unwrap();
    }

    #[test]
    fn test_load_certs_from_file_with_generated_cert() {
        let (cert_pem, key_pem) = generate_self_signed_cert("test-file").unwrap();

        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");

        std::fs::write(&cert_path, &cert_pem).unwrap();
        std::fs::write(&key_path, &key_pem).unwrap();

        let certs = load_certs_from_file(&cert_path).unwrap();
        assert_eq!(certs.len(), 1);

        let _key = load_key_from_file(&key_path).unwrap();
    }

    #[test]
    fn test_load_certs_from_file_nonexistent() {
        let result = load_certs_from_file(Path::new("/nonexistent/cert.pem"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_key_from_file_nonexistent() {
        let result = load_key_from_file(Path::new("/nonexistent/key.pem"));
        assert!(result.is_err());
    }

    /// Install the ring CryptoProvider for tests that build TLS configs.
    fn install_crypto_provider() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }

    #[test]
    fn test_tls_config_load_or_generate_self_signed() {
        install_crypto_provider();
        let config = TlsConfig {
            cert_path: None,
            key_path: None,
            self_signed: true,
            require_tls: false,
        };
        let acceptor = config.load_or_generate();
        assert!(acceptor.is_ok());
    }

    #[test]
    fn test_tls_config_load_or_generate_from_files() {
        install_crypto_provider();
        let (cert_pem, key_pem) = generate_self_signed_cert("test-load").unwrap();

        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        let key_path = dir.path().join("key.pem");

        std::fs::write(&cert_path, &cert_pem).unwrap();
        std::fs::write(&key_path, &key_pem).unwrap();

        let config = TlsConfig {
            cert_path: Some(cert_path.to_str().unwrap().to_string()),
            key_path: Some(key_path.to_str().unwrap().to_string()),
            self_signed: false,
            require_tls: false,
        };
        let acceptor = config.load_or_generate();
        assert!(acceptor.is_ok());
    }

    #[test]
    fn test_tls_config_load_or_generate_missing_key() {
        let (cert_pem, _) = generate_self_signed_cert("test-missing-key").unwrap();

        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("cert.pem");
        std::fs::write(&cert_path, &cert_pem).unwrap();

        let config = TlsConfig {
            cert_path: Some(cert_path.to_str().unwrap().to_string()),
            key_path: None,
            self_signed: false,
            require_tls: false,
        };
        let result = config.load_or_generate();
        assert!(result.is_err());
    }

    #[test]
    fn test_load_certs_from_file_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let cert_path = dir.path().join("empty.pem");
        std::fs::write(&cert_path, "").unwrap();

        let result = load_certs_from_file(&cert_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No certificates"));
    }
}
