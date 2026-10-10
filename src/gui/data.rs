//! What the GUI shows, read from `fetched-salts.jsonl`, which the watcher appends to after
//! every successful submission.

use std::collections::{HashSet, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use serde::Deserialize;

use crate::ingestion_cache::Source;
pub(crate) use crate::utils::now_secs;
use crate::{paths, service};

const RECENT_MATCHES: usize = 50;
pub(crate) const DAY_SECS: i64 = 24 * 60 * 60;

/// One line of `fetched-salts.jsonl`. Older versions wrote lines without time or source.
#[derive(Deserialize)]
struct Entry {
    match_id: u64,
    metadata_salt: Option<u32>,
    replay_salt: Option<u32>,
    #[serde(default)]
    ingested_at: Option<i64>,
    #[serde(default)]
    source: Option<Source>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct MatchRow {
    pub(crate) match_id: u64,
    pub(crate) metadata: bool,
    pub(crate) replay: bool,
    pub(crate) via_gc: bool,
    pub(crate) ingested_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    pub(crate) total: usize,
    pub(crate) last_day: usize,
    pub(crate) last_week: usize,
    pub(crate) via_gc: usize,
    pub(crate) last_ingested_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub(crate) running: bool,
    pub(crate) now: i64,
    pub(crate) stats: Stats,
    /// Newest first.
    pub(crate) recent: Vec<MatchRow>,
}

/// Follows `fetched-salts.jsonl` incrementally: the file grows up to 1 GB, so each poll
/// only reads what was appended since the previous one.
#[derive(Default)]
pub(crate) struct Tracker {
    offset: u64,
    seen: HashSet<u64>,
    /// When each distinct match was first submitted, where known.
    first_seen: Vec<i64>,
    via_gc: usize,
    last_ingested_at: Option<i64>,
    /// Newest first.
    recent: VecDeque<MatchRow>,
}

impl Tracker {
    pub(crate) fn snapshot(&mut self) -> Snapshot {
        if let Some(path) = paths::salts_log_path() {
            self.follow(&path);
        }
        let now = now_secs();
        Snapshot {
            running: service::is_running(),
            now,
            stats: self.stats(now),
            recent: self.recent.iter().cloned().collect(),
        }
    }

    fn stats(&self, now: i64) -> Stats {
        let since = |secs| self.first_seen.iter().filter(|&&t| t >= now - secs).count();
        Stats {
            total: self.seen.len(),
            last_day: since(DAY_SECS),
            last_week: since(7 * DAY_SECS),
            via_gc: self.via_gc,
            last_ingested_at: self.last_ingested_at,
        }
    }

    fn follow(&mut self, path: &Path) {
        let Ok(file) = File::open(path) else {
            return;
        };
        if file.metadata().map_or(0, |m| m.len()) < self.offset {
            // The watcher dropped the oldest half at its size cap; start over.
            *self = Self::default();
        }
        let mut reader = BufReader::new(file);
        if reader.seek(SeekFrom::Start(self.offset)).is_err() {
            return;
        }
        // Line by line, so a large backlog never sits in memory at once.
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                // A partially written last line is picked up on the next poll.
                Ok(0) | Err(_) => break,
                Ok(_) if line.last() != Some(&b'\n') => break,
                Ok(read) => {
                    self.offset += read as u64;
                    if let Ok(entry) = serde_json::from_slice::<Entry>(&line) {
                        self.add(&entry);
                    }
                }
            }
        }
    }

    fn add(&mut self, entry: &Entry) {
        let via_gc = entry.source == Some(Source::Gc);
        if self.seen.insert(entry.match_id) {
            self.first_seen.extend(entry.ingested_at);
            self.via_gc += usize::from(via_gc);
        }
        if entry.ingested_at > self.last_ingested_at {
            self.last_ingested_at = entry.ingested_at;
        }

        // Metadata and replay salts arrive separately; show them as one match.
        let existing = self
            .recent
            .iter()
            .position(|r| r.match_id == entry.match_id)
            .and_then(|pos| self.recent.remove(pos));
        let mut row = existing.unwrap_or_else(|| MatchRow {
            match_id: entry.match_id,
            ..MatchRow::default()
        });
        row.metadata |= entry.metadata_salt.is_some();
        row.replay |= entry.replay_salt.is_some();
        row.via_gc |= via_gc;
        row.ingested_at = entry.ingested_at.or(row.ingested_at);
        self.recent.push_front(row);
        self.recent.truncate(RECENT_MATCHES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_tracker_follows_appends() {
        let path = std::env::temp_dir().join(format!(
            "deadlock-gui-test-{}-salts.jsonl",
            std::process::id()
        ));
        let mut file = File::create(&path).unwrap();
        let now = now_secs();
        // Old format (no time/source), a garbage line, then a half-written line.
        writeln!(
            file,
            r#"{{"match_id":1,"cluster_id":1,"metadata_salt":5,"replay_salt":null}}"#
        )
        .unwrap();
        writeln!(file, "garbage").unwrap();
        write!(
            file,
            r#"{{"match_id":2,"metadata_salt":3,"replay_salt":null,"#
        )
        .unwrap();
        file.flush().unwrap();

        let mut tracker = Tracker::default();
        tracker.follow(&path);
        assert_eq!(tracker.stats(now).total, 1);

        writeln!(file, r#""ingested_at":{now},"source":"cache"}}"#).unwrap();
        writeln!(
            file,
            r#"{{"match_id":2,"metadata_salt":null,"replay_salt":7,"ingested_at":{now},"source":"gc"}}"#
        )
        .unwrap();
        writeln!(
            file,
            r#"{{"match_id":3,"metadata_salt":1,"replay_salt":null,"ingested_at":{},"source":"gc"}}"#,
            now - 3 * DAY_SECS
        )
        .unwrap();
        file.flush().unwrap();
        tracker.follow(&path);

        assert_eq!(
            tracker.stats(now),
            Stats {
                total: 3,
                last_day: 1,
                last_week: 2,
                via_gc: 1,
                last_ingested_at: Some(now),
            }
        );
        // Match 2's two lines are merged into one row; the latest line comes first.
        let ids: Vec<u64> = tracker.recent.iter().map(|r| r.match_id).collect();
        assert_eq!(ids, [3, 2, 1]);
        let two = &tracker.recent[1];
        assert!(two.metadata && two.replay && two.via_gc);

        // Truncation (size cap) starts over.
        File::create(&path).unwrap();
        tracker.follow(&path);
        assert_eq!(tracker.stats(now).total, 0);

        let _ = std::fs::remove_file(&path);
    }
}
