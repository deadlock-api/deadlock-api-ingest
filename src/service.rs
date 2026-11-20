//! Detecting, starting and stopping the background watcher (for the GUI, installer and
//! updater), plus the process helpers they share.

use std::ffi::OsStr;
use std::fs::{OpenOptions, TryLockError};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::paths;
#[cfg(any(windows, target_os = "linux"))]
use crate::paths::APP_NAME;

/// Whether a long-running watcher holds the instance lock. Covers every way it can be
/// running (systemd unit, scheduled task, `Run` key, launch wrapper, manual start).
pub(crate) fn is_running() -> bool {
    paths::instance_lock_path().is_some_and(|path| lock_held(&path))
}

fn lock_held(path: &Path) -> bool {
    let Ok(file) = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
    else {
        return false;
    };
    // Shared, so concurrent probes (the GUI's refresh and `ensure_started` run in parallel)
    // don't mistake each other for the watcher, whose lock is exclusive. On success the lock
    // is released again when `file` drops at the end of this call.
    matches!(file.try_lock_shared(), Err(TryLockError::WouldBlock))
}

/// Installs the watcher as a per-user autostart service if it isn't one yet and starts it.
pub(crate) fn ensure_started() -> Result<(), String> {
    if is_running() {
        // The running watcher may come from a binary that has moved since: point the agent that
        // starts it at login at one that exists, without starting a second watcher.
        #[cfg(target_os = "macos")]
        if let Ok(exe) = paths::current_exe() {
            macos::repair_stale_agent(&exe);
        }
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
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn stop_other_instances(keep_gui: bool) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};
    use tracing::info;

    let own_pid = sysinfo::get_current_pid().ok();
    let mut sys = crate::game_process::new_system();
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

# No NoNewPrivileges= or PrivateTmp=: the kernel ignores the memory helper's capability
# under the former, and the latter puts a user service in its own user namespace, where
# the capability doesn't count either.

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

#[cfg(target_os = "macos")]
mod macos {
    use std::path::PathBuf;

    use super::{Path, quiet, spawn_detached};
    use tracing::warn;

    /// The label of the per-user launch agent; `install-macos.sh` uses the same one.
    pub(super) const LABEL: &str = "com.deadlock-api.deadlock-api-ingest";

    fn plist_path() -> Option<PathBuf> {
        dirs::home_dir().map(|home| {
            home.join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist"))
        })
    }

    /// Whether the agent's program is gone (the binary was moved or deleted): launchd loads and
    /// kickstarts such an agent fine but never runs anything. One whose program exists is kept,
    /// with its arguments, as install-macos.sh or nix-darwin wrote it.
    fn is_stale(plist_path: &Path) -> bool {
        std::fs::read_to_string(plist_path)
            .ok()
            .and_then(|text| program(&text))
            .is_some_and(|program| !Path::new(&program).exists())
    }

    fn write_agent(plist_path: &Path, exe: &Path) -> Result<(), String> {
        if let Some(dir) = plist_path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
        }
        std::fs::write(plist_path, plist(exe))
            .map_err(|e| format!("Cannot write {}: {e}", plist_path.display()))
    }

    /// With a watcher running: points a stale agent at `exe` on disk only, which launchd reads
    /// at the next login. The running watcher, and launchd's loaded copy, are left alone.
    pub(super) fn repair_stale_agent(exe: &Path) {
        if let Some(plist_path) = plist_path().filter(|path| is_stale(path))
            && let Err(e) = write_agent(&plist_path, exe)
        {
            warn!("{e}");
        }
    }

    pub(super) fn ensure_started(exe: &Path) -> Result<(), String> {
        let domain = format!("gui/{}", rustix::process::getuid().as_raw());
        let service = format!("{domain}/{LABEL}");
        let Some(plist_path) = plist_path() else {
            return spawn_detached(exe, &[]);
        };
        let stale = is_stale(&plist_path);
        if stale {
            // Unload launchd's copy too, so the bootstrap below reads the repaired file.
            let _ = launchctl(&["bootout", &service]);
        }
        if stale || !plist_path.exists() {
            write_agent(&plist_path, exe)?;
        }
        // Loaded already (by install-macos.sh, at login, or an earlier start): start it.
        if launchctl(&["kickstart", &service]).is_ok() {
            return Ok(());
        }
        let plist_arg = plist_path.to_string_lossy();
        match launchctl(&["bootstrap", &domain, &plist_arg]) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Run it for this session at least.
                warn!("{e}");
                spawn_detached(exe, &[])
            }
        }
    }

    fn launchctl(args: &[&str]) -> Result<(), String> {
        let status = quiet("launchctl")
            .args(args)
            .status()
            .map_err(|e| format!("Cannot run launchctl: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("launchctl {} failed: {status}", args.join(" ")))
        }
    }

    /// Same agent as `install-macos.sh` writes: started at login, restarted when it fails
    /// (like the systemd unit's `Restart=on-failure`), but not after a clean exit, e.g.
    /// when another copy already runs.
    pub(super) fn plist(exe: &Path) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ThrottleInterval</key>
    <integer>10</integer>
</dict>
</plist>
"#,
            exe = xml_escape(&exe.to_string_lossy())
        )
    }

    fn xml_escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }

    /// The program an agent's plist starts: `Program`, else `ProgramArguments[0]`.
    pub(super) fn program(plist: &str) -> Option<String> {
        let first_string_after = |key: &str| {
            let rest = &plist[plist.find(&format!("<key>{key}</key>"))?..];
            let start = rest.find("<string>")? + "<string>".len();
            let end = start + rest[start..].find("</string>")?;
            Some(xml_unescape(&rest[start..end]))
        };
        first_string_after("Program").or_else(|| first_string_after("ProgramArguments"))
    }

    fn xml_unescape(text: &str) -> String {
        text.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&")
    }
}

