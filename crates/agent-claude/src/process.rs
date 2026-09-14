//! The child, its three pipes, and its lifetime.
//!
//! Everything that needs a real process is here, and nothing here decides what
//! an observation *means* — that is [`crate::stream`], which this module feeds
//! and which a recorded fixture exercises without any of this. Keeping the two
//! apart is what lets CI prove the translation with neither `claude` nor an API
//! key present.
//!
//! # The orphan guarantee
//!
//! "An orphaned `claude` is the worst outcome available here: it would keep
//! running, keep costing money, and then appear in the observed roster as a
//! session nobody started." So the child is put in **its own process group**
//! (`process_group(0)`), and every way this type can end reaches that whole
//! group rather than the one `claude` process:
//!
//! - [`ClaudeProcess::terminate`] runs SIGTERM → grace → SIGKILL over the group;
//! - [`Drop`] SIGKILLs the group, so a driver that panicked still leaves
//!   nothing behind;
//! - `kill_on_drop(true)` is set as well, which is what reaps the direct child
//!   after the signal.
//!
//! `killpg` is unix-only, exactly as `gritty-terminal` declares it. On any
//! other platform this falls back to killing the direct child, and that is a
//! **named limitation** rather than a claim: a grandchild would survive, and
//! nothing here has been run on Windows (rule 11 — no cross-platform claim from
//! compilation alone).
//!
//! # Cancel safety
//!
//! [`ClaudeProcess::next`] is meant to sit in a `tokio::select!` opposite a
//! per-run inbox, so it has to be cancel-safe. It is: the only await inside it
//! is `AsyncReadExt::read`, which tokio documents as losing no data when
//! dropped, and every byte it produces lands in a buffer owned by `self`
//! rather than by the future. A `select!` that abandons a read mid-line
//! resumes on the same partial line.

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::error::ClaudeError;
use crate::launch::ClaudeLaunch;
use crate::observation::{ClaudeExit, ClaudeExitStatus, ClaudeObservation};
use crate::request::{ClaudeVerdict, permission_response, unsupported_response, user_turn};
use crate::stream::{ClaudeStream, ClaudeStreamStats};

/// How long a child gets to announce `system`/`init` before the run gives up.
///
/// Task §3: "A child that never emits `init` within a bounded wait fails the
/// run with an error that says so, rather than sitting in `STARTING`." Thirty
/// seconds is many times the observed start (about two seconds on a warm
/// machine) and still short enough that a user is told rather than left
/// watching a spinner.
pub const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the process group gets to honour SIGTERM before SIGKILL.
///
/// Short on purpose: cancellation is a user action and a user is waiting. The
/// force-kill test spawns a child that traps SIGTERM precisely so this second
/// rung is exercised rather than assumed (rule 12).
pub const TERMINATE_GRACE: Duration = Duration::from_millis(500);

/// How long a child gets to exit after its stdout closed, before Gritty
/// concludes it has lost the pipe to a process that is still alive.
pub const REAP_GRACE: Duration = Duration::from_millis(500);

/// Bytes taken from the child's stderr in one read.
const READ_CHUNK: usize = 16 * 1024;

/// How much of the child's stderr is kept for a failure message.
///
/// A tail, not the whole stream: the interesting part of a crash is the end,
/// and a child that writes megabytes to stderr must not be able to grow this.
/// Untrusted like everything else the child says — clipping it *for display*
/// is still the core's job at its own ingest seam.
pub const MAX_STDERR_TAIL_BYTES: usize = 4 * 1024;

/// A running `claude`, framed.
#[derive(Debug)]
pub struct ClaudeProcess {
    child: Child,
    /// `None` once stdin has been closed, which is how a graceful shutdown
    /// tells the CLI there are no more turns.
    stdin: Option<ChildStdin>,
    /// `None` once stdout reached end of file.
    stdout: Option<ChildStdout>,
    stream: ClaudeStream,
    pending: VecDeque<ClaudeObservation>,
    read_buf: Box<[u8]>,
    stderr: Arc<Mutex<Vec<u8>>>,
    /// Kept from the spawn, because `Child::id` returns `None` after a wait
    /// and the group still has to be reachable then.
    pid: Option<u32>,
    /// The final [`ClaudeObservation::Exited`] has been produced; the stream
    /// is over and [`Self::next`] will return `None` forever.
    ended: bool,
    program: String,
}

