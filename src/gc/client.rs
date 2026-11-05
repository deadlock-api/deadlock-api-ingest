//! The Steam Game Coordinator boundary: recover per-match salts client-side through a
//! remembered account (`steam-vent` + `CMsgClientToGcGetMatchMetaData`), plus the account's
//! friends list, their persona state and match histories. One CM session per account per
//! pass, which only reports the account as playing Deadlock during each GC request.

use core::future::Future;
use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::{FutureExt as _, StreamExt as _};
use prost::Message as _;
use steam_vent::connection::UnAuthenticatedConnection;
use steam_vent::proto::MsgKind;
use steam_vent::proto::enums_clientserver::EMsg;
use steam_vent::proto::steammessages_clientserver::CMsgClientGamesPlayed;
use steam_vent::proto::steammessages_clientserver_2::CMsgClientPlayingSessionState;
use steam_vent::proto::steammessages_clientserver_friends::{
    CMsgClientFriendsList, CMsgClientPersonaState, CMsgClientRequestFriendData,
};
use steam_vent::proto::steammessages_clientserver_login::CMsgClientLoggedOff;
use steam_vent::{
    Connection, ConnectionError, ConnectionTrait, EResult as SteamResult, GameCoordinator,
    NetworkError, RawNetMessage, ReadonlyConnection as _, ServerList, UntypedMessage,
};
use tracing::{debug, warn};
use valveprotos::deadlock::{
    CMsgClientToGcGetMatchHistory, CMsgClientToGcGetMatchHistoryResponse,
    CMsgClientToGcGetMatchMetaData, CMsgClientToGcGetMatchMetaDataResponse,
    EgcCitadelClientMessages, c_msg_client_to_gc_get_match_history_response,
    c_msg_client_to_gc_get_match_meta_data_response::EResult,
};

use super::auth::{AuthContext, account_id_of};
use super::error::GcError;
use super::friends::Persona;
use super::store::now_secs;
use crate::steam_user::STEAM_ID_64_IDENT;
use crate::utils::Salts;

const DEADLOCK_APP_ID: u32 = 1422450;

// Steam pushes the friends list and the playing state right after logon, so this only
// bounds the wait when they never come.
const FRIENDS_LIST_WAIT: Duration = Duration::from_secs(5);
const PLAYING_STATE_WAIT: Duration = Duration::from_secs(5);
// Persona states arrive as a burst of pushes, not a single reply.
const PERSONA_WAIT: Duration = Duration::from_secs(5);
const FRIEND_RELATIONSHIP_FRIEND: u32 = 3;
const PERSONA_FLAGS_STATUS_PRESENCE_LAST_SEEN: u32 = 1 | 16 | 64;

// A stalled discover/login/job call must not hang the pass indefinitely.
const GC_CALL_TIMEOUT: Duration = Duration::from_secs(30);
// Runs after every GC request, so a stalled send must not hold up the pass.
const STOP_PLAYING_TIMEOUT: Duration = Duration::from_secs(2);

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

type LoggedOff = BoxFuture<'static, Result<CMsgClientLoggedOff, NetworkError>>;
// Steam pushes one right after logon and another whenever a session of the account starts
// or stops a game.
type PlayingStates = BoxStream<'static, Result<CMsgClientPlayingSessionState, NetworkError>>;

/// A logged-on CM session bound to one account. It only reports the account as playing
/// Deadlock during a GC request ([`Self::send_job`]): while it does, launching the game
/// brings up Steam's "playing on another computer" prompt.
pub(crate) struct GcSession {
    conn: Connection,
    account_id: u32,
    /// Account ids of this account's Steam friends; empty if Steam never sent the list.
    pub(crate) friends: Vec<u32>,
    logged_off: Mutex<Logoff>,
    playing: Mutex<PlayingState>,
}

/// Steam's logoff message, polled only until it arrives: polling it again would panic.
enum Logoff {
    Pending(LoggedOff),
    InUseElsewhere,
    Other,
}

struct PlayingState {
    updates: PlayingStates,
    blocked: bool,
}

