//! The translation, checked against the strings the adapter publishes.
//!
//! `gritty-agent-claude` and `gritty-core` cannot import each other's
//! vocabulary, so the mapping is written twice — once as strings in the
//! adapter, once as a typed match here — and these tests are what keep the two
//! the same. A variant added on either side fails one of them rather than
//! silently dropping an event, which is the whole point of the arrangement.

use gritty_agent_claude::{
    ClaudeError, ClaudeExit, ClaudeExitStatus, ClaudeObservation, ClaudePermissionAsk, ClaudeReady,
    ClaudeToolDone, ClaudeToolIntent, ClaudeToolUse, ClaudeTurnEnd,
};

use super::*;

fn tool(intent: ClaudeToolIntent) -> ClaudeToolUse {
    ClaudeToolUse {
        id: "toolu_1".to_owned(),
        name: "T".to_owned(),
        intent,
    }
}

fn ask(tool: &str, intent: ClaudeToolIntent) -> ClaudePermissionAsk {
    ClaudePermissionAsk {
        request_id: "req_1".to_owned(),
        tool: tool.to_owned(),
        intent,
        input: None,
        detail: None,
        blocked_path: None,
        tool_use_id: None,
    }
}

fn exit(status: ClaudeExitStatus, unfinished: bool) -> ClaudeExit {
    ClaudeExit {
        status,
        unfinished,
        stderr_tail: String::new(),
    }
}

/// One of every observation the adapter can produce, so the agreement tests
/// below fail when a variant is added without deciding what it means here.
///
/// The tool-call variants appear once per [`ClaudeToolIntent`], because the
/// intent is what the mapping actually branches on.
fn every_observation() -> Vec<ClaudeObservation> {
    let intents = [
        ClaudeToolIntent::ReadFile {
            path: "src/main.rs".to_owned(),
        },
        ClaudeToolIntent::WriteFile {
            path: "src/main.rs".to_owned(),
        },
        ClaudeToolIntent::RunCommand {
            command: "cargo test".to_owned(),
        },
        ClaudeToolIntent::Search {
            query: "needle".to_owned(),
        },
        ClaudeToolIntent::Other,
    ];

    let mut all = vec![
        ClaudeObservation::Ready(ClaudeReady {
            session_id: None,
            model: None,
            cwd: None,
            tools: 0,
        }),
        ClaudeObservation::PromptEchoed {
            text: "hello".to_owned(),
        },
        ClaudeObservation::Thought,
        ClaudeObservation::Said {
            text: "hi".to_owned(),
        },
        ClaudeObservation::PermissionRequested(ask(
            "Bash",
            ClaudeToolIntent::RunCommand {
                command: "rm -rf /".to_owned(),
            },
        )),
        ClaudeObservation::PermissionAnswered {
            request_id: "req_1".to_owned(),
            allowed: false,
        },
        ClaudeObservation::RequestUnsupported {
            request_id: "req_2".to_owned(),
            subtype: "mcp_message".to_owned(),
        },
        ClaudeObservation::TurnFinished(ClaudeTurnEnd {
            subtype: "success".to_owned(),
            ok: true,
            text: Some("done".to_owned()),
            denials: 0,
        }),
        ClaudeObservation::TurnFinished(ClaudeTurnEnd {
            subtype: "error_max_turns".to_owned(),
            ok: false,
            text: None,
            denials: 0,
        }),
        ClaudeObservation::Exited(exit(ClaudeExitStatus::Ok, false)),
        ClaudeObservation::Exited(exit(ClaudeExitStatus::Ok, true)),
        ClaudeObservation::Exited(exit(ClaudeExitStatus::Code(2), false)),
        ClaudeObservation::Exited(exit(ClaudeExitStatus::Signal(9), false)),
        ClaudeObservation::Exited(exit(ClaudeExitStatus::Lost, false)),
        ClaudeObservation::Unknown {
            kind: "rate_limit_event".to_owned(),
        },
    ];
    for intent in intents {
        all.push(ClaudeObservation::ToolStarted(tool(intent.clone())));
        all.push(ClaudeObservation::ToolFinished(ClaudeToolDone {
            call: Some(tool(intent.clone())),
            id: "toolu_1".to_owned(),
            ok: true,
        }));
        all.push(ClaudeObservation::ToolFinished(ClaudeToolDone {
            call: Some(tool(intent)),
            id: "toolu_1".to_owned(),
            ok: false,
        }));
    }
    // A result whose call was never seen: evicted from the outstanding table,
    // or the adapter attached mid-turn.
    all.push(ClaudeObservation::ToolFinished(ClaudeToolDone {
        call: None,
        id: "toolu_gone".to_owned(),
        ok: true,
    }));
    all
}

