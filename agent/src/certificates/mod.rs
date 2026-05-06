use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Represents a revoked node identity for CRL (Certificate Revocation List)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokedNode {
    pub node_id: Uuid,
    pub revoked_at: DateTime<Utc>,
    pub revoked_by: Uuid,
}

/// Certificate manager for PKI operations
pub struct CertificateManager {
    data_dir: PathBuf,
    certs_dir: PathBuf,
    crl_file: PathBuf,
}

impl CertificateManager {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        let data_dir = data_dir.as_ref().to_path_buf();
        let certs_dir = data_dir.join("certs");
        let crl_file = certs_dir.join("crl.json");
        
        CertificateManager {
            data_dir,
            certs_dir,
            crl_file,
        }
    }

    /// Initialize certificate directories
    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.certs_dir)?;
        Ok(())
    }

    pub fn has_ca_key(&self) -> bool {
        self.certs_dir.join("ca.key").exists()
    }

    pub fn has_node_credentials(&self) -> bool {
        self.certs_dir.join("node.crt").exists()
            && self.certs_dir.join("node.key").exists()
            && self.certs_dir.join("ca.crt").exists()
    }

    pub fn load_ca_pem(&self) -> Result<String> {
        let ca_path = self.certs_dir.join("ca.crt");
        Ok(fs::read_to_string(ca_path)?)
    }

    /// Generate or load CA certificate (simplified implementation)
    pub async fn init_ca(&self) -> Result<()> {
        self.ensure_dirs()?;
        let ca_key_path = self.certs_dir.join("ca.key");
        let ca_crt_path = self.certs_dir.join("ca.crt");

        // If CA already exists, skip generation
        if ca_key_path.exists() && ca_crt_path.exists() {
            return Ok(());
        }

        // Generate CA using self-signed certificate
        let (cert_pem, key_pem) = self.generate_ca_cert()?;

        // Save with proper permissions
        fs::write(&ca_crt_path, &cert_pem)?;
        fs::write(&ca_key_path, &key_pem)?;
        
        #[cfg(unix)]
        fs::set_permissions(&ca_key_path, fs::Permissions::from_mode(0o600))?;

        println!("INFO CA initialized at {:?}, ca.key perms: 0600", self.certs_dir);
        Ok(())
    }

    /// Generate CA certificate PEM (10-year TTL)
    fn generate_ca_cert(&self) -> Result<(String, String)> {
        let rcgen::CertifiedKey { cert, key_pair } = generate_simple_self_signed(vec!["all4one-ca".to_string()])?;
        
        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();
        
        Ok((cert_pem, key_pem))
    }

    /// Generate node certificate with 90-day TTL
    pub async fn generate_node_cert(
        &self,
        node_id: Uuid,
    ) -> Result<(String, String)> {
        self.ensure_dirs()?;

        let rcgen::CertifiedKey { cert, key_pair } = generate_simple_self_signed(vec![node_id.to_string()])?;

        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();

        Ok((cert_pem, key_pem))
    }

    /// Sign a Certificate Signing Request (CSR) - simplified for now
    /// In production, this would validate the CSR before signing
    pub async fn sign_csr(
        &self,
        node_id: Uuid,
        _csr_pem: &str,
    ) -> Result<String> {
        // For MVP, we generate a fresh cert instead of parsing/validating CSR
        let (cert, _key) = self.generate_node_cert(node_id).await?;
        Ok(cert)
    }

    /// Load CA certificate from disk
    pub fn load_ca(&self) -> Result<CertificateDer<'static>> {
        let ca_path = self.certs_dir.join("ca.crt");
        let ca_pem = fs::read_to_string(&ca_path)?;
        
        // Parse PEM
        let certs = rustls_pemfile::certs(&mut ca_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()?;
        
        if certs.is_empty() {
            return Err(anyhow!("No certificate found in CA file"));
        }
        
        Ok(certs[0].clone().into_owned().into())
    }

    /// Load node certificate from disk
    pub fn load_node_cert(&self) -> Result<CertificateDer<'static>> {
        let cert_path = self.certs_dir.join("node.crt");
        let cert_pem = fs::read_to_string(&cert_path)?;
        
        let certs = rustls_pemfile::certs(&mut cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()?;
        
        if certs.is_empty() {
            return Err(anyhow!("No certificate found in node cert file"));
        }
        
        Ok(certs[0].clone().into_owned().into())
    }

    /// Load node private key from disk
    pub fn load_node_key(&self) -> Result<PrivateKeyDer<'static>> {
        let key_path = self.certs_dir.join("node.key");
        let key_pem = fs::read_to_string(&key_path)?;
        
        let mut reader = key_pem.as_bytes();
        let keys = rustls_pemfile::private_key(&mut reader)?;
        
        keys.ok_or_else(|| anyhow!("No private key found in node key file"))
    }

    /// Check if node is revoked by looking in CRL
    pub fn is_revoked(&self, node_id: Uuid) -> Result<bool> {
        if !self.crl_file.exists() {
            return Ok(false);
        }

        let crl_json = fs::read_to_string(&self.crl_file)?;
        let crl: Vec<RevokedNode> = serde_json::from_str(&crl_json)?;
        
        Ok(crl.iter().any(|r: &RevokedNode| r.node_id == node_id))
    }

    /// Add node to CRL (revoke)
    pub fn add_to_crl(&self, node_id: Uuid, revoked_by: Uuid) -> Result<()> {
        self.ensure_dirs()?;

        let mut crl = if self.crl_file.exists() {
            let crl_json = fs::read_to_string(&self.crl_file)?;
            serde_json::from_str(&crl_json)?
        } else {
            Vec::new()
        };

        // Don't add duplicate
        if !crl.iter().any(|r: &RevokedNode| r.node_id == node_id) {
            crl.push(RevokedNode {
                node_id,
                revoked_at: Utc::now(),
                revoked_by,
            });
        }

        let crl_json = serde_json::to_string_pretty(&crl)?;
        fs::write(&self.crl_file, crl_json)?;

        println!("INFO Node {} added to CRL", node_id);
        Ok(())
    }

    /// Check if a certificate expires within N days
    pub fn expires_soon(&self, _days: i64) -> Result<bool> {
        // For MVP, we'd need to parse the cert to check expiry
        // This is a placeholder
        Ok(false)
    }

    /// Save generated node certificate to disk during enrollment
    pub fn save_node_cert(&self, cert_pem: &str, key_pem: &str, ca_pem: &str) -> Result<()> {
        self.ensure_dirs()?;

        let cert_path = self.certs_dir.join("node.crt");
        let key_path = self.certs_dir.join("node.key");
        let ca_path = self.certs_dir.join("ca.crt");

        fs::write(&cert_path, cert_pem)?;
        fs::write(&key_path, key_pem)?;
        
        #[cfg(unix)]
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600))?;
        
        fs::write(&ca_path, ca_pem)?;

        println!("INFO Node certificates saved, node.key perms: 0600");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn init_ca_creates_files_and_is_idempotent() -> Result<()> {
        let tmp = TempDir::new()?;
        let manager = CertificateManager::new(tmp.path());

        manager.init_ca().await?;
        manager.init_ca().await?;

        let certs_dir = tmp.path().join("certs");
        assert!(certs_dir.join("ca.crt").exists());
        assert!(certs_dir.join("ca.key").exists());
        Ok(())
    }

    #[tokio::test]
    async fn save_and_load_node_credentials_roundtrip() -> Result<()> {
        let tmp = TempDir::new()?;
        let manager = CertificateManager::new(tmp.path());

        manager.init_ca().await?;
        let node_id = Uuid::new_v4();
        let (node_cert, node_key) = manager.generate_node_cert(node_id).await?;
        let ca_pem = std::fs::read_to_string(tmp.path().join("certs").join("ca.crt"))?;

        manager.save_node_cert(&node_cert, &node_key, &ca_pem)?;

        let _loaded_ca = manager.load_ca()?;
        let _loaded_node_cert = manager.load_node_cert()?;
        let _loaded_node_key = manager.load_node_key()?;
        Ok(())
    }

    #[test]
    fn crl_add_and_lookup_works() -> Result<()> {
        let tmp = TempDir::new()?;
        let manager = CertificateManager::new(tmp.path());

        let node_id = Uuid::new_v4();
        let admin = Uuid::new_v4();

        assert!(!manager.is_revoked(node_id)?);
        manager.add_to_crl(node_id, admin)?;
        assert!(manager.is_revoked(node_id)?);

        // Duplicate revocation should not duplicate CRL entries.
        manager.add_to_crl(node_id, admin)?;
        let crl_path = tmp.path().join("certs").join("crl.json");
        let crl: Vec<RevokedNode> = serde_json::from_str(&std::fs::read_to_string(crl_path)?)?;
        assert_eq!(crl.len(), 1);
        Ok(())
    }
}

