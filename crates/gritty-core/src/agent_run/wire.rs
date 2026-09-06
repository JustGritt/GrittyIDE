//! Wire types of the `runs.*` domain (T300, Gate D), exported to TypeScript by
//! `cargo test -p gritty-core`.
//!
//! These describe a run Gritty **owns**: it started it, it can cancel it, and
//! it is answerable for what happens to it. That is the whole difference from
//! [`crate::agent_session`], whose types describe an agent some *other*
//! terminal started and which Gritty can only watch. Nothing here may be
//! confused for one of those, and nothing here imports one.
//!
//! Every string that reaches these types came out of an adapter, so it is
//! untrusted input (rule 13) — the fake included, since it exists to prove the
//! boundary works rather than to skip it. The core bounds and
//! whitespace-collapses each one *before* it crosses the protocol, via
//! [`AgentRunEvent::bounded`] and [`AgentRunActivity::bounded`]; the renderer
//! shows them as text and never as markup, an openable path, or a runnable
//! command.

use gritty_workspace::WorkspacePath;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::error::CoreError;
use crate::protocol::ids::{AgentRunId, CorrelationId, WorkspaceId, WorkstreamId};
use crate::protocol::time::Timestamp;

pub use super::state::AgentRunState;

// ---------------------------------------------------------------------------
// Bounds
//
// Named, never literal, and each one tested — the discipline (not the numbers)
// of `crates/agents/src/transcript.rs`. Every limit below counts characters,
// not bytes, so a line of emoji is bounded the way a line of ASCII is.
// ---------------------------------------------------------------------------

/// Longest prompt or reply preview kept. Long enough to recognise the request,
/// far short of the transcript the agent keeps for itself.
pub const MAX_TEXT: usize = 2_000;
/// Longest question a run may raise while asking for a decision.
pub const MAX_QUESTION: usize = 1_000;
/// Longest command line kept. Shown, never run.
pub const MAX_COMMAND: usize = 512;
/// Longest display form of a path. Matches the intent of `transcript.rs`'s
/// `MAX_TRACKING_PATH`: a path that does not fit is clipped, never dropped.
pub const MAX_DISPLAY_PATH: usize = 240;
/// Longest tool name (`Bash`, `Edit`, …). A name longer than this is a bug or
/// an attack, not a tool.
pub const MAX_TOOL: usize = 64;
/// Longest one-line detail beside a tool or a permission ask.
pub const MAX_DETAIL: usize = 300;
/// Longest explanation of why a run is blocked or how it ended.
pub const MAX_REASON: usize = 300;

/// How many events one run's ring keeps before it evicts from the oldest end.
///
/// The bound is stated here, with the wire types, because it is a promise the
/// protocol makes: [`AgentRunEventLog::dropped`] counts against exactly this
/// number, and a consumer that sees it non-zero knows a gap is real rather than
/// a delivery failure. Enforcing it is the run manager's job (T300-CORE), the
/// way `MAX_OPEN_TIMELINES` is enforced by the timeline follower and not by
/// this module.
pub const MAX_RUN_EVENTS: usize = 512;

/// How much of an oversized field is looked at before it is clipped.
///
/// Collapsing whitespace is a streaming transform, so the first `limit`
/// characters of the collapse of a prefix are the first `limit` characters of
/// the collapse of the whole. A megabyte of agent output is therefore bounded
/// without touching a megabyte. Any value comfortably above the largest limit
/// above gives the same answer.
const MAX_CLIP_SOURCE_CHARS: usize = 8 * 1024;

/// Collapse whitespace and clip to `max` characters, marking the clip.
///
/// Both halves matter and both belong in the core. The result crosses the
/// protocol boundary into a one-line row, so an oversized or newline-bearing
/// field is bounded *here* — never in the adapter, which is the untrusted side,
/// and never in the renderer, which would be trusting it.
pub fn clip(text: &str, max: usize) -> String {
    let source = match text.char_indices().nth(MAX_CLIP_SOURCE_CHARS) {
        Some((end, _)) => &text[..end],
        None => text,
    };
    let collapsed = source.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        return collapsed;
    }
    let kept: String = collapsed.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// [`clip`] for an optional field: a value that collapses to nothing is absent
/// rather than empty, so the UI has one case to handle instead of two.
pub fn clip_optional(text: Option<&str>, max: usize) -> Option<String> {
    let clipped = clip(text?, max);
    (!clipped.is_empty()).then_some(clipped)
}