/// The list is one of each, so a new adapter variant that nobody added here
/// fails loudly instead of going untested.
#[test]
fn the_observation_list_covers_every_variant() {
    let observations = every_observation();
    let mut kinds: Vec<&str> = observations.iter().map(ClaudeObservation::kind).collect();
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(
        kinds,
        vec![
            "exited",
            "permissionAnswered",
            "permissionRequested",
            "promptEchoed",
            "ready",
            "requestUnsupported",
            "said",
            "thought",
            "toolFinished",
            "toolStarted",
            "turnFinished",
            "unknown",
        ],
        "every ClaudeObservation variant needs a case in every_observation()"
    );
}

#[test]
fn events_agree_with_the_adapter() {
    for observation in every_observation() {
        let published = observation.run_events();
        let translated: Vec<&str> = events_for(&observation)
            .iter()
            .map(AgentRunEvent::as_str)
            .collect();
        assert_eq!(
            translated,
            published,
            "the events for {:?} disagree with the adapter's own mapping",
            observation.kind()
        );
    }
}

#[test]
fn activity_agrees_with_the_adapter() {
    for observation in every_observation() {
        let published = observation.run_activity();
        let translated = activity_for(&observation).map(|activity| activity.kind.as_str());
        assert_eq!(
            translated,
            published,
            "the activity for {:?} disagrees with the adapter's own mapping",
            observation.kind()
        );
    }
}

/// The one place the two halves are *allowed* to disagree, and it is asserted
/// rather than left to a reader: the adapter says `INTERRUPTED` for a child
/// that was killed or whose pipe was lost, and a driver has no standing to
/// report that state (`manager.rs`, "Who moves a run"). It becomes `FAILED`,
/// and [`exit_failure`] carries the truth in its message.
#[test]
fn state_agrees_with_the_adapter_except_for_interrupted() {
    for observation in every_observation() {
        let published = observation.run_state();
        let translated = state_for(&observation).map(AgentRunState::as_str);
        match published {
            Some("INTERRUPTED") => {
                assert_eq!(
                    translated,
                    Some("FAILED"),
                    "an interrupted ending has to arrive as FAILED, not {translated:?}"
                );
                let ClaudeObservation::Exited(exit) = &observation else {
                    panic!("only an exit can be INTERRUPTED");
                };
                let message = exit_failure(exit).message;
                assert!(
                    message.contains("stopped being able to supervise it"),
                    "the failure has to say supervision was lost: {message}"
                );
            }
            other => assert_eq!(
                translated,
                other,
                "the state for {:?} disagrees with the adapter's own mapping",
                observation.kind()
            ),
        }
    }
}

