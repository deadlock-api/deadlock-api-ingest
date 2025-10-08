use crate::error::Error;
use core::time::Duration;
use std::sync::OnceLock;
use std::thread::sleep;
use ureq::Error::StatusCode;

static HTTP_CLIENT: OnceLock<ureq::Agent> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Salts {
    pub(super) match_id: u64,
    cluster_id: u32,
    pub(super) metadata_salt: Option<u32>,
    pub(super) replay_salt: Option<u32>,
}

impl Salts {
    fn to_json(self) -> String {
        let metadata_salt = match self.metadata_salt {
            Some(val) => val.to_string(),
            None => "null".to_string(),
        };
        let replay_salt = match self.replay_salt {
            Some(val) => val.to_string(),
            None => "null".to_string(),
        };
        let match_id = self.match_id;
        let cluster_id = self.cluster_id;

        format!(
            r#"[{{"match_id":{match_id},"cluster_id":{cluster_id},"metadata_salt":{metadata_salt},"replay_salt":{replay_salt}}}]"#,
        )
    }

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

    pub(super) fn ingest(&self) -> Result<(), Error> {
        if self.match_id > 100000000 {
            return Err(Error::MatchIdTooLarge);
        }

        let max_retries = 10;
        let mut attempt = 0;
        let json_body = self.to_json();

        loop {
            attempt += 1;
            println!("Ingesting salts: {self:?} ({attempt}/{max_retries})");
            let response = HTTP_CLIENT
                .get_or_init(ureq::Agent::new_with_defaults)
                .post("https://api.deadlock-api.com/v1/matches/salts")
                .header("Content-Type", "application/json")
                .send(&json_body);
            match response {
                Ok(r) if r.status().is_success() => return Ok(()),
                Ok(mut resp) if attempt == max_retries => {
                    let text = resp.body_mut().read_to_string().unwrap_or_default();
                    return Err(Error::FailedToIngest(text));
                }
                Err(e) if attempt == max_retries || matches!(e, StatusCode(s) if s == 400) => {
                    return Err(Error::Ureq(e));
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

    #[test]
    fn test_to_json_with_metadata_salt() {
        let salts = Salts {
            match_id: 37959196,
            cluster_id: 404,
            metadata_salt: Some(937530290),
            replay_salt: None,
        };
        let json = salts.to_json();
        assert_eq!(
            json,
            r#"[{"match_id":37959196,"cluster_id":404,"metadata_salt":937530290,"replay_salt":null}]"#
        );
    }

    #[test]
    fn test_to_json_with_replay_salt() {
        let salts = Salts {
            match_id: 42476710,
            cluster_id: 183,
            metadata_salt: None,
            replay_salt: Some(428480166),
        };
        let json = salts.to_json();
        assert_eq!(
            json,
            r#"[{"match_id":42476710,"cluster_id":183,"metadata_salt":null,"replay_salt":428480166}]"#
        );
    }

    #[test]
    fn test_to_json_with_both_salts() {
        let salts = Salts {
            match_id: 12345678,
            cluster_id: 100,
            metadata_salt: Some(111111),
            replay_salt: Some(222222),
        };
        let json = salts.to_json();
        assert_eq!(
            json,
            r#"[{"match_id":12345678,"cluster_id":100,"metadata_salt":111111,"replay_salt":222222}]"#
        );
    }
}
