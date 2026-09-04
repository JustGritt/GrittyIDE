//! Terminal sessions (T090): the registry, the events and the path policy.
//!
//! The PTY itself lives in `gritty-terminal` behind [`PtyHost`]; this module
//! owns everything the protocol is made of — identity, the workspace-relative
//! working directory, the error envelope, and turning a session's output and
//! ending into `TerminalOutput` / `TerminalExited`. That split is why these
//! tests never spawn a shell: what is checked here is bookkeeping, and
//! bookkeeping tested against a real process is slow and flaky. The operating
//! system's behaviour is proven in `gritty-terminal`'s own tests.
//!
//! Terminal output is never persisted and never logged. It is whatever the user
//! typed and whatever a program printed, which routinely includes secrets
//! (`docs/architecture/security.md`); only sizes and session ids reach the logs.

pub mod wire;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use gritty_terminal::host::{
    PtyGrid, PtyHost, PtySession, SpawnRequest, TerminalExit, TerminalSink,
};
use gritty_terminal::{OutputBatch, TerminalError};
use gritty_workspace::WorkspacePath;

use crate::event::{CoreEvent, EventBus, EventOrigin};
use crate::protocol::actor::Actor;
use crate::protocol::error::{CoreError, CoreErrorCode};
use crate::protocol::ids::{CorrelationId, TerminalId};
use crate::protocol::time::Timestamp;
use crate::workspace::WorkspaceManager;
use wire::{
    KillTerminalRequest, ListProfilesRequest, ResizeTerminalRequest, SpawnTerminalRequest,
    Terminal, TerminalExitedPayload, TerminalOutputPayload, TerminalProfile, TerminalStatus,
    WriteTerminalRequest,
};

/// Concurrent sessions allowed. Each one costs a shell, two threads and a
/// megabyte of buffer; an unbounded number is a resource leak waiting for a
/// stuck loop in the UI to find it.
pub const MAX_SESSIONS: usize = 16;

/// Custom shells the settings may add. A bound, like every other list the
/// renderer can grow: the picker is a menu, not a database.
pub const MAX_CUSTOM_SHELLS: usize = 16;

/// Largest single `terminal.write`. A paste is legitimate; a megabyte from the
/// renderer in one command is not, and the PTY would block on it anyway.
pub const MAX_WRITE_BYTES: usize = 64 * 1024;

/// How long [`TerminalManager::shutdown`] waits for all sessions together.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

struct Entry {
    terminal: Terminal,
    /// Dropped once the session has ended and been reported.
    ///
    /// An `Arc`, not a `Box`, so a command can take a reference out of the
    /// registry and release the lock before touching the session. A host may
    /// call the sink from inside `kill` (the fake does, and any host may under
    /// a race), and the sink re-enters this manager to record the exit: doing
    /// that while the registry lock is held is a deadlock.
    session: Option<Arc<dyn PtySession>>,
}

/// Owns every terminal session in the process.
pub struct TerminalManager {
    sessions: Mutex<HashMap<TerminalId, Entry>>,
    events: Arc<EventBus>,
    workspaces: Arc<WorkspaceManager>,
    host: Arc<dyn PtyHost>,
    /// Handed to each session's sink so it can update the registry from the
    /// PTY threads without keeping the manager alive.
    self_ref: Mutex<Weak<Self>>,
}

