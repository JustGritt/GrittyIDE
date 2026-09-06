//! `git.diff`: what changed between a revision and the working tree, as
//! structured hunks.
//!
//! **`AGENTS.md` rule 5 is the whole design here.** Nothing in this module
//! produces, parses or stores a unified-diff *document*. `gix` computes the
//! edit script; [`gix::diff::blob::UnifiedDiff`] turns it into hunk headers
//! with 1-based line numbers and per-line `Context`/`Add`/`Remove` tags, and
//! [`Collector`] copies those into [`DiffHunk`] and [`DiffLine`]. At no point
//! does a `+`/`-` prefix get written into a string that something downstream
//! would have to parse back out.
//!
//! ## How the two sides are read
//!
//! Which paths changed comes from the same status machinery `git.status` uses,
//! with the target tree substituted for HEAD
//! ([`gix::status::Platform::head_tree`]). What each side *contains* is then
//! read from first principles rather than merged out of those events:
//!
//! - **old**: the blob the target tree has at that path, or nothing.
//! - **new**: the working-tree file at that path, or nothing.
//!
//! The change kind falls out of which of the two exist, so
//! "staged as new, then edited again in the worktree" needs no rule — the
//! target tree does not have it and the worktree does, therefore it is an
//! addition. Merging a HEAD→index event with an index→worktree event to reach
//! the same conclusion is where a diff view usually acquires its wrong badges.
//!
//! ## Symlinks
//!
//! A symlink is never followed. Git stores a symlink's *target* as the blob
//! content, so that is what is compared — which is both faithful to Git and
//! the reason no link inside the worktree can be used to read a file outside
//! it (`docs/architecture/security.md`).
//!
//! ## Why the blob pipeline is not used
//!
//! `gix::diff::blob::Platform` would apply `.gitattributes` diff drivers,
//! which is where `textconv` lives. Reading the two sides directly means the
//! program-running code path is not merely disabled by configuration
//! ([`crate::open`]) but absent. The cost is that a `binary` or `-diff`
//! attribute is not honoured; the NUL-byte heuristic below is what Git itself
//! falls back on without one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gritty_workspace::WorkspacePath;

use crate::error::GitError;
use crate::model::{
    DiffChangeKind, DiffHunk, DiffLimits, DiffLine, DiffLineKind, DiffSummary, FileDiff,
};
use crate::open::Repository;
use crate::page::{self, PageOptions};
use crate::path;

/// How much of the diff a caller wants.
#[derive(Debug, Clone, Default)]
pub struct DiffOptions {
    pub page: PageOptions,
    /// Revision to compare the working tree against; `HEAD` when absent.
    pub target: Option<String>,
    /// Restrict the diff to one file.
    pub path: Option<WorkspacePath>,
    pub limits: DiffLimits,
}

/// Bytes examined when deciding whether a blob is binary. Git's own heuristic
/// looks at the first 8000 bytes for a NUL.
const BINARY_SNIFF_BYTES: usize = 8_000;

