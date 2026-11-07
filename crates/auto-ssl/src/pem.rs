//! PEM 证书拆分与 `notAfter` 解析。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use x509_parser::prelude::*;

use crate::error::{Error, Result};

/// 将 fullchain PEM 拆成叶子与中间链（中间链可为空）。
pub fn split_leaf_and_chain(fullchain_pem: &str) -> Result<(String, String)> {
    let blocks = collect_pem_blocks(fullchain_pem, "CERTIFICATE")?;
    if blocks.is_empty() {
        return Err(Error::cert_parse("fullchain contains no CERTIFICATE blocks"));
    }
    let leaf = blocks[0].clone();
    let chain = blocks[1..].join("");
    Ok((leaf, chain))
}

/// 解析叶子证书 `notAfter`。
pub fn leaf_not_after(cert_pem: &str) -> Result<SystemTime> {
    let (_, pem) = parse_x509_pem(cert_pem.as_bytes()).map_err(|error| Error::cert_parse(format!("pem: {error}")))?;
    let (_, cert) = parse_x509_certificate(&pem.contents).map_err(|error| Error::cert_parse(format!("x509: {error}")))?;
    let timestamp = cert.validity().not_after.timestamp();
    if timestamp < 0 {
        return Err(Error::cert_parse("not_after before unix epoch"));
    }
    Ok(UNIX_EPOCH + Duration::from_secs(timestamp as u64))
}

/// 尽量取叶子证书 CN；没有则报错。
pub fn leaf_common_name(cert_pem: &str) -> Result<String> {
    let (_, pem) = parse_x509_pem(cert_pem.as_bytes()).map_err(|error| Error::cert_parse(format!("pem: {error}")))?;
    let (_, cert) = parse_x509_certificate(&pem.contents).map_err(|error| Error::cert_parse(format!("x509: {error}")))?;
    for attr in cert.subject().iter_common_name() {
        if let Ok(value) = attr.as_str()
            && !value.is_empty()
        {
            return Ok(value.to_string());
        }
    }
    Err(Error::cert_parse("leaf certificate has no common name"))
}

fn collect_pem_blocks(pem: &str, label: &str) -> Result<Vec<String>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let mut blocks = Vec::new();
    let mut rest = pem;
    while let Some(start) = rest.find(&begin) {
        let after_start = &rest[start..];
        let Some(end_idx) = after_start.find(&end)
        else {
            return Err(Error::cert_parse(format!("missing end marker for {label}")));
        };
        let end_at = end_idx + end.len();
        let mut block = after_start[..end_at].to_string();
        if !block.ends_with('\n') {
            block.push('\n');
        }
        blocks.push(block);
        rest = &after_start[end_at..];
    }
    Ok(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_keeps_leaf_first() {
        let fullchain = "-----BEGIN CERTIFICATE-----\nLEAF\n-----END CERTIFICATE-----\n\
             -----BEGIN CERTIFICATE-----\nMID\n-----END CERTIFICATE-----\n";
        let (leaf, chain) = split_leaf_and_chain(fullchain).unwrap();
        assert!(leaf.contains("LEAF"));
        assert!(chain.contains("MID"));
        assert!(!chain.contains("LEAF"));
    }
}
