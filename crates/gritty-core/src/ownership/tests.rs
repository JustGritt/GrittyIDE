//! T325 manager tests. Everything runs against a real (temporary) SQLite
//! database, like T200's: a registry that does not survive a restart is worse
//! than none, because it is believed, and an in-memory fake would prove
//! nothing about that.
//!
//! §37 of the coordination plan lists what these have to cover — independent
//! files, identical files, directory/file and glob/file overlap, several
//! leases, release, stale leases, canonical path equivalence, traversal — and
//! calls concurrency tests mandatory. Each has a test below.

use std::fs;
use std::path::Path;

use gritty_persistence::testing::TempDir;
use gritty_persistence::{DB_FILE_NAME, Database};
use gritty_workspace::WorkspacePath;
use tokio::runtime::Handle;

use super::*;
use crate::event::testing::RecordingSink;
use crate::operation::OperationRegistry;
use crate::protocol::error::CoreErrorCode;
use crate::workspace::{UnsavedPolicy, Workspace};
use crate::workstream::{
    CreateWorkstreamRequest, LinkAgentRequest, RemoveWorkstreamRequest,
    TransitionWorkstreamRequest, Workstream, WorkstreamManager,
};

struct Harness {
    workspaces: Arc<WorkspaceManager>,
    workstreams: Arc<WorkstreamManager>,
    manager: Arc<OwnershipManager>,
    sink: Arc<RecordingSink>,
}

/// A manager over the database at `db_path`; `None` for "persistence is off".
fn start(db_path: Option<&Path>) -> Harness {
    let database = db_path.map(|path| {
        Arc::new(
            Database::open(path)
                .expect("open the test database")
                .database,
        )
    });
    let sink = RecordingSink::new();
    let events = EventBus::new(sink.clone());
    let registry = OperationRegistry::new(Handle::current(), events.clone());
    let workspaces = WorkspaceManager::new(Handle::current(), events.clone(), registry);
    let workstreams = WorkstreamManager::new(
        Handle::current(),
        events.clone(),
        workspaces.clone(),
        database.clone(),
    );
    let manager = OwnershipManager::new(Handle::current(), events, workspaces.clone(), database);
    Harness {
        workspaces,
        workstreams,
        manager,
        sink,
    }
}

fn workspace_dir(label: &str) -> TempDir {
    let dir = TempDir::new(&format!("own-{label}"));
    fs::write(dir.join("README.md"), "# hi\n").expect("seed the workspace");
    dir
}

fn file(raw: &str) -> OwnershipTarget {
    OwnershipTarget::File {
        path: WorkspacePath::parse(raw).expect("valid path"),
    }
}

fn dir(raw: &str) -> OwnershipTarget {
    OwnershipTarget::Directory {
        path: WorkspacePath::parse(raw).expect("valid path"),
    }
}

fn glob(raw: &str) -> OwnershipTarget {
    OwnershipTarget::Glob {
        pattern: raw.to_owned(),
    }
}

