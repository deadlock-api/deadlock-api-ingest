//! Match salts read from Deadlock's memory while it runs.
//!
//! After the client asks the Game Coordinator for a match's metadata, Valve's
//! `GetMatchMetaData` reply stays on the game's heap. It names no match, but its
//! `replay_valid_through` is the match's start plus three weeks, so the history record in
//! memory that started then identifies it. Replay URLs left in memory are taken too.
//!
//! The game is only ever read.

#![deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::unwrap_used,
    clippy::undocumented_unsafe_blocks
)]

mod scan;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux::GameProcess;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::GameProcess;

#[cfg(not(any(windows, target_os = "linux")))]
use unsupported::GameProcess;
#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported {
    pub(super) enum GameProcess {}

    impl GameProcess {
        pub(super) fn open(_pid: u32) -> std::io::Result<Self> {
            Err(std::io::ErrorKind::Unsupported.into())
        }
        pub(super) fn regions(&self) -> Vec<super::Region> {
            match *self {}
        }
        pub(super) fn read(&self, _addr: usize, _buf: &mut [u8]) -> usize {
            match *self {}
        }
    }
}

use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::fs::{File, TryLockError};
use std::io::{self, ErrorKind};
use std::thread::sleep;
use std::time::Instant;

use sysinfo::System;
use tracing::{debug, info, warn};
use valveprotos::deadlock::CMsgClientToGcGetMatchMetaDataResponse;

use crate::ingestion_cache::{self, Source};
use crate::scan_cache::replay_urls;
use crate::utils::{Salts, now_secs};
use crate::{game_process, paths, steam_user};

const PASS_EVERY: Duration = Duration::from_mins(5);
/// A slow pass waits this many times its own length before the next one, so the game is
/// read at most a fifth of the time.
const PASS_REST_RATIO: u32 = 4;
const GAME_CHECK_EVERY: Duration = Duration::from_secs(30);
/// Larger private regions are texture and mesh data.
const MAX_REGION: usize = 1 << 30;
/// Chunks overlap by more than two history records, so nothing is split between them.
const CHUNK: usize = 4 << 20;
const OVERLAP: usize = 512;
const STRIDE: usize = CHUNK - OVERLAP;
/// A pass pauses after every so many bytes, so it never takes a whole core from the game.
const THROTTLE_EVERY: usize = 64 << 20;
const THROTTLE_PAUSE: Duration = Duration::from_millis(40);
const REPLAY_VALID_FOR: u32 = 21 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Region {
    base: usize,
    size: usize,
}

/// Reads the game whenever it runs.
pub(crate) fn spawn_background() {
    let spawned = std::thread::Builder::new()
        .name("game-memory".into())
        .spawn(|| {
            let mut lock = None;
            let mut warned = false;
            loop {
                let rest = match find_game() {
                    Some(pid) if may_read(&mut lock) => match pass(pid) {
                        Ok(read_for) => PASS_EVERY.max(read_for.saturating_mul(PASS_REST_RATIO)),
                        Err(e) => {
                            // Leave the game to a process that can read it (a launch wrapper).
                            lock = None;
                            log_open_error(&e, &mut warned);
                            GAME_CHECK_EVERY
                        }
                    },
                    _ => GAME_CHECK_EVERY,
                };
                sleep(rest);
            }
        });
    if let Err(e) = spawned {
        warn!("memory: failed to spawn worker: {e}");
    }
}

/// One pass if the game is running.
pub(crate) fn run_pass_blocking() {
    if let Some(pid) = find_game()
        && let Err(e) = pass(pid)
    {
        log_open_error(&e, &mut false);
    }
}

fn find_game() -> Option<u32> {
    game_process::find_pid(&mut System::new())
}

/// Whether this process may read the game. Only one per user does, as the watcher and a
/// launch wrapper can run side by side; without a lock file, every process may.
fn may_read(lock: &mut Option<File>) -> bool {
    if lock.is_some() {
        return true;
    }
    let Some(file) = paths::game_memory_lock_path().and_then(|path| File::create(path).ok()) else {
        return true;
    };
    match file.try_lock() {
        Ok(()) => {
            *lock = Some(file);
            true
        }
        Err(TryLockError::WouldBlock) => false,
        Err(TryLockError::Error(_)) => true,
    }
}

fn log_open_error(e: &io::Error, warned: &mut bool) {
    if e.kind() != ErrorKind::PermissionDenied {
        debug!("memory: cannot open Deadlock: {e}");
    } else if !core::mem::replace(warned, true) {
        let hint = if cfg!(target_os = "linux") {
            "; start the game through the launch wrapper (`deadlock-api-ingest -- %command%`)"
        } else {
            ""
        };
        warn!("memory: not allowed to read Deadlock's memory: {e}{hint}");
    }
}

