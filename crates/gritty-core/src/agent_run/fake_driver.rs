//! The `gritty-agent-fake` driver: the half of Gate D neither sibling could
//! build (T300-WIRE).
//!
//! # Why this file has to exist here and nowhere else
//!
//! `gritty-agent-fake` publishes the translation from its own vocabulary into
//! this protocol as *strings* — [`FakeAction::run_events`],
//! [`FakeAction::run_activity`], [`FakePhase::run_state`],
//! [`FakeError::core_error_code`] — and says plainly that it cannot perform it:
//! `gritty-core` depends on that crate, so the reverse edge is a Cargo cycle
//! and the fake can never name [`AgentRunEvent`]. Meanwhile
//! [`AgentRunManager::start`] takes a *generic* driver closure and so does not
//! depend on the fake either. Both halves were correct and neither could reach
//! the other; this module is the reach.
//!
//! The mapping below is written **typed**, by matching on [`FakeAction`], and
//! then [`tests`] asserts that every arm agrees with the strings the fake
//! publishes. That is the point of the arrangement: the specification and the
//! implementation live in different crates and a test — not a reviewer's
//! memory — is what keeps them the same. A variant added on either side fails
//! that test rather than silently drifting.
//!
//! # What the driver is responsible for, and what it must not touch
//!
//! It reports; it does not decide endings. `RUN_STARTED` is the manager's, and
//! so are `CANCELLED` and `INTERRUPTED` — [`AgentRunContext`] deliberately
//! offers no way to reach them. The driver's whole job is: advance the script,
//! translate what came back, and end the run exactly once through
//! [`AgentRunContext::finish`] or [`AgentRunContext::fail`]. Returning without
//! ending it is an orphan, and the manager fails the run rather than leaving
//! it — so the loop below has no early `return Ok(())` that skips an ending
//! except the two where the manager is already deciding one (a cancellation,
//! and a run parked on an answer that only a cancellation will clear).
//!
//! Nothing here bounds a string. Everything the fake emits is untrusted
//! adapter text by construction (rule 13, and `FakeScriptPreset::Hostile`
//! exists to prove it), and [`AgentRunEvent::bounded`] is applied by the
//! manager on the way in — once, in one place, on the trusted side.
//!
//! # Pacing, and why it is a parameter
//!
//! The fake has no clock: `advance_all` would take a run from `STARTING` to
//! `FINISHED` in microseconds, which is right for a test and useless for a
//! human watching the panel. So the caller passes the pace.
//! [`FakeDriverOptions::step_delay`] of `None` advances as fast as it can and
//! is what the core's own tests use; the command adapter passes
//! [`DEMO_STEP_DELAY`] so the run is watchable. Either way the loop yields
//! between steps, because a driver with no await point cannot be aborted and
//! could only be abandoned.
//!
//! # A run that stops on an answer parks (T300-WIRE's one deliberate gap)
//!
//! `answer_permission` and `answer_decision` are out of band by design (§10 of
//! the Gate D spec): they are not `runs.*` commands, and `runs.startFake` was
//! deliberately shipped as four commands rather than five. So a script that
//! stops on a permission has nothing in the current protocol that can clear it.
//! Rather than invent a fifth command here — that is protocol scope, and it
//! belongs beside T335's permission queue, which already answers this question
//! for *observed* agents — the driver **parks**: it holds the run in `BLOCKED`
//! or `NEEDS_DECISION` and waits for a cancellation. The state is honest, the
//! run is not spinning, and `runs.cancel` still ends it.
//!
//! Six of the seven presets never stop on an answer and run end to end;
//! `FullLifecycle` is the one that parks, and the tests below drive its stops
//! through the handle to prove the translation of both is right. Recorded as
//! follow-up: `T300-ANSWER`.

use std::time::Duration;

use gritty_agent_fake::{
    FakeAction, FakeAdvance, FakeAgent, FakeError, FakeObservation, FakePhase, FakeScript,
    FakeScriptPreset, FakeStep,
};
use gritty_workspace::WorkspacePath;

