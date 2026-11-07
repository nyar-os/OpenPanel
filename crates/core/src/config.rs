//! Per-user OpenPanel config directory (no secrets in the repo tree).

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::{Error, Result};

const CONFIG_DIR_ENV: &str = "OPEN_PANEL_CONFIG_DIR";
const APP_DIR_NAME: &str = "open-panel";

/// Resolve the per-user OpenPanel config directory.
///
/// Override with `OPEN_PANEL_CONFIG_DIR`. Default: Windows `%APPDATA%/open-panel`,
/// otherwise `$XDG_CONFIG_HOME/open-panel` or `~/.config/open-panel`.
pub fn config_dir() -> PathBuf {
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
    home_dir().join(".config").join(APP_DIR_NAME)
}

fn home_dir() -> PathBuf {
    env::var_os("USERPROFILE")
        .or_else(|| env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Ensure the config directory exists and return it.
pub fn ensure_config_dir() -> Result<PathBuf> {
    let dir = config_dir();
    fs::create_dir_all(&dir).map_err(|error| Error::message(format!("cannot create config dir {}: {error}", dir.display())))?;
    Ok(dir)
}

/// Join a file under the config directory.
pub fn config_file(name: impl AsRef<Path>) -> PathBuf {
    config_dir().join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_env_wins() {
        let previous = env::var_os(CONFIG_DIR_ENV);
        // SAFETY: test-only, single-threaded env mutation for this crate's unit tests.
        unsafe { env::set_var(CONFIG_DIR_ENV, r"E:\tmp\open-panel-test-config") };
        let resolved = config_dir();
        match previous {
            Some(value) => unsafe { env::set_var(CONFIG_DIR_ENV, value) },
            None => unsafe { env::remove_var(CONFIG_DIR_ENV) },
        }
        assert_eq!(resolved, PathBuf::from(r"E:\tmp\open-panel-test-config"));
    }
}