impl Harness {
    fn open_ws(&self, dir: &TempDir) -> Workspace {
        self.workspaces
            .open(
                dir.path(),
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .expect("open the workspace")
    }

    async fn workstream(&self, ws: &Workspace, title: &str) -> Workstream {
        self.workstreams
            .create(
                CreateWorkstreamRequest {
                    workspace_id: ws.id.clone(),
                    title: title.to_owned(),
                    goal: None,
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
            .expect("create the workstream")
    }

    async fn acquire(
        &self,
        ws: &Workspace,
        workstream: &Workstream,
        target: OwnershipTarget,
    ) -> Result<OwnershipAcquisition, CoreError> {
        self.acquire_until(ws, workstream, target, None).await
    }

    async fn acquire_until(
        &self,
        ws: &Workspace,
        workstream: &Workstream,
        target: OwnershipTarget,
        expires_at: Option<Timestamp>,
    ) -> Result<OwnershipAcquisition, CoreError> {
        self.manager
            .acquire(
                AcquireOwnershipRequest {
                    workspace_id: ws.id.clone(),
                    workstream_id: workstream.id.clone(),
                    target,
                    expires_at,
                    note: None,
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
    }

    async fn granted(
        &self,
        ws: &Workspace,
        workstream: &Workstream,
        target: OwnershipTarget,
    ) -> OwnershipLease {
        match self.acquire(ws, workstream, target).await {
            Ok(OwnershipAcquisition::Granted { lease }) => lease,
            other => panic!("expected a granted lease, got {other:?}"),
        }
    }

    async fn refused(
        &self,
        ws: &Workspace,
        workstream: &Workstream,
        target: OwnershipTarget,
    ) -> OwnershipConflict {
        match self.acquire(ws, workstream, target).await {
            Ok(OwnershipAcquisition::Conflicted { conflict }) => conflict,
            other => panic!("expected a conflict, got {other:?}"),
        }
    }

    async fn release(&self, ws: &Workspace, lease: &OwnershipLease) -> Result<(), CoreError> {
        self.manager
            .release(
                ReleaseOwnershipRequest {
                    workspace_id: ws.id.clone(),
                    lease_id: lease.id.clone(),
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
    }

    async fn resolve(
        &self,
        ws: &Workspace,
        conflict: &OwnershipConflict,
        action: ConflictResolution,
    ) -> Result<OwnershipConflict, CoreError> {
        self.manager
            .resolve_conflict(
                ResolveConflictRequest {
                    workspace_id: ws.id.clone(),
                    conflict_id: conflict.id.clone(),
                    action,
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
    }

    async fn map(&self, ws: &Workspace) -> OwnershipMap {
        self.manager
            .list(
                ListOwnershipRequest {
                    workspace_id: ws.id.clone(),
                },
                CorrelationId::new(),
            )
            .await
            .expect("list ownership")
    }

    async fn check(&self, ws: &Workspace, target: OwnershipTarget) -> OwnershipCheck {
        self.manager
            .check(
                CheckOwnershipRequest {
                    workspace_id: ws.id.clone(),
                    target,
                },
                CorrelationId::new(),
            )
            .await
            .expect("check ownership")
    }

    async fn go(&self, ws: &Workspace, workstream: &Workstream, to: WorkstreamState) {
        self.workstreams
            .transition(
                TransitionWorkstreamRequest {
                    workspace_id: ws.id.clone(),
                    workstream_id: workstream.id.clone(),
                    to,
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
            .expect("transition");
    }
}

#[tokio::test]
async fn independent_ground_is_granted_to_both() {
    let db = TempDir::new("own-independent");
    let dir_ = workspace_dir("independent");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;

    harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.granted(&ws, &payments, dir("src/payments")).await;

    let map = harness.map(&ws).await;
    assert_eq!(map.leases.len(), 2);
    assert!(map.conflicts.is_empty());
}

#[tokio::test]
async fn the_same_file_twice_is_a_conflict_that_names_both_sides() {
    let db = TempDir::new("own-same-file");
    let dir_ = workspace_dir("same-file");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;

    let held = harness.granted(&ws, &auth, file("src/api/router.ts")).await;
    let conflict = harness
        .refused(&ws, &payments, file("src/api/router.ts"))
        .await;

    assert_eq!(conflict.requested_by, payments.id);
    assert_eq!(conflict.owned_by, auth.id);
    assert_eq!(conflict.holding_lease_id, held.id);
    assert_eq!(conflict.state, ConflictState::Waiting);
    // The refusal granted nothing: one lease, not two.
    let map = harness.map(&ws).await;
    assert_eq!(map.leases.len(), 1);
    assert_eq!(map.conflicts.len(), 1);
}

#[tokio::test]
async fn a_directory_lease_blocks_a_file_inside_it_and_a_glob_reaching_into_it() {
    let db = TempDir::new("own-overlap");
    let dir_ = workspace_dir("overlap");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let other = harness.workstream(&ws, "Other").await;

    harness.granted(&ws, &auth, dir("src/auth")).await;

    harness
        .refused(&ws, &other, file("src/auth/session.ts"))
        .await;
    harness.refused(&ws, &other, glob("src/**/*.ts")).await;
    harness.refused(&ws, &other, dir("src")).await;
    // Ground the directory does not reach is still free.
    harness.granted(&ws, &other, dir("src/payments")).await;
}

#[tokio::test]
async fn a_glob_lease_blocks_a_file_it_matches() {
    let db = TempDir::new("own-glob");
    let dir_ = workspace_dir("glob");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let docs = harness.workstream(&ws, "Docs").await;
    let other = harness.workstream(&ws, "Other").await;

    harness.granted(&ws, &docs, glob("docs/**/*.md")).await;
    harness
        .refused(&ws, &other, file("docs/architecture/ownership.md"))
        .await;
    harness
        .granted(&ws, &other, file("docs/architecture/ownership.rs"))
        .await;
}

#[tokio::test]
async fn the_root_directory_blocks_everything() {
    let db = TempDir::new("own-root");
    let dir_ = workspace_dir("root");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let sweep = harness.workstream(&ws, "Repository sweep").await;
    let other = harness.workstream(&ws, "Other").await;

    harness.granted(&ws, &sweep, dir("")).await;
    harness.refused(&ws, &other, file("Cargo.toml")).await;
    harness.refused(&ws, &other, glob("**/*.rs")).await;
}

#[tokio::test]
async fn one_workstream_may_hold_several_leases_and_never_blocks_itself() {
    let db = TempDir::new("own-several");
    let dir_ = workspace_dir("several");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;

    harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.granted(&ws, &auth, dir("tests/auth")).await;
    // Overlapping its own ground is not a collision: nothing is protected from
    // its own holder.
    harness
        .granted(&ws, &auth, file("src/auth/session.ts"))
        .await;
    assert_eq!(harness.map(&ws).await.leases.len(), 3);
}

#[tokio::test]
async fn asking_again_for_ground_you_already_hold_returns_the_same_lease() {
    let db = TempDir::new("own-idempotent");
    let dir_ = workspace_dir("idempotent");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;

    let first = harness.granted(&ws, &auth, dir("src/auth")).await;
    let again = harness.granted(&ws, &auth, dir("src/auth")).await;
    assert_eq!(first.id, again.id);
    assert_eq!(harness.map(&ws).await.leases.len(), 1);
}

#[tokio::test]
async fn releasing_resolves_what_was_waiting_on_it_and_frees_the_ground() {
    let db = TempDir::new("own-release");
    let dir_ = workspace_dir("release");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;

    let held = harness.granted(&ws, &auth, file("src/api/router.ts")).await;
    let conflict = harness
        .refused(&ws, &payments, file("src/api/router.ts"))
        .await;

    harness.release(&ws, &held).await.expect("release");

    let map = harness.map(&ws).await;
    assert!(map.leases.is_empty(), "a released lease leaves the map");
    let resolved = map
        .conflicts
        .iter()
        .find(|row| row.id == conflict.id)
        .expect("the conflict is still on the record");
    assert_eq!(resolved.state, ConflictState::Resolved);
    assert!(resolved.resolved_at.is_some());

    // Nothing was granted automatically; asking again now works.
    harness
        .granted(&ws, &payments, file("src/api/router.ts"))
        .await;
}

#[tokio::test]
async fn a_lease_cannot_be_released_twice() {
    let db = TempDir::new("own-twice");
    let dir_ = workspace_dir("twice");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;

    let held = harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.release(&ws, &held).await.expect("release");
    let error = harness.release(&ws, &held).await.unwrap_err();
    assert_eq!(error.code, CoreErrorCode::Conflict);
}

#[tokio::test]
async fn an_expired_lease_stops_holding_and_says_so() {
    let db = TempDir::new("own-expiry");
    let dir_ = workspace_dir("expiry");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let other = harness.workstream(&ws, "Other").await;

    // A deadline already in the past: the lease is born expired, which is the
    // same code path as one that ran out while the app was closed.
    harness
        .acquire_until(&ws, &auth, dir("src/auth"), Some(Timestamp::from_millis(1)))
        .await
        .expect("acquire");

    let map = harness.map(&ws).await;
    let lease = map.leases.first().expect("still listed");
    assert_eq!(lease.state, LeaseState::Expired);
    // It is listed so the user can see and clear it, but it holds nothing.
    harness
        .granted(&ws, &other, file("src/auth/session.ts"))
        .await;
}

#[tokio::test]
async fn a_lease_held_by_finished_work_reads_stale_and_is_still_releasable() {
    let db = TempDir::new("own-stale");
    let dir_ = workspace_dir("stale");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let other = harness.workstream(&ws, "Other").await;

    let held = harness.granted(&ws, &auth, dir("src/auth")).await;
    assert!(!held.stale);

    harness.go(&ws, &auth, WorkstreamState::Ready).await;
    harness.go(&ws, &auth, WorkstreamState::InProgress).await;
    harness.go(&ws, &auth, WorkstreamState::InReview).await;
    harness.go(&ws, &auth, WorkstreamState::Done).await;

    let map = harness.map(&ws).await;
    let lease = map.leases.first().expect("still listed");
    assert!(lease.stale, "finished work still holding ground is stale");
    assert_eq!(lease.state, LeaseState::Active);
    // Stale is reported, never taken away on its own: it still blocks.
    harness
        .refused(&ws, &other, file("src/auth/session.ts"))
        .await;
    harness
        .release(&ws, &held)
        .await
        .expect("release a stale lease");
    harness
        .granted(&ws, &other, file("src/auth/session.ts"))
        .await;
}

#[tokio::test]
async fn removing_a_workstream_takes_its_leases_and_conflicts_with_it() {
    let db = TempDir::new("own-cascade");
    let dir_ = workspace_dir("cascade");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;

    harness.granted(&ws, &auth, file("src/api/router.ts")).await;
    harness
        .refused(&ws, &payments, file("src/api/router.ts"))
        .await;

    harness
        .workstreams
        .remove(
            RemoveWorkstreamRequest {
                workspace_id: ws.id.clone(),
                workstream_id: auth.id.clone(),
            },
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .expect("remove the workstream");

    let map = harness.map(&ws).await;
    assert!(map.leases.is_empty());
    assert!(map.conflicts.is_empty(), "the conflict went with the lease");
    harness
        .granted(&ws, &payments, file("src/api/router.ts"))
        .await;
}

#[tokio::test]
async fn leases_survive_a_restart() {
    let db = TempDir::new("own-restart");
    let dir_ = workspace_dir("restart");
    let db_path = db.join(DB_FILE_NAME);

    let lease_id = {
        let harness = start(Some(&db_path));
        let ws = harness.open_ws(&dir_);
        let auth = harness.workstream(&ws, "Authentication").await;
        harness.granted(&ws, &auth, dir("src/auth")).await.id
    };

    let harness = start(Some(&db_path));
    let ws = harness.open_ws(&dir_);
    let map = harness.map(&ws).await;
    assert_eq!(map.leases.len(), 1);
    assert_eq!(map.leases[0].id, lease_id);
    // And it still holds, which is the point: ownership is not forgotten by a
    // restart the way an in-memory roster is.
    let other = harness.workstream(&ws, "Other").await;
    harness
        .refused(&ws, &other, file("src/auth/session.ts"))
        .await;
}

#[tokio::test]
async fn another_workspace_cannot_see_or_release_these_leases() {
    let db = TempDir::new("own-isolation");
    let mine = workspace_dir("isolation-mine");
    let theirs = workspace_dir("isolation-theirs");
    let harness = start(Some(&db.join(DB_FILE_NAME)));

    let ws = harness.open_ws(&mine);
    let auth = harness.workstream(&ws, "Authentication").await;
    let held = harness.granted(&ws, &auth, dir("src/auth")).await;

    let other_ws = harness.open_ws(&theirs);
    assert!(harness.map(&other_ws).await.leases.is_empty());

    let error = harness
        .manager
        .release(
            ReleaseOwnershipRequest {
                workspace_id: other_ws.id.clone(),
                lease_id: held.id.clone(),
            },
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .unwrap_err();
    // Missing rather than forbidden: the caller has no business knowing it
    // exists.
    assert_eq!(error.code, CoreErrorCode::NotFound);

    // Nor may a workstream of one workspace claim ground in another.
    let error = harness
        .acquire(&other_ws, &auth, dir("src/payments"))
        .await
        .unwrap_err();
    assert_eq!(error.code, CoreErrorCode::NotFound);
}

#[tokio::test]
async fn an_unknown_workstream_or_lease_is_not_found() {
    let db = TempDir::new("own-unknown");
    let dir_ = workspace_dir("unknown");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);

    let error = harness
        .manager
        .acquire(
            AcquireOwnershipRequest {
                workspace_id: ws.id.clone(),
                workstream_id: WorkstreamId::from_raw("nope"),
                target: dir("src"),
                expires_at: None,
                note: None,
            },
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, CoreErrorCode::NotFound);

    let error = harness
        .manager
        .release(
            ReleaseOwnershipRequest {
                workspace_id: ws.id.clone(),
                lease_id: LeaseId::from_raw("nope"),
            },
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, CoreErrorCode::NotFound);
}

#[tokio::test]
async fn a_malformed_target_never_reaches_the_registry() {
    let db = TempDir::new("own-malformed");
    let dir_ = workspace_dir("malformed");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;

    for pattern in ["../../etc/passwd", "/etc/passwd", "src/../..", ""] {
        let error = harness
            .acquire(&ws, &auth, glob(pattern))
            .await
            .unwrap_err();
        assert_eq!(error.code, CoreErrorCode::InvalidInput, "{pattern:?}");
    }
    assert!(harness.map(&ws).await.leases.is_empty());
}

#[tokio::test]
async fn without_a_database_every_command_fails_rather_than_pretending() {
    let dir_ = workspace_dir("nodb");
    let harness = start(None);
    let ws = harness.open_ws(&dir_);

    let error = harness
        .manager
        .list(
            ListOwnershipRequest {
                workspace_id: ws.id.clone(),
            },
            CorrelationId::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, CoreErrorCode::Io);
}

#[tokio::test]
async fn overriding_releases_the_holder_and_stays_on_the_record() {
    let db = TempDir::new("own-override");
    let dir_ = workspace_dir("override");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    let docs = harness.workstream(&ws, "Docs").await;

    harness.granted(&ws, &auth, file("src/api/router.ts")).await;
    let mine = harness
        .refused(&ws, &payments, file("src/api/router.ts"))
        .await;
    let theirs = harness.refused(&ws, &docs, file("src/api/router.ts")).await;

    let resolved = harness
        .resolve(&ws, &mine, ConflictResolution::Override)
        .await
        .expect("override");
    assert_eq!(resolved.state, ConflictState::Overridden);

    let map = harness.map(&ws).await;
    assert!(map.leases.is_empty(), "the holder's lease was released");
    let other = map
        .conflicts
        .iter()
        .find(|row| row.id == theirs.id)
        .expect("the third party is still on the record");
    // Someone else queued behind that lease is unblocked by a release, not by
    // an override they never asked for.
    assert_eq!(other.state, ConflictState::Resolved);

    // Still nothing granted automatically.
    harness
        .granted(&ws, &payments, file("src/api/router.ts"))
        .await;
}

#[tokio::test]
async fn cancelling_withdraws_the_request_and_leaves_the_lease_alone() {
    let db = TempDir::new("own-cancel");
    let dir_ = workspace_dir("cancel");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;

    harness.granted(&ws, &auth, file("src/api/router.ts")).await;
    let conflict = harness
        .refused(&ws, &payments, file("src/api/router.ts"))
        .await;

    let resolved = harness
        .resolve(&ws, &conflict, ConflictResolution::Cancel)
        .await
        .expect("cancel");
    assert_eq!(resolved.state, ConflictState::Cancelled);
    assert_eq!(harness.map(&ws).await.leases.len(), 1);

    // And a closed conflict cannot be closed again.
    let error = harness
        .resolve(&ws, &conflict, ConflictResolution::Override)
        .await
        .unwrap_err();
    assert_eq!(error.code, CoreErrorCode::Conflict);
}

#[tokio::test]
async fn the_same_refusal_asked_twice_is_one_row_in_the_queue() {
    let db = TempDir::new("own-repeat");
    let dir_ = workspace_dir("repeat");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;

    harness.granted(&ws, &auth, file("src/api/router.ts")).await;
    let first = harness
        .refused(&ws, &payments, file("src/api/router.ts"))
        .await;
    let again = harness
        .refused(&ws, &payments, file("src/api/router.ts"))
        .await;
    assert_eq!(first.id, again.id);
    assert_eq!(harness.map(&ws).await.conflicts.len(), 1);
}

#[tokio::test]
async fn check_names_the_holder_of_the_ground_and_stays_silent_about_free_ground() {
    let db = TempDir::new("own-check");
    let dir_ = workspace_dir("check");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;

    // Two leases of one workstream may overlap; two workstreams' may not, so
    // more than one holder always means one workstream holding twice.
    harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.granted(&ws, &auth, glob("**/*.ts")).await;

    let answer = harness.check(&ws, file("src/auth/session.ts")).await;
    assert_eq!(answer.holders.len(), 2);
    assert!(
        answer
            .holders
            .iter()
            .all(|lease| lease.workstream_id == auth.id)
    );

    // Free ground has no holders at all.
    assert!(
        harness
            .check(&ws, file("docs/readme.md"))
            .await
            .holders
            .is_empty()
    );
    // A released lease stops answering.
    for lease in answer.holders {
        harness.release(&ws, &lease).await.expect("release");
    }
    assert!(
        harness
            .check(&ws, file("src/auth/session.ts"))
            .await
            .holders
            .is_empty()
    );
}

#[tokio::test]
async fn every_change_publishes_the_map_it_produced() {
    let db = TempDir::new("own-events");
    let dir_ = workspace_dir("events");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;

    let held = harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.release(&ws, &held).await.expect("release");
    // Reads publish nothing.
    harness.map(&ws).await;

    let maps: Vec<OwnershipMap> = harness
        .sink
        .recorded()
        .into_iter()
        .filter_map(|envelope| match envelope.event {
            CoreEvent::OwnershipChanged(map) => Some(map),
            _ => None,
        })
        .collect();
    assert_eq!(maps.len(), 2, "one per mutation, none per read");
    assert_eq!(maps[0].leases.len(), 1);
    assert!(maps[1].leases.is_empty());
}

#[tokio::test]
async fn the_registry_is_bounded() {
    let db = TempDir::new("own-bounded");
    let dir_ = workspace_dir("bounded");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;

    for index in 0..MAX_LEASES {
        harness
            .granted(&ws, &auth, file(&format!("src/file-{index}.ts")))
            .await;
    }
    let error = harness
        .acquire(&ws, &auth, file("src/one-too-many.ts"))
        .await
        .unwrap_err();
    assert_eq!(error.code, CoreErrorCode::Conflict);
}

/// §37: *"The ownership registry must not allow two concurrent acquisition
/// requests to both receive incompatible ownership."*
///
/// Both requests run on blocking threads of a multi-threaded runtime; the
/// check and the insert share one transaction, so the loser's check has
/// already seen the winner's row.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_requests_for_one_file_grant_exactly_one() {
    let db = TempDir::new("own-race");
    let dir_ = workspace_dir("race");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;

    // Overlapping, not identical: a directory against a file inside it, which
    // no unique index could have caught.
    let (left, right) = tokio::join!(
        harness.acquire(&ws, &auth, dir("src/api")),
        harness.acquire(&ws, &payments, file("src/api/router.ts")),
    );

    let granted = [&left, &right]
        .iter()
        .filter(|outcome| matches!(outcome, Ok(OwnershipAcquisition::Granted { .. })))
        .count();
    let refused = [&left, &right]
        .iter()
        .filter(|outcome| matches!(outcome, Ok(OwnershipAcquisition::Conflicted { .. })))
        .count();
    assert_eq!(granted, 1, "exactly one winner: {left:?} / {right:?}");
    assert_eq!(refused, 1, "and exactly one refusal");

    let map = harness.map(&ws).await;
    assert_eq!(map.leases.len(), 1, "the table holds one lease");
    assert_eq!(map.conflicts.len(), 1);
}

/// The same race, repeated, because a lock that usually works is not a lock.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_race_has_one_winner_every_time() {
    let db = TempDir::new("own-race-many");
    let dir_ = workspace_dir("race-many");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;

    for round in 0..20 {
        let target = format!("src/round-{round}");
        let inside = format!("{target}/router.ts");
        let (left, right) = tokio::join!(
            harness.acquire(&ws, &auth, dir(&target)),
            harness.acquire(&ws, &payments, file(&inside)),
        );
        let granted = [&left, &right]
            .iter()
            .filter(|outcome| matches!(outcome, Ok(OwnershipAcquisition::Granted { .. })))
            .count();
        assert_eq!(granted, 1, "round {round}: {left:?} / {right:?}");
    }
    assert_eq!(harness.map(&ws).await.leases.len(), 20);
}

/// Path identity is `WorkspacePath`'s, which is the repository's one
/// definition of it — the same one tabs, snapshots and documents key on. It
/// does no case folding, which is a documented limitation rather than an
/// accident, so this pins the behaviour rather than wishing it away.
#[tokio::test]
async fn path_identity_is_the_one_the_rest_of_the_product_uses() {
    let db = TempDir::new("own-identity");
    let dir_ = workspace_dir("identity");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let other = harness.workstream(&ws, "Other").await;

    harness
        .granted(&ws, &auth, file("src/auth/Session.ts"))
        .await;
    // Same bytes, same file.
    harness
        .refused(&ws, &other, file("src/auth/Session.ts"))
        .await;
    // Different bytes. On a case-insensitive volume these are one file on disk
    // and two leases here; `docs/architecture/ownership.md` says so.
    harness
        .granted(&ws, &other, file("src/auth/session.ts"))
        .await;
}

// ---------------------------------------------------------------------------
// T326: noticing a violation after it happened
//
// Everything below is a pure function of leases, links and touched paths, so
// each row of the task's failure table that is one gets a test here. What is
// *not* here, deliberately: the ring-overflow row and the whole-tick path,
// which are `agent_session`'s to exercise, because `absorb` is what truncates.
// ---------------------------------------------------------------------------

/// The roster's id for a discovered Claude Code session. Its shape does not
/// matter to the detector — only that the link and the batch agree on it.
const TOUCHER: &str = "claude_code:eca001ac-5b6f-40ff-b79a-9ae79ccc6483";

impl Harness {
    async fn link(&self, ws: &Workspace, workstream: &Workstream, session_id: &str) {
        self.workstreams
            .link_agent(
                LinkAgentRequest {
                    workspace_id: ws.id.clone(),
                    workstream_id: workstream.id.clone(),
                    provider: AgentProvider::ClaudeCode,
                    session_id: session_id.to_owned(),
                },
                CorrelationId::new(),
            )
            .await
            .expect("link the session to the workstream");
    }

    /// One tick's worth of touches, as the rail hands them over.
    async fn observe(&self, session_id: &str, paths: &[&str]) -> usize {
        self.manager
            .observe_touches(
                AgentProvider::ClaudeCode,
                session_id.to_owned(),
                paths
                    .iter()
                    .map(|raw| WorkspacePath::parse(raw).expect("valid path"))
                    .collect(),
            )
            .await
            .expect("observe the touches")
    }

    async fn observed(&self, ws: &Workspace) -> Vec<OwnershipConflict> {
        self.map(ws)
            .await
            .conflicts
            .into_iter()
            .filter(|conflict| conflict.state == ConflictState::Observed)
            .collect()
    }

    fn ownership_events(&self) -> usize {
        self.sink
            .recorded()
            .iter()
            .filter(|envelope| matches!(envelope.event, CoreEvent::OwnershipChanged(_)))
            .count()
    }
}

/// The acceptance criterion, in one test: a session linked to one workstream
/// writes inside another's lease, and that is one row, correctly attributed,
/// one event — and still one row ten ticks later.
#[tokio::test]
async fn a_write_on_another_workstreams_ground_is_recorded_once() {
    let db = TempDir::new("own-observe");
    let dir_ = workspace_dir("observe");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    let lease = harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.link(&ws, &payments, TOUCHER).await;

    let before = harness.ownership_events();
    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 1);
    assert_eq!(
        harness.ownership_events() - before,
        1,
        "one violation, one OwnershipChanged (rule 8)"
    );

    let conflicts = harness.observed(&ws).await;
    assert_eq!(conflicts.len(), 1);
    let conflict = &conflicts[0];
    assert_eq!(
        conflict.requested_by, payments.id,
        "the toucher asked for nothing, but it is who wrote"
    );
    assert_eq!(conflict.owned_by, auth.id);
    assert_eq!(conflict.holding_lease_id, lease.id);
    assert_eq!(
        conflict.target,
        file("src/auth/session.ts"),
        "the file that was written, not the folder that was claimed"
    );
    assert!(conflict.resolved_at.is_none());

    // The same violation on every later tick is still one row (§9), and a tick
    // that finds nothing new emits nothing at all.
    let before = harness.ownership_events();
    for _ in 0..10 {
        assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 0);
    }
    assert_eq!(harness.observed(&ws).await.len(), 1);
    assert_eq!(harness.ownership_events(), before, "ten silent ticks");
}

/// A session nobody attached to a piece of work resolves to zero workstreams.
/// T325's own follow-up note says it: a violation nobody can attribute is not
/// one. It is not an error either.
#[tokio::test]
async fn an_unlinked_session_touching_owned_ground_is_nobodys_violation() {
    let db = TempDir::new("own-unlinked");
    let dir_ = workspace_dir("unlinked");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    harness.granted(&ws, &auth, dir("src/auth")).await;

    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 0);
    assert!(harness.observed(&ws).await.is_empty());
}

/// Attribution keys on the *pair*, not the session id alone: T223's primary key
/// is the triple precisely so two providers reusing an id cannot collide. One
/// provider exists today, so this asserts the key rather than a second one.
#[tokio::test]
async fn attribution_is_by_session_and_not_by_anything_else() {
    let db = TempDir::new("own-provider");
    let dir_ = workspace_dir("provider");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.link(&ws, &payments, TOUCHER).await;

    // A different session id under the same provider is a different session,
    // and no working directory or title is consulted to guess otherwise.
    assert_eq!(
        harness
            .observe("claude_code:someone-else", &["src/auth/session.ts"])
            .await,
        0
    );
    assert!(harness.observed(&ws).await.is_empty());
}

/// Nothing is protected from its holder — the rule `acquire` already applies to
/// an overlapping request from the workstream that holds the ground.
#[tokio::test]
async fn a_write_on_your_own_ground_is_not_a_violation() {
    let db = TempDir::new("own-self");
    let dir_ = workspace_dir("self");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.link(&ws, &auth, TOUCHER).await;

    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 0);
    assert!(harness.observed(&ws).await.is_empty());
}

/// One row per (workstream, lease) pair, deduped independently.
///
/// Two *different* workstreams can never both cover one file — `acquire`
/// refuses the second, which is the whole point of T325 — so the only way one
/// touch overlaps several leases is one workstream holding several, which
/// `acquire` allows because nothing is protected from its holder. Each lease is
/// a separate thing the user may release, so each gets its own row.
#[tokio::test]
async fn a_write_under_two_leases_of_one_holder_is_one_row_for_each() {
    let db = TempDir::new("own-two-leases");
    let dir_ = workspace_dir("two-leases");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    let folder = harness.granted(&ws, &auth, dir("src/auth")).await;
    let pattern = harness.granted(&ws, &auth, glob("src/**/*.ts")).await;
    harness.link(&ws, &payments, TOUCHER).await;

    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 2);
    let mut leases: Vec<LeaseId> = harness
        .observed(&ws)
        .await
        .into_iter()
        .map(|conflict| conflict.holding_lease_id)
        .collect();
    leases.sort_by_key(ToString::to_string);
    let mut expected = vec![folder.id, pattern.id];
    expected.sort_by_key(ToString::to_string);
    assert_eq!(leases, expected);

    // Each pair is deduped on its own, so a repeat adds neither.
    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 0);
    assert_eq!(harness.observed(&ws).await.len(), 2);
}

/// Two files, one lease: the target on the row is the file that was written, so
/// two different files under one folder claim are two different rows.
#[tokio::test]
async fn two_files_under_one_lease_are_two_rows() {
    let db = TempDir::new("own-two-files");
    let dir_ = workspace_dir("two-files");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.link(&ws, &payments, TOUCHER).await;

    let recorded = harness
        .observe(TOUCHER, &["src/auth/session.ts", "src/auth/token.ts"])
        .await;
    assert_eq!(recorded, 2);
    // The same batch twice adds nothing, and neither does the same file twice
    // inside one batch.
    assert_eq!(
        harness
            .observe(
                TOUCHER,
                &[
                    "src/auth/session.ts",
                    "src/auth/session.ts",
                    "src/auth/token.ts"
                ],
            )
            .await,
        0
    );
    assert_eq!(harness.observed(&ws).await.len(), 2);
}

/// T223's table allows one session on several workstreams. The check runs once
/// per link, so each of them is told.
#[tokio::test]
async fn a_session_on_two_workstreams_is_checked_once_per_link() {
    let db = TempDir::new("own-two-links");
    let dir_ = workspace_dir("two-links");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    let billing = harness.workstream(&ws, "Billing").await;
    harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.link(&ws, &payments, TOUCHER).await;
    harness.link(&ws, &billing, TOUCHER).await;

    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 2);
    let askers: Vec<WorkstreamId> = harness
        .observed(&ws)
        .await
        .into_iter()
        .map(|conflict| conflict.requested_by)
        .collect();
    assert!(askers.contains(&payments.id) && askers.contains(&billing.id));
}

/// Ground nobody holds refuses nobody: the same `still_holds` test
/// `ownership.check` uses, so an expired lease is excluded here exactly as it
/// is excluded there.
#[tokio::test]
async fn an_expired_lease_is_not_violated() {
    let db = TempDir::new("own-expired");
    let dir_ = workspace_dir("expired");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    harness
        .acquire_until(&ws, &auth, dir("src/auth"), Some(Timestamp::from_millis(1)))
        .await
        .expect("acquire an already-expired lease");
    harness.link(&ws, &payments, TOUCHER).await;

    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 0);
    assert!(harness.observed(&ws).await.is_empty());
}

/// A released lease is not held either, so the write landed on free ground.
#[tokio::test]
async fn a_lease_released_before_the_check_is_not_violated() {
    let db = TempDir::new("own-released");
    let dir_ = workspace_dir("released");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    let lease = harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.link(&ws, &payments, TOUCHER).await;
    harness.release(&ws, &lease).await.expect("release");

    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 0);
    assert!(harness.observed(&ws).await.is_empty());
}

/// Releasing the violated lease closes its observed row, and leaves an
/// unrelated refusal on another lease exactly where it was. This is one of the
/// two widened `WHERE` clauses, asserted from the domain side.
#[tokio::test]
async fn releasing_a_violated_lease_resolves_its_observed_row() {
    let db = TempDir::new("own-close");
    let dir_ = workspace_dir("close");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let infra = harness.workstream(&ws, "Infrastructure").await;
    let payments = harness.workstream(&ws, "Payments").await;
    let violated = harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.granted(&ws, &infra, dir("src/infra")).await;
    // An ordinary refusal on a *different* lease, which must survive.
    let waiting = harness
        .refused(&ws, &payments, file("src/infra/db.ts"))
        .await;
    harness.link(&ws, &payments, TOUCHER).await;
    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 1);

