#![forbid(unsafe_code)]
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

use std::path::PathBuf;

use clap::Parser;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Deadlock API Ingest — uploads match data from Steam's HTTP cache.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Disable statlocker integration
    #[arg(long)]
    no_statlocker: bool,

    /// Ingest once and exit (no file watching)
    #[arg(long)]
    once: bool,

    /// Game command to wrap (launch wrapper mode).
    /// When provided, the watcher runs in the background while the game
    /// runs as a child process, and exits when the game exits.
    /// Usage: deadlock-api-ingest -- %command%
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    command: Vec<String>,
}

mod error;
mod ingestion_cache;
mod scan_cache;
mod statlocker;
mod steam_user;
mod utils;

/// Returns the directory for log files.
/// - Linux: `~/.local/share/deadlock-api-ingest/logs/`
/// - macOS: `~/Library/Application Support/deadlock-api-ingest/logs/`
/// - Windows: `C:\Users\<User>\AppData\Roaming\deadlock-api-ingest\logs\`
fn get_log_dir() -> Option<PathBuf> {
    let log_dir = dirs::data_dir()?.join("deadlock-api-ingest").join("logs");
    std::fs::create_dir_all(&log_dir).ok()?;
    Some(log_dir)
}

fn init_tracing() {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or(EnvFilter::new("debug,reqwest=warn,rustls=warn"));
    let stdout_layer = tracing_subscriber::fmt::layer();

    let file_layer = get_log_dir().and_then(|log_dir| {
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

fn run_launch_wrapper<F: FnOnce() + Send + 'static>(background_work: F, command: &[String]) -> i32 {
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
    init_tracing();

    let args = Args::parse();

    if let Some(log_dir) = get_log_dir() {
        info!("Log files are being written to: {}", log_dir.display());
    }

    if args.no_statlocker {
        statlocker::disable();
    }

    let Ok(steam_dir) = steamlocate::SteamDir::locate() else {
        error!("Could not find Steam directory. Waiting 30s before exiting.");
        std::thread::sleep(core::time::Duration::from_secs(30));
        return;
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
            return;
        }
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
        std::process::exit(0);
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
    use std::time::Instant;

    #[test]
    fn test_launch_wrapper_starts_command_promptly() {
        let start = Instant::now();
        let exit_code = run_launch_wrapper(
            || std::thread::sleep(core::time::Duration::from_secs(30)),
            &["cargo".to_string(), "--version".to_string()],
        );
        let elapsed = start.elapsed();

        assert_eq!(exit_code, 0, "wrapped command should exit successfully");
        assert!(
            elapsed < core::time::Duration::from_secs(5),
            "Wrapped command took {:.1}s to complete (limit 5s). \
             The background work is blocking game launch.",
            elapsed.as_secs_f64()
        );
    }
}