/// Every error code the adapter can name is one this driver knows, so a new
/// adapter variant cannot quietly become an `Internal`.
#[test]
fn every_adapter_error_code_maps_to_itself() {
    let errors = [
        ClaudeError::Spawn {
            program: "claude".to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        },
        ClaudeError::NotReady {
            wait: Duration::from_secs(1),
        },
        ClaudeError::Closed { reason: "gone" },
        ClaudeError::Write {
            source: std::io::Error::from(std::io::ErrorKind::BrokenPipe),
        },
        ClaudeError::InvalidSessionId {
            value: "nope".to_owned(),
        },
        ClaudeError::WorkingDirectory {
            path: "/nope".to_owned(),
            reason: "missing",
        },
    ];
    for error in errors {
        let published = error.core_error_code();
        let mapped = claude_error(error);
        assert_eq!(
            mapped.code.as_str(),
            published,
            "the adapter asked for {published} and got {}",
            mapped.code.as_str()
        );
    }
}

/// ADR-010, at the one boundary an adapter's raw string crosses.
///
/// `path` present is precisely what the renderer may open, so anything that is
/// not plainly inside the workspace has to arrive with `path: None` and a
/// display string that is shown and never resolved.
#[test]
fn only_a_path_inside_the_workspace_is_openable() {
    for inside in ["src/main.rs", "a/b/c.txt", "README.md"] {
        let path = workspace_path(inside);
        assert!(path.path.is_some(), "{inside} should be openable");
        assert_eq!(path.display, inside);
    }
    for outside in [
        "/etc/passwd",
        "../../etc/passwd",
        "~/.ssh/id_rsa",
        "~",
        "C:\\Windows\\system32",
        "\\\\server\\share",
        "src/../../escape",
    ] {
        let path = workspace_path(outside);
        assert!(
            path.path.is_none(),
            "{outside} must not be openable, got {:?}",
            path.path
        );
        assert_eq!(path.display, outside, "the display string is kept verbatim");
    }
}

/// An empty path is the workspace *root*, and openable.
///
/// Not an oversight in the list above: `WorkspacePath::parse("")` returns
/// `WorkspacePath::root()` deliberately, so the root is inside the workspace by
/// definition and nothing is escaped. Asserted rather than omitted so the next
/// reader can see it was decided — an adapter reporting an empty path draws a
/// useless row, not a dangerous one, and diverging from `fake_driver` at the
/// same boundary would cost more than it saves.
#[test]
fn an_empty_path_is_the_workspace_root() {
    let path = workspace_path("");
    assert!(path.path.is_some());
    assert_eq!(path.display, "");
}

/// A tilde is rejected rather than expanded. `WorkspacePath::parse` is right to
/// refuse it and a driver would be wrong to resolve it, so the two agree.
#[test]
fn a_home_relative_path_is_never_expanded() {
    let path = workspace_path("~/secrets.txt");
    assert!(path.path.is_none());
    assert_eq!(path.display, "~/secrets.txt");
}

/// A denied write reports the *request* and never the fact.
///
/// The two events exist separately because T330 intercepts between them, so a
/// write that was refused must not produce `FILE_WRITTEN`.
#[test]
fn a_failed_write_is_never_reported_as_written() {
    let done = ClaudeToolDone {
        call: Some(tool(ClaudeToolIntent::WriteFile {
            path: "src/main.rs".to_owned(),
        })),
        id: "toolu_1".to_owned(),
        ok: false,
    };
    let events = events_for(&ClaudeObservation::ToolFinished(done));
    assert!(
        events.is_empty(),
        "a refused write reported {events:?} instead of nothing"
    );
}

/// A line nobody modelled changes nothing: no event, no activity, no state.
#[test]
fn an_unknown_message_kind_is_inert() {
    let observation = ClaudeObservation::Unknown {
        kind: "something_new".to_owned(),
    };
    assert!(events_for(&observation).is_empty());
    assert!(activity_for(&observation).is_none());
    assert!(state_for(&observation).is_none());
}

