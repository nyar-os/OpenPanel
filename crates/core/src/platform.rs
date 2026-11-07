//! Platform identity and the domain capability report.

use serde::Serialize;

use crate::{audit::capability_ids, error::CapabilityState};

/// Runtime platform identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Windows.
    Windows,
    /// macOS.
    Macos,
    /// Linux.
    Linux,
    /// Any other target.
    Other,
}

impl Platform {
    /// Detect the current target. Diagnostics only.
    pub const fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        }
        else if cfg!(target_os = "macos") {
            Self::Macos
        }
        else if cfg!(target_os = "linux") {
            Self::Linux
        }
        else {
            Self::Other
        }
    }

    /// Stable display name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Other => "other",
        }
    }
}

/// Current platform display name.
pub fn platform_name() -> &'static str {
    Platform::current().name()
}

/// Current CPU architecture.
pub fn platform_arch() -> &'static str {
    std::env::consts::ARCH
}

/// One capability and its availability.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    /// Stable capability id.
    pub id: String,
    /// `null` means available.
    pub state: Option<CapabilityState>,
    /// Optional explanation.
    pub detail: Option<String>,
}

impl Capability {
    /// An available capability.
    pub fn available(id: &str) -> Self {
        Self { id: id.to_string(), state: None, detail: None }
    }

    /// An unavailable capability.
    pub fn unavailable(id: &str, state: CapabilityState, detail: impl Into<String>) -> Self {
        Self { id: id.to_string(), state: Some(state), detail: Some(detail.into()) }
    }
}

/// Domain capability report.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Runtime platform identity.
    pub platform: String,
    /// Runtime CPU architecture.
    pub arch: String,
    /// Domain capabilities only.
    pub items: Vec<Capability>,
    /// Boundary notes that are not capabilities.
    pub notes: Vec<String>,
}

/// Build the domain capability report for the current host.
///
/// Desktop client host capabilities stay under `gui-*`. `server` is the
/// in-process WebUI listener owned by the TypeScript meta package (same
/// installable). Native OS services are Windows SCM first. Web sites use
/// OpenPanel-hosted static listeners (not Node/Python app runtimes).
pub fn capabilities() -> Capabilities {
    let mut items = vec![
        Capability::available(capability_ids::GUI_OPEN),
        Capability::available(capability_ids::GUI_PUMP),
        Capability::available(capability_ids::GUI_CLOSE),
        Capability::available(capability_ids::SERVER),
        services_capability(),
        Capability::available(capability_ids::SITES),
        Capability::unavailable(
            capability_ids::AUTO_SSL,
            CapabilityState::NotImplemented,
            "ACME SSL issue and renew is available as the `auto-ssl` crate but not wired into server yet",
        ),
        Capability::unavailable(
            capability_ids::PROCESSES,
            CapabilityState::NotImplemented,
            "local process inventory is not implemented in this skeleton",
        ),
        Capability::unavailable(capability_ids::LOGS, CapabilityState::NotImplemented, "local log tailing is not implemented in this skeleton"),
    ];

    if Platform::current() != Platform::Windows {
        for item in &mut items {
            if item.id.starts_with("gui-") {
                item.state = Some(CapabilityState::UnsupportedPlatform);
                item.detail = Some("desktop GUI host publishes win32-x64 only in this skeleton".to_string());
            }
        }
    }

    Capabilities {
        platform: platform_name().to_string(),
        arch: platform_arch().to_string(),
        items,
        notes: vec![
            "one installable `@wae/open-panel`: `client` opens `@wae/open-panel-gui` and `server` hosts `@wae/open-panel-webui`".to_string(),
            "OpenPanel is a local ops panel (services / sites / processes / logs), not an API gateway".to_string(),
            "web sites use OpenPanel-hosted static listeners (`kind=static-host`), not IIS/Nginx config generation yet".to_string(),
            "Node/Python/Java application runtime hosting is deferred".to_string(),
        ],
    }
}

fn services_capability() -> Capability {
    if Platform::current() == Platform::Windows {
        Capability::available(capability_ids::SERVICES)
    }
    else {
        Capability::unavailable(
            capability_ids::SERVICES,
            CapabilityState::UnsupportedPlatform,
            "native OS service control is Windows SCM only in this release (Linux systemd stub later)",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_lists_gui_and_server() {
        let report = capabilities();
        let ids: Vec<_> = report.items.iter().map(|item| item.id.as_str()).collect();
        assert!(ids.contains(&capability_ids::GUI_OPEN));
        assert!(ids.contains(&capability_ids::SERVER));
        assert!(ids.contains(&capability_ids::SITES));
        assert!(ids.contains(&capability_ids::SERVICES));
        let sites = report.items.iter().find(|item| item.id == capability_ids::SITES).unwrap();
        assert!(sites.state.is_none());
        let services = report.items.iter().find(|item| item.id == capability_ids::SERVICES).unwrap();
        if Platform::current() == Platform::Windows {
            assert!(services.state.is_none());
        }
        else {
            assert_eq!(services.state, Some(CapabilityState::UnsupportedPlatform));
        }
    }
}
