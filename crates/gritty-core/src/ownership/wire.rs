//! Wire types of the `ownership.*` domain (T325), exported to TypeScript by
//! `cargo test -p gritty-core`.
//!
//! A lease is held by a **workstream**, never by an agent session. Hard rule 6
//! keeps provider concepts out of the domain, and `docs/architecture/agent-sessions.md`
//! already argues that a durable row must not carry an id that only exists
//! while a process does. [`OwnershipLease::agent_run_id`] is the seam T300
//! attaches a run to — the same move T200 made with `Decision::raised_by`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::error::CoreError;
use crate::protocol::ids::{AgentRunId, ConflictId, LeaseId, WorkspaceId, WorkstreamId};
use crate::protocol::time::Timestamp;

pub use super::target::{MAX_PATTERN, MAX_PATTERN_SEGMENTS, OwnershipTarget};

/// Longest accepted note on a lease.
pub const MAX_NOTE: usize = 200;
/// Most leases one workspace may hold at once. A registry nobody can read is
/// not coordination, and an unbounded one is a way to make the map useless.
pub const MAX_LEASES: usize = 200;
/// Most conflicts returned in one map, newest first.
pub const MAX_CONFLICTS: usize = 200;

/// What a lease permits. Only writes are coordinated: two workstreams reading
/// one file is not a collision, and a read lease nothing enforces would be
/// decoration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OwnershipMode {
    Write,
}

impl OwnershipMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Write => "WRITE",
        }
    }
}

/// Where a lease has got to.
///
/// `RELEASED` is stored. `EXPIRED` is **derived on read** from `expires_at`,
/// never written back: a state that a clock decides cannot be allowed to
/// disagree with the clock. Startup reconciliation (§21 of the coordination
/// plan) is therefore continuous rather than a pass that can be skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LeaseState {
    Active,
    Released,
    Expired,
}

impl LeaseState {
    pub const ALL: &'static [Self] = &[Self::Active, Self::Released, Self::Expired];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "ACTIVE",
            Self::Released => "RELEASED",
            Self::Expired => "EXPIRED",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        Self::ALL
            .iter()
            .copied()
            .find(|state| state.as_str() == raw)
            .ok_or_else(|| CoreError::internal(format!("unknown lease state {raw:?}")))
    }

    /// A lease that still keeps others out.
    pub const fn holds(self) -> bool {
        matches!(self, Self::Active)
    }
}

/// One workstream's claim on part of the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipLease {
    pub id: LeaseId,
    /// The workspace it belongs to, as it is open right now.
    pub workspace_id: WorkspaceId,
    pub workstream_id: WorkstreamId,
    /// The run that claimed it. Always absent today: nothing constructs an
    /// [`AgentRunId`] until Gritty owns runs (T300). It is on the wire now so
    /// that adding runs is a value, not a shape change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent_run_id: Option<AgentRunId>,
    pub target: OwnershipTarget,
    pub mode: OwnershipMode,
    /// Why this scope, in the holder's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
    pub acquired_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub expires_at: Option<Timestamp>,
    pub state: LeaseState,
    /// The holding workstream is finished or abandoned, so this lease is
    /// keeping others out of work nobody is doing. Derived on read, and
    /// reported rather than repaired: ownership is never taken away silently,
    /// but it is never invisible either.
    pub stale: bool,
}

