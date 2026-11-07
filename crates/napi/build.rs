//! Embed a build identity into the addon.

use std::{
    env, fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn git_dir() -> Option<PathBuf> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").ok()?;
    Some(PathBuf::from(manifest_dir).join("..").join("..").join(".git"))
}

fn emit_rerun_triggers() {
    let Some(git_dir) = git_dir()
    else {
        return;
    };
    println!("cargo:rerun-if-changed={}", git_dir.join("HEAD").display());
    let head = match fs::read_to_string(git_dir.join("HEAD")) {
        Ok(head) => head,
        Err(_) => return,
    };
    if let Some(reference) = head.trim().strip_prefix("ref: ") {
        println!("cargo:rerun-if-changed={}", git_dir.join(reference).display());
    }
}

fn git_commit() -> String {
    let manifest_dir = match env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => dir,
        Err(_) => return "unknown".to_string(),
    };
    let head = match Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&manifest_dir).output() {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout).trim().to_string(),
        _ => return "unknown".to_string(),
    };
    if head.is_empty() { "unknown".to_string() } else { head }
}

fn main() {
    emit_rerun_triggers();
    let epoch = SystemTime::now().duration_since(UNIX_EPOCH).map(|elapsed| elapsed.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=WAE_APP_BUILD_COMMIT={}", git_commit());
    println!("cargo:rustc-env=WAE_APP_BUILD_EPOCH={epoch}");
}
