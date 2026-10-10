//! The Steam Game Coordinator boundary: recover per-match salts client-side through a
//! remembered account (`steam-vent` + `CMsgClientToGcGetMatchMetaData`), plus the account's
//! friends list, their persona state and match histories. One CM+GC session per pass;
//! poisoned on any failure so the next pass reconnects cleanly.

use core::future::Future;
use core::time::Duration;
use std::collections::HashMap;

use futures_util::StreamExt as _;
use prost::Message as _;
use steam_vent::proto::steammessages_clientserver_friends::{
    CMsgClientFriendsList, CMsgClientPersonaState, CMsgClientRequestFriendData,
};
use steam_vent::proto::{MsgKind, RpcMessageWithKind as _};
use steam_vent::{
    Connection, ConnectionTrait, GameCoordinator, RawNetMessage, ServerList, UntypedMessage,
};
use tracing::{debug, warn};
use valveprotos::deadlock::{
    CMsgClientToGcGetMatchHistory, CMsgClientToGcGetMatchHistoryResponse,
    CMsgClientToGcGetMatchMetaData, CMsgClientToGcGetMatchMetaDataResponse,
    EgcCitadelClientMessages, c_msg_client_to_gc_get_match_history_response,
    c_msg_client_to_gc_get_match_meta_data_response::EResult,
};

use super::auth::AuthContext;
use super::error::GcError;
use super::friends::Persona;
use super::store::now_secs;
use crate::utils::Salts;

const DEADLOCK_APP_ID: u32 = 1422450;

// Steam pushes the friends list right after logon; it normally lands during the GC
// handshake, so this only bounds the wait when it never comes.
const FRIENDS_LIST_WAIT: Duration = Duration::from_secs(5);
// Persona states arrive as a burst of pushes, not a single reply.
const PERSONA_WAIT: Duration = Duration::from_secs(5);
const FRIEND_RELATIONSHIP_FRIEND: u32 = 3;
const PERSONA_FLAGS_STATUS_PRESENCE_LAST_SEEN: u32 = 1 | 16 | 64;
const STEAM_ID64_INDIVIDUAL_BASE: u64 = 76_561_197_960_265_728;

// A stalled discover/login/job call must not hang the pass indefinitely.
const GC_CALL_TIMEOUT: Duration = Duration::from_secs(30);

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
    conn: Connection,
    account_id: u32,
    /// Account ids of this account's Steam friends; empty if Steam never sent the list.
    pub(crate) friends: Vec<u32>,
}

