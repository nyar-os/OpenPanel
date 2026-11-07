//! Web site registry for the local ops panel.
//!
//! Sites are panel-managed HTTP endpoints (static hosting first). This is **not**
//! Node / Python / Java application runtime hosting.

mod host;
mod store;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use serde::{Deserialize, Serialize};

use crate::{
    Error, Result,
    audit::capability_ids,
    config::{config_file, ensure_config_dir},
    error::CapabilityState,
};

pub use host::SiteRuntimeState;
use host::{RunningSite, spawn_static_site};
use store::{SiteRecord, SiteStore};

const SITES_FILE: &str = "sites.json";

/// How a site is served. Only OpenPanel-hosted static is implemented now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WebSiteKind {
    /// OpenPanel listens and serves files from `rootDir`.
    #[default]
    StaticHost,
}

impl WebSiteKind {
    /// Stable wire string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StaticHost => "static-host",
        }
    }
}

/// Input for registering or updating a site.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSiteInput {
    /// Stable site id (path-safe).
    pub id: String,
    /// Primary hostname shown in the panel (Host header matching is best-effort).
    pub domain: String,
    /// Document root on disk.
    pub root_dir: String,
    /// Bind host. Defaults to `127.0.0.1`.
    #[serde(default = "default_host")]
    pub host: String,
    /// Bind port.
    pub port: u16,
    /// Whether the site may be started.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Serving kind. Only `static-host` is supported.
    #[serde(default)]
    pub kind: WebSiteKind,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_enabled() -> bool {
    true
}

/// Public site view including runtime state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSiteView {
    /// Stable site id.
    pub id: String,
    /// Primary domain label.
    pub domain: String,
    /// Document root.
    pub root_dir: String,
    /// Bind host.
    pub host: String,
    /// Bind port.
    pub port: u16,
    /// Whether start is allowed.
    pub enabled: bool,
    /// Serving kind.
    pub kind: WebSiteKind,
    /// Listener state.
    pub runtime: SiteRuntimeState,
}

/// In-process web site manager (registry + static listeners).
pub struct WebSiteManager {
    store: Mutex<SiteStore>,
    running: Mutex<HashMap<String, RunningSite>>,
}

impl WebSiteManager {
    /// Open the manager using the default config path (`sites.json`).
    pub fn open_default() -> Result<Self> {
        ensure_config_dir()?;
        Self::open(config_file(SITES_FILE))
    }

    /// Open the manager against an explicit store path (tests).
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let store = SiteStore::open(path)?;
        Ok(Self { store: Mutex::new(store), running: Mutex::new(HashMap::new()) })
    }

    /// List registered sites with runtime state.
    pub fn list(&self) -> Result<Vec<WebSiteView>> {
        let store = self.store.lock().map_err(|_| Error::message("web site store lock poisoned"))?;
        let running = self.running.lock().map_err(|_| Error::message("web site runtime lock poisoned"))?;
        Ok(store.records().iter().map(|record| view_of(record, running.contains_key(&record.id))).collect())
    }

    /// Register or replace a site definition.
    pub fn upsert(&self, input: WebSiteInput) -> Result<WebSiteView> {
        let record = validate_input(input)?;
        let mut store = self.store.lock().map_err(|_| Error::message("web site store lock poisoned"))?;
        store.upsert(record.clone())?;
        let running = self.running.lock().map_err(|_| Error::message("web site runtime lock poisoned"))?;
        Ok(view_of(&record, running.contains_key(&record.id)))
    }

    /// Remove a site and stop its listener when running.
    pub fn remove(&self, id: &str) -> Result<()> {
        let id = validate_id(id)?;
        self.stop_inner(id)?;
        let mut store = self.store.lock().map_err(|_| Error::message("web site store lock poisoned"))?;
        if !store.remove(id)? {
            return Err(Error::message(format!("site `{id}` not found")));
        }
        Ok(())
    }

    /// Enable or disable a site. Disabling also stops the listener.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<WebSiteView> {
        let id = validate_id(id)?;
        if !enabled {
            self.stop_inner(id)?;
        }
        let mut store = self.store.lock().map_err(|_| Error::message("web site store lock poisoned"))?;
        let record = store
            .get_mut(id)
            .ok_or_else(|| Error::message(format!("site `{id}` not found")))?;
        record.enabled = enabled;
        let snapshot = record.clone();
        store.persist()?;
        let running = self.running.lock().map_err(|_| Error::message("web site runtime lock poisoned"))?;
        Ok(view_of(&snapshot, running.contains_key(&snapshot.id)))
    }

    /// Start the OpenPanel-hosted static listener for an enabled site.
    pub fn start(&self, id: &str) -> Result<WebSiteView> {
        let id = validate_id(id)?;
        let record = {
            let store = self.store.lock().map_err(|_| Error::message("web site store lock poisoned"))?;
            store.get(id).cloned().ok_or_else(|| Error::message(format!("site `{id}` not found")))?
        };
        if !record.enabled {
            return Err(Error::message(format!("site `{id}` is disabled")));
        }
        if record.kind != WebSiteKind::StaticHost {
            return Err(Error::capability(
                capability_ids::SITES,
                CapabilityState::NotImplemented,
                format!("site kind `{}` is not implemented", record.kind.as_str()),
            ));
        }
        {
            let mut running = self.running.lock().map_err(|_| Error::message("web site runtime lock poisoned"))?;
            if running.contains_key(id) {
                return Ok(view_of(&record, true));
            }
            let handle = spawn_static_site(&record)?;
            running.insert(id.to_string(), handle);
        }
        Ok(view_of(&record, true))
    }

    /// Stop the listener for a site (registry row stays).
    pub fn stop(&self, id: &str) -> Result<WebSiteView> {
        let id = validate_id(id)?;
        self.stop_inner(id)?;
        let store = self.store.lock().map_err(|_| Error::message("web site store lock poisoned"))?;
        let record = store.get(id).cloned().ok_or_else(|| Error::message(format!("site `{id}` not found")))?;
        Ok(view_of(&record, false))
    }

    fn stop_inner(&self, id: &str) -> Result<()> {
        let mut running = self.running.lock().map_err(|_| Error::message("web site runtime lock poisoned"))?;
        if let Some(handle) = running.remove(id) {
            handle.stop();
        }
        Ok(())
    }
}

