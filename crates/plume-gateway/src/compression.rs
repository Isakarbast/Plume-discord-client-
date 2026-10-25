//! `compress=zlib-stream` support.
//!
//! Discord compresses the whole connection as *one* zlib stream. A payload
//! can arrive split over several WebSocket messages; the last piece of each
//! payload ends with the zlib "sync flush" marker `00 00 FF FF`.

use flate2::{Decompress, FlushDecompress};
use thiserror::Error;

const ZLIB_SUFFIX: [u8; 4] = [0x00, 0x00, 0xff, 0xff];

/// After a huge payload (READY can be several MB) we hand the memory back
/// instead of keeping the big buffers around forever.
const KEEP_CAPACITY: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum CompressionError {
    #[error("zlib error: {0}")]
    Zlib(#[from] flate2::DecompressError),
    #[error("decompressed payload is not valid UTF-8")]
    Utf8(#[from] std::string::FromUtf8Error),
}

pub struct ZlibStream {
    inflater: Decompress,
    /// Compressed bytes waiting for the end-of-payload marker.
    input: Vec<u8>,
}

impl ZlibStream {
    pub fn new() -> Self {
        ZlibStream {
            inflater: Decompress::new(true),
            input: Vec::new(),
        }
    }

    /// Feeds one binary WebSocket message. Returns the JSON text once a whole
    /// payload has arrived, or `None` if we're still waiting for more pieces.
    pub fn push(&mut self, data: &[u8]) -> Result<Option<String>, CompressionError> {
        self.input.extend_from_slice(data);
        if !self.input.ends_with(&ZLIB_SUFFIX) {
            return Ok(None);
        }

        let mut output = Vec::with_capacity(self.input.len() * 4);
        let mut remaining = &self.input[..];
        loop {
            if output.capacity() == output.len() {
                output.reserve(output.capacity().max(KEEP_CAPACITY));
            }
            let before = self.inflater.total_in();
            self.inflater
                .decompress_vec(remaining, &mut output, FlushDecompress::Sync)?;
            let consumed = (self.inflater.total_in() - before) as usize;
            remaining = &remaining[consumed..];

            // Done when all input is used *and* zlib didn't fill the buffer
            // (a full buffer means it may have more output waiting).
            if remaining.is_empty() && output.len() < output.capacity() {
                break;
            }
        }

        self.input.clear();
        self.input.shrink_to(KEEP_CAPACITY);
        Ok(Some(String::from_utf8(output)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

    /// Compresses like Discord does: one stream, sync-flushed per payload.
    fn compress_payloads(payloads: &[&str]) -> Vec<Vec<u8>> {
        let mut compressor = Compress::new(Compression::default(), true);
        payloads
            .iter()
            .map(|payload| {
                let mut out = Vec::with_capacity(payload.len() + 64);
                compressor
                    .compress_vec(payload.as_bytes(), &mut out, FlushCompress::Sync)
                    .unwrap();
                assert!(out.ends_with(&ZLIB_SUFFIX));
                out
            })
            .collect()
    }

    #[test]
    fn decodes_consecutive_payloads_sharing_one_stream() {
        let chunks = compress_payloads(&[r#"{"op":10}"#, r#"{"op":11}"#]);
        let mut stream = ZlibStream::new();
        assert_eq!(
            stream.push(&chunks[0]).unwrap().as_deref(),
            Some(r#"{"op":10}"#)
        );
        assert_eq!(
            stream.push(&chunks[1]).unwrap().as_deref(),
            Some(r#"{"op":11}"#)
        );
    }

    #[test]
    fn waits_for_the_end_marker_when_a_payload_is_split() {
        let chunks = compress_payloads(&[r#"{"op":0,"t":"READY"}"#]);
        let (first, second) = chunks[0].split_at(chunks[0].len() / 2);
        let mut stream = ZlibStream::new();
        assert_eq!(stream.push(first).unwrap(), None);
        assert_eq!(
            stream.push(second).unwrap().as_deref(),
            Some(r#"{"op":0,"t":"READY"}"#)
        );
    }

    #[test]
    fn handles_payloads_much_larger_than_the_input() {
        // Very repetitive JSON compresses ~100x, which exercises the
        // "buffer full, keep going" path.
        let big = format!(r#"{{"d":"{}"}}"#, "a".repeat(1_000_000));
        let chunks = compress_payloads(&[&big]);
        let mut stream = ZlibStream::new();
        assert_eq!(
            stream.push(&chunks[0]).unwrap().as_deref(),
            Some(big.as_str())
        );
    }
}
