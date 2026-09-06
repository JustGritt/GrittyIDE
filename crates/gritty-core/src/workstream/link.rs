//! Linking an observed agent session to the work it is doing (T223).
//!
//! The roster (T220) answers *what is running on this machine*; the workstream
//! model (T200) answers *what the work is*. This is the join, and it is the
//! point where observability starts paying into the domain instead of sitting
//! beside it: a roster that knows which workstream a session belongs to stops
//! being a list of processes and becomes a view of the work.
//!
//! **Why a table and not a field on [`Workstream`](super::Workstream).** Three
//! reasons, in order of weight.
//!
//! 1. Hard rule 6. A `provider` on `Workstream` is precisely the
//!    provider-specific field the rule names, and `providerSessionId` is the
//!    same thing wearing a neutral name.
//! 2. Lifetimes disagree. A workstream is durable and survives the process; a
//!    discovered session is in memory, rediscovered each launch, and its id
//!    belongs to a process that may be long dead. Putting the ephemeral inside
//!    the durable would make every workstream read carry a claim that is
//!    usually stale.
//! 3. Cardinality. One workstream legitimately accumulates several sessions
//!    over its life — resumed, restarted, a second agent on the same goal —
//!    and a single field would silently mean *the most recent one*.
//!
//! What a link is **not**: it grants no control. A linked session is still one
//! Gritty does not own, so nothing here prompts, answers or cancels it, and a
//! `BLOCKED` session does not move its linked workstream to `BLOCKED` — that
//! coupling belongs to runs Gritty owns (T300/T330).
//!
//! Nothing links automatically. A heuristic that attached a session to a
//! workstream because their directories looked alike would write a false claim
//! into durable state; the user links, explicitly.

use std::sync::Arc;

use gritty_persistence::now_ms;
use gritty_persistence::store::{agent_links, workstreams};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::wire::required_text;
use super::{WorkstreamManager, conflict, ensure_in_workspace, from_write, require_row};
use crate::agent_session::wire::AgentProvider;
use crate::protocol::error::CoreError;
use crate::protocol::ids::{CorrelationId, WorkspaceId, WorkstreamId};
use crate::protocol::time::Timestamp;

/// Longest accepted session id. The roster's own ids are far shorter; this is
/// a bound on what a renderer may write into a durable row, not a format.
pub const MAX_SESSION_ID: usize = 200;

/// Most sessions one workstream may accumulate. Generous — resumed and
/// restarted sessions add up over a long piece of work — but bounded, because
/// a loop in the renderer must not be able to turn "link" into unbounded
/// writing.
pub const MAX_LINKS_PER_WORKSTREAM: i64 = 50;

/// One agent session attached to one workstream.
///
/// There is deliberately no session *snapshot* here. Whether the session is in
/// the roster right now is a question about the present, and the answer changes
/// between this read and the next one; the caller joins this against the roster
/// it already holds and renders "linked, not running" when there is no match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamAgentLink {
    pub workstream_id: WorkstreamId,
    pub provider: AgentProvider,
    /// The roster's `AgentSession.id`, exactly as it was reported. Opaque
    /// here: nothing resolves it, and a session that never returns keeps it.
    pub session_id: String,
    pub linked_at: Timestamp,
}

/// `workstreams.linkAgent`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LinkAgentRequest {
    pub workspace_id: WorkspaceId,
    pub workstream_id: WorkstreamId,
    pub provider: AgentProvider,
    pub session_id: String,
}

/// `workstreams.unlinkAgent`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnlinkAgentRequest {
    pub workspace_id: WorkspaceId,
    pub workstream_id: WorkstreamId,
    pub provider: AgentProvider,
    pub session_id: String,
}

/// `workstreams.agentLinks`. Without `workstreamId`, every link of the
/// workspace — which is what the island needs to draw the whole roster in one
/// pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ListAgentLinksRequest {
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub workstream_id: Option<WorkstreamId>,
}

/// What `linkAgent` and `unlinkAgent` answer: the workstream's links as they
/// are *after* the call, read back from the database.
///
/// The fresh set rather than an event (§14, T047's precedent): the two surfaces
/// that draw links — the island and the attention queue — share one controller,
/// so the caller of the mutation is the only consumer, and an event nothing
/// subscribes to is a contract to maintain for nobody.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AgentLinkChange {
    pub workstream_id: WorkstreamId,
    pub links: Vec<WorkstreamAgentLink>,
    /// Whether this call actually wrote anything. `false` for a duplicate link
    /// and for an unlink of something that was not there — both are no-ops that
    /// say so rather than failures, because both leave the state the caller
    /// asked for.
    pub changed: bool,
}

