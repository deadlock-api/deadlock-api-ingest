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
use std::collections::HashMap;
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
use scan::KnownMatch;

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
/// A reply's `replay_valid_through` is the match's start plus three weeks, and its
/// `replay_processing_through` the match's end plus twenty minutes.
const REPLAY_VALID_FOR: u32 = 21 * 24 * 60 * 60;
const PROCESSED_AFTER_END: u32 = 20 * 60;
const DURATION_TOLERANCE: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Region {
    base: usize,
    size: usize,
}

/// Adjacent regions joined, so an object across their boundary is read whole.
fn coalesced(regions: Vec<Region>) -> Vec<Region> {
    let mut out: Vec<Region> = Vec::with_capacity(regions.len());
    for region in regions {
        match out.last_mut() {
            Some(last) if last.base.checked_add(last.size) == Some(region.base) => {
                last.size = last.size.saturating_add(region.size);
            }
            _ => out.push(region),
        }
    }
    out
}

/// Reads the game whenever it runs.
pub(crate) fn spawn_background() {
    if cfg!(not(any(windows, target_os = "linux"))) {
        return;
    }
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
            "; run the installer again or see the README to set up the memory helper, or start \
             the game through the launch wrapper (`deadlock-api-ingest -- %command%`)"
        } else {
            ""
        };
        warn!("memory: not allowed to read Deadlock's memory: {e}{hint}");
    }
}

/// Reads the game once and ingests the new salts; returns how long the read took.
fn pass(pid: u32) -> io::Result<Duration> {
    let game = GameProcess::open(pid)?;
    debug!("memory: reading the game (pid {pid})");
    let started = Instant::now();
    let found = Found::read(&game);
    let read_for = started.elapsed();

    let all = found.into_salts();
    debug!(
        "memory: read the game in {read_for:.1?}, salts for {} matches",
        all.len()
    );
    let salts: Vec<Salts> = all.into_iter().filter(ingestion_cache::is_new).collect();
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
    matches_by_start: HashMap<u32, Option<KnownMatch>>,
    urls: Vec<Salts>,
}

impl Found {
    fn read(game: &GameProcess) -> Self {
        let now = u32::try_from(now_secs()).unwrap_or(u32::MAX);
        let mut found = Self::default();
        let mut buf = vec![0; CHUNK];
        let mut since_pause = 0_usize;
        for region in coalesced(game.regions()) {
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
        let known = scan::history_matches(chunk, now)
            .into_iter()
            .chain(scan::match_summaries(chunk, now));
        for found in known {
            self.matches_by_start
                .entry(found.start)
                .and_modify(|known| {
                    if known.is_some_and(|k| k.match_id != found.match_id) {
                        *known = None;
                    }
                })
                .or_insert(Some(found));
        }
        self.urls
            .extend(replay_urls(chunk).filter_map(|url| Salts::from_url(&url)));
    }

    /// One `Salts` per match: a match can be both in a URL and in a reply.
    fn into_salts(self) -> Vec<Salts> {
        let Self {
            replies,
            matches_by_start,
            urls,
        } = self;
        let username = steam_user::current_steam_id3();
        let from_replies: Vec<Salts> = replies
            .iter()
            .filter_map(|reply| {
                let start = reply.replay_valid_through().checked_sub(REPLAY_VALID_FOR)?;
                let duration = reply
                    .replay_processing_through()
                    .checked_sub(PROCESSED_AFTER_END)?
                    .checked_sub(start)?;
                let known = (*matches_by_start.get(&start)?)?;
                if known.duration.abs_diff(duration) > DURATION_TOLERANCE {
                    return None;
                }
                Salts::from_metadata_reply(known.match_id, reply, username)
            })
            .collect();
        let unmatched = replies.len().saturating_sub(from_replies.len());
        if unmatched > 0 {
            debug!("memory: {unmatched} replies matched no match in memory");
        }

        let mut by_match: HashMap<u64, Salts> = HashMap::new();
        for salts in urls.into_iter().chain(from_replies) {
            by_match
                .entry(salts.match_id)
                .and_modify(|known| {
                    known.cluster_id = known.cluster_id.or(salts.cluster_id);
                    known.metadata_salt = known.metadata_salt.or(salts.metadata_salt);
                    known.replay_salt = known.replay_salt.or(salts.replay_salt);
                })
                .or_insert(salts);
        }
        by_match.into_values().collect()
    }
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// Read from the live game (Proton) for match 117936520, which started at 1792150476
    /// and ran 1596 s.
    fn live_reply() -> CMsgClientToGcGetMatchMetaDataResponse {
        CMsgClientToGcGetMatchMetaDataResponse {
            replay_salt: Some(1_330_069_048),
            metadata_salt: Some(1_285_305_573),
            replay_valid_through: Some(1_793_964_876),
            replay_group_id: Some(118),
            replay_processing_through: Some(1_792_153_272),
            ..Default::default()
        }
    }

    const LIVE_MATCH: KnownMatch = KnownMatch {
        start: 1_792_150_476,
        match_id: 117_936_520,
        duration: 1596,
    };

    fn found_with(known: Option<KnownMatch>) -> Found {
        let mut found = Found {
            replies: vec![live_reply()],
            ..Found::default()
        };
        found.matches_by_start.insert(LIVE_MATCH.start, known);
        found
    }

    #[test]
    fn a_reply_is_the_match_with_its_start_and_duration() {
        let salts = found_with(Some(LIVE_MATCH)).into_salts();
        assert_eq!(salts.len(), 1);
        assert_eq!(
            (
                salts[0].match_id,
                salts[0].cluster_id,
                salts[0].metadata_salt,
                salts[0].replay_salt
            ),
            (
                117_936_520,
                Some(118),
                Some(1_285_305_573),
                Some(1_330_069_048)
            )
        );
    }

    #[test]
    fn a_match_that_ran_differently_is_not_paired() {
        let other = KnownMatch {
            duration: 1200,
            ..LIVE_MATCH
        };
        assert_eq!(found_with(Some(other)).into_salts(), Vec::<Salts>::new());
    }

    #[test]
    fn matches_starting_in_the_same_second_are_left_out() {
        assert_eq!(found_with(None).into_salts(), Vec::<Salts>::new());
    }

    #[test]
    fn a_url_and_a_reply_for_one_match_are_merged() {
        let mut found = found_with(Some(LIVE_MATCH));
        found.urls.push(Salts {
            match_id: LIVE_MATCH.match_id,
            cluster_id: Some(118),
            metadata_salt: None,
            replay_salt: Some(1_330_069_048),
            username: None,
        });
        let salts = found.into_salts();
        assert_eq!(salts.len(), 1);
        assert_eq!(salts[0].metadata_salt, Some(1_285_305_573));
    }

    #[test]
    fn adjacent_regions_are_read_as_one() {
        let region = |base, size| Region { base, size };
        assert_eq!(
            coalesced(vec![region(0, 16), region(16, 16), region(64, 16)]),
            [region(0, 32), region(64, 16)]
        );
    }
}
