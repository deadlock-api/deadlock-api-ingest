use crate::utils;
use pktmon::Capture;
use pktmon::filter::{PktMonFilter, TransportProtocol};
use std::collections::HashSet;
use tracing::{debug, info, warn};

pub(super) fn listen() -> anyhow::Result<()> {
    let mut cap = Capture::new()?;

    // Set filter to capture HTTP traffic (both outgoing and incoming on port 80)
    cap.add_filter(PktMonFilter {
        name: "HTTP Filter".to_string(),
        port: 80.into(),
        transport_protocol: Some(TransportProtocol::TCP),
        ..Default::default()
    })?;
    cap.start()?;

    let mut ingested_urls = HashSet::new();
    loop {
        match cap.next_packet() {
            Ok(packet) => {
                let payload = packet.payload.to_vec();
                if payload.len() < 60 {
                    continue;
                }

                if let Some(http_packet) = utils::find_http_in_packet(payload.as_slice())
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
            Err(e) => {
                warn!("Error reading packet: {e}");
            }
        }
    }
}
