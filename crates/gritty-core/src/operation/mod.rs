//! The standard asynchronous operation model (addendum §3).
//!
//! ```text
//! UI command → fast validation → operation created → operationId returned
//! → supervised background execution → progress events → COMPLETED | FAILED | CANCELLED
//! ```
//!
//! [`Operation`] is the snapshot the UI sees (`operations.list()` and every
//! `Operation*` event). The state machine lives in [`state`], execution and
//! cancellation in [`registry`], progress throttling in [`progress`], and the
//! demo operation used by tests and the dev panel in [`demo`].

pub mod demo;
pub mod progress;
pub mod registry;
pub mod state;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::{CorrelationId, OperationId, WorkspaceId};
use crate::protocol::time::Timestamp;

pub use registry::{
    CancelOperationRequest, OperationContext, OperationRegistry, OperationResult, OperationSpec,
    ProgressReporter,
};
pub use state::{InvalidTransition, OperationStatus};

/// What an operation does, named like commands: `<domain>.<verb>` (`"operations.demo"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub struct OperationKind(String);

impl OperationKind {
    pub fn new(kind: impl Into<String>) -> Self {
        Self(kind.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Progress of a running operation. `total` is absent when unknown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OperationProgress {
    pub current: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub message: Option<String>,
}

/// Snapshot of an operation. Besides the addendum §3.1 fields it carries the
/// latest `progress` and, once failed, the `error`, so `operations.list()`
/// can rehydrate the UI after a renderer reload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub id: OperationId,
    pub kind: OperationKind,
    pub actor: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workspace_id: Option<WorkspaceId>,
    pub status: OperationStatus,
    pub created_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub started_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub finished_at: Option<Timestamp>,
    /// Set as soon as `operations.cancel` was accepted; the status follows once the
    /// work observed the cancellation.
    pub cancellation_requested: bool,
    pub correlation_id: CorrelationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub progress: Option<OperationProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<CoreError>,
    /// Recorded in the session database so a process death leaves it
    /// `INTERRUPTED` (T060). Core-side only: not on the wire.
    #[serde(skip)]
    #[ts(skip)]
    pub durable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_serializes_camel_case_and_omits_absent_fields() {
        let op = Operation {
            id: OperationId::from_raw("op-1"),
            kind: OperationKind::new("operations.demo"),
            actor: Actor::User,
            workspace_id: None,
            status: OperationStatus::Created,
            created_at: Timestamp::from_millis(1),
            started_at: None,
            finished_at: None,
            cancellation_requested: false,
            correlation_id: CorrelationId::from_raw("corr-1"),
            progress: None,
            error: None,
            durable: true,
        };
        let json = serde_json::to_value(&op).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "op-1",
                "kind": "operations.demo",
                "actor": { "type": "USER" },
                "status": "CREATED",
                "createdAt": 1,
                "cancellationRequested": false,
                "correlationId": "corr-1",
            })
        );
        let back: Operation = serde_json::from_value(json).unwrap();
        assert!(!back.durable, "durable never crosses the wire");
        assert_eq!(
            back,
            Operation {
                durable: false,
                ..op
            }
        );
    }
}
