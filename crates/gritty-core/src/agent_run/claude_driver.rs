//! The `gritty-agent-claude` driver: a run Gritty owns, driving a real Claude
//! Code process (T310-B).
//!
//! # Why this file is here and not in the adapter
//!
//! Exactly [`super::fake_driver`]'s reason, one provider along.
//! `gritty-agent-claude` publishes the translation from its own vocabulary into
//! this protocol as *strings* — [`ClaudeObservation::run_events`],
//! [`ClaudeObservation::run_activity`], [`ClaudeObservation::run_state`],
//! [`ClaudeError::core_error_code`] — and cannot perform it, because
//! `gritty-core` depends on that crate and the reverse edge is a Cargo cycle.
//! The mapping below is written **typed**, by matching on
//! [`ClaudeObservation`], and [`tests`] asserts that every arm agrees with the
//! strings the adapter publishes. A variant added on either side fails that
//! test rather than silently drifting.
//!
//! # What the driver owns, and what it must not touch
//!
//! It reports; it does not decide endings it has no standing to decide.
//! `RUN_STARTED` is the manager's, and so are `CANCELLED` and `INTERRUPTED` —
//! [`AgentRunContext`] deliberately offers no way to reach them, because "a
//! driver claiming to have been interrupted is claiming to know something only
//! the supervisor knows" (see the manager's own module docs).
//!
//! **That collides with the task's failure table**, which asks for
//! `INTERRUPTED` when the child is killed or the pipe is lost while it lives.
//! The frozen Gate D contract wins here and the run ends `FAILED` with an error
//! naming the signal or the lost pipe, so the *information* survives even
//! though the state's granularity does not. Widening a frozen contract needs
//! deliberate review and an ADR, which is not this ticket's to take; it is
//! recorded as a follow-up instead.
//!
//! Nothing here bounds a string. Everything the child says is untrusted adapter
//! text by construction (rule 13), and [`AgentRunEvent::bounded`] is applied by
//! the manager on the way in — once, in one place, on the trusted side.
//!
//! # One turn at a time, and why the inbox arm is gated
//!
//! A prompt written while a reply is still arriving would interleave two turns
//! on one stdin. So the loop tracks whether a turn is in flight and **does not
//! poll the inbox while one is**, which also keeps
//! [`super::manager::MAX_QUEUED_PROMPTS`] the only bound there is: a driver that
//! took a message it could not act on and held it in a local would have made a
//! second, unbounded queue behind the first.

use std::time::Duration;

use gritty_agent_claude::{
    ClaudeError, ClaudeExit, ClaudeLaunch, ClaudeObservation, ClaudePermissionAsk, ClaudeProcess,
    ClaudeToolDone, ClaudeToolIntent, ClaudeToolUse, ClaudeTurnEnd, ClaudeVerdict, READY_TIMEOUT,
};
use gritty_workspace::WorkspacePath;

use super::manager::{AgentRunContext, RunInboxReceiver, RunInput};
use super::state::AgentRunState;
use super::wire::{
    AgentRunActivity, AgentRunActivityKind as ActivityKind, AgentRunEvent, AgentRunPath,
};
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::time::Timestamp;

/// Who answers a `can_use_tool`.
///
/// **The seam task §7 asks for**, and a value rather than a trait — the same
/// choice the adapter made for [`ClaudeVerdict`], and for the same reason: a
/// trait over one implementation is the abstraction hard rule 9 forbids
/// building ahead of a requirement.
///
/// `T310-ANSWER` adds a variant carrying the user's own verdict, which is a
/// compile error at every match rather than a rework — which is the point of
/// naming the seam now instead of inlining the denial.
#[derive(Debug, Clone, Default)]
pub enum PermissionPolicy {
    /// Refuse every gated call, with a reason the agent is told.
    ///
    /// The only default that is safe before anyone can answer. A denial is
    /// **not** a failure: the run continues and the agent carries on, exactly
    /// as it does in a terminal.
    #[default]
    DenyAll,
}

impl PermissionPolicy {
    /// The verdict for one ask.
    ///
    /// `async` although the only policy today answers instantly, so that
    /// `T310-ANSWER` — which has to wait for a person — changes this body and
    /// nothing at the call site.
    ///
    /// **Never `allow` by default, and never on a timeout.** An unanswered ask
    /// is a denial; the opposite is a tool running because nobody was looking.
    /// The adapter verified there is no CLI-side timeout that could resolve one
    /// the other way.
    pub async fn decide(&self, _ask: &ClaudePermissionAsk) -> ClaudeVerdict {
        match self {
            Self::DenyAll => ClaudeVerdict::deny_by_default(),
        }
    }
}

