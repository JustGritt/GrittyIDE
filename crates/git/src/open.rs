//! Opening a repository so that nothing it contains can run a program.
//!
//! This module is ADR-023 decision 2 in code. `gix` is pure Rust and runs no
//! subprocess *of its own*, but it will faithfully do what a repository's
//! configuration asks — including running a clean/smudge filter
//! (`filter.<name>.clean`) or a `diff.<name>.textconv`. A `.git/config` and a
//! `.gitattributes` arrive with a clone, which makes those two keys the
//! sharpest form of `AGENTS.md` rule 13 in the whole tree: *"in-process" is a
//! configuration, not a property of the library*.
//!
//! Two settings make it true, and
//! `tests::no_program_named_by_the_repository_can_run` proves it against a
//! repository that asks for both.
//!
//! **1. [`gix::open::Options::isolated`].** Only the repository's own config is
//! read: no `/etc/gitconfig`, no `~/.gitconfig`, no `GIT_*` environment
//! variables and no `include.path` chasing. This is the "ambient authority"
//! half of ADR-023 — the user's aliases, pager and credential helpers cannot
//! reach Gritty because they are never loaded.
//!
//! **2. [`deny_program_named_config`] as the config-section filter.** `gix`
//! gates every configuration section that can name an executable behind this
//! predicate — `filter.*` drivers ([`gix::filter::Pipeline::options`]),
//! `diff.*` drivers including `textconv`, credential helpers, url rewrites and
//! the signing key. Refusing every section empties those lists, so the filter
//! and diff pipelines are constructed with **no drivers at all** and there is
//! nothing left to execute. It is a coarse instrument — the metadata `gix`
//! offers describes the config *file*, not the section, so a subtler rule is
//! not available — and the cost is priced below.
//!
//! ## What the filter costs, and how it is paid back
//!
//! The same predicate also hides `remote.*` and `branch.*` from `gix`'s
//! high-level remote resolution, so [`gix::Repository::find_remote`] finds
//! nothing and the upstream tracking ref cannot be resolved the usual way.
//! That would take ahead/behind (T100 §5) with it, so [`upstream_ref_name`]
//! reads `branch.<name>.remote` and `branch.<name>.merge` straight out of the
//! resolved configuration instead. Reading a *value* is not executing a
//! program, and the standard `refs/remotes/<remote>/<branch>` mapping is
//! reproduced explicitly. A repository with a non-default fetch refspec
//! therefore reports no upstream rather than a wrong one, which is the failure
//! direction this repository prefers.
//!
//! Everything else the predicate hides is either unwanted here (credential
//! helpers, url rewrites, transport scheme permissions — this crate has no
//! network features compiled in at all) or falls back to `gix`'s defaults
//! (object cache sizes, compression levels).

use std::path::{Path, PathBuf};

use crate::error::GitError;

/// A repository opened at a workspace root, safe to move between threads.
///
/// Holds `gix`'s thread-safe handle; each read makes its own thread-local view
/// of it. Nothing else in the tree may hold a `gix` type (ADR-023 decision 3),
/// which is why this is opaque.
pub struct Repository {
    inner: gix::ThreadSafeRepository,
    root: PathBuf,
}

impl std::fmt::Debug for Repository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repository")
            .field("root", &self.root)
            .finish()
    }
}

impl Repository {
    /// The directory the repository was opened at.
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn to_thread_local(&self) -> gix::Repository {
        self.inner.to_thread_local()
    }
}

/// The config-section predicate that keeps every executable-naming section out
/// of the repository. See the module documentation.
///
/// `gix` types the hook as a plain `fn` pointer over
/// [`gix::config::file::Metadata`], which describes the config *file* a section
/// came from — its path, source and trust level — and not the section's own
/// name. There is therefore no "reject `filter.*`, keep `remote.*`" version of
/// this available; refusing everything is the only shape that is guaranteed to
/// have no gap, and a guarantee with no gap is what rule 13 asks for here.
fn deny_program_named_config(_meta: &gix::config::file::Metadata) -> bool {
    false
}

