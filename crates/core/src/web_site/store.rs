//! JSON persistence for web site registrations.

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::WebSiteKind;
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteRecord {
    pub id: String,
    pub domain: String,
    pub root_dir: String,
    pub host: String,
    pub port: u16,
    pub enabled: bool,
    #[serde(default)]
    pub kind: WebSiteKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SiteFile {
    #[serde(default)]
    sites: Vec<SiteRecord>,
}

pub struct SiteStore {
    path: PathBuf,
    file: SiteFile,
}

impl SiteStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let file = if path.is_file() {
            let text = fs::read_to_string(&path)
                .map_err(|error| Error::message(format!("cannot read {}: {error}", path.display())))?;
            serde_json::from_str(&text)
                .map_err(|error| Error::message(format!("invalid site store {}: {error}", path.display())))?
        }
        else {
            SiteFile::default()
        };
        Ok(Self { path, file })
    }

    pub fn records(&self) -> &[SiteRecord] {
        &self.file.sites
    }

    pub fn get(&self, id: &str) -> Option<&SiteRecord> {
        self.file.sites.iter().find(|site| site.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut SiteRecord> {
        self.file.sites.iter_mut().find(|site| site.id == id)
    }

    pub fn upsert(&mut self, record: SiteRecord) -> Result<()> {
        if let Some(existing) = self.get_mut(&record.id) {
            *existing = record;
        }
        else {
            self.file.sites.push(record);
        }
        self.persist()
    }

    pub fn remove(&mut self, id: &str) -> Result<bool> {
        let before = self.file.sites.len();
        self.file.sites.retain(|site| site.id != id);
        let removed = self.file.sites.len() != before;
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    pub fn persist(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| Error::message(format!("cannot create {}: {error}", parent.display())))?;
        }
        let text = serde_json::to_string_pretty(&self.file)
            .map_err(|error| Error::message(format!("cannot encode site store: {error}")))?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, format!("{text}\n"))
            .map_err(|error| Error::message(format!("cannot write {}: {error}", tmp.display())))?;
        fs::rename(&tmp, &self.path)
            .map_err(|error| Error::message(format!("cannot publish {}: {error}", self.path.display())))?;
        Ok(())
    }
}
