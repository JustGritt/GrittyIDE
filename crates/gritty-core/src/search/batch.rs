//! Bounded, throttled delivery of streamed matches (hard rule 8).
//!
//! A search can find matches far faster than a WebView can render them, so
//! three bounds sit between the walk and the renderer, and every one of them
//! is *reported* rather than silent:
//!
//! | Bound | Value | What happens at it |
//! |---|---|---|
//! | [`MAX_BATCH_MATCHES`] | 200 | one event carries at most this many |
//! | [`MIN_BATCH_INTERVAL`] | 50 ms | at most one event per interval per operation |
//! | [`MAX_PENDING_MATCHES`] | 2000 | further matches are dropped and counted in `truncatedMatches` |
//! | [`MAX_MATCHES`] | 20 000 | the walk stops; the summary says `limitReached` |
//!
//! The interval is [`MIN_PROGRESS_INTERVAL`] — the same number
//! `OperationProgress` is throttled by, deliberately, so the renderer sees one
//! rhythm rather than two.
//!
//! ## Why this does not reuse [`Throttle`]
//!
//! [`Throttle`] measures with `tokio::time::Instant`, which is the runtime's
//! clock. This batcher runs on the blocking pool, where a paused test clock
//! would freeze delivery rather than model it, so it measures with
//! `std::time::Instant` and takes `now` as an argument — which is also what
//! makes the table above testable without sleeping.
//!
//! [`Throttle`]: crate::operation::progress::Throttle
//! [`MIN_PROGRESS_INTERVAL`]: crate::operation::progress::MIN_PROGRESS_INTERVAL

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use gritty_search::SearchMatch;

use crate::operation::progress::MIN_PROGRESS_INTERVAL;

/// Matches in one `SearchMatchesFound` event.
pub const MAX_BATCH_MATCHES: usize = 200;
/// Matches held while waiting for the next interval; beyond this they are
/// dropped and counted.
pub const MAX_PENDING_MATCHES: usize = 2_000;
/// Minimum spacing between two `SearchMatchesFound` events of one search.
pub const MIN_BATCH_INTERVAL: Duration = MIN_PROGRESS_INTERVAL;
/// Matches after which the walk is asked to stop. A result list nobody can
/// read is not worth the rest of the tree.
pub const MAX_MATCHES: u64 = 20_000;

/// One event's worth of matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub matches: Vec<SearchMatch>,
    /// Matches dropped since the previous batch.
    pub truncated: u64,
}

/// Accumulates matches and hands them out in bounded, spaced batches.
#[derive(Debug)]
pub struct MatchBatcher {
    pending: VecDeque<SearchMatch>,
    /// Dropped since the last batch was taken.
    dropped: u64,
    /// Dropped over the whole search.
    dropped_total: u64,
    accepted: u64,
    last_batch: Option<Instant>,
    max_batch: usize,
    max_pending: usize,
    max_matches: u64,
    min_interval: Duration,
}

impl Default for MatchBatcher {
    fn default() -> Self {
        Self::new(
            MAX_BATCH_MATCHES,
            MAX_PENDING_MATCHES,
            MAX_MATCHES,
            MIN_BATCH_INTERVAL,
        )
    }
}

impl MatchBatcher {
    pub fn new(
        max_batch: usize,
        max_pending: usize,
        max_matches: u64,
        min_interval: Duration,
    ) -> Self {
        Self {
            pending: VecDeque::new(),
            dropped: 0,
            dropped_total: 0,
            accepted: 0,
            last_batch: None,
            max_batch: max_batch.max(1),
            max_pending,
            max_matches,
            min_interval,
        }
    }

    /// Offer one match. `false` means the overall cap is reached and the walk
    /// should stop; the match itself is still counted.
    pub fn push(&mut self, found: SearchMatch) -> bool {
        self.accepted += 1;
        if self.pending.len() >= self.max_pending {
            self.dropped += 1;
            self.dropped_total += 1;
        } else {
            self.pending.push_back(found);
        }
        self.accepted < self.max_matches
    }

    /// The batch due at `now`, if any.
    pub fn take(&mut self, now: Instant) -> Option<Batch> {
        if self.pending.is_empty() && self.dropped == 0 {
            return None;
        }
        if self
            .last_batch
            .is_some_and(|last| now.duration_since(last) < self.min_interval)
        {
            return None;
        }
        self.last_batch = Some(now);
        Some(self.next_batch())
    }

