//! The workspace history behind the startup screen (T049).
//!
//! `crates/persistence` has recorded every root ever opened since T060 — the
//! session writer touches the `workspace` table on `WorkspaceOpened` — and
//! nothing has ever read it. This module is that reader.
//!
//! It is a plain pair of functions over a `&Database` rather than a method on
//! [`WorkspaceManager`](super::WorkspaceManager): the manager owns the *open*
//! workspace and deliberately holds no database handle (persistence reaches it
//! through the session tap). `SessionService`, which does hold the handle,
//! calls in here from the blocking pool.
//!
//! Both functions touch the filesystem and the database, so neither may be
//! called from a command handler directly.

use std::path::{Path, PathBuf};

use gritty_persistence::{Database, PersistenceError, now_ms};

use super::RecentWorkspace;
use crate::protocol::time::Timestamp;

/// Entries returned when the request does not say.
pub const DEFAULT_RECENTS: u32 = 10;
/// The most any one call will return. Each entry costs a `stat`, and a list
/// longer than this is not a list anyone reads.
pub const MAX_RECENTS: u32 = 20;

/// The most recently opened roots, newest first, each checked against the disk.
///
/// `home` shortens the displayed path; pass `None` when the account has no
/// resolvable home, in which case `display_root` is the root itself.
pub fn load(
    database: &Database,
    limit: u32,
    home: Option<&Path>,
) -> Result<Vec<RecentWorkspace>, PersistenceError> {
    let limit = i64::from(limit.clamp(0, MAX_RECENTS));
    let rows = database.read(|conn| gritty_persistence::store::workspace::list(conn, limit))?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let path = PathBuf::from(&row.root);
            RecentWorkspace {
                name: display_name(&path),
                display_root: shorten(&path, home),
                // A root replaced by a *file* is as unopenable as a missing
                // one, so `is_dir` — not `exists` — is the question.
                exists: std::fs::metadata(&path).is_ok_and(|meta| meta.is_dir()),
                last_opened_at: Timestamp::from_millis(
                    u64::try_from(row.last_opened_at).unwrap_or(0),
                ),
                root: row.root,
            }
        })
        .collect())
}

/// Drop one root from the history.
///
/// Also clears the last-workspace pointer when it names this root. Without
/// that, dismissing a workspace whose directory is gone would still leave the
/// next launch trying to reopen it and raising `WorkspaceRootMissing` again —
/// the user would have pressed Forget and nothing would have changed.
///
/// Tabs and recovery snapshots stored under the root are deliberately left in
/// place (ADR-014: unsaved work is never discarded as a side effect).
pub fn forget(database: &Database, root: &str) -> Result<(), PersistenceError> {
    database.write(|tx| {
        gritty_persistence::store::workspace::forget(tx, root)?;
        let session = gritty_persistence::store::session::load(tx)?;
        if session.and_then(|row| row.workspace_root).as_deref() == Some(root) {
            gritty_persistence::store::session::set_workspace_root(tx, None, now_ms())?;
        }
        Ok(())
    })
}

/// Last component of the root, or the whole root for a drive/filesystem root.
///
/// The same rule `Workspace::name` follows, on a path that is not open.
fn display_name(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.to_string_lossy().into_owned())
}

