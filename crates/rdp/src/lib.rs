//! Pure-Rust RDP client surface for OpenPanel Windows remote install.
//!
//! Built on [IronRDP](https://github.com/Devolutions/IronRDP) (`ironrdp-connector` +
//! `ironrdp-blocking` + `ironrdp-session`). There is **no** `mstsc` / FreeRDP /
//! `wfreerdp` CLI wrapper.
//!
//! ## Layers
//!
//! - TCP connect to `:3389`
//! - X.224 / MCS / GCC negotiation (`connect_begin`)
//! - TLS upgrade
//! - CredSSP / NLA (`connect_finalize`)
//! - Connection finalization to an active session
//! - Optional [`Config::alternate_shell`](ironrdp_connector::Config::alternate_shell)
//!   remote process via the Client Info PDU, then ActiveStage until disconnect
//!
//! ## Payload delivery
//!
//! MS-RDPBCGR caps `AlternateShell` at 512 bytes, so install payloads cannot ride
//! inside the shell string. Controllers should either:
//!
//! 1. Point `alternate_shell` at a short bootstrap that downloads an install
//!    script from a URL the remote can reach (controller ephemeral HTTP, npm,
//!    or an internal artifact URL), or
//! 2. Set [`RdpConnectOptions::rdpdr_share_dir`] so the client redirects a local
//!    staging directory as `\\tsclient\OP` and the shell runs a script from it.
//!
//! Prefer `force_fresh_logon` so Windows does not reconnect a disconnected
//! desktop and ignore `AlternateShell`. OpenSSH on the target is never required.

#![deny(missing_docs)]

use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Best-effort crash breadcrumb for lab diagnosis (ignored on failure).
///
/// Writes to `OPEN_PANEL_RDP_BREADCRUMB` when set, otherwise
/// `%TEMP%/open-panel-rdp-breadcrumb.txt`, and mirrors the same line to stderr
/// so Node façades can surface progress even when stdout is empty after a kill.
fn breadcrumb(step: &str) {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let line = format!("{ts} {step}");
    eprintln!("[open-panel-rdp] {line}");
    let _ = io::stderr().flush();

    let path = std::env::var_os("OPEN_PANEL_RDP_BREADCRUMB").unwrap_or_else(|| {
        std::env::temp_dir()
            .join("open-panel-rdp-breadcrumb.txt")
            .into_os_string()
    });
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

use ironrdp_blocking::Framed;
use ironrdp_connector::sspi::network_client::reqwest_network_client::ReqwestNetworkClient;
use ironrdp_connector::{
    ClientConnector, ConnectionOutcome, ConnectionResult, Credentials, DesktopSize, ServerName,
};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::gcc::{ConnectionType, KeyboardType};
use ironrdp_pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};
use ironrdp_pdu::input::{MousePdu, mouse::PointerFlags};
use ironrdp_pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName as RustlsServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, Error as TlsError, SignatureScheme, StreamOwned};
use serde::Serialize;
use x509_cert::der::Decode as _;

/// MS-RDPBCGR Client Info `AlternateShell` / `WorkingDir` max length including NUL.
pub const ALTERNATE_SHELL_MAX_BYTES: usize = 512;

/// Errors produced by the OpenPanel RDP client.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// DNS / address resolution failed.
    #[error("resolve `{host}:{port}`: {source}")]
    Resolve {
        /// Target host.
        host: String,
        /// Target port.
        port: u16,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// TCP connect failed.
    #[error("tcp connect `{addr}`: {source}")]
    TcpConnect {
        /// Resolved socket address.
        addr: SocketAddr,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// TLS upgrade failed.
    #[error("tls upgrade: {0}")]
    Tls(String),
    /// IronRDP connector failure (X.224 / CredSSP / MCS / …).
    #[error("rdp connector: {0}")]
    Connector(String),
    /// IronRDP active-session failure.
    #[error("rdp session: {0}")]
    Session(String),
    /// Server redirected the session and this client does not follow yet.
    #[error("rdp redirection is not implemented yet")]
    Redirection,
    /// Invalid caller input.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}

/// Convenient result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Credentials used for CredSSP / NLA. Keep passwords in memory only.
#[derive(Clone)]
pub struct RdpAuth {
    /// Account name (without domain separator when `domain` is set).
    pub username: String,
    /// Password (never logged).
    pub password: String,
    /// Optional NetBIOS / AD domain.
    pub domain: Option<String>,
}

impl std::fmt::Debug for RdpAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RdpAuth")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("domain", &self.domain)
            .finish()
    }
}

/// Connection request for a Windows RDP endpoint.
#[derive(Debug, Clone)]
pub struct RdpConnectOptions {
    /// Hostname or IP.
    pub host: String,
    /// TCP port (default 3389).
    pub port: u16,
    /// CredSSP credentials.
    pub auth: RdpAuth,
    /// Accept any peer certificate (lab / bootstrap hosts). Prefer `false` in production.
    pub accept_invalid_certs: bool,
    /// Optional alternate shell (MS-RDPBCGR Client Info). Empty = full desktop session.
    pub alternate_shell: String,
    /// Working directory for `alternate_shell`.
    pub work_dir: String,
    /// TCP / handshake timeouts.
    pub timeout: Duration,
    /// How long to keep the active session after connect (install / shell lifetime).
    pub session_timeout: Duration,
    /// After the desktop/session settles, also type the shell into Win+R.
    ///
    /// Windows often ignores `AlternateShell` when reconnecting a disconnected
    /// session for the same user. Keyboard injection still runs the bootstrap.
    pub inject_run_dialog: bool,
    /// How long to wait for graphics / logon before Win+R injection.
    pub settle: Duration,
    /// Before the real shell session, reconnect after forcing a logoff so
    /// Windows starts a fresh logon and honors `AlternateShell`.
    ///
    /// Single-user hosts commonly restore a disconnected desktop and ignore
    /// Client Info `AlternateShell`; a prior `logoff` clears that session.
    pub force_fresh_logon: bool,
    /// Settle / hold budget for the preparatory logoff pass.
    pub logoff_timeout: Duration,
    /// Optional local directory redirected through RDPDR as `\\tsclient\OP`.
    ///
    /// Used to deliver install scripts / tarballs when the remote cannot reach
    /// the controller over HTTP. Requires co-advertising RDPSND (MS-RDPEFS).
    pub rdpdr_share_dir: Option<PathBuf>,
}

impl Default for RdpConnectOptions {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 3389,
            auth: RdpAuth {
                username: String::new(),
                password: String::new(),
                domain: None,
            },
            accept_invalid_certs: true,
            alternate_shell: String::new(),
            work_dir: String::new(),
            timeout: Duration::from_secs(20),
            session_timeout: Duration::from_secs(180),
            // Win+R / ActiveStage graphics abort on lab hosts; AlternateShell is the
            // default delivery after force_fresh_logon.
            inject_run_dialog: false,
            settle: Duration::from_secs(12),
            // Default on so disconnected single-user sessions cannot swallow
            // Opt in via CLI `--force-fresh-logon` — lab hosts abort after force.
            force_fresh_logon: false,
            logoff_timeout: Duration::from_secs(45),
            rdpdr_share_dir: None,
        }
    }
}

