use anyhow::{Context, bail};
use core::time::Duration;
use serde::Serialize;
use std::sync::OnceLock;

static HTTP_CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
pub(super) struct Salts {
    pub(super) match_id: u64,
    cluster_id: u32,
    metadata_salt: Option<u32>,
    replay_salt: Option<u32>,
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
        let resp = HTTP_CLIENT
            .get_or_init(|| {
                reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(20))
                    .build()
                    .unwrap_or_default()
            })
            .post("https://api.deadlock-api.com/v1/matches/salts")
            .json(&[self])
            .send()
            .context("Failed to send salts to API")?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            bail!("Ingest request failed: {status} {body}");
        }
        Ok(())
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
