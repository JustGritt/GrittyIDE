//! `gritty-git-write` — the four writes, run as the user's own `git`.
//!
//! # What this crate is
//!
//! A small, auditable adapter that invokes the `git` binary for `stage`,
//! `unstage`, `discard` and `commit`, maps its exit status onto a typed
//! refusal, probes once for the binary, and can be cancelled by killing the
//! process group. That is all it does.
//!
//! # Why it is not in `gritty-git`
//!
//! ADR-029 decision 2. `crates/git` promises "nothing writes" and "no external
//! program runs", and `open.rs`'s `no_program_named_by_the_repository_can_run`
//! enforces it. The two crates never depend on each other in anything that
//! ships, so that promise stays literally true and testable, and the trust
//! boundary is visible in the dependency graph.
//!
//! The reason a write cannot simply use `gix` is worth restating, because it is
//! not obvious: `gritty-git` opens every repository with clean/smudge filters
//! disabled, so that nothing a repository names can execute. For a *read* that
//! is conservative — raw bytes instead of a rendering. For a *write* it is
//! destructive: staging is exactly the operation that runs the clean filter, so
//! staging with filters off writes un-cleaned content into the index. In an LFS
//! repository that stores the whole binary where a pointer belongs. Nothing
//! reports an error, and the corruption is found by someone else, later, on a
//! clone. Signing and hooks are the other two: `gix` does neither, so a Gritty
//! commit would be unsigned and would never have met the project's own gate.
//!
//! # The invariants
//!
//! 1. **Nothing here reaches the network.** No `fetch`, `pull`, `push`,
//!    `clone`, `remote`. [`ALLOWED_SUBCOMMANDS`] is the closed list, and
//!    `argv.rs`'s tests assert it. Credentials and transports stay out of
//!    reach, exactly as ADR-023 arranged for reads. A networked write is
//!    ADR-030's decision, not a patch to this crate.
//! 2. **No repository content reaches an option position.** Pathspecs are last,
//!    after `--`, and prefixed `:(literal)` so pathspec magic is off too.
//! 3. **The commit message is delivered by file**, verbatim, never on a command
//!    line.
//! 4. **Hooks run, deliberately** (ADR-029 decision 4). That is repository-
//!    authored code executing, so it is disclosed in the UI, `--no-verify`
//!    exists as a visible choice, and it is never a default.
//! 5. **Nothing from the repository is logged.** A hook's stderr travels to the
//!    caller, who shows it to the user; it never reaches `tracing` (rule 13).

mod argv;
mod error;
mod probe;
mod run;

use std::path::Path;

pub use argv::{ALLOWED_SUBCOMMANDS, CommitOptions, GitWrite};
pub use error::GitWriteError;
pub use probe::{Availability, MINIMUM_VERSION, probe};
pub use run::{Finished, Running, TERMINATE_GRACE, spawn};

/// Run a write to completion, without a cancellation path.
///
/// The three fast writes use this. `commit` does not: a `pre-commit` hook can
/// run a test suite, so it is driven through [`spawn`] inside an `Operation`
/// that can [`Running::terminate`] it (ADR-029 decision 6, hard rule 10).
pub async fn perform(
    program: &Path,
    root: &Path,
    write: &GitWrite,
) -> Result<Finished, GitWriteError> {
    let mut running = spawn(program, root, write)?;
    let finished = running.wait().await?;
    check(root, write, finished)
}

/// Turn a finished child into a result, naming what can be named.
///
/// Structural classification only — see `error.rs` for why `git`'s prose is not
/// pattern-matched.
pub fn check(root: &Path, write: &GitWrite, finished: Finished) -> Result<Finished, GitWriteError> {
    if finished.status.success() {
        return Ok(finished);
    }
    if index_is_locked(root) {
        return Err(GitWriteError::Locked);
    }
    Err(GitWriteError::refused(
        write.subcommand(),
        finished.status,
        finished.stderr,
    ))
}

/// Is another process holding `index.lock`?
///
/// A filesystem check rather than a match on `git`'s message, so it does not
/// depend on `git`'s locale or wording.
///
/// **Limitation:** this looks in `<root>/.git`, which is a directory for an
/// ordinary checkout and a *file* for a linked worktree or a submodule. In
/// those layouts the real git directory is elsewhere and this returns `false`,
/// so the failure is reported as a plain refusal carrying `git`'s own message
/// — less specific, never wrong. Resolving the git directory properly would
/// mean either running `git rev-parse --git-dir` (another process on every
/// failure) or reading the `.git` file (this crate parsing repository content,
/// which is what it is trying not to do). Revisit with T400, which brings
/// worktrees and makes the second layout ordinary rather than rare.
fn index_is_locked(root: &Path) -> bool {
    let git_dir = root.join(".git");
    git_dir.is_dir() && git_dir.join("index.lock").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_directory_has_no_lock() {
        assert!(!index_is_locked(Path::new("/")));
    }
}
