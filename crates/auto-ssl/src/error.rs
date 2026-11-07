//! 稳定错误类型。

use std::path::PathBuf;

/// `auto-ssl` 操作失败。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 调用方输入不合法。
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// 能力尚未实现（例如 DNS-01）。
    #[error("not implemented: {0}")]
    NotImplemented(String),
    /// 证书材料缺失。
    #[error("certificate material missing under {0}")]
    MaterialMissing(PathBuf),
    /// 文件系统失败。
    #[error("io error at {path}: {source}")]
    Io {
        /// 相关路径。
        path: PathBuf,
        /// 底层 IO 错误。
        #[source]
        source: std::io::Error,
    },
    /// JSON 编解码失败。
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// PEM / X.509 解析失败。
    #[error("certificate parse error: {0}")]
    CertParse(String),
    /// ACME 协议或 CA 交互失败。
    #[error("acme error: {0}")]
    Acme(String),
    /// HTTP-01 / DNS-01 投放钩子失败。
    #[error("challenge hook error: {0}")]
    ChallengeHook(String),
}

impl Error {
    /// 构造输入错误。
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::InvalidInput(message.into())
    }

    /// 构造未实现错误。
    pub fn not_implemented(message: impl Into<String>) -> Self {
        Self::NotImplemented(message.into())
    }

    /// 构造 IO 错误。
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }

    /// 构造证书解析错误。
    pub fn cert_parse(message: impl Into<String>) -> Self {
        Self::CertParse(message.into())
    }

    /// 构造 ACME 错误。
    pub fn acme(message: impl Into<String>) -> Self {
        Self::Acme(message.into())
    }

    /// 构造挑战钩子错误。
    pub fn challenge_hook(message: impl Into<String>) -> Self {
        Self::ChallengeHook(message.into())
    }

    /// 稳定机器可读错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "invalid_input",
            Self::NotImplemented(_) => "not_implemented",
            Self::MaterialMissing(_) => "material_missing",
            Self::Io { .. } => "io",
            Self::Json(_) => "json",
            Self::CertParse(_) => "cert_parse",
            Self::Acme(_) => "acme",
            Self::ChallengeHook(_) => "challenge_hook",
        }
    }
}

/// 库结果别名。
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(Error::invalid_input("x").code(), "invalid_input");
        assert_eq!(Error::not_implemented("dns-01").code(), "not_implemented");
        assert_eq!(Error::acme("rate limited").code(), "acme");
        assert_eq!(Error::challenge_hook("place failed").code(), "challenge_hook");
    }
}