impl TerminalManager {
    pub fn new(
        events: Arc<EventBus>,
        workspaces: Arc<WorkspaceManager>,
        host: Arc<dyn PtyHost>,
    ) -> Arc<Self> {
        let manager = Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            events,
            workspaces,
            host,
            self_ref: Mutex::new(Weak::new()),
        });
        *manager
            .self_ref
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Arc::downgrade(&manager);
        manager
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<TerminalId, Entry>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `terminal.spawn`: start a shell, in a workspace or in the home
    /// directory when the caller names none.
    pub fn spawn(
        &self,
        request: SpawnTerminalRequest,
        actor: Actor,
        correlation: CorrelationId,
    ) -> Result<Terminal, CoreError> {
        // Two rootings, one rule: the renderer may say *which workspace*, or
        // say nothing, and the core turns that into a directory. It never
        // gets to name one (ADR-010).
        let (workspace_id, cwd, resolved) = match &request.workspace_id {
            Some(id) => {
                let (workspace, root) = self.workspaces.require(id, &correlation)?;
                let cwd = request.cwd.clone().unwrap_or_else(WorkspacePath::root);
                // Resolving through the root is what keeps a terminal from
                // starting outside the workspace, symlinks included.
                let resolved = root
                    .resolve(&cwd)
                    .map_err(|error| CoreError::from(error).or_correlation(&correlation))?;
                (
                    Some(workspace.id),
                    Some(cwd),
                    resolved.native().to_path_buf(),
                )
            }
            None => {
                if request.cwd.is_some() {
                    return Err(CoreError::invalid_input(
                        "cwd",
                        "a workspace-relative directory needs a workspaceId",
                    )
                    .or_correlation(&correlation));
                }
                let home = gritty_terminal::shell::home_directory(|key| std::env::var(key).ok())
                    .map_err(|error| to_core_error(&error).or_correlation(&correlation))?;
                (None, None, home)
            }
        };

        let shell = match request.shell.as_deref() {
            Some(raw) if raw.trim().is_empty() => {
                return Err(CoreError::invalid_input("shell", "must not be empty"));
            }
            Some(raw) => {
                let path = std::path::PathBuf::from(raw);
                if !path.is_absolute() {
                    return Err(CoreError::invalid_input(
                        "shell",
                        "must be an absolute path",
                    ));
                }
                Some(path)
            }
            None => None,
        };

        let size = PtyGrid {
            cols: request.cols.unwrap_or(PtyGrid::default().cols),
            rows: request.rows.unwrap_or(PtyGrid::default().rows),
        }
        .sanitized();

        self.check_capacity()?;

        let id = TerminalId::new();
        let origin = EventOrigin {
            workspace_id: workspace_id.clone(),
            correlation_id: Some(correlation.clone()),
            actor: actor.clone(),
        };
        let sink = Arc::new(SessionSink {
            manager: self
                .self_ref
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            id: id.clone(),
            origin,
        });

        let session = self
            .host
            .spawn(
                SpawnRequest {
                    cwd: resolved,
                    shell: shell.clone(),
                    env: Vec::new(),
                    size,
                },
                sink,
            )
            .map_err(|error| to_core_error(&error).or_correlation(&correlation))?;
        let session: Arc<dyn PtySession> = Arc::from(session);

        let terminal = Terminal {
            id: id.clone(),
            workspace_id,
            cwd,
            shell: shell
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "$SHELL".into()),
            pid: session.pid(),
            status: TerminalStatus::Running,
            cols: size.cols,
            rows: size.rows,
            created_at: Timestamp::now(),
            exit_code: None,
            exit_signal: None,
        };
        tracing::info!(
            terminal_id = %id,
            pid = ?terminal.pid,
            // The home directory is a path outside any workspace, so it is
            // reported as "home" rather than written into the log.
            cwd = %terminal
                .cwd
                .as_ref()
                .map_or_else(|| "<home>".to_owned(), |cwd| cwd.to_string()),
            "terminal session opened"
        );
        // The limit is re-checked now that the registry is locked again: two
        // concurrent spawns could both have passed the first check. The loser
        // is terminated rather than silently exceeding the bound.
        let mut sessions = self.lock();
        if running(&sessions) >= MAX_SESSIONS {
            drop(sessions);
            session.kill();
            return Err(at_capacity());
        }
        sessions.insert(
            id,
            Entry {
                terminal: terminal.clone(),
                session: Some(session),
            },
        );
        Ok(terminal)
    }

    fn check_capacity(&self) -> Result<(), CoreError> {
        (running(&self.lock()) < MAX_SESSIONS)
            .then_some(())
            .ok_or_else(at_capacity)
    }

    /// `terminal.write`.
    pub fn write(&self, request: WriteTerminalRequest) -> Result<(), CoreError> {
        if request.data.len() > MAX_WRITE_BYTES {
            return Err(CoreError::invalid_input(
                "data",
                format!("at most {MAX_WRITE_BYTES} bytes per write"),
            ));
        }
        let session = self.live(&request.terminal_id)?;
        session
            .write(request.data.as_bytes())
            .map_err(|error| to_core_error(&error))
    }

    /// The live session behind an id, with the registry lock already released:
    /// every call into a session happens outside the lock.
    fn live(&self, id: &TerminalId) -> Result<Arc<dyn PtySession>, CoreError> {
        let sessions = self.lock();
        let entry = sessions.get(id).ok_or_else(|| unknown(id))?;
        entry.session.clone().ok_or_else(|| exited(id))
    }

    /// `terminal.resize`.
    pub fn resize(&self, request: ResizeTerminalRequest) -> Result<Terminal, CoreError> {
        let size = PtyGrid {
            cols: request.cols,
            rows: request.rows,
        }
        .sanitized();
        let session = self.live(&request.terminal_id)?;
        session
            .resize(size)
            .map_err(|error| to_core_error(&error))?;

        let mut sessions = self.lock();
        let entry = sessions
            .get_mut(&request.terminal_id)
            .ok_or_else(|| unknown(&request.terminal_id))?;
        entry.terminal.cols = size.cols;
        entry.terminal.rows = size.rows;
        Ok(entry.terminal.clone())
    }

    /// `terminal.kill`: terminate the process tree. Idempotent — killing a
    /// session that has already ended succeeds and changes nothing, so closing
    /// a tab twice is never an error.
    pub fn kill(&self, request: KillTerminalRequest) -> Result<Terminal, CoreError> {
        let (terminal, session) = {
            let sessions = self.lock();
            let entry = sessions
                .get(&request.terminal_id)
                .ok_or_else(|| unknown(&request.terminal_id))?;
            (entry.terminal.clone(), entry.session.clone())
        };
        if let Some(session) = session {
            tracing::info!(terminal_id = %request.terminal_id, "terminating terminal session");
            session.kill();
        }
        Ok(terminal)
    }

    /// `terminal.list`: every session, running and ended, newest last.
    pub fn list(&self) -> Vec<Terminal> {
        let mut terminals: Vec<_> = self
            .lock()
            .values()
            .map(|entry| entry.terminal.clone())
            .collect();
        // Ids are UUID v7, so sorting by id is creation order.
        terminals.sort_by(|a, b| a.id.cmp(&b.id));
        terminals
    }

    /// `terminal.profiles`: the shells the user can pick (T095).
    ///
    /// Discovered shells come from the host, which reports only what `$SHELL`
    /// and `/etc/shells` name — see `gritty_terminal::shell::discover_shells`
    /// for why guessing by file name was removed. Anything the system does not
    /// register reaches the picker through `custom`.
    ///
    /// Asked of the host every time rather than cached: a user who installs a
    /// shell expects to find it without restarting Gritty, and the cost is a
    /// handful of `stat` calls.
    pub fn profiles(
        &self,
        request: ListProfilesRequest,
    ) -> Result<Vec<TerminalProfile>, CoreError> {
        if request.custom.len() > MAX_CUSTOM_SHELLS {
            return Err(CoreError::invalid_input(
                "custom",
                format!("at most {MAX_CUSTOM_SHELLS} custom shells"),
            ));
        }

        let discovered = self.host.shells();
        let mut entries: Vec<(std::path::PathBuf, bool, bool, Option<String>)> = discovered
            .iter()
            .map(|shell| (shell.path.clone(), shell.is_default, false, None))
            .collect();

        for raw in &request.custom {
            let path = std::path::PathBuf::from(raw);
            // A custom path that discovery already found is not a second
            // entry: the user added something the system registers anyway.
            if entries.iter().any(|(known, ..)| known == &path) {
                continue;
            }
            // Reported, not dropped. The user typed this; if it no longer
            // works they are owed the reason rather than a silent absence.
            let reason = gritty_terminal::shell::check_shell(&path)
                .err()
                .map(|error| error.to_string());
            entries.push((path, false, true, reason));
        }

        // Two shells can share a file name — `/bin/bash` and Homebrew's are
        // different programs — and a menu of two entries both reading "bash"
        // is useless. Only the ambiguous ones pay for the longer label.
        let mut counts: HashMap<&std::ffi::OsStr, usize> = HashMap::new();
        for (path, ..) in &entries {
            if let Some(name) = path.file_name() {
                *counts.entry(name).or_default() += 1;
            }
        }

        Ok(entries
            .iter()
            .map(|(path, is_default, is_custom, reason)| {
                let name = match path.file_name() {
                    Some(name) if counts.get(name).copied().unwrap_or(0) > 1 => {
                        let directory = path
                            .parent()
                            .map(|parent| parent.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        format!("{} ({directory})", name.to_string_lossy())
                    }
                    Some(name) => name.to_string_lossy().into_owned(),
                    // A path ending in `..` cannot be a shell, but a label is
                    // still owed rather than an empty menu entry.
                    None => path.to_string_lossy().into_owned(),
                };
                TerminalProfile {
                    path: path.to_string_lossy().into_owned(),
                    name,
                    is_default: *is_default,
                    is_custom: *is_custom,
                    available: reason.is_none(),
                    unavailable_reason: reason.clone(),
                }
            })
            .collect())
    }

    /// Terminate every session and wait for them to be reaped. Called from the
    /// application's exit handler: leaving a shell behind would keep running
    /// after the window is gone.
    pub fn shutdown(&self) {
        // Handles are dropped outside the lock: dropping one waits for the
        // child, and holding the registry meanwhile would block every command.
        let handles: Vec<_> = {
            let mut sessions = self.lock();
            sessions
                .values_mut()
                .filter_map(|entry| entry.session.take())
                .collect()
        };
        if handles.is_empty() {
            return;
        }
        tracing::info!(count = handles.len(), "terminating terminal sessions");
        for session in &handles {
            session.kill();
        }
        let deadline = std::time::Instant::now() + SHUTDOWN_GRACE;
        for session in &handles {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if !session.wait_for_exit(remaining) {
                tracing::warn!(pid = ?session.pid(), "terminal session did not exit before shutdown");
            }
        }
    }

    /// Called from a session's flusher thread with a batch of output.
    fn on_output(&self, id: &TerminalId, origin: &EventOrigin, batch: OutputBatch) {
        if batch.truncated_bytes > 0 {
            tracing::debug!(
                terminal_id = %id,
                truncated_bytes = batch.truncated_bytes,
                "terminal output truncated"
            );
        }
        self.events.emit(
            origin,
            CoreEvent::TerminalOutput(TerminalOutputPayload {
                terminal_id: id.clone(),
                data: batch.text,
                truncated_bytes: batch.truncated_bytes,
            }),
        );
    }

    /// Called once from a session's flusher thread, after its last output.
    fn on_exit(&self, id: &TerminalId, origin: &EventOrigin, exit: TerminalExit) {
        // The handle is taken out of the registry before anything is emitted,
        // and dropped after the lock is released.
        let (snapshot, handle) = {
            let mut sessions = self.lock();
            let Some(entry) = sessions.get_mut(id) else {
                return;
            };
            if entry.terminal.status == TerminalStatus::Exited {
                return;
            }
            entry.terminal.status = TerminalStatus::Exited;
            entry.terminal.pid = None;
            entry.terminal.exit_code = exit.code;
            entry.terminal.exit_signal = exit.signal.clone();
            (entry.terminal.clone(), entry.session.take())
        };
        tracing::info!(
            terminal_id = %id,
            code = ?exit.code,
            signal = ?exit.signal,
            "terminal session ended"
        );
        self.events.emit(
            origin,
            CoreEvent::TerminalExited(TerminalExitedPayload {
                terminal: snapshot,
                code: exit.code,
                signal: exit.signal,
            }),
        );
        drop(handle);
    }
}

