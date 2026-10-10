//! Per-user locations shared by the watcher, the GUI and the installer.

use std::path::PathBuf;

pub(crate) const APP_NAME: &str = "deadlock-api-ingest";

/// - Linux: `~/.local/share/deadlock-api-ingest/`
/// - macOS: `~/Library/Application Support/deadlock-api-ingest/`
/// - Windows: `C:\Users\<User>\AppData\Roaming\deadlock-api-ingest\`
pub(crate) fn data_dir() -> Option<PathBuf> {
    let dir = dirs::data_dir()?.join(APP_NAME);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// The rolling log files live in `<data dir>/logs/`.
pub(crate) fn log_dir() -> Option<PathBuf> {
    let dir = data_dir()?.join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Every submitted salt, one JSON line each; written by the watcher, read by the GUI.
pub(crate) fn salts_log_path() -> Option<PathBuf> {
    Some(data_dir()?.join("fetched-salts.jsonl"))
}

/// Held exclusively by the long-running watcher; see `acquire_instance_lock`.
pub(crate) fn instance_lock_path() -> Option<PathBuf> {
    Some(data_dir()?.join("instance.lock"))
}

pub(crate) fn current_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("Cannot locate own executable: {e}"))
}
