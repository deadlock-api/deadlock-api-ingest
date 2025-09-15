use anyhow::Context;
use serde::Serialize;
use std::str;
use tracing::debug;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
struct Salts {
    match_id: u64,
    cluster_id: u32,
    metadata_salt: u32,
}

impl Salts {
    fn from_url(url: &str) -> Option<Self> {
        let (cluster_str, remaining) = url
            .strip_prefix("http://replay")?
            .split_once(".valve.net/")?;
        let (match_str, salt_str) = remaining
            .rsplit_once('/')
            .map(|(_, name)| name)?
            .strip_suffix(".meta.bz2")?
            .split_once('_')?;
        Self {
            cluster_id: cluster_str.parse().ok()?,
            match_id: match_str.parse().ok()?,
            metadata_salt: salt_str.parse().ok()?,
        }
        .into()
    }
}

pub(super) fn ingest_salts(url: &str) -> anyhow::Result<()> {
    let salts = Salts::from_url(url).context("Failed to extract salts from URL")?;
    let response = reqwest::blocking::Client::new()
        .post("https://api.deadlock-api.com/v1/matches/salts")
        .json(&salts)
        .send()?;
    debug!("{:?}", response.text());
    Ok(())
}

pub(super) fn find_http_in_packet(data: &[u8]) -> Option<String> {
    (40..=78)
        .step_by(2)
        .filter_map(|start| data.get(start..))
        .find_map(|payload| {
            str::from_utf8(payload)
                .ok()
                .filter(|s| s.starts_with("GET "))
                .map(ToString::to_string)
                .or_else(|| try_partial_utf8(payload).filter(|s| s.starts_with("GET ")))
        })
}

fn try_partial_utf8(data: &[u8]) -> Option<String> {
    (100..data.len().min(2000))
        .step_by(50)
        .rev()
        .find_map(|end| str::from_utf8(&data[..end]).ok().map(ToString::to_string))
}

pub(super) fn parse_http_request(http_data: &str) -> Option<String> {
    let mut lines = http_data.lines();

    // Parse the request line
    let request_line = lines.next()?.trim();
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }

    let path = parts[1];
    if path.starts_with("http://") {
        return Some(path.to_owned());
    }

    // Skip if it's not HTTP
    if !parts[2].starts_with("HTTP/") {
        return None;
    }

    lines
        .map(str::trim)
        .take_while(|l| !l.is_empty())
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.trim().to_lowercase() == "host")
                .map(|(_, value)| value.trim())
        })
        .map(|host| format!("http://{host}{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_salts() {
        let url = "http://replay404.valve.net/1422450/37959196_937530290.meta.bz2";
        assert_eq!(
            Salts::from_url(url).unwrap(),
            Salts {
                cluster_id: 404,
                match_id: 37959196,
                metadata_salt: 937530290,
            }
        );
        let url = "http://replay400.valve.net/1422450/38090632_88648761.meta.bz2";
        assert_eq!(
            Salts::from_url(url).unwrap(),
            Salts {
                cluster_id: 400,
                match_id: 38090632,
                metadata_salt: 88648761,
            }
        );
    }

    #[test]
    fn test_parse_http_request() {
        let http_data = "GET / HTTP/1.1\r\nHost: www.example.com\r\n\r\n";
        assert_eq!(
            parse_http_request(http_data).unwrap(),
            "http://www.example.com/"
        );
    }
}
