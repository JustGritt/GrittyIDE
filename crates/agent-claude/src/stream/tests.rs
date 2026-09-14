//! The translation, proved without a process.

use super::*;

fn observe(lines: &str) -> (Vec<ClaudeObservation>, ClaudeStreamStats) {
    let mut stream = ClaudeStream::new();
    let mut out = stream.ingest(lines.as_bytes());
    out.extend(stream.flush());
    (out, stream.stats())
}

fn only(lines: &str) -> ClaudeObservation {
    let (mut out, _) = observe(lines);
    assert_eq!(out.len(), 1, "{out:?}");
    out.remove(0)
}

// ---------------------------------------------------------------------------
// Framing
// ---------------------------------------------------------------------------

/// A pipe delivers arbitrary chunks. Feeding the same bytes one at a time has
/// to give the same answer as feeding them all at once, or the adapter's
/// output depends on the operating system's buffering.
#[test]
fn framing_does_not_depend_on_chunk_boundaries() {
    let text = concat!(
        r#"{"type":"system","subtype":"init","session_id":"s","tools":["Bash"]}"#,
        "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#,
        "\n",
        r#"{"type":"result","subtype":"success","is_error":false,"result":"hi"}"#,
        "\n",
    );
    let (whole, whole_stats) = observe(text);

    let mut stream = ClaudeStream::new();
    let mut byte_at_a_time = Vec::new();
    for byte in text.as_bytes() {
        byte_at_a_time.extend(stream.ingest(&[*byte]));
    }
    byte_at_a_time.extend(stream.flush());

    assert_eq!(whole, byte_at_a_time);
    assert_eq!(whole_stats, stream.stats());
    assert_eq!(whole.len(), 3);
}

/// A child killed mid-write leaves a partial line with no newline. It is
/// malformed, and the *counter* is how that is visible — silently forgetting
/// the bytes would make a truncated stream indistinguishable from a clean one.
#[test]
fn a_line_with_no_trailing_newline_is_flushed_and_counted() {
    let mut stream = ClaudeStream::new();
    assert!(
        stream
            .ingest(br#"{"type":"result","subtype":"succ"#)
            .is_empty()
    );
    assert!(stream.flush().is_empty());
    assert_eq!(stream.stats().malformed, 1);
}

/// Blank lines are not garbage and must not inflate the malformed counter.
#[test]
fn blank_lines_are_not_malformed() {
    let (out, stats) = observe("\n  \n\r\n");
    assert!(out.is_empty());
    assert_eq!(stats, ClaudeStreamStats::default());
}

/// "A malformed or oversized JSON line is dropped with a counter, never parsed
/// into a truncated event and never fatal."
#[test]
fn malformed_lines_are_dropped_with_a_counter_and_never_fatal() {
    let good = format!("{}\n", r#"{"type":"result","subtype":"success"}"#);
    let text = format!(
        "{}{}{}{}{}",
        "not json at all\n",
        "{\"type\": \n",             // truncated object
        "[1,2,3]\n",                 // json, but not an object
        "{\"no\":\"type field\"}\n", // object, but no `type`
        good,
    );
    let (out, stats) = observe(&text);
    assert_eq!(stats.malformed, 4);
    // The good line still arrived: one bad line does not poison the stream.
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0], ClaudeObservation::TurnFinished(_)));
}

