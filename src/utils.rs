use crate::error::Error;
use crate::ingestion_cache::{self, Source};
use core::time::Duration;
use serde::Serialize;
use std::sync::OnceLock;
use std::thread::sleep;
use tracing::{debug, warn};
use ureq::http::StatusCode;
use valveprotos::deadlock::CMsgClientToGcGetMatchMetaDataResponse;

/// The API rejects larger match ids.
pub(crate) const MAX_MATCH_ID: u64 = 10_000_000_000;

/// The API takes at most this many salts per request.
const MAX_SALTS_PER_REQUEST: usize = 1000;

static TIMEOUT_CLIENT: OnceLock<ureq::Agent> = OnceLock::new();

/// Shared agent for requests that must not hang (salt uploads, GC API lookups, update checks).
pub(crate) fn http_client() -> &'static ureq::Agent {
    TIMEOUT_CLIENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .new_agent()
    })
}

pub(crate) fn now_secs() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    i64::try_from(secs).unwrap_or(i64::MAX)
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Salts {
    pub(super) match_id: u64,
    pub(super) cluster_id: Option<u32>,
    pub(super) metadata_salt: Option<u32>,
    pub(super) replay_salt: Option<u32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_username"
    )]
    pub(super) username: Option<u32>,
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

impl Salts {
    pub(crate) fn from_url(url: &str) -> Option<Self> {
        // Expect URLs like: http://replay404.valve.net/1422450/37959196_937530290.meta.bz2 or http://replay183.valve.net/1422450/42476710_428480166.dem.bz2
        // Strip query parameters if present
        let base_url = url.split_once('?').map_or(url, |(path, _)| path);

        let (cluster_str, remaining) = base_url
            .strip_prefix("http://replay")?
            .split_once(".valve.net/")?;
        // remaining should be like "1422450/37959196_937530290.meta.bz2"
        let name = remaining.rsplit_once('/').map(|(_, name)| name)?;
        if name.ends_with(".meta.bz2") {
            let name = name.strip_suffix(".meta.bz2")?;
            let (match_str, salt_str) = name.split_once('_')?;

            Some(Self {
                cluster_id: Some(cluster_str.parse().ok()?),
                match_id: match_str.parse().ok()?,
                metadata_salt: salt_str.parse().ok(),
                replay_salt: None,
                username: crate::steam_user::current_steam_id3(),
            })
        } else if name.ends_with(".dem.bz2") {
            let name = name.strip_suffix(".dem.bz2")?;
            let (match_str, salt_str) = name.split_once('_')?;

            Some(Self {
                cluster_id: Some(cluster_str.parse().ok()?),
                match_id: match_str.parse().ok()?,
                replay_salt: salt_str.parse().ok(),
                metadata_salt: None,
                username: crate::steam_user::current_steam_id3(),
            })
        } else {
            None
        }
    }

    /// The salts in a Game Coordinator `GetMatchMetaData` reply for `match_id`; `None` when it
    /// carries no salt.
    pub(crate) fn from_metadata_reply(
        match_id: u64,
        reply: &CMsgClientToGcGetMatchMetaDataResponse,
        username: Option<u32>,
    ) -> Option<Self> {
        let salt = |salt: Option<u32>| salt.filter(|&s| s != 0);
        let (metadata_salt, replay_salt) = (salt(reply.metadata_salt), salt(reply.replay_salt));
        (metadata_salt.is_some() || replay_salt.is_some()).then_some(Self {
            match_id,
            cluster_id: reply.replay_group_id,
            metadata_salt,
            replay_salt,
            username,
        })
    }

    /// POSTs the salts and, on success, records them (see `ingestion_cache`).
    pub(crate) fn ingest(&self, source: Source) -> Result<(), Error> {
        if self.match_id > MAX_MATCH_ID {
            return Err(Error::MatchIdTooLarge);
        }
        Self::ingest_batch(core::slice::from_ref(self), source)
    }

    /// Like [`Self::ingest`] for many salts, in batches the API accepts. A failed batch
    /// doesn't stop the ones after it; the first failure is returned, later ones are logged.
    pub(crate) fn ingest_many(salts: &[Salts], source: Source) -> Result<(), Error> {
        for_each_batch(salts, |batch| Self::ingest_batch(batch, source))
    }

    fn ingest_batch(salts: &[Salts], source: Source) -> Result<(), Error> {
        post_salts(SALTS_URL, salts, RETRY_DELAY)?;
        for salt in salts {
            ingestion_cache::mark_ingested(salt, source);
        }
        Ok(())
    }
}

const SALTS_URL: &str = "https://api.deadlock-api.com/v1/matches/salts";
const MAX_ATTEMPTS: u32 = 10;
const RETRY_DELAY: Duration = Duration::from_secs(3);