fn running(sessions: &HashMap<TerminalId, Entry>) -> usize {
    sessions
        .values()
        .filter(|entry| entry.terminal.status == TerminalStatus::Running)
        .count()
}

fn at_capacity() -> CoreError {
    CoreError::conflict(format!("at most {MAX_SESSIONS} terminals can run at once"))
}

fn unknown(id: &TerminalId) -> CoreError {
    CoreError::not_found(format!("terminal {id} does not exist"))
}

fn exited(id: &TerminalId) -> CoreError {
    CoreError::conflict(format!("terminal {id} has exited"))
}

/// Map the PTY layer's failures onto the error envelope. A caller mistake (a
/// shell that does not exist, a directory that is not one) must not look like
/// an infrastructure failure, and vice versa.
fn to_core_error(error: &TerminalError) -> CoreError {
    match error {
        TerminalError::Cwd { .. } => CoreError::invalid_input("cwd", error.to_string()),
        TerminalError::Spawn { source, .. } => match source.kind() {
            std::io::ErrorKind::NotFound => CoreError::not_found(error.to_string()),
            std::io::ErrorKind::PermissionDenied => {
                CoreError::new(CoreErrorCode::PermissionDenied, error.to_string())
            }
            std::io::ErrorKind::InvalidInput => {
                CoreError::invalid_input("shell", error.to_string())
            }
            _ => CoreError::new(CoreErrorCode::Io, error.to_string()),
        },
        TerminalError::Exited => CoreError::conflict(error.to_string()),
        TerminalError::OpenPty(_) | TerminalError::Io { .. } => {
            CoreError::new(CoreErrorCode::Io, error.to_string())
        }
    }
}