/// How the caller wants the run driven.
#[derive(Debug, Clone)]
pub struct ClaudeDriverOptions {
    /// How long `system`/`init` may take before the run fails.
    ///
    /// This is what makes `STARTING` mean something (task §3): the run leaves
    /// it when the session announces itself, **not** when the process spawns,
    /// which is the case phase 0 found and the state was introduced for.
    pub ready_timeout: Duration,
    pub policy: PermissionPolicy,
}

impl Default for ClaudeDriverOptions {
    fn default() -> Self {
        Self {
            ready_timeout: READY_TIMEOUT,
            policy: PermissionPolicy::DenyAll,
        }
    }
}

/// Build the driver closure [`super::AgentRunManager::start_with_inbox`] takes.
///
/// The process is spawned **inside** the closure rather than captured, so a run
/// whose driver is never scheduled cannot leave a child alive with nobody to
/// supervise it — the same reason `fake_driver::drive` builds its agent inside.
pub fn drive(
    launch: ClaudeLaunch,
    inbox: RunInboxReceiver,
    options: ClaudeDriverOptions,
) -> impl FnOnce(
    AgentRunContext,
) -> std::pin::Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send>>
+ Send
+ 'static {
    move |context| Box::pin(run(context, launch, inbox, options))
}

/// What the loop did this time round.
///
/// The `select!` produces one of these and its futures are dropped before the
/// handler runs, so the handler is free to use the process again — writing an
/// answer or a prompt — without fighting the borrow the arm took.
enum Step {
    Cancelled,
    Observed(ClaudeObservation),
    /// stdout is over. The adapter guarantees the last observation before this
    /// is always an `Exited`, so the ending has already been reported.
    Drained,
    Prompt(String),
    /// The manager's end of the inbox is gone, so no prompt can ever arrive
    /// again. The arm is disabled from here rather than busy-looping on a
    /// receiver that is permanently ready with `None`.
    InboxClosed,
}

async fn run(
    context: AgentRunContext,
    launch: ClaudeLaunch,
    mut inbox: RunInboxReceiver,
    options: ClaudeDriverOptions,
) -> Result<(), CoreError> {
    let mut process = match ClaudeProcess::spawn(&launch) {
        Ok(process) => process,
        // Nothing was ever being supervised, so this is `FAILED` and never
        // `INTERRUPTED` — the task's failure table is explicit about it.
        Err(error) => {
            context.fail(claude_error(error))?;
            return Ok(());
        }
    };
    if let Some(pid) = process.pid() {
        // So the observed roster can exclude our own child (§8).
        context.set_child_pid(pid);
    }
    tracing::debug!(run = %context.run_id(), pid = ?process.pid(), "driving a claude code run");

    // The bounded wait for `system`/`init`. Everything seen on the way is
    // reported, so the run records the same events it would have from the loop.
    match process.wait_ready(options.ready_timeout).await {
        Ok(seen) => {
            for observation in seen {
                if apply(&context, &observation)?.is_some() {
                    return Ok(());
                }
            }
        }
        Err(error) => {
            // It never announced a session. Kill it before failing: a child
            // left behind is the worst outcome available here.
            process.terminate().await;
            context.fail(claude_error(error))?;
            return Ok(());
        }
    }

    let mut in_turn = false;
    let mut inbox_open = true;
    loop {
        let step = tokio::select! {
            biased;
            // Cancellation first, so a stop lands even while the child is
            // chattering. The manager already knows what ending it wants and
            // records it; reporting one here would be the supervised thing
            // answering a question that is the supervisor's.
            () = context.cancelled() => Step::Cancelled,
            observed = process.next() => match observed {
                Some(observation) => Step::Observed(observation),
                None => Step::Drained,
            },
            // Only while nothing is in flight. When the guard is false the
            // branch is not polled and the message stays in the channel.
            input = inbox.recv(), if inbox_open && !in_turn => match input {
                Some(RunInput::Prompt { text }) => Step::Prompt(text),
                None => Step::InboxClosed,
            },
        };

        match step {
            Step::Cancelled => {
                process.terminate().await;
                return Ok(());
            }
            Step::Drained => return Ok(()),
            Step::InboxClosed => inbox_open = false,
            Step::Observed(observation) => {
                if let ClaudeObservation::TurnFinished(_) = &observation {
                    in_turn = false;
                }
                if apply(&context, &observation)?.is_some() {
                    return Ok(());
                }
                // The ask holds the child until an answer goes back, and the
                // run is already `BLOCKED` and visible by the time we decide.
                if let ClaudeObservation::PermissionRequested(ask) = &observation {
                    let verdict = options.policy.decide(ask).await;
                    if let Err(error) = process.answer(&ask.request_id, &verdict).await {
                        context.fail(claude_error(error))?;
                        return Ok(());
                    }
                }
            }
            Step::Prompt(text) => {
                if let Err(error) = process.prompt(&text).await {
                    context.fail(claude_error(error))?;
                    return Ok(());
                }
                in_turn = true;
            }
        }
    }
}

