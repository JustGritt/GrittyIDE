//! Gritty's deterministic fake agent (T300-FAKE, §§8-11 of the Gate D spec).
//!
//! A whole agent-run lifecycle — start, work, stop on a permission, resume,
//! stop on a decision, resume, finish, fail, get cancelled — with **no
//! process, no network, no filesystem, no provider and no clock**. Everything
//! it emits is scripted fiction, which is exactly what makes it safe to run
//! anywhere, CI included, without a fixture ever being touched.
//!
//! ```
//! use gritty_agent_fake::{FakeAgent, FakePhase, FakeScript, FakeScriptPreset};
//!
//! let mut agent = FakeAgent::new(FakeScript::preset(FakeScriptPreset::FullLifecycle));
//! agent.advance_all();                       // runs until it stops on something
//! assert_eq!(agent.phase(), FakePhase::AwaitingPermission);
//! agent.answer_permission(true).unwrap();    // out of band, never a `runs.*` command
//! agent.advance_all();
//! assert_eq!(agent.phase(), FakePhase::AwaitingDecision);
//! agent.answer_decision().unwrap();
//! agent.advance_all();
//! assert_eq!(agent.phase(), FakePhase::Done);
//! ```
//!
//! # Determinism is the product
//!
//! The run advances because the caller advanced it and for no other reason.
//! There is no `sleep`, no `Instant`, no `SystemTime` and no timer anywhere in
//! this crate; [`FakeClock`] is a counter that steps when an action is
//! recorded. Two agents built from the same script produce byte-identical
//! [`FakeObservation`]s, sequence numbers and timestamps included. A fake that
//! is *almost* deterministic would be worse than none, because every test
//! downstream of Gate D inherits whatever flakiness it ships.
//!
//! # This crate does not depend on `gritty-core`, and cannot
//!
//! `gritty-core` has to depend on **this** crate for T300-CORE's run manager to
//! drive it, exactly as it already depends on `gritty-agents` and
//! `gritty-terminal`. A dependency the other way would be a cycle Cargo
//! refuses. Two consequences, and both are the design rather than a
//! concession:
//!
//! - **The translation into `AgentRunEvent` lives in `agent_run`**, which is
//!   what task §9 asks for. This crate speaks its own vocabulary
//!   ([`FakeStep`], [`FakeAction`]) and publishes the intended mapping as
//!   strings — [`FakeAction::run_events`], [`FakeAction::run_activity`],
//!   [`FakePhase::run_state`], [`FakeError::core_error_code`] — so the two
//!   lanes can be checked against each other without either importing the
//!   other. Those strings are a *specification*; `AgentRunEvent::as_str` and
//!   friends remain the authority.
//! - **The fake cannot sanitise its own output**, which is the correct
//!   arrangement: bounding and whitespace-collapsing untrusted adapter text is
//!   `AgentRunEvent::bounded`'s job, once, in the core (rule 13).
//!   [`FakeScriptPreset::Hostile`] exists to prove that seam holds, and this
//!   crate's tests assert that it passes its nastiest strings through
//!   untouched.
//!
//! # The interface a supervisor drives
//!
//! There is **no `AgentAdapter` trait** — one implementation exists, and a
//! trait over one implementation is the abstraction hard rule 9 forbids
//! building ahead of a requirement (T305 and T310 earn it). The interface is
//! the two concrete types:
//!
//! | | [`FakeAgent`] (the driver) | [`FakeAgentHandle`] (the supervisor's view) |
//! |---|---|---|
//! | advance the script | [`FakeAgent::advance`], [`FakeAgent::advance_all`] | — |
//! | read the phase | [`FakeAgent::phase`] | [`FakeAgentHandle::phase`] |
//! | read the record | [`FakeAgent::observations`] | [`FakeAgentHandle::observations`], [`FakeAgentHandle::observations_since`] |
//! | answer out of band (§10) | [`FakeAgent::answer_permission`], [`FakeAgent::answer_decision`] | same |
//! | cancel | [`FakeAgent::cancel`] | [`FakeAgentHandle::cancel`] |
//!
//! Both are `Send + Sync`. Take the handle **before** giving the driver away:
//! it is what makes the orphan guarantee observable.
//!
//! # The mapping, in one table
//!
//! What the core's translation should produce for each action. `RUN_STARTED`
//! appears nowhere because it is the manager's, emitted when the run is
//! created rather than reported by an adapter; a cancellation produces no
//! event at all, because the protocol says that with the run's state.
//!
//! | [`FakeAction`] | `AgentRunEvent` | `AgentRunActivityKind` | `AgentRunState` after |
//! |---|---|---|---|
//! | `prompt` | `PROMPT_STARTED` | `THINKING` | `RUNNING` |
//! | `think` | — | `THINKING` | unchanged |
//! | `read` | `FILE_READ` | `READING` | unchanged |
//! | `search` | — | `SEARCHING` | unchanged |
//! | `proposeWrite` | `FILE_WRITE_REQUESTED` | `EDITING` | unchanged |
//! | `write` | `FILE_WRITTEN` | `EDITING` | unchanged |
//! | `command` | `COMMAND_STARTED` | `EXECUTING` | unchanged |
//! | `test` | `COMMAND_STARTED` | `TESTING` | unchanged |
//! | `commandDone` | `COMMAND_FINISHED` | `THINKING` | unchanged |
//! | `reply` | `PROMPT_FINISHED` | `THINKING` | unchanged |
//! | `askPermission` | `PERMISSION_REQUESTED`, `RUN_BLOCKED` | `REQUESTING_PERMISSION` | `BLOCKED` |
//! | `askDecision` | `DECISION_REQUESTED` | `REQUESTING_DECISION` | `NEEDS_DECISION` |
//! | `idle` | — | `WAITING` | `WAITING` |
//! | `present` | — | cleared | `REVIEW` |
//! | `finish` | `RUN_FINISHED` | cleared | `FINISHED` |
//! | `fail` | `RUN_FAILED` | cleared | `FAILED` |
//! | `permissionAnswered` | `RUN_RESUMED` | `THINKING` | `RUNNING` |
//! | `decisionAnswered` | `RUN_RESUMED` | `THINKING` | `RUNNING` |
//! | `cancelled` | — | cleared | `CANCELLED` |
//! | `orphaned` | `RUN_FAILED` (`Internal`) | cleared | `FAILED` |
//!
//! A `FakeAdvance::Rejected` produces nothing at all: a duplicated or
//! out-of-order step changes no state and emits no event, matching
//! `AgentRunState::check_transition`, which refuses rather than absorbs.
//!
//! # Orphans (§11)
//!
//! A run never sits in a working phase with nobody driving it. Two ways that is
//! made true, and both end [`FakePhase::Broke`] → `FAILED` with an `Internal`
//! error:
//!
//! - the script runs out with no `finish` or `fail` in it
//!   ([`ORPHANED_UNTERMINATED`]), resolved on the next advance;
//! - the driver is dropped or panics mid-run ([`ORPHANED_DROPPED`]), resolved
//!   in `Drop` and observable through a [`FakeAgentHandle`] that outlives it.
//!
//! Neither is `INTERRUPTED`: that ending belongs to a supervisor that can no
//! longer watch a run which is otherwise fine (the workspace closed, the app is
//! shutting down), and nothing a script does means that.

mod agent;
mod error;
mod phase;
mod script;

pub use agent::{
    DEFAULT_START_MILLIS, DEFAULT_TICK_MILLIS, FakeAdvance, FakeAgent, FakeAgentHandle, FakeClock,
    FakeObservation, ORPHANED_DROPPED, ORPHANED_UNTERMINATED,
};
pub use error::FakeError;
pub use phase::FakePhase;
pub use script::{CHATTY_STEPS, FakeAction, FakeScript, FakeScriptPreset, FakeStep};
