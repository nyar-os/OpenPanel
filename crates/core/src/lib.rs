//! OpenPanel domain core.
//!
//! This crate owns local ops-panel semantics: capability states for services,
//! sites, processes, and logs, plus the product split between the desktop
//! client (`open-panel client` / `@wae/open-panel-gui`) and the WebUI listener
//! (`open-panel server` / `@wae/open-panel-webui`). Both entries ship in one
//! installable. Platform-specific sinks stay in this crate; the N-API boundary
//! remains platform-neutral.
//!
//! Capability failures use exactly three stable codes:
//! `unsupported_platform`, `not_implemented`, and `dependency_blocked`.

mod audit;
mod error;
pub mod platform;

pub use audit::capability_ids;
pub use error::{CapabilityState, Error, ErrorPayload, Result, capability_code};
pub use platform::{Capabilities, Capability, Platform, capabilities, platform_arch, platform_name};

/// Optional ACME SSL helpers (`auto-ssl` crate) for `open-panel server` site certificates.
#[cfg(feature = "auto-ssl")]
pub use auto_ssl as ssl;
