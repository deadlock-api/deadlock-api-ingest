use crate::utils;
use anyhow::Context;
use pcap::{Capture, Device};
use std::collections::HashSet;
use tracing::{debug, info, warn};

pub(super) fn listen() -> anyhow::Result<()> {
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
        .buffer_size(1_000_000) // Larger buffer
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

                if let Some(http_packet) = utils::find_http_in_packet(packet.data)
                    && let Some(url) = utils::parse_http_request(&http_packet)
                {
                    debug!("Found HTTP URL: {url}");
                    if ingested_urls.contains(&url) {
                        continue;
                    }
                    if url.ends_with(".meta.bz2") {
                        utils::ingest_salts(&url)?;
                        ingested_urls.insert(url.clone());
                        info!("Ingested salts for: {url}");
                    }
                }
            }
            Err(pcap::Error::TimeoutExpired) => {
                // This is normal, just continue
            }
            Err(e) => {
                warn!("Error reading packet: {}", e);
            }
        }
    }
}
