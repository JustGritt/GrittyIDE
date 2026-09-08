//! `git.log`: a bounded page of the history behind a revision.
//!
//! The walk is `gix`'s own topological-with-date ordering — the order
//! `git log` prints — and this module's job is to bound it, page it and
//! translate each commit into [`CommitSummary`].
//!
//! ## Why the cursor is a commit id and not an offset
//!
//! Nothing is cached (`lib.rs`: every call reads the repository again), so a
//! page boundary has to survive being recomputed. An offset would not: a
//! commit pushed between two pages shifts every later index by one and the
//! reader silently skips a row. The id of the last commit on the previous page
//! is stable — the walk either reaches it or it is gone from this history, and
//! the second case is a cursor that finds nothing rather than a page that lies.
//!
//! This is the same recompute-and-skip bargain [`crate::page`] documents for
//! status and diff, with the sort key replaced by walk order.
//!
//! ## Path filtering, and what it does not do
//!
//! `options.path` keeps the commits where that path's tree entry differs from
//! at least one parent — the question "when did this file change". It does
//! **not follow renames**: a file's history stops at the commit that renamed
//! it. Rename detection over a whole history is a different and much more
//! expensive computation than the per-diff detection `diff.rs` already does,
//! and reporting a partial answer as a complete one is what the failure table
//! forbids. A caller who needs the earlier history asks again with the old
//! path.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gritty_workspace::WorkspacePath;

use crate::error::GitError;
use crate::model::{CommitPage, CommitSignature, CommitSummary};
use crate::open::Repository;
use crate::page::{CANCEL_CHECK_INTERVAL, MAX_PAGE_SIZE, PageOptions};

/// What `git.log` should report.
#[derive(Debug, Clone, Default)]
pub struct LogOptions {
    pub page: PageOptions,
    /// Revision to walk back from; `HEAD` when absent.
    pub start: Option<String>,
    /// Only commits that changed this path. See the module documentation for
    /// what this deliberately does not do.
    pub path: Option<WorkspacePath>,
}

/// Walk the history behind `options.start`.
///
/// `interrupt` is polled every [`CANCEL_CHECK_INTERVAL`] commits, so the work
/// abandoned after a cancel is bounded by that constant and not by the size of
/// the history. Cancellation returns [`GitError::Cancelled`] and nothing
/// partial.
pub fn log(
    repo: &Repository,
    options: &LogOptions,
    interrupt: &Arc<AtomicBool>,
) -> Result<CommitPage, GitError> {
    let local = repo.to_thread_local();
    let cancelled = || interrupt.load(Ordering::Relaxed);
    let spec = options.start.clone().unwrap_or_else(|| "HEAD".to_owned());

    // An unborn HEAD is not a bad revision: the repository has no commits yet,
    // so the history is empty. The same call on a *named* revision that does
    // not exist is still the caller's mistake.
    if spec == "HEAD" && local.head().is_ok_and(|head| head.is_unborn()) {
        return Ok(CommitPage {
            start: spec,
            resolved: None,
            commits: Vec::new(),
            truncated: false,
            cursor: None,
        });
    }

    let tip = local
        .rev_parse_single(spec.as_str())
        .map_err(|_| GitError::UnknownRevision { spec: spec.clone() })?;
    let tip = tip
        .object()
        .map_err(|error| {
            GitError::repository(format!("could not read the object {spec} names"), error)
        })?
        .peel_to_commit()
        .map_err(|_| GitError::NotATree { spec: spec.clone() })?;
    let resolved = tip.id;

    let page_size = options.page.page_size.clamp(1, MAX_PAGE_SIZE);
    let after = options.page.cursor.as_deref();
    let mut skipping = after.is_some();

    let walk = local
        .rev_walk([resolved])
        .sorting(gix::revision::walk::Sorting::ByCommitTime(
            gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
        ))
        .all()
        .map_err(|error| GitError::repository("could not start the history walk", error))?;

    let mut commits: Vec<CommitSummary> = Vec::new();
    let mut truncated = false;
    for (index, info) in walk.enumerate() {
        if index % CANCEL_CHECK_INTERVAL == 0 && cancelled() {
            return Err(GitError::Cancelled);
        }
        let info = info.map_err(|error| GitError::repository("history walk failed", error))?;

        // Skip past the cursor, which is the last id of the previous page.
        if skipping {
            if after.is_some_and(|id| info.id == id) {
                skipping = false;
            }
            continue;
        }

        let commit = local.find_commit(info.id).map_err(|error| {
            GitError::repository("could not read a commit the walk reported", error)
        })?;
        if let Some(path) = options.path.as_ref()
            && !touches(&local, &commit, path)?
        {
            continue;
        }

        // One past the page is how the walk learns there is a next page,
        // without counting a history nobody asked it to count.
        if commits.len() == page_size {
            truncated = true;
            break;
        }
        commits.push(summarize(&local, &commit)?);
    }
    if cancelled() {
        return Err(GitError::Cancelled);
    }

    // Counts and durations only: a commit message, an author's name and a path
    // are all repository content, which `AGENTS.md` rule 13 keeps out of logs.
    tracing::debug!(
        commits = commits.len(),
        truncated,
        filtered = options.path.is_some(),
        "git log"
    );

    let cursor = truncated
        .then(|| commits.last().map(|commit| commit.id.clone()))
        .flatten();
    Ok(CommitPage {
        start: spec,
        resolved: Some(resolved.to_string()),
        commits,
        truncated,
        cursor,
    })
}

