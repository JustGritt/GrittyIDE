//! Test support shared with the crates that build on this one.
//!
//! [`FakePtyHost`] is what lets `gritty-core` test its session registry without
//! spawning a shell: the domain rules that matter there (ids, exactly-once
//! exit, idempotent kill, NotFound, killing everything on shutdown) are about
//! bookkeeping, and pinning them to a real process would make those tests slow
//! and flaky. The real PTY is covered by this crate's own integration tests.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::batch::OutputBatch;
use crate::error::TerminalError;
use crate::host::{PtyGrid, PtyHost, PtySession, SpawnRequest, TerminalExit, TerminalSink};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A temporary directory removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gritty-terminal-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, name: impl AsRef<Path>) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What a fake session recorded, readable after the fact.
#[derive(Debug, Default)]
pub struct FakeRecord {
    pub written: Vec<u8>,
    pub sizes: Vec<PtyGrid>,
    pub kills: usize,
}

/// A [`PtyHost`] that starts nothing. Sessions echo nothing on their own;
/// tests drive them with [`FakeSession::emit`] and [`FakeSession::finish`].
#[derive(Default)]
pub struct FakePtyHost {
    sessions: Mutex<Vec<Arc<FakeSession>>>,
    /// When set, the next spawn fails with this error instead of succeeding.
    next_error: Mutex<Option<TerminalError>>,
    pub spawn_requests: Mutex<Vec<SpawnRequest>>,
}

impl FakePtyHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Make the next `spawn` fail, to exercise the error envelope.
    pub fn fail_next_spawn(&self, error: TerminalError) {
        *self
            .next_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(error);
    }

    /// Every session opened so far, in spawn order.
    pub fn sessions(&self) -> Vec<Arc<FakeSession>> {
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The most recently opened session.
    pub fn last(&self) -> Arc<FakeSession> {
        self.sessions().pop().expect("a session was spawned")
    }

    pub fn requests(&self) -> Vec<SpawnRequest> {
        self.spawn_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PtyHost for FakePtyHost {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Arc<dyn TerminalSink>,
    ) -> Result<Box<dyn PtySession>, TerminalError> {
        if let Some(error) = self
            .next_error
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            return Err(error);
        }
        self.spawn_requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let session = Arc::new(FakeSession {
            sink,
            record: Mutex::new(FakeRecord::default()),
            finished: AtomicBool::new(false),
            pid: 4242,
        });
        self.sessions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(session.clone());
        Ok(Box::new(FakeHandle(session)))
    }
}

/// A session that does exactly what the test tells it to.
pub struct FakeSession {
    sink: Arc<dyn TerminalSink>,
    record: Mutex<FakeRecord>,
    finished: AtomicBool,
    pid: u32,
}

impl FakeSession {
    /// Deliver a batch of output, as the flusher thread would.
    pub fn emit(&self, text: &str) {
        self.emit_batch(OutputBatch {
            text: text.into(),
            truncated_bytes: 0,
        });
    }

    /// Deliver a batch that reports dropped bytes.
    pub fn emit_batch(&self, batch: OutputBatch) {
        if self.finished.load(Ordering::SeqCst) {
            return;
        }
        self.sink.output(batch);
    }

    /// End the session. Ignored after the first call, exactly like the real
    /// flusher, so a test can assert that `TerminalExited` lands once.
    pub fn finish(&self, exit: TerminalExit) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        self.sink.exited(exit);
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    /// What the session was asked to do.
    pub fn record(&self) -> FakeRecord {
        let record = self.record.lock().unwrap_or_else(PoisonError::into_inner);
        FakeRecord {
            written: record.written.clone(),
            sizes: record.sizes.clone(),
            kills: record.kills,
        }
    }

    /// Everything written to the session so far, as text.
    pub fn written(&self) -> String {
        String::from_utf8_lossy(&self.record().written).into_owned()
    }
}

/// The handle the core holds. Dropping it ends the fake session, so the fake
/// has the same "losing the handle kills the child" contract as the real one.
struct FakeHandle(Arc<FakeSession>);

impl PtySession for FakeHandle {
    fn write(&self, data: &[u8]) -> Result<(), TerminalError> {
        if self.0.is_finished() {
            return Err(TerminalError::Exited);
        }
        self.0
            .record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .written
            .extend_from_slice(data);
        Ok(())
    }

    fn resize(&self, size: PtyGrid) -> Result<(), TerminalError> {
        if self.0.is_finished() {
            return Err(TerminalError::Exited);
        }
        self.0
            .record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sizes
            .push(size.sanitized());
        Ok(())
    }

    fn kill(&self) {
        self.0
            .record
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .kills += 1;
        self.0.finish(TerminalExit {
            code: None,
            signal: Some("SIGHUP".into()),
        });
    }

    fn wait_for_exit(&self, _timeout: Duration) -> bool {
        self.0.is_finished()
    }

    fn pid(&self) -> Option<u32> {
        Some(self.0.pid)
    }
}

impl Drop for FakeHandle {
    fn drop(&mut self) {
        if !self.0.is_finished() {
            self.kill();
        }
    }
}
