//! Repositories built at test time, in known states.
//!
//! **This module is not part of the production build.** It is compiled only
//! under `cfg(test)` or the `fixtures` feature, and it is the one place in the
//! tree that runs the `git` binary. ADR-023 forbids a `git` subprocess for
//! *reading* a repository — that is the whole decision — and says nothing
//! about *building* one to read. Constructing a mid-rebase or mid-bisect
//! repository through `gix`'s read-only surface is not possible, and hand-
//! writing `.git` internals would be testing the fixture rather than the
//! adapter.
//!
//! `fixtures/small-repo` and `fixtures/large-repo` cannot stand in for these:
//! their `.git` is a decoy directory with no objects
//! (`gritty_workspace::fixtures`), which is precisely why they are useful for
//! the `NotARepository` case and useless for every other one.
//!
//! Every command runs with the developer's own Git configuration switched off
//! (`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`, a private `HOME`), so a fixture
//! means the same thing on every machine — the deterministic-fixtures rule in
//! `AGENTS.md`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// A repository in a temporary directory, deleted when this is dropped.
///
/// The temporary directory is hand-rolled the way `gritty-agent-hook`'s tests
/// do it, rather than pulling in `tempfile`: ADR-023 budgets this task exactly
/// one new external dependency, and it is spent on `gix`.
pub struct TestRepo {
    dir: PathBuf,
}

impl Drop for TestRepo {
    fn drop(&mut self) {
        // A `.git` directory contains read-only object files on some
        // configurations; make everything writable before removing it so a
        // failed cleanup cannot leak into the next run.
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl TestRepo {
    /// `git init` with a deterministic default branch and identity.
    pub fn init() -> Self {
        let repo = Self::plain_directory();
        repo.run_in(repo.path(), &["init", "--initial-branch=main", "--quiet"]);
        repo
    }

    /// `git init --bare`.
    pub fn init_bare() -> Self {
        let repo = Self::plain_directory();
        repo.run_in(
            repo.path(),
            &["init", "--bare", "--initial-branch=main", "--quiet"],
        );
        repo
    }

    /// A plain directory that is not a repository at all.
    pub fn plain_directory() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "gritty-git-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create fixture directory");
        // Canonicalize so the path compares equal to a `WorkspaceRoot`, which
        // resolves symlinks: `/var` is a link to `/private/var` on macOS.
        let dir = std::fs::canonicalize(&dir).expect("canonicalize fixture directory");
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Write a file, creating parent directories.
    pub fn write(&self, relative: &str, contents: impl AsRef<[u8]>) -> PathBuf {
        let target = self.path().join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(&target, contents).expect("write fixture file");
        target
    }

    pub fn remove(&self, relative: &str) {
        std::fs::remove_file(self.path().join(relative)).expect("remove fixture file");
    }

    /// Run `git` with the given arguments in the repository, asserting success.
    pub fn git(&self, args: &[&str]) -> String {
        self.run_in(self.path(), args)
    }

    /// Run `git`, returning `Err(stderr)` instead of panicking. Used for the
    /// commands that are *expected* to fail, like the merge that produces a
    /// conflict.
    pub fn git_allow_failure(&self, args: &[&str]) -> Result<String, String> {
        let output = self.command(self.path(), args).output().expect("run git");
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    }

    /// Stage everything and commit with a fixed identity and timestamp, so two
    /// runs of the same fixture produce the same object ids.
    pub fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "--all"]);
        self.commit(message)
    }

    /// Commit whatever is staged.
    pub fn commit(&self, message: &str) -> String {
        self.git(&["commit", "--quiet", "--allow-empty", "-m", message]);
        self.head_id()
    }

    /// Full hex id of HEAD.
    pub fn head_id(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    fn run_in(&self, cwd: &Path, args: &[&str]) -> String {
        let output = self.command(cwd, args).output().unwrap_or_else(|error| {
            panic!("could not run `git {}`: {error}", args.join(" "));
        });
        assert!(
            output.status.success(),
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    fn command(&self, cwd: &Path, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command
            .current_dir(cwd)
            .args(args)
            // A fixture must not depend on the developer's identity, aliases,
            // hooks, templates or `init.defaultBranch`.
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", &self.dir)
            .env("GIT_AUTHOR_NAME", "Gritty Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@gritty.invalid")
            .env("GIT_COMMITTER_NAME", "Gritty Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@gritty.invalid")
            .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00+00:00")
            .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00+00:00")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE");
        command
    }
}

/// A repository with one commit and a clean worktree.
pub fn clean() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("README.md", "hello\n");
    repo.write("src/main.rs", "fn main() {}\n");
    repo.commit_all("initial");
    repo
}

