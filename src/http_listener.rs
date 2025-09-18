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
        let mut ingested_matches = HashSet::new();
        for payload in self.payloads()? {
            // If the payload is too short, it's not an HTTP request.
            if payload.len() < 60 {
                continue;
            }

            // Try extract salts from the payload
            let Some(salts) = Self::extract_salts(&payload) else {
                continue;
            };
            if ingested_matches.contains(&salts.match_id) {
                debug!(salts = ?salts, "Already ingested match");
                continue;
            }

            // Ingest the Salts
            salts.ingest()?;

            info!(salts = ?salts, "Ingested salts");
            ingested_matches.insert(salts.match_id);
            if ingested_matches.len() > 1_000 {
                ingested_matches.clear(); // Clear the set if it's too large
            }
        }
        Ok(())
    }

    fn extract_salts(payload: &[u8]) -> Option<Salts> {
        let http_packet = Self::find_http_in_packet(payload)?;
        let url = Self::parse_http_request(&http_packet)?;
        debug!(url = %url, "Found HTTP URL");

        if !url.ends_with(".meta.bz2") {
            return None;
        }
        Salts::from_url(&url)
    }

    fn find_http_in_packet(data: &[u8]) -> Option<String> {
        let max_scan = data.len().min(4096);
        let data = &data[..max_scan];

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
        let mut parts_iter = request_line.split_whitespace();
        let _method = parts_iter.next()?;

        let path = parts_iter.next()?.trim_start_matches('/');

        if path.starts_with("http://") || path.starts_with("https://") {
            return Some(path.to_owned());
        }

        let proto = parts_iter.next()?;
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
        let device = pcap::Device::lookup()?.context("Failed to find network device")?;

        info!(
            "Monitoring device: {} ({})",
            device.name,
            device.desc.as_deref().unwrap_or("no description")
        );

        let mut cap = pcap::Capture::from_device(device)?
            .promisc(true)
            .snaplen(65536) // Capture full packets
            .timeout(100) // Shorter timeout for more responsive capture
            .buffer_size(1_000_000) // Larger buffer
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
}
