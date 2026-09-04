//! The boundary between the terminal domain and the PTY implementation.
//!
//! `gritty-core` owns the session registry, the ids and the events; it talks to
//! a [`PtyHost`] and never to `portable-pty`. Tests inject
//! [`crate::testing::FakePtyHost`], so core behaviour (registry bookkeeping,
//! exactly-once exit, idempotent kill) is exercised without spawning a shell.
//!
//! Output and exit travel *out* through a per-session [`TerminalSink`] rather
//! than a channel, matching the `EventSink` the core already uses. Sink methods
//! are called from the session's own threads, never from the caller's, and must
//! not block: the core forwards straight onto the event bus.

use std::path::PathBuf;
use std::sync::Arc;

use crate::batch::OutputBatch;
use crate::error::TerminalError;

/// Visible size of a terminal, in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtyGrid {
    pub cols: u16,
    pub rows: u16,
}

impl PtyGrid {
    /// Clamped to something a PTY accepts: zero would divide by zero in most
    /// programs, and absurd sizes are a malformed request, not a resize.
    pub fn sanitized(self) -> Self {
        Self {
            cols: self.cols.clamp(1, 1000),
            rows: self.rows.clamp(1, 1000),
        }
    }
}

impl Default for PtyGrid {
    fn default() -> Self {
        Self { cols: 80, rows: 24 }
    }
}

/// What the core asks for when opening a session.
#[derive(Debug, Clone)]
pub struct SpawnRequest {
    /// Working directory; the core defaults it to the workspace root.
    pub cwd: PathBuf,
    /// Overrides the resolved login shell. `None` uses the user's own.
    pub shell: Option<PathBuf>,
    /// Applied after the inherited environment and after `TERM`/`COLORTERM`.
    pub env: Vec<(String, String)>,
    pub size: PtyGrid,
}

/// How a session ended.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TerminalExit {
    /// Exit status, when the process exited normally.
    pub code: Option<i32>,
    /// Signal name, when it was terminated by one (including Gritty's own kill).
    pub signal: Option<String>,
}

/// Receives everything a session produces. Implemented by the core.
pub trait TerminalSink: Send + Sync {
    /// A coalesced batch. Never called after [`Self::exited`].
    fn output(&self, batch: OutputBatch);
    /// Called exactly once, after the child has been reaped.
    fn exited(&self, exit: TerminalExit);
}

/// Opens PTY sessions. One implementation talks to the operating system, one
/// is a deterministic fake.
pub trait PtyHost: Send + Sync {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Arc<dyn TerminalSink>,
    ) -> Result<Box<dyn PtySession>, TerminalError>;
}

/// A live PTY session. Dropping it must terminate the child: the core keeps
/// sessions in a registry, and losing the handle without killing would leak a
/// process for the lifetime of the application.
pub trait PtySession: Send + Sync {
    /// Forward keystrokes or pasted text to the child.
    fn write(&self, data: &[u8]) -> Result<(), TerminalError>;
    /// Tell the kernel the window changed; the child gets `SIGWINCH`.
    fn resize(&self, size: PtyGrid) -> Result<(), TerminalError>;
    /// Start escalating termination of the process group. Returns immediately
    /// and is idempotent: calling it on an already-dead session is not an error.
    fn kill(&self);
    /// Block until the child has been reaped, or the timeout elapses.
    /// `false` means it was still running when the wait gave up.
    fn wait_for_exit(&self, timeout: std::time::Duration) -> bool;
    /// Process id of the shell, while it is known.
    fn pid(&self) -> Option<u32>;
}
