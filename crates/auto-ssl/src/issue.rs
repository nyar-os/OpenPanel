//! 签发编排：按阈值决定续期，并委托具体 Issuer。

use std::time::SystemTime;

use crate::{
    challenge::Http01ChallengeHook,
    error::{Error, Result},
    identity::CertificateIdentity,
    renew::{RenewalPolicy, needs_renewal},
    store::{CertStore, StoredCertificate},
};

/// 签发结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnsureOutcome {
    /// 现有证书仍在阈值外，未改动。
    Reused(StoredCertificate),
    /// 新签发或已续期并写入存储。
    Issued(StoredCertificate),
}

/// 具体 ACME / mock 签发后端。
pub trait CertificateIssuer: Send + Sync {
    /// 为身份签发新证书（HTTP-01）。
    fn issue_http01<H: Http01ChallengeHook + Sync>(
        &self,
        identity: &CertificateIdentity,
        hook: &H,
    ) -> impl std::future::Future<Output = Result<StoredCertificate>> + Send;
}

/// 把签发、续期判断与磁盘布局串起来的门面。
#[derive(Debug, Clone)]
pub struct CertificateManager {
    store: CertStore,
    policy: RenewalPolicy,
}

impl CertificateManager {
    /// 使用默认 30 天续期阈值。
    pub fn new(store: CertStore) -> Self {
        Self { store, policy: RenewalPolicy::default() }
    }

    /// 自定义续期策略。
    pub fn with_policy(store: CertStore, policy: RenewalPolicy) -> Self {
        Self { store, policy }
    }

    /// 底层存储。
    pub fn store(&self) -> &CertStore {
        &self.store
    }

    /// 续期策略。
    pub fn policy(&self) -> &RenewalPolicy {
        &self.policy
    }

    /// 若无证书或进入阈值则签发；否则复用。
    pub async fn ensure_http01<I, H>(
        &self,
        site_id: &str,
        identity: &CertificateIdentity,
        issuer: &I,
        hook: &H,
        now: SystemTime,
    ) -> Result<EnsureOutcome>
    where
        I: CertificateIssuer + Sync,
        H: Http01ChallengeHook + Sync,
    {
        if self.store.has_certificate(site_id)? {
            match self.store.read_certificate(site_id) {
                Ok(existing) if !needs_renewal(existing.not_after, now, &self.policy) => {
                    return Ok(EnsureOutcome::Reused(existing));
                }
                Ok(_) | Err(Error::MaterialMissing(_)) | Err(Error::CertParse(_)) | Err(Error::Io { .. }) => {}
                Err(error) => return Err(error),
            }
        }

        let issued = issuer.issue_http01(identity, hook).await?;
        self.store.write_certificate(site_id, &issued)?;
        Ok(EnsureOutcome::Issued(issued))
    }
}

/// 测试用 Issuer：返回预置材料，并可选记录调用次数。
#[derive(Debug, Default)]
pub struct MockIssuer {
    /// 预置签发材料。
    pub material: Option<StoredCertificate>,
    /// 调用计数。
    pub calls: std::sync::Mutex<u32>,
}

impl MockIssuer {
    /// 固定返回给定材料。
    pub fn returning(material: StoredCertificate) -> Self {
        Self { material: Some(material), calls: std::sync::Mutex::new(0) }
    }

    /// 已调用次数。
    pub fn call_count(&self) -> u32 {
        *self.calls.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl CertificateIssuer for MockIssuer {
    async fn issue_http01<H: Http01ChallengeHook + Sync>(&self, identity: &CertificateIdentity, _hook: &H) -> Result<StoredCertificate> {
        *self.calls.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
        let mut material = self.material.clone().ok_or_else(|| Error::not_implemented("mock issuer has no material"))?;
        material.identity = identity.clone();
        Ok(material)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{challenge::MemoryHttp01Hook, renew::RenewalPolicy};

    fn sample_material(not_after: SystemTime) -> StoredCertificate {
        StoredCertificate {
            identity: CertificateIdentity::new(["example.com"]).unwrap(),
            cert_pem: "-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----\n".into(),
            fullchain_pem: "-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----\n".into(),
            private_key_pem: "-----BEGIN PRIVATE KEY-----\nKEY\n-----END PRIVATE KEY-----\n".into(),
            not_after,
        }
    }

    #[tokio::test]
    async fn ensure_reuses_fresh_cert() {
        let dir = tempfile::tempdir().unwrap();
        let store = CertStore::new(dir.path());
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let material = sample_material(now + Duration::from_secs(60 * 24 * 60 * 60));
        store.write_certificate("site", &material).unwrap();

        let manager = CertificateManager::with_policy(store, RenewalPolicy::renew_before(Duration::from_secs(30 * 24 * 60 * 60)));
        let issuer = MockIssuer::returning(sample_material(now + Duration::from_secs(90 * 24 * 60 * 60)));
        let hook = MemoryHttp01Hook::new();
        let identity = CertificateIdentity::new(["example.com"]).unwrap();

        let outcome = manager.ensure_http01("site", &identity, &issuer, &hook, now).await.unwrap();
        assert!(matches!(outcome, EnsureOutcome::Reused(_)));
        assert_eq!(issuer.call_count(), 0);
    }

    #[tokio::test]
    async fn ensure_issues_when_inside_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let store = CertStore::new(dir.path());
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let stale = sample_material(now + Duration::from_secs(5 * 24 * 60 * 60));
        store.write_certificate("site", &stale).unwrap();

        let manager = CertificateManager::with_policy(store, RenewalPolicy::renew_before(Duration::from_secs(30 * 24 * 60 * 60)));
        let fresh = sample_material(now + Duration::from_secs(90 * 24 * 60 * 60));
        let issuer = MockIssuer::returning(fresh.clone());
        let hook = MemoryHttp01Hook::new();
        let identity = CertificateIdentity::new(["example.com"]).unwrap();

        let outcome = manager.ensure_http01("site", &identity, &issuer, &hook, now).await.unwrap();
        assert!(matches!(outcome, EnsureOutcome::Issued(_)));
        assert_eq!(issuer.call_count(), 1);
        assert_eq!(manager.store().read_certificate("site").unwrap().not_after, fresh.not_after);
    }
}
