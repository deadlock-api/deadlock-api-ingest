//! Steam Game Coordinator match-salt recovery.
//!
//! Independently of the httpcache scraper, this logs into every remembered Steam
//! account through the user's own session, pulls the deadlock-api "to-fetch" list, and
//! recovers up to [`quota::FETCH_QUOTA_LIMIT`] match salts per account per 24h via the
//! GC `GetMatchMetaData` message, `POST`ing them to the same salts endpoint.
//!
//! It never runs while Deadlock itself is running (Steam routes GC traffic to the
//! game's own pipe), and it is best-effort per account: an account that can't reach the
//! GC (e.g. doesn't own the game) is backed off for a day and the others still run.

// steam-vent's CM/GC connection futures are inherently large; boxing them would only
// add allocations to a background daemon that awaits one at a time.
#![allow(clippy::large_futures)]

mod api;
mod auth;
mod client;
mod error;
mod quota;
mod store;

use core::time::Duration;
use std::collections::HashSet;

use tokio::time::Instant;
use tracing::{info, warn};

use auth::AuthContext;
use client::GcSession;
use error::GcError;
use store::{GcStore, now_secs};

use crate::utils::Salts;

// GetMatchMetaData is rate-limited per-account by Steam's GC. A faster cadence trips
// that limit sooner; when it does, the account's run stops (see `GcRateLimited`).
const GC_MIN_INTERVAL: Duration = Duration::from_secs(20);
// Between background passes. Most passes no-op quickly once the 24h quota is spent.
const BACKGROUND_PASS_INTERVAL: Duration = Duration::from_mins(30);
// An account whose GC handshake failed (e.g. doesn't own the game) fails identically
// every pass; skip it for a day rather than re-paying the connect/timeout cost.
const GC_BACKOFF_SECS: i64 = 24 * 60 * 60;

// Deadlock process names across platforms; if any is running, GC traffic goes to the
// game, not us, so the whole pass is skipped.
const GAME_PROCESS_NAMES: &[&str] = &["project8.exe", "deadlock.exe", "project8"];

fn game_running(sys: &mut sysinfo::System) -> bool {
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    GAME_PROCESS_NAMES.iter().any(|name| {
        sys.processes_by_exact_name(std::ffi::OsStr::new(name))
            .next()
            .is_some()
    })
}

fn fresh_newest_first(ids: Vec<u64>, processed: &HashSet<u64>, take: usize) -> Vec<u64> {
    let mut ids: Vec<u64> = ids
        .into_iter()
        .filter(|id| !processed.contains(id))
        .collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    ids.dedup();
    ids.truncate(take);
    ids
}

async fn throttle(last: &mut Option<Instant>, interval: Duration) {
    if let Some(prev) = *last {
        tokio::time::sleep_until(prev + interval).await;
    }
    *last = Some(Instant::now());
}

fn runtime() -> Option<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .inspect_err(|e| warn!("gc: cannot build async runtime: {e}"))
        .ok()
}

/// POST one match's salts; failures are logged and reported as `false`.
async fn post(salts: Salts) -> bool {
    let match_id = salts.match_id;
    match tokio::task::spawn_blocking(move || salts.ingest()).await {
        Ok(Ok(())) => true,
        Ok(Err(e)) => {
            warn!("gc: salt POST failed for {match_id}: {e}");
            false
        }
        Err(e) => {
            warn!("gc: salt POST task panicked: {e}");
            false
        }
    }
}

/// Run one pass over every currently-decryptable account, then return. Builds its own
/// Tokio runtime so the rest of the crate stays synchronous. Safe to call repeatedly;
/// the persisted per-account quota gates it to 40/24h regardless of pass frequency.
pub(crate) fn run_pass_blocking() {
    let mut sys = sysinfo::System::new();
    if game_running(&mut sys) {
        info!("gc: Deadlock is running, skipping GC pass");
        return;
    }

    let store = match GcStore::open() {
        Ok(s) => s,
        Err(e) => {
            info!("gc: quota store unavailable, skipping GC pass: {e}");
            return;
        }
    };
    let contexts = match auth::recover_all() {
        Ok(c) => c,
        Err(e) => {
            info!("gc: no usable Steam account, skipping GC pass: {e}");
            return;
        }
    };
    let Some(runtime) = runtime() else { return };
    runtime.block_on(run_pass(&contexts, &store, &mut sys));
}

