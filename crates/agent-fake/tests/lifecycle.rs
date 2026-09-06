//! The acceptance criteria for T300-FAKE, driven through the **public** API
//! only.
//!
//! This file is an integration test on purpose. §8 requires the fake to compile
//! as an ordinary dependency rather than as a `cfg(test)` module inside
//! something else, because the manual pass drives it from the running app; an
//! integration test is the cheapest thing that fails if that ever stops being
//! true, since it sees exactly what `apps/desktop/src-tauri` will see.
//!
//! There is no `sleep`, no `Instant` and no wall-clock value anywhere below.
//! Every timestamp asserted is a logical one.

use gritty_agent_fake::{
    DEFAULT_START_MILLIS, DEFAULT_TICK_MILLIS, FakeAction, FakeAdvance, FakeAgent, FakeObservation,
    FakePhase, FakeScript, FakeScriptPreset, FakeStep, ORPHANED_DROPPED, ORPHANED_UNTERMINATED,
};

/// Drive a preset to its end, answering whatever it stops on.
///
/// This is the whole of what a supervisor does, minus the protocol: advance,
/// answer out of band when the run stops, stop when it ends.
fn drive(preset: FakeScriptPreset) -> Vec<FakeObservation> {
    let mut agent = FakeAgent::new(FakeScript::preset(preset));
    loop {
        match agent.advance() {
            FakeAdvance::Moved(_) | FakeAdvance::Rejected { .. } => {}
            FakeAdvance::Blocked(FakePhase::AwaitingPermission) => {
                agent
                    .answer_permission(true)
                    .expect("a permission is pending");
            }
            FakeAdvance::Blocked(FakePhase::AwaitingDecision) => {
                agent.answer_decision().expect("a decision is pending");
            }
            FakeAdvance::Blocked(other) => panic!("a run cannot block while {other}"),
            FakeAdvance::Finished(_) => break,
        }
    }
    agent.observations()
}

fn action_kinds(observations: &[FakeObservation]) -> Vec<&'static str> {
    observations
        .iter()
        .map(|observation| observation.action.kind())
        .collect()
}

/// The `AgentRunEvent` variants the core's translation should emit, in order.
fn run_events(observations: &[FakeObservation]) -> Vec<&'static str> {
    observations
        .iter()
        .flat_map(|observation| observation.action.run_events().iter().copied())
        .collect()
}

/// The `AgentRunState` sequence, collapsed to the changes — which is exactly
/// when the protocol emits `AgentRunChanged`.
fn run_states(observations: &[FakeObservation]) -> Vec<&'static str> {
    let mut states = vec![FakePhase::Preparing.run_state()];
    for observation in observations {
        let state = observation.phase.run_state();
        if states.last() != Some(&state) {
            states.push(state);
        }
    }
    states
}

/// The acceptance criterion, verbatim: a scripted run passes through
/// `STARTING → RUNNING → BLOCKED (permission) → RUNNING → NEEDS_DECISION →
/// RUNNING → FINISHED`.
#[test]
fn the_acceptance_script_walks_the_states_the_criterion_names() {
    let observations = drive(FakeScriptPreset::FullLifecycle);
    assert_eq!(
        run_states(&observations),
        vec![
            "STARTING",
            "RUNNING",
            "BLOCKED",
            "RUNNING",
            "NEEDS_DECISION",
            "RUNNING",
            "FINISHED",
        ]
    );
}

/// The same run, asserted step by step rather than by state, so that a change
/// in what the fake *does* is as visible as a change in where it ends up.
#[test]
fn the_acceptance_script_reports_exactly_these_actions() {
    let observations = drive(FakeScriptPreset::FullLifecycle);
    assert_eq!(
        action_kinds(&observations),
        vec![
            "prompt",
            "think",
            "search",
            "read",
            "askPermission",
            "permissionAnswered",
            "proposeWrite",
            "write",
            "askDecision",
            "decisionAnswered",
            "command",
            "commandDone",
            "test",
            "commandDone",
            "reply",
            "finish",
        ]
    );
}

