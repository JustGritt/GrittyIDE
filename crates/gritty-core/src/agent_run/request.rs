//! Request shapes of the `runs.*` command surface (T300-CMD, §16).
//!
//! Five commands, four requests: `runs.startFake`, `runs.cancel`, `runs.list`
//! and `runs.events` each take one, and `core.health` reports the protocol
//! minor they were added in. Their responses are types [`super::wire`] already
//! carries — [`super::wire::AgentRun`] and [`super::wire::AgentRunEventLog`] —
//! so nothing here describes a *result*.
//!
//! # Why these live beside the manager rather than in `wire.rs`
//!
//! `wire.rs` is the run **contract**: what a run is, what states it may hold,
//! what it may say. These four are the **command surface** that reaches it, and
//! they arrived a protocol minor later, from a different ticket. Keeping them
//! apart is the split `operation::registry`'s `CancelOperationRequest` and
//! `operation::demo`'s `StartDemoRequest` already make — a request lives next
//! to the thing that serves it — and it means the frozen contract stays one
//! file that a reader can hold in their head.
//!
//! # Nothing here starts a real provider
//!
//! There is deliberately no `runs.start`. [`super::wire::AgentRunProvider`] has
//! one variant, `FAKE`, so the only run that can exist is a scripted fiction,
//! and a run cannot touch a real process by construction until T310 adds the
//! second. There is likewise no command that *answers* a run: a run can reach
//! `BLOCKED` or `NEEDS_DECISION`, but turning that into something the user
//! replies to is T335's, and half of it built here would be redone there.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::error::CoreError;
use crate::protocol::ids::{AgentRunId, WorkspaceId, WorkstreamId};

/// Longest script name accepted.
///
/// Bounded for the reason every string on this surface is (rule 13): the name
/// reaches the core from the renderer, is looked up, and is quoted back in a
/// `NotFound` message when the lookup misses. A name longer than this is a bug
/// or an attack, not a preset — the judgement `MAX_TOOL` already makes about
/// tool names, at the same size and for the same reason.
pub const MAX_SCRIPT_NAME: usize = 64;

/// `runs.startFake` — start a scripted run against a workstream.
///
/// The workstream must exist (`NotFound` otherwise) but may be in any state:
/// whether it is a *sensible* target is a scheduling question, and scheduling
/// is T405's. `workspaceId` is carried explicitly rather than derived from the
/// workstream so a request that names a closed workspace is refused as
/// `NotFound` at the boundary, before anything is created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StartFakeRunRequest {
    pub workspace_id: WorkspaceId,
    pub workstream_id: WorkstreamId,
    /// Which of the adapter's scripts to run. Absent means its default.
    ///
    /// A name rather than a script: the steps themselves belong to
    /// `gritty-agent-fake`, and letting a caller post a program would make the
    /// renderer the author of what an "agent" does. An unknown name is
    /// `NotFound`, naming what was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub script: Option<String>,
}

impl StartFakeRunRequest {
    /// Reject a script name that is empty or oversized, before it is looked up.
    ///
    /// Shape only. Whether the name *exists* is the adapter's answer, not this
    /// type's — a request that passes here can still be `NotFound`.
    pub fn validate(&self) -> Result<(), CoreError> {
        let Some(script) = self.script.as_deref() else {
            return Ok(());
        };
        if script.trim().is_empty() {
            return Err(CoreError::invalid_input(
                "script",
                "a script name cannot be blank; omit it for the default script",
            ));
        }
        if script.chars().count() > MAX_SCRIPT_NAME {
            return Err(CoreError::invalid_input(
                "script",
                format!("a script name may be at most {MAX_SCRIPT_NAME} characters"),
            ));
        }
        Ok(())
    }
}

/// `runs.cancel` — ask a run to stop.
///
/// Cooperative and idempotent, like `operations.cancel`: an unknown id is
/// `NotFound`, a run already in a terminal state returns its snapshot
/// unchanged, and anything else has its token fired and ends `CANCELLED`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CancelRunRequest {
    pub run_id: AgentRunId,
}

