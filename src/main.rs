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

mod error;
mod ingestion_cache;
mod scan_cache;
mod utils;

fn main() {
    let Ok(steam_dir) = steamlocate::SteamDir::locate() else {
        eprintln!("Could not find Steam directory. Waiting 30s before exiting.");
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
            eprintln!(
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
            eprintln!("Error in cache watcher: {e:?}");
        }
        std::thread::sleep(core::time::Duration::from_secs(10));
    }
}
