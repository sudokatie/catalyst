//! TLS configuration for secure remote execution

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use rustls_pemfile::{certs, private_key};
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// TLS configuration for mTLS communication
pub struct TlsConfig {
    /// Client certificate chain
    client_certs: Vec<CertificateDer<'static>>,
    /// Client private key (stored as bytes for cloning)
    client_key_bytes: Vec<u8>,
    /// Key type indicator
    key_type: KeyType,
    /// Root CA certificates for verification
    root_certs: RootCertStore,
}

#[derive(Clone, Copy)]
enum KeyType {
    Pkcs1,
    Pkcs8,
    Sec1,
}

impl TlsConfig {
    /// Load TLS configuration from PEM files
    ///
    /// # Arguments
    /// * `cert_path` - Path to certificate chain PEM file
    /// * `key_path` - Path to private key PEM file
    /// * `ca_path` - Path to CA certificate PEM file
    pub fn load(
        cert_path: impl AsRef<Path>,
        key_path: impl AsRef<Path>,
        ca_path: impl AsRef<Path>,
    ) -> Result<Self, TlsError> {
        // Load certificate chain
        let cert_file = File::open(cert_path.as_ref())
            .map_err(|e| TlsError::FileRead(cert_path.as_ref().to_path_buf(), e))?;
        let mut cert_reader = BufReader::new(cert_file);
        let client_certs: Vec<CertificateDer<'static>> = certs(&mut cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(TlsError::CertParse)?;
        
        if client_certs.is_empty() {
            return Err(TlsError::NoCertificates);
        }
        
        // Load private key and store bytes
        let key_bytes = std::fs::read(key_path.as_ref())
            .map_err(|e| TlsError::FileRead(key_path.as_ref().to_path_buf(), e))?;
        
        // Parse to determine key type
        let mut key_reader = BufReader::new(&key_bytes[..]);
        let key = private_key(&mut key_reader)
            .map_err(TlsError::KeyParse)?
            .ok_or(TlsError::NoPrivateKey)?;
        
        let key_type = match &key {
            PrivateKeyDer::Pkcs1(_) => KeyType::Pkcs1,
            PrivateKeyDer::Pkcs8(_) => KeyType::Pkcs8,
            PrivateKeyDer::Sec1(_) => KeyType::Sec1,
            _ => KeyType::Pkcs8,
        };
        
        // Load CA certificates
        let ca_file = File::open(ca_path.as_ref())
            .map_err(|e| TlsError::FileRead(ca_path.as_ref().to_path_buf(), e))?;
        let mut ca_reader = BufReader::new(ca_file);
        let ca_certs: Vec<CertificateDer<'static>> = certs(&mut ca_reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(TlsError::CertParse)?;
        
        let mut root_certs = RootCertStore::empty();
        for cert in ca_certs {
            root_certs.add(cert).map_err(TlsError::RootCertAdd)?;
        }
        
        Ok(Self {
            client_certs,
            client_key_bytes: key_bytes,
            key_type,
            root_certs,
        })
    }

    fn get_private_key(&self) -> Result<PrivateKeyDer<'static>, TlsError> {
        let mut reader = BufReader::new(&self.client_key_bytes[..]);
        private_key(&mut reader)
            .map_err(TlsError::KeyParse)?
            .ok_or(TlsError::NoPrivateKey)
    }

    /// Create a TLS connector for client connections
    pub fn client_connector(&self) -> Result<TlsConnector, TlsError> {
        let key = self.get_private_key()?;
        let config = ClientConfig::builder()
            .with_root_certificates(self.root_certs.clone())
            .with_client_auth_cert(self.client_certs.clone(), key)
            .map_err(TlsError::ClientConfig)?;
        
        Ok(TlsConnector::from(Arc::new(config)))
    }

    /// Create a TLS acceptor for server connections
    pub fn server_acceptor(&self) -> Result<TlsAcceptor, TlsError> {
        let key = self.get_private_key()?;
        // For mTLS, we need client cert verification
        let client_verifier = rustls::server::WebPkiClientVerifier::builder(
            Arc::new(self.root_certs.clone())
        )
        .build()
        .map_err(TlsError::VerifierBuild)?;
        
        let config = ServerConfig::builder()
            .with_client_cert_verifier(client_verifier)
            .with_single_cert(self.client_certs.clone(), key)
            .map_err(TlsError::ServerConfig)?;
        
        Ok(TlsAcceptor::from(Arc::new(config)))
    }
}

/// TLS configuration errors
#[derive(Debug)]
pub enum TlsError {
    /// Failed to read file
    FileRead(std::path::PathBuf, std::io::Error),
    /// Failed to parse certificates
    CertParse(std::io::Error),
    /// Failed to parse private key
    KeyParse(std::io::Error),
    /// No certificates found in file
    NoCertificates,
    /// No private key found in file
    NoPrivateKey,
    /// Failed to add root certificate
    RootCertAdd(rustls::Error),
    /// Failed to build client config
    ClientConfig(rustls::Error),
    /// Failed to build server config
    ServerConfig(rustls::Error),
    /// Failed to build verifier
    VerifierBuild(rustls::server::VerifierBuilderError),
}

impl std::fmt::Display for TlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileRead(path, e) => write!(f, "failed to read {}: {}", path.display(), e),
            Self::CertParse(e) => write!(f, "failed to parse certificate: {}", e),
            Self::KeyParse(e) => write!(f, "failed to parse private key: {}", e),
            Self::NoCertificates => write!(f, "no certificates found in file"),
            Self::NoPrivateKey => write!(f, "no private key found in file"),
            Self::RootCertAdd(e) => write!(f, "failed to add root certificate: {}", e),
            Self::ClientConfig(e) => write!(f, "failed to build client config: {}", e),
            Self::ServerConfig(e) => write!(f, "failed to build server config: {}", e),
            Self::VerifierBuild(e) => write!(f, "failed to build verifier: {}", e),
        }
    }
}

impl std::error::Error for TlsError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_error_display() {
        let err = TlsError::NoCertificates;
        assert_eq!(err.to_string(), "no certificates found in file");
        
        let err = TlsError::NoPrivateKey;
        assert_eq!(err.to_string(), "no private key found in file");
    }
}
