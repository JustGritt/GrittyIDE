//! Workstreams (T200, Gate C): the durable model of *work*.
//!
//! A workstream is a unit of work with a goal and a checked state machine
//! ([`state`]), plus the decisions that block it. It has no AI in it: you can
//! create one, describe it, move it through its states, block it on a question
//! and finish it entirely by hand. That is deliberate — the model has to be
//! usable and testable before any agent exists, and an agent adapter (T300)
//! must attach to this model rather than bring its own.
//!
//! What lives elsewhere, on purpose:
//! - agent runs, providers, capabilities and permissions (T300/T310);
//! - `ChangeSet`, revisions and worktrees (T100/T400, ADR-007). A ChangeSet
//!   will reference `workstream_id`; nothing here reaches into Git;
//! - aggregation across workspaces for the attention queue (T410).
//!
//! Storage is the session database (T060). Rows are keyed by the canonical
//! workspace root, like every other table there, but a [`WorkstreamId`] is
//! itself persisted: a workstream outlives the process, so its id is domain
//! data rather than a per-process handle. Commands still address the workspace
//! by its per-process id, so nothing can touch the workstreams of a workspace
//! that is not open.
//!
//! Every command reads the database back rather than trusting a cache: there
//! is no in-memory copy that a failed write could leave lying.

pub mod link;
pub mod state;
pub mod wire;

use std::sync::Arc;

use gritty_persistence::store::{decisions, workstreams};
use gritty_persistence::{Database, PersistenceError, now_ms};
use tokio::runtime::Handle;

use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::protocol::actor::Actor;
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::ids::{CorrelationId, DecisionId, WorkspaceId, WorkstreamId};
use crate::protocol::time::Timestamp;
use crate::workspace::WorkspaceManager;

pub use link::{
    AgentLinkChange, LinkAgentRequest, ListAgentLinksRequest, UnlinkAgentRequest,
    WorkstreamAgentLink,
};
pub use state::WorkstreamState;
pub use wire::{
    CreateWorkstreamRequest, Decision, DecisionOption, DecisionRequestedPayload,
    DecisionResolvedPayload, DecisionState, DismissDecisionRequest, ListDecisionsRequest,
    ListWorkstreamsRequest, RaiseDecisionRequest, RemoveWorkstreamRequest, ResolveDecisionRequest,
    TransitionWorkstreamRequest, UpdateWorkstreamRequest, Workstream, WorkstreamChangedPayload,
    WorkstreamCreatedPayload, WorkstreamRemovedPayload,
};
use wire::{MAX_PROSE, MAX_TITLE, optional_text, required_text, validate_options};

/// Owns the workstream domain. One per process, managed by the composition
/// root next to the session service it shares a database with.
pub struct WorkstreamManager {
    handle: Handle,
    events: Arc<EventBus>,
    workspaces: Arc<WorkspaceManager>,
    /// `None` when the session database never opened; every command then
    /// fails rather than keeping work that would vanish at exit.
    database: Option<Arc<Database>>,
}

