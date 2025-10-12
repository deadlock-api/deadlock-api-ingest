//! TCP stream reassembly and buffering.
//!
//! This module provides functionality for reassembling TCP streams from individual
//! packets. It handles buffering of packet data until complete HTTP requests can
//! be extracted.

mod buffer;

pub(crate) use buffer::StreamBuffer;