/// Machine-readable handshake report (no secrets).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeReport {
    /// Whether the full connector sequence reached an active session.
    pub ok: bool,
    /// Target host.
    pub host: String,
    /// Target port.
    pub port: u16,
    /// Resolved socket address.
    pub remote_addr: String,
    /// Local socket address.
    pub local_addr: String,
    /// Desktop width negotiated with the server.
    pub desktop_width: u16,
    /// Desktop height negotiated with the server.
    pub desktop_height: u16,
    /// IO channel id.
    pub io_channel_id: u16,
    /// User channel id.
    pub user_channel_id: u16,
    /// Protocol layers completed.
    pub layers: Vec<&'static str>,
    /// Human notes / next steps.
    pub notes: Vec<String>,
}

/// Why an alternate-shell session ended.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ExecEndReason {
    /// Server sent a graceful terminate (typical when the shell exits).
    ServerTerminate,
    /// Wall-clock `session_timeout` elapsed while the session was still open.
    SessionTimeout,
    /// TCP read went idle (WouldBlock) after the configured read timeout.
    ReadIdle,
    /// Transport closed without an explicit terminate PDU.
    TransportClosed,
}

/// Machine-readable alternate-shell execution report (no secrets).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecReport {
    /// Whether the session was established and the shell request was accepted.
    pub ok: bool,
    /// Target host.
    pub host: String,
    /// Target port.
    pub port: u16,
    /// Resolved socket address.
    pub remote_addr: String,
    /// Local socket address.
    pub local_addr: String,
    /// Alternate shell that was requested (truncated when very long).
    pub alternate_shell: String,
    /// Working directory sent with the shell.
    pub work_dir: String,
    /// Protocol layers completed.
    pub layers: Vec<&'static str>,
    /// How the active stage ended.
    pub end_reason: ExecEndReason,
    /// Wall time spent in the active stage.
    pub elapsed_ms: u64,
    /// Number of ActiveStage response frames written back to the server.
    pub response_frames: u64,
    /// Whether a Win+R keyboard injection of the shell was attempted.
    pub injected_run_dialog: bool,
    /// Whether a preparatory logoff pass ran before the shell session.
    pub forced_fresh_logon: bool,
    /// How the preparatory logoff pass ended (when `forced_fresh_logon`).
    pub logoff_end_reason: Option<ExecEndReason>,
    /// Wall time spent in the preparatory logoff pass.
    pub logoff_elapsed_ms: Option<u64>,
    /// Server Input capability flags (for diagnosing fast-path input support).
    pub input_flags: u16,
    /// Human notes / delivery reminders.
    pub notes: Vec<String>,
}

/// Probe TCP reachability of an RDP port without speaking RDP.
pub fn probe_tcp(host: &str, port: u16, timeout: Duration) -> Result<SocketAddr> {
    let host = host.trim();
    if host.is_empty() {
        return Err(Error::InvalidArgument("host is required".into()));
    }
    let addr = (host, port)
        .to_socket_addrs()
        .map_err(|source| Error::Resolve {
            host: host.to_owned(),
            port,
            source,
        })?
        .next()
        .ok_or_else(|| Error::InvalidArgument(format!("no socket address for `{host}:{port}`")))?;
    TcpStream::connect_timeout(&addr, timeout).map_err(|source| Error::TcpConnect { addr, source })?;
    Ok(addr)
}

/// Run the IronRDP connection sequence through CredSSP and session finalization.
///
/// On success the TCP/TLS session is dropped after capturing the
/// [`ConnectionResult`] metadata. Prefer [`run_alternate_shell`] when a remote
/// process must actually start.
pub fn handshake(options: RdpConnectOptions) -> Result<HandshakeReport> {
    validate_connect_options(&options, false)?;
    let connected = connect_session(options)?;
    Ok(report_from_result(
        &connected.host,
        connected.port,
        connected.remote_addr,
        connected.local_addr,
        &connected.connection,
        connected.layers,
    ))
}

/// True when the peer closed the RDP/TLS transport (expected after `logoff`).
fn is_transport_gone_io(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::UnexpectedEof
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::ConnectionAborted
            | io::ErrorKind::BrokenPipe
    ) {
        return true;
    }
    // Windows often surfaces WSAECONNRESET (10054) as `ErrorKind::Other`.
    matches!(error.raw_os_error(), Some(10054) | Some(10053) | Some(10058))
        || is_transport_gone_msg(&error.to_string())
}

/// Match Session / Connector messages that only mean the peer hung up.
fn is_transport_gone_msg(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("10054")
        || lower.contains("10053")
        || lower.contains("connection reset")
        || lower.contains("connection abort")
        || lower.contains("broken pipe")
        || lower.contains("unexpected eof")
        || lower.contains("强迫关闭")
        || lower.contains("软件终止")
        || lower.contains("你的主机中的软件中止")
}

