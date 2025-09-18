use anyhow::{Context, bail};
use serde::Serialize;
use std::sync::OnceLock;

static HTTP_CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
pub(super) struct Salts {
    pub(super) match_id: u64,
    cluster_id: u32,
    metadata_salt: u32,
}

impl Salts {
    pub(crate) fn from_url(url: &str) -> Option<Self> {
        // Expect URLs like: http://replay404.valve.net/1422450/37959196_937530290.meta.bz2
        let (cluster_str, remaining) = url
            .strip_prefix("http://replay")?
            .split_once(".valve.net/")?;
        // remaining should be like "1422450/37959196_937530290.meta.bz2"
        let name = remaining.rsplit_once('/').map(|(_, name)| name)?;
        let name = name.strip_suffix(".meta.bz2")?;
        let (match_str, salt_str) = name.split_once('_')?;

        Some(Self {
            cluster_id: cluster_str.parse().ok()?,
            match_id: match_str.parse().ok()?,
            metadata_salt: salt_str.parse().ok()?,
        })
    }

    pub(super) fn ingest(&self) -> anyhow::Result<()> {
        let resp = HTTP_CLIENT
            .get_or_init(reqwest::blocking::Client::new)
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
        937530290
    )]
    #[case(
        "http://replay400.valve.net/1422450/38090632_88648761.meta.bz2",
        400,
        38090632,
        88648761
    )]
    #[case(
        "http://replay183.valve.net/1422450/42476710_428480166.meta.bz2",
        183,
        42476710,
        428480166
    )]
    fn test_extract_salts(
        #[case] url: &str,
        #[case] cluster_id: u32,
        #[case] match_id: u64,
        #[case] metadata_salt: u32,
    ) {
        let salts = Salts::from_url(url).unwrap();
        assert_eq!(salts.cluster_id, cluster_id);
        assert_eq!(salts.match_id, match_id);
        assert_eq!(salts.metadata_salt, metadata_salt);
    }
}