/// Where a refusal — or a violation — has got to.
///
/// Two of these are *open*: [`Self::Waiting`] and [`Self::Observed`], which is
/// what [`Self::is_open`] answers and what the attention queue draws. The other
/// two are the record of what was decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConflictState {
    /// Still blocking; this is what reaches the attention queue.
    Waiting,
    /// The lease it was waiting on was released. The requester may ask again —
    /// nothing is granted automatically, because a lease handed out by a race
    /// nobody watched is worse than a second click.
    Resolved,
    /// The requester gave up.
    Cancelled,
    /// The user took the lease off its holder, deliberately and on the record.
    Overridden,
    /// A write that already landed on somebody else's ground (T326).
    ///
    /// The other four describe a request that was *refused before* the work
    /// started. This one points the other way in time: nothing was refused,
    /// nothing was blocked, and the file was written. It is a state on this row
    /// rather than a table of its own because "someone is in someone else's
    /// way" is one shape, and the queue, the island and the panel already read
    /// it — T326 §§6-7.
    ///
    /// `requested_by` is the workstream whose session did the writing, and
    /// `target` is the one file it touched, never the pattern the holder
    /// claimed.
    Observed,
}

impl ConflictState {
    pub const ALL: &'static [Self] = &[
        Self::Waiting,
        Self::Resolved,
        Self::Cancelled,
        Self::Overridden,
        Self::Observed,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "WAITING",
            Self::Resolved => "RESOLVED",
            Self::Cancelled => "CANCELLED",
            Self::Overridden => "OVERRIDDEN",
            Self::Observed => "OBSERVED",
        }
    }

    /// Whether the user has still to deal with this one.
    ///
    /// Both open states are closed by the same paths — the lease is released,
    /// it is overridden, or the user dismisses the row — which is why
    /// `close_conflict` and `close_conflicts_for_lease` match on both.
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Waiting | Self::Observed)
    }

    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        Self::ALL
            .iter()
            .copied()
            .find(|state| state.as_str() == raw)
            .ok_or_else(|| CoreError::internal(format!("unknown conflict state {raw:?}")))
    }
}

/// A request for ownership that was refused, and by whom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipConflict {
    pub id: ConflictId,
    pub workspace_id: WorkspaceId,
    /// The workstream that asked.
    pub requested_by: WorkstreamId,
    /// The workstream that already holds overlapping ground.
    pub owned_by: WorkstreamId,
    /// The lease in the way. It may since have been released; `state` says so.
    pub holding_lease_id: LeaseId,
    /// What was asked for, not what is held — they overlap, they are rarely
    /// the same shape.
    pub target: OwnershipTarget,
    pub created_at: Timestamp,
    pub state: ConflictState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub resolved_at: Option<Timestamp>,
}

/// Who owns what in one workspace, in one snapshot.
///
/// One shape for the command response and the event payload, so two surfaces
/// reading the same controller cannot disagree — T210's rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipMap {
    /// Leases that still hold, plus expired ones so the user can see and clear
    /// them. Released leases are history and are not carried.
    pub leases: Vec<OwnershipLease>,
    /// Newest first, every state, capped at [`MAX_CONFLICTS`]. The queue shows
    /// the `WAITING` ones; the rest are the record of what happened.
    pub conflicts: Vec<OwnershipConflict>,
}

/// The answer to `ownership.acquire`: granted, or refused with a reason. Never
/// both, and never a silent success.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OwnershipAcquisition {
    Granted { lease: OwnershipLease },
    Conflicted { conflict: OwnershipConflict },
}

/// The answer to `ownership.check`: who owns this ground right now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipCheck {
    pub target: OwnershipTarget,
    /// Every lease that overlaps, whichever workstream holds it. Empty means
    /// the ground is free.
    pub holders: Vec<OwnershipLease>,
}

/// What to do about a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConflictResolution {
    /// The requester withdraws. Nothing else changes.
    Cancel,
    /// Release the holder's lease. Coding-agent rule 7 forbids overriding
    /// *silently*, not at all: this is a user action, it is attributed by the
    /// event's actor, and it still does not grant anything — the requester
    /// asks again.
    Override,
}

impl ConflictResolution {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cancel => "CANCEL",
            Self::Override => "OVERRIDE",
        }
    }
}

/// `ownership.list`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ListOwnershipRequest {
    pub workspace_id: WorkspaceId,
}