/// Process-wide default manager for N-API / server callers.
pub fn shared_manager() -> Result<Arc<WebSiteManager>> {
    static CELL: std::sync::OnceLock<Result<Arc<WebSiteManager>>> = std::sync::OnceLock::new();
    match CELL.get_or_init(|| WebSiteManager::open_default().map(Arc::new)) {
        Ok(manager) => Ok(Arc::clone(manager)),
        Err(error) => Err(Error::message(error.to_string())),
    }
}

fn view_of(record: &SiteRecord, running: bool) -> WebSiteView {
    WebSiteView {
        id: record.id.clone(),
        domain: record.domain.clone(),
        root_dir: record.root_dir.clone(),
        host: record.host.clone(),
        port: record.port,
        enabled: record.enabled,
        kind: record.kind,
        runtime: if running { SiteRuntimeState::Running } else { SiteRuntimeState::Stopped },
    }
}

fn validate_input(input: WebSiteInput) -> Result<SiteRecord> {
    let id = validate_id(&input.id)?.to_string();
    let domain = input.domain.trim();
    if domain.is_empty() {
        return Err(Error::message("site `domain` must not be empty"));
    }
    let root = PathBuf::from(input.root_dir.trim());
    if input.root_dir.trim().is_empty() {
        return Err(Error::message("site `rootDir` must not be empty"));
    }
    if !root.is_absolute() {
        return Err(Error::message("site `rootDir` must be an absolute path"));
    }
    if input.port == 0 {
        return Err(Error::message("site `port` must be between 1 and 65535"));
    }
    let host = {
        let trimmed = input.host.trim();
        if trimmed.is_empty() { default_host() } else { trimmed.to_string() }
    };
    Ok(SiteRecord {
        id,
        domain: domain.to_string(),
        root_dir: normalize_path_display(&root),
        host,
        port: input.port,
        enabled: input.enabled,
        kind: input.kind,
    })
}

fn validate_id(id: &str) -> Result<&str> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(Error::message("site `id` must not be empty"));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err(Error::message("site `id` must be a single path-safe segment"));
    }
    if !trimmed.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.') {
        return Err(Error::message("site `id` may only use ASCII letters digits `.` `-` `_`"));
    }
    Ok(trimmed)
}

fn normalize_path_display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    #[test]
    fn registry_crud_and_static_host() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("www");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), b"hello-site").unwrap();
        let store_path = dir.path().join("sites.json");
        let manager = WebSiteManager::open(&store_path).unwrap();

        let view = manager
            .upsert(WebSiteInput {
                id: "demo".into(),
                domain: "demo.local".into(),
                root_dir: root.to_string_lossy().into_owned(),
                host: "127.0.0.1".into(),
                port: 0, // will fail
                enabled: true,
                kind: WebSiteKind::StaticHost,
            })
            .unwrap_err();
        assert!(view.to_string().contains("port"));

        // Bind an ephemeral port by asking the OS first.
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let view = manager
            .upsert(WebSiteInput {
                id: "demo".into(),
                domain: "demo.local".into(),
                root_dir: root.to_string_lossy().into_owned(),
                host: "127.0.0.1".into(),
                port,
                enabled: true,
                kind: WebSiteKind::StaticHost,
            })
            .unwrap();
        assert_eq!(view.id, "demo");
        assert_eq!(view.runtime, SiteRuntimeState::Stopped);

        let started = manager.start("demo").unwrap();
        assert_eq!(started.runtime, SiteRuntimeState::Running);

        let body = fetch(&format!("127.0.0.1:{port}"), "/");
        assert!(body.contains("hello-site"), "body={body}");

        manager.set_enabled("demo", false).unwrap();
        assert_eq!(manager.list().unwrap()[0].runtime, SiteRuntimeState::Stopped);
        manager.remove("demo").unwrap();
        assert!(manager.list().unwrap().is_empty());
    }

    fn fetch(addr: &str, path: &str) -> String {
        for _ in 0..40 {
            if let Ok(mut stream) = TcpStream::connect(addr) {
                stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let request = format!("GET {path} HTTP/1.1\r\nHost: demo.local\r\nConnection: close\r\n\r\n");
                if stream.write_all(request.as_bytes()).is_err() {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                let mut buf = String::new();
                let _ = stream.read_to_string(&mut buf);
                return buf;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("failed to connect to {addr}");
    }
}