/// Friend account ids from a friends list: confirmed friends that are individual accounts.
fn friend_account_ids(list: &CMsgClientFriendsList) -> Vec<u32> {
    list.friends
        .iter()
        .filter(|f| f.efriendrelationship == Some(FRIEND_RELATIONSHIP_FRIEND))
        .filter_map(|f| f.ulfriendid)
        // SteamID64 bits 52..56 hold the account type; 1 = individual.
        .filter(|id| (id >> 52) & 0xF == 1)
        .map(account_id_of)
        .collect()
}

impl GcSession {
    /// Log in through the user's own account. Fails (not panics) if Steam is unreachable,
    /// or with `AccountInUse` if another session of the account is playing a game.
    pub(crate) async fn connect(ctx: &AuthContext) -> Result<Self, GcError> {
        let servers = with_timeout("server discovery", ServerList::discover()).await?;
        // Listen before logging in: Steam pushes these right after logon, and steam-vent
        // keeps only the last few messages nobody listens for.
        let (conn, friends_list, mut playing_states, logged_off) =
            with_timeout("CM login", async {
                let unauthenticated = UnAuthenticatedConnection::connect(&servers).await?;
                let friends_list = unauthenticated.one::<CMsgClientFriendsList>().boxed();
                let playing_states = unauthenticated
                    .on::<CMsgClientPlayingSessionState>()
                    .boxed();
                let logged_off = unauthenticated.one::<CMsgClientLoggedOff>().boxed();
                let conn = unauthenticated
                    .access(&ctx.account_name, &ctx.refresh_token)
                    .await?;
                Ok::<_, ConnectionError>((conn, friends_list, playing_states, logged_off))
            })
            .await?;

        // Starting Deadlock while another session plays a game would interrupt it.
        let state = tokio::time::timeout(PLAYING_STATE_WAIT, playing_states.next()).await;
        let blocked = if let Ok(Some(Ok(state))) = state {
            state.playing_blocked()
        } else {
            debug!("gc: Steam sent no playing state after logon");
            false
        };
        if blocked {
            return Err(GcError::AccountInUse);
        }

        let list = tokio::time::timeout(FRIENDS_LIST_WAIT, friends_list)
            .await
            .ok()
            .and_then(Result::ok);
        let friends = list.as_ref().map(friend_account_ids).unwrap_or_default();
        if list.is_none() {
            warn!(
                "gc: Steam sent no friends list for account {}",
                ctx.account_id()
            );
        }
        Ok(Self {
            conn,
            account_id: ctx.account_id(),
            friends,
            logged_off: Mutex::new(Logoff::Pending(logged_off)),
            playing: Mutex::new(PlayingState {
                updates: playing_states,
                blocked: false,
            }),
        })
    }

    /// Whether another session of the account has started a game since the last check.
    fn playing_blocked(&self) -> bool {
        let mut playing = self.playing.lock().unwrap_or_else(PoisonError::into_inner);
        while let Some(Some(state)) = playing.updates.next().now_or_never() {
            if let Ok(state) = state {
                playing.blocked = state.playing_blocked();
            }
        }
        playing.blocked
    }

    /// `AccountInUse` instead of `error` if Steam logged this session off because another
    /// session of the account started a game.
    fn logged_off_or(&self, error: GcError) -> GcError {
        let mut logged_off = self
            .logged_off
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Logoff::Pending(pending) = &mut *logged_off
            && let Some(msg) = pending.as_mut().now_or_never()
        {
            *logged_off = match msg {
                Ok(msg) if msg.eresult() == SteamResult::LoggedInElsewhere as i32 => {
                    Logoff::InUseElsewhere
                }
                _ => Logoff::Other,
            };
        }
        match *logged_off {
            Logoff::InUseElsewhere => GcError::AccountInUse,
            Logoff::Pending(_) | Logoff::Other => error,
        }
    }