// ---------------------------------------------------------------------------
// The domain model (plan §§5-7)
// ---------------------------------------------------------------------------

/// Which kind of agent a run drives.
///
/// A **new** enum rather than [`crate::agent_session::wire::AgentProvider`]
/// reused, and deliberately so: the observed roster's provider list answers
/// "which agent CLIs might already be running on this machine", and reusing it
/// here would silently make every one of them a thing Gritty offers to start.
/// Those are different questions with different answers, and the two lanes stay
/// disjoint exactly because the types do.
///
/// One variant today. T310 adds the second, and until it does, a run cannot
/// touch a real process by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentRunProvider {
    /// The deterministic fake (T300-FAKE): no process, no network, no
    /// filesystem. Its events are scripted fiction, which is what makes it safe
    /// to run anywhere, including CI.
    Fake,
}

impl AgentRunProvider {
    pub const ALL: &'static [Self] = &[Self::Fake];

    /// The stored and transmitted spelling, which is the serde one.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fake => "FAKE",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        Self::ALL
            .iter()
            .copied()
            .find(|provider| provider.as_str() == raw)
            .ok_or_else(|| CoreError::internal(format!("unknown agent run provider {raw:?}")))
    }
}

/// A file or directory a run touched, under the `FileTouched` rule (ADR-010).
///
/// One type rather than a loose pair repeated in five places, because the rule
/// is the point and it must not be re-derived per call site:
///
/// - `path` is present **only** when the target is inside the run's workspace.
///   Its presence is precisely what the renderer may open, and its absence is
///   what stops anything outside the root being opened from the WebView.
/// - `display` is always safe to *show* and never to resolve. It is bounded and
///   whitespace-collapsed here, like every other string in this module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunPath {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<WorkspacePath>,
    pub display: String,
}

impl AgentRunPath {
    /// A target inside the workspace: openable, and displayed as itself.
    pub fn inside(path: WorkspacePath) -> Self {
        let display = clip(path.as_str(), MAX_DISPLAY_PATH);
        Self {
            path: Some(path),
            display,
        }
    }

    /// A target outside the workspace: shown, never opened. The caller has
    /// already shortened it for display (`~/…`); nothing here resolves it.
    pub fn outside(display: impl AsRef<str>) -> Self {
        Self {
            path: None,
            display: clip(display.as_ref(), MAX_DISPLAY_PATH),
        }
    }

    /// The same target with `display` bounded. Idempotent, so applying it to an
    /// already-bounded value is safe.
    #[must_use]
    pub fn bounded(self) -> Self {
        Self {
            display: clip(&self.display, MAX_DISPLAY_PATH),
            path: self.path,
        }
    }
}

/// What a run is doing *right now* (plan §7).
///
/// Separate from [`AgentRunState`] on purpose: the state holds still at
/// `RUNNING` while this cycles. A UI reads "RUNNING / Editing
/// src/auth/session.ts" from the two together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentRunActivityKind {
    Thinking,
    Reading,
    Editing,
    Searching,
    Executing,
    Testing,
    /// Nothing outstanding; the run has nothing to do until someone speaks.
    Waiting,
    /// Stopped at a permission prompt. The state beside it is `BLOCKED`.
    RequestingPermission,
    /// Stopped on a question. The state beside it is `NEEDS_DECISION`.
    RequestingDecision,
}

impl AgentRunActivityKind {
    pub const ALL: &'static [Self] = &[
        Self::Thinking,
        Self::Reading,
        Self::Editing,
        Self::Searching,
        Self::Executing,
        Self::Testing,
        Self::Waiting,
        Self::RequestingPermission,
        Self::RequestingDecision,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Thinking => "THINKING",
            Self::Reading => "READING",
            Self::Editing => "EDITING",
            Self::Searching => "SEARCHING",
            Self::Executing => "EXECUTING",
            Self::Testing => "TESTING",
            Self::Waiting => "WAITING",
            Self::RequestingPermission => "REQUESTING_PERMISSION",
            Self::RequestingDecision => "REQUESTING_DECISION",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        Self::ALL
            .iter()
            .copied()
            .find(|kind| kind.as_str() == raw)
            .ok_or_else(|| CoreError::internal(format!("unknown agent run activity {raw:?}")))
    }
}

