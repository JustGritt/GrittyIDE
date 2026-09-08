//! The translation, against a **recorded** `claude` session.
//!
//! `fixtures/two-turns-denied.jsonl` is not a hand-written stream. It is the
//! real stdout of `claude` 2.1.263 on darwin 25.6, captured on 2026-09-08 under
//! exactly the argv [`ClaudeLaunch::argv`] builds, with the machine's identity
//! redacted and nothing else changed — see `fixtures/README.md`. Every
//! assertion below is therefore about what the CLI *does*, not about what its
//! documentation says or what the author remembered.
//!
//! Nothing here spawns a process or reaches the network, which is the point:
//! CI has neither `claude` nor an API key, and the half of the adapter that
//! decides meaning must be provable without either.

use gritty_agent_claude::{
    ClaudeObservation, ClaudeStream, ClaudeToolIntent, MAX_LINE_BYTES, user_turn,
};

const RECORDING: &str = include_str!("../fixtures/two-turns-denied.jsonl");

fn replay() -> (Vec<ClaudeObservation>, ClaudeStream) {
    let mut stream = ClaudeStream::new();
    let mut out = stream.ingest(RECORDING.as_bytes());
    out.extend(stream.flush());
    (out, stream)
}

/// Every line of the recording is accounted for: nothing was dropped, nothing
/// was malformed, and the lines this crate does not model are counted rather
/// than lost.
#[test]
fn the_whole_recording_is_accounted_for() {
    let (observations, stream) = replay();
    let stats = stream.stats();
    assert_eq!(stats.lines as usize, RECORDING.lines().count());
    assert_eq!(stats.malformed, 0);
    assert_eq!(stats.oversized, 0);
    // One observation per line: this recording has no assistant message with
    // two blocks in it, so the counts happen to agree.
    assert_eq!(observations.len(), RECORDING.lines().count());
    assert!(RECORDING.lines().all(|line| line.len() < MAX_LINE_BYTES));
}

/// The shape of a real session, in this crate's vocabulary.
///
/// A sequence rather than a set, because order is the whole of what a console
/// renders: the ask has to precede the answer, and the answer has to precede
/// the tool result that carries the refusal back to the model.
#[test]
fn the_recording_translates_to_the_session_that_was_recorded() {
    let (observations, _) = replay();
    let kinds: Vec<&str> = observations.iter().map(ClaudeObservation::kind).collect();
    assert_eq!(
        kinds,
        [
            // Hooks fire before the session exists. Neither means anything to
            // a run, and neither may fail one.
            "unknown",
            "unknown",
            // The session is up: this, and not the spawn, is what ends
            // `STARTING` (§3).
            "ready",
            "unknown",
            "unknown",
            // `--replay-user-messages` earning its place.
            "promptEchoed",
            "thought",
            "toolStarted",
            "unknown",
            // The gate: the ask arrives before the tool runs.
            "permissionRequested",
            "permissionAnswered",
            // The refusal, carried back to the model as a failed tool result.
            "toolFinished",
            "unknown",
            "unknown",
            "thought",
            "said",
            "turnFinished",
            // The second turn, in the same process: `init` again.
            "ready",
            "unknown",
            "unknown",
            "promptEchoed",
            "thought",
            "said",
            "turnFinished",
        ]
    );
}

/// The state machine T310-B will drive, replayed through the published
/// mapping. Every state named here is an `AgentRunState` variant, and every
/// move is one `AgentRunState::allowed_transitions` permits.
#[test]
fn the_recording_walks_a_legal_run_through_the_state_machine() {
    let (observations, _) = replay();
    let mut states = vec!["STARTING"];
    for observation in &observations {
        if let Some(next) = observation.run_state()
            && *states.last().expect("seeded") != next
        {
            states.push(next);
        }
    }
    assert_eq!(
        states,
        [
            "STARTING", // the child is spawning
            "RUNNING",  // system/init
            "BLOCKED",  // can_use_tool
            "RUNNING",  // the answer, allowed or denied
            "WAITING",  // the turn ended
            "RUNNING",  // the second turn's init
            "WAITING",  // and it ended too
        ]
    );
}

/// The events the console would show, in order. The protocol's own spellings,
/// so T310-B's typed translation has something to be checked against.
#[test]
fn the_recording_produces_the_events_a_console_would_render() {
    let (observations, _) = replay();
    let events: Vec<&str> = observations
        .iter()
        .flat_map(|observation| observation.run_events().iter().copied())
        .collect();
    assert_eq!(
        events,
        [
            "PROMPT_STARTED",
            "COMMAND_STARTED",
            "PERMISSION_REQUESTED",
            "RUN_BLOCKED",
            "RUN_RESUMED",
            "COMMAND_FINISHED",
            "PROMPT_FINISHED",
            "PROMPT_STARTED",
            "PROMPT_FINISHED",
        ]
    );
    // Two turns, one process — which is the fact the whole `--input-format
    // stream-json` launch exists for.
    assert_eq!(events.iter().filter(|e| **e == "PROMPT_STARTED").count(), 2);
    // Nothing was written and nothing was read: the one tool call was refused.
    assert!(!events.contains(&"FILE_WRITTEN"));
    assert!(!events.contains(&"FILE_WRITE_REQUESTED"));
    assert!(!events.contains(&"FILE_READ"));
}