/// Friend account ids from a friends list: confirmed friends that are individual accounts.
fn friend_account_ids(list: &CMsgClientFriendsList) -> Vec<u32> {
    list.friends
        .iter()
        .filter(|f| f.efriendrelationship == Some(FRIEND_RELATIONSHIP_FRIEND))
        .filter_map(|f| f.ulfriendid)
        // SteamID64 bits 52..56 hold the account type; 1 = individual.
        .filter(|id| (id >> 52) & 0xF == 1)
        .map(|id| (id & 0xFFFF_FFFF) as u32)
        .collect()
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
        // Listen before draining the unprocessed buffer, so the list can't slip between them.
        let friends_list = conn.one::<CMsgClientFriendsList>();
        let buffered = conn
            .take_unprocessed()
            .into_iter()
            .find(|m| m.kind == MsgKind::from(CMsgClientFriendsList::KIND));
        let gc = with_timeout("GC handshake", GameCoordinator::new(&conn, DEADLOCK_APP_ID)).await?;

        let list = match buffered {
            Some(raw) => raw.into_message::<CMsgClientFriendsList>().ok(),
            None => tokio::time::timeout(FRIENDS_LIST_WAIT, friends_list)
                .await
                .ok()
                .and_then(Result::ok),
        };
        let friends = list.as_ref().map(friend_account_ids).unwrap_or_default();
        if list.is_none() {
            warn!(
                "gc: Steam sent no friends list for account {}",
                ctx.account_id()
            );
        }
        Ok(Self {
            gc,
            conn,
            account_id: ctx.account_id(),
            friends,
        })
    }

    /// What Steam currently says about each of `account_ids` (who must be friends). Friends
    /// Steam doesn't answer for within [`PERSONA_WAIT`] are simply missing from the map.
    pub(crate) async fn request_personas(&self, account_ids: &[u32]) -> HashMap<u32, Persona> {
        let mut personas = HashMap::new();
        if account_ids.is_empty() {
            return personas;
        }
        let mut updates = Box::pin(self.conn.on::<CMsgClientPersonaState>());
        let mut req = CMsgClientRequestFriendData::new();
        req.persona_state_requested = Some(PERSONA_FLAGS_STATUS_PRESENCE_LAST_SEEN);
        req.friends = account_ids
            .iter()
            .map(|&id| STEAM_ID64_INDIVIDUAL_BASE + u64::from(id))
            .collect();
        if let Err(e) = with_timeout("persona request", self.conn.send(req)).await {
            warn!("gc: {e}");
            return personas;
        }

        let now = now_secs();
        let deadline = tokio::time::Instant::now() + PERSONA_WAIT;
        while personas.len() < account_ids.len() {
            let msg = match tokio::time::timeout_at(deadline, updates.next()).await {
                Ok(Some(Ok(msg))) => msg,
                Ok(Some(Err(e))) => {
                    debug!("gc: unreadable persona state: {e}");
                    continue;
                }
                Ok(None) | Err(_) => break,
            };
            for f in msg.friends {
                let Some(id) = f.friendid else { continue };
                let online = f.persona_state.is_some_and(|s| s != 0);
                let last_seen = if online {
                    Some(now)
                } else {
                    [f.last_seen_online, f.last_logoff]
                        .into_iter()
                        .flatten()
                        .filter(|&t| t != 0)
                        .max()
                        .map(i64::from)
                };
                personas.insert(
                    (id & 0xFFFF_FFFF) as u32,
                    Persona {
                        in_deadlock: f.game_played_app_id == Some(DEADLOCK_APP_ID),
                        last_seen,
                    },
                );
            }
        }
        personas
    }

    /// The match ids on the newest page of `account_id`'s history. One request, so it costs
    /// a single unit of the caller's history quota.
    pub(crate) async fn fetch_recent_matches(&self, account_id: u32) -> Result<Vec<u64>, GcError> {
        use c_msg_client_to_gc_get_match_history_response::EResult as HistoryResult;

        let kind = MsgKind(EgcCitadelClientMessages::KEMsgClientToGcGetMatchHistory as i32);
        let req = CMsgClientToGcGetMatchHistory {
            account_id: Some(account_id),
            ..Default::default()
        };
        let raw = self
            .send_job(req.encode_to_vec(), kind, "match history")
            .await?;
        let resp = CMsgClientToGcGetMatchHistoryResponse::decode(raw.data.as_ref())
            .map_err(|e| GcError::GcUnavailable(format!("bad match history response: {e}")))?;
        match resp.result {
            Some(r) if r == HistoryResult::KEResultSuccess as i32 => {}
            Some(r) if r == HistoryResult::KEResultRateLimited as i32 => {
                return Err(GcError::GcRateLimited);
            }
            Some(r) if r == HistoryResult::KEResultInvalidPermission as i32 => {
                return Err(GcError::HistoryPrivate);
            }
            r => {
                return Err(GcError::GcUnavailable(format!(
                    "match history result {r:?}"
                )));
            }
        }
        Ok(resp.matches.iter().filter_map(|m| m.match_id).collect())
    }

    async fn send_job(
        &self,
        req_bytes: Vec<u8>,
        kind: MsgKind,
        label: &str,
    ) -> Result<RawNetMessage, GcError> {
        with_timeout(
            label,
            self.gc.job_untyped(UntypedMessage(req_bytes), kind, true),
        )
        .await
    }

    /// One `GetMatchMetaData` round-trip, tagged with this session's account. No internal
    /// retries: a failed fetch must cost at most one quota unit (the caller rate-limits
    /// and quota-gates). A response without any salt is an error, so it is never posted.
    pub(crate) async fn fetch_match_salts(&self, match_id: u64) -> Result<Salts, GcError> {
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
            Some(r) if r == EResult::KEResultSuccess as i32 => {}
            r => return Err(GcError::GcUnavailable(format!("salts result {r:?}"))),
        }
        if resp.metadata_salt.is_none() && resp.replay_salt.is_none() {
            return Err(GcError::GcUnavailable("response has no salts".into()));
        }

        Ok(Salts {
            match_id,
            cluster_id: resp.replay_group_id,
            metadata_salt: resp.metadata_salt,
            replay_salt: resp.replay_salt,
            username: Some(self.account_id),
        })
    }

    /// Every match id in `account_id`'s history, following `continue_cursor` pages, and
    /// whether the history is complete: if Steam rate-limits mid-way, the pages fetched so
    /// far are returned with `false`.
    pub(crate) async fn fetch_match_history(
        &self,
        account_id: u32,
    ) -> Result<(Vec<u64>, bool), GcError> {
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
                    return Ok((match_ids, false));
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
        Ok((match_ids, true))
    }
}