use super::manager::AgentRunContext;
use super::state::AgentRunState;
use super::wire::AgentRunActivity;
use super::wire::{AgentRunActivityKind as ActivityKind, AgentRunEvent, AgentRunPath};
use crate::protocol::error::CoreError;
use crate::protocol::time::Timestamp;

/// How long the `runs.startFake` adapter waits between scripted steps.
///
/// A demo pace, and the only reason it exists: the fake is instantaneous, and a
/// run that reaches `FINISHED` before the panel has painted proves nothing to
/// the person watching it. Small enough that the full lifecycle reaches its
/// first stop in well under a second, long enough that each state is legible.
pub const DEMO_STEP_DELAY: Duration = Duration::from_millis(250);

/// How the caller wants the script paced.
#[derive(Debug, Clone, Copy, Default)]
pub struct FakeDriverOptions {
    /// Wait between steps. `None` runs flat out, yielding between steps so a
    /// cancellation still lands; the core's tests use it so they stay fast and
    /// deterministic.
    pub step_delay: Option<Duration>,
}

impl FakeDriverOptions {
    /// The pace a human watches, for the command adapter.
    pub const fn demo() -> Self {
        Self {
            step_delay: Some(DEMO_STEP_DELAY),
        }
    }
}

/// Resolve `runs.startFake`'s `script` name to a script.
///
/// `None` is the default preset. An unknown name is `InvalidInput` on the
/// `script` field, which is the spelling [`FakeError::core_error_code`] asks
/// for and the one `StartFakeRunRequest::validate` already uses for a name that
/// was the wrong shape — a caller sees one error type for "that name is no
/// good" whichever way it was no good.
pub fn resolve_script(name: Option<&str>) -> Result<FakeScript, CoreError> {
    match name {
        None => Ok(FakeScript::preset(DEFAULT_PRESET)),
        Some(name) => FakeScript::preset_named(name).map_err(fake_error),
    }
}

/// The preset a `runs.startFake` with no `script` runs.
///
/// `reviewed`, not `fullLifecycle`, and the reason is the parked-run gap above:
/// the acceptance script stops on a permission that nothing in this protocol
/// can answer yet, so as a *default* it would leave every first click sitting
/// in `BLOCKED` looking broken. `reviewed` visits `STARTING → RUNNING →
/// WAITING → RUNNING → REVIEW → FINISHED` and ends, which is the honest
/// default while the answer surface is missing. `fullLifecycle` is still there
/// by name, and parking is then something the caller asked for.
const DEFAULT_PRESET: FakeScriptPreset = FakeScriptPreset::Reviewed;

/// A refusal from the fake, as the `CoreError` the fake asked for.
///
/// The code comes from [`FakeError::core_error_code`] rather than from a match
/// here, so a variant the fake adds cannot quietly become an `Internal`.
fn fake_error(error: FakeError) -> CoreError {
    let message = error.to_string();
    match error.core_error_code() {
        "InvalidInput" => CoreError::invalid_input("script", message),
        "Conflict" => CoreError::conflict(message),
        // The fake named a code this arm does not know: a new variant, and
        // guessing at it would be worse than saying so.
        other => CoreError::internal(format!(
            "the fake agent asked for an unknown error code {other:?}: {message}"
        )),
    }
}

/// Build the driver closure [`super::AgentRunManager::start`] takes.
///
/// The agent is created **inside** the closure rather than captured, so that a
/// run whose driver is never scheduled cannot leave a `FakeAgent` alive
/// somewhere with nobody to resolve it.
pub fn drive(
    script: FakeScript,
    options: FakeDriverOptions,
) -> impl FnOnce(
    AgentRunContext,
) -> std::pin::Pin<Box<dyn Future<Output = Result<(), CoreError>> + Send>>
+ Send
+ 'static {
    move |context| Box::pin(run_script(context, script, options))
}

