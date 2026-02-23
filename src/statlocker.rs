use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::{OnceLock, mpsc};
use tracing::{debug, warn};

static STATLOCKER_ENABLED: AtomicBool = AtomicBool::new(true);
static HTTP_CLIENT: OnceLock<ureq::Agent> = OnceLock::new();
static SENDER: OnceLock<mpsc::SyncSender<u64>> = OnceLock::new();

fn client() -> &'static ureq::Agent {
    HTTP_CLIENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .new_agent()
    })
}

fn sender() -> &'static mpsc::SyncSender<u64> {
    SENDER.get_or_init(|| {
        let (tx, rx) = mpsc::sync_channel::<u64>(1000);
        std::thread::Builder::new()
            .name("statlocker".into())
            .spawn(move || {
                let username = crate::steam_user::current_steam_id3();
                for match_id in rx {
                    let url = if let Some(id) = username {
                        format!("https://statlocker.gg/api/match/{match_id}/populate?username=ingest-tool:{id}")
                    } else {
                        format!("https://statlocker.gg/api/match/{match_id}/populate")
                    };
                    debug!("Notifying Statlocker for match {match_id}");

                    match client().get(&url).call() {
                        Ok(resp) if resp.status().is_success() => {
                            debug!("Statlocker notified successfully for match {match_id}");
                        }
                        Ok(resp) => {
                            warn!(
                                "Statlocker returned status {} for match {match_id}",
                                resp.status()
                            );
                        }
                        Err(e) => {
                            warn!("Statlocker request failed for match {match_id}: {e}");
                        }
                    }
                }
            })
            .expect("failed to spawn statlocker thread");
        tx
    })
}

pub(crate) fn disable() {
    STATLOCKER_ENABLED.store(false, Ordering::Relaxed);
}

pub(crate) fn notify(match_id: u64) {
    if !STATLOCKER_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    if let Err(e) = sender().try_send(match_id) {
        warn!("Failed to enqueue Statlocker notification for match {match_id}: {e}");
    }
}

pub(crate) fn notify_many(match_ids: &[u64]) {
    let mut ids = match_ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        notify(id);
    }
}
