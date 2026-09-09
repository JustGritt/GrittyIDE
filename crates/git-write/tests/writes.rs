//! `gritty-git-write` against real repositories.
//!
//! Every claim ADR-029 makes about *why* a write must be the user's own `git`
//! is checked here rather than argued: that a clean filter runs, that hooks run
//! and that a refusing hook stops the commit, that cancelling kills the hook's
//! process group, and that a message survives byte for byte.
//!
//! These need a `git` binary. So does `gritty-git`'s own fixture module, which
//! builds the repositories, so a machine that can run the read tests can run
//! these.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gritty_git::fixtures::TestRepo;
use gritty_git_write::{CommitOptions, GitWrite, GitWriteError, perform, spawn};
use gritty_workspace::WorkspacePath;

fn git() -> PathBuf {
    PathBuf::from("git")
}

fn path(raw: &str) -> WorkspacePath {
    WorkspacePath::parse(raw).expect("a valid workspace path")
}

/// Write an executable hook. Returns nothing: the hook's effect is the point.
fn install_hook(repo: &TestRepo, name: &str, script: &str) {
    let hooks = repo.path().join(".git").join("hooks");
    std::fs::create_dir_all(&hooks).expect("create hooks dir");
    let file = hooks.join(name);
    std::fs::write(&file, script).expect("write hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&file).expect("stat hook").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&file, perms).expect("chmod hook");
    }
}

/// A message file, the way the core will write one.
fn message_file(repo: &TestRepo, message: &str) -> PathBuf {
    let file = repo.path().join(".git").join("GRITTY_MSG");
    std::fs::write(&file, message).expect("write message");
    file
}

fn status(repo: &TestRepo) -> String {
    repo.git(&["status", "--porcelain"])
}

#[tokio::test]
async fn staging_and_unstaging_move_one_named_path() {
    let repo = TestRepo::init();
    repo.write("a.txt", "one\n");
    repo.write("b.txt", "one\n");
    repo.commit_all("first");
    repo.write("a.txt", "changed\n");
    repo.write("b.txt", "changed\n");

    perform(
        &git(),
        repo.path(),
        &GitWrite::Stage {
            paths: vec![path("a.txt")],
        },
    )
    .await
    .expect("stage");

    // Exactly the named path moved. `b.txt` is the control: a wildcard write
    // would have swept it up, which is the failure ADR-029 decision 3 names.
    let after = status(&repo);
    assert!(
        after.contains("M  a.txt"),
        "a.txt should be staged: {after}"
    );
    assert!(
        after.contains(" M b.txt"),
        "b.txt must not be staged: {after}"
    );

    perform(
        &git(),
        repo.path(),
        &GitWrite::Unstage {
            paths: vec![path("a.txt")],
        },
    )
    .await
    .expect("unstage");
    assert!(status(&repo).contains(" M a.txt"));
}

#[tokio::test]
async fn unstaging_works_on_an_unborn_branch() {
    // `git restore --staged` fails here with "could not resolve HEAD", which is
    // why the argv uses `reset`. Staging your very first file and changing your
    // mind is an ordinary thing to do.
    let repo = TestRepo::init();
    repo.write("a.txt", "one\n");
    repo.git(&["add", "a.txt"]);

    perform(
        &git(),
        repo.path(),
        &GitWrite::Unstage {
            paths: vec![path("a.txt")],
        },
    )
    .await
    .expect("unstage on an unborn branch");
    assert!(status(&repo).contains("?? a.txt"));
}

#[tokio::test]
async fn staging_runs_the_repositorys_clean_filter() {
    // **This is the test ADR-029 exists for.** `gritty-git` opens repositories
    // with filters disabled so nothing a repository names can execute; staging
    // through `gix` under that configuration would write un-cleaned content
    // into the index and report no error. Running the user's `git` runs the
    // filter, and the marker is how we know it did.
    let (repo, marker) = gritty_git::fixtures::hostile_filters();
    assert!(!marker.exists(), "the filter must not have run yet");

    perform(
        &git(),
        repo.path(),
        &GitWrite::Stage {
            paths: vec![path("notes.txt")],
        },
    )
    .await
    .expect("stage a filtered path");

    assert!(
        marker.exists(),
        "the clean filter did not run; a staged blob would be un-cleaned"
    );
}