impl ClaudeProcess {
    /// Start a child under `launch`.
    ///
    /// The working directory is checked first so a missing one is reported as
    /// itself: `posix_spawn` answers `ENOENT` for a missing *binary* and a
    /// missing *cwd* alike, and "claude is not installed" and "that folder is
    /// gone" are fixed differently.
    ///
    /// The child inherits Gritty's environment, which is how it finds the
    /// user's credentials — this crate never reads, copies or logs them, and
    /// passes nothing on the command line.
    pub fn spawn(launch: &ClaudeLaunch) -> Result<Self, ClaudeError> {
        let cwd = launch.working_directory();
        match std::fs::metadata(cwd) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                return Err(ClaudeError::WorkingDirectory {
                    path: cwd.display().to_string(),
                    reason: "not a directory",
                });
            }
            Err(_) => {
                return Err(ClaudeError::WorkingDirectory {
                    path: cwd.display().to_string(),
                    reason: "cannot be read",
                });
            }
        }

        let program = launch.program().to_string_lossy().into_owned();
        let mut command = Command::new(launch.program());
        command
            .args(launch.argv())
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Its own process group, so cancellation reaches whatever the agent
        // itself started rather than only the `claude` process.
        #[cfg(unix)]
        command.process_group(0);

        let mut child = command.spawn().map_err(|source| ClaudeError::Spawn {
            program: program.clone(),
            source,
        })?;

        let pid = child.id();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut pipe) = child.stderr.take() {
            let tail = Arc::clone(&stderr);
            // A task rather than a branch of `next`: stderr must drain even
            // while nothing is reading stdout, or a chatty child fills its
            // pipe buffer and blocks on a write nobody is consuming.
            tokio::spawn(async move {
                let mut buf = vec![0_u8; READ_CHUNK];
                while let Ok(read) = pipe.read(&mut buf).await {
                    if read == 0 {
                        break;
                    }
                    let Ok(mut tail) = tail.lock() else {
                        break;
                    };
                    tail.extend_from_slice(&buf[..read]);
                    let over = tail.len().saturating_sub(MAX_STDERR_TAIL_BYTES);
                    if over > 0 {
                        tail.drain(..over);
                    }
                }
            });
        }

        tracing::debug!(program = %program, session = %launch.session_id(), "started a claude child");
        Ok(Self {
            child,
            stdin,
            stdout,
            stream: ClaudeStream::new(),
            pending: VecDeque::new(),
            read_buf: vec![0_u8; READ_CHUNK].into_boxed_slice(),
            stderr,
            pid,
            ended: false,
            program,
        })
    }

    /// Read until the session announces itself, or give up.
    ///
    /// Returns everything observed on the way, `Ready` included, so the caller
    /// records the same events it would have got from [`Self::next`]. A child
    /// that dies first returns its `Exited` instead and leaves
    /// [`Self::is_ready`] false; a child that says nothing at all is
    /// [`ClaudeError::NotReady`], and the caller kills it.
    pub async fn wait_ready(
        &mut self,
        wait: Duration,
    ) -> Result<Vec<ClaudeObservation>, ClaudeError> {
        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if self.stream.is_ready() || self.ended {
                return Ok(seen);
            }
            match tokio::time::timeout_at(deadline, self.next()).await {
                Ok(Some(observation)) => seen.push(observation),
                Ok(None) => return Ok(seen),
                Err(_) => {
                    tracing::warn!(
                        program = %self.program,
                        "the agent did not report a ready session in time"
                    );
                    return Err(ClaudeError::NotReady { wait });
                }
            }
        }
    }

    /// The next observation, or `None` once the stream is over.
    ///
    /// Cancel-safe: see the module docs. The final observation is always an
    /// [`ClaudeObservation::Exited`], so a caller that drains this to `None`
    /// has been told how the child ended.
    pub async fn next(&mut self) -> Option<ClaudeObservation> {
        loop {
            if let Some(observation) = self.pending.pop_front() {
                return Some(observation);
            }
            if self.ended {
                return None;
            }
            let Some(stdout) = self.stdout.as_mut() else {
                self.finish().await;
                continue;
            };
            // Disjoint field borrows: the bytes land in `self.read_buf` and
            // are handed straight to `self.stream`, with no await between, so
            // an abandoned read cannot lose a line.
            let read = match stdout.read(&mut self.read_buf).await {
                Ok(read) => read,
                Err(error) => {
                    // The pipe broke. Not fatal by itself; `finish` decides
                    // whether the child is gone or merely unreachable.
                    tracing::debug!(program = %self.program, %error, "the agent's stdout failed");
                    0
                }
            };
            if read == 0 {
                self.stdout = None;
                let flushed = self.stream.flush();
                self.pending.extend(flushed);
                self.finish().await;
                continue;
            }
            let observations = self.stream.ingest(&self.read_buf[..read]);
            self.pending.extend(observations);
        }
    }

    /// Ask the run something. The turn the CLI acknowledges with a replayed
    /// user message.
    pub async fn prompt(&mut self, text: &str) -> Result<(), ClaudeError> {
        self.write(&user_turn(text), "the run has ended").await?;
        self.stream.note_prompt_sent();
        Ok(())
    }

    /// Answer one `can_use_tool`.
    ///
    /// The adapter has no opinion about the verdict — that is the policy seam
    /// (see [`crate::request`]) — and no timeout that could produce one.
    pub async fn answer(
        &mut self,
        request_id: &str,
        verdict: &ClaudeVerdict,
    ) -> Result<(), ClaudeError> {
        tracing::debug!(
            program = %self.program,
            allowed = verdict.allowed(),
            "answering a permission request"
        );
        self.write(
            &permission_response(request_id, verdict),
            "the run ended before the request was answered",
        )
        .await
    }

    /// Refuse a `control_request` this adapter does not answer.
    ///
    /// Written rather than skipped, because the CLI may be holding on it the
    /// way it holds on an unanswered ask (see [`crate::request`]).
    pub async fn refuse(&mut self, request_id: &str, subtype: &str) -> Result<(), ClaudeError> {
        tracing::debug!(program = %self.program, "refusing an unsupported control request");
        self.write(
            &unsupported_response(request_id, subtype),
            "the run ended before the request was refused",
        )
        .await
    }

    /// Close stdin, telling the CLI there are no more turns.
    ///
    /// The graceful ending: the child finishes what it is doing and exits on
    /// its own, so the run ends `FINISHED` rather than `CANCELLED`.
    pub fn close_input(&mut self) {
        self.stdin = None;
    }

    /// Stop the child, and everything it started.
    ///
    /// SIGTERM the whole group, give it [`TERMINATE_GRACE`], then SIGKILL. The
    /// returned [`ClaudeExit`] is the adapter's report of *how* it ended; the
    /// run's ending is the manager's to decide, because only the manager knows
    /// a cancellation was asked for.
    pub async fn terminate(&mut self) -> ClaudeExit {
        #[cfg(unix)]
        {
            self.signal_group(nix::sys::signal::Signal::SIGTERM);
            if let Ok(Ok(status)) = tokio::time::timeout(TERMINATE_GRACE, self.child.wait()).await {
                return self.exit(Some(status));
            }
            self.signal_group(nix::sys::signal::Signal::SIGKILL);
        }
        // Also the whole of the fallback on a platform without `killpg`, where
        // only the direct child is reached.
        let _ = self.child.start_kill();
        match tokio::time::timeout(TERMINATE_GRACE, self.child.wait()).await {
            Ok(Ok(status)) => self.exit(Some(status)),
            _ => self.exit(None),
        }
    }

    /// Whether `system`/`init` has arrived.
    pub fn is_ready(&self) -> bool {
        self.stream.is_ready()
    }

    /// What the framing had to throw away.
    pub fn stats(&self) -> ClaudeStreamStats {
        self.stream.stats()
    }

    /// The child's process id, while it has one.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    async fn write(&mut self, frame: &str, closed: &'static str) -> Result<(), ClaudeError> {
        let Some(stdin) = self.stdin.as_mut() else {
            return Err(ClaudeError::Closed { reason: closed });
        };
        stdin
            .write_all(frame.as_bytes())
            .await
            .map_err(|source| ClaudeError::Write { source })?;
        stdin
            .flush()
            .await
            .map_err(|source| ClaudeError::Write { source })
    }

    /// Reap the child and queue the final `Exited`.
    async fn finish(&mut self) {
        if self.ended {
            return;
        }
        // Its stdout is gone, so nothing more is coming; a child that is still
        // alive after the grace has been lost rather than ended.
        let status = match tokio::time::timeout(REAP_GRACE, self.child.wait()).await {
            Ok(Ok(status)) => Some(status),
            Ok(Err(_)) | Err(_) => None,
        };
        let exit = self.exit(status);
        if status.is_none() {
            // Nothing is going to read from it again, and an unreachable child
            // is still a child: take the group down rather than leave one.
            #[cfg(unix)]
            self.signal_group(nix::sys::signal::Signal::SIGKILL);
            let _ = self.child.start_kill();
        }
        self.pending.push_back(ClaudeObservation::Exited(exit));
    }

    fn exit(&mut self, status: Option<std::process::ExitStatus>) -> ClaudeExit {
        self.ended = true;
        self.stdin = None;
        let stats = self.stream.stats();
        if stats.oversized > 0 || stats.malformed > 0 {
            // Once, by count, never by payload.
            tracing::warn!(
                program = %self.program,
                oversized = stats.oversized,
                malformed = stats.malformed,
                "dropped lines from the agent's stdout"
            );
        }
        let status = match status {
            None => ClaudeExitStatus::Lost,
            Some(status) => exit_status(status),
        };
        ClaudeExit {
            status,
            unfinished: self.stream.turn_in_flight() || !self.stream.is_ready(),
            stderr_tail: self.stderr_tail(),
        }
    }

    fn stderr_tail(&self) -> String {
        self.stderr
            .lock()
            .map(|tail| String::from_utf8_lossy(&tail).into_owned())
            .unwrap_or_default()
    }

    #[cfg(unix)]
    fn signal_group(&self, signal: nix::sys::signal::Signal) {
        let Some(pid) = self.pid else {
            return;
        };
        // `process_group(0)` made the child its own group leader, so its pid
        // is the group id. Never 0: `killpg(0, …)` is *this* process's group.
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
                tracing::warn!(pid, %error, "could not signal the agent's process group");
            }
        }
    }
}

