//! Pure `Content-Length` frame codec for the LSP wire protocol.
//!
//! LSP messages are framed like HTTP headers: a `Content-Length: N\r\n`
//! header block terminated by a blank line, followed by exactly `N` bytes
//! of JSON payload. This module only deals in bytes — no process I/O — so
//! it can be fed arbitrary fragments (a byte at a time, several frames at
//! once, or a frame split mid-header) and unit tested exhaustively.

use thiserror::Error;

/// Header blocks larger than this (with no `\r\n\r\n` terminator yet found)
/// are treated as malformed rather than buffered forever.
pub(crate) const MAX_HEADER_BYTES: usize = 4 * 1024;

/// Declared `Content-Length` values above this are rejected rather than
/// buffered, to bound memory use against a hostile or broken server.
pub(crate) const MAX_CONTENT_BYTES: usize = 64 * 1024 * 1024;

/// Errors produced while decoding a `Content-Length`-framed byte stream.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CodecError {
    /// The header block had no `Content-Length` line.
    #[error("frame header had no Content-Length line")]
    MissingContentLength,
    /// The `Content-Length` value was present but not a valid `usize`.
    #[error("invalid Content-Length value: {0}")]
    InvalidContentLength(String),
    /// No `\r\n\r\n` header terminator was found within
    /// [`MAX_HEADER_BYTES`].
    #[error("frame header exceeded {MAX_HEADER_BYTES} bytes without a terminator")]
    HeaderTooLarge,
    /// The declared `Content-Length` exceeded [`MAX_CONTENT_BYTES`].
    #[error("frame declared length {0} exceeds the maximum of {MAX_CONTENT_BYTES}")]
    FrameTooLarge(usize),
}

/// Wraps `payload` in a `Content-Length: N\r\n\r\n` frame.
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let header = format!("Content-Length: {}\r\n\r\n", payload.len());
    let mut out = Vec::with_capacity(header.len() + payload.len());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(payload);
    out
}

/// Incremental decoder that tolerates arbitrary read fragmentation: bytes
/// can be fed in any chunk size and complete frames are extracted as they
/// become available.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    /// Creates an empty decoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends freshly read bytes to the internal buffer.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Attempts to extract one complete frame from the buffer.
    ///
    /// Returns `Ok(Some(payload))` when a full frame was removed from the
    /// front of the buffer, `Ok(None)` when more bytes are needed, and
    /// `Err` when the buffered header is malformed. Every error path
    /// discards at least the offending header (so the decoder resyncs onto
    /// the next frame) — calling `next_frame` repeatedly after an error
    /// always makes progress rather than looping forever on the same bad
    /// bytes.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, CodecError> {
        let header_end = match find_header_terminator(&self.buf) {
            Some(pos) => pos,
            None => {
                if self.buf.len() > MAX_HEADER_BYTES {
                    self.buf.clear();
                    return Err(CodecError::HeaderTooLarge);
                }
                return Ok(None);
            }
        };

        let terminator_end = header_end + 4; // len of b"\r\n\r\n"
        let header_block = &self.buf[..header_end];

        let content_length = match parse_content_length(header_block) {
            Ok(len) => len,
            Err(err) => {
                self.buf.drain(..terminator_end);
                return Err(err);
            }
        };

        if content_length > MAX_CONTENT_BYTES {
            self.buf.drain(..terminator_end);
            return Err(CodecError::FrameTooLarge(content_length));
        }

        let frame_end = terminator_end + content_length;
        if self.buf.len() < frame_end {
            return Ok(None);
        }

        let payload = self.buf[terminator_end..frame_end].to_vec();
        self.buf.drain(..frame_end);
        Ok(Some(payload))
    }
}

