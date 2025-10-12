//! HTTP request parsing functions.
//!
//! This module provides functions for parsing HTTP requests from packet data,
//! including support for obsolete line folding (RFC 7230).

use memchr::memmem;
use std::str;

/// Find HTTP request data within a packet payload
pub(crate) fn find_http_in_packet(data: &[u8]) -> Option<String> {
    memmem::find(data, b"GET ")
        .map(|pos| &data[pos..])
        .map(|r| match memmem::find(r, b"\r\n\r\n") {
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

/// Parse an HTTP request and extract the URL
pub(crate) fn parse_http_request(http_data: &str) -> Option<String> {
    // First, unfold any multi-line headers (obsolete line folding per RFC 7230)
    // Line folding is when a header value continues on the next line starting with whitespace
    let unfolded = unfold_http_headers(http_data);

    let mut lines = unfolded.lines();

    let request_line = lines.next()?.trim();
    let mut parts = request_line.split_whitespace();
    let _method = parts.next()?;

    let path = parts.next()?.trim_start_matches('/');

    if path.starts_with("http://") || path.starts_with("https://") {
        return Some(path.to_owned());
    }
    let path = path.trim_start_matches('/');

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

/// Unfold HTTP headers that use obsolete line folding (RFC 7230 Section 3.2.4)
/// Line folding occurs when a header field value is continued on the next line
/// starting with at least one space or tab character.
pub(crate) fn unfold_http_headers(http_data: &str) -> String {
    let mut result = String::with_capacity(http_data.len());
    let mut lines = http_data.lines();

    // First line is the request line, never folded
    if let Some(first_line) = lines.next() {
        result.push_str(first_line);
        result.push_str("\r\n");
    }

    let mut current_line = String::new();

    for line in lines {
        // Check if this line is a continuation (starts with space or tab)
        if line.starts_with(' ') || line.starts_with('\t') {
            // This is a folded line - append to current line with a space
            current_line.push(' ');
            current_line.push_str(line.trim());
        } else {
            // This is a new line - flush the current line if any
            if !current_line.is_empty() {
                result.push_str(&current_line);
                result.push_str("\r\n");
                current_line.clear();
            }
            current_line.push_str(line);
        }
    }

    // Don't forget the last line
    if !current_line.is_empty() {
        result.push_str(&current_line);
        result.push_str("\r\n");
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unfold_http_headers_simple() {
        // Test simple case with no folding
        let http_data = "GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let unfolded = unfold_http_headers(http_data);
        assert!(unfolded.contains("Host: example.com"));
    }

    #[test]
    fn test_unfold_http_headers_with_folding() {
        // Test obsolete line folding with space continuation
        let http_data =
            "GET / HTTP/1.1\r\nHost: example.com\r\nX-Custom-Header: value1\r\n value2\r\n\r\n";
        let unfolded = unfold_http_headers(http_data);
        assert!(
            unfolded.contains("X-Custom-Header: value1 value2"),
            "Folded header should be unfolded. Got: {unfolded}"
        );
    }

    #[test]
    fn test_unfold_http_headers_with_tab_folding() {
        // Test obsolete line folding with tab continuation
        let http_data =
            "GET / HTTP/1.1\r\nHost: example.com\r\nX-Custom-Header: value1\r\n\tvalue2\r\n\r\n";
        let unfolded = unfold_http_headers(http_data);
        assert!(
            unfolded.contains("X-Custom-Header: value1 value2"),
            "Tab-folded header should be unfolded. Got: {unfolded}"
        );
    }

    #[test]
    fn test_parse_http_request_with_folded_host_header() {
        // Test parsing HTTP request with folded Host header
        let http_data = "GET /path HTTP/1.1\r\nHost: replay404\r\n .valve.net\r\n\r\n";
        let url = parse_http_request(http_data);
        assert_eq!(
            url,
            Some("http://replay404 .valve.net/path".to_string()),
            "Should parse folded Host header"
        );
    }

    #[test]
    fn test_parse_http_request() {
        let http_data = "GET / HTTP/1.1\r\nHost: www.example.com\r\n\r\n";
        assert_eq!(
            parse_http_request(http_data).unwrap(),
            "http://www.example.com/"
        );
    }

    #[test]
    fn test_find_http_in_packet() {
        let payload =
            b"\x00\x01randomdataGET /path HTTP/1.1\r\nHost: example.com\r\n\r\nmore".to_vec();
        let found = find_http_in_packet(&payload).unwrap();
        assert!(found.contains("GET /path HTTP/1.1"));
    }
}