/// The gate, asserted as an ordering rather than as a log line.
///
/// The recording is the evidence for the claim the task's acceptance criteria
/// make: the tool did not run while the answer was outstanding. In the run that
/// produced this file the side-effect file was checked at the moment the ask
/// arrived and after the process exited, and did not exist either time. What is
/// checkable *here*, in CI, is the part that has to stay true in the stream:
/// the ask precedes the answer, and the tool's result — a failure — comes after
/// both.
#[test]
fn the_ask_precedes_the_answer_and_the_tool_never_succeeded() {
    let (observations, _) = replay();
    let at = |predicate: fn(&ClaudeObservation) -> bool| {
        observations
            .iter()
            .position(predicate)
            .expect("the recording contains it")
    };

    let started = at(|o| matches!(o, ClaudeObservation::ToolStarted(_)));
    let asked = at(|o| matches!(o, ClaudeObservation::PermissionRequested(_)));
    let answered = at(|o| matches!(o, ClaudeObservation::PermissionAnswered { .. }));
    let finished = at(|o| matches!(o, ClaudeObservation::ToolFinished(_)));
    assert!(
        started < asked && asked < answered && answered < finished,
        "started {started} asked {asked} answered {answered} finished {finished}"
    );

    let ClaudeObservation::PermissionRequested(ask) = &observations[asked] else {
        unreachable!()
    };
    assert_eq!(ask.tool, "Bash");
    assert_eq!(ask.detail.as_deref(), Some("touch probe-side-effect.txt"));
    assert!(ask.tool_use_id.is_some(), "the ask names the call it gates");

    let ClaudeObservation::PermissionAnswered { allowed, .. } = &observations[answered] else {
        unreachable!()
    };
    assert!(!allowed, "the shipped policy is a denial");

    let ClaudeObservation::ToolFinished(done) = &observations[finished] else {
        unreachable!()
    };
    assert!(!done.ok, "a denied tool call did not succeed");
    assert_eq!(
        done.call.as_ref().map(|call| &call.intent),
        Some(&ClaudeToolIntent::RunCommand {
            command: "touch probe-side-effect.txt".to_owned()
        }),
        "the refused call is paired back to what was asked for"
    );
}

/// "A denial is not a failure. The run continues; the agent is told no and
/// carries on, exactly as it does in a terminal." (§7) The recording is the
/// evidence: after the refusal the model answered, the turn ended `success`,
/// and a second turn was accepted by the same process.
#[test]
fn a_denial_does_not_end_the_run() {
    let (observations, _) = replay();
    let turns: Vec<_> = observations
        .iter()
        .filter_map(|observation| match observation {
            ClaudeObservation::TurnFinished(end) => Some(end),
            _ => None,
        })
        .collect();
    assert_eq!(turns.len(), 2);
    assert!(
        turns[0].ok,
        "the turn with the refusal in it still succeeded"
    );
    assert_eq!(turns[0].subtype, "success");
    assert_eq!(turns[0].denials, 1, "and the refusal was recorded as one");
    assert!(turns[1].ok);
    assert_eq!(turns[1].denials, 0);
    assert!(
        observations.iter().all(|o| o.run_state() != Some("FAILED")),
        "nothing in a denied-and-continued session fails the run"
    );
}

/// The reason a process is kept alive across turns at all: the second turn
/// remembered the first. Without this, `--input-format stream-json` would buy
/// nothing over two separate `-p` invocations.
#[test]
fn the_second_turn_recalled_the_first() {
    let (observations, _) = replay();
    let replies: Vec<&str> = observations
        .iter()
        .filter_map(|observation| match observation {
            ClaudeObservation::TurnFinished(end) => end.text.as_deref(),
            _ => None,
        })
        .collect();
    assert_eq!(replies.len(), 2);
    assert!(
        replies[1].contains("touch probe-side-effect.txt"),
        "the second turn should recall the command from the first: {:?}",
        replies[1]
    );
}

/// The session is the one Gritty minted, and it is reported back rather than
/// assumed — which is what lets T310-B's exclusion set (§8) name the child
/// with confidence.
#[test]
fn the_session_reports_the_id_gritty_minted() {
    let (observations, _) = replay();
    let ready: Vec<_> = observations
        .iter()
        .filter_map(|observation| match observation {
            ClaudeObservation::Ready(ready) => Some(ready),
            _ => None,
        })
        .collect();
    assert_eq!(ready.len(), 2, "one `init` per turn");
    for ready in ready {
        assert_eq!(
            ready.session_id.as_deref(),
            Some("3f8a1c2e-9b40-4d17-8e55-6a2b7c4d9f10")
        );
    }
}

/// Rule 13, at the boundary this crate is on the wrong side of: it passes
/// text through and the core bounds it. A recorded reply is the plainest case
/// — the adapter must not have tidied it.
#[test]
fn the_recording_is_passed_through_and_not_tidied() {
    let (observations, _) = replay();
    let prompts: Vec<&str> = observations
        .iter()
        .filter_map(|observation| match observation {
            ClaudeObservation::PromptEchoed { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        prompts[0],
        "Use the Bash tool to run exactly: touch probe-side-effect.txt . Do not explain, just run it."
    );
    // What was sent is what came back: the echo is a round trip, so this is
    // also a check that `user_turn` writes the shape the CLI reads.
    let sent: serde_json::Value =
        serde_json::from_str(user_turn(prompts[0]).trim_end()).expect("valid json");
    assert_eq!(sent["message"]["content"][0]["text"], prompts[0]);
}

/// The redaction held. A fixture that leaked a home directory, an API key or a
/// machine's plugin inventory would be a committed privacy problem, and this
/// is cheaper than remembering to look.
#[test]
fn the_fixture_carries_nothing_from_the_machine_that_recorded_it() {
    for needle in [
        "/Users/",
        "/home/",
        "sk-ant",
        "ANTHROPIC_API_KEY",
        "claude-501",
        ".ssh",
    ] {
        assert!(
            !RECORDING.contains(needle),
            "the fixture still contains {needle:?}"
        );
    }
}
