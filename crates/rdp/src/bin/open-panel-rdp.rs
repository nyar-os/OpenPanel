//! Thin product CLI over `open-panel-rdp` (no FreeRDP / mstsc).
//!
//! Password must come from the environment (`--password-env`), never argv.

use std::env;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use open_panel_rdp::{RdpAuth, RdpConnectOptions, handshake, probe_tcp, run_alternate_shell};
use serde_json::Value;

fn usage() -> ! {
    eprintln!(
        "usage:\n  open-panel-rdp probe --host <host> [--port 3389]\n  open-panel-rdp handshake --host <host> --user <name> --password-env <VAR> [--port 3389] [--domain <name>]\n  open-panel-rdp exec --host <host> --user <name> --password-env <VAR> (--shell <cmd> | --shell-env <VAR>) [--work-dir <path>] [--port 3389] [--domain <name>] [--session-timeout-secs <n>] [--rdpdr-dir <path>] [--force-fresh-logon|--no-force-fresh-logon] [--inject-run]"
    );
    std::process::exit(2);
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

fn arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2).find_map(|pair| (pair[0] == name).then_some(pair[1].as_str()))
}

fn emit_json(value: &Value) {
    println!("{value}");
    let _ = io::stdout().flush();
}

fn emit_pretty(value: &impl serde::Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => {
            println!("{text}");
            let _ = io::stdout().flush();
        }
        Err(error) => {
            emit_json(&serde_json::json!({
                "ok": false,
                "error": format!("failed to serialize report: {error}"),
                "stack": "open-panel-rdp (IronRDP)",
            }));
        }
    }
}

fn password_from_env(password_env: &str) -> Result<String, ExitCode> {
    match env::var(password_env) {
        Ok(value) if !value.is_empty() => Ok(value),
        _ => {
            emit_json(&serde_json::json!({
                "ok": false,
                "error": format!("environment variable `{password_env}` is empty or unset"),
            }));
            Err(ExitCode::from(2))
        }
    }
}

/// Resolve the AlternateShell / Win+R payload from `--shell` or `--shell-env`.
/// Prefer `--shell-env` on Windows hosts where argv quoting mangles spaces/quotes.
fn shell_from_args(args: &[String]) -> Result<String, ExitCode> {
    let shell_flag = arg_value(args, "--shell").map(str::to_owned);
    let shell_env = arg_value(args, "--shell-env");
    match (shell_flag, shell_env) {
        (Some(_), Some(_)) => {
            emit_json(&serde_json::json!({
                "ok": false,
                "error": "pass either `--shell` or `--shell-env`, not both",
            }));
            Err(ExitCode::from(2))
        }
        (Some(shell), None) => {
            if shell.trim().is_empty() {
                emit_json(&serde_json::json!({
                    "ok": false,
                    "error": "`--shell` must not be empty",
                }));
                return Err(ExitCode::from(2));
            }
            Ok(shell)
        }
        (None, Some(var)) => match env::var(var) {
            Ok(value) if !value.trim().is_empty() => Ok(value),
            _ => {
                emit_json(&serde_json::json!({
                    "ok": false,
                    "error": format!("environment variable `{var}` is empty or unset"),
                }));
                Err(ExitCode::from(2))
            }
        },
        (None, None) => {
            usage();
        }
    }
}

fn auth_options(args: &[String]) -> Result<(String, String, u16, Option<String>, String), ExitCode> {
    let host = arg_value(args, "--host").unwrap_or_else(|| usage()).to_owned();
    let user = arg_value(args, "--user").unwrap_or_else(|| usage()).to_owned();
    let password_env = arg_value(args, "--password-env").unwrap_or_else(|| usage());
    let port = arg_value(args, "--port")
        .unwrap_or("3389")
        .parse::<u16>()
        .unwrap_or_else(|_| usage());
    let domain = arg_value(args, "--domain").map(str::to_owned);
    let password = password_from_env(password_env)?;
    Ok((host, user, port, domain, password))
}

fn panic_to_string(payload: &Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|value| (*value).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".into())
}

