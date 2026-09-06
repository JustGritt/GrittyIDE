//! Workspace ownership (T325): who may write where, and what happens when two
//! workstreams want the same ground.
//!
//! Several coding agents working in one checkout have a problem Git does not
//! solve: two of them editing the same file at the same time, neither knowing.
//! This is the orchestration primitive for that — a registry of **logical**
//! leases, deliberately not operating-system file locks. Agents do not respect
//! OS locks, lock semantics differ per platform, a directory or a glob cannot
//! be locked at all, and none of it is visible in a UI or attributable to a
//! piece of work. A lease is domain state, which is what makes it inspectable,
//! persistent and refusable.
//!
//! **A lease is held by a workstream.** Hard rule 6 keeps provider concepts out
//! of the domain, and a durable row must not carry an id that exists only while
//! a process does. [`wire::OwnershipLease::agent_run_id`] is the seam T300
//! attaches a run to.
//!
//! What lives elsewhere, on purpose:
//! - **enforcement.** Nothing here stops a write. This is *observed* ownership:
//!   Gritty knows who claimed what and refuses a second claim, and that is all
//!   it may be said to do. Stopping a write before it happens needs an agent
//!   that offers a pre-write hook Gritty may answer, which ADR-018 currently
//!   forbids its hook from doing (T330);
//! - **worktrees** (T400). Two workstreams in separate trees cannot collide on
//!   the filesystem at all; what they can still do is collide on merge, which
//!   is a review conflict and not this;
//! - **scheduling** (T405): comparing a workstream's intended scope against the
//!   registry *before* an agent starts.
//!
//! What *is* here, and was not in T325: **noticing that an agent wrote to owned
//! ground anyway** (T326). [`OwnershipManager::observe_touches`] cross-checks
//! the files the agent rail reported against the registry, attributed through
//! T223's session→workstream link, and records what it finds as a fourth
//! [`ConflictState`], `Observed`. It runs *after* the write, never before it,
//! so the paragraph above still holds in full: this notices, it does not stop.
//!
//! Storage is the session database (T060), one migration chain, keyed by the
//! canonical workspace root like every other table there. Every command reads
//! the database back rather than trusting a cache.

pub mod target;
pub mod wire;

use std::collections::HashSet;
use std::sync::Arc;

use gritty_persistence::store::{agent_links, ownership, workstreams};
use gritty_persistence::{Connection, Database, PersistenceError, now_ms};
use gritty_workspace::WorkspacePath;
use tokio::runtime::Handle;

use crate::agent_session::AgentProvider;
use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::protocol::actor::Actor;
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::ids::{
    AgentRunId, ConflictId, CorrelationId, LeaseId, WorkspaceId, WorkstreamId,
};
use crate::protocol::time::Timestamp;
use crate::workspace::WorkspaceManager;
use crate::workstream::state::WorkstreamState;

pub use target::OwnershipTarget;
use wire::optional_note;
pub use wire::{
    AcquireOwnershipRequest, CheckOwnershipRequest, ConflictResolution, ConflictState, LeaseState,
    ListOwnershipRequest, MAX_CONFLICTS, MAX_LEASES, OwnershipAcquisition, OwnershipCheck,
    OwnershipConflict, OwnershipLease, OwnershipMap, OwnershipMode, ReleaseOwnershipRequest,
    ResolveConflictRequest,
};

/// What makes two observed violations the same one (T326 §9):
/// `(requested_by, holding_lease_id, target_kind, target_pattern)`. The state
/// is not in the key because only unresolved `OBSERVED` rows are ever put in
/// the set — a violation the user has dealt with does not suppress the next.
type ObservedKey = (String, String, String, String);

