//! Pure-Rust RDP client surface for OpenPanel Windows remote install.
//!
//! Built on [IronRDP](https://github.com/Devolutions/IronRDP) (`ironrdp-connector` +
//! `ironrdp-blocking`). There is **no** `mstsc` / FreeRDP / `wfreerdp` CLI wrapper.
//!
//! ## Current layer
//!
//! - TCP connect to `:3389`
//! - X.224 / MCS / GCC negotiation (`connect_begin`)
//! - TLS upgrade
//! - CredSSP / NLA (`connect_finalize`)
//! - Connection finalization to an active session result
//!
//! ## Next layers (install path)
//!
//! 1. Deliver payload via RDPDR or clipboard / alternate shell staging
//! 2. Set `Config::alternate_shell` to run `powershell.exe -File …`
//! 3. Observe exit via a redirected channel or a pull-back sentinel
//! 4. Expose the same API through the N-API boundary for `open-panel remote install`

#![deny(missing_docs)]

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use ironrdp_blocking::Framed;
use ironrdp_connector::sspi::network_client::reqwest_network_client::ReqwestNetworkClient;
use ironrdp_connector::{
    ClientConnector, ConnectionOutcome, ConnectionResult, Credentials, DesktopSize, ServerName,
};
use ironrdp_pdu::gcc::{ConnectionType, KeyboardType};
use ironrdp_pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
use ironrdp_pdu::rdp::client_info::{PerformanceFlags, TimezoneInfo};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName as RustlsServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, Error as TlsError, SignatureScheme, StreamOwned};
use serde::Serialize;
use x509_cert::der::Decode as _;

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
/// [`ConnectionResult`] metadata. Remote command execution is intentionally not
/// attempted here yet — callers should use [`HandshakeReport::notes`].
pub fn handshake(options: RdpConnectOptions) -> Result<HandshakeReport> {
    let host = options.host.trim().to_owned();
    if host.is_empty() {
        return Err(Error::InvalidArgument("host is required".into()));
    }
    if options.auth.username.trim().is_empty() {
        return Err(Error::InvalidArgument("username is required".into()));
    }
    if options.auth.password.is_empty() {
        return Err(Error::InvalidArgument("password is required".into()));
    }

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

    Ok(report_from_result(
        &host,
        options.port,
        addr,
        local_addr,
        &connection,
        layers,
    ))
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
            "Next: stage install payload (RDPDR or alternate_shell) and run open-panel install without OpenSSH.".into(),
            "External RDP CLIs (mstsc / freerdp / wfreerdp) are not part of this path.".into(),
        ],
    }
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
        pointer_software_rendering: true,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: false,
        performance_flags: PerformanceFlags::default(),
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

fn tls_upgrade(
    stream: TcpStream,
    server_name: &str,
    accept_invalid_certs: bool,
) -> Result<(StreamOwned<ClientConnection, TcpStream>, Vec<u8>)> {
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
