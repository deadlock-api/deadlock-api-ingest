//! Steam Game Coordinator match-salt recovery.
//!
//! Independently of the httpcache scraper, this logs into every remembered Steam
//! account through the user's own session and recovers up to [`quota::FETCH_QUOTA_LIMIT`]
//! match salts per account per 24h via the GC `GetMatchMetaData` message, `POST`ing them
//! to the same salts endpoint. Matches come first from the account's Steam friends: those
//! deadlock-api knows play Deadlock and who likely played since their last stored match
//! (see [`friends`]) get their newest history page fetched through the GC (at most
//! [`quota::FRIEND_HISTORY_QUOTA_LIMIT`] per account per 24h). Remaining quota goes to the
//! deadlock-api "to-fetch" list.
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
mod friends;
mod quota;
mod store;

use core::time::Duration;
use std::collections::HashSet;

use tokio::time::Instant;
use tracing::{info, warn};

use auth::AuthContext;
use client::GcSession;
use error::GcError;
use quota::QuotaWindow;
use store::{GcStore, MAX_SALT_ATTEMPTS, now_secs};

use crate::ingestion_cache::Source;
use crate::utils::Salts;

// GetMatchMetaData is rate-limited per-account by Steam's GC. A faster cadence trips
// that limit sooner; when it does, the account's run stops (see `GcRateLimited`).
const GC_MIN_INTERVAL: Duration = Duration::from_secs(20);
// Between background passes. Most passes no-op quickly once the 24h quota is spent.
const BACKGROUND_PASS_INTERVAL: Duration = Duration::from_mins(30);
// An account whose GC handshake failed (e.g. doesn't own the game) fails identically
// every pass; skip it for a day rather than re-paying the connect/timeout cost.
const GC_BACKOFF_SECS: i64 = 24 * 60 * 60;
// Friend history lookups per account per pass, on top of the 20/24h history quota, so one
// pass doesn't spend the whole day's budget at once.
const MAX_FRIEND_HISTORIES_PER_PASS: usize = 5;
// After a friend's history was looked up (by any account), leave them alone for a while;
// a private history won't become public soon.
const FRIEND_COOLDOWN_SECS: i64 = 6 * 60 * 60;
const PRIVATE_FRIEND_COOLDOWN_SECS: i64 = 7 * 24 * 60 * 60;

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

/// Count one salt fetch of `match_id` towards [`MAX_SALT_ATTEMPTS`]. Rate-limited fetches
/// don't count: Steam refused the account, not the match.
fn record_salt_attempt(store: Option<&GcStore>, match_id: u64) {
    if let Some(Err(e)) = store.map(|s| s.record_salt_attempt(match_id)) {
        warn!("gc: {e}");
    }
}

/// POST one match's salts; failures are logged and reported as `false`.
async fn post(salts: Salts, source: Source) -> bool {
    let match_id = salts.match_id;
    match tokio::task::spawn_blocking(move || salts.ingest(source)).await {
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
            warn!("gc: no usable Steam account, skipping GC pass: {e}");
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

/// Run a blocking deadlock-api call off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, GcError> + Send + 'static,
) -> Result<T, GcError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| GcError::Api(format!("API task panicked: {e}")))?
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

    let quota = store.load_quota(ctx.steam_id64)?;
    if quota.remaining(now) == 0 {
        info!("gc: skipping account {account}, 24h quota spent");
        return Ok(());
    }
    let history_quota = store.load_history_quota(ctx.steam_id64)?;

    // A failed to-fetch read only empties that phase; friends' matches can still be fetched.
    let to_fetch = blocking(api::to_fetch)
        .await
        .inspect_err(|e| warn!("gc: {e}"))
        .unwrap_or_default();
    if to_fetch.iter().all(|id| processed.contains(id)) && history_budget(&history_quota, now) == 0
    {
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

    let mut run = AccountRun {
        ctx,
        store,
        session,
        quota,
        history_quota,
        processed,
        last_request,
        sys,
    };
    // Friends' fresh matches go first; whatever quota is left goes to the global list.
    let mut match_ids = run.friend_matches().await;
    match_ids.extend(fresh_newest_first(to_fetch, run.processed, usize::MAX));
    let fetched = run.fetch_salts(match_ids).await;

    info!("gc: account {account} fetched {fetched} match salt(s)");
    Ok(())
}

