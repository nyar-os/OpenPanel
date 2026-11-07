//! `auto-ssl`：OpenPanel 站点免费 SSL（ACME / Let's Encrypt 类）申请与续期能力库。
//!
//! 本 crate 由 `open-panel server` 调用，负责证书身份、HTTP-01 挑战投放钩子、
//! 签发/续期阈值判断，以及把证书与私钥写入约定目录。DNS-01 仅保留 trait stub。

#![deny(missing_docs)]

mod acme;
mod challenge;
mod error;
mod identity;
mod issue;
mod pem;
mod renew;
mod store;

pub use acme::{AcmeDirectory, AcmeHttp01Issuer};
pub use challenge::{
    ChallengeKind, Dns01ChallengeHook, Http01Challenge, Http01ChallengeHook, MemoryHttp01Hook, UnsupportedDns01, WellKnownDirHook,
};
pub use error::{Error, Result};
pub use identity::CertificateIdentity;
pub use issue::{CertificateIssuer, CertificateManager, EnsureOutcome, MockIssuer};
pub use renew::{RenewalPolicy, needs_renewal, time_remaining};
pub use store::{CertStore, CertificateMeta, StoredCertificate, directory_id_for, files};