/// One activity record: the kind, and at most one thing it is about.
///
/// There is **no `metadata` bag**, unlike the coordination plan's
/// `AgentOperation`. An untyped map from an adapter is raw provider output, and
/// hard rule 4 forbids treating that as domain state: whatever a provider wants
/// to say has to be translated into a field with a meaning, or it is not said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunActivity {
    pub kind: AgentRunActivityKind,
    /// The file being read, edited or searched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<AgentRunPath>,
    /// The command being executed. Display only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub command: Option<String>,
    /// When this activity began — not when the run did.
    pub started_at: Timestamp,
}

impl AgentRunActivity {
    pub fn new(kind: AgentRunActivityKind, started_at: Timestamp) -> Self {
        Self {
            kind,
            path: None,
            command: None,
            started_at,
        }
    }

    #[must_use]
    pub fn with_path(mut self, path: AgentRunPath) -> Self {
        self.path = Some(path.bounded());
        self
    }

    #[must_use]
    pub fn with_command(mut self, command: impl AsRef<str>) -> Self {
        self.command = clip_optional(Some(command.as_ref()), MAX_COMMAND);
        self
    }

    /// Every string bounded. The one call the run manager makes on an activity
    /// an adapter produced, before it reaches [`AgentRun`].
    #[must_use]
    pub fn bounded(self) -> Self {
        Self {
            kind: self.kind,
            path: self.path.map(AgentRunPath::bounded),
            command: clip_optional(self.command.as_deref(), MAX_COMMAND),
            started_at: self.started_at,
        }
    }
}

/// One run Gritty owns.
///
/// Deliberately [`crate::operation::Operation`]'s shape, reused: an id, who it
/// belongs to, a state, timestamps, a cancellation flag, a correlation id and
/// an optional error. A run is not squeezed *into* the operation registry — a
/// ten-state machine and an event stream do not fit `OperationStatus`'s five
/// terminal states — but the fields a supervised thing needs are the same ones,
/// and answering "what is it, can I cancel it, how did it end" the same way
/// everywhere is worth more than a bespoke shape.
///
/// A run's relation to its workstream is [`Self::workstream_id`] and nothing
/// else. There is no `currentRunId` column on `Workstream`: hard rule 6 keeps
/// provider concepts out of that type, and "which run is on this workstream" is
/// a query, not a field that can go stale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRun {
    pub id: AgentRunId,
    /// The workstream this run is working on. It brings no task model of its
    /// own (`docs/architecture/workstreams.md`): the workstream is the work.
    pub workstream_id: WorkstreamId,
    /// The workspace, as it is open right now.
    pub workspace_id: WorkspaceId,
    pub provider: AgentRunProvider,
    pub state: AgentRunState,
    /// What it is doing. Absent when it is not doing anything in particular —
    /// which is most of `STARTING` and all of every terminal state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub activity: Option<AgentRunActivity>,
    pub started_at: Timestamp,
    /// Last state or activity change. What a UI sorts by.
    pub updated_at: Timestamp,
    /// Absent until the run reaches a terminal state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub finished_at: Option<Timestamp>,
    /// A cancel was asked for. Cooperative, like the operation model's: the
    /// flag is set at once and the state follows when the run notices.
    pub cancellation_requested: bool,
    /// Ties the run, its events and its errors to the command that started it.
    pub correlation_id: CorrelationId,
    /// How it failed. Only ever set alongside `FAILED`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<CoreError>,
}

// ---------------------------------------------------------------------------
// The normalized event protocol (plan §8)
// ---------------------------------------------------------------------------

