//! Minimal OpenPanel-hosted static HTTP listener for registered sites.

use std::{
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::Serialize;

use super::store::SiteRecord;
use crate::{Error, Result};

/// Runtime listener state exposed on site views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SiteRuntimeState {
    /// No listener.
    Stopped,
    /// Accept loop is running.
    Running,
}

pub struct RunningSite {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl RunningSite {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for RunningSite {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

pub fn spawn_static_site(record: &SiteRecord) -> Result<RunningSite> {
    let root = PathBuf::from(&record.root_dir);
    if !root.is_dir() {
        return Err(Error::message(format!("site root `{}` is not a directory", root.display())));
    }
    let addr: SocketAddr = format!("{}:{}", record.host, record.port)
        .parse()
        .map_err(|error| Error::message(format!("invalid bind address: {error}")))?;
    let listener = TcpListener::bind(addr)
        .map_err(|error| Error::message(format!("cannot bind `{}:{}`: {error}", record.host, record.port)))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| Error::message(format!("cannot set nonblocking: {error}")))?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let root_clone = root.clone();
    let join = thread::Builder::new()
        .name(format!("open-panel-site-{}", record.id))
        .spawn(move || accept_loop(listener, root_clone, stop_flag))
        .map_err(|error| Error::message(format!("cannot spawn site thread: {error}")))?;

    Ok(RunningSite { stop, join: Some(join) })
}

fn accept_loop(listener: TcpListener, root: PathBuf, stop: Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let root = root.clone();
                let _ = thread::spawn(move || {
                    let _ = handle_client(stream, &root);
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn handle_client(mut stream: TcpStream, root: &Path) -> Result<()> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0u8; 8192];
    let read = stream.read(&mut buf).map_err(|error| Error::message(error.to_string()))?;
    if read == 0 {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&buf[..read]);
    let path = request_path(&request).unwrap_or("/");
    match resolve_file(root, path) {
        Some(file) => {
            let body = fs::read(&file).unwrap_or_default();
            let ctype = content_type(&file);
            write_response(&mut stream, 200, "OK", ctype, &body)?;
        }
        None => {
            write_response(&mut stream, 404, "Not Found", "text/plain; charset=utf-8", b"not found")?;
        }
    }
    Ok(())
}

fn request_path(request: &str) -> Option<&str> {
    let line = request.lines().next()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    if method != "GET" && method != "HEAD" {
        return Some("/");
    }
    parts.next()
}

fn resolve_file(root: &Path, url_path: &str) -> Option<PathBuf> {
    let decoded = url_path.split('?').next().unwrap_or("/");
    let relative = if decoded == "/" { "index.html" } else { decoded.trim_start_matches('/') };
    if relative.contains("..") {
        return None;
    }
    let candidate = root.join(relative);
    let root_canon = root.canonicalize().ok()?;
    let file_canon = candidate.canonicalize().ok()?;
    if !file_canon.starts_with(&root_canon) {
        return None;
    }
    if file_canon.is_dir() {
        let index = file_canon.join("index.html");
        if index.is_file() {
            return Some(index);
        }
        return None;
    }
    if file_canon.is_file() {
        Some(file_canon)
    }
    else {
        None
    }
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn write_response(stream: &mut TcpStream, status: u16, reason: &str, ctype: &str, body: &[u8]) -> Result<()> {
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).map_err(|error| Error::message(error.to_string()))?;
    stream.write_all(body).map_err(|error| Error::message(error.to_string()))?;
    Ok(())
}
