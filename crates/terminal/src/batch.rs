//! Output batching and backpressure (addendum §3.3).
//!
//! A PTY reader must never block: blocking it stops the child, and a child
//! that writes faster than the renderer consumes (`yes`, `cat` of a large
//! file) would otherwise freeze the UI or grow memory without bound. So the
//! reader appends into a [`PendingOutput`] buffer with a hard byte cap and a
//! flusher drains it at a fixed rate.
//!
//! When the cap is reached the *oldest* bytes go, not the newest: a terminal
//! is a tail, and the newest output is what the user is waiting for. The
//! dropped byte count travels with the next batch so the UI can draw a visible
//! marker instead of silently lying about what the program printed.
//!
//! Bytes are decoded here rather than in the renderer because a batch boundary
//! falls in the middle of a UTF-8 sequence sooner or later. An incomplete tail
//! is held back for the next flush; a genuinely invalid sequence becomes
//! U+FFFD, which is what a terminal shows for such bytes anyway.

use std::time::Duration;

/// How often a session's pending output is flushed to the sink (~60 Hz).
pub const FLUSH_INTERVAL: Duration = Duration::from_millis(16);

/// Largest amount of undelivered output held per session before the oldest
/// bytes are dropped. One screenful of `yes` is a few kilobytes; a megabyte is
/// far more scrollback than a user can read between two frames.
pub const MAX_PENDING_BYTES: usize = 1024 * 1024;

/// Size of a single read from the PTY.
pub const READ_BUFFER_BYTES: usize = 64 * 1024;

/// One coalesced batch of terminal output.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutputBatch {
    /// Decoded output. Never empty when the batch is delivered.
    pub text: String,
    /// Bytes dropped before this batch because the consumer fell behind.
    /// Non-zero means output is genuinely missing and must be shown as such.
    pub truncated_bytes: u64,
}

/// The per-session buffer shared between the reader thread and the flusher.
#[derive(Debug)]
pub struct PendingOutput {
    /// Cap for this buffer. A field rather than the constant so a test can
    /// make overflow certain instead of hoping the machine outruns the
    /// flusher: whether a real flood overflows a 1 MiB buffer depends on how
    /// fast the machine is, which is not a property worth asserting.
    cap: usize,
    bytes: Vec<u8>,
    /// Bytes of an incomplete UTF-8 sequence held back from the last flush.
    tail: Vec<u8>,
    truncated_bytes: u64,
    /// Set once the session has exited; the flusher stops looking at it.
    finished: bool,
}

impl Default for PendingOutput {
    fn default() -> Self {
        Self::with_cap(MAX_PENDING_BYTES)
    }
}

impl PendingOutput {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_cap(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            bytes: Vec::new(),
            tail: Vec::new(),
            truncated_bytes: 0,
            finished: false,
        }
    }

    /// Append what the reader just read. Never blocks and never grows past
    /// [`MAX_PENDING_BYTES`]; the overflow is counted, not queued.
    pub fn push(&mut self, chunk: &[u8]) {
        // A single read larger than the whole cap: keep only its tail.
        if chunk.len() >= self.cap {
            let keep = self.cap;
            let dropped = self.bytes.len() as u64 + (chunk.len() - keep) as u64;
            self.bytes.clear();
            self.bytes.extend_from_slice(&chunk[chunk.len() - keep..]);
            self.truncated_bytes += dropped;
            return;
        }
        self.bytes.extend_from_slice(chunk);
        if self.bytes.len() > self.cap {
            let excess = self.bytes.len() - self.cap;
            self.bytes.drain(..excess);
            self.truncated_bytes += excess as u64;
        }
    }

    /// Whether a flush would produce anything.
    pub fn has_output(&self) -> bool {
        !self.bytes.is_empty() || self.truncated_bytes > 0
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Mark the session finished so no batch is delivered after the exit.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// Take everything decodable. Returns `None` when there is nothing to
    /// deliver, so the flusher never wakes the sink for an empty batch.
    pub fn take(&mut self) -> Option<OutputBatch> {
        let truncated_bytes = std::mem::take(&mut self.truncated_bytes);
        if self.bytes.is_empty() {
            // Dropped bytes still deserve a marker even with no text to show.
            return (truncated_bytes > 0).then(|| OutputBatch {
                text: String::new(),
                truncated_bytes,
            });
        }
        let mut raw = std::mem::take(&mut self.tail);
        raw.append(&mut self.bytes);
        let (text, tail) = decode_utf8_prefix(&raw);
        self.tail = tail;
        if text.is_empty() && truncated_bytes == 0 {
            return None;
        }
        Some(OutputBatch {
            text,
            truncated_bytes,
        })
    }

    /// Take everything including an undecodable tail; used once at EOF, where
    /// no further bytes can complete the sequence.
    pub fn take_final(&mut self) -> Option<OutputBatch> {
        let mut batch = self.take().unwrap_or_default();
        if !self.tail.is_empty() {
            let tail = std::mem::take(&mut self.tail);
            batch.text.push_str(&String::from_utf8_lossy(&tail));
        }
        (!batch.text.is_empty() || batch.truncated_bytes > 0).then_some(batch)
    }
}