    /// Everything still held, in batches of at most [`MAX_BATCH_MATCHES`].
    /// Called once when the walk ends, so a search that finished faster than
    /// the interval still delivers what it found.
    pub fn drain(&mut self) -> Vec<Batch> {
        let mut batches = Vec::new();
        while !self.pending.is_empty() || self.dropped > 0 {
            batches.push(self.next_batch());
        }
        batches
    }

    /// Matches dropped across the whole search.
    pub const fn dropped_total(&self) -> u64 {
        self.dropped_total
    }

    fn next_batch(&mut self) -> Batch {
        let take = self.pending.len().min(self.max_batch);
        let matches: Vec<SearchMatch> = self.pending.drain(..take).collect();
        Batch {
            matches,
            truncated: std::mem::take(&mut self.dropped),
        }
    }
}

#[cfg(test)]
mod tests {
    use gritty_search::MatchKind;
    use gritty_workspace::WorkspacePath;

    use super::*;

    fn a_match(line: u64) -> SearchMatch {
        SearchMatch {
            path: WorkspacePath::parse("src/main.rs").unwrap(),
            kind: MatchKind::Content,
            line: Some(line),
            column: Some(1),
            snippet: "needle".to_owned(),
            truncated: false,
            match_start: 0,
            match_length: 6,
        }
    }

    fn batcher() -> MatchBatcher {
        MatchBatcher::new(3, 5, 100, Duration::from_millis(50))
    }

    #[test]
    fn nothing_is_delivered_before_there_is_anything_to_deliver() {
        let mut batcher = batcher();
        assert_eq!(batcher.take(Instant::now()), None);
    }

    #[test]
    fn a_batch_is_bounded_and_the_rest_waits() {
        let mut batcher = batcher();
        for line in 1..=5 {
            assert!(batcher.push(a_match(line)));
        }
        let start = Instant::now();
        let first = batcher.take(start).expect("first batch");
        assert_eq!(first.matches.len(), 3, "bounded to max_batch");
        assert_eq!(first.truncated, 0);
        assert_eq!(
            batcher.take(start + Duration::from_millis(10)),
            None,
            "throttled"
        );
        let second = batcher
            .take(start + Duration::from_millis(50))
            .expect("second batch");
        assert_eq!(second.matches.len(), 2);
    }

    #[test]
    fn matches_beyond_the_pending_bound_are_dropped_and_counted_not_silent() {
        let mut batcher = batcher();
        for line in 1..=8 {
            assert!(batcher.push(a_match(line)));
        }
        assert_eq!(batcher.dropped_total(), 3, "5 held, 3 dropped");
        let batch = batcher.take(Instant::now()).expect("batch");
        assert_eq!(batch.truncated, 3, "the count travels with the next batch");
        let rest = batcher.drain();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].matches.len(), 2);
        assert_eq!(rest[0].truncated, 0, "reported once, not repeatedly");
    }

    #[test]
    fn the_overall_cap_asks_the_walk_to_stop() {
        let mut batcher = MatchBatcher::new(3, 100, 3, Duration::ZERO);
        assert!(batcher.push(a_match(1)));
        assert!(batcher.push(a_match(2)));
        assert!(!batcher.push(a_match(3)), "the third reaches the cap");
    }

    #[test]
    fn draining_reports_a_truncation_that_never_got_a_batch() {
        let mut batcher = MatchBatcher::new(3, 0, 100, Duration::from_secs(60));
        assert!(batcher.push(a_match(1)));
        assert_eq!(batcher.dropped_total(), 1);
        let batches = batcher.drain();
        assert_eq!(batches.len(), 1);
        assert!(batches[0].matches.is_empty());
        assert_eq!(
            batches[0].truncated, 1,
            "an empty batch still has to say a match went missing"
        );
    }

    #[test]
    fn draining_splits_into_bounded_batches() {
        let mut batcher = MatchBatcher::new(3, 100, 100, Duration::from_secs(60));
        for line in 1..=7 {
            assert!(batcher.push(a_match(line)));
        }
        let batches = batcher.drain();
        assert_eq!(
            batches.iter().map(|b| b.matches.len()).collect::<Vec<_>>(),
            vec![3, 3, 1]
        );
    }
}