/// A repository whose `.git/config` and `.gitattributes` ask Git to run an
/// external program for both a clean/smudge filter and a `textconv`.
///
/// The program writes a marker file when it runs, so a test can assert that it
/// **did not** — ADR-023 decision 2, which is a test rather than an assumption.
pub fn hostile_filters() -> (TestRepo, PathBuf) {
    let repo = TestRepo::init();
    // Both live inside `.git`, so neither shows up as a worktree change and
    // the status under test is exactly the one the fixture intends.
    let marker = repo.path().join(".git").join("FILTER-RAN");
    let script = repo.path().join(".git").join("evil.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n:> '{}'\ncat\n",
            marker.display().to_string().replace('\'', "'\\''")
        ),
    )
    .expect("write filter script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&script)
            .expect("stat script")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).expect("chmod script");
    }
    let quoted = script.display().to_string().replace('\\', "/");
    repo.write(
        ".gitattributes",
        "*.secret filter=evil diff=evil\n*.txt filter=evil diff=evil\n",
    );
    repo.write("notes.txt", "plain content\n");
    repo.write("keys.secret", "another line\n");
    repo.commit_all("with filters");

    // Written after the commit so the committed tree is not itself filtered by
    // whatever the developer's `git` decides to do with it.
    repo.git(&["config", "filter.evil.clean", &format!("{quoted} clean")]);
    repo.git(&["config", "filter.evil.smudge", &format!("{quoted} smudge")]);
    repo.git(&["config", "filter.evil.required", "false"]);
    repo.git(&["config", "diff.evil.textconv", &quoted]);
    // Exactly as long as the committed content, so a status cannot conclude
    // "modified" from the size alone and has to look at the bytes — which is
    // the code path a clean filter would run in.
    repo.write("notes.txt", "other content\n");
    (repo, marker)
}

/// A repository with an unresolved merge conflict in `conflict.txt`.
pub fn merge_conflict() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("conflict.txt", "base\n");
    repo.commit_all("base");
    repo.git(&["checkout", "--quiet", "-b", "side"]);
    repo.write("conflict.txt", "side\n");
    repo.commit_all("side");
    repo.git(&["checkout", "--quiet", "main"]);
    repo.write("conflict.txt", "main\n");
    repo.commit_all("main");
    repo.git_allow_failure(&["merge", "side"])
        .expect_err("the merge is supposed to conflict");
    repo
}

/// Two branches that change `conflict.txt` differently from a common base,
/// checked out on `main`. `side` carries two commits: the first conflicts with
/// `main`, the second adds `other.txt` and conflicts with nothing, so a
/// sequence that stops on the first still has the second to apply.
fn diverged() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("conflict.txt", "base\n");
    repo.commit_all("base");
    repo.git(&["checkout", "--quiet", "-b", "side"]);
    repo.write("conflict.txt", "side\n");
    repo.commit_all("side: conflicts");
    repo.write("other.txt", "other\n");
    repo.commit_all("side: clean");
    repo.git(&["checkout", "--quiet", "main"]);
    repo.write("conflict.txt", "main\n");
    repo.commit_all("main");
    repo
}