/// Owns the ownership domain. One per process, managed by the composition root
/// next to the workstream manager it shares a database with.
pub struct OwnershipManager {
    handle: Handle,
    events: Arc<EventBus>,
    workspaces: Arc<WorkspaceManager>,
    /// `None` when the session database never opened. Every command then fails
    /// rather than granting a lease that would vanish at exit — an ownership
    /// registry that forgets is worse than none, because it is believed.
    database: Option<Arc<Database>>,
}

impl OwnershipManager {
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

    /// `ownership.list`: who owns what in this workspace, in one snapshot.
    pub async fn list(
        self: &Arc<Self>,
        request: ListOwnershipRequest,
        correlation: CorrelationId,
    ) -> Result<OwnershipMap, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let workspace_id = request.workspace_id.clone();
        self.blocking(move |db| {
            db.read(|conn| {
                let leases = ownership::list_held(conn, &root)?;
                let conflicts = ownership::list_conflicts(conn, &root, MAX_CONFLICTS as i64)?;
                Ok((leases, conflicts))
            })
            .map_err(persistence_error)
            .and_then(|(leases, conflicts)| to_map(&leases, &conflicts, &workspace_id, now_ms()))
        })
        .await
    }

    /// `ownership.acquire`.
    ///
    /// The overlap check and the insert run inside **one** transaction, which
    /// is what makes acquisition atomic: two requests racing for the same
    /// ground cannot both be granted, because the loser's check has already
    /// seen the winner's row. A refusal rolls back and writes only the
    /// conflict that explains it.
    pub async fn acquire(
        self: &Arc<Self>,
        request: AcquireOwnershipRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<OwnershipAcquisition, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        request.target.validate()?;
        let note = optional_note(request.note.as_deref())?;
        let workspace_id = request.workspace_id.clone();
        let workstream_id = request.workstream_id.to_string();
        let target = request.target.clone();
        let expires_at = request.expires_at;

        let (acquisition, map) = self
            .blocking(move |db| {
                db.write(|tx| {
                    let holder =
                        require_row(&workstreams::get(tx, &workstream_id)?, &workstream_id)?;
                    ensure_in_workspace(&holder, &root)?;
                    let now = now_ms();
                    let held = ownership::list_held(tx, &root)?;
                    let active: Vec<&ownership::HeldLease> = held
                        .iter()
                        .filter(|entry| still_holds(&entry.lease, now))
                        .collect();

                    // Asking again for ground you already hold is not a
                    // conflict and is not a second row: it is the same lease.
                    if let Some(existing) = active.iter().find(|entry| {
                        entry.lease.workstream_id == workstream_id
                            && entry.lease.target_kind == target.kind()
                            && entry.lease.target_pattern == target.as_pattern()
                    }) {
                        let lease = to_lease(existing, &workspace_id, now).map_err(domain)?;
                        let map = read_map(tx, &root, &workspace_id, now)?;
                        return Ok((OwnershipAcquisition::Granted { lease }, map));
                    }

                    if active.len() >= MAX_LEASES {
                        return Err(conflict(format!(
                            "this workspace already holds {MAX_LEASES} leases; release one first"
                        )));
                    }

                    // The oldest overlapping lease held by somebody else is the
                    // one in the way. `list_held` is ordered, so which one that
                    // is does not depend on the order rows come back in.
                    let blocker = active
                        .iter()
                        .filter(|entry| entry.lease.workstream_id != workstream_id)
                        .find_map(|entry| {
                            let held_target = OwnershipTarget::from_storage(
                                &entry.lease.target_kind,
                                &entry.lease.target_pattern,
                            )
                            .ok()?;
                            held_target.overlaps(&target).then_some(*entry)
                        });

                    match blocker {
                        Some(entry) => {
                            let row = match ownership::waiting_conflict(
                                tx,
                                &workstream_id,
                                &entry.lease.id,
                                target.kind(),
                                &target.as_pattern(),
                            )? {
                                // The same refusal, asked twice. One blocker,
                                // one row in the queue.
                                Some(existing) => existing,
                                None => {
                                    let row = ownership::ConflictRow {
                                        id: ConflictId::new().to_string(),
                                        workspace_root: root.clone(),
                                        requested_by: workstream_id.clone(),
                                        owned_by: entry.lease.workstream_id.clone(),
                                        holding_lease_id: entry.lease.id.clone(),
                                        target_kind: target.kind().to_owned(),
                                        target_pattern: target.as_pattern(),
                                        created_at: now,
                                        state: ConflictState::Waiting.as_str().to_owned(),
                                        resolved_at: None,
                                    };
                                    ownership::insert_conflict(tx, &row)?;
                                    row
                                }
                            };
                            let conflict = to_conflict(&row, &workspace_id).map_err(domain)?;
                            let map = read_map(tx, &root, &workspace_id, now)?;
                            Ok((OwnershipAcquisition::Conflicted { conflict }, map))
                        }
                        None => {
                            let row = ownership::LeaseRow {
                                id: LeaseId::new().to_string(),
                                workspace_root: root.clone(),
                                workstream_id: workstream_id.clone(),
                                // Nothing constructs an AgentRunId until T300.
                                agent_run_id: None,
                                target_kind: target.kind().to_owned(),
                                target_pattern: target.as_pattern(),
                                mode: OwnershipMode::Write.as_str().to_owned(),
                                note,
                                acquired_at: now,
                                expires_at: expires_at.map(millis_to_i64),
                                state: LeaseState::Active.as_str().to_owned(),
                                released_at: None,
                            };
                            ownership::insert_lease(tx, &row)?;
                            let lease = to_lease(
                                &ownership::HeldLease {
                                    lease: row,
                                    holder_state: holder.state.clone(),
                                },
                                &workspace_id,
                                now,
                            )
                            .map_err(domain)?;
                            let map = read_map(tx, &root, &workspace_id, now)?;
                            Ok((OwnershipAcquisition::Granted { lease }, map))
                        }
                    }
                })
                .map_err(from_write)
            })
            .await?;

        self.emit(&request.workspace_id, &correlation, &actor, map);
        Ok(acquisition)
    }

    /// `ownership.release`. Every conflict still waiting on this lease becomes
    /// `RESOLVED` — the blocker is gone. Nothing is granted in its place: the
    /// requester asks again, deliberately.
    pub async fn release(
        self: &Arc<Self>,
        request: ReleaseOwnershipRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<(), CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let workspace_id = request.workspace_id.clone();
        let lease_id = request.lease_id.to_string();
        let map = self
            .blocking(move |db| {
                db.write(|tx| {
                    let existing = ownership::get_lease(tx, &lease_id)?.ok_or_else(|| {
                        not_found(format!("ownership lease {lease_id} does not exist"))
                    })?;
                    ensure_lease_in_workspace(&existing, &root)?;
                    let now = now_ms();
                    if !ownership::release_lease(tx, &lease_id, now)? {
                        return Err(conflict(format!(
                            "ownership lease {lease_id} is already {}",
                            existing.state
                        )));
                    }
                    ownership::close_conflicts_for_lease(
                        tx,
                        &lease_id,
                        ConflictState::Resolved.as_str(),
                        now,
                        None,
                    )?;
                    read_map(tx, &root, &workspace_id, now)
                })
                .map_err(from_write)
            })
            .await?;
        self.emit(&request.workspace_id, &correlation, &actor, map);
        Ok(())
    }

    /// `ownership.check`: which leases stand in the way of this ground,
    /// whoever holds them. The answer to "who owns this file, and why can't
    /// the other one touch it".
    pub async fn check(
        self: &Arc<Self>,
        request: CheckOwnershipRequest,
        correlation: CorrelationId,
    ) -> Result<OwnershipCheck, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        request.target.validate()?;
        let workspace_id = request.workspace_id.clone();
        let target = request.target.clone();
        self.blocking(move |db| {
            db.read(|conn| ownership::list_held(conn, &root))
                .map_err(persistence_error)
                .and_then(|held| {
                    let now = now_ms();
                    let mut holders = Vec::new();
                    for entry in &held {
                        if !still_holds(&entry.lease, now) {
                            continue;
                        }
                        let held_target = OwnershipTarget::from_storage(
                            &entry.lease.target_kind,
                            &entry.lease.target_pattern,
                        )?;
                        if held_target.overlaps(&target) {
                            holders.push(to_lease(entry, &workspace_id, now)?);
                        }
                    }
                    Ok(OwnershipCheck { target, holders })
                })
        })
        .await
    }

    /// T326: a session said it wrote these files. Record the ones that landed
    /// on another workstream's ground.
    ///
    /// **This notices, it does not stop.** By the time this runs the write has
    /// happened; ownership is still *observed* and not enforced, and
    /// `docs/architecture/ownership.md` lines 17-26 say so where the user can
    /// read it. Stopping the write is T330 and needs an ADR-018 amendment
    /// first.
    ///
    /// It is not a command. It rides the per-tick delta the agent rail already
    /// produces (`AgentTimelineAppended`), so it costs no scan of its own and
    /// no second feed — and a tick that finds nothing writes nothing and emits
    /// nothing (rule 8). No database, no open workspace or no link means no
    /// finding, which is not a failure: a violation nobody can attribute is not
    /// one.
    ///
    /// Attribution is by `(provider, session_id)` — the pair T223's link is
    /// keyed on — never by working directory or title, so nothing is guessed
    /// (rule 4).
    ///
    /// Returns how many rows it wrote, which is what the tests assert and what
    /// decides whether an event is emitted at all.
    pub async fn observe_touches(
        self: &Arc<Self>,
        provider: AgentProvider,
        session_id: String,
        paths: Vec<WorkspacePath>,
    ) -> Result<usize, CoreError> {
        if paths.is_empty() || self.database.is_none() {
            return Ok(0);
        }
        // The one workspace whose leases these paths could be in: `FileTouched`
        // only carries a path when the file is inside the open root, so a
        // second workspace cannot be the subject of this batch.
        let correlation = CorrelationId::new();
        let Some((workspace_id, _root)) = self.workspaces.current_open() else {
            return Ok(0);
        };
        let Ok(root) = self.root_of(&workspace_id, &correlation) else {
            // Closed between the tick and here. Nothing to check against.
            return Ok(0);
        };
        let for_event = workspace_id.clone();

        let (recorded, map) = self
            .blocking(move |db| {
                db.write(|tx| {
                    // T223's link, and only it. A session nobody attached to a
                    // piece of work resolves to zero workstreams.
                    let mut candidates: Vec<String> = agent_links::list_for_workspace(tx, &root)?
                        .into_iter()
                        .filter(|link| {
                            link.provider == provider.as_str() && link.session_id == session_id
                        })
                        .map(|link| link.workstream_id)
                        .collect();
                    candidates.sort_unstable();
                    candidates.dedup();
                    if candidates.is_empty() {
                        return Ok((0, None));
                    }

                    let now = now_ms();
                    let held = ownership::list_held(tx, &root)?;
                    let active: Vec<&ownership::HeldLease> = held
                        .iter()
                        .filter(|entry| still_holds(&entry.lease, now))
                        .collect();
                    if active.is_empty() {
                        return Ok((0, None));
                    }

                    // §9: one row per violation, not one per tick. `waiting_conflict`
                    // hardcodes `state = WAITING` and cannot answer this, so the
                    // dedup filters in Rust rather than adding a second query —
                    // and the same set absorbs repeats *within* this batch.
                    let mut seen: HashSet<ObservedKey> =
                        ownership::list_conflicts(tx, &root, MAX_CONFLICTS as i64)?
                            .into_iter()
                            .filter(|row| row.state == ownership::OBSERVED)
                            .map(|row| {
                                (
                                    row.requested_by,
                                    row.holding_lease_id,
                                    row.target_kind,
                                    row.target_pattern,
                                )
                            })
                            .collect();

                    let mut recorded = 0usize;
                    for workstream_id in &candidates {
                        for path in &paths {
                            let target = OwnershipTarget::File { path: path.clone() };
                            for entry in &active {
                                // Overlapping your own ground is not a
                                // violation, exactly as it is not a conflict on
                                // acquisition: nothing is protected from its
                                // holder.
                                if &entry.lease.workstream_id == workstream_id {
                                    continue;
                                }
                                // The one matcher T325 built (rule 2). A stored
                                // target this build cannot parse is skipped
                                // rather than failing the tick, as `acquire`
                                // skips it.
                                let Ok(held_target) = OwnershipTarget::from_storage(
                                    &entry.lease.target_kind,
                                    &entry.lease.target_pattern,
                                ) else {
                                    continue;
                                };
                                if !held_target.overlaps(&target) {
                                    continue;
                                }
                                let key = (
                                    workstream_id.clone(),
                                    entry.lease.id.clone(),
                                    target.kind().to_owned(),
                                    target.as_pattern(),
                                );
                                if !seen.insert(key) {
                                    continue;
                                }
                                ownership::insert_conflict(
                                    tx,
                                    &ownership::ConflictRow {
                                        id: ConflictId::new().to_string(),
                                        workspace_root: root.clone(),
                                        requested_by: workstream_id.clone(),
                                        owned_by: entry.lease.workstream_id.clone(),
                                        holding_lease_id: entry.lease.id.clone(),
                                        target_kind: target.kind().to_owned(),
                                        // The file that was written, not the
                                        // pattern the holder claimed.
                                        target_pattern: target.as_pattern(),
                                        created_at: now,
                                        state: ConflictState::Observed.as_str().to_owned(),
                                        resolved_at: None,
                                    },
                                )?;
                                recorded += 1;
                            }
                        }
                    }

                    // Read inside the transaction that changed it, like every
                    // other command here — and skipped entirely when nothing
                    // changed, which is what stops a 400 ms loop being a 400 ms
                    // event storm.
                    let map = match recorded {
                        0 => None,
                        _ => Some(read_map(tx, &root, &workspace_id, now)?),
                    };
                    Ok((recorded, map))
                })
                .map_err(from_write)
            })
            .await?;

        if let Some(map) = map {
            // `System`, not `Agent`: nothing constructs an `AgentRunId` for a
            // session Gritty only observes (ADR-019 §2), and the row is
            // Gritty's own record rather than the agent's action.
            self.emit(&for_event, &correlation, &Actor::System, map);
        }
        Ok(recorded)
    }

    /// `ownership.resolve`: what the user decided about a refusal.
    ///
    /// `OVERRIDE` releases the holder's lease and says so on the record. It
    /// does not hand the ground to the requester — coding-agent rule 7 forbids
    /// a *silent* override, and an automatic grant would be exactly that, one
    /// step later.
    ///
    /// Two verbs, still, for four states (T326 §11). On an `OBSERVED` row
    /// `CANCEL` reads "acknowledged, stop asking" — there is no request to
    /// withdraw, but dismissing it is what the user wants — and `OVERRIDE` is
    /// `InvalidInput`: it exists to unblock a pending acquisition, and there is
    /// no pending acquisition behind a write that already happened.
    pub async fn resolve_conflict(
        self: &Arc<Self>,
        request: ResolveConflictRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<OwnershipConflict, CoreError> {
        let root = self.root_of(&request.workspace_id, &correlation)?;
        let workspace_id = request.workspace_id.clone();
        let conflict_id = request.conflict_id.to_string();
        let action = request.action;
        let (resolved, map) = self
            .blocking(move |db| {
                db.write(|tx| {
                    let existing = ownership::get_conflict(tx, &conflict_id)?.ok_or_else(|| {
                        not_found(format!("ownership conflict {conflict_id} does not exist"))
                    })?;
                    if existing.workspace_root != root {
                        return Err(not_found(format!(
                            "ownership conflict {conflict_id} does not exist in this workspace"
                        )));
                    }
                    // An override releases a lease so that a *pending* request
                    // can be made again. A violation has no pending request:
                    // the toucher already wrote (T326 §11).
                    if action == ConflictResolution::Override
                        && ConflictState::parse(&existing.state).map_err(domain)?
                            == ConflictState::Observed
                    {
                        return Err(domain(CoreError::invalid_input(
                            "action",
                            "this conflict records a write that already happened, so there is \
                             nothing to unblock; release the lease itself, or dismiss the row",
                        )));
                    }
                    let now = now_ms();
                    let to = match action {
                        ConflictResolution::Cancel => ConflictState::Cancelled,
                        ConflictResolution::Override => ConflictState::Overridden,
                    };
                    if !ownership::close_conflict(tx, &conflict_id, to.as_str(), now)? {
                        return Err(conflict(format!(
                            "ownership conflict {conflict_id} is already {}",
                            existing.state
                        )));
                    }
                    if action == ConflictResolution::Override {
                        ownership::release_lease(tx, &existing.holding_lease_id, now)?;
                        // Anyone else queued behind that lease is unblocked
                        // too, but by a release rather than by an override:
                        // they did not ask for one.
                        ownership::close_conflicts_for_lease(
                            tx,
                            &existing.holding_lease_id,
                            ConflictState::Resolved.as_str(),
                            now,
                            Some(&conflict_id),
                        )?;
                    }
                    let resolved = ownership::ConflictRow {
                        state: to.as_str().to_owned(),
                        resolved_at: Some(now),
                        ..existing
                    };
                    let map = read_map(tx, &root, &workspace_id, now)?;
                    Ok((to_conflict(&resolved, &workspace_id).map_err(domain)?, map))
                })
                .map_err(from_write)
            })
            .await?;
        self.emit(&request.workspace_id, &correlation, &actor, map);
        Ok(resolved)
    }

    /// The canonical root of an open workspace, or `NotFound`. Everything
    /// funnels through here, so a command cannot reach the leases of a
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
                "ownership needs the session database, which is unavailable for this run",
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
            .map_err(|join| CoreError::internal(format!("ownership task failed: {join}")))?
    }

    fn emit(
        &self,
        workspace_id: &WorkspaceId,
        correlation: &CorrelationId,
        actor: &Actor,
        map: OwnershipMap,
    ) {
        self.events.emit(
            &EventOrigin {
                workspace_id: Some(workspace_id.clone()),
                correlation_id: Some(correlation.clone()),
                actor: actor.clone(),
            },
            CoreEvent::OwnershipChanged(map),
        );
    }
}