impl Drop for ClaudeProcess {
    /// The last line of the orphan guarantee.
    ///
    /// A driver that panicked, or a future that was abandoned, still takes the
    /// whole group with it. `kill_on_drop(true)` reaps the direct child; this
    /// is what reaches everything the agent itself started.
    fn drop(&mut self) {
        #[cfg(unix)]
        if !self.ended {
            self.signal_group(nix::sys::signal::Signal::SIGKILL);
        }
    }
}

#[cfg(unix)]
fn exit_status(status: std::process::ExitStatus) -> ClaudeExitStatus {
    use std::os::unix::process::ExitStatusExt;
    if let Some(signal) = status.signal() {
        return ClaudeExitStatus::Signal(signal);
    }
    match status.code() {
        Some(0) => ClaudeExitStatus::Ok,
        Some(code) => ClaudeExitStatus::Code(code),
        // Neither a code nor a signal: nothing to report but the absence.
        None => ClaudeExitStatus::Lost,
    }
}

#[cfg(not(unix))]
fn exit_status(status: std::process::ExitStatus) -> ClaudeExitStatus {
    match status.code() {
        Some(0) => ClaudeExitStatus::Ok,
        Some(code) => ClaudeExitStatus::Code(code),
        None => ClaudeExitStatus::Lost,
    }
}