/// An oversized line is discarded **whole**, to the next newline, and the line
/// after it is unaffected. Half a JSON object parsed into a half-event is the
/// truncated event the failure table forbids.
#[test]
fn an_oversized_line_is_discarded_whole() {
    let padding = "x".repeat(MAX_LINE_BYTES + 10);
    let text = format!(
        "{}\n{}\n",
        format_args!(r#"{{"type":"assistant","pad":"{padding}"}}"#),
        r#"{"type":"result","subtype":"success"}"#
    );
    let (out, stats) = observe(&text);
    assert_eq!(stats.oversized, 1);
    assert_eq!(stats.malformed, 0);
    assert_eq!(out.len(), 1, "{out:?}");
    assert!(matches!(out[0], ClaudeObservation::TurnFinished(_)));
}

/// The overflow flag has to survive a chunk boundary, or an oversized line
/// split across two reads would resume mid-object and parse as garbage.
#[test]
fn an_oversized_line_stays_discarded_across_chunks() {
    let mut stream = ClaudeStream::new();
    let padding = "x".repeat(MAX_LINE_BYTES + 10);
    let oversized = format!(r#"{{"type":"assistant","pad":"{padding}"}}"#);
    for chunk in oversized.as_bytes().chunks(4096) {
        assert!(stream.ingest(chunk).is_empty());
    }
    assert_eq!(stream.stats().oversized, 1);
    let out = stream.ingest(b"\n{\"type\":\"result\",\"subtype\":\"success\"}\n");
    assert_eq!(out.len(), 1);
    assert_eq!(stream.stats().malformed, 0);
}

// ---------------------------------------------------------------------------
// Unknown kinds
// ---------------------------------------------------------------------------

/// "An unrecognised `type` on the wire is recorded as unknown and skipped, not
/// an error: the CLI is a moving target and a new message kind must not fail a
/// run." All four of these are in the recorded fixture.
#[test]
fn unmodelled_kinds_are_recorded_and_skipped() {
    let (out, stats) = observe(concat!(
        r#"{"type":"rate_limit_event"}"#,
        "\n",
        r#"{"type":"system","subtype":"hook_started"}"#,
        "\n",
        r#"{"type":"system","subtype":"thinking_tokens"}"#,
        "\n",
        r#"{"type":"something_invented_next_year"}"#,
        "\n",
    ));
    let kinds: Vec<_> = out
        .iter()
        .map(|observation| match observation {
            ClaudeObservation::Unknown { kind } => kind.as_str(),
            other => panic!("{other:?} should have been unknown"),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "rate_limit_event",
            "system/hook_started",
            "system/thinking_tokens",
            "something_invented_next_year"
        ]
    );
    assert_eq!(stats.unknown, 4);
    assert_eq!(stats.malformed, 0);
}

/// The `type` is the child's string too, so it is capped — and cut on a
/// character boundary, because a UTF-8 kind sliced mid-codepoint would panic.
#[test]
fn an_unknown_kind_is_bounded_on_a_character_boundary() {
    let kind = "é".repeat(MAX_KIND_BYTES);
    let observation = only(&format!("{{\"type\":\"{kind}\"}}\n"));
    let ClaudeObservation::Unknown { kind } = observation else {
        panic!("expected unknown");
    };
    assert!(kind.len() <= MAX_KIND_BYTES, "{}", kind.len());
    assert!(kind.starts_with('é'));
}

/// A known kind whose body is the wrong shape fails only itself, and says
/// which kind it was rather than vanishing into the malformed counter.
#[test]
fn a_known_kind_with_an_unexpected_body_reports_as_that_kind() {
    let observation = only(r#"{"type":"assistant","message":{"content":"not a list"}}"#);
    assert_eq!(
        observation,
        ClaudeObservation::Unknown {
            kind: "assistant".to_owned()
        }
    );
}

// ---------------------------------------------------------------------------
// The vocabulary
// ---------------------------------------------------------------------------

/// `init` is what ends `STARTING` (§3), not the spawn.
#[test]
fn init_is_what_reports_a_ready_session() {
    let mut stream = ClaudeStream::new();
    assert!(!stream.is_ready());
    let out = stream.ingest(
        br#"{"type":"system","subtype":"init","session_id":"s-1","model":"m","cwd":"/w","tools":["Bash","Read"]}
"#,
    );
    assert!(stream.is_ready());
    assert_eq!(
        out,
        vec![ClaudeObservation::Ready(ClaudeReady {
            session_id: Some("s-1".to_owned()),
            model: Some("m".to_owned()),
            cwd: Some("/w".to_owned()),
            tools: 2,
        })]
    );
    assert_eq!(out[0].run_state(), Some("RUNNING"));
}

/// Verified against the real CLI: `init` arrives once per **turn**. The second
/// one is the run going from `WAITING` back to `RUNNING`, so it reports
/// normally rather than being suppressed as a duplicate.
#[test]
fn a_second_init_is_a_turn_boundary_and_not_a_duplicate() {
    let line = "{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[]}\n";
    let (out, _) = observe(&format!("{line}{line}"));
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|o| o.run_state() == Some("RUNNING")));
}

/// The echo `--replay-user-messages` exists for, and the reason the console's
/// prompt box never has to be optimistic (§9).
#[test]
fn a_replayed_user_message_is_the_prompt_acknowledgement() {
    let observation = only(
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"do the thing"}]},"isReplay":true}"#,
    );
    assert_eq!(
        observation,
        ClaudeObservation::PromptEchoed {
            text: "do the thing".to_owned()
        }
    );
    assert_eq!(observation.run_events(), &["PROMPT_STARTED"]);
}

