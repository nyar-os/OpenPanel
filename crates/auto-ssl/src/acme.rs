//! 基于 `instant-acme` 的 HTTP-01 签发实现。

use instant_acme::{Account, AuthorizationStatus, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder, OrderStatus, RetryPolicy};

use crate::{
    challenge::{Http01Challenge, Http01ChallengeHook},
    error::{Error, Result},
    identity::CertificateIdentity,
    issue::CertificateIssuer,
    pem,
    store::{CertStore, StoredCertificate, directory_id_for},
};

/// ACME 目录入口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcmeDirectory {
    /// Let's Encrypt 生产环境。
    LetsEncryptProduction,
    /// Let's Encrypt 预发环境。
    LetsEncryptStaging,
    /// 自定义 directory URL（例如 Pebble）。
    Custom(String),
}

impl AcmeDirectory {
    /// Directory URL。
    pub fn url(&self) -> &str {
        match self {
            Self::LetsEncryptProduction => LetsEncrypt::Production.url(),
            Self::LetsEncryptStaging => LetsEncrypt::Staging.url(),
            Self::Custom(url) => url.as_str(),
        }
    }

    /// 稳定目录 id，用于账户凭据分目录存放。
    pub fn id(&self) -> String {
        directory_id_for(self.url())
    }
}

/// HTTP-01 ACME 签发器。
#[derive(Debug, Clone)]
pub struct AcmeHttp01Issuer {
    directory: AcmeDirectory,
    contacts: Vec<String>,
    store: CertStore,
    accept_terms_of_service: bool,
}

impl AcmeHttp01Issuer {
    /// 构造签发器。账户凭据读写走 `store` 的 `accounts/` 布局。
    pub fn new(directory: AcmeDirectory, store: CertStore) -> Self {
        Self { directory, contacts: Vec::new(), store, accept_terms_of_service: true }
    }

    /// 附加 `mailto:` 联系人（可多次）。
    pub fn contact_email(mut self, email: impl Into<String>) -> Self {
        let email = email.into();
        if !email.is_empty() {
            self.contacts.push(format!("mailto:{email}"));
        }
        self
    }

    /// 是否同意 CA 服务条款（Let's Encrypt 需要为 true）。
    pub fn accept_terms_of_service(mut self, accept: bool) -> Self {
        self.accept_terms_of_service = accept;
        self
    }

    /// 使用的 ACME directory。
    pub fn directory(&self) -> &AcmeDirectory {
        &self.directory
    }

    async fn load_or_create_account(&self) -> Result<Account> {
        let directory_id = self.directory.id();
        let builder = Account::builder().map_err(|error| Error::acme(error.to_string()))?;

        if let Some(json) = self.store.read_account_credentials(&directory_id)? {
            let credentials = serde_json::from_str(&json)?;
            return builder.from_credentials(credentials).await.map_err(|error| Error::acme(error.to_string()));
        }

        let contact: Vec<&str> = self.contacts.iter().map(String::as_str).collect();
        let (account, credentials) = builder
            .create(
                &NewAccount { contact: &contact, terms_of_service_agreed: self.accept_terms_of_service, only_return_existing: false },
                self.directory.url().to_string(),
                None,
            )
            .await
            .map_err(|error| Error::acme(error.to_string()))?;

        let json = serde_json::to_string_pretty(&credentials)?;
        self.store.write_account_credentials(&directory_id, &json)?;
        Ok(account)
    }

    async fn issue_with_hook<H: Http01ChallengeHook + Sync>(&self, identity: &CertificateIdentity, hook: &H) -> Result<StoredCertificate> {
        let account = self.load_or_create_account().await?;
        let identifiers: Vec<Identifier> = identity.domains().iter().cloned().map(Identifier::Dns).collect();
        let mut order = account.new_order(&NewOrder::new(&identifiers)).await.map_err(|error| Error::acme(error.to_string()))?;

        let mut placed: Vec<Http01Challenge> = Vec::new();
        let issue_result = async {
            let mut authorizations = order.authorizations();
            while let Some(result) = authorizations.next().await {
                let mut authz = result.map_err(|error| Error::acme(error.to_string()))?;
                match authz.status {
                    AuthorizationStatus::Pending => {}
                    AuthorizationStatus::Valid => continue,
                    other => {
                        return Err(Error::acme(format!("unexpected authorization status: {other:?}")));
                    }
                }

                let domain = match authz.identifier().identifier.clone() {
                    Identifier::Dns(name) => name,
                    other => {
                        return Err(Error::acme(format!("unsupported identifier: {other:?}")));
                    }
                };

                let mut challenge = authz.challenge(ChallengeType::Http01).ok_or_else(|| Error::acme("CA did not offer http-01"))?;
                let token = challenge.token.clone();
                if token.is_empty() {
                    return Err(Error::acme("http-01 challenge missing token"));
                }
                let key_authorization = challenge.key_authorization().as_str().to_string();
                let material = Http01Challenge { domain: domain.clone(), token, key_authorization };
                hook.place(&material).await.map_err(|error| Error::challenge_hook(format!("place http-01 for {domain}: {error}")))?;
                placed.push(material);
                challenge.set_ready().await.map_err(|error| Error::acme(error.to_string()))?;
            }

            let status = order.poll_ready(&RetryPolicy::default()).await.map_err(|error| Error::acme(error.to_string()))?;
            if status != OrderStatus::Ready {
                return Err(Error::acme(format!("order not ready: {status:?}")));
            }

            let private_key_pem = order.finalize().await.map_err(|error| Error::acme(error.to_string()))?;
            let fullchain_pem = order.poll_certificate(&RetryPolicy::default()).await.map_err(|error| Error::acme(error.to_string()))?;
            let (cert_pem, _chain) = pem::split_leaf_and_chain(&fullchain_pem)?;
            let not_after = pem::leaf_not_after(&cert_pem)?;

            Ok(StoredCertificate { identity: identity.clone(), cert_pem, fullchain_pem, private_key_pem, not_after })
        }
        .await;

        for challenge in placed.iter().rev() {
            let _ = hook.clear(challenge).await;
        }

        issue_result
    }
}

impl CertificateIssuer for AcmeHttp01Issuer {
    async fn issue_http01<H: Http01ChallengeHook + Sync>(&self, identity: &CertificateIdentity, hook: &H) -> Result<StoredCertificate> {
        self.issue_with_hook(identity, hook).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lets_encrypt_urls_are_https() {
        assert!(AcmeDirectory::LetsEncryptProduction.url().starts_with("https://"));
        assert!(AcmeDirectory::LetsEncryptStaging.url().starts_with("https://"));
        assert_ne!(AcmeDirectory::LetsEncryptProduction.id(), AcmeDirectory::LetsEncryptStaging.id());
    }
}
