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

use crate::protocol::error::CoreError;
use crate::protocol::time::Timestamp;

/// Which agent CLI produced a session. Additive: an unknown value from a newer
/// core reaches an older client as a string it can show but not branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentProvider {
    ClaudeCode,
}

impl AgentProvider {
    pub const ALL: &'static [Self] = &[Self::ClaudeCode];

    /// The stored and transmitted spelling, which is the serde one. A
    /// workstream link (T223) keeps this in a column of its own, so the two
    /// have to agree — hence one function rather than a round trip through
    /// serde at every call site.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "CLAUDE_CODE",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        Self::ALL
            .iter()
            .copied()
            .find(|provider| provider.as_str() == raw)
            .ok_or_else(|| CoreError::internal(format!("unknown agent provider {raw:?}")))
    }
}

/// Which sources saw the session. Surfaced rather than hidden so the UI can be
/// honest about a partial answer instead of implying certainty.
///
/// T221 adds the hook combinations. There is deliberately no `HOOK_ONLY`: a
/// state file with no matching process and no matching transcript creates no
/// roster row, so the combination cannot occur (ADR-018 §7 — evidence, not
/// authority).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentEvidence {
    /// A live process, no readable transcript: it is running, but what it is
    /// doing is unknown.
    ProcessOnly,
    /// A transcript, no live process: what it did is known, it is not running.
    TranscriptOnly,
    /// Process and transcript agreed.
    Both,
    /// A live process and the agent's own hook, but no readable transcript.
    ProcessAndHook,
    /// A transcript and the agent's own hook, but no live process.
    TranscriptAndHook,
    /// All three.
    All,
}

impl AgentEvidence {
    /// The same evidence with the hook added, when a state file joined this
    /// session. Keeps the two axes — which sources saw it, and whether one of
    /// them was the agent itself — from becoming two fields to keep in step.
    pub const fn with_hook(self) -> Self {
        match self {
            Self::ProcessOnly | Self::ProcessAndHook => Self::ProcessAndHook,
            Self::TranscriptOnly | Self::TranscriptAndHook => Self::TranscriptAndHook,
            Self::Both | Self::All => Self::All,
        }
    }

    /// Whether the agent's own hook reported on this session.
    pub const fn has_hook(self) -> bool {
        matches!(
            self,
            Self::ProcessAndHook | Self::TranscriptAndHook | Self::All
        )
    }
}

/// Which of the three sources produced *this* verdict (T221).
///
/// With three sources that can disagree, a row has to be able to say why it
/// claims what it claims — otherwise `BLOCKED` is magic. Herdr ships the same
/// idea as an `agent explain` subcommand; here it is a field, because the rail
/// already has somewhere to put it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentActivitySource {
    /// The process table: alive, or gone.
    Process,
    /// The transcript: a tool in flight, or a long silence.
    Transcript,
    /// The agent's own hook. Only ever the source of `BLOCKED`.
    Hook,
}

/// What the session is doing, derived from evidence alone.
///
/// `BLOCKED` is the one value poll-only discovery cannot reach: a pending
/// permission prompt and a finished turn are both a live process with nothing
/// outstanding. It is set **only** from hook evidence (T221, ADR-018), which
/// satisfies rule 12 by construction — the value exists exactly where something
/// told Gritty the truth, and no transcript pattern, process state or
/// elapsed-time threshold may produce it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentActivity {
    /// A tool call is outstanding: the agent issued it and no result has been
    /// recorded yet.
    Working,
    /// Stopped at a permission prompt, and waiting for the user to answer it.
    /// Hook evidence only.
    Blocked,
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
        matches!(self, Self::Waiting | Self::Blocked)
    }

    pub const fn is_running(self) -> bool {
        matches!(
            self,
            Self::Working | Self::Blocked | Self::Waiting | Self::Idle
        )
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
    /// Which source produced [`Self::activity`] — not which sources saw the
    /// session, which is [`Self::evidence`].
    pub activity_source: AgentActivitySource,
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
    /// How many are `BLOCKED` or `WAITING` — the island's "needs you" count.
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
    ///
    /// The one entry that is not in the ring — [`AgentTimeline::pending`] — is
    /// stamped `u64::MAX`, because it has no position in the sequence: it is
    /// always the last thing in the feed, and it disappears rather than being
    /// superseded.
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
    /// The permission prompt this session is stopped at, if any (T221).
    ///
    /// An [`AgentTimelineKind::PermissionPending`] entry, held **beside** the
    /// ring rather than in it, because it is a *state* and not something that
    /// happened: it appears when the hook reports `BLOCKED` and goes when the
    /// phase leaves it. An entry in the ring could only ever be appended, so a
    /// prompt that was answered would stay in the feed forever claiming to be
    /// waiting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pending: Option<AgentTimelineEntry>,
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
    /// The current pending permission prompt, whole rather than as a delta
    /// (T221).
    ///
    /// It is a state, so every batch carries its current value and a consumer
    /// replaces rather than appends; `None` means the session is not stopped at
    /// a prompt. A batch is emitted when *this* changes even with no entries
    /// appended, because entering and leaving a prompt is exactly the change
    /// worth telling the rail about — and, as everywhere else, a tick that
    /// changes nothing emits nothing (rule 8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pending: Option<AgentTimelineEntry>,
}

