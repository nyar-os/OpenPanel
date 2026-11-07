//! 证书与私钥的约定目录布局。

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    error::{Error, Result},
    identity::CertificateIdentity,
    pem,
};

/// 站点证书目录下的固定文件名。
pub mod files {
    /// 叶子证书 PEM。
    pub const CERT: &str = "cert.pem";
    /// 完整链（叶子 + 中间）PEM。
    pub const FULLCHAIN: &str = "fullchain.pem";
    /// 私钥 PEM。
    pub const PRIVKEY: &str = "privkey.pem";
    /// 签发元数据 JSON。
    pub const META: &str = "meta.json";
    /// ACME 账户凭据 JSON。
    pub const ACCOUNT: &str = "credentials.json";
}

/// 磁盘上的一份已签发材料。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredCertificate {
    /// 证书身份。
    pub identity: CertificateIdentity,
    /// 叶子证书 PEM。
    pub cert_pem: String,
    /// 完整链 PEM。
    pub fullchain_pem: String,
    /// 私钥 PEM。
    pub private_key_pem: String,
    /// 叶子证书 `notAfter`。
    pub not_after: SystemTime,
}

/// 写入 `meta.json` 的元数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificateMeta {
    /// 域名列表。
    pub domains: Vec<String>,
    /// `notAfter` 的 unix 秒。
    pub not_after_unix: u64,
}

impl CertificateMeta {
    /// 从材料构造。
    pub fn from_stored(stored: &StoredCertificate) -> Result<Self> {
        Ok(Self { domains: stored.identity.domains().to_vec(), not_after_unix: system_time_to_unix(stored.not_after)? })
    }

    /// 还原 `notAfter`。
    pub fn not_after(&self) -> Result<SystemTime> {
        unix_to_system_time(self.not_after_unix)
    }
}

/// 约定布局：
///
/// ```text
/// {root}/
///   sites/{site_id}/cert.pem
///   sites/{site_id}/fullchain.pem
///   sites/{site_id}/privkey.pem
///   sites/{site_id}/meta.json
///   accounts/{directory_id}/credentials.json
/// ```
#[derive(Debug, Clone)]
pub struct CertStore {
    root: PathBuf,
}

impl CertStore {
    /// 以根目录构造。
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 存储根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 站点证书目录。
    pub fn site_dir(&self, site_id: &str) -> Result<PathBuf> {
        Ok(self.root.join("sites").join(validate_segment("site_id", site_id)?))
    }

    /// 账户凭据目录（按 ACME directory URL 的稳定 id）。
    pub fn account_dir(&self, directory_id: &str) -> Result<PathBuf> {
        Ok(self.root.join("accounts").join(validate_segment("directory_id", directory_id)?))
    }

    /// 账户凭据路径。
    pub fn account_credentials_path(&self, directory_id: &str) -> Result<PathBuf> {
        Ok(self.account_dir(directory_id)?.join(files::ACCOUNT))
    }

    /// 写入证书材料（同目录原子性尽力：先写临时文件再 rename）。
    pub fn write_certificate(&self, site_id: &str, material: &StoredCertificate) -> Result<PathBuf> {
        let dir = self.site_dir(site_id)?;
        fs::create_dir_all(&dir).map_err(|source| Error::io(&dir, source))?;

        write_text(&dir.join(files::CERT), &material.cert_pem)?;
        write_text(&dir.join(files::FULLCHAIN), &material.fullchain_pem)?;
        write_text(&dir.join(files::PRIVKEY), &material.private_key_pem)?;

        let meta = CertificateMeta::from_stored(material)?;
        let meta_json = serde_json::to_string_pretty(&meta)?;
        write_text(&dir.join(files::META), &meta_json)?;
        Ok(dir)
    }

    /// 读取已存在的证书材料。
    pub fn read_certificate(&self, site_id: &str) -> Result<StoredCertificate> {
        let dir = self.site_dir(site_id)?;
        let cert_pem = read_text(&dir.join(files::CERT))?;
        let fullchain_pem = read_text(&dir.join(files::FULLCHAIN))?;
        let private_key_pem = read_text(&dir.join(files::PRIVKEY))?;
        let meta_path = dir.join(files::META);
        let not_after = if meta_path.exists() {
            let meta: CertificateMeta = serde_json::from_str(&read_text(&meta_path)?)?;
            meta.not_after()?
        }
        else {
            pem::leaf_not_after(&cert_pem)?
        };
        let identity = if meta_path.exists() {
            let meta: CertificateMeta = serde_json::from_str(&read_text(&meta_path)?)?;
            CertificateIdentity::new(meta.domains)?
        }
        else {
            CertificateIdentity::new([pem::leaf_common_name(&cert_pem)?])?
        };
        Ok(StoredCertificate { identity, cert_pem, fullchain_pem, private_key_pem, not_after })
    }

