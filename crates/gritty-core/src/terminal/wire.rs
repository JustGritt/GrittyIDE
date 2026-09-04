//! Wire types of the `terminal.*` commands and the terminal events (T090).
//!
//! The frontend addresses a session by [`TerminalId`] and never by pid: a pid
//! is reported for the user's benefit (and for `pgrep` when something goes
//! wrong), but it is not an identity the protocol accepts, because it is
//! reused by the operating system and means nothing after the process is gone.
//!
//! A terminal's working directory is a workspace-relative [`WorkspacePath`],
//! like every other path in the protocol (ADR-010): the WebView cannot ask for
//! a shell rooted anywhere it likes. The user can of course `cd` afterwards —
//! this bounds where Gritty *starts* a shell, not what the user may do in it.

use gritty_workspace::WorkspacePath;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::ids::{TerminalId, WorkspaceId};
use crate::protocol::time::Timestamp;

/// Whether a session is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TerminalStatus {
    Running,
    /// The shell ended, on its own or because Gritty terminated it. The session
    /// stays listed so the user can still read why it ended.
    Exited,
}

/// A terminal session as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Terminal {
    pub id: TerminalId,
    pub workspace_id: WorkspaceId,
    /// Where the shell was started, relative to the workspace root.
    pub cwd: WorkspacePath,
    /// Absolute path of the program that was started. Display only.
    pub shell: String,
    /// Process id of the shell while it runs; `None` once it has exited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pid: Option<u32>,
    pub status: TerminalStatus,
    pub cols: u16,
    pub rows: u16,
    pub created_at: Timestamp,
    /// Exit status, once the shell has exited without a signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub exit_code: Option<i32>,
    /// Canonical signal name (`SIGKILL`), once a signal ended it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub exit_signal: Option<String>,
}

/// `terminal.spawn`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpawnTerminalRequest {
    pub workspace_id: WorkspaceId,
    /// Workspace-relative directory to start in; the root when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cwd: Option<WorkspacePath>,
    /// Absolute path of a shell to run instead of the user's own. Rejected
    /// when it is not an executable file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub shell: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cols: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rows: Option<u16>,
}

/// `terminal.write`: send keystrokes or pasted text to the shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WriteTerminalRequest {
    pub terminal_id: TerminalId,
    /// What the user typed, including control characters such as `\u{3}` for
    /// Ctrl+C. Sent verbatim; the shell, not Gritty, interprets it.
    pub data: String,
}

/// `terminal.resize`: the visible grid changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResizeTerminalRequest {
    pub terminal_id: TerminalId,
    pub cols: u16,
    pub rows: u16,
}

/// `terminal.kill`: terminate the session's process tree. Idempotent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct KillTerminalRequest {
    pub terminal_id: TerminalId,
}

/// `TerminalOutput`: a coalesced batch of what the shell printed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputPayload {
    pub terminal_id: TerminalId,
    pub data: String,
    /// Bytes dropped before this batch because output arrived faster than it
    /// could be delivered. Non-zero means output is genuinely missing and the
    /// UI must say so rather than presenting a gap as continuous output.
    /// Always present: an absent field would read as "no truncation" whether
    /// or not the core meant to say that.
    #[serde(default)]
    pub truncated_bytes: u64,
}

/// `TerminalExited`: emitted exactly once per session, after its last output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExitedPayload {
    /// The final snapshot; its `status` is `EXITED`.
    pub terminal: Terminal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub signal: Option<String>,
}
