//! File-backed behavior of the database handle: WAL, reopen, corruption
//! recovery, refusal of newer schemas, and the repositories end to end.

use std::fs;
use std::io::{Seek, SeekFrom, Write};

use gritty_persistence::store::{
    decisions, operations, ownership, session, snapshots, tabs, window, workspace, workstreams,
};
use gritty_persistence::testing::TempDir;
use gritty_persistence::{DB_FILE_NAME, Database, PersistenceError, SCHEMA_VERSION};

fn open(dir: &TempDir) -> gritty_persistence::Opened {
    Database::open(&dir.join(DB_FILE_NAME)).expect("open database")
}

#[test]
fn fresh_database_uses_wal_and_the_latest_schema_and_reopens_as_a_no_op() {
    let dir = TempDir::new("fresh");
    let opened = open(&dir);
    assert_eq!(opened.migrations.from, 0);
    assert_eq!(opened.migrations.to, SCHEMA_VERSION);
    assert!(opened.moved_aside.is_none());
    assert_eq!(opened.database.journal_mode().unwrap(), "wal");
    opened
        .database
        .write(|tx| session::set_workspace_root(tx, Some("/repo"), 7))
        .unwrap();
    drop(opened);

    let again = open(&dir);
    assert_eq!(
        again.migrations.applied(),
        0,
        "nothing to migrate on reopen"
    );
    let row = again.database.read(session::load).unwrap().unwrap();
    assert_eq!(row.workspace_root.as_deref(), Some("/repo"));
    assert!(dir.join(DB_FILE_NAME).exists());
}

#[test]
fn in_memory_database_is_migrated() {
    let database = Database::open_in_memory().unwrap();
    assert_eq!(database.journal_mode().unwrap(), "memory");
    database.quick_check().unwrap();
    assert!(database.read(session::load).unwrap().is_none());
}

#[test]
fn a_file_that_is_not_a_database_is_moved_aside_with_its_journal() {
    let dir = TempDir::new("garbage");
    let path = dir.join(DB_FILE_NAME);
    fs::write(&path, b"this is not a database at all, just bytes").unwrap();
    fs::write(dir.join(format!("{DB_FILE_NAME}-wal")), b"stale wal").unwrap();

    let opened = open(&dir);
    let aside = opened.moved_aside.clone().expect("garbage was moved aside");
    assert!(aside.exists(), "{}", aside.display());
    assert_eq!(
        fs::read(&aside).unwrap(),
        b"this is not a database at all, just bytes"
    );
    // SQLite removes a journal without valid frames when the failed
    // connection closes; whatever is left never lingers under the live name.
    assert!(
        !dir.join(format!("{DB_FILE_NAME}-wal")).exists()
            || fs::read(dir.join(format!("{DB_FILE_NAME}-wal"))).unwrap() != b"stale wal"
    );
    assert_eq!(opened.migrations.from, 0);
    assert_eq!(opened.database.journal_mode().unwrap(), "wal");
    let name = aside.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("gritty.sqlite.corrupt-"), "{name}");
}

#[test]
fn a_damaged_page_fails_quick_check_and_is_replaced_at_runtime() {
    let dir = TempDir::new("damaged");
    let path = dir.join(DB_FILE_NAME);
    {
        let opened = open(&dir);
        let text = "x".repeat(256 * 1024);
        opened
            .database
            .write(|tx| {
                snapshots::upsert(
                    tx,
                    "/repo",
                    &snapshots::SnapshotRow {
                        path: "big.txt".into(),
                        text,
                        encoding: "utf-8".into(),
                        saved_hash: "sha256:0".into(),
                        current_version: 3,
                        updated_at: 1,
                    },
                )
            })
            .unwrap();
        // Closing the last connection checkpoints the WAL into the main file.
    }
    let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    let page = 4096u64;
    file.seek(SeekFrom::Start(page * 12)).unwrap();
    file.write_all(&vec![0xFF; page as usize]).unwrap();
    drop(file);

    let opened = open(&dir);
    assert!(
        opened.moved_aside.is_none(),
        "the header and schema pages are intact, so open succeeds"
    );
    let error = opened.database.quick_check().unwrap_err();
    assert!(matches!(error, PersistenceError::Corrupt { .. }), "{error}");
    assert!(error.is_corrupt());

    let (aside, report) = opened.database.replace_corrupt().unwrap();
    assert!(aside.exists());
    assert_eq!(report.from, 0);
    opened.database.quick_check().unwrap();
    assert_eq!(opened.database.read(snapshots::count).unwrap(), 0);
    assert_eq!(opened.database.journal_mode().unwrap(), "wal");
}

