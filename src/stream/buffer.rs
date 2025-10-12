//! TCP stream buffer for reassembling fragmented HTTP requests.
//!
//! This module provides the `StreamBuffer` struct which accumulates packet data
//! from a single TCP stream until a complete HTTP request can be extracted.

use core::time::Duration;
use std::time::Instant;

const MAX_STREAM_BUFFER_SIZE: usize = 16 * 1024;

/// Represents a TCP stream buffer for reassembling fragmented HTTP requests
#[derive(Debug)]
pub(crate) struct StreamBuffer {
    pub(crate) data: Vec<u8>,
    last_activity: Instant,
}

impl StreamBuffer {
    pub(crate) fn new() -> Self {
        Self {
            data: Vec::new(),
            last_activity: Instant::now(),
        }
    }

    pub(crate) fn append(&mut self, payload: &[u8]) {
        // Prevent buffer from growing too large
        if self.data.len() + payload.len() <= MAX_STREAM_BUFFER_SIZE {
            self.data.extend_from_slice(payload);
            self.last_activity = Instant::now();
        }
    }

    pub(crate) fn clear(&mut self) {
        self.data.clear();
        self.last_activity = Instant::now();
    }

    pub(crate) fn is_stale(&self, timeout: Duration) -> bool {
        self.last_activity.elapsed() > timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_buffer_operations() {
        let mut buffer = StreamBuffer::new();

        // Test append
        buffer.append(b"Hello ");
        buffer.append(b"World");
        assert_eq!(buffer.data, b"Hello World");

        // Test clear
        buffer.clear();
        assert!(buffer.data.is_empty());

        // Test max size limit
        let large_data = vec![0u8; MAX_STREAM_BUFFER_SIZE + 1000];
        buffer.append(&large_data);
        assert!(
            buffer.data.len() <= MAX_STREAM_BUFFER_SIZE,
            "Buffer should not exceed max size"
        );
    }

    #[test]
    fn test_stream_buffer_staleness() {
        let mut buffer = StreamBuffer::new();
        buffer.append(b"test");

        // Should not be stale immediately
        assert!(!buffer.is_stale(Duration::from_secs(1)));

        // Simulate time passing (we can't actually wait, so this tests the logic)
        // In real usage, buffers would become stale after the timeout period
    }
}
