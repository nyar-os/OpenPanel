//! Non-Windows placeholder for the native service trait.

use super::{NativeServiceBackend, NativeServiceInfo, unsupported};
use crate::Result;

/// Stub backend used until Linux systemd (or another host) is wired.
pub struct StubBackend;

impl NativeServiceBackend for StubBackend {
    fn list(&self) -> Result<Vec<NativeServiceInfo>> {
        Err(unsupported("native OS service control is Windows SCM only in this release (Linux systemd stub later)"))
    }

    fn status(&self, _name: &str) -> Result<NativeServiceInfo> {
        Err(unsupported("native OS service control is Windows SCM only in this release (Linux systemd stub later)"))
    }

    fn start(&self, _name: &str) -> Result<NativeServiceInfo> {
        Err(unsupported("native OS service control is Windows SCM only in this release (Linux systemd stub later)"))
    }

    fn stop(&self, _name: &str) -> Result<NativeServiceInfo> {
        Err(unsupported("native OS service control is Windows SCM only in this release (Linux systemd stub later)"))
    }
}
