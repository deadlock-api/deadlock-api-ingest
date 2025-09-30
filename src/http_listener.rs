use crate::utils::Salts;
use std::collections::HashSet;
use std::str;
use tracing::{debug, info, warn};

/// Trait that platform-specific listeners implement. Implementors must provide a payload iterator.
/// The trait provides a default `listen()` which owns the processing loop and calls helpers for
/// extracting HTTP regions and processing packet payloads.
pub(crate) trait HttpListener {
    /// Return an iterator of packet payloads (each as a Vec<u8>).
    /// Implementations may return an error if the capture cannot be set up.
    fn payloads(&self) -> anyhow::Result<Box<dyn Iterator<Item = Vec<u8>>>>;

    /// Start listening and process payloads produced by `payloads()`.
    fn listen(&self) -> anyhow::Result<()> {
        let mut ingested_metadata = HashSet::new();
        let mut ingested_replay = HashSet::new();
        for payload in self.payloads()? {
            // If the payload is too short, it's not an HTTP request.
            if payload.len() < 60 {
                continue;
            }

            // Try extract salts from the payload
            let Some(salts) = Self::extract_salts(&payload) else {
                continue;
            };

            let is_new_metadata =
                salts.metadata_salt.is_some() && !ingested_metadata.contains(&salts.match_id);
            let is_new_replay =
                salts.replay_salt.is_some() && !ingested_replay.contains(&salts.match_id);
            if !is_new_metadata && !is_new_replay {
                debug!(salts = ?salts, "Already ingested");
                continue;
            }

            // Ingest the Salts
            match salts.ingest() {
                Ok(..) => info!(salts = ?salts, "Ingested salts"),
                Err(e) => {
                    warn!(salts = ?salts, "Failed to ingest salts: {e}");
                    continue;
                }
            }

            if salts.metadata_salt.is_some() {
                ingested_metadata.insert(salts.match_id);

                if ingested_metadata.len() > 1_000 {
                    ingested_metadata.clear(); // Clear the set if it's too large
                }
            }
            if salts.replay_salt.is_some() {
                ingested_replay.insert(salts.match_id);

                if ingested_replay.len() > 1_000 {
                    ingested_replay.clear(); // Clear the set if it's too large
                }
            }
        }
        Ok(())
    }

    fn extract_salts(payload: &[u8]) -> Option<Salts> {
        let http_packet = Self::find_http_in_packet(payload)?;
        let url = Self::parse_http_request(&http_packet)?;
        debug!(url = %url, "Found HTTP URL");

        // Strip query parameters before checking file extension
        let base_url = url.split_once('?').map_or(url.as_str(), |(path, _)| path);
        if !base_url.ends_with(".meta.bz2") && !base_url.ends_with(".dem.bz2") {
            return None;
        }
        Salts::from_url(&url)
    }

    fn find_http_in_packet(data: &[u8]) -> Option<String> {
        let scan_len = data.len().min(4096);
        let data = &data[..scan_len];

        memchr::memmem::find(data, b"GET ")
            .map(|pos| &data[pos..])
            .map(|r| match memchr::memmem::find(r, b"\r\n\r\n") {
                Some(end) => &r[..end + 4],
                None => &r[..r.len().min(1024)],
            })
            .map(|r| {
                str::from_utf8(r).map_or_else(
                    |_| String::from_utf8_lossy(r).to_string(),
                    ToString::to_string,
                )
            })
    }

    fn parse_http_request(http_data: &str) -> Option<String> {
        let mut lines = http_data.lines();

        let request_line = lines.next()?.trim();
        let mut parts = request_line.split_whitespace();
        let _method = parts.next()?;

        let path = parts.next()?.trim_start_matches('/');

        if path.starts_with("http://") || path.starts_with("https://") {
            return Some(path.to_owned());
        }

        let proto = parts.next()?;
        if !proto.starts_with("HTTP/") {
            return None;
        }

        lines
            .map(str::trim)
            .take_while(|l| !l.is_empty())
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.trim()
                        .eq_ignore_ascii_case("host")
                        .then(|| value.trim())
                })
            })
            .map(|host| format!("http://{host}/{path}"))
    }
}

pub(super) struct PlatformListener;