/// The one mapping that decides whether a tool runs. Only the user's own
/// `ALLOW_ONCE` becomes an allowance; a withdrawn ask is a denial, and every
/// denial tells the model why.
#[test]
fn only_the_users_allowance_lets_a_tool_run() {
    assert_eq!(
        claude_verdict(Some(AgentRunPermissionVerdict::AllowOnce)),
        gritty_agent_claude::ClaudeVerdict::Allow {
            updated_input: None
        },
        "an allowance runs the call as issued and carries no input"
    );
    for (verdict, reason) in [
        (Some(AgentRunPermissionVerdict::Deny), USER_DENIED_REASON),
        (None, DENY_BY_DEFAULT_REASON),
    ] {
        assert_eq!(
            claude_verdict(verdict),
            gritty_agent_claude::ClaudeVerdict::Deny {
                message: reason.to_owned()
            },
            "{verdict:?}"
        );
    }
}

/// The ask the user sees says what the call is about, from the adapter's own
/// classification of its input.
#[test]
fn an_ask_is_drafted_from_what_the_call_is_about() {
    let draft = |ask: ClaudePermissionAsk| {
        let draft = ask_draft(&ask);
        (draft.verb, draft.target)
    };
    assert_eq!(
        draft(ask(
            "Write",
            ClaudeToolIntent::WriteFile {
                path: "/w/src/a.rs".to_owned()
            }
        )),
        (AgentRunPermissionVerb::WriteFile, "/w/src/a.rs".to_owned())
    );
    assert_eq!(
        draft(ask(
            "Bash",
            ClaudeToolIntent::RunCommand {
                command: "cargo test".to_owned()
            }
        )),
        (AgentRunPermissionVerb::RunCommand, "cargo test".to_owned())
    );
    assert_eq!(
        draft(ask(
            "Read",
            ClaudeToolIntent::ReadFile {
                path: "/etc/hosts".to_owned()
            }
        )),
        (AgentRunPermissionVerb::Other, "/etc/hosts".to_owned())
    );
    // An unclassified tool falls back from its arguments, to its sentence, to
    // its name — never to an empty card.
    let mut mcp = ask("mcp__db__query", ClaudeToolIntent::Other);
    assert_eq!(draft(mcp.clone()).1, "mcp__db__query");
    mcp.detail = Some("Query the database".to_owned());
    assert_eq!(draft(mcp.clone()).1, "Query the database");
    mcp.input = Some(r#"{"sql":"select 1"}"#.to_owned());
    assert_eq!(draft(mcp).1, r#"{"sql":"select 1"}"#);
}

/// The echoes of asks the run never stopped for are swallowed exactly once,
/// and the memory of them is bounded.
#[test]
fn quiet_answers_are_remembered_once_and_bounded() {
    let mut asks = Asks::default();
    asks.remember_quiet("a".to_owned());
    assert!(asks.take_quiet("a"));
    assert!(
        !asks.take_quiet("a"),
        "an echo is swallowed once, not forever"
    );
    for index in 0..MAX_QUIET_ANSWERS + 5 {
        asks.remember_quiet(format!("r{index}"));
    }
    assert_eq!(asks.quiet.len(), MAX_QUIET_ANSWERS);
    assert!(!asks.take_quiet("r0"), "the oldest is evicted first");
}

/// `RUN_BLOCKED` says what was asked for, so the console can draw it without
/// having to pair the event with the ask beside it.
#[test]
fn a_block_names_the_tool_that_asked() {
    let ask = ClaudePermissionAsk {
        detail: Some("cargo test".to_owned()),
        ..ask(
            "Bash",
            ClaudeToolIntent::RunCommand {
                command: "cargo test".to_owned(),
            },
        )
    };
    assert_eq!(blocked_reason(&ask), "Bash needs permission: cargo test");
    let bare = ClaudePermissionAsk {
        detail: None,
        ..ask
    };
    assert_eq!(blocked_reason(&bare), "Bash needs permission");
}

/// A clean exit with a turn still outstanding is a failure, not a finish: it
/// looked like it worked and nothing happened.
#[test]
fn stdout_closing_mid_turn_is_a_failure() {
    let observation = ClaudeObservation::Exited(exit(ClaudeExitStatus::Ok, true));
    assert_eq!(state_for(&observation), Some(AgentRunState::Failed));
}