fn main() -> ExitCode {
    // Always emit JSON on panic so Node façades never see an empty stdout crash.
    std::panic::set_hook(Box::new(|info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|value| (*value).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| info.to_string());
        // Write both streams: some hosts only keep one of the redirected pipes.
        let body = serde_json::json!({
            "ok": false,
            "error": format!("open-panel-rdp panicked: {message}"),
            "stack": "open-panel-rdp (IronRDP)",
        });
        println!("{body}");
        let _ = io::stdout().flush();
        eprintln!("{body}");
        let _ = io::stderr().flush();
    }));

    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        usage();
    }

    match args[0].as_str() {
        "probe" => {
            let host = arg_value(&args, "--host").unwrap_or_else(|| usage());
            let port = arg_value(&args, "--port")
                .unwrap_or("3389")
                .parse::<u16>()
                .unwrap_or_else(|_| usage());
            match probe_tcp(host, port, Duration::from_secs(5)) {
                Ok(addr) => {
                    emit_json(&serde_json::json!({
                        "ok": true,
                        "host": host,
                        "port": port,
                        "remoteAddr": addr.to_string(),
                        "channel": "rdp",
                        "stack": "open-panel-rdp (IronRDP)",
                    }));
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    emit_json(&serde_json::json!({ "ok": false, "error": error.to_string() }));
                    ExitCode::from(1)
                }
            }
        }
        "handshake" => {
            let (host, user, port, domain, password) = match auth_options(&args) {
                Ok(value) => value,
                Err(code) => return code,
            };

            let report = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handshake(RdpConnectOptions {
                    host: host.clone(),
                    port,
                    auth: RdpAuth {
                        username: user,
                        password,
                        domain,
                    },
                    accept_invalid_certs: true,
                    ..RdpConnectOptions::default()
                })
            }));

            match report {
                Ok(Ok(value)) => {
                    emit_pretty(&value);
                    ExitCode::SUCCESS
                }
                Ok(Err(error)) => {
                    emit_json(&serde_json::json!({
                        "ok": false,
                        "host": host,
                        "port": port,
                        "error": error.to_string(),
                        "stack": "open-panel-rdp (IronRDP)",
                    }));
                    ExitCode::from(1)
                }
                Err(panic) => {
                    emit_json(&serde_json::json!({
                        "ok": false,
                        "host": host,
                        "port": port,
                        "error": format!("open-panel-rdp panicked: {}", panic_to_string(&panic)),
                        "stack": "open-panel-rdp (IronRDP)",
                    }));
                    ExitCode::from(1)
                }
            }
        }
        "exec" => {
            let (host, user, port, domain, password) = match auth_options(&args) {
                Ok(value) => value,
                Err(code) => return code,
            };
            let shell = match shell_from_args(&args) {
                Ok(value) => value,
                Err(code) => return code,
            };
            let work_dir = arg_value(&args, "--work-dir").unwrap_or("").to_owned();
            let session_timeout_secs = arg_value(&args, "--session-timeout-secs")
                .unwrap_or("180")
                .parse::<u64>()
                .unwrap_or_else(|_| usage());
            // Prefer explicit flags from the TS façade. Default ON when neither is set
            // so disconnected single-user sessions still get a fresh AlternateShell.
            let force_fresh_logon = if has_flag(&args, "--no-force-fresh-logon") {
                false
            } else if has_flag(&args, "--force-fresh-logon") {
                true
            } else {
                true
            };
            let inject_run_dialog = !has_flag(&args, "--no-inject-run");
            let rdpdr_share_dir = arg_value(&args, "--rdpdr-dir").map(PathBuf::from);

            eprintln!(
                "[open-panel-rdp] exec begin host={host} port={port} forceFresh={force_fresh_logon} inject={inject_run_dialog}"
            );
            let _ = io::stderr().flush();

            let report = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_alternate_shell(RdpConnectOptions {
                    host: host.clone(),
                    port,
                    auth: RdpAuth {
                        username: user,
                        password,
                        domain,
                    },
                    accept_invalid_certs: true,
                    alternate_shell: shell,
                    work_dir,
                    timeout: Duration::from_secs(20),
                    session_timeout: Duration::from_secs(session_timeout_secs),
                    inject_run_dialog,
                    settle: Duration::from_secs(8),
                    force_fresh_logon,
                    logoff_timeout: Duration::from_secs(25),
                    rdpdr_share_dir,
                })
            }));

            match report {
                Ok(Ok(value)) => {
                    // Non-delivered Win+R sessions still return a report with ok=false.
                    emit_pretty(&value);
                    if value.ok {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(1)
                    }
                }
                Ok(Err(error)) => {
                    emit_json(&serde_json::json!({
                        "ok": false,
                        "host": host,
                        "port": port,
                        "error": error.to_string(),
                        "stack": "open-panel-rdp (IronRDP)",
                    }));
                    ExitCode::from(1)
                }
                Err(panic) => {
                    emit_json(&serde_json::json!({
                        "ok": false,
                        "host": host,
                        "port": port,
                        "error": format!("open-panel-rdp panicked: {}", panic_to_string(&panic)),
                        "stack": "open-panel-rdp (IronRDP)",
                    }));
                    ExitCode::from(1)
                }
            }
        }
        _ => usage(),
    }
}
