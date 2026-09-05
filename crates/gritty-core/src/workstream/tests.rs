//! T200 manager tests. Everything runs against a real (temporary) SQLite
//! database: the point of Gate C is that the model survives a restart, and an
//! in-memory fake would prove nothing about that.

use std::fs;
use std::path::Path;

use gritty_persistence::testing::TempDir;
use gritty_persistence::{DB_FILE_NAME, Database};
use tokio::runtime::Handle;

use super::*;
use crate::event::testing::RecordingSink;
use crate::operation::OperationRegistry;
use crate::protocol::error::CoreErrorCode;
use crate::workspace::{UnsavedPolicy, Workspace};

struct Harness {
    workspaces: Arc<WorkspaceManager>,
    manager: Arc<WorkstreamManager>,
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
    let manager = WorkstreamManager::new(Handle::current(), events, workspaces.clone(), database);
    Harness {
        workspaces,
        manager,
        sink,
    }
}

fn workspace_dir(label: &str) -> TempDir {
    let dir = TempDir::new(&format!("ws-{label}"));
    fs::write(dir.join("README.md"), "# hi\n").expect("seed the workspace");
    dir
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

    async fn create(&self, ws: &Workspace, title: &str) -> Workstream {
        self.manager
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

    async fn list(&self, ws: &Workspace) -> Vec<Workstream> {
        self.manager
            .list(
                ListWorkstreamsRequest {
                    workspace_id: ws.id.clone(),
                },
                CorrelationId::new(),
            )
            .await
            .expect("list workstreams")
    }

    async fn go(
        &self,
        ws: &Workspace,
        workstream: &Workstream,
        to: WorkstreamState,
    ) -> Result<Workstream, CoreError> {
        self.manager
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
    }

    /// Draft → Ready → InProgress, the path everything else starts from.
    async fn start_working(&self, ws: &Workspace, title: &str) -> Workstream {
        let workstream = self.create(ws, title).await;
        self.go(ws, &workstream, WorkstreamState::Ready)
            .await
            .expect("ready");
        self.go(ws, &workstream, WorkstreamState::InProgress)
            .await
            .expect("in progress")
    }

    async fn raise(
        &self,
        ws: &Workspace,
        workstream: &Workstream,
        question: &str,
        options: Vec<DecisionOption>,
    ) -> Result<Decision, CoreError> {
        self.manager
            .raise_decision(
                RaiseDecisionRequest {
                    workspace_id: ws.id.clone(),
                    workstream_id: workstream.id.clone(),
                    question: question.to_owned(),
                    options,
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
    }

    async fn resolve(
        &self,
        ws: &Workspace,
        decision: &Decision,
        chosen: Option<&str>,
    ) -> Result<Decision, CoreError> {
        self.manager
            .resolve_decision(
                ResolveDecisionRequest {
                    workspace_id: ws.id.clone(),
                    decision_id: decision.id.clone(),
                    chosen: chosen.map(str::to_owned),
                    rationale: None,
                },
                Actor::User,
                CorrelationId::new(),
            )
            .await
    }

    async fn one(&self, ws: &Workspace, id: &WorkstreamId) -> Workstream {
        self.list(ws)
            .await
            .into_iter()
            .find(|workstream| workstream.id == *id)
            .expect("the workstream is still listed")
    }
}

fn option(key: &str, label: &str) -> DecisionOption {
    DecisionOption {
        key: key.to_owned(),
        label: label.to_owned(),
    }
}

#[tokio::test]
async fn workstreams_survive_a_restart_with_their_decisions() {
    let data = TempDir::new("restart");
    let db = data.join(DB_FILE_NAME);
    let dir = workspace_dir("restart");

    let created = {
        let harness = start(Some(&db));
        let ws = harness.open_ws(&dir);
        let workstream = harness.start_working(&ws, "Terminal profiles").await;
        harness
            .raise(&ws, &workstream, "Which shell by default?", Vec::new())
            .await
            .expect("raise");
        harness.create(&ws, "Second").await;
        workstream
    };

    // A second process over the same file: nothing was cached in the first.
    let harness = start(Some(&db));
    let ws = harness.open_ws(&dir);
    let listed = harness.list(&ws).await;
    assert_eq!(listed.len(), 2);
    let reloaded = harness.one(&ws, &created.id).await;
    assert_eq!(reloaded.title, "Terminal profiles");
    assert_eq!(
        reloaded.state,
        WorkstreamState::Blocked,
        "the question it was blocked on outlived the process"
    );
    assert_eq!(reloaded.open_decisions, 1);

    let decisions = harness
        .manager
        .list_decisions(
            ListDecisionsRequest {
                workspace_id: ws.id.clone(),
                workstream_id: None,
                open_only: Some(true),
            },
            CorrelationId::new(),
        )
        .await
        .expect("list decisions");
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].question, "Which shell by default?");
    assert_eq!(decisions[0].raised_by, Actor::User);
}

#[tokio::test]
async fn a_new_workstream_starts_as_a_draft_and_announces_itself() {
    let data = TempDir::new("create");
    let dir = workspace_dir("create");
    let harness = start(Some(&data.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir);

    let workstream = harness.create(&ws, "  Search index  ").await;
    assert_eq!(workstream.title, "Search index", "titles are trimmed");
    assert_eq!(workstream.state, WorkstreamState::Draft);
    assert_eq!(
        workstream.allowed_transitions,
        [WorkstreamState::Ready, WorkstreamState::Abandoned],
        "the UI is told what it may offer, so it cannot invent an illegal move"
    );
    assert_eq!(workstream.open_decisions, 0);
    assert_eq!(workstream.workspace_id, ws.id);

    let recorded = harness.sink.recorded();
    let event = recorded
        .iter()
        .rfind(|envelope| envelope.event.name() == "WorkstreamCreated")
        .expect("WorkstreamCreated was emitted");
    assert_eq!(event.meta.actor, Actor::User);
    assert_eq!(event.meta.workspace_id.as_ref(), Some(&ws.id));
    assert!(event.meta.correlation_id.is_some());
}

#[tokio::test]
async fn an_illegal_transition_is_refused_and_changes_nothing() {
    let data = TempDir::new("illegal");
    let dir = workspace_dir("illegal");
    let harness = start(Some(&data.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir);
    let workstream = harness.create(&ws, "Diagnostics").await;

    let error = harness
        .go(&ws, &workstream, WorkstreamState::Done)
        .await
        .expect_err("a draft cannot be finished");
    assert_eq!(error.code, CoreErrorCode::Conflict);

    let after = harness.one(&ws, &workstream.id).await;
    assert_eq!(after.state, WorkstreamState::Draft);
    assert_eq!(
        after.updated_at, workstream.updated_at,
        "a refused transition does not even touch updated_at"
    );
    assert!(
        !harness.sink.names().contains(&"WorkstreamChanged"),
        "nothing changed, so nothing was announced"
    );
}

#[tokio::test]
async fn every_legal_transition_is_accepted_end_to_end() {
    let data = TempDir::new("legal");
    let dir = workspace_dir("legal");
    let harness = start(Some(&data.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir);
    let workstream = harness.create(&ws, "Watcher").await;

    // The full happy path, plus the two ways back from a terminal state.
    for to in [
        WorkstreamState::Ready,
        WorkstreamState::InProgress,
        WorkstreamState::InReview,
        WorkstreamState::Done,
        WorkstreamState::InProgress,
        WorkstreamState::Abandoned,
        WorkstreamState::Draft,
    ] {
        let moved = harness
            .go(&ws, &workstream, to)
            .await
            .unwrap_or_else(|error| panic!("{to:?} should be reachable: {error}"));
        assert_eq!(moved.state, to);
        assert_eq!(harness.one(&ws, &workstream.id).await.state, to);
    }
}

#[tokio::test]
async fn a_question_blocks_the_work_and_the_last_answer_resumes_it() {
    let data = TempDir::new("block");
    let dir = workspace_dir("block");
    let harness = start(Some(&data.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir);
    let workstream = harness.start_working(&ws, "Diagnostics").await;

    let first = harness
        .raise(
            &ws,
            &workstream,
            "Batch in the core or the renderer?",
            vec![
                option("core", "Core, bounded queue per document"),
                option("renderer", "Renderer, throttled by frame"),
            ],
        )
        .await
        .expect("raise the first");
    assert_eq!(first.state, DecisionState::Open);
    let blocked = harness.one(&ws, &workstream.id).await;
    assert_eq!(blocked.state, WorkstreamState::Blocked);
    assert_eq!(blocked.open_decisions, 1);

    // A second question while already blocked does not move anything.
    let second = harness
        .raise(&ws, &workstream, "Do we need an ADR first?", Vec::new())
        .await
        .expect("raise the second");
    let still = harness.one(&ws, &workstream.id).await;
    assert_eq!(still.state, WorkstreamState::Blocked);
    assert_eq!(still.open_decisions, 2);

    let answered = harness
        .resolve(&ws, &first, Some("core"))
        .await
        .expect("answer the first");
    assert_eq!(answered.state, DecisionState::Resolved);
    assert_eq!(answered.chosen.as_deref(), Some("core"));
    assert!(answered.resolved_at.is_some());
    assert_eq!(
        harness.one(&ws, &workstream.id).await.state,
        WorkstreamState::Blocked,
        "one answer down, one still open"
    );

    harness
        .resolve(&ws, &second, Some("yes, ADR-017"))
        .await
        .expect("answer the second");
    let resumed = harness.one(&ws, &workstream.id).await;
    assert_eq!(
        resumed.state,
        WorkstreamState::InProgress,
        "the last answer releases the work"
    );
    assert_eq!(resumed.open_decisions, 0);

    let names = harness.sink.names();
    assert_eq!(
        names.iter().filter(|n| **n == "DecisionRequested").count(),
        2
    );
    assert_eq!(
        names.iter().filter(|n| **n == "DecisionResolved").count(),
        2
    );
}

#[tokio::test]
async fn leaving_blocked_by_hand_keeps_the_questions_open() {
    let data = TempDir::new("manual-unblock");
    let dir = workspace_dir("manual-unblock");
    let harness = start(Some(&data.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir);
    let workstream = harness.start_working(&ws, "Search").await;
    harness
        .raise(&ws, &workstream, "Which walker?", Vec::new())
        .await
        .expect("raise");

    let moved = harness
        .go(&ws, &workstream, WorkstreamState::InProgress)
        .await
        .expect("the supervisor may proceed anyway");
    assert_eq!(moved.state, WorkstreamState::InProgress);
    assert_eq!(
        moved.open_decisions, 1,
        "the question is still waiting for an answer"
    );
}

#[tokio::test]
async fn an_answer_must_be_one_of_the_offered_options() {
    let data = TempDir::new("options");
    let dir = workspace_dir("options");
    let harness = start(Some(&data.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir);
    let workstream = harness.start_working(&ws, "Diagnostics").await;
    let decision = harness
        .raise(&ws, &workstream, "Where?", vec![option("core", "Core")])
        .await
        .expect("raise");

    let invented = harness
        .resolve(&ws, &decision, Some("somewhere else"))
        .await
        .expect_err("not one of the options");
    assert_eq!(invented.code, CoreErrorCode::InvalidInput);
    assert_eq!(invented.details.unwrap()["field"], "chosen");

    let empty = harness
        .resolve(&ws, &decision, None)
        .await
        .expect_err("an answer is required");
    assert_eq!(empty.code, CoreErrorCode::InvalidInput);

    // Still open: neither refusal wrote anything.
    assert_eq!(harness.one(&ws, &workstream.id).await.open_decisions, 1);

    harness
        .resolve(&ws, &decision, Some("core"))
        .await
        .expect("a real option is accepted");
    let again = harness
        .resolve(&ws, &decision, Some("core"))
        .await
        .expect_err("already answered");
    assert_eq!(again.code, CoreErrorCode::Conflict);
}

#[tokio::test]
async fn dismissing_records_no_answer() {
    let data = TempDir::new("dismiss");
    let dir = workspace_dir("dismiss");
    let harness = start(Some(&data.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir);
    let workstream = harness.start_working(&ws, "Git").await;
    let decision = harness
        .raise(
            &ws,
            &workstream,
            "Rebase or merge?",
            vec![option("r", "Rebase")],
        )
        .await
        .expect("raise");

    let dismissed = harness
        .manager
        .dismiss_decision(
            DismissDecisionRequest {
                workspace_id: ws.id.clone(),
                decision_id: decision.id.clone(),
                rationale: Some("  the branch is gone  ".to_owned()),
            },
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .expect("dismiss");
    assert_eq!(dismissed.state, DecisionState::Dismissed);
    assert_eq!(dismissed.chosen, None, "dismissal invents no answer");
    assert_eq!(dismissed.rationale.as_deref(), Some("the branch is gone"));
    assert_eq!(
        harness.one(&ws, &workstream.id).await.state,
        WorkstreamState::InProgress,
        "a dismissed question stops blocking too"
    );
}

#[tokio::test]
async fn blank_and_oversized_input_is_refused_with_the_field_named() {
    let data = TempDir::new("validation");
    let dir = workspace_dir("validation");
    let harness = start(Some(&data.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir);

    let blank = harness
        .manager
        .create(
            CreateWorkstreamRequest {
                workspace_id: ws.id.clone(),
                title: "   ".to_owned(),
                goal: None,
            },
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .expect_err("a workstream needs a title");
    assert_eq!(blank.code, CoreErrorCode::InvalidInput);
    assert_eq!(blank.details.unwrap()["field"], "title");

    let long = harness
        .manager
        .create(
            CreateWorkstreamRequest {
                workspace_id: ws.id.clone(),
                title: "x".repeat(wire::MAX_TITLE + 1),
                goal: None,
            },
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .expect_err("bounded");
    assert_eq!(long.code, CoreErrorCode::InvalidInput);

    let workstream = harness.start_working(&ws, "Real").await;
    let blank_question = harness
        .raise(&ws, &workstream, "  ", Vec::new())
        .await
        .expect_err("a decision needs a question");
    assert_eq!(blank_question.details.unwrap()["field"], "question");

    assert!(
        harness.list(&ws).await.len() == 1,
        "nothing invalid was stored"
    );
}

#[tokio::test]
async fn update_leaves_absent_fields_alone_and_a_blank_goal_clears_it() {
    let data = TempDir::new("update");
    let dir = workspace_dir("update");
    let harness = start(Some(&data.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir);
    let workstream = harness.create(&ws, "Watcher").await;

    let update = |title: Option<&str>, goal: Option<&str>| UpdateWorkstreamRequest {
        workspace_id: ws.id.clone(),
        workstream_id: workstream.id.clone(),
        title: title.map(str::to_owned),
        goal: goal.map(str::to_owned),
    };

    let with_goal = harness
        .manager
        .update(
            update(None, Some("Debounce renames")),
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .expect("set the goal");
    assert_eq!(with_goal.title, "Watcher", "an absent title is left alone");
    assert_eq!(with_goal.goal.as_deref(), Some("Debounce renames"));

    let renamed = harness
        .manager
        .update(
            update(Some("File watcher"), None),
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .expect("rename");
    assert_eq!(renamed.title, "File watcher");
    assert_eq!(
        renamed.goal.as_deref(),
        Some("Debounce renames"),
        "an absent goal is left alone"
    );

    let cleared = harness
        .manager
        .update(update(None, Some("  ")), Actor::User, CorrelationId::new())
        .await
        .expect("clear the goal");
    assert_eq!(cleared.goal, None, "a blank goal is how you clear one");
}

#[tokio::test]
async fn removing_a_workstream_takes_its_decisions_with_it() {
    let data = TempDir::new("remove");
    let dir = workspace_dir("remove");
    let harness = start(Some(&data.join(DB_FILE_NAME)));
    let ws = harness.open_ws(&dir);
    let workstream = harness.start_working(&ws, "Doomed").await;
    harness
        .raise(&ws, &workstream, "Anything?", Vec::new())
        .await
        .expect("raise");

    harness
        .manager
        .remove(
            RemoveWorkstreamRequest {
                workspace_id: ws.id.clone(),
                workstream_id: workstream.id.clone(),
            },
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .expect("remove");

    assert!(harness.list(&ws).await.is_empty());
    let left = harness
        .manager
        .list_decisions(
            ListDecisionsRequest {
                workspace_id: ws.id.clone(),
                workstream_id: None,
                open_only: None,
            },
            CorrelationId::new(),
        )
        .await
        .expect("list decisions");
    assert!(left.is_empty(), "orphan decisions would haunt the queue");

    let gone = harness
        .go(&ws, &workstream, WorkstreamState::Ready)
        .await
        .expect_err("it is gone");
    assert_eq!(gone.code, CoreErrorCode::NotFound);
}

#[tokio::test]
async fn another_workspaces_workstream_is_not_reachable() {
    let data = TempDir::new("scoping");
    let db = data.join(DB_FILE_NAME);
    let first_dir = workspace_dir("scoping-a");
    let second_dir = workspace_dir("scoping-b");
    let harness = start(Some(&db));

    let first = harness.open_ws(&first_dir);
    let mine = harness.create(&first, "Mine").await;

    // Opening another workspace replaces the first (one at a time, T020).
    let second = harness.open_ws(&second_dir);
    assert!(harness.list(&second).await.is_empty());

    let error = harness
        .go(&second, &mine, WorkstreamState::Ready)
        .await
        .expect_err("not visible from here");
    assert_eq!(
        error.code,
        CoreErrorCode::NotFound,
        "another workspace's work reads as missing, not as forbidden"
    );

    // A workspace that is not open at all is refused before the database.
    let stale = harness
        .manager
        .list(
            ListWorkstreamsRequest {
                workspace_id: first.id.clone(),
            },
            CorrelationId::new(),
        )
        .await
        .expect_err("the first workspace is closed");
    assert_eq!(stale.code, CoreErrorCode::NotFound);
}

#[tokio::test]
async fn without_a_database_nothing_is_kept_and_every_command_says_so() {
    let dir = workspace_dir("no-db");
    let harness = start(None);
    let ws = harness.open_ws(&dir);

    let error = harness
        .manager
        .create(
            CreateWorkstreamRequest {
                workspace_id: ws.id.clone(),
                title: "Nowhere to put this".to_owned(),
                goal: None,
            },
            Actor::User,
            CorrelationId::new(),
        )
        .await
        .expect_err("no database, no workstream");
    assert_eq!(error.code, CoreErrorCode::Io);
    assert!(
        error.message.contains("session database"),
        "{}",
        error.message
    );

    let listed = harness
        .manager
        .list(
            ListWorkstreamsRequest {
                workspace_id: ws.id.clone(),
            },
            CorrelationId::new(),
        )
        .await
        .expect_err("reads fail the same way");
    assert_eq!(listed.code, CoreErrorCode::Io);
    assert!(
        !harness.sink.names().contains(&"WorkstreamCreated"),
        "a workstream that was never stored was never created"
    );
}
