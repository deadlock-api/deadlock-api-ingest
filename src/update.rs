//! Self-update from the GitHub releases, triggered from the GUI.

use core::time::Duration;
use std::io::Write;
use std::path::Path;

use serde::Deserialize;
use tracing::info;

use crate::paths;
use crate::utils::http_client;

const LATEST_RELEASE: &str =
    "https://api.github.com/repos/deadlock-api/deadlock-api-ingest/releases/latest";
const USER_AGENT: &str = "deadlock-api-ingest";
const MAX_DOWNLOAD_BYTES: u64 = 256 * 1024 * 1024;
/// The shared client's 30 s limit is for API calls; a ~30 MB download needs longer.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(10);

/// The release asset that replaces this build.
#[cfg(windows)]
const ASSET_NAME: &str = "deadlock-api-ingest-setup.exe";
#[cfg(target_os = "linux")]
const ASSET_NAME: &str = "deadlock-api-ingest-ubuntu-latest";
#[cfg(not(any(windows, target_os = "linux")))]
const ASSET_NAME: &str = "";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Update {
    pub(crate) version: String,
    download_url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// `"v1.2.3"` / `"1.2.3-abc"` -> `(1, 2, 3)`.
fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim_start_matches('v').split(['-', '+']).next()?;
    let mut parts = core.split('.').map(str::parse);
    Some((
        parts.next()?.ok()?,
        parts.next()?.ok()?,
        parts.next()?.ok()?,
    ))
}

/// Whether this build can replace itself. On Linux that means the exe's directory is
/// writable, which rules out package-manager installs (Nix store, distro packages).
fn supported() -> bool {
    if cfg!(windows) {
        return true;
    }
    if !cfg!(target_os = "linux") {
        return false;
    }
    let Some(dir) = paths::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return false;
    };
    let probe = dir.join(format!(".{}.update-probe", paths::APP_NAME));
    let writable = std::fs::File::create(&probe).is_ok();
    let _ = std::fs::remove_file(&probe);
    writable
}

/// The newest release, if it is newer than this build and has an asset for this platform.
pub(crate) fn check() -> Result<Option<Update>, String> {
    if !supported() {
        return Ok(None);
    }
    let release: Release = http_client()
        .get(LATEST_RELEASE)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .and_then(|mut r| r.body_mut().read_json())
        .map_err(|e| format!("Couldn't check for updates: {e}"))?;

    let (Some(latest), Some(current)) = (
        parse_version(&release.tag_name),
        parse_version(env!("CARGO_PKG_VERSION")),
    ) else {
        return Ok(None);
    };
    if latest <= current {
        return Ok(None);
    }
    Ok(release
        .assets
        .into_iter()
        .find(|a| a.name == ASSET_NAME)
        .map(|a| Update {
            version: release.tag_name.trim_start_matches('v').to_owned(),
            download_url: a.browser_download_url,
        }))
}

fn download(url: &str, to: &Path) -> Result<(), String> {
    info!("Downloading update from {url}");
    let mut response = http_client()
        .get(url)
        .header("User-Agent", USER_AGENT)
        .config()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .build()
        .call()
        .map_err(|e| format!("Download failed: {e}"))?;
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD_BYTES)
        .reader();
    let mut file =
        std::fs::File::create(to).map_err(|e| format!("Cannot write {}: {e}", to.display()))?;
    let written =
        std::io::copy(&mut reader, &mut file).map_err(|e| format!("Download failed: {e}"))?;
    file.flush().map_err(|e| e.to_string())?;
    // Guard against an HTML error page or a truncated file standing in for the binary.
    if written < 1024 * 1024 {
        let _ = std::fs::remove_file(to);
        return Err("Download failed: the file is incomplete".to_owned());
    }
    Ok(())
}

/// Downloads and starts the new version. On success the caller should exit: the new version
/// takes over (Windows: its installer replaces this exe and reopens the window).
pub(crate) fn apply(update: &Update) -> Result<(), String> {
    platform::apply(update)
}

#[cfg(windows)]
mod platform {
    use super::{ASSET_NAME, Update, download};
    use crate::service::spawn_detached;

    pub(super) fn apply(update: &Update) -> Result<(), String> {
        // The installer stops this GUI and the watcher, replaces the installed exe and
        // reopens the window (see `install`).
        let setup = std::env::temp_dir().join(ASSET_NAME);
        download(&update.download_url, &setup)?;
        spawn_detached(&setup, &["--install"])
            .map_err(|e| format!("Cannot start the installer: {e}"))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{Update, download};
    use crate::paths::{self, APP_NAME};
    use crate::service::{self, linux::systemctl, spawn_detached, stop_other_instances};
    use std::os::unix::fs::PermissionsExt;

    pub(super) fn apply(update: &Update) -> Result<(), String> {
        let exe = paths::current_exe()?;
        // Download next to the exe so the final rename is atomic; a running binary can be
        // replaced this way, it keeps executing the old inode.
        let staged = exe.with_extension("new");
        download(&update.download_url, &staged)?;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
            .and_then(|()| std::fs::rename(&staged, &exe))
            .map_err(|e| {
                let _ = std::fs::remove_file(&staged);
                format!("Cannot replace {}: {e}", exe.display())
            })?;

        // Under systemd the unit restarts onto the new binary; otherwise stop the old
        // watcher and the new GUI starts the new one.
        if systemctl(&["try-restart", APP_NAME]).is_err() || !service::is_running() {
            stop_other_instances(true);
        }
        spawn_detached(&exe, &["--gui"])
            .map_err(|e| format!("Updated, but couldn't reopen the window: {e}"))
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    use super::Update;

    pub(super) fn apply(_update: &Update) -> Result<(), String> {
        Err("Updating isn't supported on this platform".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_version() {
        assert_eq!(parse_version("v0.3.4"), Some((0, 3, 4)));
        assert_eq!(parse_version("0.10.0"), Some((0, 10, 0)));
        assert_eq!(parse_version("v0.3.4-1a2b3c4"), Some((0, 3, 4)));
        assert_eq!(parse_version("nightly"), None);
        assert!(parse_version("v0.10.0") > parse_version("v0.9.9"));
    }
}