async fn run_pass(contexts: &[AuthContext], store: &GcStore, sys: &mut sysinfo::System) {
    // Shared across accounts so the same match isn't fetched twice in one pass (the
    // to-fetch list is global and the server drops matches only after some lag).
    let mut processed: HashSet<u64> = HashSet::new();
    let mut last_request: Option<Instant> = None;

    for ctx in contexts {
        if game_running(sys) {
            info!("gc: Deadlock launched mid-pass, stopping");
            break;
        }
        if let Err(e) = run_account(ctx, store, &mut processed, &mut last_request, sys).await {
            warn!("gc: account {} pass failed: {e}", ctx.account_id());
        }
    }
}

async fn run_account(
    ctx: &AuthContext,
    store: &GcStore,
    processed: &mut HashSet<u64>,
    last_request: &mut Option<Instant>,
    sys: &mut sysinfo::System,
) -> Result<(), GcError> {
    let account = ctx.account_id();
    let now = now_secs();

    let backoff = store.load_backoff(ctx.steam_id64)?;
    if backoff.is_some_and(|until| now < until) {
        info!("gc: skipping account {account}, GC backed off");
        return Ok(());
    }

    let mut quota = store.load_quota(ctx.steam_id64)?;
    let remaining = quota.remaining(now);
    if remaining == 0 {
        info!("gc: skipping account {account}, 24h quota spent");
        return Ok(());
    }

    let to_fetch = tokio::task::spawn_blocking(api::to_fetch)
        .await
        .map_err(|e| GcError::Api(format!("to-fetch task panicked: {e}")))??;
    let picks = fresh_newest_first(to_fetch, processed, remaining);
    if picks.is_empty() {
        info!("gc: nothing to fetch for account {account}");
        return Ok(());
    }

    // Any connect failure means this account can't serve requests right now; back it off
    // for a day and let the batch move on. A successful connect clears any prior backoff.
    let session = match GcSession::connect(ctx).await {
        Ok(s) => {
            if backoff.is_some() {
                let _ = store.set_backoff(ctx.steam_id64, None);
            }
            s
        }
        Err(e) => {
            warn!("gc: account {account} cannot connect to GC: {e}");
            store.set_backoff(ctx.steam_id64, Some(now + GC_BACKOFF_SECS))?;
            return Ok(());
        }
    };

    let mut fetched = 0u32;
    for match_id in picks {
        throttle(last_request, GC_MIN_INTERVAL).await;
        if game_running(sys) {
            info!("gc: Deadlock launched mid-pass, stopping account {account}");
            break;
        }

        let salts = match session.fetch_match_salts(match_id).await {
            Ok(s) => s,
            // Steam itself is throttling this account: treat the 24h bucket as spent so we
            // stop hammering it. Don't mark the id processed — another account may fetch it.
            Err(GcError::GcRateLimited) => {
                warn!("gc: account {account} rate-limited by Steam GC; quota exhausted for 24h");
                quota.exhaust(now_secs());
                let _ = store.save_quota(ctx.steam_id64, &quota);
                break;
            }
            // Any other failure fetched nothing; skip the id so it isn't retried this pass.
            Err(e) => {
                warn!("gc: account {account} salt fetch failed for {match_id}: {e}");
                processed.insert(match_id);
                continue;
            }
        };

        processed.insert(match_id);
        quota.try_consume(now_secs());
        store.save_quota(ctx.steam_id64, &quota)?;
        if post(salts).await {
            fetched += 1;
        }
    }

    info!("gc: account {account} fetched {fetched} match salt(s)");
    Ok(())
}

