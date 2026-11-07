//! 证书身份：待签发的域名列表。

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 一份证书要覆盖的 DNS 名集合。首个域名为 common name / 主标识。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertificateIdentity {
    domains: Vec<String>,
}

impl CertificateIdentity {
    /// 由一个或多个域名构造。空列表或空白名会失败。
    pub fn new(domains: impl IntoIterator<Item = impl Into<String>>) -> Result<Self> {
        let mut normalized = Vec::new();
        for domain in domains {
            let domain = domain.into().trim().to_ascii_lowercase();
            if domain.is_empty() {
                return Err(Error::invalid_input("domain must not be empty"));
            }
            if domain.contains('/') || domain.contains('\\') || domain.contains(' ') {
                return Err(Error::invalid_input(format!("domain `{domain}` contains illegal characters")));
            }
            if !normalized.iter().any(|existing| existing == &domain) {
                normalized.push(domain);
            }
        }
        if normalized.is_empty() {
            return Err(Error::invalid_input("at least one domain is required"));
        }
        Ok(Self { domains: normalized })
    }

    /// 主域名（列表第一项）。
    pub fn primary(&self) -> &str {
        &self.domains[0]
    }

    /// 全部域名（去重后的顺序）。
    pub fn domains(&self) -> &[String] {
        &self.domains
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_dedupes_domains() {
        let identity = CertificateIdentity::new(["Example.COM", " www.example.com ", "example.com"]).unwrap();
        assert_eq!(identity.primary(), "example.com");
        assert_eq!(identity.domains(), &["example.com".to_string(), "www.example.com".to_string()]);
    }

    #[test]
    fn rejects_empty_identity() {
        let err = CertificateIdentity::new(Vec::<String>::new()).unwrap_err();
        assert_eq!(err.code(), "invalid_input");
    }
}