/// What a run reported, normalized.
///
/// The plan's vocabulary, renamed into the run's own: `SessionStarted`,
/// `SessionStopped`, `AgentBlocked`, `AgentResumed` and `AgentFailed` are
/// `RUN_STARTED`, `RUN_FINISHED`, `RUN_BLOCKED`, `RUN_RESUMED` and `RUN_FAILED`
/// here. The word "session" is spoken for by
/// [`crate::agent_session`] — an agent Gritty did *not* start — and a protocol
/// in which `SessionStarted` sometimes means "Gritty started it" and sometimes
/// means "something else did" is the confusion this whole lane split exists to
/// prevent.
///
/// Tagged the way [`crate::protocol::actor::Actor`] is, so a client that does
/// not know a variant can still show it as text instead of failing to parse the
/// batch it arrived in.
///
/// An adapter never constructs one of these directly from provider bytes: it
/// emits its own step vocabulary and the core translates (hard rule 4). That
/// translation seam is the thing T310's real adapter needs, which is why even
/// the fake goes through it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentRunEvent {
    /// The run is under way. Emitted once, first.
    RunStarted,
    /// The run is over, having done what it was asked. Emitted once, last —
    /// [`Self::RunFailed`] is the other ending, and cancellation is reported by
    /// the run's state rather than by a third variant.
    RunFinished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        summary: Option<String>,
    },
    /// The user asked for something.
    PromptStarted {
        text: String,
    },
    /// The run answered.
    PromptFinished {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        text: Option<String>,
    },
    FileRead {
        path: AgentRunPath,
    },
    /// A write the run intends. Its own event because the interception point
    /// T330 needs is exactly here — between wanting to write and having
    /// written.
    FileWriteRequested {
        path: AgentRunPath,
    },
    FileWritten {
        path: AgentRunPath,
    },
    CommandStarted {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        cwd: Option<AgentRunPath>,
    },
    CommandFinished {
        command: String,
        /// Whether it succeeded. Not derivable from `exitCode`: a command
        /// killed by a signal has no exit code and did not succeed.
        ok: bool,
        // `rename_all` on the enum renames variants, not the fields inside
        // them, so a struct variant spells its own camelCase — the same reason
        // `AgentTimelineKind::ToolCall` renames `toolUseId` by hand.
        #[serde(rename = "exitCode", default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        exit_code: Option<i32>,
    },
    /// The run is asking to do something it needs permission for. Reported, not
    /// answerable: nothing here creates a prompt the user can act on, which is
    /// T335's job on the observed side and a later task's here.
    PermissionRequested {
        tool: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        detail: Option<String>,
    },
    /// The run needs a question answered. Turning this into a workstream
    /// `Decision` is T335's; `Decision::raised_by` is already an `Actor`, so it
    /// costs no shape change when it happens.
    DecisionRequested {
        question: String,
    },
    /// The run stopped and cannot go on by itself.
    RunBlocked {
        reason: String,
    },
    /// It can go on again.
    RunResumed,
    /// The run ended badly.
    RunFailed {
        error: CoreError,
    },
}

impl AgentRunEvent {
    /// The `type` discriminator as it appears on the wire. Used for tracing,
    /// which records the kind and never the text.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::RunStarted => "RUN_STARTED",
            Self::RunFinished { .. } => "RUN_FINISHED",
            Self::PromptStarted { .. } => "PROMPT_STARTED",
            Self::PromptFinished { .. } => "PROMPT_FINISHED",
            Self::FileRead { .. } => "FILE_READ",
            Self::FileWriteRequested { .. } => "FILE_WRITE_REQUESTED",
            Self::FileWritten { .. } => "FILE_WRITTEN",
            Self::CommandStarted { .. } => "COMMAND_STARTED",
            Self::CommandFinished { .. } => "COMMAND_FINISHED",
            Self::PermissionRequested { .. } => "PERMISSION_REQUESTED",
            Self::DecisionRequested { .. } => "DECISION_REQUESTED",
            Self::RunBlocked { .. } => "RUN_BLOCKED",
            Self::RunResumed => "RUN_RESUMED",
            Self::RunFailed { .. } => "RUN_FAILED",
        }
    }

    /// Every string bounded and whitespace-collapsed.
    ///
    /// **The single call that makes rule 13 true for this protocol.** The run
    /// manager applies it to everything an adapter hands over, once, before the
    /// event is recorded or emitted; nothing downstream re-checks, and nothing
    /// upstream is trusted to have done it. Idempotent, so applying it twice is
    /// harmless.
    #[must_use]
    pub fn bounded(self) -> Self {
        match self {
            Self::RunStarted => Self::RunStarted,
            Self::RunResumed => Self::RunResumed,
            Self::RunFinished { summary } => Self::RunFinished {
                summary: clip_optional(summary.as_deref(), MAX_TEXT),
            },
            Self::PromptStarted { text } => Self::PromptStarted {
                text: clip(&text, MAX_TEXT),
            },
            Self::PromptFinished { text } => Self::PromptFinished {
                text: clip_optional(text.as_deref(), MAX_TEXT),
            },
            Self::FileRead { path } => Self::FileRead {
                path: path.bounded(),
            },
            Self::FileWriteRequested { path } => Self::FileWriteRequested {
                path: path.bounded(),
            },
            Self::FileWritten { path } => Self::FileWritten {
                path: path.bounded(),
            },
            Self::CommandStarted { command, cwd } => Self::CommandStarted {
                command: clip(&command, MAX_COMMAND),
                cwd: cwd.map(AgentRunPath::bounded),
            },
            Self::CommandFinished {
                command,
                ok,
                exit_code,
            } => Self::CommandFinished {
                command: clip(&command, MAX_COMMAND),
                ok,
                exit_code,
            },
            Self::PermissionRequested { tool, detail } => Self::PermissionRequested {
                tool: clip(&tool, MAX_TOOL),
                detail: clip_optional(detail.as_deref(), MAX_DETAIL),
            },
            Self::DecisionRequested { question } => Self::DecisionRequested {
                question: clip(&question, MAX_QUESTION),
            },
            Self::RunBlocked { reason } => Self::RunBlocked {
                reason: clip(&reason, MAX_REASON),
            },
            // A failure message is very often built *from* what an adapter
            // said, so it is bounded like everything else rather than trusted
            // because it arrived inside a `CoreError`. `details` is left alone:
            // it is structured context the core builds itself (for
            // `InvalidInput`, the field name the UI needs), never adapter prose.
            Self::RunFailed { error } => Self::RunFailed {
                error: CoreError {
                    message: clip(&error.message, MAX_REASON),
                    ..error
                },
            },
        }
    }
}

