//! Self-installer used by `deadlock-api-ingest-setup.exe` (the release exe under another
//! name) and `--install` / `--uninstall`. Per-user only, so no admin rights are needed.

use std::path::Path;

use crate::console;

/// The release exe renamed to `…-setup….exe` (browsers may append ` (1)`) and started
/// without arguments, i.e. double-clicked after download.
pub(crate) fn launched_as_setup() -> bool {
    std::env::args_os().len() == 1 && std::env::current_exe().is_ok_and(|exe| is_setup_name(&exe))
}

fn is_setup_name(exe: &Path) -> bool {
    exe.file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.to_ascii_lowercase().contains("-setup"))
}

pub(crate) fn run_install() -> i32 {
    match platform::install() {
        Ok(()) => 0,
        Err(e) => {
            console::message_box(&format!("Installation failed:\n\n{e}"), true);
            1
        }
    }
}

pub(crate) fn run_uninstall() -> i32 {
    match platform::uninstall() {
        Ok(()) => {
            console::message_box("Deadlock API Ingest has been uninstalled.", false);
            // Last, so the box can stay open as long as it likes before we exit.
            platform::remove_install_dir_after_exit();
            0
        }
        Err(e) => {
            console::message_box(&format!("Uninstall failed:\n\n{e}"), true);
            1
        }
    }
}

