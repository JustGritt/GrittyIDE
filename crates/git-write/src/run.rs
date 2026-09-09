//! The child process, and the guarantee that cancelling one kills everything
//! it started.
//!
//! A `pre-commit` hook is an arbitrary program the repository named, it can run
//! a whole test suite, and it is a *grandchild* — so killing the `git` process
//! is not killing the work. The child is therefore put in its own process
//! group and the group is what gets signalled, which is the shape
//! `ClaudeProcess` already proved for agent runs.
//!
//! **Platform limitation, stated rather than compiled around** (rule 11):
//! `killpg` is unix-only. On any other platform this falls back to killing the
//! direct child, and a hook that forked would survive it. That is a named
//! limitation, not a claim of cross-platform recovery, and it is untested
//! because there is no Windows machine here to test it on.

use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child as TokioChild, Command};

use crate::argv::GitWrite;
use crate::error::{GitWriteError, MAX_STDERR_BYTES, bound};

/// How long the group gets between SIGTERM and SIGKILL.
///
/// Short on purpose: cancellation is a user action and a user is waiting. The
/// force-kill test spawns a hook that traps SIGTERM precisely so the second
/// rung is exercised rather than assumed (rule 12).
pub const TERMINATE_GRACE: Duration = Duration::from_millis(500);

/// How long stderr may keep arriving after the child has exited. See
/// [`Running::wait`] for why this is a bound and not an EOF.
const STDERR_DRAIN: Duration = Duration::from_millis(250);

/// A finished `git`.
#[derive(Debug)]
pub struct Finished {
    pub status: ExitStatus,
    /// The child's stderr, trimmed and bounded. For the caller, never the log.
    pub stderr: String,
}

/// A running `git`, and the group it leads.
#[derive(Debug)]
pub struct Running {
    child: TokioChild,
    pid: Option<u32>,
    subcommand: &'static str,
    ended: bool,
}

/// Spawn `git`, in its own process group, with a scrubbed environment.
pub fn spawn(git: &Path, root: &Path, write: &GitWrite) -> Result<Running, GitWriteError> {
    let mut command = Command::new(git);
    command
        .args(write.argv())
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // ADR-029 decision 3: no pager, no editor, no prompt. A credential prompt
    // in particular must fail rather than block forever on a terminal that
    // does not exist — this crate never reaches the network, but a repository
    // can still name a submodule or a hook that tries.
    command
        .env("GIT_PAGER", "cat")
        .env("GIT_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0");

    // Gritty may itself have been launched from inside a repository, and these
    // would silently redirect the write to *that* one. `current_dir` decides
    // which repository is written to, and nothing inherited may override it.
    for redirect in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CEILING_DIRECTORIES",
    ] {
        command.env_remove(redirect);
    }

    // Its own process group, so cancellation reaches the hook rather than only
    // the `git` process that started it.
    #[cfg(unix)]
    command.process_group(0);

    let child = command
        .spawn()
        .map_err(|source| GitWriteError::io(format!("could not run {}", git.display()), source))?;
    let pid = child.id();
    Ok(Running {
        child,
        pid,
        subcommand: write.subcommand(),
        ended: false,
    })
}

impl Running {
    /// Wait for the child, collecting a bounded amount of stderr.
    ///
    /// The read runs *beside* the wait rather than before it, and is cut off
    /// [`STDERR_DRAIN`] after the child exits rather than at EOF. Reading to
    /// EOF looks obviously right and hangs: a `pre-commit` hook that starts a
    /// background process leaves that process holding the write end of the
    /// pipe, so EOF does not arrive until the grandchild dies — which may be
    /// never, and is certainly not something a commit should wait for. `git`
    /// flushes its own stderr before exiting, so the bound costs nothing real.
    pub async fn wait(&mut self) -> Result<Finished, GitWriteError> {
        let collected = Arc::new(Mutex::new(Vec::<u8>::new()));
        let reader = self.child.stderr.take().map(|mut stderr| {
            let collected = Arc::clone(&collected);
            tokio::spawn(async move {
                // Bounded by construction: a hook that prints a gigabyte
                // cannot make this allocate one.
                let mut chunk = [0_u8; 4096];
                while let Ok(read) = stderr.read(&mut chunk).await {
                    if read == 0 {
                        break;
                    }
                    let Ok(mut buffer) = collected.lock() else {
                        break;
                    };
                    if buffer.len() >= MAX_STDERR_BYTES {
                        continue;
                    }
                    let room = MAX_STDERR_BYTES - buffer.len();
                    buffer.extend_from_slice(&chunk[..read.min(room)]);
                }
            })
        });

        let status = self
            .child
            .wait()
            .await
            .map_err(|source| GitWriteError::io("could not wait for git", source))?;
        if let Some(mut reader) = reader
            && tokio::time::timeout(STDERR_DRAIN, &mut reader)
                .await
                .is_err()
        {
            // Something the hook started still holds the pipe. Take what
            // arrived and stop waiting on it.
            reader.abort();
        }

        self.ended = true;
        let text = collected
            .lock()
            .map(|buffer| String::from_utf8_lossy(&buffer).into_owned())
            .unwrap_or_default();
        Ok(Finished {
            status,
            stderr: bound(text),
        })
    }

