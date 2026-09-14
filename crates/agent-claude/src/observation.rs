//! What the adapter saw, in the adapter's own words.
//!
//! This is **not** `gritty_core::AgentRunEvent`, and the gap is the point (hard
//! rule 4). The CLI says `control_request`/`can_use_tool`; the protocol says
//! `PERMISSION_REQUESTED` **and** `RUN_BLOCKED`. The CLI says `system`/`init`;
//! the protocol says nothing at all and moves a state out of `STARTING`. The
//! CLI says `thinking`; the protocol says nothing and sets an activity. An
//! adapter that emitted protocol events directly would be raw provider output
//! wearing domain state.
//!
//! [`ClaudeObservation::run_events`], [`ClaudeObservation::run_activity`] and
//! [`ClaudeObservation::run_state`] publish the intended translation as the
//! core's own wire spellings. They are a **specification, not an
//! implementation**: the implementation is `agent_run/claude_driver.rs`
//! (T310-B), written typed by matching on these variants, with a test asserting
//! the two agree — so a variant added on either side is a compile error or a
//! test failure and never a silently dropped event.
//!
//! # Nothing here is bounded or tidied
//!
//! Every string below came out of a child process and is untrusted (rule 13).
//! It is passed through exactly as it arrived, because `AgentRunEvent::bounded`
//! is the one ingest seam that clips and whitespace-collapses, applied once in
//! the core — "never in the adapter, which is the untrusted side". What this
//! crate *does* bound is resources rather than meanings: an oversized line is
//! dropped whole ([`crate::MAX_LINE_BYTES`]), the outstanding-tool table has a
//! ceiling ([`crate::MAX_OUTSTANDING_TOOLS`]), and only a tail of stderr is
//! kept ([`crate::MAX_STDERR_TAIL_BYTES`]).

/// One thing the adapter observed.
///
/// Ordered as the child emitted it. Several of these can come out of one line —
/// an `assistant` message carrying a thinking block and two `tool_use` blocks
/// is three observations — which is why the framing yields a list.
#[derive(Debug, Clone, PartialEq)]
pub enum ClaudeObservation {
    /// `system`/`subtype: init`. The session exists and is usable.
    ///
    /// **This is what ends `STARTING`** (task §3), not the spawn: phase 0 found
    /// that a terminal-backed agent is not usable the moment its process
    /// exists, and this is the first adapter that can honour it.
    ///
    /// Verified to arrive **once per turn**, not once per process, so a second
    /// one is an ordinary turn boundary rather than a duplicate to suppress:
    /// the run is going from `WAITING` back to `RUNNING`, which is exactly
    /// what [`Self::run_state`] says.
    Ready(ClaudeReady),
    /// A user turn the CLI echoed back (`--replay-user-messages`, `isReplay`).
    ///
    /// The acknowledgement the console's prompt box waits for: the user's own
    /// text appears when this arrives and not on the keystroke (task §9).
    PromptEchoed { text: String },
    /// An assistant `thinking` block. Activity only; the protocol has no
    /// variant for it and inventing one is another ticket's call.
    Thought,
    /// An assistant `text` block.
    ///
    /// Activity only as well. The reply the user reads is
    /// [`Self::TurnFinished`]'s `text`, which is the CLI's own final answer for
    /// the turn; emitting `PROMPT_FINISHED` per text block would announce the
    /// turn finished several times.
    Said { text: String },
    /// An assistant `tool_use` block: the agent wants to do something.
    ToolStarted(ClaudeToolUse),
    /// The matching `tool_result`, paired back to its call.
    ToolFinished(ClaudeToolDone),
    /// `control_request`/`can_use_tool`: the CLI is asking whether a tool may
    /// run, and **will not run it** until an answer goes back.
    PermissionRequested(ClaudePermissionAsk),
    /// The CLI acknowledged an answer, echoed back on stdout.
    ///
    /// Observed rather than assumed: the answer is written on stdin, and the
    /// CLI re-emits the `control_response` on stdout when it has taken it. A
    /// denial is **not** a failure — the agent is told no and carries on,
    /// exactly as it does in a terminal — so this resumes the run either way.
    PermissionAnswered { request_id: String, allowed: bool },
    /// A `control_request` of a subtype this crate does not answer.
    ///
    /// Not [`Self::Unknown`], because unlike a line nobody modelled it is a
    /// *question*: the CLI may be holding on it, so it has to be refused
    /// rather than skipped (T310-ANSWER: "a malformed `control_request` is
    /// logged and denied, never guessed at"). A request with no `request_id`
    /// cannot be addressed at all and stays `Unknown`.
    RequestUnsupported { request_id: String, subtype: String },
    /// `result`: the turn is over and the process is waiting for the next one.
    TurnFinished(ClaudeTurnEnd),
    /// The child is gone, or the pipe to it is.
    Exited(ClaudeExit),
    /// A line whose `type` this crate does not model, or a known line that did
    /// not parse.
    ///
    /// **Recorded and skipped, never an error** (task §2): the CLI is a moving
    /// target, and a new message kind must not fail a run. Real examples from
    /// the recorded fixture: `system`/`hook_started`, `system`/`hook_response`,
    /// `system`/`thinking_tokens`, `rate_limit_event`.
    Unknown { kind: String },
}