/// Advance the script to an ending, translating as it goes.
async fn run_script(
    context: AgentRunContext,
    script: FakeScript,
    options: FakeDriverOptions,
) -> Result<(), CoreError> {
    let name = script.name().to_owned();
    let mut agent = FakeAgent::new(script);
    tracing::debug!(run = %context.run_id(), script = %name, "driving a fake agent run");

    loop {
        // Between steps, never inside one: an aborted driver mid-translation
        // would leave events recorded and no state change, and the pause is
        // also the await point that makes the abort possible at all.
        if !pause(&context, options).await {
            // Cancelled. The manager already knows what ending it wants and
            // records it; reporting one here would be the supervised thing
            // answering a question that is the supervisor's.
            return Ok(());
        }

        match agent.advance() {
            FakeAdvance::Moved(observation) => {
                if let Some(ended) = apply(&context, &observation)? {
                    tracing::debug!(
                        run = %context.run_id(),
                        script = %name,
                        state = ended.as_str(),
                        "the fake script ended the run"
                    );
                    return Ok(());
                }
            }
            // Nothing happened and nothing should be reported: a duplicated or
            // out-of-order step changes no state and emits no event, the same
            // answer `AgentRunState::check_transition` gives. The step was
            // consumed, so the loop makes progress.
            FakeAdvance::Rejected { step, reason } => {
                tracing::debug!(
                    run = %context.run_id(),
                    script = %name,
                    step = step.kind(),
                    code = reason.core_error_code(),
                    "the fake refused a scripted step"
                );
            }
            // Stopped on an answer nothing in this protocol can give. Hold the
            // state and wait to be cancelled rather than spinning on an advance
            // that is documented to consume nothing.
            FakeAdvance::Blocked(phase) => {
                tracing::info!(
                    run = %context.run_id(),
                    script = %name,
                    phase = phase.as_str(),
                    "the fake run is parked on an out-of-band answer until it is cancelled"
                );
                context.cancelled().await;
                return Ok(());
            }
            // Terminal before the driver ended the run. Only reachable when the
            // agent was cancelled through its own handle, which nothing in this
            // wiring does; the manager settles it either way, and failing here
            // would race that.
            FakeAdvance::Finished(phase) => {
                tracing::debug!(
                    run = %context.run_id(),
                    script = %name,
                    phase = phase.as_str(),
                    "the fake script was already over"
                );
                return Ok(());
            }
        }
    }
}

/// Wait out the step delay, or just yield. `false` once a stop was requested.
async fn pause(context: &AgentRunContext, options: FakeDriverOptions) -> bool {
    match options.step_delay {
        Some(delay) => tokio::select! {
            biased;
            () = context.cancelled() => false,
            () = tokio::time::sleep(delay) => true,
        },
        None => {
            // Still an await point, so an abort lands, and still a
            // cancellation check, so a cooperative stop does not need one.
            tokio::task::yield_now().await;
            !context.is_cancelled()
        }
    }
}

/// Report one observation. `Some(state)` once the run has been ended.
fn apply(
    context: &AgentRunContext,
    observation: &FakeObservation,
) -> Result<Option<AgentRunState>, CoreError> {
    let action = &observation.action;
    let next = state_for(observation.phase);

    // The two endings the driver owns. Both record their own event — `finish`
    // records `RUN_FINISHED` and `fail` records `RUN_FAILED` — so the general
    // translation below must not also emit one, which is why they return here.
    match action {
        FakeAction::Step(FakeStep::Finish { summary }) => {
            context.finish(summary.clone())?;
            return Ok(Some(AgentRunState::Finished));
        }
        FakeAction::Step(FakeStep::Fail { reason }) => {
            context.fail(CoreError::internal(reason.clone()))?;
            return Ok(Some(AgentRunState::Failed));
        }
        // §11: the fake resolved a run nobody was going to end. An `Internal`
        // failure, because an orphan is Gritty's bug and not the script's.
        FakeAction::Orphaned { reason } => {
            context.fail(CoreError::internal(*reason))?;
            return Ok(Some(AgentRunState::Failed));
        }
        // Reported as an action so the fake's record is complete, but the
        // ending belongs to the manager, which is already recording it.
        FakeAction::Cancelled => return Ok(Some(AgentRunState::Cancelled)),
        _ => {}
    }

    for event in events_for(action) {
        context.record(event);
    }

    let activity = activity_for(action);
    let current = context.snapshot().map(|run| run.state);
    if current == Some(next) {
        // The state did not change, so this is an activity report. Calling
        // `advance` with the state it is already in is what the manager reads
        // as a duplicated step, and it refuses rather than absorbing it.
        context.set_activity(activity)?;
    } else {
        context.advance(next, activity)?;
    }
    Ok(None)
}

