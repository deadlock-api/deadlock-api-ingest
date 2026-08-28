//! Sites that want to know a match id the moment its salts are ingested.
//!
//! Each [`Target`] gets its own bounded queue and a background thread that
//! GETs the target's URL for every match id, so a slow or unreachable site
//! never blocks ingestion or the other targets.

use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::{OnceLock, mpsc};
use tracing::{debug, warn};

static HTTP_CLIENT: OnceLock<ureq::Agent> = OnceLock::new();

fn client() -> &'static ureq::Agent {
    HTTP_CLIENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .new_agent()
    })
}

/// Every site notified after an ingestion, in the order they are pinged.
static TARGETS: [&Target; 2] = [&crate::statlocker::TARGET, &crate::deadchaps::TARGET];

/// A site that is told about every match this tool ingests.
pub(crate) struct Target {
    name: &'static str,
    enabled: AtomicBool,
    sender: OnceLock<mpsc::SyncSender<u64>>,
    /// Builds the URL to GET for a match, given the current Steam ID3 when known.
    url: fn(u64, Option<u32>) -> String,
}

impl Target {
    pub(crate) const fn new(name: &'static str, url: fn(u64, Option<u32>) -> String) -> Self {
        Self {
            name,
            enabled: AtomicBool::new(true),
            sender: OnceLock::new(),
            url,
        }
    }

    pub(crate) fn disable(&self) {
        self.enabled.store(false, Ordering::Relaxed);
    }

    fn sender(&'static self) -> &'static mpsc::SyncSender<u64> {
        self.sender.get_or_init(|| {
            let (tx, rx) = mpsc::sync_channel::<u64>(1000);
            std::thread::Builder::new()
                .name(self.name.to_lowercase())
                .spawn(move || {
                    let user = crate::steam_user::current_steam_id3();
                    for match_id in rx {
                        let url = (self.url)(match_id, user);
                        debug!("Notifying {} for match {match_id}", self.name);

                        match client().get(&url).call() {
                            Ok(resp) if resp.status().is_success() => {
                                debug!("{} notified successfully for match {match_id}", self.name);
                            }
                            Ok(resp) => {
                                warn!(
                                    "{} returned status {} for match {match_id}",
                                    self.name,
                                    resp.status()
                                );
                            }
                            Err(e) => {
                                warn!("{} request failed for match {match_id}: {e}", self.name);
                            }
                        }
                    }
                })
                .expect("failed to spawn notifier thread");
            tx
        })
    }

    fn notify(&'static self, match_id: u64) {
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }

        if let Err(e) = self.sender().try_send(match_id) {
            warn!(
                "Failed to enqueue {} notification for match {match_id}: {e}",
                self.name
            );
        }
    }
}

/// Tell every enabled target about one ingested match.
pub(crate) fn notify(match_id: u64) {
    for target in TARGETS {
        target.notify(match_id);
    }
}

/// Tell every enabled target about a batch of ingested matches, each id once.
pub(crate) fn notify_many(match_ids: &[u64]) {
    let mut ids = match_ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        notify(id);
    }
}