/// Report one observation. `Some(state)` once the run has been ended.
fn apply(
    context: &AgentRunContext,
    observation: &ClaudeObservation,
) -> Result<Option<AgentRunState>, CoreError> {
    let next = state_for(observation);

    // The endings the driver owns. `finish` records `RUN_FINISHED` and `fail`
    // records `RUN_FAILED` themselves, so the general translation below must
    // not also emit one — which is why these return.
    if let Some(state) = next
        && state.is_terminal()
    {
        match observation {
            ClaudeObservation::Exited(exit) => return end_from_exit(context, exit).map(Some),
            ClaudeObservation::TurnFinished(end) => {
                context.fail(turn_failure(end))?;
                return Ok(Some(AgentRunState::Failed));
            }
            // `state_for` produces a terminal state for exactly the two
            // observations above; a third would be a translation bug, not a
            // child's doing.
            other => {
                context.fail(CoreError::internal(format!(
                    "the claude driver produced the terminal state {} from {}, which it has no rule for",
                    state.as_str(),
                    other.kind()
                )))?;
                return Ok(Some(AgentRunState::Failed));
            }
        }
    }

    for event in events_for(observation) {
        context.record(event);
    }

    let activity = activity_for(observation);
    match next {
        // The state did not change, so this is an activity report. Calling
        // `advance` with the state it is already in is what the manager reads
        // as a duplicated step, and it refuses rather than absorbing it.
        Some(state) if context.snapshot().map(|run| run.state) != Some(state) => {
            context.advance(state, activity)?;
        }
        _ => {
            context.set_activity(activity)?;
        }
    }
    Ok(None)
}

/// End the run from the child's exit.
///
/// The one place the frozen contract and the task's failure table disagree: an
/// `INTERRUPTED` ending is not reachable from a driver, so it is reported as a
/// failure whose message says plainly that supervision was lost rather than
/// that the agent was wrong.
fn end_from_exit(
    context: &AgentRunContext,
    exit: &ClaudeExit,
) -> Result<AgentRunState, CoreError> {
    match exit.run_state() {
        "FINISHED" => {
            // No summary: the agent's own last word already arrived as
            // `PROMPT_FINISHED`, and inventing a second one here would say the
            // same thing twice.
            context.finish(None)?;
            Ok(AgentRunState::Finished)
        }
        _ => {
            context.fail(exit_failure(exit))?;
            Ok(AgentRunState::Failed)
        }
    }
}

/// The failure a child's ending means, with its stderr tail attached.
fn exit_failure(exit: &ClaudeExit) -> CoreError {
    use gritty_agent_claude::ClaudeExitStatus;
    let message = match exit.status {
        ClaudeExitStatus::Signal(signal) => format!(
            "the agent was killed by signal {signal}; Gritty stopped being able to supervise it"
        ),
        ClaudeExitStatus::Lost => {
            "the pipe to the agent went away while it was still running; Gritty stopped being able to supervise it".to_owned()
        }
        ClaudeExitStatus::Code(code) => format!("the agent exited with status {code}"),
        ClaudeExitStatus::Ok => {
            "the agent's output ended in the middle of a turn".to_owned()
        }
    };
    let message = if exit.stderr_tail.is_empty() {
        message
    } else {
        format!("{message}: {}", exit.stderr_tail)
    };
    CoreError::new(CoreErrorCode::Io, message)
}

/// The failure a turn that ended badly means.
fn turn_failure(end: &ClaudeTurnEnd) -> CoreError {
    CoreError::new(
        CoreErrorCode::Io,
        format!("the agent's turn ended {}", end.subtype),
    )
}

