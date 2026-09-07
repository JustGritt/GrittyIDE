//! The fake driver, against the real manager (T300-WIRE).
//!
//! Two kinds of test, and the split matters.
//!
//! **Agreement.** `gritty-agent-fake` publishes the translation as strings it
//! cannot itself perform, and [`super`] performs it typed. Nothing but a test
//! keeps the two the same, so the first half of this file is that test: every
//! action's events, every action's activity kind, every phase's state, and the
//! error code a refusal asks for. A variant added on this side fails to
//! compile (the translation is an exhaustive `match`); a variant added on the
//! fake's side fails here.
//!
//! **Behaviour.** The second half starts real runs through a real
//! [`AgentRunManager`] and drives every preset the command surface offers. No
//! assertion waits on a duration: a run's ending is awaited on the event
//! stream, and the driver is paced with `step_delay: None` so the tests are as
//! fast as the fake is.

use std::sync::Arc;
use std::time::Duration;

use gritty_agent_fake::{
    CHATTY_STEPS, FakeAction, FakeError, FakePhase, FakeScript, FakeScriptPreset, FakeStep,
    ORPHANED_DROPPED,
};
use gritty_persistence::testing::TempDir;
use tokio::runtime::Handle;
use tokio::time::timeout;

use super::{
    DEFAULT_PRESET, FakeDriverOptions, activity_for, drive, events_for, fake_error, resolve_script,
    state_for, workspace_path,
};
use crate::agent_run::manager::{AgentRunManager, AgentRunSpec};
use crate::agent_run::state::AgentRunState;
use crate::agent_run::wire::{
    AgentRun, AgentRunEvent, AgentRunEventRecord, AgentRunProvider, MAX_COMMAND, MAX_DISPLAY_PATH,
    MAX_RUN_EVENTS, MAX_TEXT, MAX_TOOL,
};
use crate::event::testing::RecordingSink;
use crate::event::{CoreEvent, EventBus, EventEnvelope};
use crate::operation::OperationRegistry;
use crate::protocol::actor::Actor;
use crate::protocol::error::CoreErrorCode;
use crate::protocol::ids::{AgentRunId, CorrelationId, WorkstreamId};
use crate::workspace::{UnsavedPolicy, Workspace, WorkspaceManager};

// ---------------------------------------------------------------------------
// Agreement with the specification the fake publishes
// ---------------------------------------------------------------------------

/// One of every [`FakeAction`] shape: the sixteen steps plus the four things
/// that happen *to* a run. Payloads are irrelevant; only the shapes are.
///
/// The fake keeps the same list for its own tests and cannot share it — it is
/// `#[cfg(test)]` inside that crate — so this is a second copy on purpose. It
/// is not the only guard: [`super::events_for`] and [`super::activity_for`]
/// match exhaustively, so a variant added to `FakeStep` breaks the build here
/// before it can slip past this list.
fn every_action() -> Vec<FakeAction> {
    let steps = vec![
        FakeStep::prompt("p"),
        FakeStep::Think,
        FakeStep::read("a"),
        FakeStep::search("q"),
        FakeStep::propose_write("a"),
        FakeStep::write("a"),
        FakeStep::command("c"),
        FakeStep::test("t"),
        FakeStep::CommandDone {
            ok: true,
            exit_code: Some(0),
        },
        FakeStep::reply("r"),
        FakeStep::ask_permission("Edit", "why"),
        FakeStep::ask_decision("q?"),
        FakeStep::Idle,
        FakeStep::Present,
        FakeStep::finish("s"),
        FakeStep::fail("e"),
    ];
    let mut actions: Vec<FakeAction> = steps.into_iter().map(FakeAction::Step).collect();
    actions.push(FakeAction::PermissionAnswered { granted: true });
    actions.push(FakeAction::DecisionAnswered);
    actions.push(FakeAction::Cancelled);
    actions.push(FakeAction::Orphaned {
        reason: ORPHANED_DROPPED,
    });
    actions
}

#[test]
fn the_action_list_is_one_of_each() {
    let actions = every_action();
    let mut kinds: Vec<&str> = actions.iter().map(FakeAction::kind).collect();
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(kinds.len(), actions.len(), "an action shape is duplicated");
    assert_eq!(actions.len(), 20, "the fake's vocabulary changed shape");
}

