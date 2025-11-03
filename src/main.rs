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
    let cache_dir = steam_path.join("appcache").join("httpcache");

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
