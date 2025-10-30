use crate::error::Error;
use crate::utils::Salts;
use memchr::{memchr, memmem};
use notify::event::CreateKind;
use notify::{EventKind, RecursiveMode, Watcher};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

// cache directory is  ~/.steam/steam/appcache/httpcache/ write it so that it is robost across many linux systems
#[cfg(target_os = "linux")]
pub(super) fn get_cache_directory() -> Option<PathBuf> {
    let home_dir = std::env::var("HOME").ok()?;
    Some(PathBuf::from(format!(
        "{home_dir}/.steam/steam/appcache/httpcache/"
    )))
}

#[cfg(target_os = "windows")]
use winreg::RegKey;
#[cfg(target_os = "windows")]
use winreg::enums::HKEY_CURRENT_USER;

#[cfg(target_os = "windows")]
pub(super) fn get_cache_directory2() -> Option<PathBuf> {
    if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
        let path = PathBuf::from(program_files_x86)
            .join("Steam")
            .join("appcache")
            .join("httpcache");
        if path.exists() && path.is_dir() {
            return Some(path);
        }
    }

    if let Ok(program_files) = std::env::var("ProgramFiles") {
        let path = PathBuf::from(program_files)
            .join("Steam")
            .join("appcache")
            .join("httpcache");
        if path.exists() && path.is_dir() {
            return Some(path);
        }
    }

    let hkey_current_user = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(steam_key) = hkey_current_user.open_subkey("Software\\Valve\\Steam")
        && let Ok(steam_path_str) = steam_key.get_value::<String, _>("SteamPath")
    {
        let corrected_path = PathBuf::from(steam_path_str.replace('/', "\\"));
        let path = corrected_path.join("appcache").join("httpcache");
        if path.exists() && path.is_dir() {
            return Some(path);
        }
    }

    None
}

pub(super) fn scan_directory(dir: &Path, results: &mut Vec<(String, String)>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                scan_directory(&path, results);
            } else if path.is_file()
                && let Some(url) = scan_file(&path)
            {
                let file_path = path.display().to_string();
                println!("Found: {file_path} -> {url}");
                results.push((file_path, url));
            }
        }
    }
}

pub(super) fn scan_file(path: &Path) -> Option<String> {
    if let Ok(mut file) = fs::File::open(path) {
        let mut buffer = Vec::new();
        if file.read_to_end(&mut buffer).is_ok() {
            return extract_replay_url(&buffer);
        }
    }
    None
}

fn extract_replay_url(data: &[u8]) -> Option<String> {
    let finder = memmem::Finder::new(b".valve.net");

    // Find all occurrences of .valve.net
    for i in finder.find_iter(data) {
        // Look backwards to find the start of the host (replayXXX)
        let mut host_start = i;
        while host_start > 0 {
            let c = data[host_start - 1];
            if c.is_ascii_alphanumeric() || c == b'.' {
                host_start -= 1;
            } else {
                break;
            }
        }

        // Extract host
        let host_end = i + b".valve.net".len();
        if let Ok(host) = core::str::from_utf8(&data[host_start..host_end])
            && host.starts_with("replay")
            && host.contains(".valve.net")
        {
            let mut path_start = None;

            if let Some(slash_pos) = memchr(b'/', &data[host_end..data.len().min(host_end + 200)]) {
                path_start = Some(host_end + slash_pos);
            }

            if let Some(start) = path_start {
                // Find the end of the path (null byte, newline, space, quote)
                let search_slice = &data[start..data.len().min(start + 300)];

                let end_markers = [b'\0', b'\n', b'\r', b' ', b'"', b'\''];
                let mut min_end = search_slice.len();

                for &marker in &end_markers {
                    if let Some(pos) = memchr(marker, search_slice) {
                        min_end = min_end.min(pos);
                    }
                }

                if let Ok(path) = core::str::from_utf8(&data[start..start + min_end]) {
                    return Some(format!("http://{host}{path}"));
                }
            }
        }
    }

    None
}

pub(super) fn initial_cache_dir_ingest(cache_dir: &Path) -> Result<(), Error> {
    println!("Scanning cache directory: {}", cache_dir.display());
    let mut results = Vec::new();
    scan_directory(cache_dir, &mut results);
    let salts = results
        .into_iter()
        .filter_map(|(_, url)| Salts::from_url(&url))
        .collect::<Vec<_>>();
    Salts::ingest_many(&salts)
}

pub(super) fn watch_cache_dir(cache_dir: &Path) -> notify::Result<()> {
    println!("Watching cache directory: {}", cache_dir.display());
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(cache_dir, RecursiveMode::Recursive)?;

    while let Ok(event) = rx.recv() {
        let Ok(event) = event else {
            continue;
        };
        if event.kind != EventKind::Create(CreateKind::File) {
            continue;
        }
        for path in event.paths {
            if let Some(url) = scan_file(&path)
                && let Some(salts) = Salts::from_url(&url)
            {
                match salts.ingest() {
                    Ok(..) => println!("Ingested salts: {salts:?}"),
                    Err(e) => eprintln!("Failed to ingest salts: {e:?}"),
                }
            }
        }
    }
    Ok(())
}