/// Open the repository **at** `root` — never discovered above it.
///
/// `open_path_as_is` stops `gix` from treating `root` as a `.git` directory or
/// looking for one; the caller passes a workspace root, and a workspace root
/// that is not itself a repository must answer "not a repository" rather than
/// silently reporting on a parent directory the user did not open (ADR-010:
/// nothing outside the root).
pub fn open(root: &Path) -> Result<Repository, GitError> {
    let options = gix::open::Options::isolated()
        .filter_config_section(deny_program_named_config)
        // Comments and whitespace in the config are of no interest, and
        // dropping them makes lookups cheaper.
        .lossy_config(true);
    match gix::ThreadSafeRepository::open_opts(root, options) {
        Ok(inner) => Ok(Repository {
            inner,
            root: root.to_path_buf(),
        }),
        Err(gix::open::Error::NotARepository { .. }) => Err(GitError::NotARepository {
            path: root.to_path_buf(),
        }),
        // A `.git` that exists but cannot be read is a machine failure, not a
        // "this is not a repository" answer: saying the latter would hide a
        // permission problem behind a plausible-looking empty panel.
        Err(gix::open::Error::Io(source)) => Err(GitError::io(
            format!("could not open the repository at {}", root.display()),
            source,
        )),
        Err(other) => Err(GitError::repository(
            format!("could not open the repository at {}", root.display()),
            other,
        )),
    }
}

