//! Agent runs: the ones Gritty **owns** (T300, Gate D).
//!
//! # The line this module exists to draw
//!
//! [`crate::agent_session`] describes an agent that is running on this machine
//! and that Gritty did **not** start. Nothing there is a handle: there is no
//! command that starts, stops, answers or approves one of those, "because
//! Gritty does not own these sessions: another terminal does".
//!
//! This module is the other half. A run here was started by Gritty, against a
//! workstream, with a correlation id; it can be cancelled; it ends in a state
//! Gritty recorded; and Gritty is answerable for what it did. The two are
//! different types with different lifetimes and they never meet: no type in
//! this module imports one from `agent_session`, and the reverse is likewise
//! forbidden. If correlating a run to the observed roster starts to look
//! necessary, the boundary was drawn wrong and the right move is to stop, not
//! to reach across (`docs/plans/execution-board.md`).
//!
//! # What is here, and what is not
//!
//! This is the **contract** (T300-C) and nothing else: the domain model
//! ([`wire::AgentRun`]), the state machine ([`state::AgentRunState`]), the
//! normalized event vocabulary ([`wire::AgentRunEvent`]) and the two event
//! payloads the renderer receives. It is deliberately types, invariants and
//! their tests — no manager, no supervision, no adapter, no commands.
//!
//! Three tickets build against it, and each is named where its half would
//! otherwise be missed:
//!
//! - **T300-CORE** owns the run manager: the registry, the per-run
//!   `CancellationToken`, `MAX_CONCURRENT_RUNS`, the bounded ring that
//!   [`wire::MAX_RUN_EVENTS`] sizes, and the reactions that stop a run being
//!   orphaned when its workstream is removed, its workspace closes or the app
//!   shuts down. It is [`manager`] and [`ring`], and it is here: the contract's
//!   two known gaps — an unenforced `MAX_RUN_EVENTS` and a run nothing
//!   supervised — are closed.
//! - **T300-FAKE** owns `gritty-agent-fake`: a deterministic, caller-driven
//!   script with its **own** step vocabulary. That vocabulary is deliberately
//!   not [`wire::AgentRunEvent`] — the translation between them is the seam
//!   T310's real adapter needs, and a fake that skipped it would test nothing
//!   about the boundary that matters (hard rule 4).
//! - **T300-CMD** owns the `runs.*` command adapters and the panel. Its four
//!   request shapes are [`request`], kept out of [`wire`] because that file is
//!   the frozen contract and these are the command surface that reaches it —
//!   the same split `operation::registry` and `operation::demo` already make.
//!
//! There is **no `AgentAdapter` trait**. One implementation will exist; a trait
//! over one implementation is the abstraction hard rule 9 forbids building
//! ahead of a requirement. T305 and T310 earn it.
//!
//! # Untrusted by construction
//!
//! Everything an adapter says is untrusted input (rule 13), the fake included.
//! [`wire::AgentRunEvent::bounded`] and [`wire::AgentRunActivity::bounded`] are
//! where that is made true, once, in the core — never in the adapter, which is
//! the untrusted side, and never in the renderer, which would be trusting it.
//! Tracing carries a run's id, state, provider and duration; never event text.

pub mod manager;
mod ring;
pub mod request;
pub mod state;
pub mod wire;

#[cfg(test)]
mod tests;

pub use manager::{
    AgentRunContext, AgentRunManager, AgentRunSpec, FINISHED_RETENTION, MAX_CONCURRENT_RUNS,
};
pub use request::{CancelRunRequest, ListRunsRequest, RunEventsRequest, StartFakeRunRequest};
pub use state::AgentRunState;
pub use wire::{
    AgentRun, AgentRunActivity, AgentRunActivityKind, AgentRunChangedPayload, AgentRunEvent,
    AgentRunEventAppendedPayload, AgentRunEventLog, AgentRunEventRecord, AgentRunPath,
    AgentRunProvider,
};