/// The heart of the arrangement: what this crate builds is what the other
/// crate said it should build, for every action, in order.
///
/// `finish` and `fail` are the two the driver does not translate here — the
/// manager records their event as part of ending the run, so `events_for`
/// returns nothing for them and `apply` routes them instead. They are asserted
/// separately below rather than excused.
#[test]
fn events_agree_with_the_fake() {
    for action in every_action() {
        let ours: Vec<&'static str> = events_for(&action)
            .iter()
            .map(AgentRunEvent::as_str)
            .collect();
        let theirs = action.run_events();
        if matches!(
            action,
            FakeAction::Step(FakeStep::Finish { .. } | FakeStep::Fail { .. })
                | FakeAction::Orphaned { .. }
        ) {
            assert!(
                ours.is_empty(),
                "{} is ended by the manager, so the translation emits nothing: got {ours:?}",
                action.kind()
            );
            continue;
        }
        assert_eq!(ours, theirs, "{} translated wrongly", action.kind());
    }
}

/// The three the manager ends, and the event it records while doing so. Named
/// here so the exemption above is a claim with a test behind it rather than a
/// hole.
#[test]
fn the_endings_the_manager_records_are_the_ones_the_fake_asked_for() {
    for action in every_action() {
        let expected: &[&str] = match &action {
            FakeAction::Step(FakeStep::Finish { .. }) => &["RUN_FINISHED"],
            FakeAction::Step(FakeStep::Fail { .. }) | FakeAction::Orphaned { .. } => {
                &["RUN_FAILED"]
            }
            _ => continue,
        };
        assert_eq!(action.run_events(), expected, "{}", action.kind());
    }
}

#[test]
fn activities_agree_with_the_fake() {
    for action in every_action() {
        let ours = activity_for(&action).map(|activity| activity.kind.as_str());
        assert_eq!(
            ours,
            action.run_activity(),
            "{} put the run in the wrong activity",
            action.kind()
        );
    }
}

#[test]
fn run_states_agree_with_the_fake() {
    for phase in FakePhase::ALL.iter().copied() {
        assert_eq!(
            state_for(phase).as_str(),
            phase.run_state(),
            "{} maps to the wrong state",
            phase.as_str()
        );
    }
}

/// Every ending a script can reach is one the core has; `INTERRUPTED` is the
/// one it cannot, and that is the supervisor's alone.
#[test]
fn a_script_cannot_reach_interrupted() {
    let reachable: Vec<AgentRunState> = FakePhase::ALL
        .iter()
        .copied()
        .map(state_for)
        .filter(|state| state.is_terminal())
        .collect();
    assert!(!reachable.contains(&AgentRunState::Interrupted));
    assert!(reachable.contains(&AgentRunState::Finished));
    assert!(reachable.contains(&AgentRunState::Failed));
    assert!(reachable.contains(&AgentRunState::Cancelled));
}

/// Which field an `InvalidInput` blamed. `CoreError::invalid_input` puts it in
/// `details`, so that is where a test has to look for it.
fn field_of(error: &crate::protocol::error::CoreError) -> Option<&str> {
    error.details.as_ref()?.get("field")?.as_str()
}

#[test]
fn a_refusal_becomes_the_code_the_fake_named() {
    let unknown = FakeError::UnknownPreset {
        name: "nope".to_owned(),
    };
    let error = fake_error(unknown);
    assert_eq!(error.code, CoreErrorCode::InvalidInput);
    assert_eq!(field_of(&error), Some("script"));

    let conflict = fake_error(FakeError::AlreadyEnded {
        phase: FakePhase::Done,
    });
    assert_eq!(conflict.code, CoreErrorCode::Conflict);
}