/// Decode the longest valid prefix of `raw`, returning the decoded text and
/// the trailing bytes of an incomplete sequence (at most three).
///
/// Invalid sequences become U+FFFD and decoding continues; only a truncated
/// sequence at the very end is held back.
fn decode_utf8_prefix(raw: &[u8]) -> (String, Vec<u8>) {
    let mut text = String::with_capacity(raw.len());
    let mut rest = raw;
    loop {
        match std::str::from_utf8(rest) {
            Ok(valid) => {
                text.push_str(valid);
                return (text, Vec::new());
            }
            Err(error) => {
                let valid_up_to = error.valid_up_to();
                // Safe by construction: `from_utf8` just validated this range.
                text.push_str(std::str::from_utf8(&rest[..valid_up_to]).unwrap_or_default());
                match error.error_len() {
                    // Truncated at the end: the next read may complete it.
                    None => return (text, rest[valid_up_to..].to_vec()),
                    Some(invalid) => {
                        text.push(char::REPLACEMENT_CHARACTER);
                        rest = &rest[valid_up_to + invalid..];
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesces_reads_into_one_batch() {
        let mut pending = PendingOutput::new();
        pending.push(b"hello ");
        pending.push(b"world");
        let batch = pending.take().expect("a batch");
        assert_eq!(batch.text, "hello world");
        assert_eq!(batch.truncated_bytes, 0);
        assert!(pending.take().is_none(), "nothing left after a flush");
    }

    #[test]
    fn holds_back_an_incomplete_utf8_sequence() {
        let mut pending = PendingOutput::new();
        // "é" is 0xC3 0xA9; the read split it.
        pending.push(&[b'a', 0xC3]);
        let batch = pending.take().expect("a batch");
        assert_eq!(batch.text, "a", "the partial sequence is held back");

        pending.push(&[0xA9, b'b']);
        let batch = pending.take().expect("a batch");
        assert_eq!(batch.text, "éb", "the tail completes on the next flush");
    }

    #[test]
    fn replaces_genuinely_invalid_bytes_and_keeps_going() {
        let mut pending = PendingOutput::new();
        pending.push(&[b'a', 0xFF, b'b']);
        let batch = pending.take().expect("a batch");
        assert_eq!(batch.text, "a\u{FFFD}b");
    }

    #[test]
    fn drops_oldest_bytes_and_counts_them() {
        let mut pending = PendingOutput::new();
        pending.push(&vec![b'a'; MAX_PENDING_BYTES]);
        pending.push(b"tail");
        let batch = pending.take().expect("a batch");
        assert_eq!(batch.truncated_bytes, 4, "four oldest bytes made room");
        assert_eq!(batch.text.len(), MAX_PENDING_BYTES);
        assert!(batch.text.ends_with("tail"), "the newest output survives");
    }

    #[test]
    fn a_single_oversized_read_keeps_only_its_tail() {
        let mut pending = PendingOutput::new();
        pending.push(b"earlier");
        let mut flood = vec![b'x'; MAX_PENDING_BYTES];
        flood.extend_from_slice(b"newest");
        let overflow = flood.len() - MAX_PENDING_BYTES;
        pending.push(&flood);
        let batch = pending.take().expect("a batch");
        assert_eq!(batch.text.len(), MAX_PENDING_BYTES);
        assert!(batch.text.ends_with("newest"));
        assert_eq!(batch.truncated_bytes, (7 + overflow) as u64);
    }

    #[test]
    fn a_smaller_cap_is_honoured() {
        // What the integration test relies on to force overflow deterministically.
        let mut pending = PendingOutput::with_cap(16);
        pending.push(b"0123456789abcdefghij");
        let batch = pending.take().expect("a batch");
        assert_eq!(batch.text, "456789abcdefghij");
        assert_eq!(batch.truncated_bytes, 4);
    }

    #[test]
    fn memory_stays_bounded_under_a_sustained_flood() {
        let mut pending = PendingOutput::new();
        for _ in 0..64 {
            pending.push(&vec![b'y'; READ_BUFFER_BYTES]);
        }
        assert!(
            pending.bytes.len() <= MAX_PENDING_BYTES,
            "the buffer never grows past the cap"
        );
        let batch = pending.take().expect("a batch");
        assert!(batch.truncated_bytes > 0, "the drop is reported");
    }

    #[test]
    fn reports_dropped_bytes_even_with_no_text_left() {
        let mut pending = PendingOutput::new();
        pending.push(&vec![b'a'; MAX_PENDING_BYTES + 10]);
        let _ = pending.take();
        pending.truncated_bytes = 5;
        let batch = pending.take().expect("a marker batch");
        assert_eq!(batch.text, "");
        assert_eq!(batch.truncated_bytes, 5);
    }

    #[test]
    fn the_final_flush_gives_up_on_an_incomplete_tail() {
        let mut pending = PendingOutput::new();
        pending.push(&[b'a', 0xC3]);
        let batch = pending.take_final().expect("a batch");
        assert_eq!(
            batch.text, "a\u{FFFD}",
            "no further bytes can complete it at EOF"
        );
        assert!(pending.take_final().is_none());
    }

    #[test]
    fn nothing_to_flush_produces_no_batch() {
        let mut pending = PendingOutput::new();
        assert!(pending.take().is_none());
        assert!(!pending.has_output());
        pending.push(b"x");
        assert!(pending.has_output());
    }
}
