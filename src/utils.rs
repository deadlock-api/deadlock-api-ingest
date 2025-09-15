use anyhow::Context;
use std::str;
use tracing::debug;

pub(crate) fn ingest_salts(url: &str) -> anyhow::Result<()> {
    let (cluster_id, match_id, metadata_salt) = extract_salts(url)?;
    let client = reqwest::blocking::Client::new();
    let response = client
        .post("https://api.deadlock-api.com/v1/matches/salts")
        .json(&serde_json::json!([{
            "cluster_id": cluster_id,
            "match_id": match_id,
            "metadata_salt": metadata_salt,
        }]))
        .send()?;
    debug!("{:?}", response.text());
    Ok(())
}

fn extract_salts(url: &str) -> anyhow::Result<(u64, u64, u64)> {
    // 1. Isolate the cluster ID
    // Expects "http://replay<cluster>.valve.net/..."
    let remaining = url
        .strip_prefix("http://replay")
        .context("URL missing 'http://replay' prefix")?;
    let (cluster_str, remaining) = remaining
        .split_once(".valve.net/")
        .context("URL missing '.valve.net/' separator")?;
    let cluster_id = cluster_str
        .parse::<u64>()
        .context("Failed to parse cluster ID")?;

    // 2. Isolate the filename and remove the extension
    // Expects ".../<match>_<salt>.meta.bz2"
    let filename = remaining
        .rsplit_once('/')
        .map(|(_, name)| name) // Get the part after the last '/'
        .context("URL missing filename component")?;
    let ids_str = filename
        .strip_suffix(".meta.bz2")
        .context("Filename missing '.meta.bz2' suffix")?;

    // 3. Split the remaining string to get match ID and salt
    // Expects "<match>_<salt>"
    let (match_str, salt_str) = ids_str
        .split_once('_')
        .context("Filename missing '_' separator")?;
    let match_id = match_str
        .parse::<u64>()
        .context("Failed to parse match ID")?;
    let metadata_salt = salt_str
        .parse::<u64>()
        .context("Failed to parse metadata salt")?;

    Ok((cluster_id, match_id, metadata_salt))
}

pub(crate) fn find_http_in_packet(data: &[u8]) -> Option<String> {
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

pub(crate) fn try_partial_utf8(data: &[u8]) -> Option<String> {
    (100..data.len().min(2000))
        .step_by(50)
        .rev()
        .find_map(|end| str::from_utf8(&data[..end]).ok().map(ToString::to_string))
}

pub(crate) fn parse_http_request(http_data: &str) -> Option<String> {
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
        assert_eq!(extract_salts(url).unwrap(), (404, 37959196, 937530290));
        let url = "http://replay400.valve.net/1422450/38090632_88648761.meta.bz2";
        assert_eq!(extract_salts(url).unwrap(), (400, 38090632, 88648761));
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