#[test]
fn an_unknown_script_name_is_invalid_input_and_a_known_one_resolves() {
    let error = resolve_script(Some("no-such-script")).expect_err("an unknown name is refused");
    assert_eq!(error.code, CoreErrorCode::InvalidInput);
    assert_eq!(field_of(&error), Some("script"));
    assert!(
        error.message.contains("no-such-script"),
        "the message names what was asked for: {}",
        error.message
    );

    for preset in FakeScriptPreset::ALL.iter().copied() {
        let script = resolve_script(Some(preset.as_str())).expect("a preset resolves");
        assert_eq!(script.name(), preset.as_str());
    }
}

/// The default is the preset that *ends*. `fullLifecycle` parks on a permission
/// nothing can answer yet, so it must not be what an empty request runs.
#[test]
fn the_default_script_is_one_that_finishes() {
    let script = resolve_script(None).expect("the default resolves");
    assert_eq!(script.name(), DEFAULT_PRESET.as_str());
    assert_ne!(DEFAULT_PRESET, FakeScriptPreset::FullLifecycle);
    assert!(
        !script.steps().iter().any(|step| matches!(
            step,
            FakeStep::AskPermission { .. } | FakeStep::AskDecision { .. }
        )),
        "the default script must not stop on an answer nothing can give"
    );
    assert!(
        script
            .steps()
            .iter()
            .any(|step| matches!(step, FakeStep::Finish { .. })),
        "the default script must end"
    );
}

/// Which adapter-reported paths the renderer is allowed to open.
///
/// `path: Some(..)` is precisely what the WebView may act on, so every case
/// that must not be openable is named here rather than left to the hostile
/// preset to stumble over. `~/…` is the one that is not obvious: it parses as a
/// perfectly valid workspace-relative path naming a directory called `~`, and
/// it took `nothing_hostile_survives_the_boundary` to notice that meant an
/// outside path was arriving openable.
#[test]
fn only_a_path_actually_inside_the_workspace_is_openable() {
    for raw in ["src/main.rs", "a", "deep/nested/file.txt"] {
        let path = workspace_path(raw);
        assert_eq!(
            path.path.as_ref().map(|inside| inside.as_str()),
            Some(raw),
            "{raw} is inside the workspace and should be openable"
        );
        assert_eq!(path.display, raw);
    }

    for raw in [
        "../../../../etc/passwd",
        "/etc/passwd",
        "~/elsewhere",
        "~",
        "src/../../out",
        "./here",
        "back\\slash",
        "trailing/",
    ] {
        let path = workspace_path(raw);
        assert!(
            path.path.is_none(),
            "{raw} is not inside the workspace but came back openable: {path:?}"
        );
        assert!(
            !path.display.is_empty(),
            "{raw} must still be shown, just never resolved"
        );
    }
}

// ---------------------------------------------------------------------------
// The driver, against a real manager
// ---------------------------------------------------------------------------

struct Harness {
    manager: Arc<AgentRunManager>,
    workspace: Workspace,
    sink: Arc<RecordingSink>,
    rx: tokio::sync::mpsc::UnboundedReceiver<EventEnvelope>,
    _dir: TempDir,
}

fn start() -> Harness {
    let dir = TempDir::new("fake-driver-ws");
    let (sink, rx) = RecordingSink::with_channel();
    let events = EventBus::new(sink.clone());
    let registry = OperationRegistry::new(Handle::current(), events.clone());
    let workspaces = WorkspaceManager::new(Handle::current(), events.clone(), registry);
    let workspace = workspaces
        .open(
            dir.path(),
            UnsavedPolicy::Refuse,
            Actor::User,
            CorrelationId::new(),
        )
        .expect("open the workspace");
    let manager = AgentRunManager::new(Handle::current(), events, workspaces);
    Harness {
        manager,
        workspace,
        sink,
        rx,
        _dir: dir,
    }
}

impl Harness {
    /// Start a run on a preset, driven as fast as the fake can go.
    fn run(&self, preset: FakeScriptPreset) -> AgentRun {
        self.run_script(FakeScript::preset(preset))
    }

    fn run_script(&self, script: FakeScript) -> AgentRun {
        let spec = AgentRunSpec {
            workspace_id: self.workspace.id.clone(),
            workstream_id: WorkstreamId::from_raw("ws-1"),
            provider: AgentRunProvider::Fake,
            actor: Actor::User,
            correlation_id: CorrelationId::new(),
        };
        self.manager
            .start(spec, drive(script, FakeDriverOptions::default()))
            .expect("the run starts")
    }

