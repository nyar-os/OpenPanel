//! Stable capability ids shared by the N-API boundary and the CLI.

/// Capability id constants.
pub mod capability_ids {
    /// Native addon version probe.
    pub const VERSION: &str = "version";
    /// Domain capability report.
    pub const CAPABILITIES: &str = "capabilities";
    /// Open the desktop GUI client.
    pub const GUI_OPEN: &str = "gui-open";
    /// Pump desktop GUI events.
    pub const GUI_PUMP: &str = "gui-pump";
    /// Close the desktop GUI client.
    pub const GUI_CLOSE: &str = "gui-close";
    /// Desktop GUI liveness probe.
    pub const GUI_STATUS: &str = "gui-status";
    /// Host the local WebUI listener (`open-panel server`).
    pub const SERVER: &str = "server";
    /// Local service inventory.
    pub const SERVICES: &str = "services";
    /// Local site inventory.
    pub const SITES: &str = "sites";
    /// Free ACME SSL issue and renew for local sites.
    pub const AUTO_SSL: &str = "auto-ssl";
    /// Local process inventory.
    pub const PROCESSES: &str = "processes";
    /// Local log access.
    pub const LOGS: &str = "logs";
    /// Audit query.
    pub const AUDIT_QUERY: &str = "audit-query";
}