#[test]
fn a_newer_schema_is_refused_and_left_untouched() {
    let dir = TempDir::new("newer");
    let path = dir.join(DB_FILE_NAME);
    {
        let opened = open(&dir);
        opened
            .database
            .write(|tx| {
                tx.execute(
                    "INSERT INTO schema_version (version, applied_at) VALUES (?1, 1)",
                    [SCHEMA_VERSION + 1],
                )?;
                Ok(())
            })
            .unwrap();
    }
    let error = Database::open(&path).unwrap_err();
    assert!(
        matches!(error, PersistenceError::NewerSchema { .. }),
        "{error}"
    );
    assert!(!error.is_corrupt());
    assert!(path.exists(), "a newer database is never moved aside");
    let entries: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("corrupt"))
        .collect();
    assert!(entries.is_empty(), "{entries:?}");
}

#[test]
fn two_handles_can_share_one_file() {
    let dir = TempDir::new("shared");
    let first = open(&dir);
    let second = open(&dir);
    first
        .database
        .write(|tx| workspace::touch(tx, "/a", 1))
        .unwrap();
    let seen = second
        .database
        .read(|conn| workspace::list(conn, 10))
        .unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].root, "/a");
}

#[test]
fn session_and_workspace_repositories_round_trip() {
    let database = Database::open_in_memory().unwrap();
    assert!(database.read(session::load).unwrap().is_none());
    database
        .write(|tx| {
            session::set_workspace_root(tx, Some("/repo"), 10)?;
            session::set_editor_preferences(tx, Some("{\"fontSize\":14}"), 11)?;
            session::set_layout(tx, Some("{\"version\":1}"), 12)?;
            workspace::touch(tx, "/repo", 10)?;
            workspace::touch(tx, "/other", 5)?;
            workspace::touch(tx, "/repo", 20)
        })
        .unwrap();
    let row = database.read(session::load).unwrap().unwrap();
    assert_eq!(
        row,
        session::SessionRow {
            workspace_root: Some("/repo".into()),
            editor_preferences: Some("{\"fontSize\":14}".into()),
            layout: Some("{\"version\":1}".into()),
            updated_at: 12,
        }
    );
    database
        .write(|tx| session::set_workspace_root(tx, None, 13))
        .unwrap();
    let row = database.read(session::load).unwrap().unwrap();
    assert_eq!(row.workspace_root, None);
    assert_eq!(
        row.layout.as_deref(),
        Some("{\"version\":1}"),
        "other columns untouched"
    );

    let recent = database.read(|conn| workspace::list(conn, 10)).unwrap();
    assert_eq!(
        recent
            .iter()
            .map(|w| (w.root.as_str(), w.last_opened_at))
            .collect::<Vec<_>>(),
        vec![("/repo", 20), ("/other", 5)]
    );
}

#[test]
fn the_workspace_history_is_ordered_and_bounded_by_the_limit() {
    let database = Database::open_in_memory().unwrap();
    database
        .write(|tx| {
            workspace::touch(tx, "/oldest", 1)?;
            workspace::touch(tx, "/middle", 2)?;
            workspace::touch(tx, "/newest", 3)?;
            // Same timestamp: the root breaks the tie, so the order is total.
            workspace::touch(tx, "/b-tie", 2)?;
            workspace::touch(tx, "/a-tie", 2)
        })
        .unwrap();

    let roots = |limit| {
        database
            .read(move |conn| workspace::list(conn, limit))
            .unwrap()
            .into_iter()
            .map(|row| row.root)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        roots(10),
        vec!["/newest", "/a-tie", "/b-tie", "/middle", "/oldest"]
    );
    assert_eq!(roots(2), vec!["/newest", "/a-tie"]);
    assert!(roots(0).is_empty());
}