impl WorkstreamManager {
    pub fn new(
        handle: Handle,
        events: Arc<EventBus>,
        workspaces: Arc<WorkspaceManager>,
        database: Option<Arc<Database>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            handle,
            events,
            workspaces,
            database,
        })
    }

    /// `workstreams.list`
    pub async fn list(
        self: &Arc<Self>,
        request: ListWorkstreamsRequest,
        correlation: CorrelationId,
    ) -> Result<Vec<Workstream>, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let workspace_id = request.workspace_id.clone();
        self.blocking(move |db| {
            db.read(|conn| {
                let rows = workstreams::list(conn, &root)?;
                let counts = decisions::open_counts(conn, &root)?;
                Ok((rows, counts))
            })
            .map_err(persistence_error)
            .and_then(|(rows, counts)| {
                rows.iter()
                    .map(|row| {
                        let open = counts
                            .iter()
                            .find(|(id, _)| *id == row.id)
                            .map_or(0, |(_, count)| *count);
                        to_workstream(row, &workspace_id, open)
                    })
                    .collect()
            })
        })
        .await
    }

    /// `workstreams.create`
    pub async fn create(
        self: &Arc<Self>,
        request: CreateWorkstreamRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Workstream, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let title = required_text("title", &request.title, MAX_TITLE)?;
        let goal = optional_text("goal", request.goal.as_deref(), MAX_PROSE)?;
        let workspace_id = request.workspace_id.clone();
        let id = WorkstreamId::new();
        let now = now_ms();
        let row = workstreams::WorkstreamRow {
            id: id.to_string(),
            workspace_root: root,
            title,
            goal,
            // Everything starts as a sketch; READY is a decision the user makes.
            state: WorkstreamState::Draft.as_str().to_owned(),
            created_at: now,
            updated_at: now,
        };
        let created = self
            .blocking(move |db| {
                db.write(|tx| workstreams::insert(tx, &row).map(|()| row.clone()))
                    .map_err(persistence_error)
                    .and_then(|row| to_workstream(&row, &workspace_id, 0))
            })
            .await?;
        self.emit(
            &request.workspace_id,
            &correlation,
            &actor,
            CoreEvent::WorkstreamCreated(WorkstreamCreatedPayload {
                workstream: created.clone(),
            }),
        );
        Ok(created)
    }

    /// `workstreams.update`: title and goal only. State moves through
    /// [`Self::transition`], which is the only place the machine is checked.
    pub async fn update(
        self: &Arc<Self>,
        request: UpdateWorkstreamRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Workstream, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let title = request
            .title
            .as_deref()
            .map(|title| required_text("title", title, MAX_TITLE))
            .transpose()?;
        // An absent goal leaves it alone; a present blank one clears it. The
        // request cannot express "clear" any other way.
        let goal = match request.goal.as_deref() {
            None => None,
            Some(goal) => Some(optional_text("goal", Some(goal), MAX_PROSE)?),
        };
        let workspace_id = request.workspace_id.clone();
        let id = request.workstream_id.to_string();
        let updated = self
            .blocking(move |db| {
                db.write(|tx| {
                    let existing = require_row(&workstreams::get(tx, &id)?, &id)?;
                    ensure_in_workspace(&existing, &root)?;
                    let next_title = title.unwrap_or_else(|| existing.title.clone());
                    let next_goal = goal.unwrap_or_else(|| existing.goal.clone());
                    let now = now_ms();
                    workstreams::update(tx, &id, &next_title, next_goal.as_deref(), now)?;
                    let open = decisions::count_open(tx, &id)?;
                    Ok((
                        workstreams::WorkstreamRow {
                            title: next_title,
                            goal: next_goal,
                            updated_at: now,
                            ..existing
                        },
                        open,
                    ))
                })
                .map_err(from_write)
                .and_then(|(row, open)| to_workstream(&row, &workspace_id, open))
            })
            .await?;
        self.emit(
            &request.workspace_id,
            &correlation,
            &actor,
            CoreEvent::WorkstreamChanged(WorkstreamChangedPayload {
                workstream: updated.clone(),
            }),
        );
        Ok(updated)
    }

    /// `workstreams.transition`. An illegal move is a `Conflict` and changes
    /// nothing; the transaction is what guarantees the "changes nothing".
    ///
    /// Leaving `BLOCKED` by hand is allowed even with decisions still open —
    /// the user is the supervisor. Those decisions stay open and stay in the
    /// attention queue.
    pub async fn transition(
        self: &Arc<Self>,
        request: TransitionWorkstreamRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Workstream, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let workspace_id = request.workspace_id.clone();
        let id = request.workstream_id.to_string();
        let to = request.to;
        let moved = self
            .blocking(move |db| {
                db.write(|tx| {
                    let existing = require_row(&workstreams::get(tx, &id)?, &id)?;
                    ensure_in_workspace(&existing, &root)?;
                    WorkstreamState::parse(&existing.state)
                        .and_then(|state| state.check_transition(to))
                        .map_err(domain)?;
                    let now = now_ms();
                    workstreams::set_state(tx, &id, to.as_str(), now)?;
                    let open = decisions::count_open(tx, &id)?;
                    Ok((
                        workstreams::WorkstreamRow {
                            state: to.as_str().to_owned(),
                            updated_at: now,
                            ..existing
                        },
                        open,
                    ))
                })
                .map_err(from_write)
                .and_then(|(row, open)| to_workstream(&row, &workspace_id, open))
            })
            .await?;
        self.emit(
            &request.workspace_id,
            &correlation,
            &actor,
            CoreEvent::WorkstreamChanged(WorkstreamChangedPayload {
                workstream: moved.clone(),
            }),
        );
        Ok(moved)
    }

    /// `workstreams.remove`: the workstream and its decisions, permanently.
    /// `ABANDONED` exists for the softer version.
    pub async fn remove(
        self: &Arc<Self>,
        request: RemoveWorkstreamRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<(), CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let id = request.workstream_id.to_string();
        let title = self
            .blocking(move |db| {
                db.write(|tx| {
                    let existing = require_row(&workstreams::get(tx, &id)?, &id)?;
                    ensure_in_workspace(&existing, &root)?;
                    workstreams::delete(tx, &id)?;
                    Ok(existing.title)
                })
                .map_err(from_write)
            })
            .await?;
        self.emit(
            &request.workspace_id,
            &correlation,
            &actor,
            CoreEvent::WorkstreamRemoved(WorkstreamRemovedPayload {
                workstream_id: request.workstream_id.clone(),
                title,
            }),
        );
        Ok(())
    }

    /// `decisions.list`
    pub async fn list_decisions(
        self: &Arc<Self>,
        request: ListDecisionsRequest,
        correlation: CorrelationId,
    ) -> Result<Vec<Decision>, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let workstream_id = request.workstream_id.as_ref().map(ToString::to_string);
        let open_only = request.open_only.unwrap_or(false);
        self.blocking(move |db| {
            db.read(|conn| decisions::list(conn, &root, workstream_id.as_deref(), open_only))
                .map_err(persistence_error)
                .and_then(|rows| rows.iter().map(to_decision).collect())
        })
        .await
    }

    /// `decisions.raise`.
    ///
    /// A question asked while the work is `IN_PROGRESS` blocks it: that is the
    /// "needs you" signal, and it belongs to the domain rather than to a UI
    /// that might forget to apply it. Asking about work that is still a draft,
    /// already blocked or already finished does not move it.
    pub async fn raise_decision(
        self: &Arc<Self>,
        request: RaiseDecisionRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Decision, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let question = required_text("question", &request.question, MAX_PROSE)?;
        let options = validate_options(&request.options)?;
        let workspace_id = request.workspace_id.clone();
        let workstream_id = request.workstream_id.to_string();
        let raised_by = serde_json::to_string(&actor)?;
        let id = DecisionId::new();
        let row = decisions::DecisionRow {
            id: id.to_string(),
            workstream_id: workstream_id.clone(),
            question,
            options: encode_options(&options)?,
            state: DecisionState::Open.as_str().to_owned(),
            chosen: None,
            rationale: None,
            raised_by,
            raised_at: now_ms(),
            resolved_at: None,
        };
        let (decision, blocked) = self
            .blocking(move |db| {
                db.write(|tx| {
                    let existing =
                        require_row(&workstreams::get(tx, &workstream_id)?, &workstream_id)?;
                    ensure_in_workspace(&existing, &root)?;
                    decisions::insert(tx, &row)?;
                    let state = WorkstreamState::parse(&existing.state).map_err(domain)?;
                    let now = now_ms();
                    let blocked = if state == WorkstreamState::InProgress {
                        workstreams::set_state(
                            tx,
                            &workstream_id,
                            WorkstreamState::Blocked.as_str(),
                            now,
                        )?;
                        Some(workstreams::WorkstreamRow {
                            state: WorkstreamState::Blocked.as_str().to_owned(),
                            updated_at: now,
                            ..existing
                        })
                    } else {
                        None
                    };
                    let open = decisions::count_open(tx, &row.workstream_id)?;
                    Ok((row.clone(), blocked, open))
                })
                .map_err(from_write)
                .and_then(|(row, blocked, open)| {
                    let decision = to_decision(&row)?;
                    let blocked = blocked
                        .map(|row| to_workstream(&row, &workspace_id, open))
                        .transpose()?;
                    Ok((decision, blocked))
                })
            })
            .await?;
        self.emit(
            &request.workspace_id,
            &correlation,
            &actor,
            CoreEvent::DecisionRequested(DecisionRequestedPayload {
                decision: decision.clone(),
            }),
        );
        if let Some(workstream) = blocked {
            self.emit(
                &request.workspace_id,
                &correlation,
                &actor,
                CoreEvent::WorkstreamChanged(WorkstreamChangedPayload { workstream }),
            );
        }
        Ok(decision)
    }

    /// `decisions.resolve`: an answer.
    pub async fn resolve_decision(
        self: &Arc<Self>,
        request: ResolveDecisionRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Decision, CoreError> {
        let rationale = optional_text("rationale", request.rationale.as_deref(), MAX_PROSE)?;
        self.close_decision(
            &request.workspace_id,
            &request.decision_id,
            DecisionState::Resolved,
            request.chosen.as_deref(),
            rationale,
            actor,
            correlation,
        )
        .await
    }

    /// `decisions.dismiss`: the question stopped mattering. No answer is
    /// recorded, because inventing one would be a lie in the run log.
    pub async fn dismiss_decision(
        self: &Arc<Self>,
        request: DismissDecisionRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Decision, CoreError> {
        let rationale = optional_text("rationale", request.rationale.as_deref(), MAX_PROSE)?;
        self.close_decision(
            &request.workspace_id,
            &request.decision_id,
            DecisionState::Dismissed,
            None,
            rationale,
            actor,
            correlation,
        )
        .await
    }

    /// Shared body of resolve and dismiss: close the decision and, when it was
    /// the last open one holding a `BLOCKED` workstream, let the work resume.
    #[allow(clippy::too_many_arguments)]
    async fn close_decision(
        self: &Arc<Self>,
        workspace_id: &WorkspaceId,
        decision_id: &DecisionId,
        to: DecisionState,
        chosen: Option<&str>,
        rationale: Option<String>,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Decision, CoreError> {
        let root = self.root_of(workspace_id, &correlation)?;
        let id = decision_id.to_string();
        let chosen = chosen.map(str::to_owned);
        let workspace = workspace_id.clone();
        let (decision, resumed) = self
            .blocking(move |db| {
                db.write(|tx| {
                    let existing = decisions::get(tx, &id)?
                        .ok_or_else(|| not_found(format!("decision {id} does not exist")))?;
                    let workstream = require_row(
                        &workstreams::get(tx, &existing.workstream_id)?,
                        &existing.workstream_id,
                    )?;
                    ensure_in_workspace(&workstream, &root)?;
                    let chosen = validated_answer(&existing, to, chosen.as_deref())?;
                    let now = now_ms();
                    let closed = decisions::close(
                        tx,
                        &id,
                        to.as_str(),
                        chosen.as_deref(),
                        rationale.as_deref(),
                        now,
                    )?;
                    if !closed {
                        return Err(conflict(format!(
                            "decision {id} is already {}",
                            existing.state
                        )));
                    }
                    let open = decisions::count_open(tx, &workstream.id)?;
                    let state = WorkstreamState::parse(&workstream.state).map_err(domain)?;
                    // The last answer releases the work it was holding up.
                    let resumed = if open == 0 && state == WorkstreamState::Blocked {
                        workstreams::set_state(
                            tx,
                            &workstream.id,
                            WorkstreamState::InProgress.as_str(),
                            now,
                        )?;
                        Some(workstreams::WorkstreamRow {
                            state: WorkstreamState::InProgress.as_str().to_owned(),
                            updated_at: now,
                            ..workstream
                        })
                    } else {
                        None
                    };
                    Ok((
                        decisions::DecisionRow {
                            state: to.as_str().to_owned(),
                            chosen,
                            rationale,
                            resolved_at: Some(now),
                            ..existing
                        },
                        resumed,
                        open,
                    ))
                })
                .map_err(from_write)
                .and_then(|(row, resumed, open)| {
                    let decision = to_decision(&row)?;
                    let resumed = resumed
                        .map(|row| to_workstream(&row, &workspace, open))
                        .transpose()?;
                    Ok((decision, resumed))
                })
            })
            .await?;
        self.emit(
            workspace_id,
            &correlation,
            &actor,
            CoreEvent::DecisionResolved(DecisionResolvedPayload {
                decision: decision.clone(),
            }),
        );
        if let Some(workstream) = resumed {
            self.emit(
                workspace_id,
                &correlation,
                &actor,
                CoreEvent::WorkstreamChanged(WorkstreamChangedPayload { workstream }),
            );
        }
        Ok(decision)
    }

    /// The canonical root of an open workspace, or `NotFound`. Everything
    /// funnels through here, so a command can never reach the workstreams of a
    /// workspace that is not open.
    fn root_of(
        &self,
        workspace_id: &WorkspaceId,
        correlation: &CorrelationId,
    ) -> Result<String, CoreError> {
        let (workspace, _root) = self.workspaces.require(workspace_id, correlation)?;
        Ok(workspace.root)
    }

    fn database(&self) -> Result<Arc<Database>, CoreError> {
        self.database.clone().ok_or_else(|| {
            CoreError::new(
                CoreErrorCode::Io,
                "workstreams need the session database, which is unavailable for this run",
            )
        })
    }

    /// SQLite off the async worker. Every command goes through here.
    async fn blocking<T: Send + 'static>(
        &self,
        work: impl FnOnce(&Database) -> Result<T, CoreError> + Send + 'static,
    ) -> Result<T, CoreError> {
        let database = self.database()?;
        self.handle
            .spawn_blocking(move || work(&database))
            .await
            .map_err(|join| CoreError::internal(format!("workstream task failed: {join}")))?
    }

    fn emit(
        &self,
        workspace_id: &WorkspaceId,
        correlation: &CorrelationId,
        actor: &Actor,
        event: CoreEvent,
    ) {
        self.events.emit(
            &EventOrigin {
                workspace_id: Some(workspace_id.clone()),
                correlation_id: Some(correlation.clone()),
                actor: actor.clone(),
            },
            event,
        );
    }
}

/// A domain refusal carried out through the persistence layer's error type so
/// it can travel out of a `write` closure, then unwrapped by [`from_write`].
/// SQLite rolls the transaction back on the way out, which is what makes
/// "an illegal transition changes nothing" true rather than merely intended.
fn domain(error: CoreError) -> PersistenceError {
    PersistenceError::Io {
        context: DOMAIN_MARKER.to_owned(),
        source: std::io::Error::other(error),
    }
}

const DOMAIN_MARKER: &str = "gritty:domain";

fn not_found(message: String) -> PersistenceError {
    domain(CoreError::not_found(message))
}

fn conflict(message: String) -> PersistenceError {
    domain(CoreError::conflict(message))
}

/// Unwrap a domain refusal smuggled through [`domain`]; anything else is a
/// genuine storage failure.
fn from_write(error: PersistenceError) -> CoreError {
    if let PersistenceError::Io { context, source } = &error
        && context == DOMAIN_MARKER
        && let Some(inner) = source
            .get_ref()
            .and_then(|boxed| boxed.downcast_ref::<CoreError>())
    {
        return inner.clone();
    }
    persistence_error(error)
}

fn persistence_error(error: PersistenceError) -> CoreError {
    CoreError::new(CoreErrorCode::Io, format!("workstream database: {error}"))
}

fn require_row(
    row: &Option<workstreams::WorkstreamRow>,
    id: &str,
) -> Result<workstreams::WorkstreamRow, PersistenceError> {
    row.clone()
        .ok_or_else(|| not_found(format!("workstream {id} does not exist")))
}

/// A workstream of another workspace is not visible from this one; it reads as
/// missing rather than forbidden, because the caller has no business knowing
/// it exists.
fn ensure_in_workspace(
    row: &workstreams::WorkstreamRow,
    root: &str,
) -> Result<(), PersistenceError> {
    if row.workspace_root == root {
        return Ok(());
    }
    Err(not_found(format!(
        "workstream {} does not exist in this workspace",
        row.id
    )))
}

/// What `chosen` may be: one of the offered keys, or free text when the
/// question offered none. Dismissal records no answer at all.
fn validated_answer(
    row: &decisions::DecisionRow,
    to: DecisionState,
    chosen: Option<&str>,
) -> Result<Option<String>, PersistenceError> {
    if to == DecisionState::Dismissed {
        return Ok(None);
    }
    let options = decode_options(row.options.as_deref()).map_err(domain)?;
    let answer = chosen.map(str::trim).filter(|value| !value.is_empty());
    let Some(answer) = answer else {
        return Err(domain(CoreError::invalid_input(
            "chosen",
            "resolving a decision needs an answer",
        )));
    };
    if options.is_empty() {
        if answer.chars().count() > MAX_PROSE {
            return Err(domain(CoreError::invalid_input(
                "chosen",
                format!("must be at most {MAX_PROSE} characters"),
            )));
        }
        return Ok(Some(answer.to_owned()));
    }
    if options.iter().any(|option| option.key == answer) {
        return Ok(Some(answer.to_owned()));
    }
    Err(domain(CoreError::invalid_input(
        "chosen",
        format!("{answer:?} is not one of the offered options"),
    )))
}

fn encode_options(options: &[DecisionOption]) -> Result<Option<String>, CoreError> {
    if options.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::to_string(options)?))
}

