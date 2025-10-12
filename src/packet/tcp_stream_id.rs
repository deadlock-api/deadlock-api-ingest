//! TCP stream identification
//!
//! This module provides the `TcpStreamId` struct which uniquely identifies a TCP
//! connection using the standard 5-tuple: source IP, destination IP, source port,
//! destination port, and protocol number.
//!
//! The implementation supports both IPv4 and IPv6, and can parse packets in
//! multiple formats (Ethernet frames or raw IP packets) to accommodate different
//! packet capture backends (pcap on Linux, pktmon on Windows).

use core::hash::{Hash, Hasher};
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

const ETHERTYPE_IPV4: u16 = 0x0800;
const ETHERTYPE_IPV6: u16 = 0x86DD;
const IPPROTO_TCP: u8 = 6;

/// Represents a TCP stream identifier using the 5-tuple
/// (source IP, destination IP, source port, destination port, protocol)
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TcpStreamId {
    pub(crate) src_ip: IpAddr,
    pub(crate) dst_ip: IpAddr,
    pub(crate) src_port: u16,
    pub(crate) dst_port: u16,
    pub(crate) protocol: u8,
}

impl Hash for TcpStreamId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash the components in a consistent order
        match self.src_ip {
            IpAddr::V4(ip) => {
                0u8.hash(state);
                ip.octets().hash(state);
            }
            IpAddr::V6(ip) => {
                1u8.hash(state);
                ip.octets().hash(state);
            }
        }
        match self.dst_ip {
            IpAddr::V4(ip) => {
                0u8.hash(state);
                ip.octets().hash(state);
            }
            IpAddr::V6(ip) => {
                1u8.hash(state);
                ip.octets().hash(state);
            }
        }
        self.src_port.hash(state);
        self.dst_port.hash(state);
        self.protocol.hash(state);
    }
}

impl TcpStreamId {
    /// Parse a TCP stream ID from a raw packet payload
    /// Handles both Ethernet frames (pcap on Linux) and raw IP packets (pktmon on Windows)
    pub(crate) fn from_packet(packet: &[u8]) -> Option<Self> {
        // Try parsing as Ethernet frame first (pcap format)
        if let Some(stream_id) = Self::from_ethernet_frame(packet) {
            return Some(stream_id);
        }

        // Try parsing as raw IP packet (pktmon format)
        Self::from_ip_packet(packet)
    }

    /// Parse from Ethernet frame (14-byte Ethernet header + IP packet)
    pub(crate) fn from_ethernet_frame(packet: &[u8]) -> Option<Self> {
        if packet.len() < 14 {
            return None;
        }

        // Ethernet header: [6 bytes dst MAC][6 bytes src MAC][2 bytes EtherType]
        let ethertype = u16::from_be_bytes([packet[12], packet[13]]);

        // Skip Ethernet header (14 bytes) to get to IP packet
        let ip_packet = &packet[14..];

        match ethertype {
            ETHERTYPE_IPV4 => Self::from_ipv4_packet(ip_packet),
            ETHERTYPE_IPV6 => Self::from_ipv6_packet(ip_packet),
            _ => None,
        }
    }

    /// Parse from raw IP packet (no Ethernet header)
    fn from_ip_packet(packet: &[u8]) -> Option<Self> {
        if packet.is_empty() {
            return None;
        }

        // Check IP version from first nibble
        let version = (packet[0] >> 4) & 0x0F;

        match version {
            4 => Self::from_ipv4_packet(packet),
            6 => Self::from_ipv6_packet(packet),
            _ => None,
        }
    }

    /// Parse from IPv4 packet
    pub(crate) fn from_ipv4_packet(packet: &[u8]) -> Option<Self> {
        // IPv4 header minimum size is 20 bytes
        if packet.len() < 20 {
            return None;
        }

        // Check protocol (byte 9)
        let protocol = packet[9];
        if protocol != IPPROTO_TCP {
            return None;
        }

        // Get header length (IHL field in first byte, lower 4 bits, in 32-bit words)
        let ihl = (packet[0] & 0x0F) as usize * 4;
        if packet.len() < ihl + 20 {
            // Not enough data for IP header + TCP header
            return None;
        }

        // Extract source and destination IP addresses (bytes 12-15 and 16-19)
        let src_ip = IpAddr::V4(Ipv4Addr::new(
            packet[12], packet[13], packet[14], packet[15],
        ));
        let dst_ip = IpAddr::V4(Ipv4Addr::new(
            packet[16], packet[17], packet[18], packet[19],
        ));

        // TCP header starts after IP header
        let tcp_header = &packet[ihl..];
        if tcp_header.len() < 4 {
            return None;
        }

        // Extract source and destination ports (first 4 bytes of TCP header)
        let src_port = u16::from_be_bytes([tcp_header[0], tcp_header[1]]);
        let dst_port = u16::from_be_bytes([tcp_header[2], tcp_header[3]]);

        Some(Self {
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            protocol,
        })
    }

