// Only `console` and `game_memory`'s Windows reader use unsafe (Win32 calls), with a
// local `allow`.
#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![deny(clippy::correctness)]
#![deny(clippy::suspicious)]
#![deny(clippy::style)]
#![deny(clippy::complexity)]
#![deny(clippy::perf)]
#![deny(clippy::pedantic)]
#![deny(clippy::std_instead_of_core)]
#![allow(clippy::unreadable_literal)]

use std::fs::{File, TryLockError};

use clap::Parser;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Deadlock API Ingest — uploads match data from Steam's HTTP cache.
#[derive(Parser)]
#[command(version)]
#[allow(clippy::struct_excessive_bools)] // clap flags
struct Args {
    /// Disable Steam Game Coordinator match-salt recovery
    #[arg(long)]
    no_gc: bool,

    /// Disable reading match salts from Deadlock's memory while it runs
    #[arg(long)]
    no_memory: bool,

    /// Ingest once and exit (no file watching)
    #[arg(long)]
    once: bool,

    /// Recover salts for your own match history via the Steam Game Coordinator, then exit
    #[arg(long, conflicts_with_all = ["once", "no_gc", "command"])]
    own_matches: bool,

    /// Open the status window; installs and starts the background service if it isn't running
    #[arg(long, conflicts_with_all = ["once", "no_gc", "own_matches", "install", "uninstall", "command"])]
    gui: bool,

    /// Install for the current user (Windows): copy to %LOCALAPPDATA%, add shortcuts and an
    /// uninstall entry, then open the status window. Implied when the exe is named `*-setup.exe`
    #[arg(long, conflicts_with_all = ["once", "no_gc", "own_matches", "uninstall", "command"])]
    install: bool,

    /// Remove what --install set up (Windows); keeps logs and fetched-salts.jsonl
    #[arg(long, conflicts_with_all = ["once", "no_gc", "own_matches", "command"])]
    uninstall: bool,

    /// Game command to wrap (launch wrapper mode).
    /// When provided, the watcher runs in the background while the game
    /// runs as a child process, and exits when the game exits.
    /// Usage: deadlock-api-ingest -- %command%
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

mod console;
mod error;
mod game_memory;
mod game_names;
mod game_process;
mod gc;
mod gui;
mod ingestion_cache;
mod install;
mod paths;
mod scan_cache;
mod service;
mod steam_user;
mod update;
mod utils;

/// `log_to_file` is off for the GUI and installer: the daily log files belong to the
/// watcher, which is what people look at when debugging uploads.
fn init_tracing(log_to_file: bool) {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        // The GUI's own dependencies (winit, cosmic-text, …) are very chatty at debug.
        EnvFilter::new(if log_to_file {
            "debug,reqwest=warn,rustls=warn"
        } else {
            "info"
        })
    });
    let stdout_layer = tracing_subscriber::fmt::layer();

    let file_layer = log_to_file
        .then(paths::log_dir)
        .flatten()
        .and_then(|log_dir| {
            tracing_appender::rolling::RollingFileAppender::builder()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix("deadlock-api-ingest")
                .filename_suffix("log")
                .max_log_files(7)
                .build(&log_dir)
                .ok()
                .map(|appender| {
                    tracing_subscriber::fmt::layer()
                        .with_writer(appender)
                        .with_ansi(false)
                })
        });

    tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .with(env_filter)
        .init();
}

/// Takes an exclusive lock so only one long-running watcher exists per user. Without it,
/// every `Start-ScheduledTask` (or a manual launch next to the autostart) adds another
/// copy, and the copies fight over the GC quota store. Exits if another instance holds it;
/// returns `None` (and runs unlocked) if the lock file itself can't be set up.
fn acquire_instance_lock() -> Option<File> {
    let lock = paths::instance_lock_path()
        .map(File::create)?
        .inspect_err(|e| warn!("Cannot create instance lock, running without it: {e}"))
        .ok()?;
    // The GUI briefly takes the lock to check whether a watcher is running; don't mistake
    // such a probe for another instance.
    for attempt in 1..=5 {
        match lock.try_lock() {
            Ok(()) => return Some(lock),
            Err(TryLockError::WouldBlock) if attempt < 5 => {
                std::thread::sleep(core::time::Duration::from_millis(200));
            }
            Err(TryLockError::WouldBlock) => break,
            Err(TryLockError::Error(e)) => {
                warn!("Cannot acquire instance lock, running without it: {e}");
                return None;
            }
        }
    }
    info!("Another deadlock-api-ingest instance is already running, exiting.");
    std::process::exit(0);
}

fn run_launch_wrapper<F: FnOnce() + Send + 'static>(background_work: F, command: &[String]) -> i32 {
    // Wine double-forks the game, which then belongs to the nearest subreaper. Being one keeps
    // the game our descendant, which reading its memory needs under Yama's default ptrace_scope.
    #[cfg(target_os = "linux")]
    if let Err(e) = rustix::process::set_child_subreaper(Some(rustix::process::getpid())) {
        warn!("Cannot become the game's subreaper: {e}");
    }
    std::thread::spawn(background_work);
    info!("Launching game: {}", command.join(" "));
    match std::process::Command::new(&command[0])
        .args(&command[1..])
        .status()
    {
        Ok(s) => {
            info!("Game exited with status: {s}");
            s.code().unwrap_or(0)
        }
        Err(e) => {
            error!("Failed to launch game command '{}': {e}", command[0]);
            1
        }
    }
}

