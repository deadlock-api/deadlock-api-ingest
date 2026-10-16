use crate::ingestion_cache;
use crate::utils::Salts;
use memchr::{memchr, memmem};
use notify::event::{CreateKind, ModifyKind};
use notify::{EventKind, RecursiveMode, Watcher};
use std::fs;
use std::io::Read;
use std::path::Path;
use tracing::{debug, info, warn};

const DEADLOCK_APP_ID: &str = "1422450";
const MAX_BYTES_TO_READ: usize = 200;
const SEARCH_SEQUENCE: &[u8; 10] = b".valve.net";
const PATH_END_MARKERS: [u8; 6] = *b" '\0\n\r\"";
/// Replay paths are short; this bounds the search for their end in large buffers.
const MAX_PATH_LEN: usize = 256;

pub(super) fn scan_directory(dir: &Path, results: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                scan_directory(&path, results);
            } else if path.is_file()
                && let Some(url) = extract_replay_url(&path)
            {
                let file_path = path.display().to_string();
                info!("Found: {file_path} -> {url}");
                results.push(url);
            }
        }
    }
}

fn extract_replay_url(path: &Path) -> Option<String> {
    let Ok(mut file) = fs::File::open(path) else {
        return None;
    };
    let mut data = vec![0u8; MAX_BYTES_TO_READ];
    let bytes_read = file.read(&mut data).ok()?;
    data.truncate(bytes_read);
    replay_urls(&data).next()
}

/// Deadlock replay URLs in `data`, like
/// `http://replay404.valve.net/1422450/37959196_937530290.meta.bz2`.
pub(crate) fn replay_urls(data: &[u8]) -> impl Iterator<Item = String> {
    memmem::find_iter(data, SEARCH_SEQUENCE).filter_map(|i| {
        // Extract Host
        let host_start = data
            .get(..i)?
            .iter()
            .rposition(|b| !b.is_ascii_alphanumeric() && *b != b'.')
            .map_or(0, |pos| pos.saturating_add(1));
        let host_end = i.checked_add(SEARCH_SEQUENCE.len())?;
        let host = core::str::from_utf8(data.get(host_start..host_end)?).ok()?;
        if !host.starts_with("replay") {
            return None;
        }

        // Extract Path
        let after_host = data.get(host_end..)?;
        let after_host = after_host.get(..MAX_PATH_LEN).unwrap_or(after_host);
        let path_slice = after_host.get(memchr(b'/', after_host)?..)?;
        let path_end = PATH_END_MARKERS
            .into_iter()
            .filter_map(|marker| memchr(marker, path_slice))
            .min()?;
        let path = core::str::from_utf8(path_slice.get(..path_end)?).ok()?;
        if !path.contains(DEADLOCK_APP_ID) {
            return None;
        }

        Some(format!("http://{host}{path}"))
    })
}

pub(super) fn initial_cache_dir_ingest(cache_dir: &Path) {
    debug!("Scanning cache directory: {}", cache_dir.display());
    let mut results = Vec::new();
    scan_directory(cache_dir, &mut results);
    let salts = results
        .into_iter()
        .filter_map(|url| Salts::from_url(&url))
        .collect::<Vec<_>>();

    if salts.is_empty() {
        return;
    }

    if let Err(e) = Salts::ingest_many(&salts, ingestion_cache::Source::Cache) {
        warn!("Failed to ingest salts: {e:?}");
    }
}

pub(super) fn watch_cache_dir(cache_dir: &Path) -> notify::Result<()> {
    debug!("Watching cache directory: {}", cache_dir.display());
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(cache_dir, RecursiveMode::Recursive)?;

    while let Ok(Ok(event)) = rx.recv() {
        let is_data_modify = matches!(event.kind, EventKind::Modify(ModifyKind::Data(_)));
        let is_file_create = matches!(
            event.kind,
            EventKind::Create(CreateKind::Any | CreateKind::File)
        );
        if !is_data_modify && !is_file_create {
            continue;
        }
        for path in event.paths {
            if path.is_file()
                && let Some(url) = extract_replay_url(&path)
                && let Some(salts) = Salts::from_url(&url)
            {
                // Check if we've already ingested this salt using the shared cache
                if !ingestion_cache::is_new(&salts) {
                    continue;
                }

                match salts.ingest(ingestion_cache::Source::Cache) {
                    Ok(()) => info!("Ingested salts: {salts:?}"),
                    Err(e) => warn!("Failed to ingest salts: {e:?}"),
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_urls_are_lifted_whole() {
        let mut data =
            b"\0\0http://replay404.valve.net/1422450/37959196_937530290.meta.bz2\0junk".to_vec();
        data.extend(b" replay183.valve.net/1422450/42476710_428480166.dem.bz2\n");
        data.extend(b" cdn.valve.net/1422450/1_2.meta.bz2\0");
        assert_eq!(
            replay_urls(&data).collect::<Vec<_>>(),
            [
                "http://replay404.valve.net/1422450/37959196_937530290.meta.bz2",
                "http://replay183.valve.net/1422450/42476710_428480166.dem.bz2",
            ]
        );
    }
}