#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod platform {
    use super::{Path, spawn_detached};

    pub(super) fn ensure_started(exe: &Path) -> Result<(), String> {
        spawn_detached(exe, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_concurrent_probes_dont_see_each_other() {
        let path =
            std::env::temp_dir().join(format!("deadlock-probe-test-{}.lock", std::process::id()));
        let false_positives: usize = std::thread::scope(|s| {
            let probes: Vec<_> = (0..8)
                .map(|_| s.spawn(|| (0..500).filter(|_| lock_held(&path)).count()))
                .collect();
            probes.into_iter().map(|p| p.join().unwrap()).sum()
        });
        let _ = std::fs::remove_file(&path);
        assert_eq!(false_positives, 0);
    }

    #[test]
    fn test_probe_sees_the_watchers_lock() {
        let path =
            std::env::temp_dir().join(format!("deadlock-watcher-test-{}.lock", std::process::id()));
        let watcher = std::fs::File::create(&path).unwrap();
        watcher.try_lock().unwrap();
        assert!(lock_held(&path));
        drop(watcher);
        assert!(!lock_held(&path));
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_unit_file_points_at_exe() {
        let unit = platform::unit_file(Path::new("/opt/deadlock-api-ingest"));
        assert!(unit.contains("\nExecStart=/opt/deadlock-api-ingest\n"));
        assert!(unit.contains("\nWantedBy=default.target\n"));
        // Either would keep the memory helper from its capability.
        assert!(!unit.contains("\nNoNewPrivileges="));
        assert!(!unit.contains("\nPrivateTmp="));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_plist_points_at_exe() {
        let plist = platform::plist(Path::new(
            "/Users/a&b/Library/Application Support/deadlock-api-ingest/deadlock-api-ingest",
        ));
        assert!(plist.contains(
            "<string>/Users/a&amp;b/Library/Application Support/deadlock-api-ingest/deadlock-api-ingest</string>"
        ));
        assert!(plist.contains(&format!("<string>{}</string>", platform::LABEL)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_plist_program() {
        let exe = "/Users/a&b/Library/Application Support/deadlock-api-ingest/deadlock-api-ingest";
        assert_eq!(
            platform::program(&platform::plist(Path::new(exe))).as_deref(),
            Some(exe)
        );
        // As nix-darwin writes it, with an argument after the program.
        let nix = "<dict>\n\t<key>Label</key>\n\t<string>x</string>\n\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>/nix/store/abc/bin/deadlock-api-ingest</string>\n\t\t<string>--no-gc</string>\n\t</array>\n</dict>";
        assert_eq!(
            platform::program(nix).as_deref(),
            Some("/nix/store/abc/bin/deadlock-api-ingest")
        );
        let with_program = "<key>Program</key><string>/bin/a</string><key>ProgramArguments</key><array><string>b</string></array>";
        assert_eq!(platform::program(with_program).as_deref(), Some("/bin/a"));
        assert_eq!(platform::program("<dict></dict>"), None);
    }
}