/// What `system`/`init` said about the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeReady {
    /// The CLI's own id for the session. Gritty minted it and passed it as
    /// `--session-id`, so this should equal what was asked for; it is reported
    /// rather than assumed so a mismatch is visible instead of silent.
    pub session_id: Option<String>,
    pub model: Option<String>,
    /// The directory the CLI resolved, as a raw string. Whether it is inside
    /// the workspace is the core's question, not this crate's.
    pub cwd: Option<String>,
    /// How many tools the session came up with. A count, not the list: the
    /// names are the CLI's inventory of this machine and nothing downstream
    /// has a use for them yet.
    pub tools: usize,
}

/// A tool call the agent issued.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeToolUse {
    /// `toolu_…`, which is what pairs a result back to its call.
    pub id: String,
    /// `Bash`, `Edit`, `Read`, `mcp__server__tool`, …
    pub name: String,
    pub intent: ClaudeToolIntent,
}

/// A tool call that came back.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudeToolDone {
    /// The call this closes, carried forward from the outstanding table so the
    /// core's `COMMAND_FINISHED` can name the command it finished rather than
    /// leaving it empty and relying on the ring to pair them.
    ///
    /// `None` when the call is not in the table: a result for a call that
    /// arrived before the adapter attached, or one evicted by
    /// [`crate::MAX_OUTSTANDING_TOOLS`].
    pub call: Option<ClaudeToolUse>,
    pub id: String,
    /// A `tool_result` with `is_error: true` — which is what a denied tool
    /// produces, and also what a command that failed produces.
    pub ok: bool,
}

/// What a tool call means to Gritty.
///
/// The classification is by the tool *names Claude Code ships*, which is
/// knowledge about a provider and therefore belongs on this side of the seam;
/// the core must never learn that a tool called `Edit` writes a file. An
/// unrecognised name is [`Self::Other`] and never an error: a new built-in, or
/// any MCP tool, still reports as work happening.
///
/// Paths and commands are raw strings on purpose. Whether a path resolves
/// inside the workspace is the core's question (ADR-010, `AgentRunPath`), and
/// answering it here would be an adapter deciding what the renderer may open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaudeToolIntent {
    ReadFile {
        path: String,
    },
    /// A write the agent *intends*. The protocol keeps wanting-to-write and
    /// having-written apart because T330's interception point is exactly there.
    WriteFile {
        path: String,
    },
    RunCommand {
        command: String,
    },
    Search {
        query: String,
    },
    /// Recognised as a tool call and nothing more specific.
    Other,
}

