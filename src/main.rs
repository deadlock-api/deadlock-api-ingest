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

use crate::http_listener::{HttpListener, PlatformListener};

mod error;
mod http_listener;
mod ingestion_cache;
mod scan_cache;
mod utils;

fn main() {
    // Check for --once flag
    let once_mode = std::env::args().any(|arg| arg == "--once");

    if once_mode {
        println!("Running in once mode: performing initial cache ingest only");
        let cache_dir = scan_cache::get_cache_directory();
        if let Some(cache_dir) = cache_dir {
            scan_cache::initial_cache_dir_ingest(&cache_dir);
            println!("Initial cache ingest completed. Exiting.");
        } else {
            println!("Could not find cache directory. Exiting.");
        }
        return;
    }

    let mut handles = Vec::new();

    handles.push(std::thread::spawn(|| {
        loop {
            if let Err(e) = PlatformListener.listen() {
                println!("Error in HTTP listener: {e:?}");
            }
            std::thread::sleep(core::time::Duration::from_secs(10));
        }
    }));

    let cache_dir = scan_cache::get_cache_directory();
    if let Some(cache_dir) = cache_dir {
        handles.push(std::thread::spawn(move || {
            scan_cache::initial_cache_dir_ingest(&cache_dir);
            loop {
                if let Err(e) = scan_cache::watch_cache_dir(&cache_dir) {
                    println!("Error in cache watcher: {e:?}");
                }
                std::thread::sleep(core::time::Duration::from_secs(10));
            }
        }));
    }

    #[allow(clippy::never_loop)]
    for handle in handles {
        handle.join().unwrap();
    }
}