/// A refusal from the adapter, as the `CoreError` the adapter asked for.
///
/// The code comes from [`ClaudeError::core_error_code`] rather than from a
/// match here, so a variant the adapter adds cannot quietly become an
/// `Internal`.
fn claude_error(error: ClaudeError) -> CoreError {
    let message = error.to_string();
    match error.core_error_code() {
        "Io" => CoreError::new(CoreErrorCode::Io, message),
        "Timeout" => CoreError::new(CoreErrorCode::Timeout, message),
        "Conflict" => CoreError::conflict(message),
        "InvalidInput" => CoreError::invalid_input("agent", message),
        // The adapter named a code this arm does not know: a new variant, and
        // guessing at it would be worse than saying so.
        other => CoreError::internal(format!(
            "the claude adapter asked for an unknown error code {other:?}: {message}"
        )),
    }
}

/// The events an observation becomes, in order.
///
/// The typed half of [`ClaudeObservation::run_events`]. `RUN_STARTED` is absent
/// because the manager emits it.
fn events_for(observation: &ClaudeObservation) -> Vec<AgentRunEvent> {
    match observation {
        // A state change and an activity; no event. The run leaving `STARTING`
        // is visible in the run itself.
        ClaudeObservation::Ready(_) => Vec::new(),
        ClaudeObservation::PromptEchoed { text } => vec![AgentRunEvent::PromptStarted {
            text: text.clone(),
        }],
        ClaudeObservation::Thought
        | ClaudeObservation::Said { .. }
        | ClaudeObservation::Unknown { .. } => Vec::new(),
        ClaudeObservation::ToolStarted(call) => tool_started_events(call),
        ClaudeObservation::ToolFinished(done) => tool_finished_events(done),
        ClaudeObservation::PermissionRequested(ask) => vec![
            AgentRunEvent::PermissionRequested {
                tool: ask.tool.clone(),
                detail: ask.detail.clone(),
            },
            AgentRunEvent::RunBlocked {
                reason: blocked_reason(ask),
            },
        ],
        ClaudeObservation::PermissionAnswered { .. } => vec![AgentRunEvent::RunResumed],
        ClaudeObservation::TurnFinished(end) => vec![AgentRunEvent::PromptFinished {
            text: end.text.clone(),
        }],
        // Reported through `finish`/`fail`, which record their own event; this
        // arm exists so the agreement test can check it against the adapter.
        ClaudeObservation::Exited(exit) => match exit.run_state() {
            "FINISHED" => vec![AgentRunEvent::RunFinished { summary: None }],
            "FAILED" => vec![AgentRunEvent::RunFailed {
                error: exit_failure(exit),
            }],
            _ => Vec::new(),
        },
    }
}

fn tool_started_events(call: &ClaudeToolUse) -> Vec<AgentRunEvent> {
    match &call.intent {
        ClaudeToolIntent::ReadFile { path } => vec![AgentRunEvent::FileRead {
            path: workspace_path(path),
        }],
        ClaudeToolIntent::WriteFile { path } => vec![AgentRunEvent::FileWriteRequested {
            path: workspace_path(path),
        }],
        ClaudeToolIntent::RunCommand { command } => vec![AgentRunEvent::CommandStarted {
            command: command.clone(),
            cwd: None,
        }],
        ClaudeToolIntent::Search { .. } | ClaudeToolIntent::Other => Vec::new(),
    }
}

fn tool_finished_events(done: &ClaudeToolDone) -> Vec<AgentRunEvent> {
    match done.call.as_ref().map(|call| &call.intent) {
        // A write is reported as *done* only once its result came back, and
        // only when it succeeded: the request and the fact are separate events
        // because T330 intercepts between them, and a denied write must never
        // produce `FILE_WRITTEN`.
        Some(ClaudeToolIntent::WriteFile { path }) if done.ok => {
            vec![AgentRunEvent::FileWritten {
                path: workspace_path(path),
            }]
        }
        Some(ClaudeToolIntent::RunCommand { command }) => vec![AgentRunEvent::CommandFinished {
            command: command.clone(),
            ok: done.ok,
            // The CLI reports a tool result, not a process status; inventing a
            // number would be claiming to know something nobody said.
            exit_code: None,
        }],
        _ => Vec::new(),
    }
}

/// Why a run stopped, for `RUN_BLOCKED`.
fn blocked_reason(ask: &ClaudePermissionAsk) -> String {
    match &ask.detail {
        Some(detail) => format!("{} needs permission: {detail}", ask.tool),
        None => format!("{} needs permission", ask.tool),
    }
}

