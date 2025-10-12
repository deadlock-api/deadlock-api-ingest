//! HTTP request parsing.
//!
//! This module provides functionality for parsing HTTP requests from packet data,
//! including handling of multi-line headers and URL extraction.

mod parser;

pub(crate) use parser::{find_http_in_packet, parse_http_request};