    /// The subcommand this child is running, for a refusal's label.
    pub fn subcommand(&self) -> &'static str {
        self.subcommand
    }

    /// Stop the child, and everything it started.
    ///
    /// SIGTERM the whole group, give it [`TERMINATE_GRACE`], then SIGKILL.
    ///
    /// **The grace waits for the group, not for the child**, and that
    /// distinction is the whole point here. `ClaudeProcess` can return as soon
    /// as its child is reaped because for an agent run the child *is* the work.
    /// For a commit it is not: `git` starts the `pre-commit` hook and exits
    /// when signalled, while a hook that traps SIGTERM keeps running. Waiting
    /// on the child alone therefore reports success with the hook still alive —
    /// which is what the force-kill test caught, and is precisely the "recovery
    /// behaviour claimed but not tested" that rule 12 is about.
    pub async fn terminate(&mut self) {
        #[cfg(unix)]
        {
            self.signal_group(nix::sys::signal::Signal::SIGTERM);
            if self.wait_for_group(TERMINATE_GRACE).await {
                self.ended = true;
                return;
            }
            self.signal_group(nix::sys::signal::Signal::SIGKILL);
            // Reap the direct child so it is not left a zombie holding the
            // group open, then confirm the group actually went.
            let _ = tokio::time::timeout(TERMINATE_GRACE, self.child.wait()).await;
            self.wait_for_group(TERMINATE_GRACE).await;
            self.ended = true;
        }
        // The whole of the fallback on a platform without `killpg`, where only
        // the direct child is reached — a hook that forked would survive it.
        #[cfg(not(unix))]
        {
            let _ = self.child.start_kill();
            let _ = tokio::time::timeout(TERMINATE_GRACE, self.child.wait()).await;
            self.ended = true;
        }
    }

    /// Poll until nothing is left in the process group, or `grace` runs out.
    /// Returns whether the group is gone.
    ///
    /// The child is reaped as we go: an unreaped zombie is still a member of
    /// the group, so skipping that would make the group look alive forever.
    #[cfg(unix)]
    async fn wait_for_group(&mut self, grace: Duration) -> bool {
        const STEP: Duration = Duration::from_millis(20);
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            let _ = self.child.try_wait();
            if !self.group_alive() {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(STEP).await;
        }
    }

    /// Does any process remain in the group? Signal 0 tests for existence
    /// without delivering anything.
    #[cfg(unix)]
    fn group_alive(&self) -> bool {
        let Some(pid) = self.pid.and_then(|pid| i32::try_from(pid).ok()) else {
            return false;
        };
        if pid <= 0 {
            return false;
        }
        nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), None).is_ok()
    }

    #[cfg(unix)]
    fn signal_group(&self, signal: nix::sys::signal::Signal) {
        let Some(pid) = self.pid else {
            return;
        };
        // `process_group(0)` made the child its own group leader, so its pid is
        // the group id. Never 0: `killpg(0, …)` is *this* process's group.
        let Ok(pid) = i32::try_from(pid) else {
            return;
        };
        if pid <= 0 {
            return;
        }
        match nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), signal) {
            // `ESRCH` means the group is already gone, which is the outcome
            // being asked for.
            Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
            Err(error) => {
                tracing::warn!(pid, %error, "could not signal git's process group");
            }
        }
    }
}

impl Drop for Running {
    /// The last line of the orphan guarantee: a future that was abandoned
    /// still takes the hook with it. `kill_on_drop(true)` reaps the direct
    /// child; this is what reaches what the hook started.
    fn drop(&mut self) {
        #[cfg(unix)]
        if !self.ended {
            self.signal_group(nix::sys::signal::Signal::SIGKILL);
        }
    }
}
