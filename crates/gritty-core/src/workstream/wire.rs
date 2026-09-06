//! Wire types of the `workstreams.*` and `decisions.*` domains (T200),
//! exported to TypeScript by `cargo test -p gritty-core`.
//!
//! Unlike the session tables (T060), which key everything by the workspace
//! root because `WorkspaceId` is per process, a [`WorkstreamId`] is persisted
//! as-is: a workstream is durable domain state, not a handle to something
//! open. The workspace is still addressed by its per-process id, so every
//! request goes through `WorkspaceManager::require` and a command cannot touch
//! the workstreams of a workspace that is not open.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::protocol::actor::Actor;
use crate::protocol::error::CoreError;
use crate::protocol::ids::{DecisionId, WorkspaceId, WorkstreamId};
use crate::protocol::time::Timestamp;
use crate::workstream::state::WorkstreamState;

/// Longest accepted title. Long enough for a task line, short enough to render
/// in the island without truncation deciding what the user sees.
pub const MAX_TITLE: usize = 200;
/// Longest accepted goal, question or rationale.
pub const MAX_PROSE: usize = 4_000;
/// Most options one decision may offer.
pub const MAX_OPTIONS: usize = 12;
/// Longest accepted option key and label.
pub const MAX_OPTION_KEY: usize = 64;
pub const MAX_OPTION_LABEL: usize = 200;

/// A unit of work: what it is for, where it has got to, and how much of it is
/// waiting on the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Workstream {
    pub id: WorkstreamId,
    /// The workspace it belongs to, as it is open right now.
    pub workspace_id: WorkspaceId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub goal: Option<String>,
    pub state: WorkstreamState,
    /// Where `state` may go next. Derived from the same table the core checks
    /// against, so a UI can offer exactly the legal moves instead of keeping
    /// its own copy of the machine and drifting from it.
    pub allowed_transitions: Vec<WorkstreamState>,
    /// Decisions still `OPEN`. Derived on read, never stored: the decisions
    /// table is the truth.
    pub open_decisions: u32,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// One answer a decision offers. Free-form questions carry no options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DecisionOption {
    /// Stable identifier of the choice; what `chosen` records.
    pub key: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DecisionState {
    /// Waiting for the user.
    Open,
    /// Answered; `chosen` says how.
    Resolved,
    /// Set aside without an answer — the question stopped mattering.
    Dismissed,
}

impl DecisionState {
    pub const ALL: &'static [Self] = &[Self::Open, Self::Resolved, Self::Dismissed];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "OPEN",
            Self::Resolved => "RESOLVED",
            Self::Dismissed => "DISMISSED",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        Self::ALL
            .iter()
            .copied()
            .find(|state| state.as_str() == raw)
            .ok_or_else(|| CoreError::internal(format!("unknown decision state {raw:?}")))
    }
}

/// A question that has to be answered before the work can go on.
///
/// Raised by the user today; `raised_by` is an [`Actor`], so an agent run
/// (T335) raises the same thing without changing this shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub id: DecisionId,
    pub workstream_id: WorkstreamId,
    pub question: String,
    /// Empty for a free-form question.
    #[serde(default)]
    pub options: Vec<DecisionOption>,
    pub state: DecisionState,
    /// The option key that was picked, or free text when there were no options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub chosen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rationale: Option<String>,
    pub raised_by: Actor,
    pub raised_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub resolved_at: Option<Timestamp>,
}

/// `workstreams.list`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ListWorkstreamsRequest {
    pub workspace_id: WorkspaceId,
}

/// `workstreams.create`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkstreamRequest {
    pub workspace_id: WorkspaceId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub goal: Option<String>,
}

/// `workstreams.update`. An absent field is left alone; a present blank goal
/// clears it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkstreamRequest {
    pub workspace_id: WorkspaceId,
    pub workstream_id: WorkstreamId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub goal: Option<String>,
}

/// `workstreams.transition`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TransitionWorkstreamRequest {
    pub workspace_id: WorkspaceId,
    pub workstream_id: WorkstreamId,
    pub to: WorkstreamState,
}

/// `workstreams.remove`. Deletes the workstream and its decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RemoveWorkstreamRequest {
    pub workspace_id: WorkspaceId,
    pub workstream_id: WorkstreamId,
}

/// `decisions.list`. Without `workstreamId`, every decision of the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ListDecisionsRequest {
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workstream_id: Option<WorkstreamId>,
    /// Only decisions still `OPEN`. Absent means all of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub open_only: Option<bool>,
}

/// `decisions.raise`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RaiseDecisionRequest {
    pub workspace_id: WorkspaceId,
    pub workstream_id: WorkstreamId,
    pub question: String,
    #[serde(default)]
    pub options: Vec<DecisionOption>,
}

/// `decisions.resolve`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResolveDecisionRequest {
    pub workspace_id: WorkspaceId,
    pub decision_id: DecisionId,
    /// An option key when the decision offered options, free text otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub chosen: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rationale: Option<String>,
}

/// `decisions.dismiss`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DismissDecisionRequest {
    pub workspace_id: WorkspaceId,
    pub decision_id: DecisionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamCreatedPayload {
    pub workstream: Workstream,
}