fn decode_options(raw: Option<&str>) -> Result<Vec<DecisionOption>, CoreError> {
    match raw {
        None => Ok(Vec::new()),
        Some(json) => serde_json::from_str(json).map_err(|error| {
            CoreError::internal(format!("stored decision options are unreadable: {error}"))
        }),
    }
}

fn to_workstream(
    row: &workstreams::WorkstreamRow,
    workspace_id: &WorkspaceId,
    open_decisions: i64,
) -> Result<Workstream, CoreError> {
    let state = WorkstreamState::parse(&row.state)?;
    Ok(Workstream {
        id: WorkstreamId::from_raw(row.id.clone()),
        workspace_id: workspace_id.clone(),
        title: row.title.clone(),
        goal: row.goal.clone(),
        state,
        allowed_transitions: state.allowed_transitions().to_vec(),
        open_decisions: u32::try_from(open_decisions).unwrap_or(u32::MAX),
        created_at: Timestamp::from_millis(u64::try_from(row.created_at).unwrap_or(0)),
        updated_at: Timestamp::from_millis(u64::try_from(row.updated_at).unwrap_or(0)),
    })
}

fn to_decision(row: &decisions::DecisionRow) -> Result<Decision, CoreError> {
    Ok(Decision {
        id: DecisionId::from_raw(row.id.clone()),
        workstream_id: WorkstreamId::from_raw(row.workstream_id.clone()),
        question: row.question.clone(),
        options: decode_options(row.options.as_deref())?,
        state: DecisionState::parse(&row.state)?,
        chosen: row.chosen.clone(),
        rationale: row.rationale.clone(),
        // A row Gritty cannot attribute is shown as external rather than
        // silently credited to the user.
        raised_by: serde_json::from_str(&row.raised_by).unwrap_or(Actor::External),
        raised_at: Timestamp::from_millis(u64::try_from(row.raised_at).unwrap_or(0)),
        resolved_at: row
            .resolved_at
            .map(|at| Timestamp::from_millis(u64::try_from(at).unwrap_or(0))),
    })
}

#[cfg(test)]
mod tests;