    async fn ended(&mut self, id: &AgentRunId) -> AgentRun {
        self.await_run(id, |run| run.state.is_terminal()).await
    }

    /// Wait on the event stream — never on a duration — for a run the predicate
    /// accepts.
    async fn await_run(&mut self, id: &AgentRunId, accept: impl Fn(&AgentRun) -> bool) -> AgentRun {
        timeout(Duration::from_secs(10), async {
            if let Some(run) = self.manager.get(id).filter(&accept) {
                return run;
            }
            loop {
                let envelope = self.rx.recv().await.expect("the event stream is open");
                if let CoreEvent::AgentRunChanged(payload) = &envelope.event
                    && payload.run.id == *id
                    && accept(&payload.run)
                {
                    return payload.run.clone();
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("run {id} never reached the state the test wanted"))
    }

    /// The states this run was announced in, with consecutive repeats folded
    /// away. An activity change re-announces the run in the state it is already
    /// in, which is correct and is not what these tests are about.
    fn state_path(&self, id: &AgentRunId) -> Vec<&'static str> {
        let mut path: Vec<&'static str> = Vec::new();
        for envelope in self.sink.recorded().iter() {
            if let CoreEvent::AgentRunChanged(payload) = &envelope.event
                && payload.run.id == *id
                && path.last() != Some(&payload.run.state.as_str())
            {
                path.push(payload.run.state.as_str());
            }
        }
        path
    }

    /// Every event this run reported, in order, flattened out of its batches.
    fn events(&self, id: &AgentRunId) -> Vec<AgentRunEvent> {
        self.sink
            .recorded()
            .iter()
            .filter_map(|envelope| match &envelope.event {
                CoreEvent::AgentRunEventAppended(payload) if payload.run_id == *id => {
                    Some(payload.events.iter().map(|record| record.event.clone()))
                }
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn kinds(&self, id: &AgentRunId) -> Vec<&'static str> {
        self.events(id).iter().map(AgentRunEvent::as_str).collect()
    }
}

/// The whole point, end to end: the default preset walks four states and ends,
/// with no help from the test.
#[tokio::test]
async fn the_default_preset_runs_itself_to_finished() {
    let mut harness = start();
    let run = harness.run(DEFAULT_PRESET);
    let ended = harness.ended(&run.id).await;

    assert_eq!(ended.state, AgentRunState::Finished);
    assert!(ended.error.is_none());
    assert!(ended.finished_at.is_some());
    assert!(ended.activity.is_none(), "an ended run is doing nothing");
    assert_eq!(
        harness.state_path(&run.id),
        vec![
            "STARTING", "RUNNING", "WAITING", "RUNNING", "REVIEW", "FINISHED"
        ],
        "the preset's documented path through the state machine"
    );

    let kinds = harness.kinds(&run.id);
    assert_eq!(kinds.first(), Some(&"RUN_STARTED"), "the manager's, first");
    assert_eq!(kinds.last(), Some(&"RUN_FINISHED"), "and the ending, last");
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "RUN_STARTED").count(),
        1,
        "the protocol promises RUN_STARTED exactly once"
    );
}

/// Determinism is the fake's product, so it has to survive the driver: the same
/// preset twice gives the same events and the same states.
#[tokio::test]
async fn the_same_script_twice_reports_the_same_thing() {
    let mut harness = start();
    let first = harness.run(FakeScriptPreset::Quick);
    harness.ended(&first.id).await;
    let second = harness.run(FakeScriptPreset::Quick);
    harness.ended(&second.id).await;

    assert_eq!(harness.kinds(&first.id), harness.kinds(&second.id));
    assert_eq!(
        harness.state_path(&first.id),
        harness.state_path(&second.id)
    );
    assert_eq!(
        harness.kinds(&first.id),
        vec![
            "RUN_STARTED",
            "PROMPT_STARTED",
            "PROMPT_FINISHED",
            "RUN_FINISHED"
        ]
    );
}

#[tokio::test]
async fn a_scripted_failure_ends_the_run_failed_and_says_why() {
    let mut harness = start();
    let run = harness.run(FakeScriptPreset::Failing);
    let ended = harness.ended(&run.id).await;

    assert_eq!(ended.state, AgentRunState::Failed);
    let error = ended.error.expect("a failed run says why");
    assert!(
        error.message.contains("the provider closed the connection"),
        "{}",
        error.message
    );
    assert_eq!(harness.kinds(&run.id).last(), Some(&"RUN_FAILED"));
}

/// §11: a script that runs out with nothing terminal in it resolves rather than
/// sitting in `RUNNING` forever. The fake notices; the driver reports it as the
/// `Internal` failure it is, because an orphan is Gritty's bug.
#[tokio::test]
async fn an_unterminated_script_resolves_rather_than_hanging() {
    let mut harness = start();
    let run = harness.run(FakeScriptPreset::Unterminated);
    let ended = harness.ended(&run.id).await;

    assert_eq!(ended.state, AgentRunState::Failed);
    let error = ended.error.expect("an orphan says why");
    assert_eq!(error.code, CoreErrorCode::Internal);
    assert!(
        error.message.contains("without a finish or fail"),
        "{}",
        error.message
    );
}

/// The parked run, which is T300-WIRE's one deliberate gap. It holds `BLOCKED`
/// honestly instead of spinning, and `runs.cancel` is what ends it.
#[tokio::test]
async fn a_run_that_stops_on_a_permission_parks_until_it_is_cancelled() {
    let mut harness = start();
    let run = harness.run(FakeScriptPreset::FullLifecycle);
    let blocked = harness
        .await_run(&run.id, |run| run.state == AgentRunState::Blocked)
        .await;

    assert_eq!(blocked.state, AgentRunState::Blocked);
    let activity = blocked.activity.expect("a blocked run says what it wants");
    assert_eq!(activity.kind.as_str(), "REQUESTING_PERMISSION");
    let kinds = harness.kinds(&run.id);
    let ask = kinds
        .iter()
        .position(|kind| *kind == "PERMISSION_REQUESTED")
        .expect("the ask was reported");
    assert_eq!(
        kinds.get(ask + 1),
        Some(&"RUN_BLOCKED"),
        "the ask, then the fact that the run stopped on it"
    );

    // It stays parked: nothing in this protocol answers a permission yet.
    assert!(
        !harness
            .manager
            .get(&run.id)
            .expect("still there")
            .state
            .is_terminal(),
        "a parked run has not ended itself"
    );

    harness
        .manager
        .cancel(&run.id, Actor::User)
        .expect("a parked run cancels");
    let ended = harness.ended(&run.id).await;
    assert_eq!(ended.state, AgentRunState::Cancelled);
    assert!(
        ended.error.is_none(),
        "a cancellation is not a failure and carries no error"
    );
}

/// A cancellation lands on a run that is working flat out, not only on a parked
/// one. `step_delay: None` still yields between steps, which is the await point
/// that makes this possible at all.
#[tokio::test]
async fn a_working_run_can_be_cancelled_mid_script() {
    let mut harness = start();
    let run = harness.run(FakeScriptPreset::Chatty);
    harness
        .await_run(&run.id, |run| run.state == AgentRunState::Running)
        .await;

    harness
        .manager
        .cancel(&run.id, Actor::User)
        .expect("a working run cancels");
    let ended = harness.ended(&run.id).await;
    assert_eq!(ended.state, AgentRunState::Cancelled);
}

/// Rule 8, from the other end: a script that reports far more than the ring
/// holds still ends, and the log it leaves is bounded.
#[tokio::test]
async fn a_chatty_script_cannot_grow_the_log_without_limit() {
    let mut harness = start();
    let run = harness.run(FakeScriptPreset::Chatty);
    let ended = harness.ended(&run.id).await;
    assert_eq!(ended.state, AgentRunState::Finished);

    let log = harness.manager.events(&run.id).expect("the log is there");
    assert!(
        log.events.len() <= MAX_RUN_EVENTS,
        "{} events kept, but the ring holds {MAX_RUN_EVENTS}",
        log.events.len()
    );
    // Both sides are constants, so this is a fact about the two crates rather
    // than about this run: checked at compile time, where it belongs.
    const {
        assert!(
            CHATTY_STEPS > MAX_RUN_EVENTS,
            "the preset has to overflow the ring for this to prove anything"
        );
    }
}

/// Rule 13 at the seam that matters. The fake emits its worst strings verbatim
/// — that is what it is for — and every one of them arrives bounded, collapsed
/// and, for a path that leaves the root, unopenable.
#[tokio::test]
async fn nothing_hostile_survives_the_boundary() {
    let mut harness = start();
    let run = harness.run(FakeScriptPreset::Hostile);
    // Hostile stops on a permission before its `fail`, like the acceptance
    // script does; everything under test is reported before that.
    harness
        .await_run(&run.id, |run| run.state == AgentRunState::Blocked)
        .await;
    let events = harness.events(&run.id);

    for event in &events {
        match event {
            AgentRunEvent::PromptStarted { text } => {
                assert!(
                    text.chars().count() <= MAX_TEXT,
                    "a 64 KiB prompt reached the wire, {} characters long",
                    text.chars().count()
                );
            }
            // The one that would be a security bug rather than a display bug:
            // `../../../../etc/passwd` must not come back openable.
            AgentRunEvent::FileRead { path } => {
                assert!(
                    path.path.is_none(),
                    "a path that escapes the root was made openable: {path:?}"
                );
                assert!(!path.display.is_empty());
                assert!(path.display.chars().count() <= MAX_DISPLAY_PATH);
            }
            AgentRunEvent::CommandStarted { command, cwd } => {
                assert!(
                    command.chars().count() <= MAX_COMMAND,
                    "an oversized command reached the wire"
                );
                let cwd = cwd.as_ref().expect("the hostile script names a cwd");
                assert!(
                    cwd.path.is_none(),
                    "a cwd outside the workspace was made openable: {cwd:?}"
                );
            }
            AgentRunEvent::RunBlocked { reason } => {
                assert!(
                    !reason.contains('\n') && !reason.contains('\r'),
                    "a newline survived into a one-line field: {reason:?}"
                );
                assert_eq!(reason.trim(), *reason, "the reason was not collapsed");
            }
            AgentRunEvent::PermissionRequested { tool, .. } => {
                assert!(
                    tool.chars().count() <= MAX_TOOL,
                    "a 4 KiB tool name reached the wire"
                );
            }
            _ => {}
        }
    }

    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentRunEvent::FileRead { .. })),
        "the hostile read has to be among the events for this to prove anything"
    );

    harness
        .manager
        .cancel(&run.id, Actor::User)
        .expect("the parked hostile run cancels");
    harness.ended(&run.id).await;
}