fn main() {
    let args = Args::parse();
    let setup = args.install || install::launched_as_setup();
    let windowed = args.gui || setup || args.uninstall;
    init_tracing(!windowed);

    if windowed {
        console::hide_own_console();
        let exit_code = if args.gui {
            gui::run()
        } else if setup {
            install::run_install()
        } else {
            install::run_uninstall()
        };
        std::process::exit(exit_code);
    }

    if let Some(log_dir) = paths::log_dir() {
        info!("Log files are being written to: {}", log_dir.display());
    }

    if args.own_matches {
        std::process::exit(i32::from(!gc::run_own_matches_blocking()));
    }

    // Only the long-running watcher is single-instance: `--once` is a short manual run, and
    // the launch wrapper must always start the game even if the autostart copy is running.
    let watcher = args.command.is_empty() && !args.once;
    let _instance_lock = watcher.then(acquire_instance_lock).flatten();
    if watcher {
        // Started by autostart or a shortcut, the watcher shouldn't leave a console open.
        console::hide_own_console();
    }

    let Ok(steam_dir) = steamlocate::SteamDir::locate() else {
        error!("Could not find Steam directory. Waiting 30s before exiting.");
        std::thread::sleep(core::time::Duration::from_secs(30));
        std::process::exit(1);
    };
    let steam_path = steam_dir.path();
    let mut cache_dir = steam_path.join("appcache").join("httpcache");

    if !cache_dir.exists() {
        let home_dir = dirs::home_dir().unwrap_or_default();
        let appcache_search = std::ffi::OsStr::new("appcache");
        for entry in walkdir::WalkDir::new(&home_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_dir())
        {
            if entry.file_name() == "httpcache"
                && entry
                    .path()
                    .parent()
                    .and_then(|p| p.file_name())
                    .is_some_and(|p| p == appcache_search)
            {
                cache_dir = entry.path().to_path_buf();
                break;
            }
        }
        if !cache_dir.exists() {
            error!(
                "Could not find Steam cache directory at {}. Waiting 30s before exiting.",
                cache_dir.display()
            );
            std::thread::sleep(core::time::Duration::from_secs(30));
            std::process::exit(1);
        }
    }

    // In launch-wrapper mode on Linux this runs in the game's ancestor, which Yama's
    // default `ptrace_scope` needs for reading its memory.
    if !args.no_memory && !args.once {
        game_memory::spawn_background();
    }

    if !args.command.is_empty() {
        let exit_code = run_launch_wrapper(
            move || {
                scan_cache::initial_cache_dir_ingest(&cache_dir);
                loop {
                    if let Err(e) = scan_cache::watch_cache_dir(&cache_dir) {
                        warn!("Error in cache watcher: {e:?}");
                    }
                    std::thread::sleep(core::time::Duration::from_secs(10));
                }
            },
            &args.command,
        );
        std::process::exit(exit_code);
    }

    scan_cache::initial_cache_dir_ingest(&cache_dir);

    if args.once {
        if !args.no_gc {
            gc::run_pass_blocking();
        }
        if !args.no_memory {
            game_memory::run_pass_blocking();
        }
        std::process::exit(0);
    }

    if !args.no_gc {
        gc::spawn_background();
    }

    loop {
        if let Err(e) = scan_cache::watch_cache_dir(&cache_dir) {
            warn!("Error in cache watcher: {e:?}");
        }
        std::thread::sleep(core::time::Duration::from_secs(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_memory_goes_with_every_mode() {
        for mode in [
            "--own-matches",
            "--once",
            "--gui",
            "--install",
            "--uninstall",
        ] {
            assert!(
                Args::try_parse_from(["deadlock-api-ingest", "--no-memory", mode]).is_ok(),
                "--no-memory {mode}"
            );
        }
    }
    use std::time::Instant;

    #[test]
    fn test_launch_wrapper_starts_command_promptly() {
        let marker =
            std::env::temp_dir().join(format!("deadlock-launch-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);

        let command = vec!["touch".to_string(), marker.to_str().unwrap().to_string()];

        let start = Instant::now();
        let exit_code = run_launch_wrapper(
            || std::thread::sleep(core::time::Duration::from_secs(30)),
            &command,
        );
        let elapsed = start.elapsed();

        assert_eq!(exit_code, 0, "wrapped command should exit successfully");
        assert!(
            marker.exists(),
            "wrapped command should have created marker file"
        );
        assert!(
            elapsed < core::time::Duration::from_secs(5),
            "Wrapped command took {:.1}s to complete (limit 5s). \
             The background work is blocking game launch.",
            elapsed.as_secs_f64()
        );

        let _ = std::fs::remove_file(&marker);
    }
}