impl WorkstreamManager {
    /// `workstreams.linkAgent`.
    ///
    /// Linking a session id that is **not** in the roster is allowed. The
    /// roster is a snapshot of now; a link records a fact about the past, and
    /// refusing would make a link impossible to create for a session that ended
    /// two seconds ago.
    pub async fn link_agent(
        self: &Arc<Self>,
        request: LinkAgentRequest,
        correlation: CorrelationId,
    ) -> Result<AgentLinkChange, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let session_id = required_text("sessionId", &request.session_id, MAX_SESSION_ID)?;
        let provider = request.provider;
        let id = request.workstream_id.to_string();
        let workstream_id = request.workstream_id.clone();
        let now = now_ms();
        let (links, changed) = self
            .blocking(move |db| {
                db.write(|tx| {
                    let existing = require_row(&workstreams::get(tx, &id)?, &id)?;
                    ensure_in_workspace(&existing, &root)?;
                    let held = agent_links::count_for_workstream(tx, &id)?;
                    let changed = agent_links::insert(
                        tx,
                        &agent_links::AgentLinkRow {
                            workstream_id: id.clone(),
                            provider: provider.as_str().to_owned(),
                            session_id,
                            linked_at: now,
                        },
                    )?;
                    // Checked after the insert so that a *duplicate* at the
                    // bound stays the no-op it is, instead of being refused for
                    // asking for something that is already true. Returning an
                    // error here rolls the insert back with the transaction.
                    if changed && held >= MAX_LINKS_PER_WORKSTREAM {
                        return Err(conflict(format!(
                            "a workstream may be linked to at most {MAX_LINKS_PER_WORKSTREAM} agent sessions"
                        )));
                    }
                    Ok((agent_links::list_for_workstream(tx, &id)?, changed))
                })
                .map_err(from_write)
                .map(|(rows, changed)| (to_links(&rows), changed))
            })
            .await?;
        if changed {
            tracing::debug!(
                workstream = %workstream_id,
                provider = provider.as_str(),
                "agent session linked to a workstream"
            );
        }
        Ok(AgentLinkChange {
            workstream_id,
            links,
            changed,
        })
    }

    /// `workstreams.unlinkAgent`. Removes the link and nothing else: the
    /// session is not Gritty's to touch.
    pub async fn unlink_agent(
        self: &Arc<Self>,
        request: UnlinkAgentRequest,
        correlation: CorrelationId,
    ) -> Result<AgentLinkChange, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let session_id = required_text("sessionId", &request.session_id, MAX_SESSION_ID)?;
        let provider = request.provider;
        let id = request.workstream_id.to_string();
        let workstream_id = request.workstream_id.clone();
        let (links, changed) = self
            .blocking(move |db| {
                db.write(|tx| {
                    let existing = require_row(&workstreams::get(tx, &id)?, &id)?;
                    ensure_in_workspace(&existing, &root)?;
                    let changed = agent_links::delete(tx, &id, provider.as_str(), &session_id)?;
                    Ok((agent_links::list_for_workstream(tx, &id)?, changed))
                })
                .map_err(from_write)
                .map(|(rows, changed)| (to_links(&rows), changed))
            })
            .await?;
        Ok(AgentLinkChange {
            workstream_id,
            links,
            changed,
        })
    }

    /// `workstreams.agentLinks`. A read, so it answers with whatever is stored
    /// even when none of it is running.
    pub async fn agent_links(
        self: &Arc<Self>,
        request: ListAgentLinksRequest,
        correlation: CorrelationId,
    ) -> Result<Vec<WorkstreamAgentLink>, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let workstream_id = request.workstream_id.as_ref().map(ToString::to_string);
        self.blocking(move |db| {
            db.read(|conn| match workstream_id.as_deref() {
                Some(id) => {
                    // A workstream of another workspace reads as missing rather
                    // than forbidden, exactly as every other command here.
                    let existing = require_row(&workstreams::get(conn, id)?, id)?;
                    ensure_in_workspace(&existing, &root)?;
                    agent_links::list_for_workstream(conn, id)
                }
                None => agent_links::list_for_workspace(conn, &root),
            })
            .map_err(from_write)
            .map(|rows| to_links(&rows))
        })
        .await
    }
}

/// Stored rows onto the wire.
///
/// A row whose provider this build does not know is **skipped and logged**
/// rather than failing the read. It can only come from a database edited by
/// hand or written by a newer build (a newer schema is refused outright), and
/// losing one row from a list is a smaller lie than telling the user their
/// workstreams are unreadable.
fn to_links(rows: &[agent_links::AgentLinkRow]) -> Vec<WorkstreamAgentLink> {
    rows.iter()
        .filter_map(|row| match AgentProvider::parse(&row.provider) {
            Ok(provider) => Some(WorkstreamAgentLink {
                workstream_id: WorkstreamId::from_raw(row.workstream_id.clone()),
                provider,
                session_id: row.session_id.clone(),
                linked_at: Timestamp::from_millis(u64::try_from(row.linked_at).unwrap_or(0)),
            }),
            Err(error) => {
                tracing::warn!(
                    provider = %row.provider,
                    %error,
                    "skipping an agent link whose provider this build does not know"
                );
                None
            }
        })
        .collect()
}