/// Runs `send` on every batch of at most [`MAX_SALTS_PER_REQUEST`] salts, even after one fails.
fn for_each_batch(
    salts: &[Salts],
    mut send: impl FnMut(&[Salts]) -> Result<(), Error>,
) -> Result<(), Error> {
    let mut first_error = None;
    for batch in salts.chunks(MAX_SALTS_PER_REQUEST) {
        if let Err(e) = send(batch) {
            if first_error.is_none() {
                first_error = Some(e);
            } else {
                warn!("Failed to ingest {} salts: {e}", batch.len());
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// POSTs `salts` to `url`. Connection errors, 429 and 5xx are retried, up to
/// [`MAX_ATTEMPTS`] tries `retry_delay` apart; any other non-2xx fails at once, since
/// sending the same body again won't change the answer.
fn post_salts(url: &str, salts: &[Salts], retry_delay: Duration) -> Result<(), Error> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        if let [salt] = salts {
            debug!("Ingesting salts: {salt:?} (attempt {attempt}/{MAX_ATTEMPTS})");
        } else {
            let n = salts.len();
            debug!("Ingesting {n} salts (attempt {attempt}/{MAX_ATTEMPTS})");
        }

        let response = http_client()
            .post(url)
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(salts);
        let error = match response {
            Ok(r) if r.status().is_success() => return Ok(()),
            Ok(mut r) => {
                let status = r.status();
                let body = r.body_mut().read_to_string().unwrap_or_default();
                let error = Error::Rejected {
                    status: status.as_u16(),
                    body,
                };
                if !(status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS) {
                    return Err(error);
                }
                error
            }
            Err(e) => Error::Ureq(e),
        };
        if attempt >= MAX_ATTEMPTS {
            return Err(error);
        }
        debug!("Salt upload failed, retrying in {retry_delay:?}: {error}");
        sleep(retry_delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    /// A local server answering its `n`th request with `answers[n]` (the last one once
    /// they run out); returns its URL and a count of the requests it got.
    fn serve(answers: &'static [(u16, &'static str)]) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut reader = BufReader::new(stream.unwrap());
                let mut content_length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        content_length = value.trim().parse().unwrap();
                    }
                }
                reader.read_exact(&mut vec![0; content_length]).unwrap();

                let n = count.fetch_add(1, Ordering::SeqCst);
                let (status, body) = answers[n.min(answers.len() - 1)];
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (url, requests)
    }

    fn salt(match_id: u64) -> Salts {
        Salts {
            match_id,
            cluster_id: Some(1),
            metadata_salt: Some(2),
            replay_salt: None,
            username: None,
        }
    }

    #[test]
    fn a_client_error_is_not_retried_and_keeps_the_apis_reason() {
        let (url, requests) = serve(&[(422, "Too many salts provided (max 1000, got 1001)")]);
        let err = post_salts(&url, &[salt(1)], Duration::ZERO).unwrap_err();
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            err.to_string(),
            "API answered 422: Too many salts provided (max 1000, got 1001)"
        );
    }

    #[test]
    fn server_errors_and_rate_limits_are_retried() {
        let (url, requests) = serve(&[(500, ""), (429, ""), (503, ""), (200, "")]);
        post_salts(&url, &[salt(1)], Duration::ZERO).unwrap();
        assert_eq!(requests.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn retries_stop_after_max_attempts_with_the_last_answer() {
        let (url, requests) = serve(&[(500, "down")]);
        let err = post_salts(&url, &[salt(1)], Duration::ZERO).unwrap_err();
        assert_eq!(requests.load(Ordering::SeqCst), MAX_ATTEMPTS as usize);
        assert_eq!(err.to_string(), "API answered 500: down");
    }

    #[test]
    fn a_long_rejection_body_is_cut() {
        let body = "x".repeat(1000);
        let err = Error::Rejected { status: 502, body };
        assert_eq!(
            err.to_string(),
            format!("API answered 502: {}…", "x".repeat(300))
        );
    }

    #[test]
    fn a_failed_batch_does_not_stop_the_rest() {
        let salts: Vec<Salts> = (1..=2500).map(salt).collect();
        let mut sent = Vec::new();
        let result = for_each_batch(&salts, |batch| {
            sent.push(batch.len());
            if sent.len() == 1 {
                Err(Error::Rejected {
                    status: 500,
                    body: String::new(),
                })
            } else {
                Ok(())
            }
        });
        assert_eq!(sent, [1000, 1000, 500]);
        assert_eq!(result.unwrap_err().to_string(), "API answered 500");
    }

    #[test]
    fn test_extract_salts() {
        #[allow(clippy::type_complexity)]
        let cases: &[(&str, u32, u64, Option<u32>, Option<u32>)] = &[
            (
                "http://replay404.valve.net/1422450/37959196_937530290.meta.bz2",
                404,
                37959196,
                Some(937530290),
                None,
            ),
            (
                "http://replay400.valve.net/1422450/38090632_88648761.meta.bz2",
                400,
                38090632,
                Some(88648761),
                None,
            ),
            (
                "http://replay183.valve.net/1422450/42476710_428480166.meta.bz2",
                183,
                42476710,
                Some(428480166),
                None,
            ),
            (
                "http://replay183.valve.net/1422450/42476710_428480166.dem.bz2",
                183,
                42476710,
                None,
                Some(428480166),
            ),
            (
                "http://replay404.valve.net/1422450/37959196_937530290.meta.bz2?v=2",
                404,
                37959196,
                Some(937530290),
                None,
            ),
            (
                "http://replay183.valve.net/1422450/42476710_428480166.dem.bz2?v=2",
                183,
                42476710,
                None,
                Some(428480166),
            ),
        ];

        for &(url, cluster_id, match_id, metadata_salt, replay_salt) in cases {
            let salts = Salts::from_url(url).unwrap();
            assert_eq!(salts.cluster_id, Some(cluster_id));
            assert_eq!(salts.match_id, match_id);
            assert_eq!(salts.metadata_salt, metadata_salt);
            assert_eq!(salts.replay_salt, replay_salt);
        }
    }
}