    /// Parse from IPv6 packet
    pub(crate) fn from_ipv6_packet(packet: &[u8]) -> Option<Self> {
        // IPv6 header is fixed 40 bytes
        if packet.len() < 40 {
            return None;
        }

        // Check next header (byte 6) - should be TCP (6)
        // Note: This doesn't handle extension headers, which is a simplification
        let next_header = packet[6];
        if next_header != IPPROTO_TCP {
            return None;
        }

        // Extract source IP (bytes 8-23)
        let src_ip = IpAddr::V6(Ipv6Addr::new(
            u16::from_be_bytes([packet[8], packet[9]]),
            u16::from_be_bytes([packet[10], packet[11]]),
            u16::from_be_bytes([packet[12], packet[13]]),
            u16::from_be_bytes([packet[14], packet[15]]),
            u16::from_be_bytes([packet[16], packet[17]]),
            u16::from_be_bytes([packet[18], packet[19]]),
            u16::from_be_bytes([packet[20], packet[21]]),
            u16::from_be_bytes([packet[22], packet[23]]),
        ));

        // Extract destination IP (bytes 24-39)
        let dst_ip = IpAddr::V6(Ipv6Addr::new(
            u16::from_be_bytes([packet[24], packet[25]]),
            u16::from_be_bytes([packet[26], packet[27]]),
            u16::from_be_bytes([packet[28], packet[29]]),
            u16::from_be_bytes([packet[30], packet[31]]),
            u16::from_be_bytes([packet[32], packet[33]]),
            u16::from_be_bytes([packet[34], packet[35]]),
            u16::from_be_bytes([packet[36], packet[37]]),
            u16::from_be_bytes([packet[38], packet[39]]),
        ));

        // TCP header starts at byte 40
        let tcp_header = &packet[40..];
        if tcp_header.len() < 4 {
            return None;
        }

        // Extract source and destination ports
        let src_port = u16::from_be_bytes([tcp_header[0], tcp_header[1]]);
        let dst_port = u16::from_be_bytes([tcp_header[2], tcp_header[3]]);

        Some(Self {
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            protocol: next_header,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::hash::{Hash, Hasher};
    use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_tcp_stream_id_from_ipv4_packet() {
        // Create a minimal IPv4 + TCP packet
        // IPv4 header (20 bytes) + TCP header (20 bytes)
        let mut packet = vec![0u8; 40];

        // IPv4 header
        packet[0] = 0x45; // Version 4, IHL 5 (20 bytes)
        packet[9] = 6; // Protocol: TCP

        // Source IP: 192.168.1.100
        packet[12..16].copy_from_slice(&[192, 168, 1, 100]);

        // Destination IP: 10.0.0.1
        packet[16..20].copy_from_slice(&[10, 0, 0, 1]);

        // TCP header (starts at byte 20)
        // Source port: 12345 (0x3039)
        packet[20..22].copy_from_slice(&[0x30, 0x39]);

        // Destination port: 80 (0x0050)
        packet[22..24].copy_from_slice(&[0x00, 0x50]);

        let stream_id = TcpStreamId::from_ip_packet(&packet).unwrap();

        assert_eq!(
            stream_id.src_ip,
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100))
        );
        assert_eq!(stream_id.dst_ip, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(stream_id.src_port, 12345);
        assert_eq!(stream_id.dst_port, 80);
        assert_eq!(stream_id.protocol, 6);
    }

    #[test]
    fn test_tcp_stream_id_from_ethernet_frame() {
        // Create Ethernet frame with IPv4 + TCP
        let mut packet = vec![0u8; 54]; // 14 (Ethernet) + 20 (IPv4) + 20 (TCP)

        // Ethernet header (14 bytes)
        // Destination MAC: 00:11:22:33:44:55
        packet[0..6].copy_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);

        // Source MAC: AA:BB:CC:DD:EE:FF
        packet[6..12].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);

        // EtherType: IPv4 (0x0800)
        packet[12..14].copy_from_slice(&[0x08, 0x00]);

        // IPv4 header (starts at byte 14)
        packet[14] = 0x45; // Version 4, IHL 5
        packet[23] = 6; // Protocol: TCP

        // Source IP: 172.16.0.1
        packet[26..30].copy_from_slice(&[172, 16, 0, 1]);

        // Destination IP: 8.8.8.8
        packet[30..34].copy_from_slice(&[8, 8, 8, 8]);

        // TCP header (starts at byte 34)
        // Source port: 54321 (0xD431)
        packet[34..36].copy_from_slice(&[0xD4, 0x31]);

        // Destination port: 443 (0x01BB)
        packet[36..38].copy_from_slice(&[0x01, 0xBB]);

        let stream_id = TcpStreamId::from_ethernet_frame(&packet).unwrap();

        assert_eq!(stream_id.src_ip, IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1)));
        assert_eq!(stream_id.dst_ip, IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
        assert_eq!(stream_id.src_port, 54321);
        assert_eq!(stream_id.dst_port, 443);
        assert_eq!(stream_id.protocol, 6);
    }

    #[test]
    fn test_tcp_stream_id_hash_consistency() {
        use std::collections::hash_map::DefaultHasher;

        let stream_id1 = TcpStreamId {
            src_ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            dst_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            src_port: 12345,
            dst_port: 80,
            protocol: 6,
        };

        let stream_id2 = TcpStreamId {
            src_ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            dst_ip: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            src_port: 12345,
            dst_port: 80,
            protocol: 6,
        };

        // Same stream IDs should be equal
        assert_eq!(stream_id1, stream_id2);

        // Same stream IDs should produce same hash
        let mut hasher1 = DefaultHasher::new();
        let mut hasher2 = DefaultHasher::new();
        stream_id1.hash(&mut hasher1);
        stream_id2.hash(&mut hasher2);
        assert_eq!(hasher1.finish(), hasher2.finish());
    }

    #[test]
    fn test_tcp_stream_id_invalid_packets() {
        // Too short packet
        let short_packet = vec![0u8; 10];
        assert!(TcpStreamId::from_packet(&short_packet).is_none());

        // UDP packet (not TCP)
        let mut udp_packet = vec![0u8; 40];
        udp_packet[0] = 0x45; // IPv4
        udp_packet[9] = 17; // Protocol: UDP
        assert!(TcpStreamId::from_ip_packet(&udp_packet).is_none());

        // Invalid IP version
        let mut invalid_packet = vec![0u8; 40];
        invalid_packet[0] = 0x35; // Version 3 (invalid)
        assert!(TcpStreamId::from_ip_packet(&invalid_packet).is_none());
    }

    #[test]
    fn test_tcp_stream_id_from_ipv6_packet() {
        // Create a minimal IPv6 + TCP packet
        // IPv6 header (40 bytes) + TCP header (20 bytes)
        let mut packet = vec![0u8; 60];

        // IPv6 header
        packet[0] = 0x60; // Version 6
        packet[6] = 6; // Next header: TCP

        // Source IP: 2001:db8::1
        packet[8..24].copy_from_slice(&[
            0x20, 0x01, 0x0d, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01,
        ]);

        // Destination IP: 2001:db8::2
        packet[24..40].copy_from_slice(&[
            0x20, 0x01, 0x0d, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x02,
        ]);

        // TCP header (starts at byte 40)
        // Source port: 8080 (0x1F90)
        packet[40..42].copy_from_slice(&[0x1F, 0x90]);

        // Destination port: 80 (0x0050)
        packet[42..44].copy_from_slice(&[0x00, 0x50]);

        let stream_id = TcpStreamId::from_ip_packet(&packet).unwrap();

        assert_eq!(
            stream_id.src_ip,
            IpAddr::V6(Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 1))
        );
        assert_eq!(
            stream_id.dst_ip,
            IpAddr::V6(Ipv6Addr::new(0x2001, 0x0db8, 0, 0, 0, 0, 0, 2))
        );
        assert_eq!(stream_id.src_port, 8080);
        assert_eq!(stream_id.dst_port, 80);
        assert_eq!(stream_id.protocol, 6);
    }
}
