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

use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod error;
mod ingestion_cache;
mod scan_cache;
mod statlocker;
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

fn main() {
    init_tracing();

    if let Some(log_dir) = get_log_dir() {
        info!("Log files are being written to: {}", log_dir.display());
    }

    if std::env::args().any(|arg| arg == "--no-statlocker") {
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

    scan_cache::initial_cache_dir_ingest(&cache_dir);

    if std::env::args().any(|arg| arg == "--once") {
        std::process::exit(0);
    }

    loop {
        if let Err(e) = scan_cache::watch_cache_dir(&cache_dir) {
            warn!("Error in cache watcher: {e:?}");
        }
        std::thread::sleep(core::time::Duration::from_secs(10));
    }
}