/// Friend history lookups this account may still make this pass.
fn history_budget(history_quota: &QuotaWindow, now: i64) -> usize {
    history_quota
        .remaining(now)
        .min(MAX_FRIEND_HISTORIES_PER_PASS)
}

/// One account's connected pass: the live session plus the state its fetches spend.
struct AccountRun<'a> {
    ctx: &'a AuthContext,
    store: &'a GcStore,
    session: GcSession,
    quota: QuotaWindow,
    history_quota: QuotaWindow,
    processed: &'a mut HashSet<u64>,
    last_request: &'a mut Option<Instant>,
    sys: &'a mut sysinfo::System,
}

impl AccountRun<'_> {
    /// Record one salt fetch (or, if `exhaust`, Steam's rate limit) and persist it.
    fn spend_salt(&mut self, exhaust: bool) -> Result<(), GcError> {
        if exhaust {
            self.quota.exhaust(now_secs());
        } else {
            self.quota.try_consume(now_secs());
        }
        self.store.save_quota(self.ctx.steam_id64, &self.quota)
    }

    /// Record one history lookup (or, if `exhaust`, Steam's rate limit) and persist it.
    fn spend_history(&mut self, exhaust: bool) -> Result<(), GcError> {
        if exhaust {
            self.history_quota.exhaust(now_secs());
        } else {
            self.history_quota.try_consume(now_secs());
        }
        self.store
            .save_history_quota(self.ctx.steam_id64, &self.history_quota)
    }

    /// Fetch and post salts for `match_ids` in order, skipping ones already processed this
    /// pass, until the salt quota is spent or the run must stop (rate limit, game
    /// launched). Returns how many were posted.
    async fn fetch_salts(&mut self, match_ids: Vec<u64>) -> u32 {
        let account = self.ctx.account_id();
        let given_up = self
            .store
            .given_up_matches()
            .inspect_err(|e| warn!("gc: {e}"))
            .unwrap_or_default();
        let mut fetched = 0;
        for match_id in match_ids {
            if self.quota.remaining(now_secs()) == 0 {
                break;
            }
            if self.processed.contains(&match_id) || given_up.contains(&match_id) {
                continue;
            }
            throttle(self.last_request, GC_MIN_INTERVAL).await;
            if game_running(self.sys) {
                info!("gc: Deadlock launched mid-pass, stopping account {account}");
                break;
            }

            let salts = match self.session.fetch_match_salts(match_id).await {
                Ok(s) => s,
                // Steam itself is throttling this account: treat the 24h bucket as spent so
                // we stop hammering it. Don't mark the id processed — another account may
                // fetch it.
                Err(GcError::GcRateLimited) => {
                    warn!(
                        "gc: account {account} rate-limited by Steam GC; quota exhausted for 24h"
                    );
                    if let Err(e) = self.spend_salt(true) {
                        warn!("gc: {e}");
                    }
                    break;
                }
                // Any other failure fetched nothing; skip the id so it isn't retried this pass.
                Err(e) => {
                    warn!("gc: account {account} salt fetch failed for {match_id}: {e}");
                    self.processed.insert(match_id);
                    record_salt_attempt(Some(self.store), match_id);
                    continue;
                }
            };

            self.processed.insert(match_id);
            record_salt_attempt(Some(self.store), match_id);
            if let Err(e) = self.spend_salt(false) {
                warn!("gc: {e}");
                break;
            }
            if post(salts, Source::Gc).await {
                fetched += 1;
            }
        }
        fetched
    }

    /// Matches deadlock-api is missing salts for, newest first, from the histories of
    /// friends who most likely played since the API last saw them. Best-effort: any
    /// failure just yields fewer (or no) matches.
    async fn friend_matches(&mut self) -> Vec<u64> {
        let now = now_secs();
        let budget = history_budget(&self.history_quota, now).min(self.quota.remaining(now));
        if budget == 0 || self.session.friends.is_empty() {
            return Vec::new();
        }
        let picks = match self.pick_friends(budget).await {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    "gc: friend lookup failed for account {}: {e}",
                    self.ctx.account_id()
                );
                return Vec::new();
            }
        };

        let found = self.lookup_histories(picks).await;
        let mut new_ids = fresh_newest_first(found, self.processed, usize::MAX);
        if new_ids.is_empty() {
            return new_ids;
        }
        let ids = new_ids.clone();
        match blocking(move || api::known_match_ids(&ids)).await {
            Ok(known) => {
                new_ids.retain(|id| !known.contains(id));
                if let Err(e) = self.store.forget_salt_attempts(&known) {
                    warn!("gc: {e}");
                }
            }
            Err(e) => warn!("gc: {e}"),
        }
        info!(
            "gc: account {} found {} new friend match(es)",
            self.ctx.account_id(),
            new_ids.len()
        );
        new_ids
    }

    /// Up to `budget` friends worth a history lookup, most likely first, each with the
    /// newest match id deadlock-api has stored for them.
    async fn pick_friends(&self, budget: usize) -> Result<Vec<(u32, u64)>, GcError> {
        let now = now_secs();
        let cooldowns = self.store.friend_cooldowns()?;
        let due: Vec<u32> = self
            .session
            .friends
            .iter()
            .copied()
            .filter(|id| cooldowns.get(id).is_none_or(|&until| until <= now))
            .collect();
        // Steam's persona replies and the deadlock-api reads don't depend on each other.
        let (candidates, personas) = {
            let due_api = due.clone();
            tokio::join!(
                blocking(move || active_friends(&due_api)),
                self.session.request_personas(&due)
            )
        };
        let candidates = candidates?;

        let scored = candidates
            .iter()
            .filter_map(|c| {
                let persona = personas.get(&c.account_id).copied();
                friends::estimate(now, c.last_end, &c.starts, persona)
                    .map(|score| ((c.account_id, c.latest_match_id), score))
            })
            .collect();
        let picks = friends::pick(scored, budget);
        info!(
            "gc: account {}: {} friend(s), {} recently active, {} likely with new matches",
            self.ctx.account_id(),
            self.session.friends.len(),
            candidates.len(),
            picks.len()
        );
        Ok(picks)
    }

    /// Fetch each friend's newest history page and return the match ids newer than what
    /// deadlock-api has stored for them. Spends the history quota and sets cooldowns.
    async fn lookup_histories(&mut self, picks: Vec<(u32, u64)>) -> Vec<u64> {
        let mut new_ids = Vec::new();
        for (friend, latest_stored) in picks {
            throttle(self.last_request, GC_MIN_INTERVAL).await;
            if game_running(self.sys) {
                break;
            }
            let result = self.session.fetch_recent_matches(friend).await;
            let rate_limited = matches!(result, Err(GcError::GcRateLimited));
            // Every request counts against Steam's limit, whatever it answers.
            if let Err(e) = self.spend_history(rate_limited) {
                warn!("gc: {e}");
                break;
            }

            let cooldown = match result {
                Ok(history) => {
                    new_ids.extend(history.into_iter().filter(|&id| id > latest_stored));
                    FRIEND_COOLDOWN_SECS
                }
                Err(GcError::GcRateLimited) => {
                    warn!(
                        "gc: account {} rate-limited on match history; skipping friends",
                        self.ctx.account_id()
                    );
                    break;
                }
                Err(GcError::HistoryPrivate) => PRIVATE_FRIEND_COOLDOWN_SECS,
                Err(e) => {
                    warn!("gc: match history for friend {friend} failed: {e}");
                    FRIEND_COOLDOWN_SECS
                }
            };
            let now = now_secs();
            if let Err(e) = self.store.set_friend_cooldown(friend, now + cooldown, now) {
                warn!("gc: {e}");
            }
        }
        new_ids
    }
}

