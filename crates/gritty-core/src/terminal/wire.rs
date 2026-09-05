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
//!
//! A terminal does not *need* a workspace (T095). Without one the shell starts
//! in the user's home directory, and `workspaceId`/`cwd` are both absent. That
//! keeps the same property rather than weakening it: the renderer asks for "a
//! terminal, no workspace" and the **core** decides where that is. There is
//! still no way to name an arbitrary directory from the WebView.

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

/// A colour the user gave a session, so groups of shells can be told apart at
/// a glance (T098).
///
/// A closed set, not a free colour. The palette is CSS custom properties
/// (ADR-016) with a light ramp as well as a dark one; an arbitrary value chosen
/// against one of them would be unreadable in the other, and the core has no
/// business holding a hex string it cannot reason about. These are the six ANSI
/// hues the terminal already draws with, so a coloured pill and the output
/// under it agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TerminalColor {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
}

/// A terminal session as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Terminal {
    pub id: TerminalId,
    /// The workspace the shell was rooted in; absent for a session started
    /// without one, which lives in the user's home directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_id: Option<WorkspaceId>,
    /// Where the shell was started, relative to the workspace root. Absent
    /// exactly when `workspace_id` is: a home directory has no workspace to be
    /// relative to, and the protocol does not put absolute paths on the wire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cwd: Option<WorkspacePath>,
    /// Absolute path of the program that was started. Display only.
    pub shell: String,
    /// A label the user gave this session, replacing the shell's name in the
    /// tab. Absent until they set one. Sessions increasingly hold something
    /// worth telling apart — an agent, a dev server, a build — and a strip of
    /// tabs all reading "zsh" identifies none of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
    /// A colour the user gave this session, for grouping. Absent until they
    /// pick one, which is when the pill falls back to saying nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color: Option<TerminalColor>,
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
    /// Workspace to root the shell in. Omit it to start a shell that belongs
    /// to no workspace, in the user's home directory — the renderer never
    /// names that path, the core resolves it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_id: Option<WorkspaceId>,
    /// Workspace-relative directory to start in; the workspace root when
    /// omitted. Rejected without a `workspaceId`: there would be nothing to
    /// resolve it against, and resolving it against anything else is exactly
    /// what ADR-010 forbids.
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

/// `terminal.profiles`: what the picker should offer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ListProfilesRequest {
    /// Absolute paths of shells the user added by hand, from the terminal
    /// settings. Discovery is deliberately conservative — it reports only
    /// `$SHELL` and `/etc/shells` — so this is how a shell the system does not
    /// register (a Homebrew `fish` the user never added to `/etc/shells`)
    /// reaches the picker.
    ///
    /// Each is checked like a discovered one, but an unusable entry is
    /// *reported* rather than dropped: the user typed it, so they are owed the
    /// reason it does not work.
    #[serde(default)]
    pub custom: Vec<String>,
}

/// A shell the user can pick for a new session (`terminal.profiles`, T095).
///
/// `path` is what [`SpawnTerminalRequest::shell`] accepts, so a picker hands
/// back exactly what it was given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TerminalProfile {
    /// Absolute path of the shell.
    pub path: String,
    /// Short label for a menu: the file name, and its directory too when two
    /// shells on this machine share a file name (`/bin/bash` next to
    /// Homebrew's). Display only; `path` is the identity.
    pub name: String,
    /// The shell a session gets when `terminal.spawn` names none.
    pub is_default: bool,
    /// The user added this path themselves; it was not discovered. Only these
    /// can be removed in the settings.
    pub is_custom: bool,
    /// Whether it can be started right now. Always true for a discovered
    /// shell, which was checked as it was found; a custom one can go stale.
    pub available: bool,
    /// Why it cannot be started, when `available` is false. Display only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub unavailable_reason: Option<String>,
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

/// `terminal.rename`: label a session, or clear the label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RenameTerminalRequest {
    pub terminal_id: TerminalId,
    /// The new label. Absent, empty or all whitespace clears it, so the tab
    /// falls back to the shell's own name rather than showing a blank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub name: Option<String>,
}

/// `terminal.recolor`: give a session a colour, or clear it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecolorTerminalRequest {
    pub terminal_id: TerminalId,
    /// The new colour. Absent clears it, so the pill goes back to carrying no
    /// colour rather than to some arbitrary default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub color: Option<TerminalColor>,
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
