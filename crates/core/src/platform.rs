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
/// `gui` is the desktop client host. `serve` is the in-process WebUI listener
/// owned by the TypeScript meta package (same installable). Service / site /
/// process panel domains stay `not_implemented` in this skeleton.
pub fn capabilities() -> Capabilities {
    let mut items = vec![
        Capability::available(capability_ids::GUI_OPEN),
        Capability::available(capability_ids::GUI_PUMP),
        Capability::available(capability_ids::GUI_CLOSE),
        Capability::available(capability_ids::SERVE),
        Capability::unavailable(
            capability_ids::SERVICES,
            CapabilityState::NotImplemented,
            "local service inventory is not implemented in this skeleton",
        ),
        Capability::unavailable(
            capability_ids::SITES,
            CapabilityState::NotImplemented,
            "local site inventory is not implemented in this skeleton",
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
            "one installable `@wae/open-panel`: `gui` opens the desktop client and `serve` hosts the WebUI".to_string(),
            "OpenPanel is a local ops panel (services / sites / processes / logs), not an API gateway".to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_lists_gui_and_serve() {
        let report = capabilities();
        let ids: Vec<_> = report.items.iter().map(|item| item.id.as_str()).collect();
        assert!(ids.contains(&capability_ids::GUI_OPEN));
        assert!(ids.contains(&capability_ids::SERVE));
    }
}