/// Finds the byte offset of the start of the first `\r\n\r\n` in `buf`, if
/// any.
fn find_header_terminator(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Parses a header block (the bytes before `\r\n\r\n`) for a
/// case-insensitive `Content-Length` line. Other headers (e.g.
/// `Content-Type`) are ignored.
fn parse_content_length(header_block: &[u8]) -> Result<usize, CodecError> {
    let header_str = String::from_utf8_lossy(header_block);
    for line in header_str.split("\r\n") {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            let value = value.trim();
            return value
                .parse::<usize>()
                .map_err(|_| CodecError::InvalidContentLength(value.to_string()));
        }
    }
    Err(CodecError::MissingContentLength)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encode_decode() {
        let payload = b"{\"jsonrpc\":\"2.0\"}";
        let frame = encode_frame(payload);
        let mut decoder = FrameDecoder::new();
        decoder.feed(&frame);
        assert_eq!(decoder.next_frame(), Ok(Some(payload.to_vec())));
        assert_eq!(decoder.next_frame(), Ok(None));
    }

    #[test]
    fn empty_payload_roundtrips() {
        let frame = encode_frame(b"");
        let mut decoder = FrameDecoder::new();
        decoder.feed(&frame);
        assert_eq!(decoder.next_frame(), Ok(Some(Vec::new())));
    }

    #[test]
    fn feed_one_byte_at_a_time() {
        let payload = b"hello world";
        let frame = encode_frame(payload);
        let mut decoder = FrameDecoder::new();
        for (i, byte) in frame.iter().enumerate() {
            decoder.feed(std::slice::from_ref(byte));
            let result = decoder.next_frame().expect("no error expected");
            if i + 1 < frame.len() {
                assert_eq!(result, None, "should not complete before all bytes fed");
            } else {
                assert_eq!(result, Some(payload.to_vec()));
            }
        }
    }

    #[test]
    fn three_frames_in_one_feed_drain_individually() {
        let p1 = b"one".to_vec();
        let p2 = b"two".to_vec();
        let p3 = b"three".to_vec();
        let mut combined = Vec::new();
        combined.extend(encode_frame(&p1));
        combined.extend(encode_frame(&p2));
        combined.extend(encode_frame(&p3));

        let mut decoder = FrameDecoder::new();
        decoder.feed(&combined);
        assert_eq!(decoder.next_frame(), Ok(Some(p1)));
        assert_eq!(decoder.next_frame(), Ok(Some(p2)));
        assert_eq!(decoder.next_frame(), Ok(Some(p3)));
        assert_eq!(decoder.next_frame(), Ok(None));
    }

    #[test]
    fn frame_split_at_mid_payload_boundary() {
        let payload = b"0123456789";
        let frame = encode_frame(payload);
        let split_at = frame.len() - 4; // splits inside the payload
        let mut decoder = FrameDecoder::new();
        decoder.feed(&frame[..split_at]);
        assert_eq!(decoder.next_frame(), Ok(None));
        decoder.feed(&frame[split_at..]);
        assert_eq!(decoder.next_frame(), Ok(Some(payload.to_vec())));
    }

    #[test]
    fn frame_split_at_mid_header_boundary() {
        let payload = b"abc";
        let frame = encode_frame(payload);
        // "Content-Length: 3\r\n\r\n" -- split in the middle of the header line.
        let split_at = 5;
        let mut decoder = FrameDecoder::new();
        decoder.feed(&frame[..split_at]);
        assert_eq!(decoder.next_frame(), Ok(None));
        decoder.feed(&frame[split_at..]);
        assert_eq!(decoder.next_frame(), Ok(Some(payload.to_vec())));
    }

    #[test]
    fn header_name_is_case_insensitive() {
        for header_name in ["content-length", "CONTENT-LENGTH"] {
            let payload = b"hi";
            let raw = format!(
                "{}: {}\r\n\r\n{}",
                header_name,
                payload.len(),
                std::str::from_utf8(payload).expect("valid utf8")
            );
            let mut decoder = FrameDecoder::new();
            decoder.feed(raw.as_bytes());
            assert_eq!(decoder.next_frame(), Ok(Some(payload.to_vec())));
        }
    }

    #[test]
    fn extra_content_type_header_is_ignored() {
        let payload = b"payload";
        let raw = format!(
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
            payload.len(),
            std::str::from_utf8(payload).expect("valid utf8")
        );
        let mut decoder = FrameDecoder::new();
        decoder.feed(raw.as_bytes());
        assert_eq!(decoder.next_frame(), Ok(Some(payload.to_vec())));
    }

    #[test]
    fn missing_content_length_errors_and_then_resyncs() {
        let mut decoder = FrameDecoder::new();
        decoder.feed(b"Content-Type: application/json\r\n\r\n");
        assert_eq!(decoder.next_frame(), Err(CodecError::MissingContentLength));

        // A valid frame fed afterwards still decodes correctly.
        let payload = b"ok";
        decoder.feed(&encode_frame(payload));
        assert_eq!(decoder.next_frame(), Ok(Some(payload.to_vec())));
    }

    #[test]
    fn non_numeric_length_is_an_error() {
        let mut decoder = FrameDecoder::new();
        decoder.feed(b"Content-Length: not-a-number\r\n\r\n");
        assert_eq!(
            decoder.next_frame(),
            Err(CodecError::InvalidContentLength("not-a-number".to_string()))
        );
    }

    #[test]
    fn length_over_max_is_frame_too_large() {
        let mut decoder = FrameDecoder::new();
        let declared = MAX_CONTENT_BYTES + 1;
        decoder.feed(format!("Content-Length: {declared}\r\n\r\n").as_bytes());
        assert_eq!(
            decoder.next_frame(),
            Err(CodecError::FrameTooLarge(declared))
        );
    }

    #[test]
    fn garbage_without_terminator_exceeding_max_header_is_header_too_large() {
        let mut decoder = FrameDecoder::new();
        decoder.feed(&vec![b'x'; MAX_HEADER_BYTES + 1]);
        assert_eq!(decoder.next_frame(), Err(CodecError::HeaderTooLarge));
        // Buffer was cleared on the error.
        assert_eq!(decoder.next_frame(), Ok(None));
    }
}
