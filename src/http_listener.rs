use crate::error::Error;
use crate::http;
use crate::packet::TcpStreamId;
use crate::stream::StreamBuffer;
use crate::utils::Salts;
use core::time::Duration;
use std::collections::{HashMap, HashSet};

const MAX_CONCURRENT_STREAMS: usize = 1000;

/// Trait that platform-specific listeners implement. Implementors must provide a payload iterator.
/// The trait provides a default `listen()` which owns the processing loop and calls helpers for
/// extracting HTTP regions and processing packet payloads.
pub(crate) trait HttpListener {
    /// Return an iterator of packet payloads (each as a Vec<u8>).
    /// Implementations may return an error if the capture cannot be set up.
    fn payloads(&self) -> Result<Box<dyn Iterator<Item = Vec<u8>>>, Error>;

    /// Start listening and process payloads produced by `payloads()`.
    fn listen(&self) -> Result<(), Error> {
        let mut ingested_metadata = HashSet::new();
        let mut ingested_replay = HashSet::new();
        let mut stream_buffers: HashMap<TcpStreamId, StreamBuffer> = HashMap::new();
        let stream_timeout = Duration::from_secs(30);

        for payload in self.payloads()? {
            let Some(stream_id) = TcpStreamId::from_packet(&payload) else {
                continue;
            };

            // Get or create stream buffer
            let buffer = stream_buffers
                .entry(stream_id)
                .or_insert_with(StreamBuffer::new);

            // Append payload to stream buffer
            buffer.append(&payload);

            // Try to extract salts from the accumulated stream data
            let salts = Self::extract_salts(&buffer.data);

            // If we successfully extracted salts, clear the buffer for this stream
            if salts.is_some() {
                buffer.clear();
            }

            // Process the salts if found
            if let Some(salts) = salts {
                let is_new_metadata =
                    salts.metadata_salt.is_some() && !ingested_metadata.contains(&salts.match_id);
                let is_new_replay =
                    salts.replay_salt.is_some() && !ingested_replay.contains(&salts.match_id);

                if is_new_metadata || is_new_replay {
                    // Ingest the Salts
                    match salts.ingest() {
                        Ok(..) => println!("Ingested salts: {salts:?}"),
                        Err(e) => {
                            eprintln!("Failed to ingest salts: {e:?}");
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
            }

            // Clean up stale stream buffers
            if stream_buffers.len() > MAX_CONCURRENT_STREAMS {
                stream_buffers.retain(|_, buffer| !buffer.is_stale(stream_timeout));
            }
        }
        Ok(())
    }

    fn extract_salts(payload: &[u8]) -> Option<Salts> {
        let http_packet = http::find_http_in_packet(payload)?;
        let url = http::parse_http_request(&http_packet)?;

        // Strip query parameters before checking file extension
        let base_url = url.split_once('?').map_or(url.as_str(), |(path, _)| path);
        if !base_url.contains(".meta.bz2") && !base_url.contains(".dem.bz2") {
            println!("Found URL (without salts): {url}");
            return None;
        }
        println!("Found URL: {url}");
        Salts::from_url(&url)
    }
}

pub(super) struct PlatformListener;

#[cfg(target_os = "windows")]
impl HttpListener for PlatformListener {
    fn payloads(&self) -> Result<Box<dyn Iterator<Item = Vec<u8>>>, Error> {
        let mut cap = pktmon::Capture::new().map_err(Error::PktMon)?;

        // Set filter to capture HTTP traffic (both outgoing and incoming on port 80)
        cap.add_filter(pktmon::filter::PktMonFilter {
            name: "HTTP Filter".to_string(),
            port: 80.into(),
            transport_protocol: Some(pktmon::filter::TransportProtocol::TCP),
            ..Default::default()
        })
        .map_err(Error::PktMon)?;
        cap.start().map_err(Error::PktMon)?;

        // Build a boxed iterator that drives the pktmon capture. On errors we log and continue.
        let iter = core::iter::from_fn(move || {
            loop {
                match cap.next_packet() {
                    Ok(packet) => return Some(packet.payload.to_vec().clone()),
                    Err(e) => {
                        eprintln!("Error reading packet: {e}");
                    }
                }
            }
        });

        Ok(Box::new(iter))
    }
}

#[cfg(target_os = "linux")]
impl HttpListener for PlatformListener {
    fn payloads(&self) -> Result<Box<dyn Iterator<Item = Vec<u8>>>, Error> {
        let device = Self::get_device()?;
        println!("Monitoring device: {}", device.name);

        let mut cap = pcap::Capture::from_device(device)
            .map_err(Error::PCap)?
            .promisc(true)
            .timeout(1000)
            .open()
            .map_err(Error::PCap)?;

        // Set filter to capture HTTP traffic (both outgoing and incoming on port 80)
        cap.filter("tcp port 80", true).map_err(Error::PCap)?;

        // Build a boxed iterator that drives the pcap capture. The closure will loop on timeouts
        // and only return None on fatal errors (ending the iterator).
        let iter = core::iter::from_fn(move || {
            loop {
                match cap.next_packet() {
                    Ok(packet) => return Some(packet.data.to_vec()),
                    Err(pcap::Error::TimeoutExpired) => {}
                    Err(e) => {
                        println!("Error reading packet: {e}");
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
    fn get_device() -> Result<pcap::Device, Error> {
        if let Some(device_name) = std::env::args().nth(1)
            && let Ok(device_list) = pcap::Device::list()
        {
            if let Some(device) = device_list.iter().find(|d| d.name == device_name) {
                return Ok(device.clone());
            }
            println!(
                "Device {device_name} not found, pick one from the list: {:?}",
                device_list
                    .iter()
                    .map(|d| d.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        pcap::Device::lookup()
            .map_err(Error::PCap)
            .and_then(|r| r.ok_or(Error::NoDeviceFound))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyListener;
    impl HttpListener for DummyListener {
        fn payloads(&self) -> Result<Box<dyn Iterator<Item = Vec<u8>>>, Error> {
            Ok(Box::new(core::iter::empty::<Vec<u8>>()))
        }
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

    #[test]
    fn test_multi_packet_http_request() {
        // Simulate an HTTP request split across two packets
        let packet1 = b"GET /1422450/37959196_937530290.meta.bz2 HTTP/1.1\r\n";
        let packet2 = b"Host: replay404.valve.net\r\n\r\n";

        // First packet alone should not extract salts (incomplete request)
        let salts1 = <DummyListener as HttpListener>::extract_salts(packet1);
        assert!(
            salts1.is_none(),
            "Incomplete request should not extract salts"
        );

        // Combined packets should extract salts
        let mut combined = packet1.to_vec();
        combined.extend_from_slice(packet2);
        let salts_combined = <DummyListener as HttpListener>::extract_salts(&combined);
        assert!(
            salts_combined.is_some(),
            "Complete reassembled request should extract salts"
        );
    }

    #[test]
    fn test_fragmented_http_request_with_body() {
        // Test HTTP request split in the middle of headers
        let packet1 = b"randomdataGET /1422450/37959196_937530290.meta.bz2 HTTP/1.1\r\nHo";
        let packet2 = b"st: replay404.valve.net\r\n\r\n";

        // First packet alone should not work
        let salts1 = <DummyListener as HttpListener>::extract_salts(packet1);
        assert!(
            salts1.is_none(),
            "Fragmented request should not extract salts"
        );

        // Combined should work
        let mut combined = packet1.to_vec();
        combined.extend_from_slice(packet2);
        let salts_combined = <DummyListener as HttpListener>::extract_salts(&combined);
        assert!(
            salts_combined.is_some(),
            "Reassembled fragmented request should extract salts"
        );
    }
}