/// One session's route back into the manager. Holds a `Weak` so a session
/// thread that outlives the manager during shutdown cannot resurrect it.
struct SessionSink {
    manager: Weak<TerminalManager>,
    id: TerminalId,
    origin: EventOrigin,
}

impl TerminalSink for SessionSink {
    fn output(&self, batch: OutputBatch) {
        if let Some(manager) = self.manager.upgrade() {
            manager.on_output(&self.id, &self.origin, batch);
        }
    }

    fn exited(&self, exit: TerminalExit) {
        if let Some(manager) = self.manager.upgrade() {
            manager.on_exit(&self.id, &self.origin, exit);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use gritty_persistence::testing::TempDir;
    use gritty_terminal::testing::FakePtyHost;
    use tokio::runtime::Handle;

    use super::*;
    use crate::event::testing::RecordingSink;
    use crate::operation::OperationRegistry;
    use crate::protocol::ids::WorkspaceId;
    use crate::workspace::{UnsavedPolicy, Workspace};

    struct Harness {
        manager: Arc<TerminalManager>,
        host: Arc<FakePtyHost>,
        sink: Arc<RecordingSink>,
        workspace: Workspace,
        _dir: TempDir,
    }

    fn start() -> Harness {
        let dir = TempDir::new("terminal-ws");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        let sink = RecordingSink::new();
        let events = EventBus::new(sink.clone());
        let registry = OperationRegistry::new(Handle::current(), events.clone());
        let workspaces = WorkspaceManager::new(Handle::current(), events.clone(), registry);
        let workspace = workspaces
            .open(
                dir.path(),
                UnsavedPolicy::Refuse,
                Actor::User,
                CorrelationId::new(),
            )
            .unwrap();
        let host = FakePtyHost::new();
        let manager = TerminalManager::new(events, workspaces, host.clone());
        Harness {
            manager,
            host,
            sink,
            workspace,
            _dir: dir,
        }
    }

    impl Harness {
        fn spawn(&self) -> Terminal {
            self.spawn_request(SpawnTerminalRequest {
                workspace_id: Some(self.workspace.id.clone()),
                cwd: None,
                shell: None,
                cols: None,
                rows: None,
            })
        }

        fn spawn_request(&self, request: SpawnTerminalRequest) -> Terminal {
            self.manager
                .spawn(request, Actor::User, CorrelationId::new())
                .expect("spawn")
        }

        fn request(&self) -> SpawnTerminalRequest {
            SpawnTerminalRequest {
                workspace_id: Some(self.workspace.id.clone()),
                cwd: None,
                shell: None,
                cols: None,
                rows: None,
            }
        }

        fn event_names(&self) -> Vec<&'static str> {
            self.sink.names()
        }
    }

    #[tokio::test]
    async fn spawn_registers_a_running_session_in_the_workspace_root() {
        let h = start();
        let terminal = h.spawn();

        assert_eq!(terminal.status, TerminalStatus::Running);
        assert_eq!(terminal.workspace_id, Some(h.workspace.id.clone()));
        assert!(
            terminal
                .cwd
                .expect("a workspace session reports its cwd")
                .is_root()
        );
        assert_eq!(terminal.pid, Some(4242));
        assert_eq!((terminal.cols, terminal.rows), (80, 24));

        let listed = h.manager.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, terminal.id);

        let request = &h.host.requests()[0];
        assert_eq!(
            request.cwd,
            std::fs::canonicalize(h._dir.path()).unwrap(),
            "the shell starts at the workspace root"
        );
    }