/// The `AgentRunState` a phase means.
///
/// The typed half of [`FakePhase::run_state`]; `run_state_agrees_with_the_fake`
/// asserts the two say the same thing for every phase.
fn state_for(phase: FakePhase) -> AgentRunState {
    match phase {
        FakePhase::Preparing => AgentRunState::Starting,
        FakePhase::Working => AgentRunState::Running,
        FakePhase::Idle => AgentRunState::Waiting,
        FakePhase::AwaitingPermission => AgentRunState::Blocked,
        FakePhase::AwaitingDecision => AgentRunState::NeedsDecision,
        FakePhase::Presenting => AgentRunState::Review,
        FakePhase::Done => AgentRunState::Finished,
        FakePhase::Broke => AgentRunState::Failed,
        FakePhase::Stopped => AgentRunState::Cancelled,
    }
}

/// The events an action becomes, in order.
///
/// The typed half of [`FakeAction::run_events`]. `RUN_STARTED` is absent
/// because the manager emits it, and a cancellation is absent because the
/// protocol says that with the run's state.
fn events_for(action: &FakeAction) -> Vec<AgentRunEvent> {
    match action {
        FakeAction::Step(step) => match step {
            FakeStep::Prompt { text } => vec![AgentRunEvent::PromptStarted { text: text.clone() }],
            FakeStep::Reply { text } => vec![AgentRunEvent::PromptFinished {
                text: Some(text.clone()),
            }],
            FakeStep::Read { path } => vec![AgentRunEvent::FileRead {
                path: workspace_path(path),
            }],
            FakeStep::ProposeWrite { path } => vec![AgentRunEvent::FileWriteRequested {
                path: workspace_path(path),
            }],
            FakeStep::Write { path } => vec![AgentRunEvent::FileWritten {
                path: workspace_path(path),
            }],
            FakeStep::Command { command, cwd } => vec![AgentRunEvent::CommandStarted {
                command: command.clone(),
                cwd: cwd.as_deref().map(workspace_path),
            }],
            FakeStep::Test { command } => vec![AgentRunEvent::CommandStarted {
                command: command.clone(),
                cwd: None,
            }],
            FakeStep::CommandDone { ok, exit_code } => vec![AgentRunEvent::CommandFinished {
                // The fake does not carry the command forward onto its
                // `commandDone`, and the protocol's variant requires one. The
                // ring is what pairs them: the `COMMAND_STARTED` immediately
                // before this in the same run is the command that finished.
                command: String::new(),
                ok: *ok,
                exit_code: *exit_code,
            }],
            // Two events, in this order: the ask, then the fact that the run
            // stopped on it. `RUN_BLOCKED` carries the step's own reason.
            FakeStep::AskPermission {
                tool,
                detail,
                reason,
            } => vec![
                AgentRunEvent::PermissionRequested {
                    tool: tool.clone(),
                    detail: detail.clone(),
                },
                AgentRunEvent::RunBlocked {
                    reason: reason.clone(),
                },
            ],
            // One event: `NEEDS_DECISION` is a different stop from `BLOCKED`
            // and does not also need announcing as one.
            FakeStep::AskDecision { question } => vec![AgentRunEvent::DecisionRequested {
                question: question.clone(),
            }],
            // Activity or state only: the protocol has no variant for these,
            // and inventing one to fill the gap is another ticket's call.
            FakeStep::Think | FakeStep::Search { .. } | FakeStep::Idle | FakeStep::Present => {
                Vec::new()
            }
            // Handled in `apply`, which ends the run through the manager so the
            // event and the terminal state arrive together.
            FakeStep::Finish { .. } | FakeStep::Fail { .. } => Vec::new(),
        },
        FakeAction::PermissionAnswered { .. } | FakeAction::DecisionAnswered => {
            vec![AgentRunEvent::RunResumed]
        }
        FakeAction::Cancelled | FakeAction::Orphaned { .. } => Vec::new(),
    }
}

