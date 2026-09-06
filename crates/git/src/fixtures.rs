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
