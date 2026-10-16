//! Detecting, starting and stopping the background watcher (for the GUI, installer and
//! updater), plus the process helpers they share.

use std::ffi::OsStr;
use std::fs::{OpenOptions, TryLockError};
use std::path::Path;
use std::process::{Command, Stdio};

use tracing::info;

use crate::paths::{self, APP_NAME};

/// Whether a long-running watcher holds the instance lock. Covers every way it can be
/// running (systemd unit, scheduled task, `Run` key, launch wrapper, manual start).
pub(crate) fn is_running() -> bool {
    let Some(path) = paths::instance_lock_path() else {
        return false;
    };
    let Ok(file) = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
    else {
        return false;
    };
    // On success the lock is released again when `file` drops at the end of this call.
    matches!(file.try_lock(), Err(TryLockError::WouldBlock))
}

/// Installs the watcher as a per-user autostart service if it isn't one yet and starts it.
pub(crate) fn ensure_started() -> Result<(), String> {
    if is_running() {
        return Ok(());
    }
    platform::ensure_started(&paths::current_exe()?)
}

/// A command with all stdio detached and, on Windows, no console window.
pub(crate) fn quiet(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(windows::CREATE_NO_WINDOW);
    }
    cmd
}

/// Runs a tool to completion; returns whether it succeeded.
#[cfg(windows)]
pub(crate) fn run_quiet(program: &str, args: &[&str]) -> bool {
    quiet(program)
        .args(args)
        .status()
        .is_ok_and(|s| s.success())
}

/// Starts `exe args` so that it outlives this process.
pub(crate) fn spawn_detached(exe: &Path, args: &[&str]) -> Result<(), String> {
    let mut cmd = quiet(exe);
    cmd.args(args);
    #[cfg(unix)]
    {
        // Own process group, so a Ctrl+C in the terminal that started the GUI doesn't reach it.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(windows::DETACHED_PROCESS);
    }
    cmd.spawn()
        .map(drop)
        .map_err(|e| format!("Failed to start {}: {e}", exe.display()))
}

/// Kills every other running copy of this program (watcher and GUIs, or only the watchers
/// with `keep_gui`), so its exe can be replaced or removed.
pub(crate) fn stop_other_instances(keep_gui: bool) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};

    let own_pid = sysinfo::get_current_pid().ok();
    let mut sys = sysinfo::System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cmd(UpdateKind::OnlyIfNotSet),
    );
    let is_app = |name: &OsStr| {
        Path::new(name)
            .file_stem()
            .is_some_and(|stem| stem.eq_ignore_ascii_case(APP_NAME))
    };
    for process in sys.processes().values() {
        // The name alone isn't enough: Linux truncates it to 15 characters. argv[0] isn't
        // either: reading another process's command line can fail (Wine, other sessions).
        let cmd = process.cmd();
        let is_ours = is_app(process.name()) || cmd.first().is_some_and(|arg0| is_app(arg0));
        let is_gui = cmd.iter().any(|arg| arg == "--gui");
        if is_ours && Some(process.pid()) != own_pid && !(keep_gui && is_gui) {
            info!("Stopping running instance (pid {})", process.pid());
            process.kill();
            process.wait();
        }
    }
}

#[cfg(target_os = "linux")]
pub(crate) mod linux {
    use super::{APP_NAME, Path, quiet, spawn_detached};

    pub(super) fn ensure_started(exe: &Path) -> Result<(), String> {
        let unit_path =
            dirs::config_dir().map(|d| d.join("systemd/user").join(format!("{APP_NAME}.service")));
        match unit_path {
            Some(unit_path) if unit_path.exists() => {
                if systemctl(&["start", APP_NAME]).is_ok() {
                    return Ok(());
                }
            }
            Some(unit_path) if systemctl(&["--version"]).is_ok() => {
                if let Some(dir) = unit_path.parent() {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
                }
                std::fs::write(&unit_path, unit_file(exe))
                    .map_err(|e| format!("Cannot write {}: {e}", unit_path.display()))?;
                systemctl(&["daemon-reload"])?;
                return systemctl(&["enable", "--now", APP_NAME]);
            }
            _ => {}
        }
        // No (user) systemd: run it for this session at least.
        spawn_detached(exe, &[])
    }

    pub(crate) fn systemctl(args: &[&str]) -> Result<(), String> {
        let status = quiet("systemctl")
            .arg("--user")
            .args(args)
            .status()
            .map_err(|e| format!("Cannot run systemctl: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "systemctl --user {} failed: {status}",
                args.join(" ")
            ))
        }
    }

    /// Same unit as `install-linux.sh` writes.
    pub(super) fn unit_file(exe: &Path) -> String {
        format!(
            "[Unit]
Description=Deadlock API Ingest - Monitors Steam cache for match replays
Documentation=https://github.com/deadlock-api/deadlock-api-ingest

[Service]
Type=simple
ExecStart={exe}
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal
SyslogIdentifier={APP_NAME}

# Security Hardening (no NoNewPrivileges: it would keep the memory helper from its capability)
PrivateTmp=true

[Install]
WantedBy=default.target
",
            exe = exe.display()
        )
    }
}

#[cfg(target_os = "linux")]
use linux as platform;

#[cfg(windows)]
pub(crate) mod windows {
    use super::{APP_NAME, Path, is_running, run_quiet, spawn_detached};
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    pub(crate) const DETACHED_PROCESS: u32 = 0x0000_0008;
    pub(crate) const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    pub(super) fn ensure_started(exe: &Path) -> Result<(), String> {
        // Installed by install-windows.ps1: let the scheduler own the process. A task can
        // exist yet not start (stale path, policy), so only trust it once the watcher runs.
        if run_quiet("schtasks", &["/Query", "/TN", APP_NAME])
            && run_quiet("schtasks", &["/Run", "/TN", APP_NAME])
            && wait_until_running()
        {
            return Ok(());
        }

        // Per-user autostart; unlike a scheduled task this needs no admin rights.
        RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(RUN_KEY)
            .and_then(|(key, _)| key.set_value(APP_NAME, &format!("\"{}\"", exe.display())))
            .map_err(|e| format!("Cannot register autostart: {e}"))?;
        spawn_detached(exe, &[])
    }

    /// Gives a just-started watcher a few seconds to take the instance lock.
    fn wait_until_running() -> bool {
        (0..20).any(|_| {
            std::thread::sleep(core::time::Duration::from_millis(250));
            is_running()
        })
    }
}

#[cfg(windows)]
use windows as platform;

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::{Path, spawn_detached};

    pub(super) fn ensure_started(exe: &Path) -> Result<(), String> {
        spawn_detached(exe, &[])
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn test_unit_file_points_at_exe() {
        let unit = platform::unit_file(Path::new("/opt/deadlock-api-ingest"));
        assert!(unit.contains("\nExecStart=/opt/deadlock-api-ingest\n"));
        assert!(unit.contains("\nWantedBy=default.target\n"));
    }
}
