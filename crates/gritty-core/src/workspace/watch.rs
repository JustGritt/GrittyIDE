//! The open workspace's filesystem watch (T070, ADR-024), wired to the event
//! bus.
//!
//! `gritty-workspace` owns the backend, the coalescing and the bounds; this
//! module owns only what the core owns everywhere else — identity, the event
//! envelope, and the decision about what a signal means:
//!
//! - a batch of changed directories becomes one `WorkspaceEntriesChanged`,
//!   attributed to `SYSTEM` and to no correlation id, because nothing the user
//!   asked for caused it;
//! - a vanished root triggers the *existing* `RootMissing` close path eagerly
//!   instead of waiting for the next command to notice it (hard rule 2: no new
//!   closing logic).
//!
//! What this module deliberately does not do is decide anything about
//! documents. The watcher never says `MODIFIED_EXTERNALLY`; it makes the
//! on-demand check ADR-011 already specifies fire sooner, through the same
//! `documents.status` path. Two checkers that could disagree is exactly the
//! silent resolution hard rule 7 forbids.

use gritty_workspace::{WatchConfig, WatchEvent, WatchStrategy, WorkspacePath, WorkspaceRoot};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{Workspace, WorkspaceManager};
use crate::event::{CoreEvent, EventOrigin};
use crate::protocol::actor::Actor;
use crate::protocol::ids::WorkspaceId;

/// Payload of `WorkspaceEntriesChanged`: what changed on disk under the open
/// workspace, coalesced and bounded.
///
/// Directories, never entries. That is the coarsest signal every backend can
/// give honestly — FSEvents coalesces by directory and cannot say which entry
/// moved across a rename — and it is what the explorer already keys its nodes
/// by. A consumer converges by re-listing what it holds, which is bounded by
/// what the user expanded rather than by the size of the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntriesChangedPayload {
    pub workspace_id: WorkspaceId,
    /// Workspace-relative directories whose entry set may have changed:
    /// deduplicated, sorted, and empty when `overflowed`. Names inside them
    /// are untrusted repository content and nothing is built from them here.
    pub directories: Vec<WorkspacePath>,
    /// More directories changed than one batch may name. The list is empty
    /// rather than partial: a consumer refreshes everything it has loaded
    /// instead of trusting an incomplete answer.
    pub overflowed: bool,
}

impl WorkspaceManager {
    /// Start watching `root` on behalf of `workspace`.
    ///
    /// A backend that refuses to start is logged and nothing else: the
    /// workspace still opens, and every on-demand check stays exactly as
    /// correct as it was before T070 — just not proactive.
    pub(super) fn start_watch(
        &self,
        workspace: &Workspace,
        root: &WorkspaceRoot,
    ) -> Option<gritty_workspace::Watch> {
        let events = self.events.clone();
        let manager = self.me.clone();
        let workspace_id = workspace.id.clone();
        let sink = move |event: WatchEvent| match event {
            WatchEvent::Changed(batch) => {
                tracing::debug!(
                    workspace = %workspace_id,
                    directories = batch.directories.len(),
                    overflowed = batch.overflowed,
                    "workspace entries changed"
                );
                events.emit(
                    &EventOrigin {
                        workspace_id: Some(workspace_id.clone()),
                        // Nothing the user asked for caused this.
                        correlation_id: None,
                        actor: Actor::System,
                    },
                    CoreEvent::WorkspaceEntriesChanged(WorkspaceEntriesChangedPayload {
                        workspace_id: workspace_id.clone(),
                        directories: batch.directories,
                        overflowed: batch.overflowed,
                    }),
                );
            }
            WatchEvent::RootMissing => {
                if let Some(manager) = manager.upgrade() {
                    manager.close_missing_root(&workspace_id);
                }
            }
        };
        match gritty_workspace::watch(
            root,
            WatchConfig::default(),
            WatchStrategy::for_platform(),
            sink,
        ) {
            Ok(watch) => Some(watch),
            Err(error) => {
                tracing::warn!(
                    workspace = %workspace.id,
                    root = %workspace.root,
                    %error,
                    "could not watch the workspace; external changes are only noticed on demand"
                );
                None
            }
        }
    }

    /// The watcher observed the root gone. Runs the same close every other
    /// caller runs — `require` and `current` reach it lazily — just without
    /// waiting for a command.
    fn close_missing_root(&self, id: &WorkspaceId) {
        let mut current = self.lock();
        let Some(open) = current.as_ref() else {
            return;
        };
        if open.workspace.id != *id {
            return;
        }
        // Confirm rather than trust: between the observation and this lock the
        // root may have come back, and a close that cannot be justified now
        // would be a close the user cannot explain either.
        if open.root.is_present() {
            return;
        }
        let gone = current.take().expect("checked above");
        self.emit_closed(
            &gone,
            super::WorkspaceClosedReason::RootMissing,
            Actor::System,
            None,
        );
    }
}