/// The ask the CLI will not proceed past.
///
/// # An unanswered ask holds, and that was not previously established
///
/// Task §7 recorded this as an inference rather than an observation: "What was
/// not tested is what happens if nobody answers." Tested, against `claude`
/// 2.1.263 on darwin 25.6 on 2026-09-08 — the ask was left outstanding and the
/// process was watched for 145 seconds. It emitted **nothing further**, stayed
/// alive, and the command never ran. There is no CLI-side timeout that resolves
/// an unanswered ask, and in particular none that resolves it to `allow`.
///
/// So the safe default is available: hold, and let the run sit in `BLOCKED`
/// with the ask visible, until the user answers. Never `allow` by default and
/// never on a timeout.
///
/// # What the ask carries of the call's input, and what it does not
///
/// The CLI sends the call's whole `input`. What crosses this type is what the
/// call is *about* — [`Self::intent`], classified from the input exactly as a
/// `tool_use` is — plus a one-line rendering of the input for a tool this crate
/// cannot classify, because for an MCP tool the arguments are the only honest
/// answer to "what is it asking to do". A `Write`'s file content never leaves
/// the adapter: it is not what the user is deciding on, and it is precisely
/// the kind of text that holds a secret (rule 13).
#[derive(Debug, Clone, PartialEq)]
pub struct ClaudePermissionAsk {
    /// What the answer has to quote back. Opaque; never parsed.
    pub request_id: String,
    /// Empty when the CLI named none, which makes the ask malformed: there is
    /// nothing to put in front of a person, so the driver denies it unasked.
    pub tool: String,
    /// What the call is about, from its input.
    pub intent: ClaudeToolIntent,
    /// The input as one compact line, only for [`ClaudeToolIntent::Other`].
    /// Raw and unbounded here; clipping it is the core's.
    pub input: Option<String>,
    /// The CLI's own one-line rendering of the call — for `Bash`, the command
    /// itself. Untrusted text, shown and never run.
    pub detail: Option<String>,
    /// The path the CLI says the call was stopped over, when it names one.
    pub blocked_path: Option<String>,
    /// The `tool_use` this ask belongs to.
    pub tool_use_id: Option<String>,
}

/// A `result` line: one turn is over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeTurnEnd {
    /// `success`, `error_max_turns`, `error_during_execution`, …
    pub subtype: String,
    pub ok: bool,
    /// The CLI's final answer for the turn.
    pub text: Option<String>,
    /// How many asks were refused during it. Counted, not carried: the asks
    /// themselves already arrived as [`ClaudeObservation::PermissionRequested`]
    /// and were recorded then.
    pub denials: usize,
}

/// How the child ended, or stopped being reachable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeExit {
    pub status: ClaudeExitStatus,
    /// Whether the stream ended with work outstanding: a prompt had no
    /// `result`, or `system`/`init` never arrived at all.
    ///
    /// The task's failure table treats "stdout closes mid-turn" as a failure
    /// even when the exit code is clean, and a child that exits 0 without ever
    /// announcing a session is the same shape of wrong — it looked like it
    /// worked and nothing happened.
    pub unfinished: bool,
    /// The last [`crate::MAX_STDERR_TAIL_BYTES`] of stderr, lossily decoded.
    /// Untrusted, and bounded here only because a tail is a resource decision;
    /// clipping it for display is still the core's.
    pub stderr_tail: String,
}

/// The operating system's answer, or the absence of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeExitStatus {
    /// Exited 0.
    Ok,
    /// Exited with a non-zero code.
    Code(i32),
    /// Killed by a signal. Somebody or something stopped it; the adapter does
    /// not know who, and a cancellation is the manager's to report either way.
    Signal(i32),
    /// The pipe went away while the process was still alive, so there is no
    /// status at all.
    Lost,
}

impl ClaudeObservation {
    /// The discriminator used for tracing. The kind and never the text.
    pub fn kind(&self) -> &str {
        match self {
            Self::Ready(_) => "ready",
            Self::PromptEchoed { .. } => "promptEchoed",
            Self::Thought => "thought",
            Self::Said { .. } => "said",
            Self::ToolStarted(_) => "toolStarted",
            Self::ToolFinished(_) => "toolFinished",
            Self::PermissionRequested(_) => "permissionRequested",
            Self::PermissionAnswered { .. } => "permissionAnswered",
            Self::RequestUnsupported { .. } => "requestUnsupported",
            Self::TurnFinished(_) => "turnFinished",
            Self::Exited(_) => "exited",
            Self::Unknown { .. } => "unknown",
        }
    }