/// The activity an observation puts the run in.
///
/// The typed half of [`ClaudeObservation::run_activity`], carrying the path or
/// command from the observation's own payload.
fn activity_for(observation: &ClaudeObservation) -> Option<AgentRunActivity> {
    let at = Timestamp::now();
    let activity = match observation {
        ClaudeObservation::Ready(_)
        | ClaudeObservation::PromptEchoed { .. }
        | ClaudeObservation::Thought
        | ClaudeObservation::Said { .. } => AgentRunActivity::new(ActivityKind::Thinking, at),
        ClaudeObservation::ToolStarted(call) => match &call.intent {
            ClaudeToolIntent::ReadFile { path } => {
                AgentRunActivity::new(ActivityKind::Reading, at).with_path(workspace_path(path))
            }
            ClaudeToolIntent::WriteFile { path } => {
                AgentRunActivity::new(ActivityKind::Editing, at).with_path(workspace_path(path))
            }
            ClaudeToolIntent::RunCommand { command } => {
                AgentRunActivity::new(ActivityKind::Executing, at).with_command(command)
            }
            ClaudeToolIntent::Search { .. } => AgentRunActivity::new(ActivityKind::Searching, at),
            ClaudeToolIntent::Other => AgentRunActivity::new(ActivityKind::Thinking, at),
        },
        ClaudeObservation::ToolFinished(_) | ClaudeObservation::PermissionAnswered { .. } => {
            AgentRunActivity::new(ActivityKind::Thinking, at)
        }
        ClaudeObservation::PermissionRequested(_) => {
            AgentRunActivity::new(ActivityKind::RequestingPermission, at)
        }
        // The turn is over and nothing is outstanding: the run has nothing to
        // do until someone speaks, which is what `WAITING` means.
        ClaudeObservation::TurnFinished(_) => AgentRunActivity::new(ActivityKind::Waiting, at),
        // An ending has no activity, and a line nobody modelled says nothing
        // about what the run is doing.
        ClaudeObservation::Exited(_) | ClaudeObservation::Unknown { .. } => return None,
    };
    Some(activity)
}

/// The `AgentRunState` an observation means, or `None` for "unchanged".
///
/// The typed half of [`ClaudeObservation::run_state`]. `CANCELLED` never
/// appears: cancellation is the manager's ending.
fn state_for(observation: &ClaudeObservation) -> Option<AgentRunState> {
    match observation {
        ClaudeObservation::Ready(_)
        | ClaudeObservation::PromptEchoed { .. }
        | ClaudeObservation::PermissionAnswered { .. } => Some(AgentRunState::Running),
        ClaudeObservation::PermissionRequested(_) => Some(AgentRunState::Blocked),
        ClaudeObservation::TurnFinished(end) if end.ok => Some(AgentRunState::Waiting),
        // A turn that failed is a run that failed: the process may well still
        // be alive, but nothing is going to move it on.
        ClaudeObservation::TurnFinished(_) => Some(AgentRunState::Failed),
        ClaudeObservation::Exited(exit) => Some(state_named(exit.run_state())),
        ClaudeObservation::Thought
        | ClaudeObservation::Said { .. }
        | ClaudeObservation::ToolStarted(_)
        | ClaudeObservation::ToolFinished(_)
        | ClaudeObservation::Unknown { .. } => None,
    }
}

/// The state the adapter named, as this protocol's own value.
///
/// `INTERRUPTED` collapses to `FAILED` here, which is the contract collision
/// the module docs describe: the adapter is right that supervision was lost and
/// the driver has no standing to say so, so the state is `FAILED` and
/// [`exit_failure`] carries the truth in its message.
fn state_named(name: &str) -> AgentRunState {
    match name {
        "FINISHED" => AgentRunState::Finished,
        _ => AgentRunState::Failed,
    }
}

/// An adapter's raw path string, as the protocol's path type (ADR-010).
///
/// The same rule `fake_driver::workspace_path` follows, and for the same
/// reason: `path` is present only when the target is inside the workspace, and
/// its presence is precisely what the renderer may open. A leading `~` is
/// always outside — `WorkspacePath::parse` is right to reject it and a driver
/// would be wrong to expand it.
fn workspace_path(raw: &str) -> AgentRunPath {
    if raw.starts_with('~') {
        return AgentRunPath::outside(raw);
    }
    match WorkspacePath::parse(raw) {
        Ok(path) => AgentRunPath::inside(path),
        Err(_) => AgentRunPath::outside(raw),
    }
}

#[cfg(test)]
mod tests;