/// And the events those actions become. The translation lives in `agent_run`
/// (T300-CORE) — this asserts the specification this crate publishes for it,
/// which is the half a fake can own.
#[test]
fn the_acceptance_script_specifies_exactly_this_event_sequence() {
    let observations = drive(FakeScriptPreset::FullLifecycle);
    assert_eq!(
        run_events(&observations),
        vec![
            "PROMPT_STARTED",
            "FILE_READ",
            "PERMISSION_REQUESTED",
            "RUN_BLOCKED",
            "RUN_RESUMED",
            "FILE_WRITE_REQUESTED",
            "FILE_WRITTEN",
            "DECISION_REQUESTED",
            "RUN_RESUMED",
            "COMMAND_STARTED",
            "COMMAND_FINISHED",
            "COMMAND_STARTED",
            "COMMAND_FINISHED",
            "PROMPT_FINISHED",
            "RUN_FINISHED",
        ]
    );
}

/// **The point of the whole ticket.** Twice, identical, field for field —
/// sequence numbers, logical timestamps, payloads and all. No `sleep` and no
/// wall-clock value takes part in the comparison, because none exists.
#[test]
fn every_preset_produces_a_byte_identical_run_twice() {
    for &preset in FakeScriptPreset::ALL {
        let first = drive(preset);
        let second = drive(preset);
        assert_eq!(first, second, "{preset} is not reproducible");
        assert!(!first.is_empty(), "{preset} did nothing");
        assert!(
            first
                .last()
                .is_some_and(|observation| observation.phase.is_terminal()),
            "{preset} did not end"
        );
    }
}

/// Determinism includes the clock. `seq` counts from zero and `at_millis` is a
/// fixed step per action, so the record is reproducible without a fixture
/// pinning it.
#[test]
fn sequence_numbers_and_timestamps_are_a_counter_not_a_clock() {
    let observations = drive(FakeScriptPreset::FullLifecycle);
    for (index, observation) in observations.iter().enumerate() {
        let index = index as u64;
        assert_eq!(observation.seq, index);
        assert_eq!(
            observation.at_millis,
            DEFAULT_START_MILLIS + index * DEFAULT_TICK_MILLIS
        );
    }
}

/// The state machine, walked at least once across the presets: every phase the
/// fake can reach, reached. `INTERRUPTED` is absent by design — that ending
/// belongs to a supervisor, not to a script.
#[test]
fn the_presets_and_a_cancellation_cover_every_phase_the_fake_can_reach() {
    let mut reached: Vec<FakePhase> = [
        FakeScriptPreset::FullLifecycle,
        FakeScriptPreset::Reviewed,
        FakeScriptPreset::Failing,
        FakeScriptPreset::Unterminated,
    ]
    .into_iter()
    .flat_map(|preset| {
        let mut phases = vec![FakePhase::Preparing];
        // Re-drive by hand so the blocked phases are seen before they clear.
        let mut agent = FakeAgent::new(FakeScript::preset(preset));
        loop {
            match agent.advance() {
                FakeAdvance::Moved(observation) => phases.push(observation.phase),
                FakeAdvance::Rejected { .. } => {}
                FakeAdvance::Blocked(FakePhase::AwaitingPermission) => {
                    agent.answer_permission(true).unwrap();
                }
                FakeAdvance::Blocked(FakePhase::AwaitingDecision) => {
                    agent.answer_decision().unwrap();
                }
                FakeAdvance::Blocked(other) => panic!("a run cannot block while {other}"),
                FakeAdvance::Finished(_) => break,
            }
        }
        phases
    })
    .collect();

    // The one ending no script can produce.
    let mut agent = FakeAgent::new(FakeScript::preset(FakeScriptPreset::Quick));
    agent.advance();
    reached.push(agent.cancel().expect("a running run cancels").phase);

    reached.sort_unstable_by_key(|phase| phase.as_str());
    reached.dedup();
    let names: Vec<&str> = reached.iter().map(|phase| phase.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "awaitingDecision",
            "awaitingPermission",
            "broke",
            "done",
            "idle",
            "preparing",
            "presenting",
            "stopped",
            "working",
        ]
    );
}

/// §11, half one, through the public API.
#[test]
fn a_script_that_never_ends_still_ends() {
    let mut agent = FakeAgent::new(FakeScript::preset(FakeScriptPreset::Unterminated));
    let observations = agent.advance_all();
    assert_eq!(agent.phase(), FakePhase::Broke);
    assert_eq!(agent.phase().run_state(), "FAILED");
    let last = observations.last().expect("something happened");
    assert_eq!(
        last.action,
        FakeAction::Orphaned {
            reason: ORPHANED_UNTERMINATED
        }
    );
    assert_eq!(last.action.run_events(), &["RUN_FAILED"]);
    assert!(
        ORPHANED_UNTERMINATED.contains("without a finish or fail step"),
        "the reason must name the gap"
    );
    assert!(agent.advance().is_finished());
}

