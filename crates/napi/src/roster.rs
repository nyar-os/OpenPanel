//! Client-side managed server roster persistence shared with the TypeScript CLI.
//!
//! File shape matches `packages/open-panel/src/managed-servers.ts`. Secrets are
//! never stored here.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const CONFIG_DIR_ENV: &str = "OPEN_PANEL_CONFIG_DIR";
const APP_DIR_NAME: &str = "open-panel";
const ROSTER_FILE: &str = "servers.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedServerRef {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ClientServerRoster {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_id: Option<String>,
    #[serde(default)]
    servers: Vec<ManagedServerRef>,
}

fn config_dir() -> PathBuf {
    if let Ok(override_dir) = env::var(CONFIG_DIR_ENV) {
        let trimmed = override_dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if cfg!(windows) {
        if let Ok(app_data) = env::var("APPDATA") {
            let trimmed = app_data.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join(APP_DIR_NAME);
            }
        }
    }
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join(APP_DIR_NAME);
        }
    }
    dirs_fallback().join(".config").join(APP_DIR_NAME)
}

fn dirs_fallback() -> PathBuf {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn roster_path() -> PathBuf {
    config_dir().join(ROSTER_FILE)
}

fn resolve_url(server: &ManagedServerRef) -> Option<String> {
    if let Some(url) = server.url.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        return Some(if url.ends_with('/') { url.to_string() } else { format!("{url}/") });
    }
    let host = server.host.as_deref().map(str::trim).filter(|value| !value.is_empty())?;
    let port = server.port.unwrap_or(17890);
    Some(format!("http://{host}:{port}/"))
}

fn load_roster(path: &Path) -> Result<ClientServerRoster, String> {
    if !path.is_file() {
        return Ok(ClientServerRoster::default());
    }
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    if text.trim().is_empty() {
        return Ok(ClientServerRoster::default());
    }
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

fn save_roster(path: &Path, roster: &ClientServerRoster) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let body = serde_json::to_string_pretty(roster).map_err(|error| error.to_string())?;
    let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temp, format!("{body}\n")).map_err(|error| error.to_string())?;
    fs::rename(&temp, path).map_err(|error| error.to_string())?;
    Ok(())
}

fn snapshot(roster: &ClientServerRoster) -> Value {
    let path = roster_path();
    let active = roster
        .active_id
        .as_ref()
        .and_then(|id| roster.servers.iter().find(|server| &server.id == id));
    let servers: Vec<Value> = roster
        .servers
        .iter()
        .map(|server| {
            let mut value = serde_json::to_value(server).unwrap_or_else(|_| json!({}));
            if let Some(object) = value.as_object_mut() {
                if let Some(url) = resolve_url(server) {
                    object.insert("resolvedUrl".to_string(), json!(url));
                }
            }
            value
        })
        .collect();
    json!({
        "path": path.display().to_string(),
        "activeId": roster.active_id,
        "active": active,
        "servers": servers,
    })
}

/// Return the current roster snapshot for the GUI.
pub fn list_snapshot() -> Result<Value, String> {
    let path = roster_path();
    let roster = load_roster(&path)?;
    Ok(snapshot(&roster))
}

/// Select the active managed server and return the updated snapshot.
pub fn use_server(id: &str) -> Result<Value, String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("server `id` must be a non-empty string".to_string());
    }
    let path = roster_path();
    let mut roster = load_roster(&path)?;
    if !roster.servers.iter().any(|server| server.id == trimmed) {
        return Err(format!("server id `{trimmed}` not found"));
    }
    roster.active_id = Some(trimmed.to_string());
    save_roster(&path, &roster)?;
    Ok(snapshot(&roster))
}