/// The distinction `isReplay` carries. Reading a tool result as a prompt would
/// put tool output in the console as if the user had typed it.
#[test]
fn a_tool_result_is_not_read_as_something_the_user_said() {
    let observation = only(
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","is_error":false}]}}"#,
    );
    assert!(matches!(observation, ClaudeObservation::ToolFinished(_)));
    assert!(!matches!(
        observation,
        ClaudeObservation::PromptEchoed { .. }
    ));
}

/// One `assistant` line can carry several blocks, and each is its own
/// observation in the order it appeared.
#[test]
fn one_assistant_line_can_be_several_observations() {
    let (out, _) = observe(
        r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"","signature":"x"},{"type":"text","text":"working"},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls -la"}}]}}"#,
    );
    assert_eq!(out.len(), 3);
    assert_eq!(out[0], ClaudeObservation::Thought);
    assert_eq!(
        out[1],
        ClaudeObservation::Said {
            text: "working".to_owned()
        }
    );
    let ClaudeObservation::ToolStarted(call) = &out[2] else {
        panic!("expected a tool call, got {:?}", out[2]);
    };
    assert_eq!(call.name, "Bash");
    assert_eq!(
        call.intent,
        ClaudeToolIntent::RunCommand {
            command: "ls -la".to_owned()
        }
    );
    assert_eq!(out[2].run_events(), &["COMMAND_STARTED"]);
    assert_eq!(out[2].run_activity(), Some("EXECUTING"));
}

/// A `tool_result` is paired back to its call, so `COMMAND_FINISHED` can name
/// the command instead of leaving it empty for the ring to pair.
#[test]
fn a_tool_result_carries_its_call_forward() {
    let (out, _) = observe(concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"cargo test"}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","is_error":false}]}}"#,
        "\n",
    ));
    let ClaudeObservation::ToolFinished(done) = &out[1] else {
        panic!("expected a finished tool, got {:?}", out[1]);
    };
    assert!(done.ok);
    assert_eq!(
        done.call.as_ref().map(|call| &call.intent),
        Some(&ClaudeToolIntent::RunCommand {
            command: "cargo test".to_owned()
        })
    );
}

/// A result for a call nobody saw still reports. Losing it would be worse than
/// reporting it without its command.
#[test]
fn a_tool_result_with_no_matching_call_still_reports() {
    let observation = only(
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_ghost","is_error":true}]}}"#,
    );
    let ClaudeObservation::ToolFinished(done) = observation else {
        panic!("expected a finished tool");
    };
    assert_eq!(done.id, "toolu_ghost");
    assert!(done.call.is_none());
    assert!(!done.ok);
}

/// The table has a ceiling, so a child that opens calls and never closes them
/// cannot grow it. The oldest is evicted; its result then reports unpaired.
#[test]
fn the_outstanding_tool_table_is_bounded() {
    let mut stream = ClaudeStream::new();
    for index in 0..MAX_OUTSTANDING_TOOLS + 8 {
        let line = format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"toolu_{index}","name":"Bash","input":{{"command":"c{index}"}}}}]}}}}"#
        );
        stream.ingest(format!("{line}\n").as_bytes());
    }
    let evicted = stream.ingest(
        br#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_0","is_error":false}]}}
"#,
    );
    let ClaudeObservation::ToolFinished(done) = &evicted[0] else {
        panic!("expected a finished tool");
    };
    assert!(done.call.is_none(), "the oldest call should be evicted");

    let kept = stream.ingest(
        format!(
            "{}\n",
            format_args!(
                r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"toolu_{}","is_error":false}}]}}}}"#,
                MAX_OUTSTANDING_TOOLS + 7
            )
        )
        .as_bytes(),
    );
    let ClaudeObservation::ToolFinished(done) = &kept[0] else {
        panic!("expected a finished tool");
    };
    assert!(
        done.call.is_some(),
        "the newest call should still be paired"
    );
}