    /// The `AgentRunEvent` variants this observation should become, in order,
    /// spelled as `AgentRunEvent::as_str` spells them.
    ///
    /// **A specification, not an implementation**, for the same reason
    /// `FakeAction::run_events` is one: this crate cannot name the type without
    /// a dependency cycle. Publishing it as strings is what lets T310-B's
    /// translation be checked against it by a test rather than by a reviewer's
    /// memory.
    ///
    /// Two protocol variants appear nowhere below, on purpose. `RUN_STARTED` is
    /// the manager's, emitted when the run is created rather than reported by
    /// an adapter. And nothing at all corresponds to a cancellation, which the
    /// protocol says with the run's state.
    pub fn run_events(&self) -> &'static [&'static str] {
        match self {
            // A state change and an activity; no event. The run leaving
            // `STARTING` is visible in the run itself.
            Self::Ready(_) => &[],
            Self::PromptEchoed { .. } => &["PROMPT_STARTED"],
            Self::ToolStarted(call) => match call.intent {
                ClaudeToolIntent::ReadFile { .. } => &["FILE_READ"],
                ClaudeToolIntent::WriteFile { .. } => &["FILE_WRITE_REQUESTED"],
                ClaudeToolIntent::RunCommand { .. } => &["COMMAND_STARTED"],
                ClaudeToolIntent::Search { .. } | ClaudeToolIntent::Other => &[],
            },
            Self::ToolFinished(done) => match done.call.as_ref().map(|call| &call.intent) {
                // A write is only reported as *done* once its result came
                // back: the request and the fact are separate events because
                // T330 intercepts between them, and a denied write must never
                // produce `FILE_WRITTEN`.
                Some(ClaudeToolIntent::WriteFile { .. }) if done.ok => &["FILE_WRITTEN"],
                Some(ClaudeToolIntent::RunCommand { .. }) => &["COMMAND_FINISHED"],
                _ => &[],
            },
            // The ask, then the fact that the run stopped on it. `RUN_BLOCKED`
            // carries the reason.
            Self::PermissionRequested(_) => &["PERMISSION_REQUESTED", "RUN_BLOCKED"],
            Self::PermissionAnswered { .. } => &["RUN_RESUMED"],
            Self::TurnFinished(_) => &["PROMPT_FINISHED"],
            Self::Exited(exit) => match exit.run_state() {
                RUN_FINISHED_STATE => &["RUN_FINISHED"],
                RUN_FAILED_STATE => &["RUN_FAILED"],
                // `INTERRUPTED` is reported by the run's state, like a
                // cancellation, and has no event of its own.
                _ => &[],
            },
            // Activity only, or nothing at all. A refused request is answered
            // on the pipe and is not something the run did.
            Self::Thought
            | Self::Said { .. }
            | Self::RequestUnsupported { .. }
            | Self::Unknown { .. } => &[],
        }
    }

    /// The `AgentRunActivityKind` this observation puts the run in, spelled as
    /// `AgentRunActivityKind::as_str` spells it, or `None` when the run has no
    /// particular activity — which is every ending.
    ///
    /// A suggestion in the same sense as [`Self::run_events`]: the core
    /// decides, and the paths and commands an activity carries come from this
    /// observation's own payload.
    pub fn run_activity(&self) -> Option<&'static str> {
        match self {
            Self::Ready(_) | Self::PromptEchoed { .. } | Self::Thought | Self::Said { .. } => {
                Some("THINKING")
            }
            Self::ToolStarted(call) => Some(match call.intent {
                ClaudeToolIntent::ReadFile { .. } => "READING",
                ClaudeToolIntent::WriteFile { .. } => "EDITING",
                ClaudeToolIntent::RunCommand { .. } => "EXECUTING",
                ClaudeToolIntent::Search { .. } => "SEARCHING",
                ClaudeToolIntent::Other => "THINKING",
            }),
            Self::ToolFinished(_) | Self::PermissionAnswered { .. } => Some("THINKING"),
            Self::PermissionRequested(_) => Some("REQUESTING_PERMISSION"),
            // The turn is over and nothing is outstanding: the run has nothing
            // to do until someone speaks, which is what `WAITING` means.
            Self::TurnFinished(_) => Some("WAITING"),
            Self::Exited(_) => None,
            // A line nobody modelled says nothing about what the run is doing,
            // and neither does a request the adapter refused.
            Self::Unknown { .. } | Self::RequestUnsupported { .. } => None,
        }
    }

    /// The `AgentRunState` this observation puts the run in, spelled as
    /// `AgentRunState`'s serde form spells it, or `None` for "unchanged".
    ///
    /// `CANCELLED` never appears: cancellation is the manager's ending and
    /// [`crate::ClaudeProcess`] is told to stop rather than deciding to.
    pub fn run_state(&self) -> Option<&'static str> {
        match self {
            Self::Ready(_) | Self::PromptEchoed { .. } | Self::PermissionAnswered { .. } => {
                Some("RUNNING")
            }
            Self::PermissionRequested(_) => Some("BLOCKED"),
            Self::TurnFinished(end) if end.ok => Some("WAITING"),
            // `error_max_turns` and `error_during_execution` end the turn
            // badly. The process may well still be alive, but a turn that
            // failed is a run that failed: nothing is going to move it on.
            Self::TurnFinished(_) => Some("FAILED"),
            Self::Exited(exit) => Some(exit.run_state()),
            Self::Thought
            | Self::Said { .. }
            | Self::ToolStarted(_)
            | Self::ToolFinished(_)
            | Self::RequestUnsupported { .. }
            | Self::Unknown { .. } => None,
        }
    }
}

