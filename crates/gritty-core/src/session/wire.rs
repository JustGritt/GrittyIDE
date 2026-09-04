//! Wire types of the `session.*` domain (T060), exported to TypeScript.
//!
//! Everything persisted is keyed by the workspace root and a `WorkspacePath`;
//! ids never cross a process boundary. `preferences` and `layout` are owned
//! by the UI: opaque JSON with a producer-side `version`, validated there.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use gritty_workspace::WorkspacePath;

use crate::protocol::ids::WorkspaceId;
use crate::protocol::time::Timestamp;
use crate::workspace::Workspace;

/// One-based line and column, as the editor reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TextPosition {
    pub line: u64,
    pub column: u64,
}

/// A persisted tab: where it is, what it shows, where the cursor was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionTab {
    pub path: WorkspacePath,
    #[serde(default)]
    pub active: bool,
    /// `source | split | preview`; absent means the UI default for the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub view_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub cursor: Option<TextPosition>,
    /// Selection anchor; equals `cursor` when nothing is selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub anchor: Option<TextPosition>,
    /// First visible line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub scroll_line: Option<u64>,
}

/// Whether the session database is usable right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PersistenceStatus {
    /// The database opened and migrated; `false` means the editor runs without persistence.
    pub available: bool,
    /// Writes are succeeding; `false` while the writer is retrying with backoff.
    pub healthy: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub database_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionNoticeKind {
    /// A damaged database was renamed and a fresh one created (`path` is the old file).
    DatabaseMovedAside,
    /// The database could not be opened or migrated; nothing is persisted.
    PersistenceUnavailable,
    /// The last workspace root no longer exists or could not be opened.
    WorkspaceRootMissing,
    /// Durable operations from the previous process were marked `INTERRUPTED`.
    OperationsInterrupted,
}

/// Something the user should know about the session layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionNotice {
    pub kind: SessionNoticeKind,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<String>,
}

/// `session.restore`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SessionRestoreResponse {
    /// The open workspace: the one already open, or the last one reopened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace: Option<Workspace>,
    /// Persisted tabs of that workspace, in order.
    pub tabs: Vec<SessionTab>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub preferences: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub layout: Option<serde_json::Value>,
    pub persistence: PersistenceStatus,
    pub notices: Vec<SessionNotice>,
}

/// How a recovery snapshot relates to the file on disk right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryState {
    /// The file still has the content the snapshot was derived from.
    SameAsDisk,
    /// The file changed since; recovering opens the document as `CONFLICTED`.
    DiskChanged,
    /// The file is gone; recovering opens it as `DELETED_EXTERNALLY`.
    FileMissing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCandidate {
    pub path: WorkspacePath,
    pub state: RecoveryState,
    pub updated_at: Timestamp,
    /// UTF-8 byte length of the snapshot.
    pub size: u64,
}

/// `session.candidates`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CandidatesRequest {
    pub workspace_id: WorkspaceId,
}

/// `session.recover`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecoverSnapshotRequest {
    pub workspace_id: WorkspaceId,
    pub path: WorkspacePath,
}

/// `session.discard`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DiscardSnapshotRequest {
    pub workspace_id: WorkspaceId,
    pub path: WorkspacePath,
}

/// `session.update`: what the UI owns. `tabs` needs `workspaceId`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSessionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_id: Option<WorkspaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tabs: Option<Vec<SessionTab>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub preferences: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub layout: Option<serde_json::Value>,
}

/// Payload of `PersistenceStatusChanged`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PersistenceStatusPayload {
    pub status: PersistenceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub notice: Option<SessionNotice>,
}