    #[tokio::test]
    async fn a_subdirectory_cwd_is_resolved_inside_the_root() {
        let h = start();
        let mut request = h.request();
        request.cwd = Some(WorkspacePath::parse("src").unwrap());
        let terminal = h.spawn_request(request);

        assert_eq!(terminal.cwd.expect("a cwd").as_str(), "src");
        assert_eq!(
            h.host.requests()[0].cwd,
            std::fs::canonicalize(h._dir.join("src")).unwrap()
        );
    }

    #[tokio::test]
    async fn a_cwd_outside_the_workspace_is_refused() {
        let h = start();
        let mut request = h.request();
        // The path parser rejects traversal outright; assert the command does
        // not open a session either way.
        let error = WorkspacePath::parse("../elsewhere")
            .map(|path| {
                request.cwd = Some(path);
                h.manager
                    .spawn(request.clone(), Actor::User, CorrelationId::new())
                    .expect_err("must not escape the workspace")
            })
            .unwrap_or_else(CoreError::from);
        assert!(
            matches!(
                error.code,
                CoreErrorCode::InvalidInput | CoreErrorCode::PermissionDenied
            ),
            "unexpected code {:?}",
            error.code
        );
        assert!(h.manager.list().is_empty());
        assert!(h.host.sessions().is_empty(), "nothing was spawned");
    }

    #[tokio::test]
    async fn an_unknown_workspace_is_not_found() {
        let h = start();
        let mut request = h.request();
        request.workspace_id = Some(WorkspaceId::from_raw("nope"));
        let error = h
            .manager
            .spawn(request, Actor::User, CorrelationId::new())
            .expect_err("no such workspace");
        assert_eq!(error.code, CoreErrorCode::NotFound);
    }

    #[tokio::test]
    async fn a_relative_shell_override_is_invalid_input() {
        let h = start();
        let mut request = h.request();
        request.shell = Some("zsh".into());
        let error = h
            .manager
            .spawn(request, Actor::User, CorrelationId::new())
            .expect_err("a relative shell is not accepted");
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert!(h.host.sessions().is_empty());
    }

    #[tokio::test]
    async fn a_spawn_failure_becomes_the_matching_error_and_leaves_no_session() {
        let h = start();
        h.host.fail_next_spawn(TerminalError::Spawn {
            program: PathBuf::from("/bin/nope"),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        });
        let error = h
            .manager
            .spawn(h.request(), Actor::User, CorrelationId::new())
            .expect_err("spawn fails");
        assert_eq!(error.code, CoreErrorCode::NotFound);
        assert!(h.manager.list().is_empty(), "no half-open session remains");

        h.host
            .fail_next_spawn(TerminalError::OpenPty("no ptmx".into()));
        let error = h
            .manager
            .spawn(h.request(), Actor::User, CorrelationId::new())
            .expect_err("spawn fails");
        assert_eq!(error.code, CoreErrorCode::Io);
    }