/// Diff the working tree against `options.target`.
pub fn diff(
    repo: &Repository,
    options: &DiffOptions,
    interrupt: &Arc<AtomicBool>,
) -> Result<DiffSummary, GitError> {
    let local = repo.to_thread_local();
    let cancelled = || interrupt.load(Ordering::Relaxed);
    let spec = options.target.clone().unwrap_or_else(|| "HEAD".to_owned());

    let target = resolve_tree(&local, &spec)?;
    let resolved = target.commit.map(|id| id.to_string());

    // A bare repository has no working tree to compare against, so the answer
    // is an empty, labelled result rather than an error (T100's failure table).
    if local.is_bare() {
        return Ok(DiffSummary {
            target: spec,
            resolved,
            files: Vec::new(),
            total: 0,
            truncated: false,
            cursor: None,
            unrepresentable: 0,
        });
    }
    let Some(workdir) = local.workdir().map(ToOwned::to_owned) else {
        return Ok(DiffSummary {
            target: spec,
            resolved,
            files: Vec::new(),
            total: 0,
            truncated: false,
            cursor: None,
            unrepresentable: 0,
        });
    };

    let (paths, unrepresentable) = changed_paths(&local, &target, options, interrupt, &cancelled)?;

    let mut files = Vec::with_capacity(paths.len());
    page::drain(
        paths.into_iter().map(Ok::<_, std::convert::Infallible>),
        &cancelled,
        "could not read a file's diff",
        |(path, old_path)| {
            if let Some(file) =
                describe_file(&local, &target, &workdir, path, old_path, &options.limits)?
            {
                files.push(file);
            }
            Ok(())
        },
    )?;

    let page = page::page_collected(files, &options.page, |file: &FileDiff| {
        file.path.to_string()
    });
    tracing::debug!(
        files = page.entries.len(),
        total = page.total,
        truncated = page.truncated,
        unrepresentable,
        "git diff read"
    );
    Ok(DiffSummary {
        target: spec,
        resolved,
        files: page.entries,
        total: page.total,
        truncated: page.truncated,
        cursor: page.cursor,
        unrepresentable,
    })
}

struct Target {
    tree: gix::ObjectId,
    /// The commit the revision named, when it named one. Absent for an unborn
    /// HEAD, where the target is the empty tree.
    commit: Option<gix::ObjectId>,
}

fn resolve_tree(repo: &gix::Repository, spec: &str) -> Result<Target, GitError> {
    // An unborn HEAD is not a bad revision: the repository has no commits yet
    // and everything tracked reads as an addition against the empty tree.
    if spec == "HEAD" && repo.head().is_ok_and(|head| head.is_unborn()) {
        return Ok(Target {
            tree: repo.empty_tree().id,
            commit: None,
        });
    }
    let id = repo
        .rev_parse_single(spec)
        .map_err(|_| GitError::UnknownRevision {
            spec: spec.to_owned(),
        })?;
    let object = id.object().map_err(|error| {
        GitError::repository(format!("could not read the object {spec} names"), error)
    })?;
    let commit = (object.kind == gix::object::Kind::Commit).then_some(object.id);
    let tree = object.peel_to_tree().map_err(|_| GitError::NotATree {
        spec: spec.to_owned(),
    })?;
    Ok(Target {
        tree: tree.id,
        commit,
    })
}

/// A path that differs, and where it came from when it was renamed.
type ChangedPath = (WorkspacePath, Option<WorkspacePath>);

/// Every path that differs between the target tree and the working tree, with
/// the rename source when there is one.
fn changed_paths(
    repo: &gix::Repository,
    target: &Target,
    options: &DiffOptions,
    interrupt: &Arc<AtomicBool>,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Vec<ChangedPath>, u32), GitError> {
    let iter = repo
        .status(gix::progress::Discard)
        .map_err(|error| GitError::repository("could not prepare the diff", error))?
        .head_tree(target.tree)
        .should_interrupt_owned(Arc::clone(interrupt))
        // `git diff` does not list untracked files, and neither does this:
        // they are `git.status`'s answer. Turning the walk off entirely is
        // also what keeps an unignored `node_modules` from being read.
        .untracked_files(gix::status::UntrackedFiles::None)
        .index_worktree_submodules(gix::status::Submodule::Given {
            ignore: gix::submodule::config::Ignore::All,
            check_dirty: false,
        })
        .into_iter(pathspec(options))
        .map_err(|error| GitError::repository("could not start the diff", error))?;

    let mut seen: Vec<ChangedPath> = Vec::new();
    let mut unrepresentable = 0u32;
    page::drain(iter, cancelled, "could not read the diff", |item| {
        let (location, source) = match &item {
            gix::status::Item::TreeIndex(change) => match change {
                gix::diff::index::Change::Rewrite {
                    location,
                    source_location,
                    ..
                } => (location.as_ref(), Some(source_location.as_ref())),
                other => (other.location(), None),
            },
            gix::status::Item::IndexWorktree(inner) => (inner.rela_path(), None),
        };
        let Some(path) = path::decode(location) else {
            unrepresentable = unrepresentable.saturating_add(1);
            return Ok(());
        };
        if let Some(wanted) = &options.path
            && path != *wanted
        {
            return Ok(());
        }
        let source = source.and_then(|raw| path::decode(raw));
        match seen.iter_mut().find(|(seen, _)| *seen == path) {
            // The tree-index and index-worktree sides can both mention a path;
            // it is one file either way, and only one of them knows a rename
            // source.
            Some((_, existing_source)) => {
                if existing_source.is_none() {
                    *existing_source = source;
                }
            }
            None => seen.push((path, source)),
        }
        Ok(())
    })?;
    Ok((seen, unrepresentable))
}

