use anyhow::bail;
use core::time::Duration;
use serde::Serialize;
use std::sync::OnceLock;
use std::thread::sleep;
use tracing::debug;
use ureq::Error::StatusCode;

static HTTP_CLIENT: OnceLock<ureq::Agent> = OnceLock::new();

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
pub(super) struct Salts {
    pub(super) match_id: u64,
    cluster_id: u32,
    pub(super) metadata_salt: Option<u32>,
    pub(super) replay_salt: Option<u32>,
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
                cluster_id: cluster_str.parse().ok()?,
                match_id: match_str.parse().ok()?,
                metadata_salt: salt_str.parse().ok(),
                replay_salt: None,
            })
        } else if name.ends_with(".dem.bz2") {
            let name = name.strip_suffix(".dem.bz2")?;
            let (match_str, salt_str) = name.split_once('_')?;

            Some(Self {
                cluster_id: cluster_str.parse().ok()?,
                match_id: match_str.parse().ok()?,
                replay_salt: salt_str.parse().ok(),
                metadata_salt: None,
            })
        } else {
            None
        }
    }

    pub(super) fn ingest(&self) -> anyhow::Result<()> {
        if self.match_id > 100000000 {
            bail!("Match ID is too large: {}", self.match_id);
        }

        let max_retries = 10;
        let mut attempt = 0;
        loop {
            attempt += 1;
            debug!(salts = ?self, attempt, "Ingesting salts");
            let response = HTTP_CLIENT
                .get_or_init(ureq::Agent::new_with_defaults)
                .post("https://api.deadlock-api.com/v1/matches/salts")
                .send_json([self]);
            match response {
                Ok(r) if r.status().is_success() => return Ok(()),
                Ok(mut resp) if attempt == max_retries => {
                    let text = resp.body_mut().read_to_string().unwrap_or_default();
                    bail!("Ingest request failed: {} {text}", resp.status());
                }
                Err(e) if attempt == max_retries || matches!(e, StatusCode(s) if s == 400) => {
                    bail!("Failed to send salts to API: {e}");
                }
                _ => sleep(Duration::from_secs(3)), // Retry on error
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(
        "http://replay404.valve.net/1422450/37959196_937530290.meta.bz2",
        404,
        37959196,
        Some(937530290),
        None
    )]
    #[case(
        "http://replay400.valve.net/1422450/38090632_88648761.meta.bz2",
        400,
        38090632,
        Some(88648761),
        None
    )]
    #[case(
        "http://replay183.valve.net/1422450/42476710_428480166.meta.bz2",
        183,
        42476710,
        Some(428480166),
        None
    )]
    #[case(
        "http://replay183.valve.net/1422450/42476710_428480166.dem.bz2",
        183,
        42476710,
        None,
        Some(428480166)
    )]
    #[case(
        "http://replay404.valve.net/1422450/37959196_937530290.meta.bz2?v=2",
        404,
        37959196,
        Some(937530290),
        None
    )]
    #[case(
        "http://replay183.valve.net/1422450/42476710_428480166.dem.bz2?v=2",
        183,
        42476710,
        None,
        Some(428480166)
    )]
    fn test_extract_salts(
        #[case] url: &str,
        #[case] cluster_id: u32,
        #[case] match_id: u64,
        #[case] metadata_salt: Option<u32>,
        #[case] replay_salt: Option<u32>,
    ) {
        let salts = Salts::from_url(url).unwrap();
        assert_eq!(salts.cluster_id, cluster_id);
        assert_eq!(salts.match_id, match_id);
        assert_eq!(salts.metadata_salt, metadata_salt);
        assert_eq!(salts.replay_salt, replay_salt);
    }
}
