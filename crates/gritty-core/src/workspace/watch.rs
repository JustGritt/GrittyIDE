//! The open workspace's filesystem watch (T070, ADR-025), wired to the event
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
//!   closing logic);
//! - a watch that cannot cover the whole tree records why, answers
//!   `workspace.watchStatus` with it, and announces each new reason once as
//!   `WorkspaceWatchStatusChanged` (WATCH-DEGRADED, amending ADR-025).
//!
//! What this module deliberately does not do is decide anything about
//! documents. The watcher never says `MODIFIED_EXTERNALLY`; it makes the
//! on-demand check ADR-011 already specifies fire sooner, through the same
//! `documents.status` path. Two checkers that could disagree is exactly the
//! silent resolution hard rule 7 forbids.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use gritty_workspace::{
    Degradation, WatchConfig, WatchEvent, WatchStrategy, WorkspacePath, WorkspaceRoot,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::{Workspace, WorkspaceManager};
use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::{CorrelationId, WorkspaceId};

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

/// Why part of the open workspace is not being watched (WATCH-DEGRADED).
///
/// A reason, never a count: the registration walk stops at its limit without
/// knowing how many directories it did not reach, so a number would be a lower
/// bound presented as a total. Whatever the reason, what is not watched behaves
/// as it did before T070 — correct on demand, just not proactive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WatchDegradedReason {
    /// The watch could not start at all: nothing is watched.
    BackendUnavailable,
    /// The platform refused to watch some directories (on Linux, the per-user
    /// inotify budget). The ones it accepted keep reporting.
    RegistrationRefused,
    /// The tree has more directories than a watch registers; the rest is not
    /// watched.
    DirectoryLimit,
    /// The platform's watch reported an error while running.
    BackendError,
}

impl From<Degradation> for WatchDegradedReason {
    fn from(degradation: Degradation) -> Self {
        match degradation {
            Degradation::RegistrationRefused => Self::RegistrationRefused,
            Degradation::DirectoryLimit => Self::DirectoryLimit,
            Degradation::BackendError => Self::BackendError,
        }
    }
}

/// How well the open workspace is watched: the answer to
/// `workspace.watchStatus`, and the payload of `WorkspaceWatchStatusChanged`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceWatchStatus {
    pub workspace_id: WorkspaceId,
    /// Every reason the watch is degraded, sorted, each at most once. It only
    /// grows while the workspace is open. Empty means the whole tree is
    /// watched, as far as the watch can tell.
    pub degraded: Vec<WatchDegradedReason>,
}

/// `workspace.watchStatus`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WatchStatusRequest {
    pub workspace_id: WorkspaceId,
}

/// The degradations of the one open watch, keyed by the workspace they belong
/// to so a report from a watch that is already gone cannot land on the next.
#[derive(Debug, Default)]
pub(super) struct WatchHealth {
    workspace_id: Option<WorkspaceId>,
    degraded: BTreeSet<WatchDegradedReason>,
}

impl WatchHealth {
    /// Record `reason` for `workspace_id` and emit the whole status, when that
    /// reason is new for the watch that is current. Recorded before it is
    /// emitted, so a `workspace.watchStatus` asked after subscribing never
    /// misses what the event said.
    pub(super) fn report(
        health: &Mutex<Self>,
        events: &EventBus,
        workspace_id: &WorkspaceId,
        reason: WatchDegradedReason,
    ) {
        let status = {
            let mut held = health.lock().unwrap_or_else(PoisonError::into_inner);
            if held.workspace_id.as_ref() != Some(workspace_id) || !held.degraded.insert(reason) {
                return;
            }
            WorkspaceWatchStatus {
                workspace_id: workspace_id.clone(),
                degraded: held.degraded.iter().copied().collect(),
            }
        };
        tracing::warn!(
            workspace = %workspace_id,
            ?reason,
            "the workspace is not fully watched; changes outside Gritty may go unnoticed there"
        );
        events.emit(
            &EventOrigin {
                workspace_id: Some(workspace_id.clone()),
                // Nothing the user asked for caused this.
                correlation_id: None,
                actor: Actor::System,
            },
            CoreEvent::WorkspaceWatchStatusChanged(status),
        );
    }
}

/// Test seam: how `start_watch` watches, instead of the platform default.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(super) enum WatchSetup {
    With(WatchConfig, WatchStrategy),
    /// Behave as a backend that refuses to start.
    Refuse,
}

impl WorkspaceManager {
    /// `workspace.watchStatus`: how well the open workspace is watched.
    /// `NotFound` when `workspace_id` is not the open workspace.
    pub fn watch_status(
        &self,
        request: WatchStatusRequest,
        correlation: &CorrelationId,
    ) -> Result<WorkspaceWatchStatus, CoreError> {
        let (workspace, _root) = self.require(&request.workspace_id, correlation)?;
        let held = self
            .watch_health
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let degraded = if held.workspace_id.as_ref() == Some(&workspace.id) {
            held.degraded.iter().copied().collect()
        } else {
            Vec::new()
        };
        Ok(WorkspaceWatchStatus {
            workspace_id: workspace.id,
            degraded,
        })
    }

    /// Start watching `root` on behalf of `workspace`.
    ///
    /// A backend that refuses to start is logged and recorded as
    /// `BACKEND_UNAVAILABLE`: the workspace still opens, and every on-demand
    /// check stays exactly as correct as it was before T070 — just not
    /// proactive, and now not silently so.
    pub(super) fn start_watch(
        &self,
        workspace: &Workspace,
        root: &WorkspaceRoot,
    ) -> Option<gritty_workspace::Watch> {
        {
            let mut held = self
                .watch_health
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            *held = WatchHealth {
                workspace_id: Some(workspace.id.clone()),
                degraded: BTreeSet::new(),
            };
        }
        let events = self.events.clone();
        let manager = self.me.clone();
        let health = Arc::clone(&self.watch_health);
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
            WatchEvent::Degraded(degradation) => {
                WatchHealth::report(&health, &events, &workspace_id, degradation.into());
            }
        };
        #[cfg(not(test))]
        let started = gritty_workspace::watch(
            root,
            WatchConfig::default(),
            WatchStrategy::for_platform(),
            sink,
        );
        #[cfg(test)]
        let started = match *self
            .watch_setup
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
        {
            Some(WatchSetup::Refuse) => Err(gritty_workspace::WorkspaceError::Io {
                context: "a test's refusing backend".to_owned(),
                source: std::io::Error::other("refused"),
            }),
            Some(WatchSetup::With(config, strategy)) => {
                gritty_workspace::watch(root, config, strategy, sink)
            }
            None => gritty_workspace::watch(
                root,
                WatchConfig::default(),
                WatchStrategy::for_platform(),
                sink,
            ),
        };
        match started {
            Ok(watch) => Some(watch),
            Err(error) => {
                tracing::warn!(
                    workspace = %workspace.id,
                    root = %workspace.root,
                    %error,
                    "could not watch the workspace; external changes are only noticed on demand"
                );
                WatchHealth::report(
                    &self.watch_health,
                    &self.events,
                    &workspace.id,
                    WatchDegradedReason::BackendUnavailable,
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
