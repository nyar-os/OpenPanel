//! 挑战类型与投放钩子。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// ACME 挑战类型。DNS-01 仅预留接口，完整实现未闭合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChallengeKind {
    /// HTTP-01：由站点响应 `/.well-known/acme-challenge/{token}`。
    Http01,
    /// DNS-01：写 `_acme-challenge` TXT（当前为 stub）。
    Dns01,
}

/// HTTP-01 挑战材料，供 OpenPanel 投放到站点根。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Http01Challenge {
    /// 该授权对应的域名。
    pub domain: String,
    /// ACME token。
    pub token: String,
    /// 响应体：key authorization。
    pub key_authorization: String,
}

impl Http01Challenge {
    /// 相对站点根的 well-known URL 路径。
    pub fn well_known_path(&self) -> String {
        format!("/.well-known/acme-challenge/{}", self.token)
    }

    /// 相对站点文档根的文件系统相对路径（使用 `/`）。
    pub fn relative_file_path(&self) -> String {
        format!(".well-known/acme-challenge/{}", self.token)
    }
}

/// OpenPanel（或测试）实现此钩子，在 CA 校验前投放 / 清理 HTTP-01 响应。
pub trait Http01ChallengeHook: Send + Sync {
    /// 投放挑战，使 `well_known_path` 可返回 `key_authorization`。
    fn place(&self, challenge: &Http01Challenge) -> impl std::future::Future<Output = Result<()>> + Send;

    /// 清理已投放的挑战材料。
    fn clear(&self, challenge: &Http01Challenge) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// DNS-01 钩子预留。默认实现直接返回 `not_implemented`。
pub trait Dns01ChallengeHook: Send + Sync {
    /// 发布 TXT：`record_name` 通常为 `_acme-challenge.example.com`。
    fn place_txt(&self, record_name: &str, value: &str) -> impl std::future::Future<Output = Result<()>> + Send;

    /// 删除此前发布的 TXT。
    fn clear_txt(&self, record_name: &str) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// 明确拒绝 DNS-01 的 stub。
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedDns01;

impl Dns01ChallengeHook for UnsupportedDns01 {
    async fn place_txt(&self, _record_name: &str, _value: &str) -> Result<()> {
        Err(Error::not_implemented("dns-01 challenge is not implemented yet"))
    }

    async fn clear_txt(&self, _record_name: &str) -> Result<()> {
        Err(Error::not_implemented("dns-01 challenge is not implemented yet"))
    }
}

/// 内存 HTTP-01 表：OpenPanel 可把 Web 服务器指到 `response_for`。
#[derive(Debug, Default)]
pub struct MemoryHttp01Hook {
    entries: Mutex<HashMap<String, String>>,
}

impl MemoryHttp01Hook {
    /// 新建空表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 按 token 查询 key authorization。
    pub fn response_for(&self, token: &str) -> Option<String> {
        self.entries.lock().ok()?.get(token).cloned()
    }

    /// 当前投放条数（测试用）。
    pub fn len(&self) -> usize {
        self.entries.lock().map(|guard| guard.len()).unwrap_or(0)
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Http01ChallengeHook for MemoryHttp01Hook {
    async fn place(&self, challenge: &Http01Challenge) -> Result<()> {
        let mut guard = self.entries.lock().map_err(|_| Error::challenge_hook("memory hook poisoned"))?;
        guard.insert(challenge.token.clone(), challenge.key_authorization.clone());
        Ok(())
    }

    async fn clear(&self, challenge: &Http01Challenge) -> Result<()> {
        let mut guard = self.entries.lock().map_err(|_| Error::challenge_hook("memory hook poisoned"))?;
        guard.remove(&challenge.token);
        Ok(())
    }
}

/// 把挑战写到站点文档根下的 `.well-known/acme-challenge/`。
#[derive(Debug, Clone)]
pub struct WellKnownDirHook {
    web_root: PathBuf,
}

impl WellKnownDirHook {
    /// `web_root` 为站点文档根目录。
    pub fn new(web_root: impl Into<PathBuf>) -> Self {
        Self { web_root: web_root.into() }
    }

    /// 站点文档根。
    pub fn web_root(&self) -> &Path {
        &self.web_root
    }

    fn challenge_path(&self, challenge: &Http01Challenge) -> PathBuf {
        self.web_root.join(".well-known").join("acme-challenge").join(&challenge.token)
    }
}

impl Http01ChallengeHook for WellKnownDirHook {
    async fn place(&self, challenge: &Http01Challenge) -> Result<()> {
        let path = self.challenge_path(challenge);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::io(parent, source))?;
        }
        std::fs::write(&path, challenge.key_authorization.as_bytes()).map_err(|source| Error::io(&path, source))?;
        Ok(())
    }

    async fn clear(&self, challenge: &Http01Challenge) -> Result<()> {
        let path = self.challenge_path(challenge);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::io(path, source)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http01_paths_match_acme_layout() {
        let challenge = Http01Challenge { domain: "example.com".into(), token: "abc".into(), key_authorization: "abc.thumb".into() };
        assert_eq!(challenge.well_known_path(), "/.well-known/acme-challenge/abc");
        assert_eq!(challenge.relative_file_path(), ".well-known/acme-challenge/abc");
    }

    #[tokio::test]
    async fn memory_hook_round_trips() {
        let hook = MemoryHttp01Hook::new();
        let challenge = Http01Challenge { domain: "example.com".into(), token: "tok".into(), key_authorization: "tok.auth".into() };
        hook.place(&challenge).await.unwrap();
        assert_eq!(hook.response_for("tok").as_deref(), Some("tok.auth"));
        hook.clear(&challenge).await.unwrap();
        assert!(hook.is_empty());
    }

    #[tokio::test]
    async fn unsupported_dns01_returns_stable_code() {
        let err = UnsupportedDns01.place_txt("_acme-challenge.example.com", "value").await.unwrap_err();
        assert_eq!(err.code(), "not_implemented");
    }
}
