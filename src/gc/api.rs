//! The deadlock-api reads this subsystem makes: the global to-fetch list and which matches
//! are already known. Salts are posted via [`crate::utils::Salts::ingest`]. Blocking
//! `ureq`, matching the rest of the crate; called from `spawn_blocking` inside the async pass.

use core::time::Duration;
use std::collections::HashSet;
use std::sync::OnceLock;

use serde::Deserialize;

use super::error::GcError;

const TO_FETCH_URL: &str = "https://api.deadlock-api.com/v1/matches/to-fetch";
const METADATA_URL: &str = "https://api.deadlock-api.com/v1/matches/metadata";
const METADATA_CHUNK: usize = 100;

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