/// One-shot: recover salts for every match in each remembered account's own match history
/// that deadlock-api doesn't know yet. Ignores the 40/24h quota and runs until done or
/// until Steam rate-limits the account. Returns whether every match was recovered.
pub(crate) fn run_own_matches_blocking() -> bool {
    let mut sys = sysinfo::System::new();
    if game_running(&mut sys) {
        warn!("Deadlock is running, close it and run this again");
        return false;
    }
    let contexts = match auth::recover_all() {
        Ok(c) => c,
        Err(e) => {
            warn!("gc: no usable Steam account (is Steam logged in with 'remember me'?): {e}");
            return false;
        }
    };
    let Some(runtime) = runtime() else {
        return false;
    };

    let mut done = HashSet::new();
    let mut last_request = None;
    let mut rate_limited = false;
    let mut failed = 0usize;
    for ctx in &contexts {
        if game_running(&mut sys) {
            warn!("Deadlock was launched, stopping. Close it and run this again.");
            return false;
        }
        match runtime.block_on(run_own_account(ctx, &mut done, &mut last_request, &mut sys)) {
            Ok(n) => failed += n,
            Err(GcError::GcRateLimited) => {
                warn!("gc: account {} was rate-limited by Steam", ctx.account_id());
                rate_limited = true;
            }
            Err(e) => {
                warn!("gc: account {} failed: {e}", ctx.account_id());
                failed += 1;
            }
        }
    }
    if rate_limited {
        warn!(
            "Steam rate limit reached. Run this again on another day to fetch the remaining matches."
        );
    } else if failed > 0 {
        warn!("{failed} match(es)/account(s) could not be processed. Run this again to retry.");
    } else {
        info!("Done, all own matches processed.");
    }
    !rate_limited && failed == 0
}

/// Returns how many missing matches could not be recovered and ingested, or
/// `GcRateLimited` if Steam rate-limited the account (after processing what it could).
async fn run_own_account(
    ctx: &AuthContext,
    done: &mut HashSet<u64>,
    last_request: &mut Option<Instant>,
    sys: &mut sysinfo::System,
) -> Result<usize, GcError> {
    let account = ctx.account_id();
    let session = GcSession::connect(ctx).await?;
    let (history, history_complete) = session.fetch_match_history(account).await?;
    let known = {
        let ids = history.clone();
        tokio::task::spawn_blocking(move || api::known_match_ids(&ids))
            .await
            .map_err(|e| GcError::Api(format!("metadata task panicked: {e}")))??
    };
    let missing: Vec<u64> = history
        .iter()
        .copied()
        .filter(|id| !known.contains(id) && !done.contains(id))
        .collect();
    info!(
        "gc: account {account} has {} match(es) in history, {} missing salts (~20s each)",
        history.len(),
        missing.len()
    );

    let mut failed = 0;
    for (i, &match_id) in missing.iter().enumerate() {
        throttle(last_request, GC_MIN_INTERVAL).await;
        if game_running(sys) {
            return Err(GcError::GcUnavailable("Deadlock was launched".into()));
        }

        let salts = match session.fetch_match_salts(match_id).await {
            Ok(s) => s,
            Err(GcError::GcRateLimited) => return Err(GcError::GcRateLimited),
            Err(e) => {
                warn!("gc: salt fetch failed for {match_id}: {e}");
                failed += 1;
                continue;
            }
        };
        done.insert(match_id);
        if post(salts).await {
            info!(
                "gc: ingested match {match_id} ({}/{})",
                i + 1,
                missing.len()
            );
        } else {
            failed += 1;
        }
    }
    // Older pages were cut off by Steam's rate limit, so they still need another run.
    if !history_complete {
        return Err(GcError::GcRateLimited);
    }
    Ok(failed)
}

/// Spawn the recurring background GC worker on a dedicated thread. Runs one pass now,
/// then every [`BACKGROUND_PASS_INTERVAL`]. Used by the long-running watcher mode.
pub(crate) fn spawn_background() {
    let spawned = std::thread::Builder::new()
        .name("gc-sync".into())
        .spawn(|| {
            loop {
                run_pass_blocking();
                std::thread::sleep(BACKGROUND_PASS_INTERVAL);
            }
        });
    if let Err(e) = spawned {
        warn!("gc: failed to spawn background worker: {e}");
    }
}