/// `ownership.acquire`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AcquireOwnershipRequest {
    pub workspace_id: WorkspaceId,
    pub workstream_id: WorkstreamId,
    pub target: OwnershipTarget,
    /// When the lease should stop holding. Absent means it holds until it is
    /// released — which is why `stale` exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub expires_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}

/// `ownership.release`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseOwnershipRequest {
    pub workspace_id: WorkspaceId,
    pub lease_id: LeaseId,
}

/// `ownership.check`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CheckOwnershipRequest {
    pub workspace_id: WorkspaceId,
    pub target: OwnershipTarget,
}

/// `ownership.resolve`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResolveConflictRequest {
    pub workspace_id: WorkspaceId,
    pub conflict_id: ConflictId,
    pub action: ConflictResolution,
}

/// Trim, reject over-long, treat blank as absent.
pub(crate) fn optional_note(value: Option<&str>) -> Result<Option<String>, CoreError> {
    match value.map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) => {
            if text.chars().count() > MAX_NOTE {
                return Err(CoreError::invalid_input(
                    "note",
                    format!("must be at most {MAX_NOTE} characters"),
                ));
            }
            Ok(Some(text.to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::error::CoreErrorCode;
    use gritty_workspace::WorkspacePath;

    fn lease() -> OwnershipLease {
        OwnershipLease {
            id: LeaseId::from_raw("l-1"),
            workspace_id: WorkspaceId::from_raw("w-1"),
            workstream_id: WorkstreamId::from_raw("ws-1"),
            agent_run_id: None,
            target: OwnershipTarget::Directory {
                path: WorkspacePath::parse("src/auth").unwrap(),
            },
            mode: OwnershipMode::Write,
            note: None,
            acquired_at: Timestamp::from_millis(1),
            expires_at: None,
            state: LeaseState::Active,
            stale: false,
        }
    }

    #[test]
    fn states_round_trip_through_storage_and_the_wire() {
        for &state in LeaseState::ALL {
            assert_eq!(LeaseState::parse(state.as_str()).unwrap(), state);
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::json!(state.as_str())
            );
        }
        for &state in ConflictState::ALL {
            assert_eq!(ConflictState::parse(state.as_str()).unwrap(), state);
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::json!(state.as_str())
            );
        }
        assert_eq!(
            LeaseState::parse("HELD").unwrap_err().code,
            CoreErrorCode::Internal
        );
        assert_eq!(
            ConflictState::parse("QUEUED").unwrap_err().code,
            CoreErrorCode::Internal
        );
    }

    #[test]
    fn only_an_active_lease_keeps_others_out() {
        assert!(LeaseState::Active.holds());
        assert!(!LeaseState::Released.holds());
        assert!(!LeaseState::Expired.holds());
    }

    #[test]
    fn a_lease_omits_absent_fields_and_uses_camel_case() {
        let json = serde_json::to_value(lease()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "l-1",
                "workspaceId": "w-1",
                "workstreamId": "ws-1",
                "target": { "type": "DIRECTORY", "path": "src/auth" },
                "mode": "WRITE",
                "acquiredAt": 1,
                "state": "ACTIVE",
                "stale": false,
            })
        );
    }

    #[test]
    fn an_acquisition_is_granted_or_conflicted_never_both() {
        let json = serde_json::to_value(OwnershipAcquisition::Granted { lease: lease() }).unwrap();
        assert_eq!(json["type"], "GRANTED");
        assert!(json.get("conflict").is_none());
    }

    #[test]
    fn notes_are_trimmed_bounded_and_blank_means_absent() {
        assert_eq!(optional_note(None).unwrap(), None);
        assert_eq!(optional_note(Some("  ")).unwrap(), None);
        assert_eq!(
            optional_note(Some("  the auth refactor ")).unwrap(),
            Some("the auth refactor".to_owned())
        );
        let long = "n".repeat(MAX_NOTE + 1);
        assert_eq!(
            optional_note(Some(&long)).unwrap_err().code,
            CoreErrorCode::InvalidInput
        );
    }
}
