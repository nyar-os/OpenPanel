//! Thin product CLI over `open-panel-rdp` (no FreeRDP / mstsc).
//!
//! Password must come from the environment (`--password-env`), never argv.

use std::env;
use std::process::ExitCode;
use std::time::Duration;

use open_panel_rdp::{RdpAuth, RdpConnectOptions, handshake, probe_tcp};

fn usage() -> ! {
    eprintln!(
        "usage:\n  open-panel-rdp probe --host <host> [--port 3389]\n  open-panel-rdp handshake --host <host> --user <name> --password-env <VAR> [--port 3389] [--domain <name>]"
    );
    std::process::exit(2);
}

fn arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2).find_map(|pair| (pair[0] == name).then_some(pair[1].as_str()))
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
            let host = arg_value(&args, "--host").unwrap_or_else(|| usage());
            let user = arg_value(&args, "--user").unwrap_or_else(|| usage());
            let password_env = arg_value(&args, "--password-env").unwrap_or_else(|| usage());
            let port = arg_value(&args, "--port")
                .unwrap_or("3389")
                .parse::<u16>()
                .unwrap_or_else(|_| usage());
            let domain = arg_value(&args, "--domain").map(str::to_owned);
            let password = match env::var(password_env) {
                Ok(value) if !value.is_empty() => value,
                _ => {
                    println!(
                        "{}",
                        serde_json::json!({
                            "ok": false,
                            "error": format!("environment variable `{password_env}` is empty or unset"),
                        })
                    );
                    return ExitCode::from(2);
                }
            };

            let report = handshake(RdpConnectOptions {
                host: host.to_owned(),
                port,
                auth: RdpAuth {
                    username: user.to_owned(),
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
        _ => usage(),
    }
}
