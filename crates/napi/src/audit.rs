//! Audit trail at the N-API capability boundary.

use std::sync::{Arc, PoisonError, RwLock};

use open_panel_core::{Error as CoreError, Result as CoreResult, capability_code};
use serde_json::Value;
use wae::audit::{Actor, Auditor, Entry, Mode, Outcome, Query};

/// Application name written into every record.
const APP: &str = "open-panel";

static AUDITOR: RwLock<Option<Arc<Auditor>>> = RwLock::new(None);

/// Target count and human-readable summary attached to a record.
#[derive(Debug, Clone)]
pub struct Summary {
    pub count: u64,
    pub summary: String,
}

impl Summary {
    pub fn new(count: u64, summary: impl Into<String>) -> Self {
        Self { count, summary: summary.into() }
    }
}

/// Process-wide auditor; built from environment defaults on first use.
fn auditor() -> Arc<Auditor> {
    if let Some(existing) = AUDITOR.read().unwrap_or_else(PoisonError::into_inner).as_ref() {
        return Arc::clone(existing);
    }
    let built = Arc::new(Auditor::from_env(APP, env!("CARGO_PKG_VERSION")));
    let mut guard = AUDITOR.write().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(guard.get_or_insert(built))
}

/// Replace the process-wide auditor after an explicit actor/session override.
pub fn configure(actor: Actor, session: Option<String>, host: Option<String>) {
    let mut builder = Auditor::builder(APP, env!("CARGO_PKG_VERSION")).actor(actor);
    if let Some(session) = session {
        builder = builder.session(session);
    }
    if let Some(host) = host {
        builder = builder.host(host);
    }
    let configured = Arc::new(builder.build());
    audit_configured(&configured);
    *AUDITOR.write().unwrap_or_else(PoisonError::into_inner) = Some(configured);
}

fn audit_configured(auditor: &Auditor) {
    let actor = auditor.actor();
    let params = serde_json::json!({
        "actorKind": actor.kind,
        "actorId": actor.id,
        "actorSource": actor.source,
        "session": auditor.session(),
        "host": auditor.host(),
    });
    let _ = run_with(auditor, "audit-configure", Mode::DryRun, params, || Ok(((), Summary::new(0, "audit configuration updated"))));
}

/// Record one capability call and return its result unchanged.
pub fn run<T>(capability: &str, mode: Mode, params: Value, body: impl FnOnce() -> CoreResult<(T, Summary)>) -> CoreResult<T> {
    let auditor = auditor();
    run_with(&auditor, capability, mode, params, body)
}

/// [`run`] against an explicit auditor, used by tests.
pub fn run_with<T>(
    auditor: &Auditor,
    capability: &str,
    mode: Mode,
    params: Value,
    body: impl FnOnce() -> CoreResult<(T, Summary)>,
) -> CoreResult<T> {
    let call = auditor.call(capability, mode).params(params);
    match body() {
        Ok((value, summary)) => {
            let _ = call.targets(summary.count, summary.summary).succeed();
            Ok(value)
        }
        Err(error) => {
            let code = error_code(&error);
            match error_outcome(&error) {
                Outcome::Rejected => {
                    let _ = call.reject(code);
                }
                _ => {
                    let _ = call.fail(code);
                }
            }
            Err(error)
        }
    }
}

/// Query the shared history.
pub fn query(filter: &Query) -> Result<Vec<Entry>, wae::audit::AuditError> {
    auditor().query(filter)
}

fn error_code(error: &CoreError) -> String {
    if let Some(code) = capability_code(error) {
        return code.to_string();
    }
    match error {
        CoreError::Message(_) => "message".to_string(),
        CoreError::Capability { .. } => "capability".to_string(),
    }
}

fn error_outcome(error: &CoreError) -> Outcome {
    match error {
        CoreError::Capability { .. } => Outcome::Rejected,
        _ => Outcome::Failure,
    }
}
