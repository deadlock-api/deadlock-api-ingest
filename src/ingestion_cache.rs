use crate::paths;
use crate::utils::Salts;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Mutex, OnceLock, RwLock};
use tracing::warn;

/// Size at which the log drops its oldest half (1GB)
const MAX_LOG_SIZE: u64 = 1_073_741_824;

/// Serializes appends and shrinking: the cache watcher and the GC worker both write the log.
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// Global cache to track successfully ingested salts.
/// Key is the `match_id`, value is a tuple of `(has_metadata, has_replay)`.
static INGESTION_CACHE: OnceLock<RwLock<HashMap<u64, (bool, bool)>>> = OnceLock::new();

/// Where a submitted salt came from; recorded in the log for the GUI's stats.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Source {
    /// Steam's HTTP cache (match opened in-game).
    Cache,
    /// Recovered through the Steam Game Coordinator (see `gc`).
    Gc,
}

/// One line of the log file.
#[derive(Serialize)]
struct LogEntry<'a> {
    #[serde(flatten)]
    salts: &'a Salts,
    /// Unix seconds.
    ingested_at: i64,
    source: Source,
}

/// Drops the oldest half of the log, keeping whole lines, so recent history survives the
/// size cap. Readers following the file (the GUI) see it shrink below their read offset
/// and start over.
fn shrink_log(path: &Path, len: u64) -> std::io::Result<()> {
    let mut old = File::open(path)?;
    old.seek(SeekFrom::Start(len / 2))?;
    let mut reader = BufReader::new(old);
    // Skip the line the cut falls into.
    reader.read_until(b'\n', &mut Vec::new())?;

    let tmp = path.with_extension("jsonl.tmp");
    let mut out = BufWriter::new(File::create(&tmp)?);
    std::io::copy(&mut reader, &mut out)?;
    out.flush()?;
    drop(out);
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn append_to_log_file(salt: &Salts, source: Source) {
    let Some(log_path) = paths::salts_log_path() else {
        warn!("Failed to determine log file path");
        return;
    };
    let _guard = LOG_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if let Ok(metadata) = std::fs::metadata(&log_path)
        && metadata.len() >= MAX_LOG_SIZE
        && let Err(e) = shrink_log(&log_path, metadata.len())
    {
        // Still enforce the cap, at the cost of the history.
        warn!("Failed to shrink log file, starting a new one: {e:?}");
        let _ = std::fs::remove_file(&log_path);
    }

    // Serialize the salt to JSON
    let entry = LogEntry {
        salts: salt,
        ingested_at: crate::utils::now_secs(),
        source,
    };
    let json_line = match serde_json::to_string(&entry) {
        Ok(json) => json,
        Err(e) => {
            warn!("Failed to serialize salt to JSON: {e:?}");
            return;
        }
    };

    // Open the file in append mode (create if it doesn't exist)
    let mut file = match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(f) => f,
        Err(e) => {
            warn!("Failed to open log file at {}: {e:?}", log_path.display());
            return;
        }
    };

    // Write the JSON line followed by a newline
    if let Err(e) = writeln!(file, "{json_line}") {
        warn!("Failed to write to log file: {e:?}");
    }
}

/// Mark a salt as successfully ingested.
/// This should only be called after successful ingestion.
pub(crate) fn mark_ingested(salt: &Salts, source: Source) {
    append_to_log_file(salt, source);

    let cache = INGESTION_CACHE.get_or_init(Default::default);
    let mut cache = cache.write().unwrap_or_else(|poisoned| {
        warn!("Failed to lock ingestion cache for writing");
        poisoned.into_inner()
    });

    cache
        .entry(salt.match_id)
        .and_modify(|entry| {
            if salt.metadata_salt.is_some() {
                entry.0 = true;
            }
            if salt.replay_salt.is_some() {
                entry.1 = true;
            }
        })
        .or_insert((salt.metadata_salt.is_some(), salt.replay_salt.is_some()));

    // Prevent unbounded growth - clear cache if it gets too large
    if cache.len() > 10_000 {
        cache.clear();
    }
}

/// Check if a salt has already been ingested.
/// Returns true if the specific salt type (metadata or replay) has been ingested for this `match_id`.
pub(crate) fn is_ingested(match_id: u64, is_metadata: bool) -> bool {
    let cache = INGESTION_CACHE.get_or_init(Default::default);
    let cache = cache.read().unwrap_or_else(|poisoned| {
        warn!("Failed to lock ingestion cache for reading");
        poisoned.into_inner()
    });

    if let Some(entry) = cache.get(&match_id) {
        let (has_metadata, has_replay) = *entry;
        if is_metadata {
            has_metadata
        } else {
            has_replay
        }
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shrink_log_keeps_newest_whole_lines() {
        let path =
            std::env::temp_dir().join(format!("deadlock-shrink-test-{}.jsonl", std::process::id()));
        let content: String = (0..100)
            .flat_map(|i| [format!("{{\"line\":{i}}}"), "\n".to_owned()])
            .collect();
        std::fs::write(&path, &content).unwrap();

        shrink_log(&path, content.len() as u64).unwrap();

        let kept = std::fs::read_to_string(&path).unwrap();
        assert!(kept.len() <= content.len() / 2);
        assert!(content.ends_with(&kept), "keeps a suffix of the log");
        assert!(kept.starts_with("{\"line\":"), "starts on a whole line");
        assert!(kept.ends_with("{\"line\":99}\n"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_cache_operations() {
        let match_id = 12345678;

        // Initially not ingested
        assert!(!is_ingested(match_id, true));
        assert!(!is_ingested(match_id, false));

        // Mark metadata as ingested
        mark_ingested(
            &Salts {
                match_id,
                cluster_id: None,
                metadata_salt: Some(0),
                replay_salt: None,
                username: None,
            },
            Source::Cache,
        );
        assert!(is_ingested(match_id, true));
        assert!(!is_ingested(match_id, false));

        // Mark replay as ingested
        mark_ingested(
            &Salts {
                match_id,
                cluster_id: None,
                metadata_salt: None,
                replay_salt: Some(0),
                username: None,
            },
            Source::Cache,
        );
        assert!(is_ingested(match_id, true));
        assert!(is_ingested(match_id, false));
    }
}
