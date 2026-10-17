//! The deadlock-api reads this subsystem makes: the global to-fetch list, which matches
//! are already known, and the stored activity of the user's friends. Salts are posted via [`crate::utils::Salts::ingest`]. Blocking
//! `ureq`, matching the rest of the crate; called from `spawn_blocking` inside the async pass.

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use tracing::warn;
use ureq::http::header::{COOKIE, SET_COOKIE};
use ureq::http::{HeaderMap, Response};

use super::error::GcError;

const TO_FETCH_URL: &str = "https://api.deadlock-api.com/v1/matches/to-fetch";
const TO_FETCH_RELEASE_URL: &str = "https://api.deadlock-api.com/v1/matches/to-fetch/release";
const CLAIM_COOKIE: &str = "to_fetch_claim";
const METADATA_URL: &str = "https://api.deadlock-api.com/v1/matches/metadata";
const METADATA_CHUNK: usize = 100;
const PLAYERS_STEAM_URL: &str = "https://api.deadlock-api.com/v1/players/steam";
const PLAYERS_STEAM_CHUNK: usize = 1000;

use crate::utils::http_client as client;

/// Send `req` and parse its JSON body; `what` names the endpoint in errors. deadlock-api
/// answers 404 for queries that match nothing, so that reads as an empty `T`.
fn get_json<T: DeserializeOwned + Default>(
    req: ureq::RequestBuilder<ureq::typestate::WithoutBody>,
    what: &str,
) -> Result<T, GcError> {
    match req.call() {
        Ok(mut resp) => read_json(&mut resp, what),
        Err(ureq::Error::StatusCode(404)) => Ok(T::default()),
        Err(e) => Err(GcError::Api(format!("{what} request failed: {e}"))),
    }
}

fn read_json<T: DeserializeOwned>(
    resp: &mut Response<ureq::Body>,
    what: &str,
) -> Result<T, GcError> {
    resp.body_mut()
        .read_json()
        .map_err(|e| GcError::Api(format!("invalid {what} response: {e}")))
}

fn join_ids(ids: &[impl ToString]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// The `to_fetch_claim` cookie of the last to-fetch list. deadlock-api hides a list's
/// matches from other workers for 15 min; sending the cookie back hands them back early.
static CLAIM: Mutex<Option<String>> = Mutex::new(None);

fn held_claim() -> MutexGuard<'static, Option<String>> {
    CLAIM.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The value of a `to_fetch_claim` `Set-Cookie` header, if `header` is one. An empty
/// value is the server clearing the cookie.
fn parse_claim(header: &str) -> Option<&str> {
    let pair = header.split(';').next()?;
    let (name, value) = pair.split_once('=')?;
    let value = value.trim();
    (name.trim() == CLAIM_COOKIE && !value.is_empty()).then_some(value)
}

fn claim_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(parse_claim)
        .map(str::to_owned)
}

/// The global "missing salts" list. Best-effort: any HTTP/parse failure is an error the
/// caller logs and treats as "nothing to do this pass". Sends the previous list's claim
/// cookie, so the server releases that list before claiming this one.
pub(crate) fn to_fetch() -> Result<Vec<u64>, GcError> {
    let mut req = client().get(TO_FETCH_URL);
    let held = held_claim().clone();
    if let Some(claim) = held {
        req = req.header(COOKIE, format!("{CLAIM_COOKIE}={claim}"));
    }
    let mut resp = match req.call() {
        Ok(resp) => resp,
        // Nothing to fetch, so nothing claimed.
        Err(ureq::Error::StatusCode(404)) => {
            *held_claim() = None;
            return Ok(Vec::new());
        }
        // Keep the held claim: releasing it later is harmless even if the server got this.
        Err(e) => return Err(GcError::Api(format!("to-fetch request failed: {e}"))),
    };
    *held_claim() = claim_from_headers(resp.headers());
    read_json(&mut resp, "to-fetch")
}

/// Release the matches the last to-fetch list claimed, once this worker won't fetch any
/// more of them. Best-effort: a failure is only logged and the claim expires on its own.
pub(crate) fn release_to_fetch() {
    let Some(claim) = held_claim().take() else {
        return;
    };
    let req = client()
        .post(TO_FETCH_RELEASE_URL)
        .header(COOKIE, format!("{CLAIM_COOKIE}={claim}"));
    if let Err(e) = req.send_empty() {
        warn!("gc: to-fetch release failed: {e}");
    }
}

#[derive(Deserialize)]
struct MatchIdRow {
    match_id: u64,
}

/// The subset of `match_ids` deadlock-api already has metadata for (and therefore salts).
pub(crate) fn known_match_ids(match_ids: &[u64]) -> Result<HashSet<u64>, GcError> {
    let mut known = HashSet::new();
    for chunk in match_ids.chunks(METADATA_CHUNK) {
        let req = client()
            .get(METADATA_URL)
            .query("match_ids", join_ids(chunk))
            .query("include_info", "true")
            .query("limit", METADATA_CHUNK.to_string());
        let rows: Vec<MatchIdRow> = get_json(req, "metadata")?;
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
        let req = client()
            .get(PLAYERS_STEAM_URL)
            .query("account_ids", join_ids(chunk));
        let rows: Vec<SteamProfileRow> = get_json(req, "players/steam")?;
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
    let url = format!("https://api.deadlock-api.com/v1/players/{account_id}/match-history");
    get_json(client().get(url), "match-history")
}

#[cfg(test)]
mod tests {
    use super::*;

    use ureq::http::HeaderValue;

    #[test]
    fn parses_claim_cookie_value() {
        let header = "to_fetch_claim=0b6f9c1e-2d4a-4c1b-9a51-6f3e8d2c7b10; Path=/v1/matches/to-fetch; Max-Age=900; HttpOnly; Secure; SameSite=Strict";
        assert_eq!(
            parse_claim(header),
            Some("0b6f9c1e-2d4a-4c1b-9a51-6f3e8d2c7b10")
        );
        assert_eq!(parse_claim("to_fetch_claim=abc"), Some("abc"));
    }

    #[test]
    fn ignores_other_and_cleared_cookies() {
        assert_eq!(parse_claim("session=abc; Path=/"), None);
        assert_eq!(parse_claim("to_fetch_claim_old=abc"), None);
        assert_eq!(parse_claim("to_fetch_claim=; Max-Age=0"), None);
        assert_eq!(parse_claim("garbage"), None);
    }

    #[test]
    fn finds_claim_among_several_set_cookies() {
        let mut headers = HeaderMap::new();
        headers.append(SET_COOKIE, HeaderValue::from_static("other=1; Path=/"));
        headers.append(
            SET_COOKIE,
            HeaderValue::from_static("to_fetch_claim=abc; Max-Age=900"),
        );
        assert_eq!(claim_from_headers(&headers).as_deref(), Some("abc"));
        assert_eq!(claim_from_headers(&HeaderMap::new()), None);
    }
}