#[cfg(windows)]
mod platform {
    use core::time::Duration;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};

    use tracing::{info, warn};
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    use crate::paths::{self, APP_NAME};
    use crate::service::windows::RUN_KEY;
    use crate::service::{quiet, run_quiet, spawn_detached, stop_other_instances};

    const DISPLAY_NAME: &str = "Deadlock API Ingest";
    const UNINSTALL_KEY: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Uninstall\deadlock-api-ingest";

    /// Same directory as install-windows.ps1, so script installs are upgraded in place.
    fn install_dir() -> Result<PathBuf, String> {
        dirs::data_local_dir()
            .map(|d| d.join(APP_NAME))
            .ok_or_else(|| "Cannot locate %LOCALAPPDATA%".to_owned())
    }

    fn shortcut_paths() -> Vec<PathBuf> {
        let name = format!("{DISPLAY_NAME}.lnk");
        let start_menu =
            dirs::data_dir().map(|d| d.join(r"Microsoft\Windows\Start Menu\Programs").join(&name));
        let desktop = dirs::desktop_dir().map(|d| d.join(&name));
        start_menu.into_iter().chain(desktop).collect()
    }

    fn create_shortcut(lnk: &Path, exe: &Path, dir: &Path) -> Result<(), String> {
        let mut shortcut = mslnk::ShellLink::new(exe).map_err(|e| format!("{e:?}"))?;
        shortcut.set_arguments(Some("--gui".to_owned()));
        shortcut.set_working_dir(Some(dir.display().to_string()));
        shortcut.set_icon_location(Some(exe.display().to_string()));
        shortcut.set_name(Some(
            "Shows what Deadlock API Ingest is uploading".to_owned(),
        ));
        if let Some(parent) = lnk.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        shortcut.create_lnk(lnk).map_err(|e| format!("{e:?}"))
    }

    fn register_uninstall_entry(exe: &Path) -> std::io::Result<()> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY)?;
        let exe = exe.display().to_string();
        key.set_value("DisplayName", &DISPLAY_NAME)?;
        key.set_value("DisplayVersion", &env!("CARGO_PKG_VERSION"))?;
        key.set_value("Publisher", &"Deadlock API")?;
        key.set_value("DisplayIcon", &exe)?;
        key.set_value("UninstallString", &format!("\"{exe}\" --uninstall"))?;
        key.set_value(
            "InstallLocation",
            &Path::new(&exe)
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        )?;
        key.set_value("NoModify", &1u32)?;
        key.set_value("NoRepair", &1u32)
    }

    pub(super) fn install() -> Result<(), String> {
        let me = paths::current_exe()?;
        let dir = install_dir()?;
        let target = dir.join(format!("{APP_NAME}.exe"));
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;

        stop_other_instances(false);
        if !std::fs::canonicalize(&me)
            .and_then(|me| Ok(me == std::fs::canonicalize(&target)?))
            .unwrap_or(false)
        {
            // The killed process may hold the file for a moment longer.
            let mut attempt = 0;
            while let Err(e) = std::fs::copy(&me, &target) {
                attempt += 1;
                if attempt == 10 {
                    return Err(format!("Cannot copy to {}: {e}", target.display()));
                }
                std::thread::sleep(Duration::from_millis(300));
            }
        }
        info!("Installed to {}", target.display());

        for lnk in shortcut_paths() {
            if let Err(e) = create_shortcut(&lnk, &target, &dir) {
                warn!("Cannot create shortcut {}: {e}", lnk.display());
            }
        }
        if let Err(e) = register_uninstall_entry(&target) {
            warn!("Cannot register uninstall entry: {e}");
        }

        // The GUI registers autostart and starts the watcher (see `service::ensure_started`).
        spawn_detached(&target, &["--gui"]).map_err(|e| format!("Installed, but {e}"))
    }

    // Same signature as the non-Windows stub, which does fail.
    #[allow(clippy::unnecessary_wraps)]
    pub(super) fn uninstall() -> Result<(), String> {
        stop_other_instances(false);

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if let Ok((run, _)) = hkcu.create_subkey(RUN_KEY) {
            let _ = run.delete_value(APP_NAME);
        }
        let _ = hkcu.delete_subkey_all(UNINSTALL_KEY);
        // Installed by install-windows.ps1; removing it may need admin, best effort.
        let _ = run_quiet("schtasks", &["/Delete", "/TN", APP_NAME, "/F"]);

        let mut shortcuts = shortcut_paths();
        // install-windows.ps1 names its desktop shortcuts `deadlock-api-ingest*.lnk`.
        if let Some(desktop) = dirs::desktop_dir()
            && let Ok(entries) = std::fs::read_dir(desktop)
        {
            shortcuts.extend(entries.flatten().map(|e| e.path()).filter(|p| {
                p.extension().is_some_and(|e| e == "lnk")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(APP_NAME))
            }));
        }
        for lnk in shortcuts {
            let _ = std::fs::remove_file(lnk);
        }

        Ok(())
    }

    /// A running exe can't delete itself: let cmd keep retrying (up to ~30 s) until we have
    /// exited and the directory is gone. Data (logs, fetched-salts.jsonl) in %APPDATA% is
    /// kept, like uninstall-windows.ps1 does.
    pub(super) fn remove_install_dir_after_exit() {
        let Ok(dir) = install_dir() else { return };
        if !dir.exists() {
            return;
        }
        let dir_arg = dir.display();
        let spawned = quiet("cmd")
            .raw_arg(format!(
                "/c for /l %i in (1,1,30) do @(ping -n 2 127.0.0.1 >nul & rmdir /s /q \"{dir_arg}\" 2>nul & if not exist \"{dir_arg}\" exit)"
            ))
            .current_dir(std::env::temp_dir())
            .spawn();
        if let Err(e) = spawned {
            warn!("Cannot remove {}: {e}", dir.display());
        }
    }
}

#[cfg(not(windows))]
mod platform {
    const UNSUPPORTED: &str = "The built-in installer is Windows-only; on Linux use install-linux.sh (or just run with --gui, which sets up the systemd user service).";

    pub(super) fn install() -> Result<(), String> {
        Err(UNSUPPORTED.to_owned())
    }

    pub(super) fn uninstall() -> Result<(), String> {
        Err(UNSUPPORTED.to_owned())
    }

    pub(super) fn remove_install_dir_after_exit() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_name_detection() {
        for name in [
            "deadlock-api-ingest-setup.exe",
            "Deadlock-API-Ingest-Setup (1).exe",
            "/tmp/deadlock-api-ingest-setup",
        ] {
            assert!(is_setup_name(Path::new(name)), "{name}");
        }
        for name in [
            "deadlock-api-ingest.exe",
            "deadlock-api-ingest-windows-latest.exe",
            "setup.exe",
        ] {
            assert!(!is_setup_name(Path::new(name)), "{name}");
        }
    }
}
