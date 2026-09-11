//! Persisted, per-account GC state: the rolling-window quota timestamps (so the
//! 40/24h cap survives restarts) and a GC "backoff until" for accounts whose
//! handshake is known-broken (e.g. they don't own the game). A single JSON file in
//! the app data dir, loaded and saved whole — the data is tiny.

use std::collections::HashMap;
use std::fs::{File, TryLockError};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::error::GcError;
use super::quota::{FETCH_QUOTA_LIMIT, FETCH_QUOTA_WINDOW_SECS, QuotaWindow};

const STORE_FILE: &str = "gc-quota.json";
const LOCK_FILE: &str = "gc-quota.lock";

pub(crate) fn now_secs() -> i64 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    i64::try_from(secs).unwrap_or(i64::MAX)
}

#[derive(Default, Serialize, Deserialize)]
struct AccountState {
    #[serde(default)]
    quota_hits: Vec<i64>,
    #[serde(default)]
    gc_backoff_until: Option<i64>,
}

#[derive(Default, Serialize, Deserialize)]
struct StoreData {
    #[serde(default)]
    accounts: HashMap<u64, AccountState>,
}

/// Exclusive owner of the store while alive: it holds an OS file lock, so two processes
/// (e.g. the service and a `--once` run) can't both spend the same account's quota.
pub(crate) struct GcStore {
    path: PathBuf,
    _lock: File,
}

fn store_err(what: &str, e: impl core::fmt::Display) -> GcError {
    GcError::Store(format!("{what}: {e}"))
}

impl GcStore {
    /// Fails if the data dir is unusable or another process currently holds the store.
    pub(crate) fn open() -> Result<Self, GcError> {
        let dir = dirs::data_dir()
            .ok_or_else(|| GcError::Store("no data dir".into()))?
            .join("deadlock-api-ingest");
        std::fs::create_dir_all(&dir).map_err(|e| store_err("mkdir failed", e))?;
        let lock = File::create(dir.join(LOCK_FILE)).map_err(|e| store_err("lock file", e))?;
        lock.try_lock().map_err(|e| match e {
            TryLockError::WouldBlock => GcError::Store("in use by another process".into()),
            TryLockError::Error(e) => store_err("lock failed", e),
        })?;
        Ok(Self {
            path: dir.join(STORE_FILE),
            _lock: lock,
        })
    }

    // Only a missing file means "fresh"; an unreadable one fails closed instead of
    // handing every account a full quota again.
    fn load(&self) -> Result<StoreData, GcError> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|e| store_err(&format!("corrupt {}", self.path.display()), e)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(StoreData::default()),
            Err(e) => Err(store_err("read failed", e)),
        }
    }

    // Write-then-rename so a crash mid-write can't leave a truncated file behind.
    fn save(&self, data: &StoreData) -> Result<(), GcError> {
        let text = serde_json::to_string_pretty(data).map_err(|e| store_err("serialize", e))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| store_err("write failed", e))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| store_err("rename failed", e))
    }

    fn account(&self, steam_id64: u64) -> Result<AccountState, GcError> {
        Ok(self
            .load()?
            .accounts
            .remove(&steam_id64)
            .unwrap_or_default())
    }

    fn update(&self, steam_id64: u64, f: impl FnOnce(&mut AccountState)) -> Result<(), GcError> {
        let mut data = self.load()?;
        f(data.accounts.entry(steam_id64).or_default());
        self.save(&data)
    }

    pub(crate) fn load_quota(&self, steam_id64: u64) -> Result<QuotaWindow, GcError> {
        let hits = self.account(steam_id64)?.quota_hits;
        Ok(QuotaWindow::new(
            hits,
            FETCH_QUOTA_LIMIT,
            FETCH_QUOTA_WINDOW_SECS,
        ))
    }

    pub(crate) fn save_quota(&self, steam_id64: u64, quota: &QuotaWindow) -> Result<(), GcError> {
        self.update(steam_id64, |a| a.quota_hits = quota.snapshot())
    }

    pub(crate) fn load_backoff(&self, steam_id64: u64) -> Result<Option<i64>, GcError> {
        Ok(self.account(steam_id64)?.gc_backoff_until)
    }

    pub(crate) fn set_backoff(&self, steam_id64: u64, until: Option<i64>) -> Result<(), GcError> {
        self.update(steam_id64, |a| a.gc_backoff_until = until)
    }
}