/// Whether `path` differs between this commit and at least one parent.
///
/// A root commit counts when the path exists in it at all — there is no parent
/// to differ from, and the file appearing is the change.
fn touches(
    repo: &gix::Repository,
    commit: &gix::Commit<'_>,
    path: &WorkspacePath,
) -> Result<bool, GitError> {
    let here = entry_id(commit, path)?;
    let mut parents = commit.parent_ids().peekable();
    if parents.peek().is_none() {
        return Ok(here.is_some());
    }
    for parent in parents {
        let parent = repo
            .find_commit(parent)
            .map_err(|error| GitError::repository("could not read a parent commit", error))?;
        if entry_id(&parent, path)? != here {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The object id `path` resolves to in a commit's tree, or `None` when the
/// path is not in it.
fn entry_id(
    commit: &gix::Commit<'_>,
    path: &WorkspacePath,
) -> Result<Option<gix::ObjectId>, GitError> {
    let tree = commit
        .tree()
        .map_err(|error| GitError::repository("could not read a commit's tree", error))?;
    let found = tree
        .lookup_entry_by_path(path.as_str())
        .map_err(|error| GitError::repository("could not walk a tree", error))?;
    Ok(found.map(|entry| entry.oid().to_owned()))
}

fn summarize(repo: &gix::Repository, commit: &gix::Commit<'_>) -> Result<CommitSummary, GitError> {
    let id = commit.id;
    let short_id = repo
        .find_object(id)
        .ok()
        .and_then(|object| object.id().shorten().ok())
        .map_or_else(
            // A repository that cannot answer still gets a usable abbreviation
            // rather than an error: this is a label, not an identifier.
            || id.to_string()[..7.min(id.to_string().len())].to_owned(),
            |prefix| prefix.to_string(),
        );

    let message = commit
        .message()
        .map_err(|error| GitError::repository("could not read a commit message", error))?;
    // Untrusted repository content: decoded lossily, passed through unchanged,
    // never interpreted, never logged (`AGENTS.md` rule 13).
    let subject = String::from_utf8_lossy(message.summary().as_ref()).into_owned();
    let body = message
        .body
        .map(|body| String::from_utf8_lossy(body).into_owned())
        .filter(|body| !body.trim().is_empty());

    let author = commit
        .author()
        .map_err(|error| GitError::repository("could not read a commit author", error))?;
    let committer = commit
        .committer()
        .map_err(|error| GitError::repository("could not read a commit committer", error))?;

    Ok(CommitSummary {
        id: id.to_string(),
        short_id,
        parents: commit.parent_ids().map(|id| id.to_string()).collect(),
        author: signature(author),
        committer: signature(committer),
        subject,
        body,
    })
}

fn signature(raw: gix::actor::SignatureRef<'_>) -> CommitSignature {
    let time = raw.time().unwrap_or_default();
    CommitSignature {
        name: String::from_utf8_lossy(raw.name).into_owned(),
        email: String::from_utf8_lossy(raw.email).into_owned(),
        time: time.seconds,
        offset_seconds: time.offset,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::fixtures::{self, TestRepo};
    use crate::open::open;

    fn walk(repo: &TestRepo, options: &LogOptions) -> CommitPage {
        let opened = open(repo.path()).expect("open");
        log(&opened, options, &Arc::new(AtomicBool::new(false))).expect("log")
    }

    fn options(page_size: usize) -> LogOptions {
        LogOptions {
            page: PageOptions {
                page_size,
                cursor: None,
            },
            ..Default::default()
        }
    }

    #[test]
    fn walks_the_history_newest_first() {
        let repo = fixtures::history();
        let page = walk(&repo, &options(10));

        let subjects: Vec<&str> = page
            .commits
            .iter()
            .map(|commit| commit.subject.as_str())
            .collect();
        assert_eq!(
            subjects,
            [
                "fourth: change main and add a binary",
                "third: README only",
                "second: add main",
                "first",
            ]
        );
        assert!(!page.truncated);
        assert_eq!(page.cursor, None);
        assert_eq!(page.resolved.as_deref(), Some(repo.head_id().as_str()));
    }

    #[test]
    fn a_commit_carries_its_author_its_parents_and_its_short_id() {
        let repo = fixtures::history();
        let page = walk(&repo, &options(10));
        let newest = &page.commits[0];
        let oldest = page.commits.last().expect("four commits");

        assert_eq!(newest.parents.len(), 1);
        assert_eq!(newest.parents[0], page.commits[1].id);
        // A root commit has no parent, which is how a caller finds the end of a
        // history without walking past it.
        assert!(oldest.parents.is_empty());

        assert!(newest.id.starts_with(&newest.short_id));
        assert!(newest.short_id.len() < newest.id.len());
        // The fixture pins the identity and the clock, so this is exact.
        assert_eq!(newest.author.name, "Gritty Fixture");
        assert_eq!(newest.committer.name, "Gritty Fixture");
        assert!(newest.author.time > 0);
    }

    #[test]
    fn a_page_boundary_is_a_commit_id_and_continues_where_it_left_off() {
        let repo = fixtures::history();
        let first = walk(&repo, &options(2));
        assert!(first.truncated);
        assert_eq!(first.commits.len(), 2);
        let cursor = first.cursor.clone().expect("a truncated page has a cursor");
        assert_eq!(cursor, first.commits[1].id);

        let second = walk(
            &repo,
            &LogOptions {
                page: PageOptions {
                    page_size: 2,
                    cursor: Some(cursor),
                },
                ..Default::default()
            },
        );
        assert_eq!(second.commits.len(), 2);
        assert_eq!(second.commits[0].subject, "second: add main");
        assert_eq!(second.commits[1].subject, "first");
        assert!(!second.truncated);

        // The two pages together are the whole history, with nothing repeated.
        let ids: Vec<&String> = first
            .commits
            .iter()
            .chain(second.commits.iter())
            .map(|commit| &commit.id)
            .collect();
        let unique: std::collections::BTreeSet<&&String> = ids.iter().collect();
        assert_eq!(ids.len(), 4);
        assert_eq!(unique.len(), 4);
    }

    #[test]
    fn a_path_filter_keeps_only_the_commits_that_changed_it() {
        let repo = fixtures::history();
        let page = walk(
            &repo,
            &LogOptions {
                path: Some(WorkspacePath::parse("src/main.rs").expect("a valid workspace path")),
                ..options(10)
            },
        );

        let subjects: Vec<&str> = page
            .commits
            .iter()
            .map(|commit| commit.subject.as_str())
            .collect();
        // "third" touched only README, and "first" predates the file.
        assert_eq!(
            subjects,
            ["fourth: change main and add a binary", "second: add main"]
        );
    }

    #[test]
    fn a_repository_with_no_commits_has_an_empty_history_rather_than_an_error() {
        let repo = TestRepo::init();
        let page = walk(&repo, &options(10));
        assert!(page.commits.is_empty());
        assert_eq!(page.resolved, None);
        assert!(!page.truncated);
    }

    #[test]
    fn an_unknown_revision_is_the_callers_fault() {
        let repo = fixtures::history();
        let opened = open(repo.path()).expect("open");
        let error = log(
            &opened,
            &LogOptions {
                start: Some("no-such-branch".to_owned()),
                ..options(10)
            },
            &Arc::new(AtomicBool::new(false)),
        )
        .expect_err("an unknown revision cannot be walked");
        assert!(matches!(error, GitError::UnknownRevision { .. }));
        assert!(error.is_caller_fault());
    }

    #[test]
    fn cancelling_abandons_the_walk_rather_than_finishing_it() {
        let repo = fixtures::history();
        let opened = open(repo.path()).expect("open");
        // Already set: the very first cancellation check must see it, so no
        // commit is ever summarized.
        let interrupt = Arc::new(AtomicBool::new(true));
        let error = log(&opened, &options(10), &interrupt).expect_err("cancelled");
        assert!(matches!(error, GitError::Cancelled));
    }

    #[test]
    fn a_cursor_that_names_no_commit_yields_nothing_rather_than_the_first_page() {
        let repo = fixtures::history();
        let page = walk(
            &repo,
            &LogOptions {
                page: PageOptions {
                    page_size: 10,
                    // A well-formed id that is not in this history.
                    cursor: Some("0".repeat(40)),
                },
                ..Default::default()
            },
        );
        assert!(page.commits.is_empty());
    }
}
