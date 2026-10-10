//! The deadlock-api reads this subsystem makes: the global to-fetch list, which matches
//! are already known, and the stored activity of the user's friends. Salts are posted via [`crate::utils::Salts::ingest`]. Blocking
//! `ureq`, matching the rest of the crate; called from `spawn_blocking` inside the async pass.

use core::time::Duration;
use std::collections::HashSet;
use std::sync::OnceLock;

use serde::Deserialize;

use super::error::GcError;

const TO_FETCH_URL: &str = "https://api.deadlock-api.com/v1/matches/to-fetch";
const METADATA_URL: &str = "https://api.deadlock-api.com/v1/matches/metadata";
const METADATA_CHUNK: usize = 100;
const PLAYERS_STEAM_URL: &str = "https://api.deadlock-api.com/v1/players/steam";
const PLAYERS_STEAM_CHUNK: usize = 1000;

static HTTP_CLIENT: OnceLock<ureq::Agent> = OnceLock::new();

fn client() -> &'static ureq::Agent {
    HTTP_CLIENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .new_agent()
    })
}

/// The global "missing salts" list. Best-effort: any HTTP/parse failure is an error the
/// caller logs and treats as "nothing to do this pass".
pub(crate) fn to_fetch() -> Result<Vec<u64>, GcError> {
    let mut resp = client()
        .get(TO_FETCH_URL)
        .call()
        .map_err(|e| GcError::Api(format!("to-fetch request failed: {e}")))?;
    resp.body_mut()
        .read_json::<Vec<u64>>()
        .map_err(|e| GcError::Api(format!("invalid to-fetch response: {e}")))
}

#[derive(Deserialize)]
struct MatchIdRow {
    match_id: u64,
}

/// The subset of `match_ids` deadlock-api already has metadata for (and therefore salts).
pub(crate) fn known_match_ids(match_ids: &[u64]) -> Result<HashSet<u64>, GcError> {
    let mut known = HashSet::new();
    for chunk in match_ids.chunks(METADATA_CHUNK) {
        let ids = chunk
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let mut resp = match client()
            .get(METADATA_URL)
            .query("match_ids", &ids)
            .query("include_info", "true")
            .query("limit", METADATA_CHUNK.to_string())
            .call()
        {
            Ok(resp) => resp,
            // The endpoint answers 404 when none of the requested matches are known.
            Err(ureq::Error::StatusCode(404)) => continue,
            Err(e) => return Err(GcError::Api(format!("metadata request failed: {e}"))),
        };
        let rows: Vec<MatchIdRow> = resp
            .body_mut()
            .read_json()
            .map_err(|e| GcError::Api(format!("invalid metadata response: {e}")))?;
        known.extend(rows.into_iter().map(|r| r.match_id));
    }
    Ok(known)
}

#[derive(Deserialize)]
struct SteamProfileRow {
    account_id: u32,
    #[serde(default)]
    matches_played_last_30d: Option<u64>,
}

/// Of `account_ids`, the ones deadlock-api has seen play Deadlock in the last 30 days.
pub(crate) fn recent_players(account_ids: &[u32]) -> Result<HashSet<u32>, GcError> {
    let mut players = HashSet::new();
    for chunk in account_ids.chunks(PLAYERS_STEAM_CHUNK) {
        let ids = chunk
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let rows: Vec<SteamProfileRow> = client()
            .get(PLAYERS_STEAM_URL)
            .query("account_ids", &ids)
            .call()
            .map_err(|e| GcError::Api(format!("players/steam request failed: {e}")))?
            .body_mut()
            .read_json()
            .map_err(|e| GcError::Api(format!("invalid players/steam response: {e}")))?;
        players.extend(
            rows.into_iter()
                .filter(|r| r.matches_played_last_30d.is_some_and(|n| n > 0))
                .map(|r| r.account_id),
        );
    }
    Ok(players)
}

#[derive(Deserialize)]
pub(crate) struct HistoryRow {
    pub(crate) match_id: u64,
    pub(crate) start_time: u32,
    #[serde(default)]
    pub(crate) match_duration_s: u32,
}

/// The match history deadlock-api has stored for `account_id`, newest first. Without
/// `force_refetch` this is a plain database read for accounts that aren't bot friends.
pub(crate) fn stored_match_history(account_id: u32) -> Result<Vec<HistoryRow>, GcError> {
    client()
        .get(format!(
            "https://api.deadlock-api.com/v1/players/{account_id}/match-history"
        ))
        .call()
        .map_err(|e| GcError::Api(format!("match-history request failed: {e}")))?
        .body_mut()
        .read_json()
        .map_err(|e| GcError::Api(format!("invalid match-history response: {e}")))
}