fn pathspec(options: &DiffOptions) -> Vec<gix::bstr::BString> {
    match &options.path {
        // The pathspec narrows `gix`'s own work; the exact match above is what
        // makes the result exactly one file, since a pathspec matches prefixes.
        Some(path) => vec![gix::bstr::BString::from(path.to_string())],
        None => Vec::new(),
    }
}

/// Read both sides of one path and build its [`FileDiff`].
fn describe_file(
    repo: &gix::Repository,
    target: &Target,
    workdir: &std::path::Path,
    path: WorkspacePath,
    old_path: Option<WorkspacePath>,
    limits: &DiffLimits,
) -> Result<Option<FileDiff>, GitError> {
    // A rename reads its old side from where the file used to be.
    let old_lookup = old_path.as_ref().unwrap_or(&path);
    let old = read_tree_blob(repo, target, old_lookup)?;
    let new = read_worktree_blob(workdir, &path)?;

    let change = match (&old, &new) {
        (None, None) => return Ok(None),
        (None, Some(_)) => DiffChangeKind::Added,
        (Some(_), None) => DiffChangeKind::Deleted,
        (Some(old), Some(new)) => {
            if old_path.is_some() {
                DiffChangeKind::Renamed
            } else if old.kind != new.kind {
                DiffChangeKind::TypeChanged
            } else {
                DiffChangeKind::Modified
            }
        }
    };

    let size = new
        .as_ref()
        .or(old.as_ref())
        .map(|side| side.bytes.len() as u64);
    let old_bytes = old
        .as_ref()
        .map(|side| side.bytes.as_slice())
        .unwrap_or(&[]);
    let new_bytes = new
        .as_ref()
        .map(|side| side.bytes.as_slice())
        .unwrap_or(&[]);
    let binary = is_binary(old_bytes, limits) || is_binary(new_bytes, limits);

    let (additions, deletions, hunks, hunks_truncated) = if binary {
        (0, 0, Vec::new(), false)
    } else {
        hunks_of(old_bytes, new_bytes, limits)?
    };

    Ok(Some(FileDiff {
        path,
        old_path,
        change,
        additions,
        deletions,
        binary,
        // Only meaningful for a binary file, where it is all Gritty says about
        // the content.
        size: binary.then_some(size).flatten(),
        hunks,
        hunks_truncated,
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SideKind {
    Blob,
    Symlink,
    /// A submodule, or anything else with no blob content to compare.
    Opaque,
}

struct Side {
    bytes: Vec<u8>,
    kind: SideKind,
}

fn read_tree_blob(
    repo: &gix::Repository,
    target: &Target,
    path: &WorkspacePath,
) -> Result<Option<Side>, GitError> {
    let tree = repo
        .find_object(target.tree)
        .map_err(|error| GitError::repository("could not read the target tree", error))?
        .peel_to_tree()
        .map_err(|error| GitError::repository("could not read the target tree", error))?;
    let Some(entry) = tree
        .lookup_entry_by_path(path.to_string().as_str())
        .map_err(|error| GitError::repository("could not walk the target tree", error))?
    else {
        return Ok(None);
    };
    let kind = match entry.mode().kind() {
        gix::object::tree::EntryKind::Link => SideKind::Symlink,
        gix::object::tree::EntryKind::Blob | gix::object::tree::EntryKind::BlobExecutable => {
            SideKind::Blob
        }
        // A submodule or a tree: reported as changed, never entered.
        _ => {
            return Ok(Some(Side {
                bytes: Vec::new(),
                kind: SideKind::Opaque,
            }));
        }
    };
    let object = entry
        .object()
        .map_err(|error| GitError::repository("could not read a blob", error))?;
    Ok(Some(Side {
        bytes: object.data.clone(),
        kind,
    }))
}

fn read_worktree_blob(
    workdir: &std::path::Path,
    path: &WorkspacePath,
) -> Result<Option<Side>, GitError> {
    // `WorkspacePath` is relative and has no `.`/`..` segments, so this join
    // cannot climb out lexically; `symlink_metadata` then makes sure no link
    // is followed out either.
    let absolute = workdir.join(path.to_string());
    let metadata = match std::fs::symlink_metadata(&absolute) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(GitError::io(
                format!("could not stat {path} in the working tree"),
                error,
            ));
        }
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        // Git stores the link target as the blob, so that is the content.
        let target = std::fs::read_link(&absolute)
            .map_err(|error| GitError::io(format!("could not read the symlink {path}"), error))?;
        return Ok(Some(Side {
            bytes: target.to_string_lossy().into_owned().into_bytes(),
            kind: SideKind::Symlink,
        }));
    }
    if !file_type.is_file() {
        // A directory where a file used to be, or a device node: it exists but
        // has no comparable content.
        return Ok(Some(Side {
            bytes: Vec::new(),
            kind: SideKind::Opaque,
        }));
    }
    let bytes = std::fs::read(&absolute)
        .map_err(|error| GitError::io(format!("could not read {path}"), error))?;
    Ok(Some(Side {
        bytes,
        kind: SideKind::Blob,
    }))
}

/// Git's heuristic, and the size bound: a NUL in the first
/// [`BINARY_SNIFF_BYTES`], or a blob too large to be worth splitting into
/// lines nobody will scroll.
fn is_binary(bytes: &[u8], limits: &DiffLimits) -> bool {
    if bytes.len() as u64 > limits.max_blob_bytes {
        return true;
    }
    bytes.iter().take(BINARY_SNIFF_BYTES).any(|byte| *byte == 0)
}

type Hunks = (u32, u32, Vec<DiffHunk>, bool);

fn hunks_of(old: &[u8], new: &[u8], limits: &DiffLimits) -> Result<Hunks, GitError> {
    use gix::diff::blob::{Algorithm, Diff, InternedInput, UnifiedDiff, unified_diff::ContextSize};

    let input = InternedInput::new(
        gix::diff::blob::sources::byte_lines(old),
        gix::diff::blob::sources::byte_lines(new),
    );
    let mut computed = Diff::compute(Algorithm::Histogram, &input);
    // Git's slider heuristics: move a hunk boundary to where a human would put
    // it, so the hunk Gritty shows and the one `git diff` shows agree.
    computed.postprocess_lines(&input);
    let additions = computed.count_additions();
    let deletions = computed.count_removals();

    let collector = Collector {
        limits: *limits,
        hunks: Vec::new(),
        truncated: false,
    };
    let collector = UnifiedDiff::new(
        &computed,
        &input,
        collector,
        ContextSize::symmetrical(limits.context_lines),
    )
    .consume()
    .map_err(|error| GitError::io("could not build the diff hunks", error))?;

    Ok((additions, deletions, collector.hunks, collector.truncated))
}

/// Turns `gix`'s unified-diff callbacks into [`DiffHunk`]s, applying the
/// per-file and per-hunk bounds.
///
/// This is where "no field carries the whole diff as one string" is actually
/// enforced: the callback hands over `(kind, bytes)` pairs and they are stored
/// as tagged lines, never concatenated.
struct Collector {
    limits: DiffLimits,
    hunks: Vec<DiffHunk>,
    truncated: bool,
}

impl gix::diff::blob::unified_diff::ConsumeHunk for Collector {
    type Out = Self;

    fn consume_hunk(
        &mut self,
        header: gix::diff::blob::unified_diff::HunkHeader,
        lines: &[(gix::diff::blob::unified_diff::DiffLineKind, &[u8])],
    ) -> std::io::Result<()> {
        if self.hunks.len() >= self.limits.max_hunks {
            self.truncated = true;
            return Ok(());
        }
        let line_truncated = lines.len() > self.limits.max_hunk_lines;
        let kept = lines.iter().take(self.limits.max_hunk_lines);
        let lines = kept
            .map(|(kind, bytes)| DiffLine {
                kind: match kind {
                    gix::diff::blob::unified_diff::DiffLineKind::Context => DiffLineKind::Context,
                    gix::diff::blob::unified_diff::DiffLineKind::Add => DiffLineKind::Added,
                    gix::diff::blob::unified_diff::DiffLineKind::Remove => DiffLineKind::Removed,
                },
                // Untrusted repository content: decoded lossily, passed
                // through, never interpreted, never logged.
                content: String::from_utf8_lossy(bytes)
                    .trim_end_matches(['\n', '\r'])
                    .to_owned(),
            })
            .collect();
        self.hunks.push(DiffHunk {
            old_start: header.before_hunk_start,
            old_lines: header.before_hunk_len,
            new_start: header.after_hunk_start,
            new_lines: header.after_hunk_len,
            lines,
            truncated: line_truncated,
        });
        Ok(())
    }

    fn finish(self) -> Self::Out {
        self
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::fixtures::{self, TestRepo};
    use crate::model::DiffLineKind;
    use crate::open::open;

    fn read(repo: &TestRepo, options: &DiffOptions) -> DiffSummary {
        let opened = open(repo.path()).expect("open");
        diff(&opened, options, &Arc::new(AtomicBool::new(false))).expect("diff")
    }

    fn file<'a>(summary: &'a DiffSummary, path: &str) -> &'a FileDiff {
        summary
            .files
            .iter()
            .find(|file| file.path.to_string() == path)
            .unwrap_or_else(|| {
                panic!(
                    "no diff for {path}; had {:?}",
                    summary
                        .files
                        .iter()
                        .map(|f| f.path.to_string())
                        .collect::<Vec<_>>()
                )
            })
    }

    #[test]
    fn a_clean_repository_has_an_empty_diff_against_head() {
        let repo = fixtures::clean();
        let summary = read(&repo, &DiffOptions::default());
        assert_eq!(summary.target, "HEAD");
        assert_eq!(summary.resolved.as_deref(), Some(repo.head_id().as_str()));
        assert!(summary.files.is_empty());
        assert_eq!(summary.total, 0);
    }

    #[test]
    fn a_modified_file_carries_counts_and_tagged_hunk_lines() {
        let repo = fixtures::clean();
        repo.write("README.md", "hello\nworld\n");
        let summary = read(&repo, &DiffOptions::default());
        let file = file(&summary, "README.md");
        assert_eq!(file.change, DiffChangeKind::Modified);
        assert_eq!(file.additions, 1);
        assert_eq!(file.deletions, 0);
        assert!(!file.binary);
        assert_eq!(file.hunks.len(), 1);

        let hunk = &file.hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert!(!hunk.truncated);
        let added: Vec<&str> = hunk
            .lines
            .iter()
            .filter(|line| line.kind == DiffLineKind::Added)
            .map(|line| line.content.as_str())
            .collect();
        assert_eq!(added, vec!["world"]);
        let context: Vec<&str> = hunk
            .lines
            .iter()
            .filter(|line| line.kind == DiffLineKind::Context)
            .map(|line| line.content.as_str())
            .collect();
        assert_eq!(context, vec!["hello"]);
        // Rule 5, checked rather than asserted: no line is the whole diff.
        for line in &hunk.lines {
            assert!(!line.content.contains('\n'));
            assert!(!line.content.starts_with("@@"));
            assert!(!line.content.starts_with("diff --git"));
        }
    }

    #[test]
    fn a_staged_new_file_is_an_addition_against_head() {
        let repo = fixtures::clean();
        repo.write("added.txt", "one\ntwo\n");
        repo.git(&["add", "added.txt"]);
        let summary = read(&repo, &DiffOptions::default());
        let file = file(&summary, "added.txt");
        assert_eq!(file.change, DiffChangeKind::Added);
        assert_eq!(file.additions, 2);
        assert_eq!(file.deletions, 0);
    }

    #[test]
    fn a_deleted_file_is_a_deletion_with_its_removed_lines() {
        let repo = fixtures::clean();
        repo.remove("README.md");
        let summary = read(&repo, &DiffOptions::default());
        let file = file(&summary, "README.md");
        assert_eq!(file.change, DiffChangeKind::Deleted);
        assert_eq!(file.deletions, 1);
        assert_eq!(file.additions, 0);
    }

    #[test]
    fn a_rename_names_where_the_file_came_from() {
        let repo = fixtures::clean();
        repo.git(&["mv", "README.md", "READ.md"]);
        let summary = read(&repo, &DiffOptions::default());
        let file = file(&summary, "READ.md");
        assert_eq!(file.change, DiffChangeKind::Renamed);
        assert_eq!(
            file.old_path.as_ref().map(ToString::to_string),
            Some("README.md".to_owned())
        );
        // Renaming without editing changes no lines.
        assert_eq!((file.additions, file.deletions), (0, 0));
    }

    #[test]
    fn a_binary_file_carries_its_size_and_no_content() {
        let repo = fixtures::clean();
        repo.write("logo.bin", [0u8, 1, 2, 3, 0, 5]);
        repo.commit_all("binary");
        repo.write("logo.bin", [0u8, 9, 9, 9, 0, 9, 9]);

        let summary = read(&repo, &DiffOptions::default());
        let file = file(&summary, "logo.bin");
        assert!(file.binary);
        assert!(
            file.hunks.is_empty(),
            "binary content must not cross the wire"
        );
        assert_eq!(file.size, Some(7));
        assert_eq!((file.additions, file.deletions), (0, 0));
    }

    #[test]
    fn diffs_against_an_arbitrary_revision() {
        let repo = fixtures::clean();
        let first = repo.head_id();
        repo.write("README.md", "second\n");
        repo.commit_all("second");
        repo.write("README.md", "third\n");

        let against_head = read(&repo, &DiffOptions::default());
        assert_eq!(file(&against_head, "README.md").additions, 1);

        let against_first = read(
            &repo,
            &DiffOptions {
                target: Some(first.clone()),
                ..Default::default()
            },
        );
        assert_eq!(against_first.resolved.as_deref(), Some(first.as_str()));
        assert_eq!(
            file(&against_first, "README.md").change,
            DiffChangeKind::Modified
        );
    }

    #[test]
    fn an_unknown_revision_is_the_callers_fault() {
        let repo = fixtures::clean();
        let opened = open(repo.path()).expect("open");
        let error = diff(
            &opened,
            &DiffOptions {
                target: Some("no-such-revision".to_owned()),
                ..Default::default()
            },
            &Arc::new(AtomicBool::new(false)),
        )
        .expect_err("unknown revision");
        assert!(matches!(error, GitError::UnknownRevision { .. }));
        assert!(error.is_caller_fault());
    }

    #[test]
    fn a_path_filter_returns_exactly_that_file() {
        let repo = fixtures::clean();
        repo.write("README.md", "changed\n");
        repo.write("src/main.rs", "fn main() { }\n");
        let summary = read(
            &repo,
            &DiffOptions {
                path: Some(WorkspacePath::parse("src/main.rs").expect("path")),
                ..Default::default()
            },
        );
        assert_eq!(summary.files.len(), 1);
        assert_eq!(summary.files[0].path.to_string(), "src/main.rs");
    }

    #[test]
    fn hunks_are_bounded_and_say_so() {
        let repo = fixtures::clean();
        let original: String = (0..400).map(|index| format!("line {index}\n")).collect();
        repo.write("big.txt", &original);
        repo.commit_all("big");
        // Change every other line so the file has many separate hunks.
        let changed: String = (0..400)
            .map(|index| {
                if index % 2 == 0 {
                    format!("line {index}\n")
                } else {
                    format!("changed {index}\n")
                }
            })
            .collect();
        repo.write("big.txt", &changed);

        let summary = read(
            &repo,
            &DiffOptions {
                limits: DiffLimits {
                    max_hunks: 3,
                    max_hunk_lines: 1,
                    // Without context every changed line is its own hunk;
                    // with the default 3 they all merge into one.
                    context_lines: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let file = file(&summary, "big.txt");
        assert_eq!(file.hunks.len(), 3);
        assert!(file.hunks_truncated, "the file must say hunks were dropped");
        assert!(
            file.hunks.iter().all(|hunk| hunk.lines.len() <= 1),
            "each hunk must respect its line bound"
        );
        assert!(
            file.hunks.iter().any(|hunk| hunk.truncated),
            "a cut hunk must say so"
        );
        // The counts are still for the whole file, not for what was kept.
        assert_eq!(file.additions, 200);
        assert_eq!(file.deletions, 200);
    }

    #[test]
    fn a_bare_repository_diffs_to_an_empty_labelled_result() {
        let repo = TestRepo::init_bare();
        let opened = open(repo.path()).expect("open");
        let summary = diff(
            &opened,
            &DiffOptions {
                // A bare repository with no commits has no `HEAD` to resolve.
                target: Some("HEAD".to_owned()),
                ..Default::default()
            },
            &Arc::new(AtomicBool::new(false)),
        )
        .expect("diff");
        assert!(summary.files.is_empty());
        assert_eq!(summary.total, 0);
        assert_eq!(summary.target, "HEAD");
    }

    #[test]
    fn a_symlink_is_compared_by_its_target_and_never_followed() {
        let repo = fixtures::clean();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("README.md", repo.path().join("link")).expect("symlink");
            repo.commit_all("link");
            std::fs::remove_file(repo.path().join("link")).expect("remove");
            std::os::unix::fs::symlink("src/main.rs", repo.path().join("link")).expect("symlink");

            let summary = read(&repo, &DiffOptions::default());
            let file = file(&summary, "link");
            assert_eq!(file.change, DiffChangeKind::Modified);
            let lines: Vec<&str> = file.hunks[0]
                .lines
                .iter()
                .map(|line| line.content.as_str())
                .collect();
            // The link targets, not the contents of what they point at.
            assert!(lines.contains(&"README.md"));
            assert!(lines.contains(&"src/main.rs"));
            assert!(!lines.iter().any(|line| line.contains("fn main")));
        }
    }

    #[test]
    fn pages_files_and_continues_from_the_cursor() {
        let repo = fixtures::clean();
        for index in 0..5 {
            repo.write(&format!("d{index}.txt"), "one\n");
        }
        repo.commit_all("five");
        for index in 0..5 {
            repo.write(&format!("d{index}.txt"), "two\n");
        }

        let first = read(
            &repo,
            &DiffOptions {
                page: PageOptions {
                    page_size: 2,
                    cursor: None,
                },
                ..Default::default()
            },
        );
        assert_eq!(first.files.len(), 2);
        assert!(first.truncated);
        assert_eq!(first.total, 5);

        let second = read(
            &repo,
            &DiffOptions {
                page: PageOptions {
                    page_size: 10,
                    cursor: first.cursor.clone(),
                },
                ..Default::default()
            },
        );
        assert_eq!(second.files.len(), 3);
        assert!(!second.truncated);
    }

    #[test]
    fn a_cancelled_diff_returns_cancelled_and_nothing_partial() {
        let repo = fixtures::clean();
        repo.write("README.md", "changed\n");
        let opened = open(repo.path()).expect("open");
        let error = diff(
            &opened,
            &DiffOptions::default(),
            &Arc::new(AtomicBool::new(true)),
        )
        .expect_err("cancelled");
        assert!(matches!(error, GitError::Cancelled));
    }
}
