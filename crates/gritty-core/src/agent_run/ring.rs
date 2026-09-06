//! The per-run event ring (T300-CORE, §7's half of the contract that needed a
//! manager to be true).
//!
//! **Why a ring.** A run's event stream grows without bound and a renderer only
//! ever draws a tail of it, so a bounded buffer that evicts from the *oldest*
//! end is the shape — the one [`crate::agent_session::timeline`] already uses
//! for observed sessions, which took it in turn from the terminal's output
//! batcher. What is copied is not the code but the rule: **report the loss,
//! never hide it**. An evicted record is counted, the count crosses the
//! protocol, and the UI draws a gap instead of splicing unrelated entries
//! together.
//!
//! **Why a batch.** Rule 8 forbids unbounded high-frequency forwarding to the
//! renderer. Records accumulate in `undelivered` and leave as one coalesced
//! [`AgentRunEventAppendedPayload`]; a step that produced fifty events becomes
//! one event to the WebView, never fifty.
//!
//! **What `dropped` means.** Unlike the agent timeline's, both sides of this
//! protocol carry the **cumulative** total, because that is what
//! [`AgentRunEventAppendedPayload::dropped`]'s contract says: "a client compares
//! it with what it last saw to know a gap opened". So a batch is worth emitting
//! when it has records *or* when the total moved since the last one — a burst
//! that overflowed the ring must still be able to report that it did.

use std::collections::VecDeque;

use super::wire::{
    AgentRunEvent, AgentRunEventAppendedPayload, AgentRunEventLog, AgentRunEventRecord,
    MAX_RUN_EVENTS,
};
use crate::protocol::ids::AgentRunId;
use crate::protocol::time::Timestamp;

/// One run's events: what is kept, what has not been delivered yet, and what
/// was lost.
pub(super) struct EventRing {
    /// Newest last, oldest first — the eviction end.
    events: VecDeque<AgentRunEventRecord>,
    /// Recorded but not yet delivered. Drained by [`Self::take_batch`], so a
    /// hundred records become one event rather than a hundred (rule 8).
    undelivered: Vec<AgentRunEventRecord>,
    /// Cumulative evictions since the run started.
    dropped: u32,
    /// The `dropped` total the last delivered batch carried, so a batch that
    /// only reports new loss still goes out.
    delivered_dropped: u32,
    /// Strictly increasing, per run. Carries order and nothing else.
    next_seq: u64,
}

impl EventRing {
    pub(super) fn new() -> Self {
        Self {
            events: VecDeque::new(),
            undelivered: Vec::new(),
            dropped: 0,
            delivered_dropped: 0,
            next_seq: 0,
        }
    }

    /// Record one event, bounding it on the way in.
    ///
    /// [`AgentRunEventRecord::new`] is the bounding seam
    /// ([`AgentRunEvent::bounded`]), and this is the only place the manager
    /// builds a record — so an adapter's string cannot reach the ring, the
    /// snapshot or the wire unbounded.
    pub(super) fn record(&mut self, run_id: &AgentRunId, at: Timestamp, event: AgentRunEvent) {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        let record = AgentRunEventRecord::new(seq, run_id.clone(), at, event);
        self.events.push_back(record.clone());
        self.undelivered.push(record);
        while self.events.len() > MAX_RUN_EVENTS {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        // The delta is bounded too. A burst that overflowed the ring must not
        // then cross the protocol whole; what the client cannot have is already
        // counted above.
        if self.undelivered.len() > MAX_RUN_EVENTS {
            self.undelivered
                .drain(..self.undelivered.len() - MAX_RUN_EVENTS);
        }
    }

    /// One coalesced batch, or **nothing at all** when nothing changed.
    ///
    /// Emitting nothing for an idle run is what keeps a flush at every step
    /// boundary from being an event storm.
    pub(super) fn take_batch(
        &mut self,
        run_id: &AgentRunId,
    ) -> Option<AgentRunEventAppendedPayload> {
        let events = std::mem::take(&mut self.undelivered);
        if events.is_empty() && self.dropped == self.delivered_dropped {
            return None;
        }
        self.delivered_dropped = self.dropped;
        Some(AgentRunEventAppendedPayload {
            run_id: run_id.clone(),
            events,
            dropped: self.dropped,
        })
    }

    /// Everything the ring holds: the answer to `runs.events`, which is what
    /// rehydrates a renderer after a reload.
    pub(super) fn snapshot(&self, run_id: &AgentRunId) -> AgentRunEventLog {
        AgentRunEventLog {
            run_id: run_id.clone(),
            events: self.events.iter().cloned().collect(),
            dropped: self.dropped,
        }
    }

    /// Drop an undelivered batch, because the caller already has those records
    /// in a snapshot it just took.
    pub(super) fn forget_pending(&mut self) {
        self.undelivered.clear();
        self.delivered_dropped = self.dropped;
    }

    /// How many records are waiting for a batch. The manager flushes on its own
    /// once this reaches [`MAX_RUN_EVENTS`], so a driver that never flushes
    /// still cannot make the pending list unbounded.
    pub(super) fn pending(&self) -> usize {
        self.undelivered.len()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.events.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run() -> AgentRunId {
        AgentRunId::from_raw("run-1")
    }

    fn ring_with(count: usize) -> EventRing {
        let mut ring = EventRing::new();
        for index in 0..count {
            ring.record(
                &run(),
                Timestamp::from_millis(index as u64),
                AgentRunEvent::PromptStarted {
                    text: format!("prompt {index}"),
                },
            );
        }
        ring
    }

    #[test]
    fn records_carry_a_strictly_increasing_seq() {
        let ring = ring_with(3);
        let seqs: Vec<u64> = ring.snapshot(&run()).events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2]);
    }