/// What the run is doing after an action, with the payload the action carried.
///
/// The typed half of [`FakeAction::run_activity`]. The *kind* is the fake's
/// suggestion; the path and the command are this side's, because only the core
/// knows whether a path is inside the workspace.
///
/// `started_at` is **wall-clock, not the fake's clock**, and that is deliberate.
/// The fake's logical clock starts at 1 January 2026 so its observations stay
/// byte-identical between runs, which is the fake's determinism and not a fact
/// about when anything happened. The manager already stamps every event in the
/// ring with `Timestamp::now()`; taking the activity's time from the fiction
/// instead would put an activity nine months older than the events beside it in
/// the same run, and the panel would show it.
fn activity_for(action: &FakeAction) -> Option<AgentRunActivity> {
    let started_at = Timestamp::now();
    let activity = |kind| AgentRunActivity::new(kind, started_at);
    match action {
        FakeAction::Step(step) => match step {
            FakeStep::Prompt { .. } | FakeStep::Think => Some(activity(ActivityKind::Thinking)),
            FakeStep::Read { path } => {
                Some(activity(ActivityKind::Reading).with_path(workspace_path(path)))
            }
            FakeStep::Search { .. } => Some(activity(ActivityKind::Searching)),
            FakeStep::ProposeWrite { path } | FakeStep::Write { path } => {
                Some(activity(ActivityKind::Editing).with_path(workspace_path(path)))
            }
            FakeStep::Command { command, .. } => {
                Some(activity(ActivityKind::Executing).with_command(command))
            }
            FakeStep::Test { command } => {
                Some(activity(ActivityKind::Testing).with_command(command))
            }
            FakeStep::CommandDone { .. } | FakeStep::Reply { .. } => {
                Some(activity(ActivityKind::Thinking))
            }
            FakeStep::AskPermission { .. } => Some(activity(ActivityKind::RequestingPermission)),
            FakeStep::AskDecision { .. } => Some(activity(ActivityKind::RequestingDecision)),
            FakeStep::Idle => Some(activity(ActivityKind::Waiting)),
            // Every ending clears it, and so does `present`: the run is no
            // longer doing anything in particular.
            FakeStep::Present | FakeStep::Finish { .. } | FakeStep::Fail { .. } => None,
        },
        FakeAction::PermissionAnswered { .. } | FakeAction::DecisionAnswered => {
            Some(activity(ActivityKind::Thinking))
        }
        FakeAction::Cancelled | FakeAction::Orphaned { .. } => None,
    }
}

/// A path the fake reported, decided against the workspace.
///
/// The fake emits raw strings on purpose and answering this is documented as
/// the core's question: a string that parses as a workspace-relative path is
/// openable ([`AgentRunPath::inside`]), and anything else — absolute, escaping
/// the root, malformed — is shown and never resolved ([`AgentRunPath::outside`]).
/// That is the whole of the `FileTouched` rule at this seam, and it is why a
/// hostile script cannot hand the renderer something it will open.
///
/// # The tilde, which [`WorkspacePath`] is right not to reject
///
/// `~/elsewhere` **parses**: `~` is an ordinary filename character on POSIX, so
/// as a workspace-relative path it names a directory literally called `~` and
/// `WorkspacePath` is correct to accept it. It is this function that would be
/// wrong to pass it through, because an *adapter* writing `~/elsewhere` means
/// the home directory every time and never a folder named `~` in the project.
/// Accepting it marked a path outside the workspace as openable — caught by
/// `nothing_hostile_survives_the_boundary`, which is what that preset is for.
///
/// Deciding this here rather than in `gritty-workspace` is the point: the
/// convention is about what an untrusted adapter's strings mean, and the path
/// type has no adapter in it.
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