    /// 是否已有完整材料。
    pub fn has_certificate(&self, site_id: &str) -> Result<bool> {
        let dir = self.site_dir(site_id)?;
        Ok(dir.join(files::FULLCHAIN).is_file() && dir.join(files::PRIVKEY).is_file())
    }

    /// 写入 ACME 账户凭据 JSON。
    pub fn write_account_credentials(&self, directory_id: &str, json: &str) -> Result<PathBuf> {
        let dir = self.account_dir(directory_id)?;
        fs::create_dir_all(&dir).map_err(|source| Error::io(&dir, source))?;
        let path = dir.join(files::ACCOUNT);
        write_text(&path, json)?;
        Ok(path)
    }

    /// 读取 ACME 账户凭据 JSON。
    pub fn read_account_credentials(&self, directory_id: &str) -> Result<Option<String>> {
        let path = self.account_credentials_path(directory_id)?;
        if !path.is_file() {
            return Ok(None);
        }
        Ok(Some(read_text(&path)?))
    }
}

/// 由 directory URL 生成稳定、可作目录名的 id。
pub fn directory_id_for(directory_url: &str) -> String {
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
    };

    let mut hasher = DefaultHasher::new();
    directory_url.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn validate_segment(label: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::invalid_input(format!("{label} must not be empty")));
    }
    if value.contains("..") || value.contains('/') || value.contains('\\') || value.contains('\0') {
        return Err(Error::invalid_input(format!("{label} contains illegal path characters")));
    }
    Ok(value.to_string())
}

fn write_text(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, content).map_err(|source| Error::io(&tmp, source))?;
    fs::rename(&tmp, path).map_err(|source| Error::io(path, source))?;
    Ok(())
}

fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|source| Error::io(path, source))
}

fn system_time_to_unix(time: SystemTime) -> Result<u64> {
    Ok(time.duration_since(UNIX_EPOCH).map_err(|_| Error::cert_parse("not_after before unix epoch"))?.as_secs())
}

fn unix_to_system_time(secs: u64) -> Result<SystemTime> {
    Ok(UNIX_EPOCH + Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renew::{RenewalPolicy, needs_renewal};

    #[test]
    fn layout_paths_are_stable() {
        let store = CertStore::new("/tmp/ssl-root");
        assert_eq!(store.site_dir("site-a").unwrap(), PathBuf::from("/tmp/ssl-root/sites/site-a"));
        assert_eq!(store.account_credentials_path("abcd").unwrap(), PathBuf::from("/tmp/ssl-root/accounts/abcd/credentials.json"));
    }

    #[test]
    fn rejects_path_traversal_site_id() {
        let store = CertStore::new("/tmp/ssl-root");
        let err = store.site_dir("../etc").unwrap_err();
        assert_eq!(err.code(), "invalid_input");
    }

    #[test]
    fn write_and_read_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = CertStore::new(dir.path());
        let identity = CertificateIdentity::new(["example.com"]).unwrap();
        let not_after = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let material = StoredCertificate {
            identity,
            cert_pem: "-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----\n".into(),
            fullchain_pem: "-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----\n".into(),
            private_key_pem: "-----BEGIN PRIVATE KEY-----\nKEY\n-----END PRIVATE KEY-----\n".into(),
            not_after,
        };
        store.write_certificate("blog", &material).unwrap();
        let loaded = store.read_certificate("blog").unwrap();
        assert_eq!(loaded.identity.primary(), "example.com");
        assert_eq!(loaded.private_key_pem, material.private_key_pem);
        assert!(!needs_renewal(loaded.not_after, SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000), &RenewalPolicy::default(),));
    }

    #[test]
    fn directory_id_is_stable() {
        let a = directory_id_for("https://acme-v02.api.letsencrypt.org/directory");
        let b = directory_id_for("https://acme-v02.api.letsencrypt.org/directory");
        assert_eq!(a, b);
        assert_ne!(a, directory_id_for("https://acme-staging-v02.api.letsencrypt.org/directory"));
    }
}