#[test]
fn forgetting_a_root_leaves_its_tabs_and_snapshots_alone() {
    let database = Database::open_in_memory().unwrap();
    database
        .write(|tx| {
            workspace::touch(tx, "/gone", 1)?;
            workspace::touch(tx, "/kept", 2)?;
            tabs::replace(
                tx,
                "/gone",
                &[tabs::TabRow {
                    path: "notes.md".into(),
                    ..Default::default()
                }],
                1,
            )?;
            snapshots::upsert(
                tx,
                "/gone",
                &snapshots::SnapshotRow {
                    path: "notes.md".into(),
                    text: "unsaved".into(),
                    encoding: "UTF8".into(),
                    saved_hash: "hash".into(),
                    current_version: 3,
                    updated_at: 1,
                },
            )
        })
        .unwrap();

    assert!(database.write(|tx| workspace::forget(tx, "/gone")).unwrap());
    // Idempotent: forgetting what is already forgotten is not an error.
    assert!(!database.write(|tx| workspace::forget(tx, "/gone")).unwrap());

    let remaining = database.read(|conn| workspace::list(conn, 10)).unwrap();
    assert_eq!(
        remaining.iter().map(|row| &row.root).collect::<Vec<_>>(),
        vec!["/kept"]
    );
    // ADR-014: unsaved work under a root the user dismissed is never discarded
    // as a side effect.
    assert_eq!(
        database
            .read(|conn| tabs::list(conn, "/gone"))
            .unwrap()
            .len(),
        1
    );
    assert!(
        database
            .read(|conn| snapshots::get(conn, "/gone", "notes.md"))
            .unwrap()
            .is_some()
    );
}