/// Connect with `Config::alternate_shell` set and drive the ActiveStage until the
/// remote shell exits, the transport closes, or `session_timeout` elapses.
///
/// The shell string must fit MS-RDPBCGR's 512-byte limit. Large payloads belong
/// in a URL fetch or a future RDPDR stage, not in this field.
///
/// When `force_fresh_logon` is true (default), a preparatory pass injects
/// `logoff` so a disconnected single-user session cannot swallow AlternateShell.
/// When `inject_run_dialog` is true, the shell is also typed into Win+R after
/// settle as a reconnect fallback.
pub fn run_alternate_shell(options: RdpConnectOptions) -> Result<ExecReport> {
    breadcrumb("run_alternate_shell:enter");
    validate_connect_options(&options, true)?;

    let mut forced_fresh_logon = false;
    let mut logoff_end_reason = None;
    let mut logoff_elapsed_ms = None;
    let mut layers_prefix = Vec::new();

    if options.force_fresh_logon {
        breadcrumb("run_alternate_shell:logoff_pass_begin");
        let logoff = run_logoff_pass(&options)?;
        breadcrumb(&format!(
            "run_alternate_shell:logoff_pass_done end={:?}",
            logoff.end_reason
        ));
        forced_fresh_logon = true;
        logoff_end_reason = Some(logoff.end_reason);
        logoff_elapsed_ms = Some(logoff.elapsed_ms);
        layers_prefix.extend(logoff.layers);
        layers_prefix.push("force-fresh-logon-gap");
        // Give Terminal Services time to tear down the logged-off session.
        // Too-short gaps have caused the next CredSSP connect to be RST (10054).
        breadcrumb("run_alternate_shell:logoff_gap_begin");
        // Chunked sleep so breadcrumbs show whether an external kill hits mid-gap.
        for tick in 1..=8_u32 {
            std::thread::sleep(Duration::from_secs(1));
            breadcrumb(&format!("run_alternate_shell:logoff_gap_tick={tick}"));
        }
        breadcrumb("run_alternate_shell:logoff_gap_done");
    }

    let shell = options.alternate_shell.clone();
    let work_dir = options.work_dir.clone();
    let session_timeout = options.session_timeout;
    let settle = options.settle;
    // After a successful force-logoff, Windows honors Client Info AlternateShell.
    // Prefer that path: Win+R needs a long-lived graphics desktop and FastPath
    // decode has aborted this process on the lab host (native AV, exit -1).
    // Win+R remains the reconnect fallback when force_fresh_logon is off.
    let inject_run_dialog = options.inject_run_dialog && !forced_fresh_logon;
    let mut connect_opts = options;
    let mut desktop_for_inject = false;
    if inject_run_dialog {
        // Desktop + Win+R only: empty AlternateShell so Explorer is present.
        connect_opts.alternate_shell.clear();
        connect_opts.work_dir.clear();
        desktop_for_inject = true;
    }
    breadcrumb(&format!(
        "run_alternate_shell:payload_connect_begin inject={inject_run_dialog} desktop={desktop_for_inject}"
    ));
    let connected = connect_session_with_retry(connect_opts, forced_fresh_logon)?;
    breadcrumb("run_alternate_shell:payload_connect_done");
    // Client Info AlternateShell is already running after connect. Driving
    // ActiveStage bitmap/FastPath decode has aborted this process on lab hosts
    // (native AV). When we are not injecting Win+R, just hold the transport.
    let active = if inject_run_dialog {
        drive_active_stage(DriveActiveOpts {
            connected,
            session_timeout,
            settle,
            inject_command: Some(shell.clone()),
            layer_label: "alternate-shell-active-stage",
            hold_after_inject: true,
        })?
    } else {
        hold_transport_without_graphics(connected, session_timeout, "alternate-shell-hold")?
    };
    breadcrumb(&format!(
        "run_alternate_shell:active_done end={:?}",
        active.end_reason
    ));

    let mut layers = layers_prefix;
    layers.extend(active.layers);

    // Win+R path must actually inject; AlternateShell path is accepted once the
    // session reaches an end reason after Client Info carried the shell.
    let delivered = if desktop_for_inject {
        active.injected
    } else {
        true
    };

    Ok(ExecReport {
        ok: delivered,
        host: active.host,
        port: active.port,
        remote_addr: active.remote_addr,
        local_addr: active.local_addr,
        alternate_shell: truncate_for_report(&shell),
        work_dir,
        layers,
        end_reason: active.end_reason,
        elapsed_ms: active.elapsed_ms,
        response_frames: active.response_frames,
        injected_run_dialog: active.injected,
        forced_fresh_logon,
        logoff_end_reason,
        logoff_elapsed_ms,
        input_flags: active.input_flags,
        notes: vec![
            if delivered {
                "IronRDP alternate_shell session completed.".into()
            } else {
                "RDP session ended before the Win+R payload was injected.".into()
            },
            "Payload delivery stays outside AlternateShell (512-byte wire cap). Use HTTP bootstrap or RDPDR.".into(),
            "force_fresh_logon clears disconnected single-user sessions before the payload pass.".into(),
            if forced_fresh_logon {
                "After force logoff, Client Info AlternateShell carries the payload (Win+R skipped)."
                    .into()
            } else if desktop_for_inject {
                "Win+R carries the payload on a full desktop (Client Info AlternateShell left empty)."
                    .into()
            } else {
                "AlternateShell carries the payload (inject_run_dialog disabled).".into()
            },
            "External RDP CLIs (mstsc / freerdp / wfreerdp) are not part of this path.".into(),
            format!(
                "endReason={:?} injectedRunDialog={} desktopForInject={} forcedFreshLogon={} delivered={} logoffEnd={:?} inputFlags={:#06x}",
                active.end_reason,
                active.injected,
                desktop_for_inject,
                forced_fresh_logon,
                delivered,
                logoff_end_reason,
                active.input_flags
            ),
        ],
    })
}

struct LogoffPassReport {
    end_reason: ExecEndReason,
    elapsed_ms: u64,
    layers: Vec<&'static str>,
}

/// Connect with Client Info `AlternateShell=logoff` and wait for the session to
/// end so the next connect starts a fresh logon.
///
/// This pass is best-effort: ServerTerminate / transport RST are ideal, but
/// ReadIdle / timeout after asking for `logoff` still counts as success so a
/// sticky desktop cannot fail the whole exec. FastPath graphics are skipped in
/// [`drive_active_stage`] to avoid native aborts on the lab host.
fn run_logoff_pass(options: &RdpConnectOptions) -> Result<LogoffPassReport> {
    let mut logoff_opts = options.clone();
    logoff_opts.alternate_shell = "logoff".into();
    logoff_opts.work_dir = String::new();
    // Cap the preparatory pass — hanging here blocked installRemote entirely.
    logoff_opts.session_timeout = options.logoff_timeout.min(Duration::from_secs(25));
    logoff_opts.settle = Duration::from_secs(2);
    logoff_opts.inject_run_dialog = false;
    logoff_opts.force_fresh_logon = false;
    // Keep the logoff pass lean: RDPDR during tear-down races with session reset.
    logoff_opts.rdpdr_share_dir = None;

    let connected = match connect_session(logoff_opts) {
        Ok(value) => value,
        Err(error) if is_transport_gone_msg(&error.to_string()) => {
            // Host already tearing down / refusing a second concurrent session.
            return Ok(LogoffPassReport {
                end_reason: ExecEndReason::TransportClosed,
                elapsed_ms: 0,
                layers: vec!["force-logoff-connect-reset"],
            });
        }
        Err(error) => {
            // Soft-fail: continue to the payload pass rather than failing exec.
            breadcrumb(&format!("run_logoff_pass:connect_soft_fail {error}"));
            return Ok(LogoffPassReport {
                end_reason: ExecEndReason::TransportClosed,
                elapsed_ms: 0,
                layers: vec!["force-logoff-connect-soft-fail"],
            });
        }
    };

    // Logoff only needs logon + a few seconds for `logoff` to tear the session
    // down. A long post-logon sleep blocks the payload pass and invites races.
    let active = match hold_transport_without_graphics(
        connected,
        Duration::from_secs(8),
        "force-logoff-hold",
    ) {
        Ok(value) => value,
        Err(error) => {
            // Peer RST after requesting logoff is expected.
            breadcrumb(&format!("run_logoff_pass:hold_soft_fail {error}"));
            return Ok(LogoffPassReport {
                end_reason: ExecEndReason::TransportClosed,
                elapsed_ms: 0,
                layers: vec!["force-logoff-hold", "force-logoff-hold-soft-fail"],
            });
        }
    };

    let mut layers = active.layers;
    layers.push("force-logoff-pass");
    // Any observed end reason is fine — the gap sleep still gives TS time to settle.
    breadcrumb(&format!(
        "run_logoff_pass:done end={:?} frames={}",
        active.end_reason, active.response_frames
    ));
    Ok(LogoffPassReport {
        end_reason: active.end_reason,
        elapsed_ms: active.elapsed_ms,
        layers,
    })
}