// ---------------------------------------------------------------------------
// The hook integration (T221, ADR-018)
//
// Reading another tool's configuration file, and — only on an explicit user
// action, additively, and with a backup — writing one entry into it. These
// types are what the settings surface shows *before* it applies anything: the
// file, the events, the entry and the backup path.
//
// Deliberately absent: the configuration's contents. The change is the command
// line and the events it lands on; echoing a user's whole settings file across
// the protocol would put whatever else lives in it into an event payload for no
// gain (rule 13).
// ---------------------------------------------------------------------------

/// Whether Gritty's hook is in a provider's configuration, and whether it works.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentIntegrationStatus {
    /// Nothing of Gritty's is in the file — including when there is no file.
    /// The default state, and the one discovery is complete in.
    NotInstalled,
    /// Present, and pointing at a binary that exists.
    Installed,
    /// Present, and pointing at a binary that is not there: the app moved or
    /// was upgraded. Nothing breaks for the user — the agent tolerates a hook
    /// that fails — the roster simply loses `BLOCKED`, and this says why.
    Broken,
    /// The file does not parse, or its hook section has a shape Gritty does not
    /// recognise. It is never rewritten, repaired or replaced from a template
    /// (ADR-018 §4).
    UnsupportedConfig,
}

/// `agents.integrationStatus`, and the subject of an install or uninstall.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentIntegrationRequest {
    pub provider: AgentProvider,
}

/// `agents.integrationInstall` / `agents.integrationUninstall`.
///
/// One shape for both the preview and the change, so the UI shows the diff it
/// is about to apply rather than a description of one (ADR-018 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentIntegrationChangeRequest {
    pub provider: AgentProvider,
    /// `false` returns the proposed change and writes nothing.
    pub apply: bool,
}

/// What the provider's configuration says today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentIntegration {
    pub provider: AgentProvider,
    pub status: AgentIntegrationStatus,
    /// The file, shortened for display (`~/…`) like every other path that
    /// crosses this boundary. The renderer has no filesystem and opens nothing
    /// from here.
    pub config_path: String,
    /// The events Gritty's entry covers today. Empty unless installed.
    pub events: Vec<String>,
    /// Why it is broken or unsupported — the parse error, or the binary that is
    /// not there. Absent when there is nothing to explain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub detail: Option<String>,
    /// Whether this build knows where its own hook binary is. `false` means an
    /// install would produce a [`AgentIntegrationStatus::Broken`] entry, so the
    /// UI offers nothing rather than writing one.
    pub hook_available: bool,
}

/// One event's share of a proposed or performed change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentIntegrationEvent {
    pub event: String,
    /// How many entries from *other* tools this event already carries, and
    /// which the change leaves exactly as they are. Shown because "we are
    /// appending to a list that already has something in it" is the fact that
    /// makes the change trustworthy (ADR-018 §2).
    pub foreign_entries: u32,
}

/// A change to a provider's configuration: proposed when `applied` is false,
/// performed when it is true.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentIntegrationChange {
    pub provider: AgentProvider,
    /// Whether this actually happened, or is what would happen.
    pub applied: bool,
    pub config_path: String,
    /// Where the prior contents were copied, or would be. Absent when the file
    /// does not exist yet, because there is nothing to back up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub backup_path: Option<String>,
    /// The file does not exist and would be created carrying only Gritty's
    /// entry.
    pub creates_config: bool,
    /// The exact command being added or removed. Shown, never run from here.
    pub command: String,
    /// The events the change lands on. Empty means there is nothing to do —
    /// an install over an existing entry is a no-op that says so rather than
    /// duplicating it.
    pub events: Vec<AgentIntegrationEvent>,
    /// The status after this change, or after it would be applied.
    pub status: AgentIntegrationStatus,
}
