//! Request shapes of the `runs.*` command surface (T300-CMD §16, T310-B).
//!
//! Seven commands, seven requests: `runs.startFake`, `runs.start`,
//! `runs.prompt`, `runs.answerPermission`, `runs.cancel`, `runs.list` and
//! `runs.events` each take one, and `core.health`
//! reports the protocol minor they were added in. Their responses are types
//! [`super::wire`] already carries — [`super::wire::AgentRun`] and
//! [`super::wire::AgentRunEventLog`] — so nothing here describes a *result*.
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
//! # The fiction keeps its own front door
//!
//! `runs.start` (T310-B) reaches a real provider; `runs.startFake` is unchanged
//! and still the only way to a scripted one. Folding them together would put a
//! fiction and a process behind one name, and the fake is the deterministic
//! path every other test depends on — so [`StartRunRequest::validate`] refuses
//! `FAKE` outright rather than quietly forwarding it.
//!
//! `runs.prompt` still resolves neither `BLOCKED` nor `NEEDS_DECISION` — one
//! stop at a time. A permission block is answered with a verdict, through
//! `runs.answerPermission` (T310-ANSWER); a decision has no command yet.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::wire::{AgentRunPermissionVerdict, AgentRunProvider};
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

/// Longest prompt accepted, in characters.
///
/// **Deliberately much larger than `MAX_TEXT`, and the difference is the
/// point.** `MAX_TEXT` (2000) bounds untrusted *adapter output* for display, so
/// clipping it costs a reader nothing. This bounds what a person typed, and it
/// is about to be sent to a model and paid for — silently truncating it would
/// be worse than refusing it, so the request refuses and the full text reaches
/// the child. The recorded `PROMPT_STARTED` is still clipped to `MAX_TEXT` on
/// the way into the ring, because that one is a preview.
pub const MAX_PROMPT_TEXT: usize = 16 * 1024;

/// `runs.start` — start a run against a real provider.
///
/// Separate from `StartFakeRunRequest`, which stays exactly as it is: folding a
/// scripted fiction and a real process behind one command name would make the
/// deterministic path every other test depends on indistinguishable from the
/// one that spends money.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StartRunRequest {
    pub workspace_id: WorkspaceId,
    pub workstream_id: WorkstreamId,
    /// Which provider to start. `FAKE` is refused here — it has its own
    /// command, and accepting it would give the fiction two front doors.
    pub provider: AgentRunProvider,
    /// The first thing to say to the agent. Absent starts a run that waits.
    ///
    /// It is *seeded into the run's inbox* before the run exists, so the first
    /// prompt and every later one travel the same path and the driver needs no
    /// special case for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub prompt: Option<String>,
}

impl StartRunRequest {
    /// Reject a provider that has no process behind it, and a prompt that is
    /// blank or oversized.
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.provider == AgentRunProvider::Fake {
            return Err(CoreError::invalid_input(
                "provider",
                "a fake run is started with runs.startFake, not runs.start",
            ));
        }
        if let Some(prompt) = self.prompt.as_deref() {
            validate_prompt(prompt)?;
        }
        Ok(())
    }
}

/// `runs.prompt` — say something to a run that is already running.
///
/// The one thing the run contract has never had. A prompt for a run that is not
/// accepting one is `Conflict` naming the state — never silently dropped — and
/// a caller that outruns the agent is refused rather than queued without limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PromptRunRequest {
    pub run_id: AgentRunId,
    pub text: String,
}

impl PromptRunRequest {
    pub fn validate(&self) -> Result<(), CoreError> {
        validate_prompt(&self.text)
    }
}

/// Shape only: blank is a mistake, oversized is refused rather than clipped.
fn validate_prompt(text: &str) -> Result<(), CoreError> {
    if text.trim().is_empty() {
        return Err(CoreError::invalid_input("text", "a prompt cannot be blank"));
    }
    if text.chars().count() > MAX_PROMPT_TEXT {
        return Err(CoreError::invalid_input(
            "text",
            format!("a prompt may be at most {MAX_PROMPT_TEXT} characters"),
        ));
    }
    Ok(())
}

/// Longest ask id accepted. The core mints a UUID (36 characters); anything
/// much longer did not come from it.
pub const MAX_ASK_ID: usize = 64;

/// `runs.answerPermission` — the user's verdict on the ask a run is stopped at
/// (T310-ANSWER).
///
/// Addressed by the run *and* the ask, so an answer given to a card that has
/// since been replaced by a newer ask cannot land on the newer one. An ask
/// that is no longer outstanding is not an error; the run comes back and says
/// so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AnswerRunPermissionRequest {
    pub run_id: AgentRunId,
    pub ask_id: String,
    pub verdict: AgentRunPermissionVerdict,
}

impl AnswerRunPermissionRequest {
    /// Shape only: a blank or oversized ask id cannot name an ask the core
    /// minted. Whether it names the *outstanding* one is the manager's answer.
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.ask_id.trim().is_empty() {
            return Err(CoreError::invalid_input(
                "askId",
                "an ask id cannot be blank",
            ));
        }
        if self.ask_id.chars().count() > MAX_ASK_ID {
            return Err(CoreError::invalid_input(
                "askId",
                format!("an ask id may be at most {MAX_ASK_ID} characters"),
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

    fn answer(ask_id: &str) -> AnswerRunPermissionRequest {
        AnswerRunPermissionRequest {
            run_id: AgentRunId::from_raw("run-1"),
            ask_id: ask_id.to_owned(),
            verdict: AgentRunPermissionVerdict::Deny,
        }
    }

    #[test]
    fn an_answer_names_its_run_its_ask_and_its_verdict_in_camel_case() {
        let value = serde_json::to_value(answer("ask-1")).expect("serialize");
        assert_eq!(
            value,
            serde_json::json!({ "runId": "run-1", "askId": "ask-1", "verdict": "DENY" })
        );
        assert!(answer("ask-1").validate().is_ok());
    }

    #[test]
    fn a_blank_or_oversized_ask_id_is_invalid_input() {
        for bad in ["", "   ", &"a".repeat(MAX_ASK_ID + 1)] {
            let error = answer(bad).validate().unwrap_err();
            assert_eq!(error.code, CoreErrorCode::InvalidInput);
            assert_eq!(error.details.expect("details")["field"], "askId");
        }
        // A UUID, which is what the core mints, fits.
        assert!(
            answer(&uuid::Uuid::now_v7().hyphenated().to_string())
                .validate()
                .is_ok()
        );
    }

    /// No remembered grant for a run (`T310-GRANTS`), so the observed side's
    /// middle answer is refused at the boundary rather than read as either
    /// of the other two.
    #[test]
    fn allow_for_run_is_not_an_answer_a_run_accepts() {
        let parsed = serde_json::from_value::<AnswerRunPermissionRequest>(serde_json::json!({
            "runId": "run-1", "askId": "ask-1", "verdict": "ALLOW_FOR_RUN"
        }));
        assert!(parsed.is_err());
    }

    #[test]
    fn an_omitted_script_is_omitted_from_the_json_rather_than_sent_as_null() {
        let value = serde_json::to_value(request(None)).expect("serialize");
        assert!(value.get("script").is_none(), "{value}");
    }

    #[test]
    fn a_request_without_a_script_still_deserializes() {
        let parsed: StartFakeRunRequest = serde_json::from_value(
            serde_json::json!({ "workspaceId": "w-1", "workstreamId": "wst-1" }),
        )
        .expect("deserialize");
        assert_eq!(parsed.script, None);
    }
}