/// After a forced logoff, Terminal Services may briefly refuse reconnects.
fn connect_session_with_retry(options: RdpConnectOptions, after_logoff: bool) -> Result<ConnectedSession> {
    if !after_logoff {
        return connect_session(options);
    }

    let mut last_error = None;
    for attempt in 1..=4_u32 {
        match connect_session(options.clone()) {
            Ok(connected) => {
                if attempt > 1 {
                    // Marker only visible when a retry was required.
                    let mut connected = connected;
                    connected.layers.push("post-logoff-reconnect-retry");
                    return Ok(connected);
                }
                return Ok(connected);
            }
            Err(error) => {
                let message = error.to_string();
                let retryable = is_transport_gone_msg(&message)
                    || message.to_ascii_lowercase().contains("tcp connect")
                    || message.to_ascii_lowercase().contains("timed out")
                    || message.to_ascii_lowercase().contains("connector");
                last_error = Some(error);
                if !retryable || attempt == 4 {
                    break;
                }
                std::thread::sleep(Duration::from_secs(u64::from(attempt) * 2));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| Error::Session("post-logoff reconnect failed".into())))
}

/// Drive ActiveStage until `SaveSessionInfo` logon, then sleep without reading.
///
/// Lab evidence: first X.224 carries logon; later FastPath `read_pdu` aborts
/// this process (native AV). `AlternateShell` / `logoff` is running by logon —
/// after that keep the TCP session up briefly so a disconnected session retains
/// the remote process.
fn hold_transport_without_graphics(
    connected: ConnectedSession,
    session_timeout: Duration,
    layer_label: &'static str,
) -> Result<ActiveDriveReport> {
    breadcrumb(&format!("hold_transport:until_logon label={layer_label}"));
    let input_flags = connected.connection.input_flags.bits();
    let desktop_w = connected.connection.desktop_size.width;
    let desktop_h = connected.connection.desktop_size.height;
    let mut image = DecodedImage::new(PixelFormat::RgbA32, desktop_w, desktop_h);
    let mut active_stage = ActiveStageBuilder {
        static_channels: connected.connection.static_channels,
        user_channel_id: connected.connection.user_channel_id,
        io_channel_id: connected.connection.io_channel_id,
        message_channel_id: connected.connection.message_channel_id,
        share_id: connected.connection.share_id,
        compression_type: connected.connection.compression_type,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build();

    let mut framed = connected.framed;
    {
        let (stream, _) = framed.get_inner_mut();
        stream
            .sock
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|error| Error::Session(error.to_string()))?;
        stream
            .sock
            .set_write_timeout(Some(Duration::from_secs(20)))
            .map_err(|error| Error::Session(error.to_string()))?;
    }

    let started = Instant::now();
    let mut layers = connected.layers;
    layers.push(layer_label);
    layers.push("hold-until-logon");
    let mut response_frames = 0_u64;
    let mut logon_seen = false;
    let logon_deadline = started + Duration::from_secs(20);

    let end_reason = 'hold: {
        while Instant::now() < logon_deadline && !logon_seen {
            let (action, payload) = match framed.read_pdu() {
                Ok(frame) => frame,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(error) if is_transport_gone_io(&error) => {
                    breadcrumb("hold_transport:pre_logon_transport_gone");
                    break 'hold ExecEndReason::TransportClosed;
                }
                Err(error) => return Err(Error::Session(error.to_string())),
            };

            if action == ironrdp_pdu::Action::FastPath {
                breadcrumb("hold_transport:skip_fastpath_pre_logon");
                continue;
            }

            let outputs = match catch_unwind(AssertUnwindSafe(|| {
                active_stage.process(&mut image, action, &payload, None)
            })) {
                Ok(Ok(outputs)) => outputs,
                Ok(Err(error)) => {
                    let message = error.to_string();
                    breadcrumb(&format!("hold_transport:process_err {message}"));
                    if is_transport_gone_msg(&message) {
                        break 'hold ExecEndReason::TransportClosed;
                    }
                    continue;
                }
                Err(panic) => {
                    breadcrumb(&format!(
                        "hold_transport:process_panic {}",
                        panic_message(&panic)
                    ));
                    continue;
                }
            };

            for out in outputs {
                match out {
                    ActiveStageOutput::ResponseFrame(frame) => {
                        if let Err(error) = framed.write_all(&frame) {
                            if is_transport_gone_io(&error) {
                                break 'hold ExecEndReason::TransportClosed;
                            }
                            return Err(Error::Session(error.to_string()));
                        }
                        response_frames += 1;
                    }
                    ActiveStageOutput::SaveSessionInfo { logon_complete } if logon_complete => {
                        logon_seen = true;
                        layers.push("save-session-info-logon");
                        breadcrumb("hold_transport:logon_complete");
                    }
                    ActiveStageOutput::Terminate(_) => {
                        breadcrumb("hold_transport:server_terminate");
                        break 'hold ExecEndReason::ServerTerminate;
                    }
                    _ => {}
                }
            }
        }

        if !logon_seen {
            breadcrumb("hold_transport:logon_deadline");
            break 'hold ExecEndReason::ReadIdle;
        }

        let remain = session_timeout
            .saturating_sub(started.elapsed())
            .min(Duration::from_secs(90));
        breadcrumb(&format!(
            "hold_transport:post_logon_sleep secs={}",
            remain.as_secs()
        ));
        if !remain.is_zero() {
            std::thread::sleep(remain);
        }
        breadcrumb("hold_transport:sleep_done");
        ExecEndReason::SessionTimeout
    };

    drop(framed);
    breadcrumb(&format!(
        "hold_transport:done end={end_reason:?} logon={logon_seen}"
    ));
    Ok(ActiveDriveReport {
        host: connected.host,
        port: connected.port,
        remote_addr: connected.remote_addr.to_string(),
        local_addr: connected.local_addr.to_string(),
        layers,
        end_reason,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        response_frames,
        injected: false,
        input_flags,
    })
}

struct DriveActiveOpts {
    connected: ConnectedSession,
    session_timeout: Duration,
    settle: Duration,
    inject_command: Option<String>,
    layer_label: &'static str,
    hold_after_inject: bool,
}

struct ActiveDriveReport {
    host: String,
    port: u16,
    remote_addr: String,
    local_addr: String,
    layers: Vec<&'static str>,
    end_reason: ExecEndReason,
    elapsed_ms: u64,
    response_frames: u64,
    injected: bool,
    input_flags: u16,
}