/// `runs.list` — every run of one workspace, terminal ones included.
///
/// Scoped to a workspace rather than global because that is the boundary a run
/// is cancelled at: closing a workspace ends its runs, so a list that crossed
/// workspaces would be listing work that is already gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ListRunsRequest {
    pub workspace_id: WorkspaceId,
}

/// `runs.events` — one run's event log, for rehydration after a reload.
///
/// The T010 convention: a renderer that reloads asks for the state it missed
/// rather than replaying events it never received. The answer is an
/// [`super::wire::AgentRunEventLog`], which reports what the ring dropped
/// instead of hiding it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RunEventsRequest {
    pub run_id: AgentRunId,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::error::CoreErrorCode;

    fn request(script: Option<&str>) -> StartFakeRunRequest {
        StartFakeRunRequest {
            workspace_id: WorkspaceId::from_raw("w-1"),
            workstream_id: WorkstreamId::from_raw("wst-1"),
            script: script.map(str::to_owned),
        }
    }

    #[test]
    fn an_absent_script_is_the_default_script() {
        assert!(request(None).validate().is_ok());
    }

    #[test]
    fn a_named_script_passes_the_shape_check() {
        assert!(request(Some("permission-then-decision")).validate().is_ok());
    }

    #[test]
    fn a_blank_script_name_is_invalid_input() {
        let error = request(Some("   ")).validate().unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert_eq!(error.details.expect("details")["field"], "script");
    }

    #[test]
    fn an_oversized_script_name_is_invalid_input_and_says_the_limit() {
        let long = "s".repeat(MAX_SCRIPT_NAME + 1);
        let error = request(Some(&long)).validate().unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert!(
            error.message.contains(&MAX_SCRIPT_NAME.to_string()),
            "{error}"
        );
    }

    #[test]
    fn the_limit_counts_characters_rather_than_bytes() {
        // Every limit in this domain counts characters, so a line of emoji is
        // bounded the way a line of ASCII is (`wire.rs`'s rule, kept here).
        let emoji = "🙂".repeat(MAX_SCRIPT_NAME);
        assert!(request(Some(&emoji)).validate().is_ok());
        let one_too_many = "🙂".repeat(MAX_SCRIPT_NAME + 1);
        assert!(request(Some(&one_too_many)).validate().is_err());
    }

    #[test]
    fn every_request_is_camel_case_on_the_wire() {
        // The `rename_all` trap `AgentTimelineKind` and `exitCode` both hit:
        // a missing attribute is invisible until a client cannot read a field.
        let start = serde_json::to_value(request(Some("blocked"))).expect("serialize");
        assert!(start.get("workspaceId").is_some(), "{start}");
        assert!(start.get("workstreamId").is_some(), "{start}");
        assert!(start.get("script").is_some(), "{start}");

        let cancel = serde_json::to_value(CancelRunRequest {
            run_id: AgentRunId::from_raw("run-1"),
        })
        .expect("serialize");
        assert!(cancel.get("runId").is_some(), "{cancel}");

        let list = serde_json::to_value(ListRunsRequest {
            workspace_id: WorkspaceId::from_raw("w-1"),
        })
        .expect("serialize");
        assert!(list.get("workspaceId").is_some(), "{list}");

        let events = serde_json::to_value(RunEventsRequest {
            run_id: AgentRunId::from_raw("run-1"),
        })
        .expect("serialize");
        assert!(events.get("runId").is_some(), "{events}");
    }

    #[test]
    fn an_omitted_script_is_omitted_from_the_json_rather_than_sent_as_null() {
        let value = serde_json::to_value(request(None)).expect("serialize");
        assert!(value.get("script").is_none(), "{value}");
    }

    #[test]
    fn a_request_without_a_script_still_deserializes() {
        let parsed: StartFakeRunRequest =
            serde_json::from_value(serde_json::json!({ "workspaceId": "w-1", "workstreamId": "wst-1" }))
                .expect("deserialize");
        assert_eq!(parsed.script, None);
    }
}
