//! The deadlock-api calls this subsystem makes: read the global to-fetch list, check which
//! matches are already known, and POST recovered salts to the same route the httpcache
//! scraper uses. Blocking `ureq`, matching the rest of the crate; called from
//! `spawn_blocking` inside the async pass.

use core::time::Duration;
use std::collections::HashSet;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use tracing::debug;

use super::client::MatchSalts;
use super::error::GcError;

const TO_FETCH_URL: &str = "https://api.deadlock-api.com/v1/matches/to-fetch";
const SALTS_URL: &str = "https://api.deadlock-api.com/v1/matches/salts";
const METADATA_URL: &str = "https://api.deadlock-api.com/v1/matches/metadata";
const METADATA_CHUNK: usize = 100;
const POST_MAX_RETRIES: u32 = 5;
const POST_RETRY_DELAY: Duration = Duration::from_secs(3);

static HTTP_CLIENT: OnceLock<ureq::Agent> = OnceLock::new();

fn client() -> &'static ureq::Agent {
    HTTP_CLIENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .new_agent()
    })
}

// One element of the `POST /v1/matches/salts` body. `username` is serialized as
// `ingest-tool:{steam_id3}`, matching the httpcache scraper's tagging.
#[derive(Serialize)]
struct SaltPayload {
    match_id: u64,
    cluster_id: Option<u32>,
    metadata_salt: Option<u32>,
    replay_salt: Option<u32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_username"
    )]
    username: Option<u32>,
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::ref_option)]
fn serialize_username<S: serde::Serializer>(
    value: &Option<u32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(id) => serializer.serialize_str(&format!("ingest-tool:{id}")),
        None => serializer.serialize_none(),
    }
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
        let rows: Vec<MatchIdRow> = client()
            .get(METADATA_URL)
            .query("match_ids", &ids)
            .query("include_info", "true")
            .query("limit", METADATA_CHUNK.to_string())
            .call()
            .map_err(|e| GcError::Api(format!("metadata request failed: {e}")))?
            .body_mut()
            .read_json()
            .map_err(|e| GcError::Api(format!("invalid metadata response: {e}")))?;
        known.extend(rows.into_iter().map(|r| r.match_id));
    }
    Ok(known)
}

/// POST a single match's salts, retrying transient failures. A 400 means the payload
/// itself is wrong, so it returns immediately without retrying.
pub(crate) fn post_salts(salts: &MatchSalts, username: Option<u32>) -> Result<(), GcError> {
    let payload = [SaltPayload {
        match_id: salts.match_id,
        cluster_id: salts.cluster_id,
        metadata_salt: salts.metadata_salt,
        replay_salt: salts.replay_salt,
        username,
    }];

    let mut attempt = 0;
    loop {
        attempt += 1;
        debug!(
            "gc: posting salts for match {} (attempt {attempt}/{POST_MAX_RETRIES})",
            salts.match_id
        );
        match client().post(SALTS_URL).send_json(&payload) {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(r) => {
                let status = r.status();
                if attempt >= POST_MAX_RETRIES {
                    return Err(GcError::Api(format!("salts POST returned {status}")));
                }
            }
            Err(ureq::Error::StatusCode(400)) => {
                return Err(GcError::Api("salts POST rejected (400)".into()));
            }
            Err(e) if attempt >= POST_MAX_RETRIES => {
                return Err(GcError::Api(format!("salts POST failed: {e}")));
            }
            Err(_) => {}
        }
        std::thread::sleep(POST_RETRY_DELAY);
    }
}