/// The full name of the tracking ref of `branch`, read directly from the
/// resolved configuration.
///
/// Reimplements the default fetch refspec (`+refs/heads/*:refs/remotes/<remote>/*`)
/// rather than asking `gix`, because [`deny_program_named_config`] hides the
/// `remote.*` sections `gix`'s own resolution needs. `Ok(None)` means "no
/// upstream", which is a normal state and never an error.
pub(crate) fn upstream_ref_name(repo: &gix::Repository, branch: &str) -> Option<String> {
    let config = repo.config_snapshot();
    let raw = config.plumbing();
    let merge = raw.string_by("branch", Some(branch.into()), "merge")?;
    let merge = String::from_utf8_lossy(merge.as_ref()).into_owned();
    let remote = raw
        .string_by("branch", Some(branch.into()), "remote")
        .map(|value| String::from_utf8_lossy(value.as_ref()).into_owned())
        .unwrap_or_else(|| ".".to_owned());

    // `branch.<name>.remote = .` means the upstream is another local branch,
    // and `merge` already names it in full.
    if remote == "." {
        return Some(merge);
    }
    let short = merge.strip_prefix("refs/heads/").unwrap_or(&merge);
    Some(format!("refs/remotes/{remote}/{short}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::TestRepo;

    #[test]
    fn a_plain_directory_is_not_a_repository() {
        let dir = TestRepo::plain_directory();
        let error = open(dir.path()).expect_err("not a repository");
        assert!(matches!(error, GitError::NotARepository { .. }));
    }

    #[test]
    fn a_missing_directory_is_not_a_repository_rather_than_a_panic() {
        let dir = TestRepo::plain_directory();
        let missing = dir.path().join("nowhere");
        let error = open(&missing).expect_err("missing");
        assert!(matches!(
            error,
            GitError::NotARepository { .. } | GitError::Io { .. }
        ));
    }

    #[test]
    fn opens_a_repository_at_the_root_it_was_given() {
        let repo = crate::fixtures::clean();
        let opened = open(repo.path()).expect("open");
        assert_eq!(opened.root(), repo.path());
    }

    /// The repository is opened *at* the root, never discovered above it: a
    /// subdirectory of a repository is not a repository.
    #[test]
    fn does_not_discover_a_repository_above_the_root() {
        let repo = crate::fixtures::clean();
        let error = open(&repo.path().join("src")).expect_err("no discovery");
        assert!(matches!(error, GitError::NotARepository { .. }));
    }

    #[test]
    fn reads_the_upstream_ref_name_from_the_configuration() {
        let repo = crate::fixtures::clean();
        repo.git(&["config", "branch.main.remote", "origin"]);
        repo.git(&["config", "branch.main.merge", "refs/heads/main"]);
        let opened = open(repo.path()).expect("open");
        let local = opened.to_thread_local();
        assert_eq!(
            upstream_ref_name(&local, "main").as_deref(),
            Some("refs/remotes/origin/main")
        );
    }

    #[test]
    fn a_local_upstream_is_the_merge_ref_itself() {
        let repo = crate::fixtures::clean();
        repo.git(&["config", "branch.main.remote", "."]);
        repo.git(&["config", "branch.main.merge", "refs/heads/trunk"]);
        let opened = open(repo.path()).expect("open");
        let local = opened.to_thread_local();
        assert_eq!(
            upstream_ref_name(&local, "main").as_deref(),
            Some("refs/heads/trunk")
        );
    }

    #[test]
    fn a_branch_without_an_upstream_has_none() {
        let repo = crate::fixtures::clean();
        let opened = open(repo.path()).expect("open");
        let local = opened.to_thread_local();
        assert_eq!(upstream_ref_name(&local, "main"), None);
    }

    /// ADR-023 decision 2, proven rather than asserted from code shape
    /// (`AGENTS.md` rule 12).
    ///
    /// The fixture's `.git/config` defines `filter.evil.{clean,smudge}` and
    /// `diff.evil.textconv`, and its `.gitattributes` points two file patterns
    /// at them. Each of those commands is a script that touches a marker file
    /// before passing its input through, so "did a program run?" is a question
    /// the filesystem answers.
    ///
    /// The **control** is what makes this a test of [`open`] rather than a
    /// test of `gix`: the same repository opened with `gix`'s own defaults
    /// *does* produce a filter driver. If a future `gix` upgrade moved the
    /// driver behind a different gate, the control would fail first and say
    /// so, instead of the guarantee quietly becoming vacuous.
    #[cfg(unix)]
    #[test]
    fn no_program_named_by_the_repository_can_run() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;

        let (repo, marker) = crate::fixtures::hostile_filters();

        // Control: with `gix`'s defaults the hostile configuration is live.
        let permissive = gix::open(repo.path()).expect("open with defaults");
        let permissive_drivers =
            gix::filter::Pipeline::options(&permissive).expect("filter options");
        assert!(
            permissive_drivers
                .drivers
                .iter()
                .any(|driver| driver.name == "evil"),
            "the fixture must actually configure a filter driver, or this test proves nothing"
        );

        // The guarantee: opened the way Gritty opens repositories, there is no
        // driver to run — neither a clean/smudge filter nor a diff textconv.
        let opened = open(repo.path()).expect("open");
        let hardened = opened.to_thread_local();
        let drivers = gix::filter::Pipeline::options(&hardened).expect("filter options");
        assert!(
            drivers.drivers.is_empty(),
            "a repository must not be able to configure a filter program"
        );

        // And end to end: a full status and a full diff over the files the
        // `.gitattributes` targets leave the marker untouched.
        let interrupt = Arc::new(AtomicBool::new(false));
        let status =
            crate::status::status(&opened, &Default::default(), &interrupt).expect("status");
        assert!(
            status
                .entries
                .iter()
                .any(|entry| entry.path.to_string() == "notes.txt"),
            "the filtered file must actually have been examined"
        );
        let diff = crate::diff::diff(&opened, &Default::default(), &interrupt).expect("diff");
        assert!(
            diff.files
                .iter()
                .any(|file| file.path.to_string() == "notes.txt"),
            "the filtered file must actually have been diffed"
        );

        assert!(
            !marker.exists(),
            "a program named by the repository's own configuration was executed"
        );
    }
}