/// A domain refusal carried out through the persistence layer's error type so
/// it can travel out of a `write` closure, then unwrapped by [`from_write`].
/// SQLite rolls the transaction back on the way out, which is what makes
/// "a refused acquisition granted nothing" true rather than merely intended.
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
    CoreError::new(CoreErrorCode::Io, format!("ownership database: {error}"))
}

fn require_row(
    row: &Option<workstreams::WorkstreamRow>,
    id: &str,
) -> Result<workstreams::WorkstreamRow, PersistenceError> {
    row.clone()
        .ok_or_else(|| not_found(format!("workstream {id} does not exist")))
}

/// A workstream of another workspace is not visible from this one; it reads as
/// missing rather than forbidden.
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

fn ensure_lease_in_workspace(
    row: &ownership::LeaseRow,
    root: &str,
) -> Result<(), PersistenceError> {
    if row.workspace_root == root {
        return Ok(());
    }
    Err(not_found(format!(
        "ownership lease {} does not exist in this workspace",
        row.id
    )))
}

/// A lease that still keeps others out: stored as active, and not past its own
/// expiry. Expiry is applied here rather than stored, so the clock and the
/// database cannot disagree about it.
fn still_holds(row: &ownership::LeaseRow, now: i64) -> bool {
    row.state == ownership::ACTIVE && row.expires_at.is_none_or(|at| at > now)
}