/// The classification is by the names Claude Code ships, and a name this crate
/// does not know is `Other` rather than an error — which is how every MCP tool
/// and every future built-in still reports work happening.
#[test]
fn tool_intents_follow_the_names_claude_code_ships() {
    let cases: &[(&str, &str, ClaudeToolIntent)] = &[
        (
            "Read",
            r#"{"file_path":"src/a.rs"}"#,
            ClaudeToolIntent::ReadFile {
                path: "src/a.rs".to_owned(),
            },
        ),
        (
            "Edit",
            r#"{"file_path":"src/a.rs"}"#,
            ClaudeToolIntent::WriteFile {
                path: "src/a.rs".to_owned(),
            },
        ),
        (
            "Write",
            r#"{"file_path":"src/a.rs"}"#,
            ClaudeToolIntent::WriteFile {
                path: "src/a.rs".to_owned(),
            },
        ),
        (
            "NotebookEdit",
            r#"{"notebook_path":"a.ipynb"}"#,
            ClaudeToolIntent::WriteFile {
                path: "a.ipynb".to_owned(),
            },
        ),
        (
            "Bash",
            r#"{"command":"ls"}"#,
            ClaudeToolIntent::RunCommand {
                command: "ls".to_owned(),
            },
        ),
        (
            "Grep",
            r#"{"pattern":"fn main"}"#,
            ClaudeToolIntent::Search {
                query: "fn main".to_owned(),
            },
        ),
        (
            "Glob",
            r#"{"pattern":"**/*.rs"}"#,
            ClaudeToolIntent::Search {
                query: "**/*.rs".to_owned(),
            },
        ),
        (
            "WebFetch",
            r#"{"url":"https://x"}"#,
            ClaudeToolIntent::Other,
        ),
        (
            "mcp__server__tool",
            r#"{"anything":1}"#,
            ClaudeToolIntent::Other,
        ),
        // Recognised name, input of the wrong shape: `Other`, never a guess.
        ("Read", r#"{"path":"src/a.rs"}"#, ClaudeToolIntent::Other),
        ("Bash", r#"{"command":42}"#, ClaudeToolIntent::Other),
    ];
    for (name, input, want) in cases {
        let observation = only(&format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"t","name":"{name}","input":{input}}}]}}}}"#
        ));
        let ClaudeObservation::ToolStarted(call) = observation else {
            panic!("{name}: expected a tool call");
        };
        assert_eq!(&call.intent, want, "{name} {input}");
    }
}

/// A path is passed through raw. Deciding whether it is inside the workspace is
/// the core's question (ADR-010), and an adapter that answered it would be
/// choosing what the renderer may open.
#[test]
fn paths_are_passed_through_untouched() {
    for raw in [
        "../../etc/passwd",
        "/etc/passwd",
        "~/.ssh/id_rsa",
        "src/../src/a.rs",
    ] {
        let observation = only(&format!(
            r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"t","name":"Read","input":{{"file_path":"{raw}"}}}}]}}}}"#
        ));
        let ClaudeObservation::ToolStarted(call) = observation else {
            panic!("expected a tool call");
        };
        assert_eq!(
            call.intent,
            ClaudeToolIntent::ReadFile {
                path: raw.to_owned()
            }
        );
    }
}

/// Rule 13's other half: nothing here clips or collapses a meaning, because
/// `AgentRunEvent::bounded` is the one seam that does, in the core.
#[test]
fn hostile_text_survives_the_adapter_untouched() {
    let hostile = "  <script>alert(1)</script>\t\tand   lots\n\rof   space  ";
    let line = serde_json::json!({
        "type": "user",
        "isReplay": true,
        "message": { "content": [{ "type": "text", "text": hostile }] },
    });
    let observation = only(&format!("{line}\n"));
    assert_eq!(
        observation,
        ClaudeObservation::PromptEchoed {
            text: hostile.to_owned()
        }
    );
}

// ---------------------------------------------------------------------------
// The permission channel
// ---------------------------------------------------------------------------

#[test]
fn a_can_use_tool_request_is_the_permission_ask() {
    let observation = only(
        r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"touch x"},"description":"touch x","blocked_path":"/w/x","tool_use_id":"toolu_1"}}"#,
    );
    assert_eq!(
        observation,
        ClaudeObservation::PermissionRequested(ClaudePermissionAsk {
            request_id: "req-1".to_owned(),
            tool: "Bash".to_owned(),
            intent: ClaudeToolIntent::RunCommand {
                command: "touch x".to_owned(),
            },
            input: None,
            detail: Some("touch x".to_owned()),
            blocked_path: Some("/w/x".to_owned()),
            tool_use_id: Some("toolu_1".to_owned()),
        })
    );
    assert_eq!(observation.run_state(), Some("BLOCKED"));
}