#[test]
fn tab_repository_replaces_the_set_per_root_in_order() {
    let database = Database::open_in_memory().unwrap();
    let tab = |path: &str, active: bool| tabs::TabRow {
        path: path.into(),
        active,
        view_mode: Some("split".into()),
        cursor_line: Some(3),
        cursor_column: Some(5),
        anchor_line: Some(1),
        anchor_column: Some(1),
        scroll_line: Some(2),
        ..Default::default()
    };
    database
        .write(|tx| {
            tabs::replace(tx, "/repo", &[tab("b.md", false), tab("a.ts", true)], 1)?;
            tabs::replace(tx, "/other", &[tab("z.rs", true)], 1)
        })
        .unwrap();
    let listed = database.read(|c| tabs::list(c, "/repo")).unwrap();
    assert_eq!(
        listed.iter().map(|t| t.path.as_str()).collect::<Vec<_>>(),
        ["b.md", "a.ts"]
    );
    assert_eq!(
        listed.iter().map(|t| t.position).collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(listed[1].active && !listed[0].active);
    assert_eq!(listed[0].cursor_line, Some(3));
    assert_eq!(listed[0].scroll_line, Some(2));
    assert_eq!(listed[0].view_mode.as_deref(), Some("split"));

    database
        .write(|tx| tabs::replace(tx, "/repo", &[tab("a.ts", true)], 2))
        .unwrap();
    let listed = database.read(|c| tabs::list(c, "/repo")).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].updated_at, 2);
    assert_eq!(
        database.read(|c| tabs::list(c, "/other")).unwrap().len(),
        1,
        "other roots untouched"
    );
    database
        .write(|tx| tabs::replace(tx, "/repo", &[], 3))
        .unwrap();
    assert!(
        database
            .read(|c| tabs::list(c, "/repo"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn snapshot_repository_upserts_lists_and_deletes() {
    let database = Database::open_in_memory().unwrap();
    let row = snapshots::SnapshotRow {
        path: "src/main.rs".into(),
        text: "fn main() {}\n".into(),
        encoding: "utf-8".into(),
        saved_hash: "sha256:aa".into(),
        current_version: 4,
        updated_at: 100,
    };
    database
        .write(|tx| snapshots::upsert(tx, "/repo", &row))
        .unwrap();
    let updated = snapshots::SnapshotRow {
        text: "fn main() { println!(\"é\"); }\n".into(),
        current_version: 5,
        updated_at: 101,
        ..row.clone()
    };
    database
        .write(|tx| {
            snapshots::upsert(tx, "/repo", &updated)?;
            snapshots::upsert(
                tx,
                "/repo",
                &snapshots::SnapshotRow {
                    path: "a.txt".into(),
                    text: "a".into(),
                    encoding: "utf-8-bom".into(),
                    saved_hash: "sha256:bb".into(),
                    current_version: 2,
                    updated_at: 50,
                },
            )?;
            snapshots::upsert(tx, "/elsewhere", &row)
        })
        .unwrap();
    let got = database
        .read(|c| snapshots::get(c, "/repo", "src/main.rs"))
        .unwrap()
        .unwrap();
    assert_eq!(got, updated);
    let meta = database.read(|c| snapshots::list_meta(c, "/repo")).unwrap();
    assert_eq!(
        meta.iter().map(|m| m.path.as_str()).collect::<Vec<_>>(),
        ["a.txt", "src/main.rs"]
    );
    assert_eq!(
        meta[1].size,
        updated.text.len() as i64,
        "size is the UTF-8 byte length"
    );
    assert_eq!(meta[1].current_version, 5);
    assert_eq!(database.read(snapshots::count).unwrap(), 3);
    assert!(
        database
            .write(|tx| snapshots::delete(tx, "/repo", "a.txt"))
            .unwrap()
    );
    assert!(
        !database
            .write(|tx| snapshots::delete(tx, "/repo", "a.txt"))
            .unwrap()
    );
    assert!(
        database
            .read(|c| snapshots::get(c, "/repo", "a.txt"))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        database
            .read(|c| snapshots::list_meta(c, "/elsewhere"))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn window_repository_round_trips_geometry() {
    let database = Database::open_in_memory().unwrap();
    assert!(
        database
            .read(|c| window::load(c, "main"))
            .unwrap()
            .is_none()
    );
    let row = window::WindowRow {
        label: "main".into(),
        x: Some(-10),
        y: Some(40),
        width: 1280,
        height: 800,
        scale_factor: 2.0,
        maximized: false,
        updated_at: 9,
    };
    database.write(|tx| window::save(tx, &row)).unwrap();
    assert_eq!(
        database.read(|c| window::load(c, "main")).unwrap().unwrap(),
        row
    );
    let maximized = window::WindowRow {
        x: None,
        y: None,
        maximized: true,
        updated_at: 10,
        ..row
    };
    database.write(|tx| window::save(tx, &maximized)).unwrap();
    assert_eq!(
        database.read(|c| window::load(c, "main")).unwrap().unwrap(),
        maximized
    );
}

#[test]
fn operation_repository_marks_unfinished_records_interrupted() {
    let database = Database::open_in_memory().unwrap();
    let record = |id: &str, status: &str, created_at: i64| operations::OperationRow {
        id: id.into(),
        kind: "operations.demo".into(),
        workspace_root: Some("/repo".into()),
        status: status.into(),
        correlation_id: "corr".into(),
        created_at,
        started_at: Some(created_at + 1),
        finished_at: None,
        updated_at: created_at + 1,
    };
    database
        .write(|tx| {
            operations::upsert(tx, &record("running", "RUNNING", 1))?;
            operations::upsert(tx, &record("created", "CREATED", 2))?;
            operations::upsert(
                tx,
                &operations::OperationRow {
                    finished_at: Some(4),
                    ..record("done", "COMPLETED", 3)
                },
            )
        })
        .unwrap();
    let changed = database
        .write(|tx| operations::mark_interrupted(tx, 99))
        .unwrap();
    assert_eq!(changed, 2);
    let listed = database.read(operations::list).unwrap();
    assert_eq!(
        listed.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(),
        ["done", "created", "running"]
    );
    assert_eq!(listed[0].status, "COMPLETED");
    assert_eq!(listed[0].finished_at, Some(4));
    for interrupted in &listed[1..] {
        assert_eq!(interrupted.status, operations::INTERRUPTED);
        assert_eq!(interrupted.finished_at, Some(99));
        assert_eq!(interrupted.updated_at, 99);
    }
    let one = database
        .read(|c| operations::get(c, "running"))
        .unwrap()
        .unwrap();
    assert_eq!(one.status, "INTERRUPTED");
    assert_eq!(
        database
            .write(|tx| operations::mark_interrupted(tx, 100))
            .unwrap(),
        0
    );
}

#[test]
fn workstream_repository_round_trips_and_lists_newest_first() {
    let database = Database::open_in_memory().unwrap();
    let row = |id: &str, root: &str, created_at: i64| workstreams::WorkstreamRow {
        id: id.into(),
        workspace_root: root.into(),
        title: format!("title {id}"),
        goal: None,
        state: "DRAFT".into(),
        created_at,
        updated_at: created_at,
    };
    database
        .write(|tx| {
            workstreams::insert(tx, &row("a", "/repo", 1))?;
            workstreams::insert(tx, &row("b", "/repo", 2))?;
            workstreams::insert(tx, &row("elsewhere", "/other", 3))
        })
        .unwrap();

    let listed = database.read(|c| workstreams::list(c, "/repo")).unwrap();
    assert_eq!(
        listed.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
        ["b", "a"],
        "newest first, and never another workspace's"
    );

    assert!(
        database
            .write(|tx| workstreams::update(tx, "a", "renamed", Some("the goal"), 40))
            .unwrap()
    );
    assert!(
        database
            .write(|tx| workstreams::set_state(tx, "a", "READY", 41))
            .unwrap()
    );
    let updated = database
        .read(|c| workstreams::get(c, "a"))
        .unwrap()
        .expect("still there");
    assert_eq!(updated.title, "renamed");
    assert_eq!(updated.goal.as_deref(), Some("the goal"));
    assert_eq!(updated.state, "READY");
    assert_eq!(updated.updated_at, 41);

    // A row that is not there reports it rather than pretending to have written.
    assert!(
        !database
            .write(|tx| workstreams::set_state(tx, "missing", "READY", 42))
            .unwrap()
    );
    assert!(
        database
            .read(|c| workstreams::get(c, "missing"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn decisions_cascade_with_their_workstream_and_count_only_the_open_ones() {
    let database = Database::open_in_memory().unwrap();
    let decision =
        |id: &str, workstream: &str, state: &str, raised_at: i64| decisions::DecisionRow {
            id: id.into(),
            workstream_id: workstream.into(),
            question: format!("question {id}"),
            options: None,
            state: state.into(),
            chosen: None,
            rationale: None,
            raised_by: r#"{"type":"USER"}"#.into(),
            raised_at,
            resolved_at: None,
        };
    database
        .write(|tx| {
            workstreams::insert(
                tx,
                &workstreams::WorkstreamRow {
                    id: "w1".into(),
                    workspace_root: "/repo".into(),
                    title: "one".into(),
                    goal: None,
                    state: "IN_PROGRESS".into(),
                    created_at: 1,
                    updated_at: 1,
                },
            )?;
            workstreams::insert(
                tx,
                &workstreams::WorkstreamRow {
                    id: "w2".into(),
                    workspace_root: "/repo".into(),
                    title: "two".into(),
                    goal: None,
                    state: "IN_PROGRESS".into(),
                    created_at: 2,
                    updated_at: 2,
                },
            )?;
            decisions::insert(tx, &decision("d2", "w1", "OPEN", 20))?;
            decisions::insert(tx, &decision("d1", "w1", "OPEN", 10))?;
            decisions::insert(tx, &decision("d3", "w2", "RESOLVED", 30))
        })
        .unwrap();

    let all = database
        .read(|c| decisions::list(c, "/repo", None, false))
        .unwrap();
    assert_eq!(
        all.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(),
        ["d1", "d2", "d3"],
        "oldest first: the order the queue triages in"
    );
    let open = database
        .read(|c| decisions::list(c, "/repo", None, true))
        .unwrap();
    assert_eq!(open.len(), 2);
    let mine = database
        .read(|c| decisions::list(c, "/repo", Some("w2"), false))
        .unwrap();
    assert_eq!(mine.len(), 1);

    let counts = database
        .read(|c| decisions::open_counts(c, "/repo"))
        .unwrap();
    assert_eq!(
        counts,
        [("w1".to_owned(), 2)],
        "only workstreams with open ones"
    );
    assert_eq!(
        database.read(|c| decisions::count_open(c, "w2")).unwrap(),
        0
    );

    // Closing is conditional on still being open, so a double answer is visible.
    assert!(
        database
            .write(|tx| decisions::close(tx, "d1", "RESOLVED", Some("core"), None, 50))
            .unwrap()
    );
    assert!(
        !database
            .write(|tx| decisions::close(tx, "d1", "RESOLVED", Some("core"), None, 51))
            .unwrap(),
        "an answered decision cannot be answered again"
    );
    let closed = database
        .read(|c| decisions::get(c, "d1"))
        .unwrap()
        .expect("still there");
    assert_eq!(closed.state, "RESOLVED");
    assert_eq!(closed.chosen.as_deref(), Some("core"));
    assert_eq!(closed.resolved_at, Some(50));

    // Deleting the workstream takes its decisions with it.
    assert!(database.write(|tx| workstreams::delete(tx, "w1")).unwrap());
    assert!(
        database
            .read(|c| decisions::get(c, "d1"))
            .unwrap()
            .is_none()
    );
    assert!(
        database
            .read(|c| decisions::get(c, "d2"))
            .unwrap()
            .is_none()
    );
    assert!(
        database
            .read(|c| decisions::get(c, "d3"))
            .unwrap()
            .is_some(),
        "another workstream's decisions are untouched"
    );
}

#[test]
fn ownership_repository_round_trips_and_only_lists_what_still_holds() {
    let database = Database::open_in_memory().unwrap();
    let workstream = |id: &str| workstreams::WorkstreamRow {
        id: id.into(),
        workspace_root: "/repo".into(),
        title: format!("title {id}"),
        goal: None,
        state: "IN_PROGRESS".into(),
        created_at: 1,
        updated_at: 1,
    };
    let lease = |id: &str, holder: &str, pattern: &str, acquired_at: i64| ownership::LeaseRow {
        id: id.into(),
        workspace_root: "/repo".into(),
        workstream_id: holder.into(),
        agent_run_id: None,
        target_kind: "DIRECTORY".into(),
        target_pattern: pattern.into(),
        mode: "WRITE".into(),
        note: None,
        acquired_at,
        expires_at: None,
        state: "ACTIVE".into(),
        released_at: None,
    };
    database
        .write(|tx| {
            workstreams::insert(tx, &workstream("auth"))?;
            workstreams::insert(tx, &workstream("pay"))?;
            workstreams::insert(
                tx,
                &workstreams::WorkstreamRow {
                    workspace_root: "/other".into(),
                    ..workstream("elsewhere")
                },
            )?;
            ownership::insert_lease(tx, &lease("l-2", "pay", "src/payments/**", 2))?;
            ownership::insert_lease(tx, &lease("l-1", "auth", "src/auth/**", 1))?;
            ownership::insert_lease(
                tx,
                &ownership::LeaseRow {
                    workspace_root: "/other".into(),
                    ..lease("l-3", "elsewhere", "src/**", 3)
                },
            )
        })
        .unwrap();

    let held = database.read(|c| ownership::list_held(c, "/repo")).unwrap();
    assert_eq!(
        held.iter().map(|e| e.lease.id.as_str()).collect::<Vec<_>>(),
        ["l-1", "l-2"],
        "oldest first, and never another workspace's"
    );
    assert_eq!(held[0].holder_state, "IN_PROGRESS", "the join carries it");

    // Releasing is once only, and a released lease leaves the listing.
    assert!(
        database
            .write(|tx| ownership::release_lease(tx, "l-1", 9))
            .unwrap()
    );
    assert!(
        !database
            .write(|tx| ownership::release_lease(tx, "l-1", 10))
            .unwrap()
    );
    let held = database.read(|c| ownership::list_held(c, "/repo")).unwrap();
    assert_eq!(held.len(), 1);
    let released = database
        .read(|c| ownership::get_lease(c, "l-1"))
        .unwrap()
        .expect("still stored");
    assert_eq!(released.state, "RELEASED");
    assert_eq!(released.released_at, Some(9));
}

#[test]
fn conflicts_close_with_their_lease_and_cascade_with_their_workstream() {
    let database = Database::open_in_memory().unwrap();
    database
        .write(|tx| {
            for id in ["auth", "pay", "docs"] {
                workstreams::insert(
                    tx,
                    &workstreams::WorkstreamRow {
                        id: id.into(),
                        workspace_root: "/repo".into(),
                        title: id.into(),
                        goal: None,
                        state: "DRAFT".into(),
                        created_at: 1,
                        updated_at: 1,
                    },
                )?;
            }
            ownership::insert_lease(
                tx,
                &ownership::LeaseRow {
                    id: "l-1".into(),
                    workspace_root: "/repo".into(),
                    workstream_id: "auth".into(),
                    agent_run_id: None,
                    target_kind: "FILE".into(),
                    target_pattern: "src/api/router.ts".into(),
                    mode: "WRITE".into(),
                    note: None,
                    acquired_at: 1,
                    expires_at: None,
                    state: "ACTIVE".into(),
                    released_at: None,
                },
            )?;
            for (id, requester) in [("c-1", "pay"), ("c-2", "docs")] {
                ownership::insert_conflict(
                    tx,
                    &ownership::ConflictRow {
                        id: id.into(),
                        workspace_root: "/repo".into(),
                        requested_by: requester.into(),
                        owned_by: "auth".into(),
                        holding_lease_id: "l-1".into(),
                        target_kind: "FILE".into(),
                        target_pattern: "src/api/router.ts".into(),
                        created_at: 2,
                        state: "WAITING".into(),
                        resolved_at: None,
                    },
                )?;
            }
            Ok(())
        })
        .unwrap();

    // The same refusal is found rather than duplicated.
    assert!(
        database
            .read(|c| ownership::waiting_conflict(c, "pay", "l-1", "FILE", "src/api/router.ts"))
            .unwrap()
            .is_some()
    );

    // Closing one leaves the other, and closing the rest of a lease's queue
    // can skip the one already dealt with.
    assert!(
        database
            .write(|tx| ownership::close_conflict(tx, "c-1", "OVERRIDDEN", 5))
            .unwrap()
    );
    assert!(
        !database
            .write(|tx| ownership::close_conflict(tx, "c-1", "CANCELLED", 6))
            .unwrap()
    );
    assert_eq!(
        database
            .write(|tx| ownership::close_conflicts_for_lease(tx, "l-1", "RESOLVED", 7, Some("c-1")))
            .unwrap(),
        1
    );

    let listed = database
        .read(|c| ownership::list_conflicts(c, "/repo", 10))
        .unwrap();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|row| row.state != "WAITING"));

    // Everything hangs off the workstream: remove the holder and the lease
    // goes, and the conflicts waiting on it go with it.
    assert!(
        database
            .write(|tx| workstreams::delete(tx, "auth"))
            .unwrap()
    );
    assert!(
        database
            .read(|c| ownership::get_lease(c, "l-1"))
            .unwrap()
            .is_none()
    );
    assert!(
        database
            .read(|c| ownership::list_conflicts(c, "/repo", 10))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_database_written_before_ownership_existed_migrates_forward_with_its_rows() {
    let dir = TempDir::new("migrate-v3");
    let path = dir.join(DB_FILE_NAME);

    // Schema 2 exactly as it shipped: the migration runner applied up to the
    // workstream migration and stopped.
    {
        let conn = rusqlite::Connection::open(&path).expect("open");
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)",
        )
        .unwrap();
        for migration in gritty_persistence::MIGRATIONS.iter().take(2) {
            conn.execute_batch(migration.sql).expect("apply");
            conn.execute(
                "INSERT INTO schema_version (version, applied_at) VALUES (?1, 0)",
                [migration.version],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO workstream (id, workspace_root, title, goal, state, created_at, updated_at)
             VALUES ('kept', '/repo', 'Authentication', NULL, 'IN_PROGRESS', 1, 1)",
            [],
        )
        .unwrap();
    }

    let opened = Database::open(&path).expect("migrate the old database");
    assert_eq!(opened.migrations.from, 2);
    assert_eq!(opened.migrations.to, SCHEMA_VERSION);

    // The row that was there is still there, and the new tables work against it.
    let existing = opened
        .database
        .read(|c| workstreams::get(c, "kept"))
        .unwrap()
        .expect("the workstream survived");
    assert_eq!(existing.title, "Authentication");
    opened
        .database
        .write(|tx| {
            ownership::insert_lease(
                tx,
                &ownership::LeaseRow {
                    id: "l-1".into(),
                    workspace_root: "/repo".into(),
                    workstream_id: "kept".into(),
                    agent_run_id: None,
                    target_kind: "DIRECTORY".into(),
                    target_pattern: "src/auth/**".into(),
                    mode: "WRITE".into(),
                    note: None,
                    acquired_at: 2,
                    expires_at: None,
                    state: "ACTIVE".into(),
                    released_at: None,
                },
            )
        })
        .unwrap();
    assert_eq!(
        opened
            .database
            .read(|c| ownership::list_held(c, "/repo"))
            .unwrap()
            .len(),
        1
    );
}