fn millis_to_i64(at: Timestamp) -> i64 {
    i64::try_from(at.as_millis()).unwrap_or(i64::MAX)
}

fn to_timestamp(millis: i64) -> Timestamp {
    Timestamp::from_millis(u64::try_from(millis).unwrap_or(0))
}

fn to_lease(
    entry: &ownership::HeldLease,
    workspace_id: &WorkspaceId,
    now: i64,
) -> Result<OwnershipLease, CoreError> {
    let row = &entry.lease;
    let stored = LeaseState::parse(&row.state)?;
    // A lease is expired when its own deadline has passed, whatever the row
    // says; a released one stays released.
    let state = if stored == LeaseState::Active && !still_holds(row, now) {
        LeaseState::Expired
    } else {
        stored
    };
    // A holder Gritty cannot parse is not silently treated as running work.
    let holder = WorkstreamState::parse(&entry.holder_state)?;
    Ok(OwnershipLease {
        id: LeaseId::from_raw(row.id.clone()),
        workspace_id: workspace_id.clone(),
        workstream_id: WorkstreamId::from_raw(row.workstream_id.clone()),
        agent_run_id: row.agent_run_id.clone().map(AgentRunId::from_raw),
        target: OwnershipTarget::from_storage(&row.target_kind, &row.target_pattern)?,
        mode: OwnershipMode::Write,
        note: row.note.clone(),
        acquired_at: to_timestamp(row.acquired_at),
        expires_at: row.expires_at.map(to_timestamp),
        state,
        stale: holder.is_terminal(),
    })
}