    harness.release(&ws, &violated).await.expect("release");

    let conflicts = harness.map(&ws).await.conflicts;
    let observed = conflicts
        .iter()
        .find(|conflict| conflict.holding_lease_id == violated.id)
        .expect("the observed row is still listed");
    assert_eq!(observed.state, ConflictState::Resolved);
    assert!(observed.resolved_at.is_some());
    let untouched = conflicts
        .iter()
        .find(|conflict| conflict.id == waiting.id)
        .expect("the unrelated refusal is still listed");
    assert_eq!(
        untouched.state,
        ConflictState::Waiting,
        "another lease's refusal is nobody else's business"
    );
}

/// Overriding a refusal on the violated lease resolves what was observed on it
/// too — the other widened clause, reached the other way.
#[tokio::test]
async fn overriding_the_violated_lease_resolves_its_observed_row() {
    let db = TempDir::new("own-override-close");
    let dir_ = workspace_dir("override-close");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    harness.granted(&ws, &auth, dir("src/auth")).await;
    // Payments asks properly and is refused; it also wrote there anyway.
    let refusal = harness
        .refused(&ws, &payments, file("src/auth/token.ts"))
        .await;
    harness.link(&ws, &payments, TOUCHER).await;
    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 1);

    harness
        .resolve(&ws, &refusal, ConflictResolution::Override)
        .await
        .expect("override the refusal");

    let observed = harness
        .map(&ws)
        .await
        .conflicts
        .into_iter()
        .find(|conflict| conflict.target == file("src/auth/session.ts"))
        .expect("the observed row");
    assert_eq!(observed.state, ConflictState::Resolved);
}

