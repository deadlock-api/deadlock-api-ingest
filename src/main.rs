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
    let once_mode = std::env::args().any(|arg| arg == "--once");

    let cache_dir = scan_cache::get_cache_directory();
    if let Some(cache_dir) = cache_dir {
        if once_mode {
            println!("Running in once mode: performing initial cache ingest only");
            scan_cache::initial_cache_dir_ingest(&cache_dir);
            println!("Initial cache ingest completed. Exiting.");
        }

        scan_cache::initial_cache_dir_ingest(&cache_dir);
        loop {
            if let Err(e) = scan_cache::watch_cache_dir(&cache_dir) {
                eprintln!("Error in cache watcher: {e:?}");
            }
            std::thread::sleep(core::time::Duration::from_secs(10));
        }
    } else {
        eprintln!("Could not find cache directory. Exiting.");
    }
}