fn to_conflict(
    row: &ownership::ConflictRow,
    workspace_id: &WorkspaceId,
) -> Result<OwnershipConflict, CoreError> {
    Ok(OwnershipConflict {
        id: ConflictId::from_raw(row.id.clone()),
        workspace_id: workspace_id.clone(),
        requested_by: WorkstreamId::from_raw(row.requested_by.clone()),
        owned_by: WorkstreamId::from_raw(row.owned_by.clone()),
        holding_lease_id: LeaseId::from_raw(row.holding_lease_id.clone()),
        target: OwnershipTarget::from_storage(&row.target_kind, &row.target_pattern)?,
        created_at: to_timestamp(row.created_at),
        state: ConflictState::parse(&row.state)?,
        resolved_at: row.resolved_at.map(to_timestamp),
    })
}

fn to_map(
    leases: &[ownership::HeldLease],
    conflicts: &[ownership::ConflictRow],
    workspace_id: &WorkspaceId,
    now: i64,
) -> Result<OwnershipMap, CoreError> {
    Ok(OwnershipMap {
        leases: leases
            .iter()
            .map(|entry| to_lease(entry, workspace_id, now))
            .collect::<Result<Vec<_>, _>>()?,
        conflicts: conflicts
            .iter()
            .map(|row| to_conflict(row, workspace_id))
            .collect::<Result<Vec<_>, _>>()?,
    })
}

/// The fresh snapshot, read inside the transaction that changed it, so the
/// event a command emits cannot describe a different state from the one the
/// command returned.
fn read_map(
    conn: &Connection,
    root: &str,
    workspace_id: &WorkspaceId,
    now: i64,
) -> Result<OwnershipMap, PersistenceError> {
    let leases = ownership::list_held(conn, root)?;
    let conflicts = ownership::list_conflicts(conn, root, MAX_CONFLICTS as i64)?;
    to_map(&leases, &conflicts, workspace_id, now).map_err(domain)
}

#[cfg(test)]
mod tests;