    #[tokio::test]
    async fn output_is_forwarded_as_events_with_the_truncation_count() {
        let h = start();
        let terminal = h.spawn();
        let session = h.host.last();

        session.emit("hello");
        session.emit_batch(OutputBatch {
            text: "world".into(),
            truncated_bytes: 12,
        });

        let payloads: Vec<_> = h
            .sink
            .recorded()
            .into_iter()
            .filter_map(|envelope| match envelope.event {
                CoreEvent::TerminalOutput(payload) => Some(payload),
                _ => None,
            })
            .collect();
        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[0].terminal_id, terminal.id);
        assert_eq!(payloads[0].data, "hello");
        assert_eq!(payloads[0].truncated_bytes, 0);
        assert_eq!(payloads[1].data, "world");
        assert_eq!(
            payloads[1].truncated_bytes, 12,
            "dropped output is reported, never hidden"
        );
    }

    #[tokio::test]
    async fn exit_is_announced_once_and_marks_the_session_exited() {
        let h = start();
        let terminal = h.spawn();
        let session = h.host.last();

        session.finish(TerminalExit {
            code: Some(3),
            signal: None,
        });
        // A second ending, however it arrives, changes nothing.
        session.finish(TerminalExit {
            code: Some(0),
            signal: None,
        });

        let exits: Vec<_> = h
            .sink
            .recorded()
            .into_iter()
            .filter_map(|envelope| match envelope.event {
                CoreEvent::TerminalExited(payload) => Some(payload),
                _ => None,
            })
            .collect();
        assert_eq!(exits.len(), 1, "TerminalExited lands exactly once");
        assert_eq!(exits[0].code, Some(3));
        assert_eq!(exits[0].terminal.status, TerminalStatus::Exited);
        assert_eq!(exits[0].terminal.pid, None, "a dead pid is not reported");

        let listed = h.manager.list();
        assert_eq!(listed[0].status, TerminalStatus::Exited);
        assert_eq!(listed[0].exit_code, Some(3));
        assert_eq!(
            listed[0].id, terminal.id,
            "the session stays listed so the user can read why it ended"
        );
    }

    #[tokio::test]
    async fn output_arriving_after_the_exit_is_not_forwarded() {
        let h = start();
        h.spawn();
        let session = h.host.last();

        session.finish(TerminalExit::default());
        session.emit("too late");

        let names = h.event_names();
        let exit_at = names.iter().position(|name| *name == "TerminalExited");
        assert!(exit_at.is_some());
        assert!(
            !names[exit_at.unwrap()..].contains(&"TerminalOutput"),
            "no output may follow the exit: {names:?}"
        );
    }

    #[tokio::test]
    async fn write_reaches_the_session_and_is_bounded() {
        let h = start();
        let terminal = h.spawn();

        h.manager
            .write(WriteTerminalRequest {
                terminal_id: terminal.id.clone(),
                data: "ls -la\n".into(),
            })
            .expect("write");
        // Ctrl+C is just a control character on the same path.
        h.manager
            .write(WriteTerminalRequest {
                terminal_id: terminal.id.clone(),
                data: "\u{3}".into(),
            })
            .expect("write");
        assert_eq!(h.host.last().written(), "ls -la\n\u{3}");

        let error = h
            .manager
            .write(WriteTerminalRequest {
                terminal_id: terminal.id,
                data: "x".repeat(MAX_WRITE_BYTES + 1),
            })
            .expect_err("an oversized write is refused");
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
    }

    #[tokio::test]
    async fn resize_updates_the_session_and_the_snapshot() {
        let h = start();
        let terminal = h.spawn();

        let resized = h
            .manager
            .resize(ResizeTerminalRequest {
                terminal_id: terminal.id.clone(),
                cols: 120,
                rows: 40,
            })
            .expect("resize");
        assert_eq!((resized.cols, resized.rows), (120, 40));
        assert_eq!(
            h.host.last().record().sizes,
            vec![PtyGrid {
                cols: 120,
                rows: 40
            }]
        );
        assert_eq!(h.manager.list()[0].cols, 120);

        // A nonsense size is clamped rather than rejected: a UI mid-layout can
        // legitimately report a zero-height container.
        let clamped = h
            .manager
            .resize(ResizeTerminalRequest {
                terminal_id: terminal.id,
                cols: 0,
                rows: 0,
            })
            .expect("clamped");
        assert_eq!((clamped.cols, clamped.rows), (1, 1));
    }

    #[tokio::test]
    async fn unknown_sessions_are_not_found_on_every_command() {
        let h = start();
        let id = TerminalId::from_raw("missing");

        let write = h
            .manager
            .write(WriteTerminalRequest {
                terminal_id: id.clone(),
                data: "x".into(),
            })
            .expect_err("write");
        assert_eq!(write.code, CoreErrorCode::NotFound);

        let resize = h
            .manager
            .resize(ResizeTerminalRequest {
                terminal_id: id.clone(),
                cols: 10,
                rows: 10,
            })
            .expect_err("resize");
        assert_eq!(resize.code, CoreErrorCode::NotFound);

        let kill = h
            .manager
            .kill(KillTerminalRequest { terminal_id: id })
            .expect_err("kill");
        assert_eq!(kill.code, CoreErrorCode::NotFound);
    }

    #[tokio::test]
    async fn kill_is_idempotent_and_later_writes_conflict() {
        let h = start();
        let terminal = h.spawn();

        h.manager
            .kill(KillTerminalRequest {
                terminal_id: terminal.id.clone(),
            })
            .expect("kill");
        // Closing a tab twice, or killing a session that already ended, is not
        // an error.
        h.manager
            .kill(KillTerminalRequest {
                terminal_id: terminal.id.clone(),
            })
            .expect("second kill");

        assert_eq!(h.host.last().record().kills, 1, "one termination only");
        assert_eq!(h.manager.list()[0].status, TerminalStatus::Exited);

        let error = h
            .manager
            .write(WriteTerminalRequest {
                terminal_id: terminal.id,
                data: "x".into(),
            })
            .expect_err("the session is gone");
        assert_eq!(error.code, CoreErrorCode::Conflict);
    }

    #[tokio::test]
    async fn concurrent_sessions_are_independent_and_bounded() {
        let h = start();
        let mut ids = Vec::new();
        for _ in 0..MAX_SESSIONS {
            ids.push(h.spawn().id);
        }
        assert_eq!(h.manager.list().len(), MAX_SESSIONS);

        let error = h
            .manager
            .spawn(h.request(), Actor::User, CorrelationId::new())
            .expect_err("the limit holds");
        assert_eq!(error.code, CoreErrorCode::Conflict);

        // Output goes to the session that produced it.
        let sessions = h.host.sessions();
        sessions[0].emit("first");
        sessions[1].emit("second");
        let payloads: Vec<_> = h
            .sink
            .recorded()
            .into_iter()
            .filter_map(|envelope| match envelope.event {
                CoreEvent::TerminalOutput(payload) => Some(payload),
                _ => None,
            })
            .collect();
        assert_eq!(payloads[0].terminal_id, ids[0]);
        assert_eq!(payloads[0].data, "first");
        assert_eq!(payloads[1].terminal_id, ids[1]);

        // An ended session frees a slot.
        sessions[0].finish(TerminalExit::default());
        h.manager
            .spawn(h.request(), Actor::User, CorrelationId::new())
            .expect("a slot is free again");
    }

    #[tokio::test]
    async fn shutdown_terminates_every_running_session() {
        let h = start();
        h.spawn();
        h.spawn();
        h.spawn();
        let sessions = h.host.sessions();
        // One already ended on its own; shutdown must not fault on it.
        sessions[1].finish(TerminalExit::default());

        h.manager.shutdown();

        for session in &sessions {
            assert!(session.is_finished(), "a shell was left running");
        }
        assert!(
            h.manager
                .list()
                .iter()
                .all(|terminal| terminal.status == TerminalStatus::Exited)
        );
        // Idempotent: the exit handler may run after an explicit shutdown.
        h.manager.shutdown();
    }

    #[tokio::test]
    async fn terminal_events_carry_the_workspace_and_the_spawning_actor() {
        let h = start();
        let terminal = h.spawn();
        h.host.last().emit("x");

        let envelope = h
            .sink
            .recorded()
            .into_iter()
            .find(|envelope| matches!(envelope.event, CoreEvent::TerminalOutput(_)))
            .expect("an output event");
        assert_eq!(envelope.meta.workspace_id, terminal.workspace_id);
        assert_eq!(envelope.meta.actor, Actor::User);
        assert!(
            envelope.meta.correlation_id.is_some(),
            "output stays tied to the command that opened the session"
        );
    }

    fn shell(path: &str, is_default: bool) -> gritty_terminal::ShellProfile {
        gritty_terminal::ShellProfile {
            path: PathBuf::from(path),
            is_default,
        }
    }

    #[tokio::test]
    async fn profiles_label_shells_and_mark_the_default() {
        let harness = start();
        harness
            .host
            .set_shells(vec![shell("/bin/zsh", true), shell("/bin/bash", false)]);

        let profiles = harness
            .manager
            .profiles(ListProfilesRequest::default())
            .expect("profiles");

        assert_eq!(
            profiles
                .iter()
                .map(|p| (p.name.as_str(), p.is_default, p.is_custom, p.available))
                .collect::<Vec<_>>(),
            vec![("zsh", true, false, true), ("bash", false, false, true)]
        );
        assert_eq!(profiles[0].path, "/bin/zsh");
    }

    #[tokio::test]
    async fn two_shells_with_the_same_file_name_are_told_apart() {
        let harness = start();
        harness.host.set_shells(vec![
            shell("/bin/bash", true),
            shell("/opt/homebrew/bin/bash", false),
            shell("/bin/zsh", false),
        ]);

        let profiles = harness
            .manager
            .profiles(ListProfilesRequest::default())
            .expect("profiles");
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();

        assert_eq!(
            names,
            vec!["bash (/bin)", "bash (/opt/homebrew/bin)", "zsh"],
            "only the ambiguous entries carry a directory"
        );
    }

    #[tokio::test]
    async fn a_machine_that_reports_no_shell_produces_an_empty_list_not_an_error() {
        let harness = start();
        assert!(
            harness
                .manager
                .profiles(ListProfilesRequest::default())
                .expect("profiles")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_custom_shell_that_runs_joins_the_list() {
        let harness = start();
        harness.host.set_shells(vec![shell("/bin/zsh", true)]);
        // A real executable, because availability is a filesystem fact and
        // faking it would test nothing.
        let dir = TempDir::new("custom-shell");
        let custom = dir.join("fish");
        std::fs::write(&custom, b"#!/bin/sh\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&custom, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }

        let profiles = harness
            .manager
            .profiles(ListProfilesRequest {
                custom: vec![custom.display().to_string()],
            })
            .expect("profiles");

        let added = profiles.last().expect("the custom entry");
        assert_eq!(added.name, "fish");
        assert!(added.is_custom);
        assert!(added.available);
        assert!(!added.is_default);
    }

    #[tokio::test]
    async fn a_custom_shell_that_is_gone_is_reported_not_hidden() {
        let harness = start();
        harness.host.set_shells(vec![shell("/bin/zsh", true)]);

        let profiles = harness
            .manager
            .profiles(ListProfilesRequest {
                custom: vec!["/nope/fish".into(), "relative/fish".into()],
            })
            .expect("profiles");

        let broken: Vec<_> = profiles.iter().filter(|p| !p.available).collect();
        assert_eq!(broken.len(), 2, "the user typed these; say why they fail");
        for profile in broken {
            assert!(profile.is_custom);
            assert!(
                profile
                    .unavailable_reason
                    .as_ref()
                    .is_some_and(|reason| !reason.is_empty()),
                "an unavailable profile without a reason is a dead menu entry"
            );
        }
    }

    #[tokio::test]
    async fn a_custom_shell_the_system_already_registers_is_not_listed_twice() {
        let harness = start();
        harness.host.set_shells(vec![shell("/bin/zsh", true)]);

        let profiles = harness
            .manager
            .profiles(ListProfilesRequest {
                custom: vec!["/bin/zsh".into()],
            })
            .expect("profiles");

        assert_eq!(profiles.len(), 1);
        assert!(!profiles[0].is_custom, "the discovered entry wins");
    }

    #[tokio::test]
    async fn the_number_of_custom_shells_is_bounded() {
        let harness = start();
        let error = harness
            .manager
            .profiles(ListProfilesRequest {
                custom: (0..MAX_CUSTOM_SHELLS + 1)
                    .map(|i| format!("/bin/shell-{i}"))
                    .collect(),
            })
            .expect_err("bounded");
        assert_eq!(error.code, CoreErrorCode::InvalidInput);
    }

    #[tokio::test]
    async fn a_terminal_without_a_workspace_starts_at_home() {
        let h = start();
        let terminal = h.spawn_request(SpawnTerminalRequest {
            workspace_id: None,
            cwd: None,
            shell: None,
            cols: None,
            rows: None,
        });

        assert_eq!(terminal.status, TerminalStatus::Running);
        assert_eq!(terminal.workspace_id, None);
        assert_eq!(
            terminal.cwd, None,
            "a home directory has no workspace to be relative to"
        );

        let started = &h.host.requests()[0].cwd;
        assert!(started.is_absolute(), "{started:?}");
        assert!(started.is_dir(), "{started:?}");
        assert!(
            !started.starts_with(h._dir.path()),
            "it must not silently fall back to the open workspace"
        );
    }

    #[tokio::test]
    async fn a_relative_cwd_without_a_workspace_is_refused() {
        // There is nothing to resolve it against, and resolving it against
        // anything else is exactly what ADR-010 forbids.
        let h = start();
        let error = h
            .manager
            .spawn(
                SpawnTerminalRequest {
                    workspace_id: None,
                    cwd: Some(WorkspacePath::parse("src").unwrap()),
                    shell: None,
                    cols: None,
                    rows: None,
                },
                Actor::User,
                CorrelationId::new(),
            )
            .expect_err("a cwd needs a workspace");

        assert_eq!(error.code, CoreErrorCode::InvalidInput);
        assert!(h.host.sessions().is_empty(), "nothing was spawned");
    }

    #[tokio::test]
    async fn a_workspaceless_session_is_listed_like_any_other() {
        let h = start();
        h.spawn();
        h.spawn_request(SpawnTerminalRequest {
            workspace_id: None,
            cwd: None,
            shell: None,
            cols: None,
            rows: None,
        });

        // The registry is not partitioned by workspace: a shell is a shell,
        // and one the user can no longer see is one they cannot kill.
        let listed = h.manager.list();
        assert_eq!(listed.len(), 2);
        assert_eq!(
            listed.iter().filter(|t| t.workspace_id.is_none()).count(),
            1
        );
    }
}
