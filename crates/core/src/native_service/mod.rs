//! Native OS service control (Windows SCM first).
//!
//! This is **not** application-runtime hosting (Node / Python / Java). Those
//! stay out of scope until a later phase.

use serde::Serialize;

use crate::{Error, Result, audit::capability_ids, error::CapabilityState};

#[cfg(windows)]
mod windows;
#[cfg(not(windows))]
mod stub;

/// Stable service state for panel JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeServiceState {
    /// Service is not running.
    Stopped,
    /// Start has been requested.
    StartPending,
    /// Stop has been requested.
    StopPending,
    /// Service is running.
    Running,
    /// Continue has been requested after pause.
    ContinuePending,
    /// Pause has been requested.
    PausePending,
    /// Service is paused.
    Paused,
    /// Backend returned an unrecognized code.
    Unknown,
}

impl NativeServiceState {
    /// Stable wire string.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::StartPending => "start-pending",
            Self::StopPending => "stop-pending",
            Self::Running => "running",
            Self::ContinuePending => "continue-pending",
            Self::PausePending => "pause-pending",
            Self::Paused => "paused",
            Self::Unknown => "unknown",
        }
    }
}

/// One native OS service entry.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeServiceInfo {
    /// Service key name (SCM `lpServiceName`).
    pub name: String,
    /// Localized display name when available.
    pub display_name: String,
    /// Current state.
    pub state: NativeServiceState,
}

/// Cross-platform native service backend.
pub trait NativeServiceBackend: Send + Sync {
    /// List installed services the caller may enumerate.
    fn list(&self) -> Result<Vec<NativeServiceInfo>>;

    /// Query one service by name.
    fn status(&self, name: &str) -> Result<NativeServiceInfo>;

    /// Start a stopped service.
    fn start(&self, name: &str) -> Result<NativeServiceInfo>;

    /// Stop a running service.
    fn stop(&self, name: &str) -> Result<NativeServiceInfo>;

    /// Stop then start. Optional convenience over start/stop.
    fn restart(&self, name: &str) -> Result<NativeServiceInfo> {
        let current = self.status(name)?;
        if current.state != NativeServiceState::Stopped && current.state != NativeServiceState::StopPending {
            let _ = self.stop(name)?;
            wait_until(self, name, NativeServiceState::Stopped, 30)?;
        }
        self.start(name)?;
        wait_until(self, name, NativeServiceState::Running, 30)
    }
}

fn wait_until<B: NativeServiceBackend + ?Sized>(
    backend: &B,
    name: &str,
    want: NativeServiceState,
    attempts: u32,
) -> Result<NativeServiceInfo> {
    for _ in 0..attempts {
        let info = backend.status(name)?;
        if info.state == want {
            return Ok(info);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let info = backend.status(name)?;
    Err(Error::message(format!(
        "service `{name}` did not reach `{}` (last={})",
        want.as_str(),
        info.state.as_str()
    )))
}

/// Construct the host native-service backend.
pub fn backend() -> Box<dyn NativeServiceBackend> {
    #[cfg(windows)]
    {
        Box::new(windows::ScmBackend)
    }
    #[cfg(not(windows))]
    {
        Box::new(stub::StubBackend)
    }
}

fn refuse_empty_name(name: &str) -> Result<&str> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::message("service name must not be empty"));
    }
    Ok(trimmed)
}

#[cfg(not(windows))]
fn unsupported(detail: impl Into<String>) -> Error {
    Error::capability(capability_ids::SERVICES, CapabilityState::UnsupportedPlatform, detail)
}

#[cfg(windows)]
fn dependency(detail: impl Into<String>) -> Error {
    Error::capability(capability_ids::SERVICES, CapabilityState::DependencyBlocked, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_codes_are_stable() {
        assert_eq!(NativeServiceState::Running.as_str(), "running");
        assert_eq!(NativeServiceState::Stopped.as_str(), "stopped");
    }

    #[cfg(windows)]
    #[test]
    fn windows_can_list_services() {
        let items = backend().list().expect("list services");
        assert!(!items.is_empty(), "expected at least one Windows service");
        let sample = items.iter().find(|item| item.name.eq_ignore_ascii_case("EventLog")).or_else(|| items.first()).expect("sample");
        let status = backend().status(&sample.name).expect("status");
        assert_eq!(status.name, sample.name);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_lists_as_unsupported() {
        let error = backend().list().unwrap_err();
        assert_eq!(error.code(), "unsupported_platform");
    }
}
