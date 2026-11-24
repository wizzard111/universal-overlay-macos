//! macOS specifics of the updater.

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn no_window(_c: &mut Command) {}

/// The .app bundle we run from, else /Applications/Universal Overlay.app.
pub fn install_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    if let Some(app) = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
    {
        return app.to_path_buf();
    }
    PathBuf::from("/Applications/Universal Overlay.app")
}

pub fn version_file(dir: &Path) -> Option<PathBuf> {
    Some(dir.join("Contents/Resources/VERSION"))
}

/// macOS can replace a running binary: no staging needed.
pub fn restart_staged(
    _work: &Path,
    _repo: &str,
    _install: &Path,
) -> Option<Result<String, String>> {
    None
}

pub fn run_installer(pkg: &Path, install: &Path) -> Result<(), String> {
    let st = Command::new("/bin/bash")
        .arg(pkg.join("install.command"))
        .args(["--update", "--install-dir"])
        .arg(install)
        .status()
        .map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("the installer exited with code {st}"))
    }
}

pub fn pause_if_console() {}