#[tokio::test]
async fn a_commit_runs_a_passing_pre_commit_hook() {
    let repo = TestRepo::init();
    repo.write("a.txt", "one\n");
    repo.commit_all("first");
    repo.write("a.txt", "two\n");
    repo.git(&["add", "a.txt"]);

    let ran = repo.path().join(".git").join("HOOK-RAN");
    install_hook(
        &repo,
        "pre-commit",
        &format!("#!/bin/sh\n:> '{}'\nexit 0\n", ran.display()),
    );

    let file = message_file(&repo, "second\n");
    perform(
        &git(),
        repo.path(),
        &GitWrite::Commit {
            message_file: file,
            options: CommitOptions::default(),
        },
    )
    .await
    .expect("commit");

    assert!(ran.exists(), "the pre-commit hook did not run");
    assert_eq!(repo.git(&["log", "--format=%s", "-1"]).trim(), "second");
}

#[tokio::test]
async fn a_refusing_hook_stops_the_commit_and_its_words_reach_the_caller() {
    let repo = TestRepo::init();
    repo.write("a.txt", "one\n");
    repo.commit_all("first");
    let before = repo.head_id();
    repo.write("a.txt", "two\n");
    repo.git(&["add", "a.txt"]);

    install_hook(
        &repo,
        "pre-commit",
        "#!/bin/sh\necho 'lint failed: a.txt' >&2\nexit 1\n",
    );

    let file = message_file(&repo, "second\n");
    let refused = perform(
        &git(),
        repo.path(),
        &GitWrite::Commit {
            message_file: file,
            options: CommitOptions::default(),
        },
    )
    .await
    .expect_err("the hook refused, so the commit must not succeed");

    match refused {
        GitWriteError::Refused {
            subcommand, stderr, ..
        } => {
            assert_eq!(subcommand, "commit");
            // The hook's own words, so the user can act on them. Never
            // swallowed, and never logged (rule 13).
            assert!(
                stderr.contains("lint failed: a.txt"),
                "the hook's output should reach the caller: {stderr:?}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }

    // Never reported as success, and nothing happened.
    assert_eq!(repo.head_id(), before, "HEAD must not have moved");
}

#[tokio::test]
async fn no_verify_skips_the_hook_that_would_have_refused() {
    let repo = TestRepo::init();
    repo.write("a.txt", "one\n");
    repo.commit_all("first");
    repo.write("a.txt", "two\n");
    repo.git(&["add", "a.txt"]);
    install_hook(&repo, "pre-commit", "#!/bin/sh\nexit 1\n");

    let file = message_file(&repo, "second\n");
    perform(
        &git(),
        repo.path(),
        &GitWrite::Commit {
            message_file: file,
            options: CommitOptions {
                no_verify: true,
                ..CommitOptions::default()
            },
        },
    )
    .await
    .expect("--no-verify should skip the refusing hook");
    assert_eq!(repo.git(&["log", "--format=%s", "-1"]).trim(), "second");
}

#[tokio::test]
async fn a_message_survives_byte_for_byte() {
    let repo = TestRepo::init();
    repo.write("a.txt", "one\n");
    repo.commit_all("first");
    repo.write("a.txt", "two\n");
    repo.git(&["add", "a.txt"]);

    // A quote, a subshell that must never be evaluated, a newline, and a line
    // starting with `#` — which git's default cleanup would delete, and
    // `--cleanup=verbatim` keeps.
    let message = "fix: don't run $(touch /tmp/pwned) here\n\n# not a comment\nsecond line\n";
    let file = message_file(&repo, message);
    perform(
        &git(),
        repo.path(),
        &GitWrite::Commit {
            message_file: file,
            options: CommitOptions::default(),
        },
    )
    .await
    .expect("commit");

    let committed = repo.git(&["log", "--format=%B", "-1"]);
    assert!(
        committed.contains("don't run $(touch /tmp/pwned) here"),
        "the message was altered: {committed:?}"
    );
    assert!(
        committed.contains("# not a comment"),
        "verbatim cleanup should keep a `#` line: {committed:?}"
    );
    assert!(
        !Path::new("/tmp/pwned").exists(),
        "the subshell was evaluated"
    );
}

#[tokio::test]
async fn a_discard_stashes_the_work_before_it_disappears() {
    let repo = TestRepo::init();
    repo.write("a.txt", "one\n");
    repo.commit_all("first");
    repo.write("a.txt", "precious\n");

    perform(
        &git(),
        repo.path(),
        &GitWrite::Stash {
            paths: vec![path("a.txt")],
        },
    )
    .await
    .expect("stash");

    assert_eq!(
        std::fs::read_to_string(repo.path().join("a.txt")).unwrap(),
        "one\n"
    );
    // Recoverable, which is recoverability's place in the priority order.
    assert!(repo.git(&["stash", "list"]).contains("stash@{0}"));
    repo.git(&["stash", "pop"]);
    assert_eq!(
        std::fs::read_to_string(repo.path().join("a.txt")).unwrap(),
        "precious\n"
    );
}

#[tokio::test]
async fn a_held_index_lock_is_named_rather_than_retried() {
    let repo = TestRepo::init();
    repo.write("a.txt", "one\n");
    repo.commit_all("first");
    repo.write("a.txt", "two\n");
    std::fs::write(repo.path().join(".git").join("index.lock"), "").expect("hold the lock");

    let held = perform(
        &git(),
        repo.path(),
        &GitWrite::Stage {
            paths: vec![path("a.txt")],
        },
    )
    .await
    .expect_err("a held lock must refuse");
    assert!(matches!(held, GitWriteError::Locked));
    assert!(held.is_retryable(), "a lock is the one retryable failure");
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_a_commit_kills_the_hooks_process_group() {
    // Rule 12: recovery behaviour is not claimed from code shape. The hook
    // ignores SIGTERM on purpose, so passing this needs the second rung of the
    // ladder — SIGKILL to the *group* — to actually fire.
    let repo = TestRepo::init();
    repo.write("a.txt", "one\n");
    repo.commit_all("first");
    repo.write("a.txt", "two\n");
    repo.git(&["add", "a.txt"]);

    let pidfile = repo.path().join(".git").join("HOOK-PID");
    install_hook(
        &repo,
        "pre-commit",
        &format!(
            "#!/bin/sh\ntrap '' TERM\nsleep 30 &\necho $! > '{}'\nwait\n",
            pidfile.display()
        ),
    );

    let file = message_file(&repo, "second\n");
    let mut running = spawn(
        &git(),
        repo.path(),
        &GitWrite::Commit {
            message_file: file,
            options: CommitOptions::default(),
        },
    )
    .expect("spawn");

    // Wait for the hook's grandchild to exist, so the kill has something to
    // miss if it only reaches the direct child.
    let mut grandchild = None;
    for _ in 0..200 {
        if let Ok(text) = std::fs::read_to_string(&pidfile)
            && let Ok(pid) = text.trim().parse::<i32>()
        {
            grandchild = Some(pid);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let grandchild = grandchild.expect("the hook never recorded its child's pid");
    assert!(alive(grandchild), "the hook's child should be running");

    running.terminate().await;

    // The group, not just `git`. Give the kernel a moment to reap.
    let mut gone = false;
    for _ in 0..100 {
        if !alive(grandchild) {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(gone, "the hook's process group survived cancellation");
    assert_eq!(
        repo.git(&["log", "--format=%s", "-1"]).trim(),
        "first",
        "a cancelled commit must not have happened"
    );
}

#[cfg(unix)]
fn alive(pid: i32) -> bool {
    // Signal 0 tests for existence without delivering anything.
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}
