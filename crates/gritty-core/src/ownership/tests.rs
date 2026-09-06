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
    CreateWorkstreamRequest, RemoveWorkstreamRequest, TransitionWorkstreamRequest, Workstream,
    WorkstreamManager,
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