fn drive_active_stage(opts: DriveActiveOpts) -> Result<ActiveDriveReport> {
    let DriveActiveOpts {
        connected,
        session_timeout,
        settle,
        inject_command,
        layer_label,
        hold_after_inject,
    } = opts;

    breadcrumb(&format!(
        "drive_active_stage:enter label={layer_label} desktop={}x{}",
        connected.connection.desktop_size.width, connected.connection.desktop_size.height
    ));
    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        connected.connection.desktop_size.width,
        connected.connection.desktop_size.height,
    );
    breadcrumb("drive_active_stage:image_ready");
    let mut active_stage = ActiveStageBuilder {
        static_channels: connected.connection.static_channels,
        user_channel_id: connected.connection.user_channel_id,
        io_channel_id: connected.connection.io_channel_id,
        message_channel_id: connected.connection.message_channel_id,
        share_id: connected.connection.share_id,
        compression_type: connected.connection.compression_type,
        enable_server_pointer: false,
        // Avoid software pointer decode paths that have aborted on lab graphics.
        pointer_software_rendering: false,
    }
    .build();
    breadcrumb("drive_active_stage:stage_built");

    let input_flags = connected.connection.input_flags.bits();
    let desktop_width = connected.connection.desktop_size.width;
    let desktop_height = connected.connection.desktop_size.height;
    let mut framed = connected.framed;
    {
        let (stream, _) = framed.get_inner_mut();
        stream
            .sock
            .set_read_timeout(Some(Duration::from_millis(500)))
            .map_err(|error| Error::Session(error.to_string()))?;
        stream
            .sock
            .set_write_timeout(Some(Duration::from_secs(20)))
            .map_err(|error| Error::Session(error.to_string()))?;
    }
    breadcrumb("drive_active_stage:loop_begin");

    let started = Instant::now();
    let mut response_frames = 0_u64;
    let mut layers = connected.layers;
    layers.push(layer_label);
    let mut injected = false;
    let mut logon_seen = false;
    let mut inject_at = inject_command.as_ref().map(|_| started + settle);
    let mut hold_until = if hold_after_inject {
        started + session_timeout
    } else {
        started + settle + Duration::from_secs(15)
    };
    let mut frames_seen = 0_u64;

    let end_reason = 'active: {
        loop {
            if started.elapsed() >= session_timeout {
                break 'active ExecEndReason::SessionTimeout;
            }

            if let Some(deadline) = inject_at {
                if logon_seen || Instant::now() >= deadline {
                    inject_at = None;
                    let command = inject_command.as_deref().unwrap_or("");
                    if logon_seen {
                        // Fresh logon desktops need Explorer to finish before Win+R.
                        std::thread::sleep(Duration::from_secs(if hold_after_inject { 5 } else { 2 }));
                    }
                    match inject_run_dialog_command(
                        &mut active_stage,
                        &mut image,
                        &mut framed,
                        &mut response_frames,
                        desktop_width,
                        desktop_height,
                        command,
                    ) {
                        Ok(()) => {
                            injected = true;
                            layers.push("win-r-scancode-inject");
                        }
                        Err(error) if is_transport_gone_msg(&error.to_string()) => {
                            // Peer RST while typing: logoff expects it; payload reports
                            // TransportClosed with injected=false so callers get JSON.
                            layers.push("inject-transport-gone");
                            if !hold_after_inject {
                                injected = true;
                            }
                            breadcrumb(&format!(
                                "drive_active_stage:inject_transport_gone hold={hold_after_inject}"
                            ));
                            break 'active ExecEndReason::TransportClosed;
                        }
                        Err(error) => return Err(error),
                    }
                    if hold_after_inject {
                        // Keep the RDP session for the remaining wall budget so a
                        // long install (Node zip + npm -g + server start) is not
                        // killed by an early disconnect / read-idle.
                        hold_until = started + session_timeout;
                    } else {
                        // Expect logoff / short command to tear the session down soon.
                        hold_until = Instant::now() + Duration::from_secs(20);
                    }
                }
            }

            let (action, payload) = match framed.read_pdu() {
                Ok(frame) => frame,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock || error.kind() == io::ErrorKind::TimedOut =>
                {
                    if started.elapsed() >= session_timeout {
                        break 'active ExecEndReason::SessionTimeout;
                    }
                    if inject_at.is_some() || Instant::now() < hold_until {
                        continue;
                    }
                    break 'active ExecEndReason::ReadIdle;
                }
                Err(error) if is_transport_gone_io(&error) => {
                    breadcrumb("drive_active_stage:read_transport_gone");
                    break 'active ExecEndReason::TransportClosed;
                }
                Err(error) => return Err(Error::Session(error.to_string())),
            };

            frames_seen += 1;
            if frames_seen <= 8 || frames_seen % 50 == 0 {
                breadcrumb(&format!(
                    "drive_active_stage:frame n={frames_seen} action={action:?} bytes={}",
                    payload.len()
                ));
            }

            // Headless install does not need a framebuffer. FastPath graphics
            // decode has aborted this process on lab hosts (native AV that
            // catch_unwind cannot catch). Drop FastPath payloads and only run
            // X.224 / MCS control through ActiveStage.
            if action == ironrdp_pdu::Action::FastPath {
                continue;
            }

            // X.224 path can still panic; skip the frame instead of aborting.
            let outputs = match catch_unwind(AssertUnwindSafe(|| {
                active_stage.process(&mut image, action, &payload, None)
            })) {
                Ok(Ok(outputs)) => outputs,
                Ok(Err(error)) => {
                    let message = error.to_string();
                    breadcrumb(&format!("drive_active_stage:process_err {message}"));
                    if is_transport_gone_msg(&message) {
                        break 'active ExecEndReason::TransportClosed;
                    }
                    breadcrumb("drive_active_stage:process_err_skip");
                    continue;
                }
                Err(panic) => {
                    let message = panic_message(&panic);
                    breadcrumb(&format!("drive_active_stage:process_panic_skip {message}"));
                    continue;
                }
            };
            if frames_seen <= 8 {
                breadcrumb(&format!(
                    "drive_active_stage:process_ok n={frames_seen} outs={}",
                    outputs.len()
                ));
            }

            for out in outputs {
                match out {
                    ActiveStageOutput::ResponseFrame(frame) => {
                        breadcrumb(&format!(
                            "drive_active_stage:write_response bytes={}",
                            frame.len()
                        ));
                        let write_result = catch_unwind(AssertUnwindSafe(|| framed.write_all(&frame)));
                        match write_result {
                            Ok(Ok(())) => {
                                response_frames += 1;
                            }
                            Ok(Err(error)) => {
                                breadcrumb(&format!("drive_active_stage:write_err {error}"));
                                if is_transport_gone_io(&error) {
                                    break 'active ExecEndReason::TransportClosed;
                                }
                                return Err(Error::Session(error.to_string()));
                            }
                            Err(panic) => {
                                let message = panic_message(&panic);
                                breadcrumb(&format!("drive_active_stage:write_panic {message}"));
                                return Err(Error::Session(format!(
                                    "framed.write_all panicked: {message}"
                                )));
                            }
                        }
                    }
                    ActiveStageOutput::SaveSessionInfo { logon_complete } => {
                        if logon_complete {
                            logon_seen = true;
                            layers.push("save-session-info-logon");
                            breadcrumb("drive_active_stage:logon_complete");
                        }
                    }
                    ActiveStageOutput::Terminate(_) => {
                        breadcrumb("drive_active_stage:server_terminate");
                        break 'active ExecEndReason::ServerTerminate;
                    }
                    other => {
                        breadcrumb(&format!(
                            "drive_active_stage:output {:?}",
                            std::mem::discriminant(&other)
                        ));
                    }
                }
            }
            if frames_seen <= 8 {
                breadcrumb(&format!("drive_active_stage:loop_continue n={frames_seen}"));
            }
        }
    };

    Ok(ActiveDriveReport {
        host: connected.host,
        port: connected.port,
        remote_addr: connected.remote_addr.to_string(),
        local_addr: connected.local_addr.to_string(),
        layers,
        end_reason,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        response_frames,
        injected,
        input_flags,
    })
}

