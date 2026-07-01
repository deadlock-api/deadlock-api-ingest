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

// GetMatchMetaData is rate-limited per-account by Steam's GC; a fast cadence reliably
// trips that limit, so requests are spaced two minutes apart.
const GC_MIN_INTERVAL: Duration = Duration::from_mins(2);
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
    let mut ids: Vec<u64> = ids.into_iter().filter(|id| !processed.contains(id)).collect();
    ids.sort_unstable_by(|a, b| b.cmp(a));
    ids.dedup();
    ids.truncate(take);
    ids
}

async fn throttle(last: &mut Option<Instant>, interval: Duration) {
    if let Some(prev) = *last {
        let elapsed = prev.elapsed();
        if elapsed < interval {
            tokio::time::sleep(interval.saturating_sub(elapsed)).await;
        }
    }
    *last = Some(Instant::now());
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
            warn!("gc: cannot open quota store: {e}");
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

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            warn!("gc: cannot build async runtime: {e}");
            return;
        }
    };
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
            warn!("gc: {} pass failed: {e}", ctx.account_name);
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
    let now = now_secs();

    if store.load_backoff(ctx.steam_id64).is_some_and(|until| now < until) {
        info!("gc: skipping {}, GC backed off", ctx.account_name);
        return Ok(());
    }

    let mut quota = store.load_quota(ctx.steam_id64);
    let remaining = quota.remaining(now);
    if remaining == 0 {
        info!("gc: skipping {}, 24h quota spent", ctx.account_name);
        return Ok(());
    }

    let to_fetch = tokio::task::spawn_blocking(api::to_fetch)
        .await
        .map_err(|e| GcError::Api(format!("to-fetch task panicked: {e}")))??;
    let picks = fresh_newest_first(to_fetch, processed, remaining);
    if picks.is_empty() {
        info!("gc: nothing to fetch for {}", ctx.account_name);
        return Ok(());
    }

    // Any connect failure means this account can't serve requests right now; back it off
    // for a day and let the batch move on. A successful connect clears any prior backoff.
    let session = match GcSession::connect(ctx).await {
        Ok(s) => {
            let _ = store.set_backoff(ctx.steam_id64, None);
            s
        }
        Err(e) => {
            warn!("gc: {} cannot connect to GC: {e}", ctx.account_name);
            store.set_backoff(ctx.steam_id64, Some(now + GC_BACKOFF_SECS))?;
            return Ok(());
        }
    };

    let username = Some(ctx.account_id());
    let mut fetched = 0u32;

    for match_id in picks {
        if game_running(sys) {
            info!("gc: Deadlock launched mid-pass, stopping {}", ctx.account_name);
            break;
        }
        throttle(last_request, GC_MIN_INTERVAL).await;

        let salts = match session.fetch_match_salts(match_id).await {
            Ok(s) => s,
            // Steam itself is throttling this account: treat the 24h bucket as spent so we
            // stop hammering it. Don't mark the id processed — another account may fetch it.
            Err(GcError::GcRateLimited) => {
                warn!("gc: {} rate-limited by Steam GC; quota exhausted for 24h", ctx.account_name);
                quota.exhaust(now);
                let _ = store.save_quota(ctx.steam_id64, &quota);
                break;
            }
            // Any other failure fetched nothing; skip the id so it isn't retried this pass.
            Err(e) => {
                warn!("gc: {} salt fetch failed for {match_id}: {e}", ctx.account_name);
                processed.insert(match_id);
                continue;
            }
        };

        processed.insert(match_id);
        let consume_ts = now_secs();
        quota.try_consume(consume_ts);
        store.save_quota(ctx.steam_id64, &quota)?;

        match tokio::task::spawn_blocking(move || api::post_salts(&salts, username)).await {
            Ok(Ok(())) => fetched += 1,
            Ok(Err(e)) => warn!("gc: {} salt POST failed for {match_id}: {e}", ctx.account_name),
            Err(e) => warn!("gc: {} salt POST task panicked: {e}", ctx.account_name),
        }

        if quota.remaining(consume_ts) == 0 {
            break;
        }
    }

    info!("gc: {} fetched {fetched} match salt(s)", ctx.account_name);
    Ok(())
}

/// Spawn the recurring background GC worker on a dedicated thread. Runs one pass now,
/// then every [`BACKGROUND_PASS_INTERVAL`]. Used by the long-running watcher mode.
pub(crate) fn spawn_background() {
    let spawned = std::thread::Builder::new()
        .name("gc-sync".into())
        .spawn(|| loop {
            run_pass_blocking();
            std::thread::sleep(BACKGROUND_PASS_INTERVAL);
        });
    if let Err(e) = spawned {
        warn!("gc: failed to spawn background worker: {e}");
    }
}
