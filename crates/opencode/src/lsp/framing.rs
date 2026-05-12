//! LSP wire framing: Content-Length headers around JSON payloads.
//!
//! Every LSP message is:
//!
//!   Content-Length: <N>\r\n
//!   \r\n
//!   <N bytes of JSON>
//!
//! Optional headers (Content-Type) are allowed before the blank line.
//! Servers in the wild only send Content-Length, so we accept any
//! header but only require Content-Length.
//!
//! This module is pure — no I/O — so it can be unit-tested without
//! spawning processes.

use std::io::{self, BufRead, Read, Write};

/// Encode a JSON value into LSP wire format, writing the result to `out`.
pub fn write_message(out: &mut impl Write, body: &[u8]) -> io::Result<()> {
    write!(out, "Content-Length: {}\r\n\r\n", body.len())?;
    out.write_all(body)?;
    out.flush()
}

/// Read one complete LSP message from `r`. Returns the payload bytes
/// (the JSON body) without any framing. Returns `Ok(None)` on clean
/// EOF before any header bytes were read.
pub fn read_message(r: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        let n = r.read_line(&mut line)?;
        if n == 0 {
            // Clean EOF if we haven't seen any header yet; otherwise
            // a truncated message.
            return if content_length.is_none() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF in LSP header",
                ))
            };
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        // Headers are case-insensitive per HTTP convention. The only
        // one LSP requires is Content-Length.
        if let Some(rest) = line
            .to_ascii_lowercase()
            .strip_prefix("content-length:")
        {
            let s = rest.trim();
            content_length = Some(s.parse().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid Content-Length: {}", s),
                )
            })?);
        }
        // Unknown headers are silently dropped (servers occasionally
        // send a Content-Type we don't care about).
    }

    let len = content_length.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "LSP message missing Content-Length header",
        )
    })?;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn roundtrip_empty_object() {
        let mut out = Vec::new();
        write_message(&mut out, b"{}").unwrap();
        let mut cur = Cursor::new(out);
        let body = read_message(&mut cur).unwrap().unwrap();
        assert_eq!(body, b"{}");
    }

    #[test]
    fn roundtrip_with_utf8_payload() {
        let payload = r#"{"jsonrpc":"2.0","id":1,"result":"héllo 🦀"}"#;
        let mut out = Vec::new();
        write_message(&mut out, payload.as_bytes()).unwrap();
        let mut cur = Cursor::new(out);
        let body = read_message(&mut cur).unwrap().unwrap();
        assert_eq!(body, payload.as_bytes());
    }

    #[test]
    fn header_is_case_insensitive() {
        // Lowercase content-length should still parse.
        let raw = b"content-length: 7\r\n\r\n{\"k\":1}";
        let mut cur = Cursor::new(&raw[..]);
        let body = read_message(&mut cur).unwrap().unwrap();
        assert_eq!(body, br#"{"k":1}"#);
    }

    #[test]
    fn extra_headers_are_ignored() {
        let raw = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: 2\r\n\r\n{}";
        let mut cur = Cursor::new(&raw[..]);
        let body = read_message(&mut cur).unwrap().unwrap();
        assert_eq!(body, b"{}");
    }

    #[test]
    fn lf_only_blank_separator_accepted() {
        // Some servers send LF instead of CRLF for the blank line. We
        // accept both per RFC 7230 §3.5.
        let raw = b"Content-Length: 2\n\n{}";
        let mut cur = Cursor::new(&raw[..]);
        let body = read_message(&mut cur).unwrap().unwrap();
        assert_eq!(body, b"{}");
    }

    #[test]
    fn clean_eof_before_any_header_returns_none() {
        let raw: &[u8] = b"";
        let mut cur = Cursor::new(raw);
        assert!(read_message(&mut cur).unwrap().is_none());
    }

    #[test]
    fn truncated_body_is_unexpected_eof() {
        let raw = b"Content-Length: 100\r\n\r\n{}";
        let mut cur = Cursor::new(&raw[..]);
        let err = read_message(&mut cur).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn missing_content_length_is_invalid_data() {
        let raw = b"X-Foo: bar\r\n\r\n{}";
        let mut cur = Cursor::new(&raw[..]);
        let err = read_message(&mut cur).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn non_numeric_content_length_is_invalid_data() {
        let raw = b"Content-Length: many\r\n\r\n";
        let mut cur = Cursor::new(&raw[..]);
        let err = read_message(&mut cur).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