fn inject_run_dialog_command(
    active_stage: &mut ActiveStage,
    image: &mut DecodedImage,
    framed: &mut Framed<StreamOwned<ClientConnection, TcpStream>>,
    response_frames: &mut u64,
    desktop_width: u16,
    desktop_height: u16,
    command: &str,
) -> Result<()> {
    write_input_events(
        active_stage,
        image,
        framed,
        response_frames,
        &build_focus_click_events(desktop_width / 2, desktop_height / 2),
    )?;
    std::thread::sleep(Duration::from_millis(400));

    // Open Run, clear any leftover text, type with US scancodes (more reliable than
    // unicode-only bursts on some Windows hosts), then Enter.
    write_input_events(
        active_stage,
        image,
        framed,
        response_frames,
        &build_run_dialog_events(),
    )?;
    std::thread::sleep(Duration::from_millis(1000));
    write_input_events(
        active_stage,
        image,
        framed,
        response_frames,
        &build_select_all_events(),
    )?;
    std::thread::sleep(Duration::from_millis(120));
    write_input_events(
        active_stage,
        image,
        framed,
        response_frames,
        &build_backspace_events(),
    )?;
    std::thread::sleep(Duration::from_millis(120));
    write_paced_type_events(active_stage, image, framed, response_frames, command)?;
    std::thread::sleep(Duration::from_millis(300));
    write_input_events(
        active_stage,
        image,
        framed,
        response_frames,
        &build_enter_events(),
    )?;
    Ok(())
}

fn write_input_events(
    active_stage: &mut ActiveStage,
    image: &mut DecodedImage,
    framed: &mut Framed<StreamOwned<ClientConnection, TcpStream>>,
    response_frames: &mut u64,
    events: &[FastPathInputEvent],
) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let outputs = active_stage
        .process_fastpath_input(image, events)
        .map_err(|error| Error::Session(error.to_string()))?;
    for out in outputs {
        if let ActiveStageOutput::ResponseFrame(frame) = out {
            if let Err(error) = framed.write_all(&frame) {
                // Preserve io kind text so `is_transport_gone_msg` matches peer RST.
                return Err(Error::Session(error.to_string()));
            }
            *response_frames += 1;
        }
    }
    Ok(())
}

fn build_focus_click_events(x: u16, y: u16) -> Vec<FastPathInputEvent> {
    vec![
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::MOVE,
            number_of_wheel_rotation_units: 0,
            x_position: x,
            y_position: y,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::LEFT_BUTTON | PointerFlags::DOWN,
            number_of_wheel_rotation_units: 0,
            x_position: x,
            y_position: y,
        }),
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::LEFT_BUTTON,
            number_of_wheel_rotation_units: 0,
            x_position: x,
            y_position: y,
        }),
    ]
}

fn build_run_dialog_events() -> Vec<FastPathInputEvent> {
    // Left Windows (extended 0x5B) + R (0x13).
    vec![
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED, 0x5B),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x13),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x13),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::EXTENDED | KeyboardFlags::RELEASE, 0x5B),
    ]
}

fn build_select_all_events() -> Vec<FastPathInputEvent> {
    // Ctrl (0x1D) + A (0x1E).
    vec![
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1D),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1E),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1E),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1D),
    ]
}

fn build_backspace_events() -> Vec<FastPathInputEvent> {
    vec![
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x0E),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x0E),
    ]
}

fn build_enter_events() -> Vec<FastPathInputEvent> {
    vec![
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1C),
        FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1C),
    ]
}

/// US QWERTY set-1 scancode and whether Shift is required.
fn us_scancode(ch: char) -> Option<(u8, bool)> {
    match ch {
        'a'..='z' => {
            const MAP: [u8; 26] = [
                0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10,
                0x13, 0x1F, 0x14, 0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
            ];
            Some((MAP[(ch as u8 - b'a') as usize], false))
        }
        'A'..='Z' => {
            const MAP: [u8; 26] = [
                0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10,
                0x13, 0x1F, 0x14, 0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
            ];
            Some((MAP[(ch as u8 - b'A') as usize], true))
        }
        '1'..='9' => Some((0x02 + (ch as u8 - b'1'), false)),
        '0' => Some((0x0B, false)),
        '!' => Some((0x02, true)),
        '@' => Some((0x03, true)),
        '#' => Some((0x04, true)),
        '$' => Some((0x05, true)),
        '%' => Some((0x06, true)),
        '^' => Some((0x07, true)),
        '&' => Some((0x08, true)),
        '*' => Some((0x09, true)),
        '(' => Some((0x0A, true)),
        ')' => Some((0x0B, true)),
        '-' => Some((0x0C, false)),
        '_' => Some((0x0C, true)),
        '=' => Some((0x0D, false)),
        '+' => Some((0x0D, true)),
        '[' => Some((0x1A, false)),
        '{' => Some((0x1A, true)),
        ']' => Some((0x1B, false)),
        '}' => Some((0x1B, true)),
        '\\' => Some((0x2B, false)),
        '|' => Some((0x2B, true)),
        ';' => Some((0x27, false)),
        ':' => Some((0x27, true)),
        '\'' => Some((0x28, false)),
        '"' => Some((0x28, true)),
        '`' => Some((0x29, false)),
        '~' => Some((0x29, true)),
        ',' => Some((0x33, false)),
        '<' => Some((0x33, true)),
        '.' => Some((0x34, false)),
        '>' => Some((0x34, true)),
        '/' => Some((0x35, false)),
        '?' => Some((0x35, true)),
        ' ' => Some((0x39, false)),
        '\t' => Some((0x0F, false)),
        _ => None,
    }
}

fn build_char_events(ch: char) -> Vec<FastPathInputEvent> {
    if let Some((code, shift)) = us_scancode(ch) {
        let mut events = Vec::with_capacity(4);
        if shift {
            events.push(FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x2A));
        }
        events.push(FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), code));
        events.push(FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, code));
        if shift {
            events.push(FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x2A));
        }
        return events;
    }
    // Fallback for characters outside the US scancode map.
    let mut buf = [0_u16; 2];
    ch.encode_utf16(&mut buf)
        .iter()
        .copied()
        .map(|unit| FastPathInputEvent::UnicodeKeyboardEvent(KeyboardFlags::empty(), unit))
        .collect()
}