#[cfg(target_os = "windows")]
impl HttpListener for PlatformListener {
    fn payloads(&self) -> anyhow::Result<Box<dyn Iterator<Item = Vec<u8>>>> {
        let mut cap = pktmon::Capture::new()?;

        // Set filter to capture HTTP traffic (both outgoing and incoming on port 80)
        cap.add_filter(pktmon::filter::PktMonFilter {
            name: "HTTP Filter".to_string(),
            port: 80.into(),
            transport_protocol: Some(pktmon::filter::TransportProtocol::TCP),
            ..Default::default()
        })?;
        cap.start()?;

        // Build a boxed iterator that drives the pktmon capture. On errors we log and continue.
        let iter = core::iter::from_fn(move || {
            loop {
                match cap.next_packet() {
                    Ok(packet) => return Some(packet.payload.to_vec().clone()),
                    Err(e) => {
                        warn!("Error reading packet: {e}");
                    }
                }
            }
        });

        Ok(Box::new(iter))
    }
}

#[cfg(target_os = "linux")]
use anyhow::Context;

#[cfg(target_os = "linux")]
impl HttpListener for PlatformListener {
    fn payloads(&self) -> anyhow::Result<Box<dyn Iterator<Item = Vec<u8>>>> {
        let device = Self::get_device()?;

        info!(
            "Monitoring device: {} ({})",
            device.name,
            device.desc.as_deref().unwrap_or("no description")
        );

        let mut cap = pcap::Capture::from_device(device)?
            .promisc(true)
            .timeout(1000)
            .open()?;

        // Set filter to capture HTTP traffic (both outgoing and incoming on port 80)
        cap.filter("tcp port 80", true)?;

        // Build a boxed iterator that drives the pcap capture. The closure will loop on timeouts
        // and only return None on fatal errors (ending the iterator).
        let iter = core::iter::from_fn(move || {
            loop {
                match cap.next_packet() {
                    Ok(packet) => return Some(packet.data.to_vec()),
                    Err(pcap::Error::TimeoutExpired) => {}
                    Err(e) => {
                        warn!("Error reading packet: {e}");
                        return None;
                    }
                }
            }
        });

        Ok(Box::new(iter))
    }
}

#[cfg(target_os = "linux")]
impl PlatformListener {
    fn get_device() -> anyhow::Result<pcap::Device> {
        if let Some(device_name) = std::env::args().nth(1)
            && let Ok(device_list) = pcap::Device::list()
        {
            if let Some(device) = device_list.iter().find(|d| d.name == device_name) {
                return Ok(device.clone());
            }
            warn!(
                "Device {device_name} not found, pick one from the list: {:?}",
                device_list.iter().find(|d| d.name == device_name)
            );
        }
        pcap::Device::lookup()?.context("Failed to find network device")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyListener;
    impl HttpListener for DummyListener {
        fn payloads(&self) -> anyhow::Result<Box<dyn Iterator<Item = Vec<u8>>>> {
            Ok(Box::new(core::iter::empty::<Vec<u8>>()))
        }
    }

    #[test]
    fn test_parse_http_request_via_trait() {
        let http_data = "GET / HTTP/1.1\r\nHost: www.example.com\r\n\r\n";
        assert_eq!(
            <DummyListener as HttpListener>::parse_http_request(http_data).unwrap(),
            "http://www.example.com/"
        );
    }

    #[test]
    fn test_find_http_in_packet_via_trait() {
        let payload =
            b"\x00\x01randomdataGET /path HTTP/1.1\r\nHost: example.com\r\n\r\nmore".to_vec();
        let found = <DummyListener as HttpListener>::find_http_in_packet(&payload).unwrap();
        assert!(found.contains("GET /path HTTP/1.1"));
    }

    #[test]
    fn test_extract_salts_with_query_params() {
        // Test URL without query params - should work
        let http_data_without_query = "GET /1422450/37959196_937530290.meta.bz2 HTTP/1.1\r\nHost: replay404.valve.net\r\n\r\n";
        let packet_without_query = format!("randomdata{http_data_without_query}").into_bytes();
        let salts = <DummyListener as HttpListener>::extract_salts(&packet_without_query);
        assert!(
            salts.is_some(),
            "Should extract salts from URL without query params"
        );

        // Test URL with query params - currently fails but should work after fix
        let http_data_with_query = "GET /1422450/37959196_937530290.meta.bz2?v=2 HTTP/1.1\r\nHost: replay404.valve.net\r\n\r\n";
        let packet_with_query = format!("randomdata{http_data_with_query}").into_bytes();
        let salts_with_query = <DummyListener as HttpListener>::extract_salts(&packet_with_query);
        assert!(
            salts_with_query.is_some(),
            "Should extract salts from URL with query params"
        );
    }
}
