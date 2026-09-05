//! Wire types of the `agents.*` domain (T220), exported to TypeScript by
//! `cargo test -p gritty-core`.
//!
//! These describe an **observation**, not a run. Nothing here is a handle: the
//! renderer cannot act on an [`AgentSession`], because Gritty does not own the
//! session — some other terminal does. When T300 introduces runs Gritty *does*
//! own, that is a different type with a different lifecycle, and this one keeps
//! meaning "something else on this machine is working".
//!
//! Every string that came out of a transcript is untrusted and already bounded
//! by the adapter (rule 13). The renderer shows them as text and never as
//! markup, a path it opens, or a command it runs.

use gritty_workspace::WorkspacePath;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::time::Timestamp;

/// Which agent CLI produced a session. Additive: an unknown value from a newer
/// core reaches an older client as a string it can show but not branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentProvider {
    ClaudeCode,
}

/// Which sources saw the session. Surfaced rather than hidden so the UI can be
/// honest about a partial answer instead of implying certainty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentEvidence {
    /// A live process, no readable transcript: it is running, but what it is
    /// doing is unknown.
    ProcessOnly,
    /// A transcript, no live process: what it did is known, it is not running.
    TranscriptOnly,
    /// Both agreed.
    Both,
}

/// What the session is doing, derived from evidence alone.
///
/// There is deliberately no `WAITING_FOR_APPROVAL`. Poll-only discovery cannot
/// tell a pending permission prompt from a finished turn — both look like a
/// live process with nothing outstanding — and inventing the distinction would
/// be exactly the kind of claim `AGENTS.md` rule 12 forbids. A hook
/// integration is what would earn that value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentActivity {
    /// A tool call is outstanding: the agent issued it and no result has been
    /// recorded yet.
    Working,
    /// The process is alive with nothing outstanding — the user's move.
    Waiting,
    /// Alive, but the transcript has been quiet past the idle threshold.
    Idle,
    /// No live process was found.
    Ended,
}

impl AgentActivity {
    /// Whether this session is the reason to look at the island.
    pub const fn needs_you(self) -> bool {
        matches!(self, Self::Waiting)
    }

    pub const fn is_running(self) -> bool {
        matches!(self, Self::Working | Self::Waiting | Self::Idle)
    }
}

/// The tool call in flight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolCall {
    /// `Bash`, `Edit`, `Read`, …
    pub name: String,
    /// A bounded preview of the input. Display only — never re-run, never
    /// resolved as a path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
}

/// One agent session running on (or recently run on) this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    /// Stable for the life of the session, and unique across providers.
    pub id: String,
    pub provider: AgentProvider,
    pub activity: AgentActivity,
    pub evidence: AgentEvidence,
    /// The agent's own name for the session — the closest thing to "the task".
    /// Absent until the agent has published one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    /// The standing user request, when the transcript carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_prompt: Option<String>,
    /// The agent's last message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_reply: Option<String>,
    /// The session's working directory, shortened for display (`~/…`). The
    /// renderer has no filesystem, so the shortening happens in the core. There
    /// is no unshortened root on purpose: nothing may open it from here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub root: Option<String>,
    /// Last component of the working directory — what the roster groups by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub root_name: Option<String>,
    /// Whether the session is running inside the workspace that is open now.
    /// The roster is machine-wide; this is what lets "here" sort first.
    pub in_workspace: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub git_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tool: Option<AgentToolCall>,
    /// The process id, when one was seen. Shown so the user can find the
    /// session themselves; Gritty never signals it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pid: Option<u32>,
    /// The controlling terminal, which is how the user finds the right window.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tty: Option<String>,
    /// When the transcript was last written. Absent for a session with no
    /// readable transcript: Gritty knows it is running, not for how long.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub last_active_at: Option<Timestamp>,
}

/// `agents.list` / `agents.refresh`. No fields yet: the roster is
/// machine-wide and already bounded by the core's scan limits.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ListAgentSessionsRequest {}

/// The whole roster: the response to `agents.list` / `agents.refresh` and the
/// payload of `AgentSessionsChanged`, which are the same thing.
///
/// One event carrying everything, rather than per-session deltas: the roster is
/// small and bounded, and a consumer that missed an event still converges on
/// the next one — which matters more than the bytes saved (rule 8).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentRoster {
    pub sessions: Vec<AgentSession>,
    /// How many are `WAITING` — the island's "needs you" count.
    pub needs_you: u32,
    /// Whether process evidence was available at all. `false` means the roster
    /// is transcript-only, so every session reads as `ENDED` and the UI should
    /// say why rather than claim nothing is running.
    pub process_evidence: bool,
}