/// Three commits on `main` that each rewrite `conflict.txt`, then one that
/// adds `other.txt`. Reverting the first rewrite conflicts with the later
/// ones; reverting the last commit conflicts with nothing.
fn rewritten() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("conflict.txt", "base\n");
    repo.commit_all("base");
    repo.write("conflict.txt", "one\n");
    repo.commit_all("one");
    repo.write("conflict.txt", "two\n");
    repo.commit_all("two");
    repo.write("other.txt", "other\n");
    repo.commit_all("three");
    repo
}

/// Assert that a command left the marker Git keeps for an operation in
/// progress, so a fixture that failed to reach its state fails here rather than
/// as a misleading status assertion.
fn assert_git_dir_has(repo: &TestRepo, marker: &str) {
    assert!(
        repo.path().join(".git").join(marker).exists(),
        "fixture did not reach its state: .git/{marker} is missing"
    );
}

/// `git rebase main` from `side`, stopped on a conflict in `conflict.txt`.
///
/// Git's default merge backend writes `.git/rebase-merge/interactive` even for
/// a rebase nobody asked to be interactive, so `gix` reports this as
/// `RebaseInteractive`.
pub fn rebase_conflict() -> TestRepo {
    let repo = diverged();
    repo.git(&["checkout", "--quiet", "side"]);
    repo.git_allow_failure(&["rebase", "main"])
        .expect_err("the rebase is supposed to stop on a conflict");
    assert_git_dir_has(&repo, "rebase-merge");
    repo
}

/// `git rebase --apply main` from `side`, stopped on a conflict: the older
/// apply backend, which keeps its state in `.git/rebase-apply` with a
/// `rebasing` marker, where `git am` would write `applying`.
pub fn rebase_apply_conflict() -> TestRepo {
    let repo = diverged();
    repo.git(&["checkout", "--quiet", "side"]);
    repo.git_allow_failure(&["rebase", "--apply", "main"])
        .expect_err("the rebase is supposed to stop on a conflict");
    assert_git_dir_has(&repo, "rebase-apply/rebasing");
    repo
}

/// An interactive rebase stopped on `edit` with no conflict: the state where
/// the worktree is clean and the operation is still in progress.
pub fn interactive_rebase_edit() -> TestRepo {
    let repo = diverged();
    // The sequence editor rewrites the todo list's `pick` into `edit`. It runs
    // through the shell, like the filter script `hostile_filters` installs.
    repo.git(&[
        "-c",
        "sequence.editor=sed -i.bak -e s/^pick/edit/",
        "rebase",
        "--interactive",
        "HEAD~1",
    ]);
    assert_git_dir_has(&repo, "rebase-merge/interactive");
    repo
}

/// `git cherry-pick` of one commit, stopped on a conflict. A single pick keeps
/// no sequencer state, so `gix` reports `CherryPick`.
pub fn cherry_pick_conflict() -> TestRepo {
    let repo = diverged();
    repo.git_allow_failure(&["cherry-pick", "side~1"])
        .expect_err("the cherry-pick is supposed to conflict");
    assert_git_dir_has(&repo, "CHERRY_PICK_HEAD");
    repo
}

/// `git cherry-pick` of two commits, stopped on a conflict in the first, so
/// `.git/sequencer/todo` still names the second: `CherryPickSequence`.
pub fn cherry_pick_sequence_conflict() -> TestRepo {
    let repo = diverged();
    repo.git_allow_failure(&["cherry-pick", "side~1", "side"])
        .expect_err("the cherry-pick is supposed to conflict");
    assert_git_dir_has(&repo, "CHERRY_PICK_HEAD");
    assert_git_dir_has(&repo, "sequencer/todo");
    repo
}