/// A friend deadlock-api has seen play recently, with what it has stored for them.
struct ActiveFriend {
    account_id: u32,
    latest_match_id: u64,
    /// End of their newest stored match.
    last_end: i64,
    /// Start times of their newest stored matches, newest first.
    starts: Vec<i64>,
}

/// Of `friends`, the ones deadlock-api knows play Deadlock, with their stored history.
/// Friends whose stored history can't be read, or is empty, are skipped.
fn active_friends(friends: &[u32]) -> Result<Vec<ActiveFriend>, GcError> {
    let players = api::recent_players(friends)?;
    Ok(players
        .into_iter()
        .filter_map(|account_id| {
            let history = api::stored_match_history(account_id)
                .inspect_err(|e| warn!("gc: stored history for friend {account_id}: {e}"))
                .ok()?;
            let latest_match_id = history.iter().map(|r| r.match_id).max()?;
            let last_end = history
                .iter()
                .map(|r| i64::from(r.start_time) + i64::from(r.match_duration_s))
                .max()?;
            Some(ActiveFriend {
                account_id,
                latest_match_id,
                last_end,
                starts: history
                    .iter()
                    .take(friends::GAP_SAMPLE)
                    .map(|r| i64::from(r.start_time))
                    .collect(),
            })
        })
        .collect())
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

    // Only for the per-match attempt counts. A background pass holding it right now
    // shouldn't block a run the user started, so this runs without the cap instead.
    let store = GcStore::open()
        .inspect_err(|e| warn!("gc: store unavailable, not tracking salt attempts: {e}"))
        .ok();
    let mut done = HashSet::new();
    let mut last_request = None;
    let mut rate_limited = false;
    let mut failed = 0usize;
    for ctx in &contexts {
        if game_running(&mut sys) {
            warn!("Deadlock was launched, stopping. Close it and run this again.");
            return false;
        }
        match runtime.block_on(run_own_account(
            ctx,
            store.as_ref(),
            &mut done,
            &mut last_request,
            &mut sys,
        )) {
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
    store: Option<&GcStore>,
    done: &mut HashSet<u64>,
    last_request: &mut Option<Instant>,
    sys: &mut sysinfo::System,
) -> Result<usize, GcError> {
    let account = ctx.account_id();
    let session = GcSession::connect(ctx).await?;
    let (history, history_complete) = session.fetch_match_history(account).await?;
    let known = {
        let ids = history.clone();
        blocking(move || api::known_match_ids(&ids)).await?
    };
    let given_up = store.map_or_else(HashSet::new, |store| {
        if let Err(e) = store.forget_salt_attempts(&known) {
            warn!("gc: {e}");
        }
        store
            .given_up_matches()
            .inspect_err(|e| warn!("gc: {e}"))
            .unwrap_or_default()
    });
    let (missing, skipped): (Vec<u64>, Vec<u64>) = history
        .iter()
        .copied()
        .filter(|id| !known.contains(id) && !done.contains(id))
        .partition(|id| !given_up.contains(id));
    info!(
        "gc: account {account} has {} match(es) in history, {} missing salts (~20s each)",
        history.len(),
        missing.len()
    );
    if !skipped.is_empty() {
        info!(
            "gc: account {account} skips {} match(es) given up on after {MAX_SALT_ATTEMPTS} attempts",
            skipped.len()
        );
    }

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
                record_salt_attempt(store, match_id);
                failed += 1;
                continue;
            }
        };
        done.insert(match_id);
        record_salt_attempt(store, match_id);
        if post(salts, Source::Own).await {
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
/// Report once at startup whether any Steam session is decryptable. Passes skip early
/// while Deadlock is running, so without this an auth failure (e.g. DPAPI unavailable
/// because the process has no interactive logon) can stay out of the logs for days.
fn log_auth_status() {
    match auth::recover_all() {
        Ok(contexts) => info!("gc: {} Steam account(s) available", contexts.len()),
        Err(e) => warn!("gc: no usable Steam account, GC passes will be skipped: {e}"),
    }
}

pub(crate) fn spawn_background() {
    let spawned = std::thread::Builder::new()
        .name("gc-sync".into())
        .spawn(|| {
            log_auth_status();
            loop {
                run_pass_blocking();
                std::thread::sleep(BACKGROUND_PASS_INTERVAL);
            }
        });
    if let Err(e) = spawned {
        warn!("gc: failed to spawn background worker: {e}");
    }
}