/// `/Users/someone/code/gritty` under `/Users/someone` becomes `~/code/gritty`.
///
/// Presentation only, and never a path anything is opened by: the untouched
/// `root` stays on the entry for that.
fn shorten(root: &Path, home: Option<&Path>) -> String {
    let Some(home) = home else {
        return root.to_string_lossy().into_owned();
    };
    match root.strip_prefix(home) {
        // The home directory itself.
        Ok(rest) if rest.components().next().is_none() => "~".to_owned(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.to_string_lossy()),
        // A sibling like `/Users/someone-else` must not become `~-else`;
        // `strip_prefix` is component-wise, so this is already safe, and the
        // match arm exists to say so.
        Err(_) => root.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use gritty_persistence::store::{session, workspace};

    use super::*;

    fn seed(database: &Database, roots: &[(&str, i64)]) {
        database
            .write(|tx| {
                for (root, at) in roots {
                    workspace::touch(tx, root, *at)?;
                }
                Ok(())
            })
            .expect("seed");
    }

    #[test]
    fn entries_come_back_newest_first_and_bounded_by_the_limit() {
        let database = Database::open_in_memory().expect("db");
        seed(&database, &[("/a", 1), ("/b", 2), ("/c", 3)]);

        let all = load(&database, 10, None).expect("load");
        assert_eq!(
            all.iter().map(|e| e.root.as_str()).collect::<Vec<_>>(),
            vec!["/c", "/b", "/a"]
        );
        assert_eq!(all[0].last_opened_at, Timestamp::from_millis(3));

        let two = load(&database, 2, None).expect("load");
        assert_eq!(two.len(), 2);
    }

    #[test]
    fn the_limit_is_capped_rather_than_refused() {
        let database = Database::open_in_memory().expect("db");
        seed(&database, &[("/a", 1)]);
        // A caller asking for a million gets MAX_RECENTS, not an error: the
        // bound is the core's to enforce, not the renderer's to respect.
        assert_eq!(load(&database, u32::MAX, None).expect("load").len(), 1);
    }

    #[test]
    fn existence_distinguishes_a_directory_from_a_file_and_from_nothing() {
        let dir = gritty_persistence::testing::TempDir::new("recents-exists");
        let present = dir.join("present");
        std::fs::create_dir(&present).expect("create dir");
        let a_file = dir.join("a-file");
        std::fs::write(&a_file, b"not a workspace").expect("write file");
        let missing = dir.join("missing");

        let database = Database::open_in_memory().expect("db");
        seed(
            &database,
            &[
                (&present.to_string_lossy(), 3),
                (&a_file.to_string_lossy(), 2),
                (&missing.to_string_lossy(), 1),
            ],
        );

        let entries = load(&database, 10, None).expect("load");
        assert_eq!(
            entries.iter().map(|e| e.exists).collect::<Vec<_>>(),
            vec![true, false, false],
            "a file where a workspace used to be is as unopenable as nothing"
        );
        assert_eq!(entries[0].name, "present");
    }

    #[test]
    fn the_display_path_shortens_under_home_and_nowhere_else() {
        let database = Database::open_in_memory().expect("db");
        seed(
            &database,
            &[
                ("/Users/someone/code/gritty", 4),
                ("/Users/someone", 3),
                ("/Users/someone-else/code", 2),
                ("/opt/elsewhere", 1),
            ],
        );

        let home = PathBuf::from("/Users/someone");
        let shown = load(&database, 10, Some(&home))
            .expect("load")
            .into_iter()
            .map(|e| e.display_root)
            .collect::<Vec<_>>();
        assert_eq!(
            shown,
            vec![
                "~/code/gritty",
                "~",
                // Not `~-else/code`: the prefix match is component-wise.
                "/Users/someone-else/code",
                "/opt/elsewhere",
            ]
        );
    }

    #[test]
    fn without_a_home_the_display_path_is_the_root() {
        let database = Database::open_in_memory().expect("db");
        seed(&database, &[("/Users/someone/code", 1)]);
        let entry = load(&database, 10, None).expect("load").remove(0);
        assert_eq!(entry.display_root, "/Users/someone/code");
        assert_eq!(entry.root, "/Users/someone/code");
    }

    #[test]
    fn forgetting_clears_the_last_workspace_pointer_only_when_it_matches() {
        let database = Database::open_in_memory().expect("db");
        seed(&database, &[("/kept", 2), ("/gone", 1)]);
        database
            .write(|tx| session::set_workspace_root(tx, Some("/gone"), 1))
            .expect("point at /gone");

        forget(&database, "/gone").expect("forget");
        assert_eq!(
            load(&database, 10, None)
                .expect("load")
                .iter()
                .map(|e| e.root.as_str())
                .collect::<Vec<_>>(),
            vec!["/kept"]
        );
        assert_eq!(
            database
                .read(session::load)
                .expect("load session")
                .and_then(|row| row.workspace_root),
            None,
            "the next launch must not retry a root the user just dismissed"
        );

        database
            .write(|tx| session::set_workspace_root(tx, Some("/kept"), 2))
            .expect("point at /kept");
        forget(&database, "/never-opened").expect("forget an unknown root");
        assert_eq!(
            database
                .read(session::load)
                .expect("load session")
                .and_then(|row| row.workspace_root)
                .as_deref(),
            Some("/kept"),
            "forgetting some other root leaves the pointer alone"
        );
    }
}