/// Reads the game once and ingests the new salts; returns how long the read took.
fn pass(pid: u32) -> io::Result<Duration> {
    let game = GameProcess::open(pid)?;
    let started = Instant::now();
    let found = Found::read(&game);
    let read_for = started.elapsed();

    let salts: Vec<Salts> = found
        .into_salts()
        .into_iter()
        .filter(ingestion_cache::is_new)
        .collect();
    debug!(
        "memory: read the game in {read_for:.1?}, {} new salts",
        salts.len()
    );
    if !salts.is_empty() {
        match Salts::ingest_many(&salts, Source::Memory) {
            Ok(()) => info!("memory: ingested salts: {salts:?}"),
            Err(e) => warn!("memory: failed to ingest salts: {e:?}"),
        }
    }
    Ok(read_for)
}

#[derive(Default)]
struct Found {
    replies: Vec<CMsgClientToGcGetMatchMetaDataResponse>,
    /// `None` where two matches started in the same second.
    matches_by_start: HashMap<u32, Option<u64>>,
    urls: HashSet<Salts>,
}

impl Found {
    fn read(game: &GameProcess) -> Self {
        let now = u32::try_from(now_secs()).unwrap_or(u32::MAX);
        let mut found = Self::default();
        let mut buf = vec![0; CHUNK];
        let mut since_pause = 0_usize;
        for region in game.regions() {
            let end = region.base.saturating_add(region.size);
            let mut addr = region.base;
            while addr < end {
                let left = end.saturating_sub(addr);
                let chunk = buf.get_mut(..left.min(CHUNK)).unwrap_or_default();
                let read = game.read(addr, chunk);
                found.scan(chunk.get(..read).unwrap_or_default(), addr, now);
                since_pause = since_pause.saturating_add(read);
                if since_pause >= THROTTLE_EVERY {
                    since_pause = 0;
                    sleep(THROTTLE_PAUSE);
                }
                if left <= CHUNK {
                    break;
                }
                addr = addr.saturating_add(STRIDE);
            }
        }
        found
    }

    fn scan(&mut self, chunk: &[u8], addr: usize, now: u32) {
        self.replies.extend(scan::metadata_replies(chunk, addr));
        for (start, id) in scan::history_matches(chunk, now) {
            self.matches_by_start
                .entry(start)
                .and_modify(|known| {
                    if *known != Some(id) {
                        *known = None;
                    }
                })
                .or_insert(Some(id));
        }
        self.urls
            .extend(replay_urls(chunk).filter_map(|url| Salts::from_url(&url)));
    }

    fn into_salts(self) -> HashSet<Salts> {
        let Self {
            replies,
            matches_by_start,
            mut urls,
        } = self;
        let username = steam_user::current_steam_id3();
        urls.extend(replies.iter().filter_map(|reply| {
            let start = reply.replay_valid_through().checked_sub(REPLAY_VALID_FOR)?;
            let match_id = (*matches_by_start.get(&start)?)?;
            Salts::from_metadata_reply(match_id, reply, username)
        }));
        urls
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_is_the_match_that_started_three_weeks_before_it_expires() {
        let start = 1_788_222_625;
        let reply = |valid_through| CMsgClientToGcGetMatchMetaDataResponse {
            replay_salt: Some(509_659_246),
            metadata_salt: Some(818_291_387),
            replay_valid_through: Some(valid_through),
            replay_group_id: Some(273),
            ..Default::default()
        };
        let mut found = Found {
            replies: vec![reply(start + REPLAY_VALID_FOR), reply(start + 1)],
            ..Found::default()
        };
        found.matches_by_start.insert(start, Some(102_948_911));
        let salts: Vec<Salts> = found.into_salts().into_iter().collect();
        assert_eq!(salts.len(), 1);
        assert_eq!(
            (
                salts[0].match_id,
                salts[0].cluster_id,
                salts[0].metadata_salt
            ),
            (102_948_911, Some(273), Some(818_291_387))
        );
    }

    #[test]
    fn matches_starting_in_the_same_second_are_left_out() {
        let mut found = Found::default();
        found.matches_by_start.insert(1_788_222_625, None);
        found.replies.push(CMsgClientToGcGetMatchMetaDataResponse {
            metadata_salt: Some(1),
            replay_valid_through: Some(1_788_222_625 + REPLAY_VALID_FOR),
            ..Default::default()
        });
        assert!(found.into_salts().is_empty());
    }
}
