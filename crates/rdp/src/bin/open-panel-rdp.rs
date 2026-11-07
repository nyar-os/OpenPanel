//! Thin product CLI over `open-panel-rdp` (no FreeRDP / mstsc).
//!
//! Password must come from the environment (`--password-env`), never argv.

use std::env;
use std::process::ExitCode;
use std::time::Duration;

use open_panel_rdp::{RdpAuth, RdpConnectOptions, handshake, probe_tcp, run_alternate_shell};

fn usage() -> ! {
    eprintln!(
        "usage:\n  open-panel-rdp probe --host <host> [--port 3389]\n  open-panel-rdp handshake --host <host> --user <name> --password-env <VAR> [--port 3389] [--domain <name>]\n  open-panel-rdp exec --host <host> --user <name> --password-env <VAR> --shell <cmd> [--work-dir <path>] [--port 3389] [--domain <name>] [--session-timeout-secs <n>] [--no-force-fresh-logon] [--no-inject-run]"
    );
    std::process::exit(2);
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

fn arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2).find_map(|pair| (pair[0] == name).then_some(pair[1].as_str()))
}

fn password_from_env(password_env: &str) -> Result<String, ExitCode> {
    match env::var(password_env) {
        Ok(value) if !value.is_empty() => Ok(value),
        _ => {
            println!(
                "{}",
                serde_json::json!({
                    "ok": false,
                    "error": format!("environment variable `{password_env}` is empty or unset"),
                })
            );
            Err(ExitCode::from(2))
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

fn main() -> ExitCode {
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
                    println!(
                        "{}",
                        serde_json::json!({
                            "ok": true,
                            "host": host,
                            "port": port,
                            "remoteAddr": addr.to_string(),
                            "channel": "rdp",
                            "stack": "open-panel-rdp (IronRDP)",
                        })
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    println!("{}", serde_json::json!({ "ok": false, "error": error.to_string() }));
                    ExitCode::from(1)
                }
            }
        }
        "handshake" => {
            let (host, user, port, domain, password) = match auth_options(&args) {
                Ok(value) => value,
                Err(code) => return code,
            };

            let report = handshake(RdpConnectOptions {
                host: host.clone(),
                port,
                auth: RdpAuth {
                    username: user,
                    password,
                    domain,
                },
                accept_invalid_certs: true,
                ..RdpConnectOptions::default()
            });

            match report {
                Ok(value) => {
                    println!("{}", serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into()));
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "ok": false,
                            "host": host,
                            "port": port,
                            "error": error.to_string(),
                            "stack": "open-panel-rdp (IronRDP)",
                        })
                    );
                    ExitCode::from(1)
                }
            }
        }
        "exec" => {
            let (host, user, port, domain, password) = match auth_options(&args) {
                Ok(value) => value,
                Err(code) => return code,
            };
            let shell = arg_value(&args, "--shell").unwrap_or_else(|| usage()).to_owned();
            let work_dir = arg_value(&args, "--work-dir").unwrap_or("").to_owned();
            let session_timeout_secs = arg_value(&args, "--session-timeout-secs")
                .unwrap_or("180")
                .parse::<u64>()
                .unwrap_or_else(|_| usage());
            let force_fresh_logon = !has_flag(&args, "--no-force-fresh-logon");
            let inject_run_dialog = !has_flag(&args, "--no-inject-run");

            let report = run_alternate_shell(RdpConnectOptions {
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
                logoff_timeout: Duration::from_secs(45),
            });

            match report {
                Ok(value) => {
                    println!("{}", serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into()));
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "ok": false,
                            "host": host,
                            "port": port,
                            "error": error.to_string(),
                            "stack": "open-panel-rdp (IronRDP)",
                        })
                    );
                    ExitCode::from(1)
                }
            }
        }
        _ => usage(),
    }
}
