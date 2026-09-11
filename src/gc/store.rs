//! Persisted, per-account GC state: the rolling-window quota timestamps (so the
//! 40/24h cap survives restarts) and a GC "backoff until" for accounts whose
//! handshake is known-broken (e.g. they don't own the game). A single JSON file in
//! the app data dir, loaded and saved whole — the data is tiny.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::warn;

use super::error::GcError;
use super::quota::{FETCH_QUOTA_LIMIT, FETCH_QUOTA_WINDOW_SECS, QuotaWindow};

const STORE_FILE: &str = "gc-quota.json";

pub(crate) fn now_secs() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    i64::try_from(secs).unwrap_or(i64::MAX)
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct AccountState {
    #[serde(default)]
    quota_hits: Vec<i64>,
    #[serde(default)]
    gc_backoff_until: Option<i64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreData {
    #[serde(default)]
    accounts: HashMap<String, AccountState>,
}

/// Serializes all reads/writes so concurrent passes can't lose a quota update (a lost
/// update there would let the hard cap be exceeded). The whole file is small.
pub(crate) struct GcStore {
    path: PathBuf,
    lock: Mutex<()>,
}

fn store_path() -> Result<PathBuf, GcError> {
    let dir = dirs::data_dir()
        .ok_or_else(|| GcError::Store("no data dir".into()))?
        .join("deadlock-api-ingest");
    std::fs::create_dir_all(&dir).map_err(|e| GcError::Store(format!("mkdir failed: {e}")))?;
    Ok(dir.join(STORE_FILE))
}

impl GcStore {
    pub(crate) fn open() -> Result<Self, GcError> {
        Ok(Self {
            path: store_path()?,
            lock: Mutex::new(()),
        })
    }

    fn load(&self) -> StoreData {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                warn!("gc: corrupt quota store, starting fresh: {e}");
                StoreData::default()
            }),
            Err(_) => StoreData::default(),
        }
    }

    fn save(&self, data: &StoreData) -> Result<(), GcError> {
        let text = serde_json::to_string_pretty(data)
            .map_err(|e| GcError::Store(format!("serialize failed: {e}")))?;
        std::fs::write(&self.path, text).map_err(|e| GcError::Store(format!("write failed: {e}")))
    }

    pub(crate) fn load_quota(&self, steam_id64: u64) -> QuotaWindow {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let data = self.load();
        let hits = data
            .accounts
            .get(&steam_id64.to_string())
            .map(|a| a.quota_hits.clone())
            .unwrap_or_default();
        QuotaWindow::new(hits, FETCH_QUOTA_LIMIT, FETCH_QUOTA_WINDOW_SECS)
    }

    pub(crate) fn save_quota(&self, steam_id64: u64, quota: &QuotaWindow) -> Result<(), GcError> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut data = self.load();
        data.accounts
            .entry(steam_id64.to_string())
            .or_default()
            .quota_hits = quota.snapshot();
        self.save(&data)
    }

    pub(crate) fn load_backoff(&self, steam_id64: u64) -> Option<i64> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        self.load()
            .accounts
            .get(&steam_id64.to_string())
            .and_then(|a| a.gc_backoff_until)
    }

    pub(crate) fn set_backoff(&self, steam_id64: u64, until: Option<i64>) -> Result<(), GcError> {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut data = self.load();
        data.accounts
            .entry(steam_id64.to_string())
            .or_default()
            .gc_backoff_until = until;
        self.save(&data)
    }
}
