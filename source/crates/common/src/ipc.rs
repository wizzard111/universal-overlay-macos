//! Unix IPC: the overlay listens on a per-user socket; `--toggle`, the
//! settings app and the tray send one-word commands ("toggle", "reload").
//! (Windows uses window messages instead.)

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .or_else(|| std::env::var_os("TMPDIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let user = std::env::var("USER").unwrap_or_default();
    dir.join(format!("universal-overlay-{user}.sock"))
}

/// Send a command to a running overlay. Returns false if none is running.
pub fn send(cmd: &str) -> bool {
    match UnixStream::connect(socket_path()) {
        Ok(mut s) => s.write_all(cmd.as_bytes()).is_ok(),
        Err(_) => false,
    }
}

pub fn is_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}