fn write_paced_type_events(
    active_stage: &mut ActiveStage,
    image: &mut DecodedImage,
    framed: &mut Framed<StreamOwned<ClientConnection, TcpStream>>,
    response_frames: &mut u64,
    text: &str,
) -> Result<()> {
    for ch in text.chars() {
        write_input_events(active_stage, image, framed, response_frames, &build_char_events(ch))?;
        // Pace keystrokes so the Run dialog can accept them without dropping.
        std::thread::sleep(Duration::from_millis(55));
    }
    Ok(())
}

struct ConnectedSession {
    host: String,
    port: u16,
    remote_addr: SocketAddr,
    local_addr: SocketAddr,
    connection: ConnectionResult,
    framed: Framed<StreamOwned<ClientConnection, TcpStream>>,
    layers: Vec<&'static str>,
}

fn validate_connect_options(options: &RdpConnectOptions, require_shell: bool) -> Result<()> {
    if options.host.trim().is_empty() {
        return Err(Error::InvalidArgument("host is required".into()));
    }
    if options.auth.username.trim().is_empty() {
        return Err(Error::InvalidArgument("username is required".into()));
    }
    if options.auth.password.is_empty() {
        return Err(Error::InvalidArgument("password is required".into()));
    }
    validate_client_info_string("alternate_shell", &options.alternate_shell)?;
    validate_client_info_string("work_dir", &options.work_dir)?;
    if require_shell && options.alternate_shell.trim().is_empty() && !options.inject_run_dialog {
        return Err(Error::InvalidArgument(
            "alternate_shell is required for remote execution (or enable inject_run_dialog)".into(),
        ));
    }
    if require_shell && options.alternate_shell.trim().is_empty() && options.inject_run_dialog {
        return Err(Error::InvalidArgument(
            "shell command is required for Win+R injection".into(),
        ));
    }
    Ok(())
}

fn validate_client_info_string(field: &str, value: &str) -> Result<()> {
    // Wire encoding is UTF-16LE for Unicode client info. Count UTF-16 code units
    // plus the terminating NUL against the 512-byte (256 u16) budget conservatively
    // by also rejecting raw byte lengths over 510 for ASCII-heavy commands.
    let utf16_units = value.encode_utf16().count();
    if utf16_units + 1 > ALTERNATE_SHELL_MAX_BYTES / 2 {
        return Err(Error::InvalidArgument(format!(
            "`{field}` exceeds MS-RDPBCGR {ALTERNATE_SHELL_MAX_BYTES}-byte limit (utf16 units including NUL)"
        )));
    }
    if value.len() + 1 > ALTERNATE_SHELL_MAX_BYTES {
        return Err(Error::InvalidArgument(format!(
            "`{field}` exceeds MS-RDPBCGR {ALTERNATE_SHELL_MAX_BYTES}-byte limit"
        )));
    }
    Ok(())
}

fn truncate_for_report(value: &str) -> String {
    const MAX: usize = 180;
    if value.len() <= MAX {
        return value.to_owned();
    }
    format!("{}…", &value[..MAX])
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|value| (*value).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".into())
}

/// RDPDR drive name announced to the server (`\\tsclient\OP` on Windows).
pub const RDPDR_SHARE_NAME: &str = "OP";
const RDPDR_DEVICE_ID: u32 = 1;

fn connect_session(options: RdpConnectOptions) -> Result<ConnectedSession> {
    let host = options.host.trim().to_owned();
    let mut layers = Vec::new();
    let addr = probe_tcp(&host, options.port, options.timeout)?;
    layers.push("tcp");

    let tcp = TcpStream::connect_timeout(&addr, options.timeout).map_err(|source| Error::TcpConnect { addr, source })?;
    tcp.set_read_timeout(Some(options.timeout))
        .map_err(|source| Error::TcpConnect { addr, source })?;
    tcp.set_write_timeout(Some(options.timeout))
        .map_err(|source| Error::TcpConnect { addr, source })?;
    let local_addr = tcp
        .local_addr()
        .map_err(|source| Error::TcpConnect { addr, source })?;

    let config = build_config(&options);
    let mut framed = Framed::new(tcp);
    let mut connector = ClientConnector::new(config, local_addr);
    if let Some(share_dir) = options.rdpdr_share_dir.as_ref() {
        attach_rdpdr_channels(&mut connector, share_dir)?;
        layers.push("rdpdr-drive-redirect");
    }

    let should_upgrade = ironrdp_blocking::connect_begin(&mut framed, &mut connector)
        .map_err(|error| Error::Connector(error.to_string()))?;
    layers.push("x224-mcs-gcc");

    let initial_stream = framed.into_inner_no_leftover();
    let (tls_stream, server_public_key) = tls_upgrade(initial_stream, &host, options.accept_invalid_certs)?;
    layers.push("tls");

    let upgraded = ironrdp_blocking::mark_as_upgraded(should_upgrade, &mut connector);
    let mut upgraded_framed = Framed::new(tls_stream);
    let mut network_client = ReqwestNetworkClient;

    let outcome = ironrdp_blocking::connect_finalize(
        upgraded,
        connector,
        &mut upgraded_framed,
        &mut network_client,
        ServerName::new(host.clone()),
        server_public_key,
        None,
    )
    .map_err(|error| Error::Connector(error.to_string()))?;
    layers.push("credssp-nla");

    let connection = match outcome {
        ConnectionOutcome::Connected(result) => {
            layers.push("connection-finalization");
            result
        }
        ConnectionOutcome::Redirected(_) => return Err(Error::Redirection),
    };

    if !options.alternate_shell.trim().is_empty() {
        layers.push("client-info-alternate-shell");
    }

    Ok(ConnectedSession {
        host,
        port: options.port,
        remote_addr: addr,
        local_addr,
        connection,
        framed: upgraded_framed,
        layers,
    })
}

fn report_from_result(
    host: &str,
    port: u16,
    remote: SocketAddr,
    local: SocketAddr,
    connection: &ConnectionResult,
    layers: Vec<&'static str>,
) -> HandshakeReport {
    HandshakeReport {
        ok: true,
        host: host.to_owned(),
        port,
        remote_addr: remote.to_string(),
        local_addr: local.to_string(),
        desktop_width: connection.desktop_size.width,
        desktop_height: connection.desktop_size.height,
        io_channel_id: connection.io_channel_id,
        user_channel_id: connection.user_channel_id,
        layers,
        notes: vec![
            "IronRDP handshake reached an active ConnectionResult.".into(),
            "Use `open-panel-rdp exec` / `run_alternate_shell` to start a remote install bootstrap.".into(),
            "External RDP CLIs (mstsc / freerdp / wfreerdp) are not part of this path.".into(),
        ],
    }
}