/// A control request of a subtype this crate does not answer must not be read
/// as a permission ask — a run blocked on something nobody can answer — and
/// must not be skipped either, because the CLI may be holding on it. It is
/// reported as a request to refuse (T310-ANSWER).
#[test]
fn a_control_request_of_another_subtype_is_one_to_refuse() {
    let observation =
        only(r#"{"type":"control_request","request_id":"r","request":{"subtype":"initialize"}}"#);
    assert_eq!(
        observation,
        ClaudeObservation::RequestUnsupported {
            request_id: "r".to_owned(),
            subtype: "initialize".to_owned(),
        }
    );
}

/// The child picks the subtype's length, and it is carried into an
/// observation, so it is bounded like an unmodelled `type`.
#[test]
fn an_unsupported_subtype_is_bounded() {
    let long = "x".repeat(MAX_KIND_BYTES * 4);
    let observation = only(&format!(
        r#"{{"type":"control_request","request_id":"r","request":{{"subtype":"{long}"}}}}"#
    ));
    let ClaudeObservation::RequestUnsupported { subtype, .. } = observation else {
        panic!("expected a request to refuse, got {observation:?}");
    };
    assert_eq!(subtype.len(), MAX_KIND_BYTES);
}

/// A request with no id cannot be answered whatever it asks, so it is never
/// raised as an ask or a refusal — there is nothing to quote back.
#[test]
fn a_control_request_without_an_id_is_unknown() {
    for body in [
        r#"{"type":"control_request","request":{"subtype":"can_use_tool","tool_name":"Bash"}}"#,
        r#"{"type":"control_request","request_id":"","request":{"subtype":"initialize"}}"#,
    ] {
        assert!(
            matches!(only(body), ClaudeObservation::Unknown { .. }),
            "{body}"
        );
    }
}

/// The ask says what the call is about from its input, the same way a
/// `tool_use` does, and carries a rendering of the input only when nothing
/// more specific can be said.
#[test]
fn an_ask_classifies_its_input_and_keeps_a_writes_content_out() {
    let write = only(
        r#"{"type":"control_request","request_id":"w","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"/w/.env","content":"SECRET=hunter2"}}}"#,
    );
    let ClaudeObservation::PermissionRequested(ask) = write else {
        panic!("expected an ask, got {write:?}");
    };
    assert_eq!(
        ask.intent,
        ClaudeToolIntent::WriteFile {
            path: "/w/.env".to_owned()
        }
    );
    assert_eq!(ask.input, None);
    assert!(
        !format!("{ask:?}").contains("hunter2"),
        "a write's content must not leave the adapter: {ask:?}"
    );

    let mcp = only(
        r#"{"type":"control_request","request_id":"m","request":{"subtype":"can_use_tool","tool_name":"mcp__db__query","input":{"sql":"drop table users"}}}"#,
    );
    let ClaudeObservation::PermissionRequested(ask) = mcp else {
        panic!("expected an ask, got {mcp:?}");
    };
    assert_eq!(ask.intent, ClaudeToolIntent::Other);
    assert_eq!(ask.input.as_deref(), Some(r#"{"sql":"drop table users"}"#));

    let bare = only(
        r#"{"type":"control_request","request_id":"b","request":{"subtype":"can_use_tool","tool_name":"mcp__x__y"}}"#,
    );
    let ClaudeObservation::PermissionRequested(ask) = bare else {
        panic!("expected an ask, got {bare:?}");
    };
    assert_eq!(ask.input, None, "no input is absent, not the string null");
}

/// The echo of an answer is what resumes the run, and it is only trusted for
/// an ask that was actually outstanding.
#[test]
fn an_answer_is_observed_from_the_echo_and_only_once() {
    let (out, _) = observe(concat!(
        r#"{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash"}}"#,
        "\n",
        r#"{"type":"control_response","response":{"subtype":"success","request_id":"req-1","response":{"behavior":"deny","message":"no"}}}"#,
        "\n",
        // The same echo again: nothing is outstanding now, so it must not
        // resume the run a second time.
        r#"{"type":"control_response","response":{"subtype":"success","request_id":"req-1","response":{"behavior":"deny"}}}"#,
        "\n",
    ));
    assert!(matches!(out[0], ClaudeObservation::PermissionRequested(_)));
    assert_eq!(
        out[1],
        ClaudeObservation::PermissionAnswered {
            request_id: "req-1".to_owned(),
            allowed: false,
        }
    );
    assert_eq!(
        out[2],
        ClaudeObservation::Unknown {
            kind: "control_response".to_owned()
        }
    );
}

/// The one direction it is unsafe to guess in. Anything that is not literally
/// `allow` reads as a denial, so a garbled or renamed verdict can never be
/// reported to the user as a tool they permitted.
#[test]
fn only_the_word_allow_reads_as_an_allowance() {
    for (behavior, expected) in [
        (r#""allow""#, true),
        (r#""deny""#, false),
        (r#""ALLOW""#, false),
        (r#""allow_once""#, false),
        ("null", false),
        ("42", false),
    ] {
        let (out, _) = observe(&format!(
            concat!(
                r#"{{"type":"control_request","request_id":"r","request":{{"subtype":"can_use_tool","tool_name":"Bash"}}}}"#,
                "\n",
                r#"{{"type":"control_response","response":{{"request_id":"r","response":{{"behavior":{behavior}}}}}}}"#,
                "\n"
            ),
            behavior = behavior
        ));
        assert_eq!(
            out[1],
            ClaudeObservation::PermissionAnswered {
                request_id: "r".to_owned(),
                allowed: expected,
            },
            "behavior {behavior}"
        );
    }
}

/// Asks are bounded too: an ask is a tool call that has not run, and a child
/// that raises them without ever being answered must not grow the queue.
#[test]
fn outstanding_asks_are_bounded() {
    let mut stream = ClaudeStream::new();
    for index in 0..MAX_OUTSTANDING_TOOLS + 4 {
        stream.ingest(
            format!(
                "{}\n",
                format_args!(
                    r#"{{"type":"control_request","request_id":"r{index}","request":{{"subtype":"can_use_tool","tool_name":"Bash"}}}}"#
                )
            )
            .as_bytes(),
        );
    }
    let out = stream.ingest(
        br#"{"type":"control_response","response":{"request_id":"r0","response":{"behavior":"deny"}}}
"#,
    );
    assert_eq!(
        out[0],
        ClaudeObservation::Unknown {
            kind: "control_response".to_owned()
        },
        "the oldest ask should have been evicted"
    );
}

// ---------------------------------------------------------------------------
// Turn endings
// ---------------------------------------------------------------------------

#[test]
fn a_successful_result_ends_the_turn_and_waits() {
    let observation = only(
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done","permission_denials":[{"tool_name":"Bash"}]}"#,
    );
    assert_eq!(
        observation,
        ClaudeObservation::TurnFinished(ClaudeTurnEnd {
            subtype: "success".to_owned(),
            ok: true,
            text: Some("done".to_owned()),
            denials: 1,
        })
    );
    assert_eq!(observation.run_events(), &["PROMPT_FINISHED"]);
    assert_eq!(observation.run_state(), Some("WAITING"));
    assert_eq!(observation.run_activity(), Some("WAITING"));
}

#[test]
fn a_failed_result_fails_the_run() {
    for subtype in ["error_max_turns", "error_during_execution"] {
        let observation = only(&format!(
            r#"{{"type":"result","subtype":"{subtype}","is_error":true}}"#
        ));
        assert_eq!(observation.run_state(), Some("FAILED"), "{subtype}");
    }
    // `is_error` and the subtype disagreeing is still a failure: the safe read
    // of "success but an error" is not success.
    let observation = only(r#"{"type":"result","subtype":"success","is_error":true}"#);
    assert_eq!(observation.run_state(), Some("FAILED"));
}

/// A turn boundary clears the pairing table, so one turn's unclosed calls
/// cannot be paired to the next turn's results.
#[test]
fn a_turn_ending_clears_what_was_outstanding() {
    let (out, _) = observe(concat!(
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls"}}]}}"#,
        "\n",
        r#"{"type":"result","subtype":"success"}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1"}]}}"#,
        "\n",
    ));
    let ClaudeObservation::ToolFinished(done) = &out[2] else {
        panic!("expected a finished tool, got {:?}", out[2]);
    };
    assert!(done.call.is_none());
}

/// The flag `ClaudeExit` reads when the child goes away.
#[test]
fn a_turn_is_in_flight_between_the_prompt_and_the_result() {
    let mut stream = ClaudeStream::new();
    assert!(!stream.turn_in_flight());
    stream.note_prompt_sent();
    assert!(stream.turn_in_flight());
    stream.ingest(b"{\"type\":\"result\",\"subtype\":\"success\"}\n");
    assert!(!stream.turn_in_flight());
    stream.ingest(
        br#"{"type":"user","message":{"content":[{"type":"text","text":"again"}]},"isReplay":true}
"#,
    );
    assert!(stream.turn_in_flight());
}
