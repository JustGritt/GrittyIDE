//! Gritty's Claude Code adapter (T310-A): the child process, the JSON-line
//! framing, and the translation of its stdout into typed observations.
//!
//! The sibling of [`gritty_agent_fake`], and deliberately the same shape: the
//! provider speaks its own vocabulary here, and `gritty-core` translates that
//! vocabulary into the protocol. What differs is that this one has a real
//! process behind it, so everything the fake gets for free — determinism, no
//! clock, no I/O — has to be *earned*, by keeping the two halves separable.
//!
//! ```
//! use gritty_agent_claude::{ClaudeObservation, ClaudeStream};
//!
//! let mut stream = ClaudeStream::new();
//! let observations = stream.ingest(
//!     br#"{"type":"system","subtype":"init","session_id":"s","tools":[]}
//! {"type":"rate_limit_event"}
//! "#,
//! );
//! // `init` is what ends `STARTING`, and an unmodelled kind is skipped.
//! assert!(stream.is_ready());
//! assert_eq!(observations[0].run_state(), Some("RUNNING"));
//! assert_eq!(
//!     observations[1],
//!     ClaudeObservation::Unknown { kind: "rate_limit_event".to_owned() }
//! );
//! ```
//!
//! # The two halves, and why the line is where it is
//!
//! | | pure | needs a process |
//! |---|---|---|
//! | [`launch`] | the argv, fixed and inspectable | — |
//! | [`request`] | the frames written on stdin | — |
//! | [`stream`] | bytes → [`ClaudeObservation`] | — |
//! | [`observation`] | the published mapping into the protocol | — |
//! | [`process`] | — | spawn, pipes, signals, the child's lifetime |
//!
//! Everything above the line runs in CI with neither `claude` on `PATH` nor an
//! API key, driven by `fixtures/two-turns-denied.jsonl` — a **real** recording
//! of `claude` 2.1.263, redacted but not synthesised. That is the fake's
//! determinism argument applied one layer out: a translation proved against a
//! transcript nobody wrote by hand cannot drift from what the CLI actually
//! emits, and a spike that only ever exercised a hand-built stream would be
//! testing its author's memory.
//!
//! # This crate does not depend on `gritty-core`, and cannot
//!
//! `gritty-core` has to depend on **this** crate for its run manager to drive
//! one, exactly as it already does for `gritty-agent-fake`. A dependency the
//! other way would be a cycle Cargo refuses. Two consequences, and both are the
//! design rather than a concession:
//!
//! - **The translation into `AgentRunEvent` lives in `agent_run`.** This crate
//!   speaks [`ClaudeObservation`] and publishes the intended mapping as
//!   strings — [`ClaudeObservation::run_events`],
//!   [`ClaudeObservation::run_activity`], [`ClaudeObservation::run_state`],
//!   [`ClaudeError::core_error_code`] — so the two lanes can be checked against
//!   each other without either importing the other. Those strings are a
//!   *specification*; `AgentRunEvent::as_str` and friends remain the authority,
//!   and `agent_run/claude_driver.rs` (T310-B) is the implementation.
//! - **This crate cannot sanitise its own output**, which is the correct
//!   arrangement: bounding and whitespace-collapsing untrusted adapter text is
//!   `AgentRunEvent::bounded`'s job, once, in the core (rule 13). Everything a
//!   child says passes through here exactly as it was written.
//!
//! What this crate *does* bound is resources, which is a different question and
//! has to be answered on the untrusted side: [`MAX_LINE_BYTES`],
//! [`MAX_OUTSTANDING_TOOLS`], [`MAX_KIND_BYTES`] and
//! [`MAX_STDERR_TAIL_BYTES`].
//!
//! # There is no `AgentAdapter` trait
//!
//! Two implementations is still not a reason (task §"Explicit non-goals"). The
//! fake and this one are driven by the same `FnOnce(AgentRunContext)` seam on
//! the core's side, and neither knows about the other. The one seam this crate
//! *does* define is [`ClaudeVerdict`] — a value, not a trait — because task §7
//! asks for the permission policy to be swappable by `T310-ANSWER` without a
//! rework, and an enum is that seam with none of an abstraction's cost.
//!
//! # What this cut does not do
//!
//! T310-A is the adapter and the translation. It adds **no** command, **no**
//! event and **no** wire type, so it claims no `PROTOCOL_VERSION`. Reaching a
//! run from `runs.start` and `runs.prompt`, the per-run inbox, and keeping
//! Gritty's own child out of the observed roster are T310-B; the console's
//! prompt box is T310-U.

pub mod error;
pub mod launch;
mod message;
pub mod observation;
pub mod process;
pub mod request;
pub mod stream;

pub use error::ClaudeError;
pub use launch::{ClaudeLaunch, DEFAULT_PROGRAM};
pub use observation::{
    ClaudeExit, ClaudeExitStatus, ClaudeObservation, ClaudePermissionAsk, ClaudeReady,
    ClaudeToolDone, ClaudeToolIntent, ClaudeToolUse, ClaudeTurnEnd,
};
pub use process::{
    ClaudeProcess, MAX_STDERR_TAIL_BYTES, READY_TIMEOUT, REAP_GRACE, TERMINATE_GRACE,
};
pub use request::{ClaudeVerdict, DENY_BY_DEFAULT_REASON, permission_response, user_turn};
pub use stream::{
    ClaudeStream, ClaudeStreamStats, MAX_KIND_BYTES, MAX_LINE_BYTES, MAX_OUTSTANDING_TOOLS,
};