/// §11, half two, through the public API: the supervisor keeps a handle, the
/// driver is handed off and never comes back, and the run resolves anyway.
#[test]
fn a_driver_that_is_dropped_mid_run_leaves_no_orphan() {
    let mut agent = FakeAgent::new(FakeScript::preset(FakeScriptPreset::FullLifecycle));
    let handle = agent.handle();
    agent.advance();
    agent.advance();
    assert_eq!(handle.phase(), FakePhase::Working);

    drop(agent);

    assert_eq!(handle.phase(), FakePhase::Broke);
    assert_eq!(handle.phase().run_state(), "FAILED");
    let last = handle.observations().pop().expect("an ending was recorded");
    assert_eq!(
        last.action,
        FakeAction::Orphaned {
            reason: ORPHANED_DROPPED
        }
    );
    // And it stays ended: a handle that outlives everything reports the same
    // thing every time it is asked.
    assert!(handle.is_finished());
    assert!(handle.cancel().is_none());
    assert_eq!(handle.phase(), FakePhase::Broke);
}

/// A cancellation lands from wherever the run is, including from a stop.
#[test]
fn a_blocked_run_can_be_cancelled_without_being_answered() {
    let mut agent = FakeAgent::new(FakeScript::preset(FakeScriptPreset::FullLifecycle));
    let handle = agent.handle();
    agent.advance_all();
    assert_eq!(agent.phase(), FakePhase::AwaitingPermission);

    let cancelled = handle.cancel().expect("a blocked run cancels");
    assert_eq!(cancelled.action, FakeAction::Cancelled);
    assert_eq!(cancelled.phase.run_state(), "CANCELLED");
    // Cancellation is a fact about the run's state, not a fifteenth event.
    assert!(cancelled.action.run_events().is_empty());
    assert!(agent.advance().is_finished());
    assert!(handle.cancel().is_none(), "idempotent");
}

/// Rule 13, from the untrusted side: the fake hands over its worst strings
/// exactly as written. Bounding them is `AgentRunEvent::bounded`'s job, once,
/// in the core — a fake that tidied its own output would prove the seam works
/// when it does not.
#[test]
fn the_fake_never_sanitises_its_own_output() {
    let observations = drive(FakeScriptPreset::Hostile);
    let FakeAction::Step(FakeStep::Prompt { text }) = &observations[0].action else {
        panic!("the hostile script starts with a prompt");
    };
    assert_eq!(text.chars().count(), 64 * 1024, "not clipped");

    let FakeAction::Step(FakeStep::Read { path }) = &observations[1].action else {
        panic!("then reads");
    };
    assert!(path.starts_with("../"), "the escape is passed through");

    let FakeAction::Step(FakeStep::Search { query }) = &observations[2].action else {
        panic!("then searches");
    };
    assert!(query.contains('\n'), "whitespace is not collapsed here");

    assert_eq!(observations.last().map(|o| o.phase), Some(FakePhase::Broke));
}

/// A supervisor batches events rather than forwarding one per call (rule 8),
/// so it needs to ask for the tail it has not seen.
#[test]
fn a_supervisor_can_batch_the_tail_it_has_not_seen() {
    let mut agent = FakeAgent::new(FakeScript::preset(FakeScriptPreset::Quick));
    let handle = agent.handle();
    let first = agent.advance();
    let seen = first.observation().expect("a step ran").seq;
    agent.advance_all();
    let tail = handle.observations_since(seen + 1);
    assert_eq!(action_kinds(&tail), vec!["reply", "finish"]);
    assert_eq!(handle.observations().len(), tail.len() + 1);
}

/// An unknown preset name is refused rather than defaulted — `runs.startFake`
/// starting the wrong script silently would be worse than not starting one.
#[test]
fn an_unknown_preset_name_is_refused() {
    let error = FakeScript::preset_named("definitely-not-a-preset").expect_err("no such preset");
    assert_eq!(error.core_error_code(), "InvalidInput");
    for &preset in FakeScriptPreset::ALL {
        assert_eq!(
            FakeScript::preset_named(preset.as_str()).unwrap().name(),
            preset.as_str()
        );
    }
}