/// The title, goal, state or open-decision count changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamChangedPayload {
    pub workstream: Workstream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamRemovedPayload {
    pub workstream_id: WorkstreamId,
    /// The title it had, so the UI can say what disappeared.
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DecisionRequestedPayload {
    pub decision: Decision,
}

/// Answered or dismissed; `decision.state` says which.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DecisionResolvedPayload {
    pub decision: Decision,
}

/// Trim, reject blank, reject over-long. Returns the trimmed value.
pub(crate) fn required_text(field: &str, value: &str, max: usize) -> Result<String, CoreError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(CoreError::invalid_input(field, "must not be blank"));
    }
    if trimmed.chars().count() > max {
        return Err(CoreError::invalid_input(
            field,
            format!("must be at most {max} characters"),
        ));
    }
    Ok(trimmed.to_owned())
}

/// Same, but a blank value means "no value" rather than an error.
pub(crate) fn optional_text(
    field: &str,
    value: Option<&str>,
    max: usize,
) -> Result<Option<String>, CoreError> {
    match value.map(str::trim) {
        None => Ok(None),
        Some("") => Ok(None),
        Some(text) => Ok(Some(required_text(field, text, max)?)),
    }
}

/// Options must be unique by key and individually sane.
pub(crate) fn validate_options(
    options: &[DecisionOption],
) -> Result<Vec<DecisionOption>, CoreError> {
    if options.len() > MAX_OPTIONS {
        return Err(CoreError::invalid_input(
            "options",
            format!("must be at most {MAX_OPTIONS} options"),
        ));
    }
    let mut validated: Vec<DecisionOption> = Vec::with_capacity(options.len());
    for option in options {
        let key = required_text("options[].key", &option.key, MAX_OPTION_KEY)?;
        let label = required_text("options[].label", &option.label, MAX_OPTION_LABEL)?;
        if validated.iter().any(|existing| existing.key == key) {
            return Err(CoreError::invalid_input(
                "options[].key",
                format!("duplicate option key {key:?}"),
            ));
        }
        validated.push(DecisionOption { key, label });
    }
    Ok(validated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::error::CoreErrorCode;

    #[test]
    fn decision_states_round_trip() {
        for &state in DecisionState::ALL {
            assert_eq!(DecisionState::parse(state.as_str()).unwrap(), state);
            assert_eq!(
                serde_json::to_value(state).unwrap(),
                serde_json::json!(state.as_str())
            );
        }
        assert_eq!(
            DecisionState::parse("PENDING").unwrap_err().code,
            CoreErrorCode::Internal
        );
    }

    #[test]
    fn required_text_trims_and_bounds() {
        assert_eq!(required_text("title", "  hello  ", 10).unwrap(), "hello");
        assert_eq!(
            required_text("title", "   ", 10).unwrap_err().code,
            CoreErrorCode::InvalidInput
        );
        let error = required_text("title", "abcdefghijk", 10).unwrap_err();
        assert_eq!(error.details.unwrap()["field"], "title");
    }

    #[test]
    fn required_text_counts_characters_not_bytes() {
        // Ten emoji are ten characters, not forty bytes.
        assert!(required_text("title", &"🙂".repeat(10), 10).is_ok());
        assert!(required_text("title", &"🙂".repeat(11), 10).is_err());
    }

    #[test]
    fn optional_text_treats_blank_as_absent() {
        assert_eq!(optional_text("goal", None, 10).unwrap(), None);
        assert_eq!(optional_text("goal", Some("  "), 10).unwrap(), None);
        assert_eq!(
            optional_text("goal", Some(" x "), 10).unwrap(),
            Some("x".to_owned())
        );
    }

    #[test]
    fn options_reject_duplicates_and_overflow() {
        let ok = validate_options(&[
            DecisionOption {
                key: " a ".into(),
                label: "A".into(),
            },
            DecisionOption {
                key: "b".into(),
                label: "B".into(),
            },
        ])
        .unwrap();
        assert_eq!(ok[0].key, "a");

        let duplicate = validate_options(&[
            DecisionOption {
                key: "a".into(),
                label: "A".into(),
            },
            DecisionOption {
                key: "a".into(),
                label: "Again".into(),
            },
        ])
        .unwrap_err();
        assert_eq!(duplicate.code, CoreErrorCode::InvalidInput);

        let too_many: Vec<DecisionOption> = (0..=MAX_OPTIONS)
            .map(|index| DecisionOption {
                key: index.to_string(),
                label: index.to_string(),
            })
            .collect();
        assert_eq!(
            validate_options(&too_many).unwrap_err().code,
            CoreErrorCode::InvalidInput
        );
    }

    #[test]
    fn workstream_omits_absent_fields_and_uses_camel_case() {
        let workstream = Workstream {
            id: WorkstreamId::from_raw("ws-1"),
            workspace_id: WorkspaceId::from_raw("w-1"),
            title: "Terminal".to_owned(),
            goal: None,
            state: WorkstreamState::Draft,
            allowed_transitions: WorkstreamState::Draft.allowed_transitions().to_vec(),
            open_decisions: 0,
            created_at: Timestamp::from_millis(1),
            updated_at: Timestamp::from_millis(2),
        };
        let json = serde_json::to_value(&workstream).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "ws-1",
                "workspaceId": "w-1",
                "title": "Terminal",
                "state": "DRAFT",
                "allowedTransitions": ["READY", "ABANDONED"],
                "openDecisions": 0,
                "createdAt": 1,
                "updatedAt": 2,
            })
        );
    }
}