/// One event as it is delivered: which run, when, and what.
///
/// `run_id` plus a timestamp is the whole of the correlation the plan asks for.
/// Looking the run up supplies `workstream_id` and `workspace_id`, so the chain
/// **Workspace → Workstream → Agent Run** is answered without repeating three
/// ids on every event. The plan names a fourth link, "→ Agent Session"; a run
/// Gritty owns has no reason to also be a row in the observed roster, so this
/// protocol does not carry it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunEventRecord {
    /// Strictly increasing per run, so a consumer appends rather than
    /// reconciles. It carries no meaning beyond order.
    pub seq: u64,
    pub run_id: AgentRunId,
    pub at: Timestamp,
    pub event: AgentRunEvent,
}

impl AgentRunEventRecord {
    /// A record whose event has been bounded. The constructor the run manager
    /// uses, so that bounding cannot be forgotten at a call site.
    pub fn new(seq: u64, run_id: AgentRunId, at: Timestamp, event: AgentRunEvent) -> Self {
        Self {
            seq,
            run_id,
            at,
            event: event.bounded(),
        }
    }
}

/// Everything a client holds for one run's events: the response to
/// `runs.events`, which is what rehydrates a renderer after a reload (the T010
/// convention).
///
/// The shape mirrors `AgentTimeline`, and for the same reason: a bounded ring
/// that reports what it dropped rather than hiding it, so the UI can draw a gap
/// instead of splicing unrelated entries together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunEventLog {
    pub run_id: AgentRunId,
    pub events: Vec<AgentRunEventRecord>,
    /// Events evicted from the **oldest** end because the ring filled — see
    /// [`MAX_RUN_EVENTS`]. Reported, never hidden.
    pub dropped: u32,
}

/// Payload of `AgentRunChanged`: the whole run.
///
/// Emitted on a **state or activity change only**, never per event, matching
/// `WorkstreamChanged`. A run is small and a consumer that missed one still
/// converges on the next, which is worth more than the bytes a delta would
/// save (rule 8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunChangedPayload {
    pub run: AgentRun,
}