    /// Stop reporting the account as playing Deadlock. Clears the game instead of logging
    /// off: the session shares the Steam client's login, and logging off would sign the
    /// client out too.
    async fn clear_playing(&self) {
        let sent = tokio::time::timeout(
            STOP_PLAYING_TIMEOUT,
            self.conn.send_with_kind(
                CMsgClientGamesPlayed::default(),
                EMsg::k_EMsgClientGamesPlayedWithDataBlob,
            ),
        )
        .await;
        match sent {
            Ok(Ok(())) => {}
            Ok(Err(e)) => debug!("gc: clearing games played failed: {e}"),
            Err(_) => debug!("gc: clearing games played timed out after {STOP_PLAYING_TIMEOUT:?}"),
        }
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
            .map(|&id| STEAM_ID_64_IDENT + u64::from(id))
            .collect();
        if let Err(e) = with_timeout("persona request", self.conn.send(req)).await {
            warn!("gc: {e}");
            return personas;
        }

        // Steam also pushes updates for friends we didn't ask about; only the requested
        // ones count towards being done.
        let mut pending: HashSet<u32> = account_ids.iter().copied().collect();
        let now = now_secs();
        let deadline = tokio::time::Instant::now() + PERSONA_WAIT;
        while !pending.is_empty() {
            let msg = match tokio::time::timeout_at(deadline, updates.next()).await {
                Ok(Some(Ok(msg))) => msg,
                Ok(Some(Err(e))) => {
                    debug!("gc: unreadable persona state: {e}");
                    continue;
                }
                Ok(None) | Err(_) => break,
            };
            for f in msg.friends {
                let Some(id) = f.friendid.map(account_id_of) else {
                    continue;
                };
                if !pending.remove(&id) {
                    continue;
                }
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
                    id,
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
        let page = self.history_page(account_id, None).await?;
        Ok(page.matches.iter().filter_map(|m| m.match_id).collect())
    }

    /// One `GetMatchHistory` page, older than `cursor` if given. Non-success results are
    /// errors: rate limits as `GcRateLimited`, private histories as `HistoryPrivate`.
    async fn history_page(
        &self,
        account_id: u32,
        cursor: Option<u64>,
    ) -> Result<CMsgClientToGcGetMatchHistoryResponse, GcError> {
        use c_msg_client_to_gc_get_match_history_response::EResult as HistoryResult;

        let kind = MsgKind(EgcCitadelClientMessages::KEMsgClientToGcGetMatchHistory as i32);
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
        let err = match resp.result {
            Some(r) if r == HistoryResult::KEResultSuccess as i32 => return Ok(resp),
            Some(r) if r == HistoryResult::KEResultRateLimited as i32 => GcError::GcRateLimited,
            Some(r) if r == HistoryResult::KEResultInvalidPermission as i32 => {
                GcError::HistoryPrivate
            }
            r => GcError::GcUnavailable(format!("match history result {r:?}")),
        };
        Err(err)
    }

    /// One GC request. The GC only serves a session that reports the account as playing
    /// Deadlock, so this hand-shakes right before the request and stops playing right
    /// after it, instead of playing for the whole pass.
    async fn send_job(
        &self,
        req_bytes: Vec<u8>,
        kind: MsgKind,
        label: &str,
    ) -> Result<RawNetMessage, GcError> {
        if self.playing_blocked() {
            return Err(GcError::AccountInUse);
        }
        let result = async {
            let gc = with_timeout(
                "GC handshake",
                GameCoordinator::new(&self.conn, DEADLOCK_APP_ID),
            )
            .await?;
            with_timeout(label, gc.job_untyped(UntypedMessage(req_bytes), kind, true)).await
        }
        .await;
        // Also after a failure: the handshake may have started playing.
        self.clear_playing().await;
        result.map_err(|e| self.logged_off_or(e))
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
        Salts::from_metadata_reply(match_id, &resp, Some(self.account_id))
            .ok_or_else(|| GcError::GcUnavailable("response has no salts".into()))
    }

    /// Every match id in `account_id`'s history, following `continue_cursor` pages, and
    /// whether the history is complete: if Steam rate-limits mid-way, the pages fetched so
    /// far are returned with `false`.
    pub(crate) async fn fetch_match_history(
        &self,
        account_id: u32,
    ) -> Result<(Vec<u64>, bool), GcError> {
        let mut match_ids = Vec::new();
        let mut cursor: Option<u64> = None;
        loop {
            let resp = match self.history_page(account_id, cursor).await {
                Ok(resp) => resp,
                Err(GcError::GcRateLimited) if !match_ids.is_empty() => {
                    warn!(
                        "gc: match history rate-limited, continuing with {} match(es)",
                        match_ids.len()
                    );
                    return Ok((match_ids, false));
                }
                Err(e) => return Err(e),
            };

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