    #[test]
    fn many_records_become_one_batch() {
        let mut ring = ring_with(50);
        let batch = ring.take_batch(&run()).expect("a batch");
        assert_eq!(batch.events.len(), 50, "coalesced, not fifty events");
        assert_eq!(batch.dropped, 0);
        assert!(
            ring.take_batch(&run()).is_none(),
            "a batch is delivered once"
        );
    }

    #[test]
    fn an_idle_ring_emits_nothing() {
        let mut ring = EventRing::new();
        assert!(ring.take_batch(&run()).is_none());
    }

    /// The overflow contract: eviction is oldest-first, the loss is counted,
    /// and the count is what crosses the protocol.
    #[test]
    fn overflow_evicts_the_oldest_and_reports_the_loss() {
        let overflow = 7;
        let mut ring = ring_with(MAX_RUN_EVENTS + overflow);
        assert_eq!(ring.len(), MAX_RUN_EVENTS, "the ring stays bounded");

        let snapshot = ring.snapshot(&run());
        assert_eq!(snapshot.events.len(), MAX_RUN_EVENTS);
        assert_eq!(snapshot.dropped, overflow as u32, "the loss is reported");
        assert_eq!(
            snapshot.events.first().expect("an oldest record").seq,
            overflow as u64,
            "the oldest survivors are the ones after the evicted head"
        );

        let batch = ring.take_batch(&run()).expect("a batch");
        assert_eq!(
            batch.events.len(),
            MAX_RUN_EVENTS,
            "the delta is bounded like the ring"
        );
        assert_eq!(
            batch.dropped, overflow as u32,
            "the batch carries the cumulative total, per the wire contract"
        );
    }

    /// A burst that evicts everything an undelivered batch held still tells the
    /// client that a gap opened, rather than going out as silence.
    #[test]
    fn new_loss_alone_is_worth_a_batch() {
        let mut ring = ring_with(2);
        ring.take_batch(&run()).expect("the first batch");
        assert!(ring.take_batch(&run()).is_none());

        // Fill past the bound, then forget the records so only the loss is new.
        for index in 0..MAX_RUN_EVENTS + 3 {
            ring.record(
                &run(),
                Timestamp::from_millis(index as u64),
                AgentRunEvent::RunResumed,
            );
        }
        let dropped = ring.snapshot(&run()).dropped;
        assert!(dropped > 0);
        ring.undelivered.clear();

        let batch = ring
            .take_batch(&run())
            .expect("new loss is worth an otherwise empty batch");
        assert!(batch.events.is_empty());
        assert_eq!(batch.dropped, dropped);
        assert!(
            ring.take_batch(&run()).is_none(),
            "the same loss is not reported twice"
        );
    }

    /// Every string is bounded on the way in, so nothing unbounded can reach a
    /// snapshot even if the manager never emitted a batch.
    #[test]
    fn records_are_bounded_on_the_way_in() {
        let mut ring = EventRing::new();
        ring.record(
            &run(),
            Timestamp::from_millis(1),
            AgentRunEvent::PromptStarted {
                text: "x".repeat(super::super::wire::MAX_TEXT * 4),
            },
        );
        let snapshot = ring.snapshot(&run());
        let AgentRunEvent::PromptStarted { text } = &snapshot.events[0].event else {
            panic!("expected PROMPT_STARTED");
        };
        assert_eq!(text.chars().count(), super::super::wire::MAX_TEXT);
        assert!(text.ends_with('…'), "the clip is marked");
    }

    #[test]
    fn forgetting_a_pending_batch_leaves_the_snapshot_alone() {
        let mut ring = ring_with(4);
        ring.forget_pending();
        assert!(
            ring.take_batch(&run()).is_none(),
            "the caller already has them"
        );
        assert_eq!(
            ring.snapshot(&run()).events.len(),
            4,
            "rehydration still answers in full"
        );
    }
}