/// Payload of `AgentRunEventAppended`: a **delta**, not a snapshot.
///
/// Matching `AgentTimelineAppended`, and unlike the payload above: a run's event
/// stream grows without bound, so resending it would be the unbounded
/// forwarding rule 8 forbids. Batches are coalesced by the run manager — never
/// one event per call — and a consumer that missed one calls `runs.events` to
/// resynchronise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunEventAppendedPayload {
    pub run_id: AgentRunId,
    pub events: Vec<AgentRunEventRecord>,
    /// Cumulative, like [`AgentRunEventLog::dropped`]: a client compares it
    /// with what it last saw to know a gap opened.
    pub dropped: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::error::CoreErrorCode;

    fn run() -> AgentRun {
        AgentRun {
            id: AgentRunId::from_raw("run-1"),
            workstream_id: WorkstreamId::from_raw("ws-1"),
            workspace_id: WorkspaceId::from_raw("w-1"),
            provider: AgentRunProvider::Fake,
            state: AgentRunState::Starting,
            activity: None,
            started_at: Timestamp::from_millis(1),
            updated_at: Timestamp::from_millis(1),
            finished_at: None,
            cancellation_requested: false,
            correlation_id: CorrelationId::from_raw("corr-1"),
            error: None,
        }
    }

    #[test]
    fn a_fresh_run_omits_absent_fields_and_uses_camel_case() {
        let json = serde_json::to_value(run()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "run-1",
                "workstreamId": "ws-1",
                "workspaceId": "w-1",
                "provider": "FAKE",
                "state": "STARTING",
                "startedAt": 1,
                "updatedAt": 1,
                "cancellationRequested": false,
                "correlationId": "corr-1",
            })
        );
        let back: AgentRun = serde_json::from_value(json).unwrap();
        assert_eq!(back, run());
    }

    #[test]
    fn a_finished_run_carries_its_ending() {
        let mut finished = run();
        finished.state = AgentRunState::Failed;
        finished.finished_at = Some(Timestamp::from_millis(9));
        finished.error = Some(CoreError::internal("the script ended without finishing"));
        let json = serde_json::to_value(&finished).unwrap();
        assert_eq!(json["state"], "FAILED");
        assert_eq!(json["finishedAt"], 9);
        assert_eq!(json["error"]["code"], "Internal");
        let back: AgentRun = serde_json::from_value(json).unwrap();
        assert_eq!(back, finished);
    }

    #[test]
    fn providers_round_trip_and_reject_the_observed_rosters_spelling() {
        for &provider in AgentRunProvider::ALL {
            assert_eq!(
                AgentRunProvider::parse(provider.as_str()).unwrap(),
                provider
            );
            assert_eq!(
                serde_json::to_value(provider).unwrap(),
                serde_json::json!(provider.as_str())
            );
        }
        // The point of the separate enum: a provider only the observed roster
        // knows about is not a run provider.
        let error = AgentRunProvider::parse("CLAUDE_CODE").expect_err("not a run provider");
        assert_eq!(error.code, CoreErrorCode::Internal);
    }

    #[test]
    fn activity_kinds_round_trip() {
        for &kind in AgentRunActivityKind::ALL {
            assert_eq!(AgentRunActivityKind::parse(kind.as_str()).unwrap(), kind);
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                serde_json::json!(kind.as_str())
            );
        }
        assert_eq!(
            AgentRunActivityKind::parse("WORKING").unwrap_err().code,
            CoreErrorCode::Internal
        );
    }

    /// State and activity are separate concepts (plan §6): the state holds
    /// still while the activity cycles.
    #[test]
    fn activity_serialises_beside_an_unchanged_state() {
        let mut running = run();
        running.state = AgentRunState::Running;
        for kind in [
            AgentRunActivityKind::Editing,
            AgentRunActivityKind::Executing,
        ] {
            running.activity = Some(AgentRunActivity::new(kind, Timestamp::from_millis(2)));
            let json = serde_json::to_value(&running).unwrap();
            assert_eq!(json["state"], "RUNNING");
            assert_eq!(json["activity"]["kind"], kind.as_str());
            assert_eq!(json["activity"]["startedAt"], 2);
            assert!(json["activity"].get("path").is_none());
        }
    }

    #[test]
    fn clip_collapses_whitespace_and_bounds_characters_not_bytes() {
        assert_eq!(clip("  a \n b\tc  ", 100), "a b c");
        assert_eq!(clip("abcdefghij", 5), "abcd…");
        // Ten emoji are ten characters, not forty bytes.
        assert_eq!(clip(&"🙂".repeat(10), 10).chars().count(), 10);
        assert_eq!(clip(&"🙂".repeat(11), 10).chars().count(), 10);
        assert!(clip(&"🙂".repeat(11), 10).ends_with('…'));
        // Bounding is idempotent: applying it to its own output changes
        // nothing, which is what lets the manager call it without tracking
        // whether it already has.
        let once = clip(&"x".repeat(500), MAX_COMMAND);
        assert_eq!(clip(&once, MAX_COMMAND), once);
    }

    #[test]
    fn clip_does_not_walk_an_enormous_field() {
        // Far past MAX_CLIP_SOURCE_CHARS, and still bounded.
        let huge = "a ".repeat(200_000);
        assert_eq!(clip(&huge, MAX_TEXT).chars().count(), MAX_TEXT);
    }

    #[test]
    fn clip_optional_treats_blank_as_absent() {
        assert_eq!(clip_optional(None, 10), None);
        assert_eq!(clip_optional(Some("   \n "), 10), None);
        assert_eq!(clip_optional(Some(" x "), 10), Some("x".to_owned()));
    }

    /// A path outside the workspace is shown and never openable; one inside is
    /// both. ADR-010's rule, as the type enforces it.
    #[test]
    fn paths_follow_the_file_touched_rule() {
        let inside = AgentRunPath::inside(WorkspacePath::parse("src/main.rs").unwrap());
        let json = serde_json::to_value(&inside).unwrap();
        assert_eq!(json["path"], "src/main.rs");
        assert_eq!(json["display"], "src/main.rs");

        let outside = AgentRunPath::outside("~/other/repo/main.rs");
        let json = serde_json::to_value(&outside).unwrap();
        assert!(json.get("path").is_none(), "nothing outside is openable");
        assert_eq!(json["display"], "~/other/repo/main.rs");

        // The display form is bounded like everything else.
        let long = AgentRunPath::outside("~/".to_owned() + &"deep/".repeat(200));
        assert_eq!(long.display.chars().count(), MAX_DISPLAY_PATH);
        assert!(long.path.is_none());
    }

    fn oversized() -> Vec<AgentRunEvent> {
        let long = "x ".repeat(4_000);
        let path = AgentRunPath {
            path: None,
            display: long.clone(),
        };
        vec![
            AgentRunEvent::RunStarted,
            AgentRunEvent::RunFinished {
                summary: Some(long.clone()),
            },
            AgentRunEvent::PromptStarted { text: long.clone() },
            AgentRunEvent::PromptFinished {
                text: Some(long.clone()),
            },
            AgentRunEvent::FileRead { path: path.clone() },
            AgentRunEvent::FileWriteRequested { path: path.clone() },
            AgentRunEvent::FileWritten { path: path.clone() },
            AgentRunEvent::CommandStarted {
                command: long.clone(),
                cwd: Some(path.clone()),
            },
            AgentRunEvent::CommandFinished {
                command: long.clone(),
                ok: false,
                exit_code: Some(1),
            },
            AgentRunEvent::PermissionRequested {
                tool: long.clone(),
                detail: Some(long.clone()),
            },
            AgentRunEvent::DecisionRequested {
                question: long.clone(),
            },
            AgentRunEvent::RunBlocked {
                reason: long.clone(),
            },
            AgentRunEvent::RunResumed,
            AgentRunEvent::RunFailed {
                error: CoreError::internal(long),
            },
        ]
    }

    /// Rule 13, as a test over the whole vocabulary: no string an adapter can
    /// produce reaches the wire unbounded, and none of them carries a newline
    /// into a one-line row.
    #[test]
    fn every_string_field_is_bounded_before_it_reaches_the_wire() {
        let mut seen = Vec::new();
        for event in oversized() {
            seen.push(event.as_str());
            let json = serde_json::to_value(event.bounded()).unwrap();
            walk(&json, &json);
        }
        // The list is the vocabulary: a variant added without a bounding arm
        // fails here rather than shipping unbounded.
        assert_eq!(seen.len(), 14);
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 14, "every variant has its own discriminator");

        fn walk(value: &serde_json::Value, whole: &serde_json::Value) {
            match value {
                serde_json::Value::String(text) => {
                    assert!(
                        text.chars().count() <= MAX_TEXT,
                        "unbounded string {text:?} in {whole}"
                    );
                    assert!(
                        !text.contains('\n') && !text.contains('\t'),
                        "uncollapsed whitespace in {text:?}"
                    );
                }
                serde_json::Value::Object(map) => {
                    for nested in map.values() {
                        walk(nested, whole);
                    }
                }
                serde_json::Value::Array(items) => {
                    for nested in items {
                        walk(nested, whole);
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn each_field_is_bounded_by_its_own_limit() {
        let long = "y".repeat(9_000);
        let bounded = AgentRunEvent::CommandStarted {
            command: long.clone(),
            cwd: Some(AgentRunPath::outside(long.clone())),
        }
        .bounded();
        let AgentRunEvent::CommandStarted { command, cwd } = bounded else {
            panic!("variant changed under bounding");
        };
        assert_eq!(command.chars().count(), MAX_COMMAND);
        assert_eq!(cwd.unwrap().display.chars().count(), MAX_DISPLAY_PATH);

        let bounded = AgentRunEvent::PermissionRequested {
            tool: long.clone(),
            detail: Some(long),
        }
        .bounded();
        let AgentRunEvent::PermissionRequested { tool, detail } = bounded else {
            panic!("variant changed under bounding");
        };
        assert_eq!(tool.chars().count(), MAX_TOOL);
        assert_eq!(detail.unwrap().chars().count(), MAX_DETAIL);

        // A failure message is bounded too: it is routinely built from what an
        // adapter said, and arriving inside a `CoreError` does not make it
        // trusted.
        let bounded = AgentRunEvent::RunFailed {
            error: CoreError::internal("z".repeat(9_000)),
        }
        .bounded();
        let AgentRunEvent::RunFailed { error } = bounded else {
            panic!("variant changed under bounding");
        };
        assert_eq!(error.message.chars().count(), MAX_REASON);
        assert_eq!(error.code, CoreErrorCode::Internal);
    }

    #[test]
    fn bounding_is_idempotent() {
        for event in oversized() {
            let once = event.bounded();
            assert_eq!(once.clone().bounded(), once);
        }
    }

    #[test]
    fn the_record_constructor_bounds_what_it_is_given() {
        let record = AgentRunEventRecord::new(
            1,
            AgentRunId::from_raw("run-1"),
            Timestamp::from_millis(5),
            AgentRunEvent::PromptStarted {
                text: "x".repeat(9_000),
            },
        );
        let AgentRunEvent::PromptStarted { text } = &record.event else {
            panic!("variant changed under bounding");
        };
        assert_eq!(text.chars().count(), MAX_TEXT);
    }

    #[test]
    fn events_are_tagged_and_round_trip() {
        for event in oversized() {
            let name = event.as_str();
            let json = serde_json::to_value(event.clone().bounded()).unwrap();
            assert_eq!(json["type"], name);
            let back: AgentRunEvent = serde_json::from_value(json).unwrap();
            assert_eq!(back, event.bounded());
        }
        assert_eq!(
            serde_json::to_value(AgentRunEvent::RunStarted).unwrap(),
            serde_json::json!({ "type": "RUN_STARTED" })
        );
    }

    /// `rename_all` on a tagged enum renames its *variants*, not the fields
    /// inside them — so every multi-word field of a struct variant has to spell
    /// its own camelCase, and this is the test that notices when one does not.
    #[test]
    fn struct_variant_fields_are_camel_case_on_the_wire() {
        let json = serde_json::to_value(AgentRunEvent::CommandFinished {
            command: "cargo test".into(),
            ok: false,
            exit_code: Some(101),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "type": "COMMAND_FINISHED",
                "command": "cargo test",
                "ok": false,
                "exitCode": 101,
            })
        );
        for event in oversized() {
            let json = serde_json::to_value(event.bounded()).unwrap();
            for key in json.as_object().expect("a tagged object").keys() {
                assert!(
                    !key.contains('_') || key == "type",
                    "{key:?} is not camelCase in {json}"
                );
            }
        }
    }

    #[test]
    fn a_record_carries_the_run_and_the_time_and_nothing_else() {
        let record = AgentRunEventRecord::new(
            7,
            AgentRunId::from_raw("run-1"),
            Timestamp::from_millis(11),
            AgentRunEvent::RunResumed,
        );
        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "seq": 7,
                "runId": "run-1",
                "at": 11,
                "event": { "type": "RUN_RESUMED" },
            })
        );
        let back: AgentRunEventRecord = serde_json::from_value(json).unwrap();
        assert_eq!(back, record);
    }

    #[test]
    fn the_log_and_the_delta_report_drops_rather_than_hiding_them() {
        let log = AgentRunEventLog {
            run_id: AgentRunId::from_raw("run-1"),
            events: Vec::new(),
            dropped: 3,
        };
        assert_eq!(
            serde_json::to_value(&log).unwrap(),
            serde_json::json!({ "runId": "run-1", "events": [], "dropped": 3 })
        );
        let delta = AgentRunEventAppendedPayload {
            run_id: AgentRunId::from_raw("run-1"),
            events: vec![AgentRunEventRecord::new(
                1,
                AgentRunId::from_raw("run-1"),
                Timestamp::from_millis(2),
                AgentRunEvent::RunStarted,
            )],
            dropped: 0,
        };
        let json = serde_json::to_value(&delta).unwrap();
        assert_eq!(json["events"][0]["event"]["type"], "RUN_STARTED");
        assert_eq!(json["dropped"], 0);
    }
}
