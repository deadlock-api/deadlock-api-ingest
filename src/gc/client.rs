//! The Steam Game Coordinator boundary: recover per-match salts client-side through a
//! remembered account (`steam-vent` + `CMsgClientToGcGetMatchMetaData`). One CM+GC
//! session per pass; poisoned on any failure so the next pass reconnects cleanly.

// steam-vent's CM/GC connection futures are inherently large; boxing them would only
// add allocations to a background daemon that awaits one at a time.
#![allow(clippy::large_futures)]

use core::future::Future;
use core::time::Duration;

use prost::Message as _;
use steam_vent::proto::MsgKind;
use steam_vent::{
    Connection, ConnectionTrait, GameCoordinator, RawNetMessage, ServerList, UntypedMessage,
};
use tracing::warn;
use valveprotos::deadlock::{
    CMsgClientToGcGetMatchHistory, CMsgClientToGcGetMatchHistoryResponse,
    CMsgClientToGcGetMatchMetaData, CMsgClientToGcGetMatchMetaDataResponse,
    EgcCitadelClientMessages, c_msg_client_to_gc_get_match_history_response,
    c_msg_client_to_gc_get_match_meta_data_response::EResult,
};

use super::auth::AuthContext;
use super::error::GcError;

const DEADLOCK_APP_ID: u32 = 1422450;

// A stalled discover/login/job call must not hang the pass indefinitely.
const GC_CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The salts recovered for a single match (the same fields the httpcache scraper posts).
pub(crate) struct MatchSalts {
    pub(crate) match_id: u64,
    pub(crate) cluster_id: Option<u32>,
    pub(crate) metadata_salt: Option<u32>,
    pub(crate) replay_salt: Option<u32>,
}

async fn with_timeout<T, E: core::fmt::Display>(
    label: &str,
    fut: impl Future<Output = Result<T, E>>,
) -> Result<T, GcError> {
    match tokio::time::timeout(GC_CALL_TIMEOUT, fut).await {
        Ok(result) => result.map_err(|e| GcError::GcUnavailable(format!("{label}: {e}"))),
        Err(_) => Err(GcError::GcUnavailable(format!(
            "{label}: timed out after {GC_CALL_TIMEOUT:?}"
        ))),
    }
}

/// A live GC session bound to one account. Dropped between passes.
pub(crate) struct GcSession {
    gc: GameCoordinator,
    // Held only to keep the CM connection alive for the GC session.
    _conn: Connection,
}

impl GcSession {
    /// Log in through the user's own account and hand-shake the Deadlock GC. Fails
    /// (not panics) if the account doesn't own the game or Steam is unreachable.
    pub(crate) async fn connect(ctx: &AuthContext) -> Result<Self, GcError> {
        let servers = with_timeout("server discovery", ServerList::discover()).await?;
        let conn = with_timeout(
            "CM login",
            Connection::access(&servers, &ctx.account_name, &ctx.refresh_token),
        )
        .await?;
        let gc = with_timeout("GC handshake", GameCoordinator::new(&conn, DEADLOCK_APP_ID)).await?;
        Ok(Self { gc, _conn: conn })
    }

    async fn send_job(
        &self,
        req_bytes: Vec<u8>,
        kind: MsgKind,
        label: &str,
    ) -> Result<RawNetMessage, GcError> {
        match tokio::time::timeout(
            GC_CALL_TIMEOUT,
            self.gc.job_untyped(UntypedMessage(req_bytes), kind, true),
        )
        .await
        {
            Ok(Ok(raw)) => Ok(raw),
            Ok(Err(e)) => Err(GcError::GcUnavailable(format!(
                "{label} request failed: {e}"
            ))),
            Err(_) => Err(GcError::GcUnavailable(format!(
                "{label} request timed out after {GC_CALL_TIMEOUT:?}"
            ))),
        }
    }

    /// One `GetMatchMetaData` round-trip. No internal retries: a failed fetch must cost
    /// at most one quota unit (the caller rate-limits and quota-gates).
    pub(crate) async fn fetch_match_salts(&self, match_id: u64) -> Result<MatchSalts, GcError> {
        let req = CMsgClientToGcGetMatchMetaData {
            match_id: Some(match_id),
            ..Default::default()
        };
        let kind = MsgKind(EgcCitadelClientMessages::KEMsgClientToGcGetMatchMetaData as i32);
        let raw = self.send_job(req.encode_to_vec(), kind, "salts").await?;

        let resp = CMsgClientToGcGetMatchMetaDataResponse::decode(raw.data.as_ref())
            .map_err(|e| GcError::GcUnavailable(format!("bad salts response: {e}")))?;

        match resp.result {
            Some(r) if r == EResult::KEResultRateLimited as i32 => {
                return Err(GcError::GcRateLimited);
            }
            Some(r) if r != EResult::KEResultSuccess as i32 => {
                return Err(GcError::GcUnavailable(format!("salts result {r}")));
            }
            _ => {}
        }

        if resp.metadata_salt.is_none() && resp.replay_salt.is_none() {
            warn!("gc: match {match_id} returned no salts");
        }

        Ok(MatchSalts {
            match_id,
            cluster_id: resp.replay_group_id,
            metadata_salt: resp.metadata_salt,
            replay_salt: resp.replay_salt,
        })
    }

    /// Every match id in `account_id`'s history, following `continue_cursor` pages. If
    /// Steam rate-limits mid-way, the pages fetched so far are returned.
    pub(crate) async fn fetch_match_history(&self, account_id: u32) -> Result<Vec<u64>, GcError> {
        use c_msg_client_to_gc_get_match_history_response::EResult as HistoryResult;

        let kind = MsgKind(EgcCitadelClientMessages::KEMsgClientToGcGetMatchHistory as i32);
        let mut match_ids = Vec::new();
        let mut cursor: Option<u64> = None;
        loop {
            let req = CMsgClientToGcGetMatchHistory {
                account_id: Some(account_id),
                continue_cursor: cursor,
                ..Default::default()
            };
            let raw = self
                .send_job(req.encode_to_vec(), kind, "match history")
                .await?;
            let resp = CMsgClientToGcGetMatchHistoryResponse::decode(raw.data.as_ref())
                .map_err(|e| GcError::GcUnavailable(format!("bad match history response: {e}")))?;

            match resp.result {
                Some(r) if r == HistoryResult::KEResultRateLimited as i32 => {
                    if match_ids.is_empty() {
                        return Err(GcError::GcRateLimited);
                    }
                    warn!(
                        "gc: match history rate-limited, continuing with {} match(es)",
                        match_ids.len()
                    );
                    break;
                }
                Some(r) if r == HistoryResult::KEResultSuccess as i32 => {}
                r => {
                    return Err(GcError::GcUnavailable(format!(
                        "match history result {r:?}"
                    )));
                }
            }

            if resp.matches.is_empty() {
                break;
            }
            match_ids.extend(resp.matches.iter().filter_map(|m| m.match_id));

            // The cursor walks backwards in time; a missing, zero or non-decreasing cursor
            // means there are no older pages.
            match resp.continue_cursor {
                Some(next) if next != 0 && cursor.is_none_or(|prev| next < prev) => {
                    cursor = Some(next);
                }
                _ => break,
            }
        }
        Ok(match_ids)
    }
}