// ---------------------------------------------------------------------------
// Timeline (T230 contract)
//
// The roster above answers *which* agents are running. These types answer what
// one of them is doing, and they are the frozen surface T230 (the follower and
// the domain) and T231/T232 (the rail) are both built against.
//
// The same rules apply as above, and more sharply: every string here came out
// of an agent's transcript, so it is untrusted input that the core bounds
// before it crosses this boundary (rule 13). Nothing here is provider-shaped —
// the adapter translates Claude Code's records into these kinds, and a second
// provider adds no variant (rule 4).
// ---------------------------------------------------------------------------

/// How much a [`AgentTimelineKind::Notice`] matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentNoticeLevel {
    Info,
    Warning,
    Error,
}

/// What a single timeline entry *is*.
///
/// Tagged the way [`crate::protocol::actor::Actor`] is, so an older client can
/// show an unknown kind as text instead of failing to parse the batch it
/// arrived in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentTimelineKind {
    /// Something the user asked for.
    UserPrompt { text: String },
    /// Something the agent said.
    AssistantText { text: String },
    /// Reasoning the agent showed. Separate from `AssistantText` so the UI can
    /// treat it as subordinate without parsing the text to guess.
    Thinking { text: String },
    /// A tool the agent invoked. `detail` is the core's bounded preview of the
    /// input — display only, never re-run and never resolved as a path.
    ToolCall {
        #[serde(rename = "toolUseId")]
        tool_use_id: Option<String>,
        name: String,
        detail: Option<String>,
    },
    /// How that call ended, paired to it by `toolUseId`.
    ToolResult {
        #[serde(rename = "toolUseId")]
        tool_use_id: Option<String>,
        ok: bool,
        detail: Option<String>,
    },
    /// A file the agent wrote.
    ///
    /// `path` is present **only** when the file is inside the open workspace,
    /// which is the whole point: it is what the renderer may open, and its
    /// absence is what stops anything outside the root being opened from the
    /// WebView (ADR-010). `display` is always safe to show and never to resolve.
    FileTouched {
        path: Option<WorkspacePath>,
        display: String,
    },
    /// Something the run itself reported — a hook failure, a system notice.
    Notice {
        level: AgentNoticeLevel,
        text: String,
    },
    /// The agent is stopped at a permission prompt, naming what it is waiting
    /// on. Only ever produced from hook evidence (T221, ADR-018); no transcript
    /// pattern may synthesise it, because a prompt and a running tool look
    /// identical in a transcript (rule 12).
    PermissionPending {
        tool: String,
        detail: Option<String>,
    },
}

/// One entry, with the ordering the renderer appends by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentTimelineEntry {
    /// Strictly increasing per session, so a consumer appends rather than
    /// reconciles. It is not a transcript offset and carries no meaning beyond
    /// order.
    pub seq: u64,
    /// When the underlying record was written, when it says. Absent rather than
    /// invented: stamping an entry with the scan time would make a static
    /// timeline look like it was changing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub at: Option<Timestamp>,
    pub kind: AgentTimelineKind,
    /// Subagent work. Marked rather than dropped — T220 dropped it and listed
    /// that as a known limitation.
    pub sidechain: bool,
}

/// `agents.openTimeline`, `agents.closeTimeline` and `agents.timeline` all
/// address a session and nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentTimelineRequest {
    pub session_id: String,
}

/// The whole timeline a client holds: the response to `agents.openTimeline` and
/// to `agents.timeline`, which is what rehydrates a renderer after a reload
/// without reopening anything (the T010 convention).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentTimeline {
    pub session_id: String,
    pub entries: Vec<AgentTimelineEntry>,
    /// Entries discarded from the **oldest** end because the ring filled. The
    /// terminal's rule: report the loss rather than hide it, so the UI can draw
    /// a gap instead of splicing unrelated entries together.
    pub dropped: u32,
    /// The transcript was truncated or replaced under the follower, so this
    /// view was reset rather than continued.
    pub restarted: bool,
    /// Whether a transcript was readable at all. `false` with an empty timeline
    /// means "nothing to read", not "the agent did nothing".
    pub transcript_evidence: bool,
}

/// Payload of `AgentTimelineAppended`: a **delta**, not a snapshot.
///
/// Unlike `AgentSessionsChanged`, which carries the whole roster because it is
/// small, a timeline grows without bound and resending it per tick would be the
/// unbounded forwarding rule 8 forbids. A consumer that missed an event calls
/// `agents.timeline` to resynchronise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentTimelineAppendedPayload {
    pub session_id: String,
    pub entries: Vec<AgentTimelineEntry>,
    pub dropped: u32,
    pub restarted: bool,
}