/// `CANCEL` on a violation means "acknowledged, stop asking": there is no
/// request to withdraw, but dismissing the row is the effect the user wants.
#[tokio::test]
async fn cancel_dismisses_an_observed_violation() {
    let db = TempDir::new("own-dismiss");
    let dir_ = workspace_dir("dismiss");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.link(&ws, &payments, TOUCHER).await;
    harness.observe(TOUCHER, &["src/auth/session.ts"]).await;
    let observed = harness.observed(&ws).await.remove(0);

    let resolved = harness
        .resolve(&ws, &observed, ConflictResolution::Cancel)
        .await
        .expect("dismiss the violation");
    assert_eq!(resolved.state, ConflictState::Cancelled);

    // Dismissed — and a later write raises it again, because the dedup set
    // holds only rows that are still open. Acknowledging is not silencing.
    assert!(harness.observed(&ws).await.is_empty());
    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 1);
}

/// An override releases a lease so that a *pending* request can be made again.
/// A violation has no pending request, so there is nothing to unblock.
#[tokio::test]
async fn override_on_an_observed_violation_is_invalid_input() {
    let db = TempDir::new("own-no-override");
    let dir_ = workspace_dir("no-override");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    let lease = harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.link(&ws, &payments, TOUCHER).await;
    harness.observe(TOUCHER, &["src/auth/session.ts"]).await;
    let observed = harness.observed(&ws).await.remove(0);

    let error = harness
        .resolve(&ws, &observed, ConflictResolution::Override)
        .await
        .expect_err("an override has nothing to unblock here");
    assert_eq!(error.code, CoreErrorCode::InvalidInput);

    // And it changed nothing: the lease it names still holds, and the row is
    // still open. A refusal that half-applied would be the worse failure.
    let map = harness.map(&ws).await;
    let still = map
        .leases
        .iter()
        .find(|held| held.id == lease.id)
        .expect("the lease is still there");
    assert_eq!(still.state, LeaseState::Active);
    assert_eq!(harness.observed(&ws).await.len(), 1);
}

