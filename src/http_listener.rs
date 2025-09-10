use anyhow::Context;
use pcap::{Capture, Device};
use regex::Regex;
use std::collections::HashSet;
use std::str;
use tracing::{debug, info};

const POSSIBLE_HTTP_STARTS: [usize; 17] = [
    54, 66, 78, 40, 42, 44, 46, 48, 50, 52, 56, 58, 60, 62, 64, 68, 70,
];

pub(crate) fn listen() -> anyhow::Result<()> {
    let device = Device::lookup()?.context("Failed to find network device")?;

    info!(
        "\nMonitoring device: {} ({})",
        device.name,
        device.desc.as_deref().unwrap_or("no description")
    );

    let mut cap = Capture::from_device(device)?
        .promisc(true)
        .snaplen(65536) // Capture full packets
        .timeout(100) // Shorter timeout for more responsive capture
        .buffer_size(1000000) // Larger buffer
        .open()?;

    // Set filter to capture HTTP traffic (both outgoing and incoming on port 80)
    cap.filter("tcp port 80", true)?;

    let mut ingested_urls = HashSet::new();
    loop {
        match cap.next_packet() {
            Ok(packet) => {
                if packet.data.len() < 60 {
                    continue;
                }

                if let Some(http_packet) = find_http_in_packet(packet.data)
                    && let Some(url) = parse_http_request(&http_packet)
                {
                    debug!("Found HTTP URL: {url}");
                    if ingested_urls.contains(&url) {
                        continue;
                    }
                    if url.ends_with(".meta.bz2") {
                        ingest_salts(&url)?;
                        ingested_urls.insert(url.clone());
                        info!("Ingested salts for: {url}");
                    }
                }
            }
            Err(pcap::Error::TimeoutExpired) => {
                // This is normal, just continue
                continue;
            }
            Err(e) => {
                eprintln!("Error reading packet: {}", e);
                continue;
            }
        }
    }
}

fn ingest_salts(url: &str) -> anyhow::Result<()> {
    // http://replay404.valve.net/1422450/37959196_937530290.meta.bz2
    // extract cluster_id = 404
    // extract match_id = 37959196
    // extract metadata_salt = 937530290

    let re = Regex::new(r"http://replay(\d+)\.valve\.net/\d+/(\d+)_(\d+)\.meta\.bz2")?;
    let caps = re.captures(url).context("Failed to parse URL")?;
    let cluster_id = caps
        .get(1)
        .and_then(|m| m.as_str().parse::<u64>().ok())
        .context("Failed to parse cluster ID")?;
    let match_id = caps
        .get(2)
        .and_then(|m| m.as_str().parse::<u64>().ok())
        .context("Failed to parse match ID")?;
    let metadata_salt = caps
        .get(3)
        .and_then(|m| m.as_str().parse::<u64>().ok())
        .context("Failed to parse metadata salt")?;

    let client = reqwest::blocking::Client::new();
    client
        .post("https://api.deadlock-api.com/v1/matches/salts")
        .json(&serde_json::json!({
            "cluster_id": cluster_id,
            "match_id": match_id,
            "metadata_salt": metadata_salt,
        }))
        .send()?;
    Ok(())
}

fn find_http_in_packet(data: &[u8]) -> Option<String> {
    for &start in &POSSIBLE_HTTP_STARTS {
        if start >= data.len() {
            continue;
        }

        let payload = &data[start..];

        // Try to convert to string
        if let Ok(payload_str) = str::from_utf8(payload)
            && payload_str.starts_with("GET ")
        {
            return Some(payload_str.to_string());
        }

        // Also try with partial UTF-8
        if let Some(valid_str) = try_partial_utf8(payload)
            && valid_str.starts_with("GET ")
        {
            return Some(valid_str);
        }
    }

    None
}

fn try_partial_utf8(data: &[u8]) -> Option<String> {
    // Try to get valid UTF-8 from the beginning of the data
    for end in (100..data.len().min(2000)).step_by(50).rev() {
        if let Ok(s) = str::from_utf8(&data[..end]) {
            return Some(s.to_string());
        }
    }
    None
}

fn parse_http_request(http_data: &str) -> Option<String> {
    let lines: Vec<&str> = http_data.lines().collect();
    if lines.is_empty() {
        return None;
    }

    // Parse the request line
    let request_line = lines[0].trim();
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }

    let path = parts[1];

    // Skip if it's not HTTP
    if !parts[2].starts_with("HTTP/") {
        return None;
    }

    // Find the Host header
    let mut host = None;
    for line in &lines[1..] {
        if line.is_empty() {
            break; // End of headers
        }

        let line = line.trim();
        if let Some(colon_pos) = line.find(':') {
            let header_name = line[..colon_pos].trim().to_lowercase();
            let header_value = line[colon_pos + 1..].trim();

            if header_name == "host" {
                host = Some(header_value.to_string());
                break;
            }
        }
    }

    host.map(|host| {
        if path.starts_with("http://") {
            path.to_owned()
        } else {
            format!("http://{host}{path}")
        }
    })
}