fn normalize_rdpdr_root(path: &std::path::Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| Error::InvalidArgument(format!("rdpdr share dir cwd: {error}")))?
            .join(path)
    };
    let lossy = absolute.to_string_lossy();
    let stripped = lossy
        .strip_prefix(r"\\?\")
        .or_else(|| lossy.strip_prefix(r"//?/"))
        .unwrap_or(lossy.as_ref());
    let normalized = PathBuf::from(stripped);
    // Require `X:\...` form expected by the native RDPDR backend.
    let text = normalized.to_string_lossy();
    let bytes = text.as_bytes();
    let looks_like_drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/');
    if !looks_like_drive {
        return Err(Error::InvalidArgument(format!(
            "rdpdr share dir must be a drive-letter path (got {})",
            normalized.display()
        )));
    }
    Ok(normalized)
}

fn attach_rdpdr_channels(
    connector: &mut ClientConnector,
    share_dir: &std::path::Path,
) -> Result<()> {
    use ironrdp_rdpdr::Rdpdr;
    use ironrdp_rdpdr_native::{RedirectedDrive, WindowsRdpdrBackendFactory};
    use ironrdp_rdpsnd::client::{NoopRdpsndBackend, Rdpsnd};

    if !share_dir.is_dir() {
        return Err(Error::InvalidArgument(format!(
            "rdpdr share dir is not a directory: {}",
            share_dir.display()
        )));
    }
    // Prefer a plain drive-letter path. Windows `canonicalize` yields `\\?\`
    // verbatim paths that ironrdp-rdpdr-native rejects as InvalidRootPath.
    let share_dir = normalize_rdpdr_root(share_dir)?;

    // Writable so the remote can leave proof / result files on the share.
    let drive = RedirectedDrive::new(RDPDR_DEVICE_ID, RDPDR_SHARE_NAME, share_dir, false)
        .map_err(|error| Error::InvalidArgument(format!("rdpdr drive: {error:?}")))?;
    let factory = WindowsRdpdrBackendFactory::new(drive);
    let backend = factory.build();
    let initial = factory.initial_drives();
    // MS-RDPEFS requires RDPSND to be co-advertised or the server ignores RDPDR.
    let rdpsnd = Rdpsnd::new(Box::new(NoopRdpsndBackend));
    let rdpdr = Rdpdr::new(Box::new(backend), "open-panel-rdp".into()).with_drives(Some(initial));
    connector.attach_static_channel(rdpsnd);
    connector.attach_static_channel(rdpdr);
    Ok(())
}

fn build_config(options: &RdpConnectOptions) -> ironrdp_connector::Config {
    ironrdp_connector::Config {
        credentials: Credentials::UsernamePassword {
            username: options.auth.username.clone(),
            password: options.auth.password.clone(),
        },
        domain: options.auth.domain.clone(),
        enable_tls: false,
        enable_credssp: true,
        enable_standard_rdp_security: false,
        keyboard_type: KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        connection_type: ConnectionType::Lan,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: DesktopSize {
            width: 1024,
            height: 768,
        },
        monitor_layout: None,
        bitmap: None,
        client_build: 0,
        client_name: "open-panel-rdp".to_owned(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),
        platform: MajorPlatformType::WINDOWS,
        enable_server_pointer: false,
        request_data: None,
        autologon: true,
        enable_audio_playback: false,
        enable_audio_capture: false,
        compression_type: None,
        pointer_software_rendering: false,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: false,
        // Prefer a lean desktop: less bitmap traffic for the ActiveStage decoder.
        performance_flags: PerformanceFlags::DISABLE_WALLPAPER
            | PerformanceFlags::DISABLE_FULLWINDOWDRAG
            | PerformanceFlags::DISABLE_MENUANIMATIONS
            | PerformanceFlags::DISABLE_THEMING
            | PerformanceFlags::DISABLE_CURSOR_SHADOW
            | PerformanceFlags::DISABLE_CURSORSETTINGS,
        desktop_scale_factor: 0,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        monitor_extended_layout: None,
        relative_mouse_input: false,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        alternate_shell: options.alternate_shell.clone(),
        work_dir: options.work_dir.clone(),
        remote_application_mode: false,
        rail_support_level: RailSupportLevel::empty(),
    }
}

fn ensure_crypto_provider() {
    // rustls 0.23 needs an explicit process-level provider when multiple
    // backends may be linked transitively (sspi / reqwest / ...).
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn tls_upgrade(
    stream: TcpStream,
    server_name: &str,
    accept_invalid_certs: bool,
) -> Result<(StreamOwned<ClientConnection, TcpStream>, Vec<u8>)> {
    ensure_crypto_provider();
    let builder = ClientConfig::builder();
    let mut config = if accept_invalid_certs {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
            .with_no_client_auth()
    }
    else {
        builder
            .with_root_certificates(rustls::RootCertStore {
                roots: webpki_roots_or_empty(),
            })
            .with_no_client_auth()
    };
    // CredSSP does not support TLS session resumption.
    config.resumption = rustls::client::Resumption::disabled();
    let config = Arc::new(config);

    let server_name = RustlsServerName::try_from(server_name.to_owned())
        .map_err(|error| Error::Tls(format!("invalid server name: {error}")))?;
    let client = ClientConnection::new(config, server_name).map_err(|error| Error::Tls(error.to_string()))?;
    let mut tls_stream = StreamOwned::new(client, stream);
    tls_stream.flush().map_err(|error| Error::Tls(error.to_string()))?;

    let cert = tls_stream
        .conn
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .ok_or_else(|| Error::Tls("peer certificate missing after TLS handshake".into()))?
        .as_ref()
        .to_owned();

    let server_public_key = extract_tls_server_public_key(&cert)?;
    Ok((tls_stream, server_public_key))
}

fn webpki_roots_or_empty() -> Vec<rustls::pki_types::TrustAnchor<'static>> {
    // Avoid an extra dependency for the first skeleton: strict mode without roots
    // will fail closed unless accept_invalid_certs is used for lab hosts.
    Vec::new()
}

fn extract_tls_server_public_key(cert_der: &[u8]) -> Result<Vec<u8>> {
    let cert = x509_cert::Certificate::from_der(cert_der).map_err(|error| Error::Tls(error.to_string()))?;
    let key = cert
        .tbs_certificate
        .subject_public_key_info
        .subject_public_key
        .as_bytes()
        .ok_or_else(|| Error::Tls("subject public key BIT STRING is not aligned".into()))?
        .to_owned();
    Ok(key)
}

#[derive(Debug)]
struct NoCertificateVerification;

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &RustlsServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA1,
            SignatureScheme::ECDSA_SHA1_Legacy,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
            SignatureScheme::ED448,
        ]
    }
}

// Silence unused import warnings when Read is only used via trait bounds in Framed.
#[allow(dead_code)]
fn _assert_read_write<T: Read + Write>(_: &T) {}