/// No session database: the check finds nothing and fails nothing.
#[tokio::test]
async fn no_database_finds_no_violation() {
    let dir_ = workspace_dir("no-db");
    let harness = start(None);
    let _ws = harness.open_ws(&dir_);
    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 0);
}

/// Nothing to check: an empty batch never reaches the database at all, which is
/// the common case on a 400 ms loop.
#[tokio::test]
async fn a_tick_with_no_touched_files_does_nothing() {
    let db = TempDir::new("own-empty");
    let dir_ = workspace_dir("empty");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.link(&ws, &payments, TOUCHER).await;

    let before = harness.ownership_events();
    assert_eq!(harness.observe(TOUCHER, &[]).await, 0);
    assert_eq!(harness.ownership_events(), before);
}

/// A workstream removed under a violation takes the row with it: a conflict
/// between two pieces of work is meaningless once one of them is gone. T325
/// chose that cascade; reusing its row is what inherits it.
#[tokio::test]
async fn removing_the_toucher_removes_its_observed_rows() {
    let db = TempDir::new("own-cascade");
    let dir_ = workspace_dir("cascade");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir_);
    let auth = harness.workstream(&ws, "Authentication").await;
    let payments = harness.workstream(&ws, "Payments").await;
    harness.granted(&ws, &auth, dir("src/auth")).await;
    harness.link(&ws, &payments, TOUCHER).await;
    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 1);

    harness
        .workstreams
        .remove(
            RemoveWorkstreamRequest {
                workspace_id: ws.id.clone(),
                workstream_id: payments.id.clone(),
            },
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .expect("remove the workstream");

    assert!(harness.observed(&ws).await.is_empty());
}

/// No workspace open: there is no registry to check against, because leases are
/// keyed by workspace root. Not an error — the agent rail is machine-wide and
/// keeps running with no workspace at all.
#[tokio::test]
async fn no_open_workspace_finds_no_violation() {
    let db = TempDir::new("own-no-ws");
    let harness = start(Some(&db.join(DB_FILE_NAME)));
    assert_eq!(harness.observe(TOUCHER, &["src/auth/session.ts"]).await, 0);
}
