//! Packet parsing and TCP stream identification.
//!
//! This module provides functionality for parsing network packets and extracting
//! TCP connection information. It supports both IPv4 and IPv6 packets in various
//! formats (Ethernet frames or raw IP packets).

mod tcp_stream_id;

pub(crate) use tcp_stream_id::TcpStreamId;