const RUN_FINISHED_STATE: &str = "FINISHED";
const RUN_FAILED_STATE: &str = "FAILED";
const RUN_INTERRUPTED_STATE: &str = "INTERRUPTED";

impl ClaudeExit {
    /// The `AgentRunState` this ending means.
    ///
    /// Straight from the task's failure table, and the three answers are
    /// deliberately different: "the agent broke", "we stopped being able to
    /// watch it" and "someone asked it to stop" are what a user deciding
    /// whether to retry needs told apart.
    pub const fn run_state(&self) -> &'static str {
        match self.status {
            // Killed, or the pipe went away while it lived. Nobody said the
            // run was wrong; Gritty stopped being able to supervise it.
            ClaudeExitStatus::Signal(_) | ClaudeExitStatus::Lost => RUN_INTERRUPTED_STATE,
            // Clean exit with work outstanding is stdout closing mid-turn,
            // which the failure table calls a failure.
            ClaudeExitStatus::Ok if self.unfinished => RUN_FAILED_STATE,
            ClaudeExitStatus::Ok => RUN_FINISHED_STATE,
            ClaudeExitStatus::Code(_) => RUN_FAILED_STATE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(intent: ClaudeToolIntent) -> ClaudeToolUse {
        ClaudeToolUse {
            id: "toolu_1".to_owned(),
            name: "T".to_owned(),
            intent,
        }
    }

    fn exit(status: ClaudeExitStatus, unfinished: bool) -> ClaudeExit {
        ClaudeExit {
            status,
            unfinished,
            stderr_tail: String::new(),
        }
    }

    /// Every observation the crate can produce, so the vocabulary tests below
    /// fail when a variant is added without deciding what it means.
    fn every_observation() -> Vec<ClaudeObservation> {
        vec![
            ClaudeObservation::Ready(ClaudeReady {
                session_id: None,
                model: None,
                cwd: None,
                tools: 0,
            }),
            ClaudeObservation::PromptEchoed {
                text: "hi".to_owned(),
            },
            ClaudeObservation::Thought,
            ClaudeObservation::Said {
                text: "hi".to_owned(),
            },
            ClaudeObservation::ToolStarted(tool(ClaudeToolIntent::ReadFile {
                path: "a".to_owned(),
            })),
            ClaudeObservation::ToolStarted(tool(ClaudeToolIntent::WriteFile {
                path: "a".to_owned(),
            })),
            ClaudeObservation::ToolStarted(tool(ClaudeToolIntent::RunCommand {
                command: "ls".to_owned(),
            })),
            ClaudeObservation::ToolStarted(tool(ClaudeToolIntent::Search {
                query: "q".to_owned(),
            })),
            ClaudeObservation::ToolStarted(tool(ClaudeToolIntent::Other)),
            ClaudeObservation::ToolFinished(ClaudeToolDone {
                call: Some(tool(ClaudeToolIntent::WriteFile {
                    path: "a".to_owned(),
                })),
                id: "toolu_1".to_owned(),
                ok: true,
            }),
            ClaudeObservation::ToolFinished(ClaudeToolDone {
                call: Some(tool(ClaudeToolIntent::RunCommand {
                    command: "ls".to_owned(),
                })),
                id: "toolu_1".to_owned(),
                ok: false,
            }),
            ClaudeObservation::ToolFinished(ClaudeToolDone {
                call: None,
                id: "toolu_x".to_owned(),
                ok: true,
            }),
            ClaudeObservation::PermissionRequested(ClaudePermissionAsk {
                request_id: "r".to_owned(),
                tool: "Bash".to_owned(),
                intent: ClaudeToolIntent::RunCommand {
                    command: "ls".to_owned(),
                },
                input: None,
                detail: None,
                blocked_path: None,
                tool_use_id: None,
            }),
            ClaudeObservation::PermissionAnswered {
                request_id: "r".to_owned(),
                allowed: false,
            },
            ClaudeObservation::RequestUnsupported {
                request_id: "r".to_owned(),
                subtype: "mcp_message".to_owned(),
            },
            ClaudeObservation::TurnFinished(ClaudeTurnEnd {
                subtype: "success".to_owned(),
                ok: true,
                text: None,
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
            ClaudeObservation::Exited(exit(ClaudeExitStatus::Code(1), false)),
            ClaudeObservation::Exited(exit(ClaudeExitStatus::Signal(9), false)),
            ClaudeObservation::Exited(exit(ClaudeExitStatus::Lost, true)),
            ClaudeObservation::Unknown {
                kind: "rate_limit_event".to_owned(),
            },
        ]
    }

    /// The mapping this crate publishes has to be spelled in the core's
    /// vocabulary, or T310-B's translation cannot bind to it. Copied from
    /// `AgentRunEvent::as_str`, `AgentRunActivityKind::as_str` and
    /// `AgentRunState`'s serde form; the cross-crate agreement test lives in
    /// `agent_run`, which can name both sides.
    #[test]
    fn the_published_mapping_uses_only_protocol_spellings() {
        const EVENTS: &[&str] = &[
            "RUN_STARTED",
            "RUN_FINISHED",
            "PROMPT_STARTED",
            "PROMPT_FINISHED",
            "FILE_READ",
            "FILE_WRITE_REQUESTED",
            "FILE_WRITTEN",
            "COMMAND_STARTED",
            "COMMAND_FINISHED",
            "PERMISSION_REQUESTED",
            "DECISION_REQUESTED",
            "RUN_BLOCKED",
            "RUN_RESUMED",
            "RUN_FAILED",
        ];
        const ACTIVITIES: &[&str] = &[
            "THINKING",
            "READING",
            "EDITING",
            "SEARCHING",
            "EXECUTING",
            "TESTING",
            "WAITING",
            "REQUESTING_PERMISSION",
            "REQUESTING_DECISION",
        ];
        const STATES: &[&str] = &[
            "STARTING",
            "RUNNING",
            "WAITING",
            "NEEDS_DECISION",
            "BLOCKED",
            "REVIEW",
            "FINISHED",
            "FAILED",
            "CANCELLED",
            "INTERRUPTED",
        ];

        for observation in every_observation() {
            for name in observation.run_events() {
                assert!(
                    EVENTS.contains(name),
                    "{name} is not an AgentRunEvent variant ({})",
                    observation.kind()
                );
            }
            if let Some(activity) = observation.run_activity() {
                assert!(
                    ACTIVITIES.contains(&activity),
                    "{activity} is not an AgentRunActivityKind ({})",
                    observation.kind()
                );
            }
            if let Some(state) = observation.run_state() {
                assert!(
                    STATES.contains(&state),
                    "{state} is not an AgentRunState ({})",
                    observation.kind()
                );
            }
        }
    }

    /// `RUN_STARTED` is the manager's and a cancellation is a fact about the
    /// run's state. An adapter that claimed either would be reporting
    /// something it did not observe.
    #[test]
    fn the_adapter_never_claims_an_ending_that_is_not_its_own() {
        for observation in every_observation() {
            assert!(
                !observation.run_events().contains(&"RUN_STARTED"),
                "{} claimed RUN_STARTED",
                observation.kind()
            );
            assert_ne!(
                observation.run_state(),
                Some("CANCELLED"),
                "{} claimed CANCELLED",
                observation.kind()
            );
        }
    }

    /// The gate is the whole reason `--permission-prompt-tool stdio` is passed:
    /// an ask has to be *visible* and the state has to be *honest*, or the run
    /// looks stuck for a reason nobody can see.
    #[test]
    fn a_permission_ask_blocks_the_run_and_announces_itself() {
        let ask = ClaudeObservation::PermissionRequested(ClaudePermissionAsk {
            request_id: "r".to_owned(),
            tool: "Bash".to_owned(),
            intent: ClaudeToolIntent::RunCommand {
                command: "touch x".to_owned(),
            },
            input: None,
            detail: Some("touch x".to_owned()),
            blocked_path: None,
            tool_use_id: None,
        });
        assert_eq!(ask.run_events(), &["PERMISSION_REQUESTED", "RUN_BLOCKED"]);
        assert_eq!(ask.run_state(), Some("BLOCKED"));
        assert_eq!(ask.run_activity(), Some("REQUESTING_PERMISSION"));
    }

    /// "A denial is not a failure. The run continues; the agent is told no and
    /// carries on, exactly as it does in a terminal." (§7)
    #[test]
    fn a_denial_resumes_the_run_rather_than_ending_it() {
        let answered = ClaudeObservation::PermissionAnswered {
            request_id: "r".to_owned(),
            allowed: false,
        };
        assert_eq!(answered.run_events(), &["RUN_RESUMED"]);
        assert_eq!(answered.run_state(), Some("RUNNING"));
    }

    /// A write that was refused must never be reported as a write that
    /// happened. This is the one place the `ok` flag on a tool result changes
    /// which event is produced.
    #[test]
    fn a_failed_write_does_not_become_file_written() {
        let write = tool(ClaudeToolIntent::WriteFile {
            path: "src/a.rs".to_owned(),
        });
        let done = |ok| {
            ClaudeObservation::ToolFinished(ClaudeToolDone {
                call: Some(write.clone()),
                id: write.id.clone(),
                ok,
            })
        };
        assert_eq!(done(true).run_events(), &["FILE_WRITTEN"]);
        assert_eq!(done(false).run_events(), &[] as &[&str]);
    }

    /// A command reports finishing whether it worked or not: `ok` is a field
    /// on `COMMAND_FINISHED`, not a reason to withhold it.
    #[test]
    fn a_failed_command_still_reports_finishing() {
        for ok in [true, false] {
            let done = ClaudeObservation::ToolFinished(ClaudeToolDone {
                call: Some(tool(ClaudeToolIntent::RunCommand {
                    command: "ls".to_owned(),
                })),
                id: "toolu_1".to_owned(),
                ok,
            });
            assert_eq!(done.run_events(), &["COMMAND_FINISHED"]);
        }
    }

    /// The task's failure table, asserted rather than described.
    #[test]
    fn every_ending_maps_the_way_the_failure_table_says() {
        assert_eq!(exit(ClaudeExitStatus::Ok, false).run_state(), "FINISHED");
        assert_eq!(exit(ClaudeExitStatus::Code(1), false).run_state(), "FAILED");
        assert_eq!(exit(ClaudeExitStatus::Code(2), true).run_state(), "FAILED");
        // stdout closed mid-turn, clean exit code: still a failure.
        assert_eq!(exit(ClaudeExitStatus::Ok, true).run_state(), "FAILED");
        // Killed, and the pipe lost while it lived: not the agent's fault.
        assert_eq!(
            exit(ClaudeExitStatus::Signal(15), true).run_state(),
            "INTERRUPTED"
        );
        assert_eq!(
            exit(ClaudeExitStatus::Lost, false).run_state(),
            "INTERRUPTED"
        );
    }

    /// An unmodelled line changes nothing at all. It is recorded so the count
    /// is visible, and it must not move a state or an activity.
    #[test]
    fn an_unknown_line_changes_nothing() {
        let unknown = ClaudeObservation::Unknown {
            kind: "rate_limit_event".to_owned(),
        };
        assert_eq!(unknown.run_events(), &[] as &[&str]);
        assert_eq!(unknown.run_activity(), None);
        assert_eq!(unknown.run_state(), None);
    }

    /// A refused request is an answer on the pipe, not something the run did:
    /// no event, no activity, no state.
    #[test]
    fn an_unsupported_request_changes_nothing_about_the_run() {
        let refused = ClaudeObservation::RequestUnsupported {
            request_id: "r".to_owned(),
            subtype: "mcp_message".to_owned(),
        };
        assert_eq!(refused.run_events(), &[] as &[&str]);
        assert_eq!(refused.run_activity(), None);
        assert_eq!(refused.run_state(), None);
    }
}