/// `git revert` of one commit, stopped on a conflict.
pub fn revert_conflict() -> TestRepo {
    let repo = rewritten();
    repo.git_allow_failure(&["revert", "--no-edit", "HEAD~2"])
        .expect_err("the revert is supposed to conflict");
    assert_git_dir_has(&repo, "REVERT_HEAD");
    repo
}

/// `git revert` of two commits, stopped on a conflict in the first:
/// `RevertSequence`.
pub fn revert_sequence_conflict() -> TestRepo {
    let repo = rewritten();
    repo.git_allow_failure(&["revert", "--no-edit", "HEAD~2", "HEAD"])
        .expect_err("the revert is supposed to conflict");
    assert_git_dir_has(&repo, "REVERT_HEAD");
    assert_git_dir_has(&repo, "sequencer/todo");
    repo
}

/// A stopped sequence whose conflicted commit the user concluded with a plain
/// `git commit` instead of `--continue`. The commit removes
/// `CHERRY_PICK_HEAD`/`REVERT_HEAD` and leaves `.git/sequencer/todo`, and
/// `git status` still says the operation is in progress.
pub fn sequence_committed_mid_way(sequence: TestRepo) -> TestRepo {
    sequence.write("conflict.txt", "resolved\n");
    sequence.git(&["add", "conflict.txt"]);
    sequence.commit("resolved by hand");
    assert!(
        !sequence.path().join(".git/CHERRY_PICK_HEAD").exists()
            && !sequence.path().join(".git/REVERT_HEAD").exists(),
        "the commit was supposed to conclude the stopped commit"
    );
    assert_git_dir_has(&sequence, "sequencer/todo");
    sequence
}

/// `git bisect start HEAD HEAD~2`, which checks out the midpoint detached.
pub fn bisecting() -> TestRepo {
    let repo = rewritten();
    repo.git(&["bisect", "start", "HEAD", "HEAD~2"]);
    assert_git_dir_has(&repo, "BISECT_LOG");
    repo
}

/// `git am --3way` of a patch that conflicts with `main`, stopped with the
/// conflict in the index.
pub fn am_conflict() -> TestRepo {
    let repo = diverged();
    let patch = repo.git(&["format-patch", "-1", "side~1", "--stdout"]);
    // Inside `.git`, so the patch file is not itself an untracked entry.
    let patch_path = repo.path().join(".git").join("side.patch");
    std::fs::write(&patch_path, patch).expect("write patch");
    repo.git_allow_failure(&["am", "--3way", patch_path.to_str().expect("utf-8 path")])
        .expect_err("the mailbox is supposed to conflict");
    assert_git_dir_has(&repo, "rebase-apply/applying");
    repo
}

/// A repository with `n` tracked files, all modified in the worktree.
///
/// Used to show that cancellation does not depend on repository size.
pub fn many_changes(files: usize) -> TestRepo {
    let repo = TestRepo::init();
    for index in 0..files {
        repo.write(&format!("f/{index:06}.txt"), "one\n");
    }
    repo.commit_all("many");
    for index in 0..files {
        repo.write(&format!("f/{index:06}.txt"), "two\n");
    }
    repo
}

/// A repository with a short, deterministic history: four commits, one of
/// which does not touch `src/main.rs`, plus a binary blob.
///
/// Built for `log` and `show`, which need *history* rather than the working-tree
/// changes [`clean`] and [`many_changes`] provide.
pub fn history() -> TestRepo {
    let repo = TestRepo::init();
    repo.write("README.md", "one\n");
    repo.commit_all("first");
    repo.write("src/main.rs", "fn main() {}\n");
    repo.commit_all("second: add main");
    // Touches only README, so a path-filtered log over src/main.rs must skip it.
    repo.write("README.md", "one\ntwo\n");
    repo.commit_all("third: README only");
    repo.write("src/main.rs", "fn main() { println!(); }\n");
    repo.write("logo.bin", [0u8, 1, 2, 3, 0, 255]);
    repo.commit_all("fourth: change main and add a binary");
    repo
}