/// Every preset the command surface offers reaches a state the user can act on
/// — ended, or parked on an answer — rather than any of them hanging.
#[tokio::test]
async fn every_preset_the_command_surface_offers_settles() {
    let mut harness = start();
    for preset in FakeScriptPreset::ALL.iter().copied() {
        let run = harness.run(preset);
        let settled = harness
            .await_run(&run.id, |run| {
                run.state.is_terminal()
                    || matches!(
                        run.state,
                        AgentRunState::Blocked | AgentRunState::NeedsDecision
                    )
            })
            .await;
        if !settled.state.is_terminal() {
            harness
                .manager
                .cancel(&run.id, Actor::User)
                .expect("a parked run cancels");
            harness.ended(&run.id).await;
        }
    }
}

/// The record the panel reads back is the record that was broadcast: `runs.events`
/// and the `AgentRunEventAppended` stream cannot disagree about what happened.
#[tokio::test]
async fn the_replayed_log_matches_what_was_broadcast() {
    let mut harness = start();
    let run = harness.run(FakeScriptPreset::Quick);
    harness.ended(&run.id).await;

    let log = harness.manager.events(&run.id).expect("the log is there");
    let replayed: Vec<&'static str> = log
        .events
        .iter()
        .map(|record: &AgentRunEventRecord| record.event.as_str())
        .collect();
    assert_eq!(replayed, harness.kinds(&run.id));
}
