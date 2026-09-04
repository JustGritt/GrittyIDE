//! The real PTY host: `portable-pty` plus two threads per session.
//!
//! **Reader thread.** Blocks in `read()` on the master side and appends into
//! the shared [`PendingOutput`]. It never blocks on the consumer, so a child
//! that floods (`yes`, `cat` of a large file) is never throttled into freezing
//! the UI; the buffer's cap discards the oldest bytes instead. On EOF it reaps
//! the child, records the exit status and hands the ending to the flusher.
//!
//! **Flusher thread.** Sleeps on a condition variable, so an idle terminal
//! costs no wakeups at all. The first byte after a quiet period is delivered
//! immediately (interactive echo keeps its latency), then the thread sleeps
//! [`FLUSH_INTERVAL`] before looking again, which caps delivery at ~60 Hz and
//! coalesces whatever arrived meanwhile.
//!
//! The flusher, not the reader, owns the ending: it delivers the last batch and
//! only then calls `exited`. That is what makes `TerminalExited` land exactly
//! once, after every byte the program wrote, with no output following it.
//!
//! **Termination.** `portable-pty` puts the child in its own session
//! (`setsid`), so the shell is a process-group leader and `killpg` reaches it.
//! A shell running a job puts that job in a *different* group in the same
//! session, so we signal two groups: the shell's own, and whatever group
//! currently owns the terminal. SIGHUP first — an interactive shell forwards it
//! to its jobs, which is how a closed terminal window has always worked — then
//! SIGTERM, then SIGKILL. A process that escapes into a session of its own
//! (`setsid`, a daemon) survives, exactly as it survives closing any terminal.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::batch::{FLUSH_INTERVAL, PendingOutput, READ_BUFFER_BYTES};
use crate::error::TerminalError;
use crate::host::{PtyGrid, PtyHost, PtySession, SpawnRequest, TerminalExit, TerminalSink};
use crate::shell::{COLORTERM_VALUE, TERM_VALUE, check_cwd, resolve_shell};

/// How long each termination signal is given before escalating.
const ESCALATION_STEP: Duration = Duration::from_millis(250);

/// State shared by the reader, the flusher and the public handle.
#[derive(Default)]
struct Shared {
    pending: PendingOutput,
    /// Set by the reader once the child has been reaped.
    exit: Option<TerminalExit>,
    /// Set by the flusher after `exited` was delivered to the sink.
    delivered: bool,
}

struct Channel {
    state: Mutex<Shared>,
    signal: Condvar,
}

impl Channel {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(Shared::default()),
            signal: Condvar::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn is_delivered(&self) -> bool {
        self.lock().delivered
    }

    /// Block until the ending has been delivered, or the timeout elapses.
    fn wait_delivered(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        while !state.delivered {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            let (next, timed_out) = self
                .signal
                .wait_timeout(state, remaining)
                .unwrap_or_else(PoisonError::into_inner);
            state = next;
            if timed_out.timed_out() && !state.delivered {
                return false;
            }
        }
        true
    }
}

/// Opens PTY sessions through the operating system.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemPtyHost;

impl SystemPtyHost {
    pub fn new() -> Self {
        Self
    }
}

impl PtyHost for SystemPtyHost {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Arc<dyn TerminalSink>,
    ) -> Result<Box<dyn PtySession>, TerminalError> {
        // Validated before anything is allocated: a rejected request must have
        // no side effect, and `portable-pty` would silently substitute $HOME
        // for an unusable working directory.
        check_cwd(&request.cwd)?;
        let shell = resolve_shell(request.shell.as_deref(), |key| std::env::var(key).ok())?;
        let size = request.size.sanitized();

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| TerminalError::OpenPty(error.to_string()))?;

        let mut command = CommandBuilder::new_default_prog();
        // `new_default_prog` runs `$SHELL` as a login shell (argv[0] = `-zsh`).
        // Pointing SHELL at the resolved path keeps that behaviour while
        // honouring an explicit override.
        command.env("SHELL", &shell);
        command.cwd(&request.cwd);
        command.env("TERM", TERM_VALUE);
        command.env("COLORTERM", COLORTERM_VALUE);
        for (key, value) in &request.env {
            command.env(key, value);
        }

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| TerminalError::Spawn {
                program: shell.clone(),
                source: std::io::Error::other(error.to_string()),
            })?;
        // The slave fd must go, or the master never reaches EOF and the session
        // would look alive forever after the shell exits.
        drop(pair.slave);

        let reader = pair.master.try_clone_reader().map_err(|error| {
            TerminalError::io(
                "cloning the pty reader",
                std::io::Error::other(error.to_string()),
            )
        })?;
        let writer = pair.master.take_writer().map_err(|error| {
            TerminalError::io(
                "taking the pty writer",
                std::io::Error::other(error.to_string()),
            )
        })?;
        let killer = child.clone_killer();
        let pid = child.process_id();

        let channel = Channel::new();
        spawn_reader(channel.clone(), reader, child);
        spawn_flusher(channel.clone(), sink);

        tracing::info!(
            ?pid,
            shell = %shell.display(),
            cwd = %request.cwd.display(),
            cols = size.cols,
            rows = size.rows,
            "pty session started"
        );

        Ok(Box::new(SystemSession {
            master: Arc::new(Mutex::new(pair.master)),
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            pid,
            channel,
            kill_started: AtomicBool::new(false),
        }))
    }
}

/// Drains the master side until EOF, then reaps the child.
fn spawn_reader(
    channel: Arc<Channel>,
    mut reader: Box<dyn Read + Send>,
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
) {
    std::thread::Builder::new()
        .name("gritty-pty-reader".into())
        .spawn(move || {
            let mut buffer = vec![0u8; READ_BUFFER_BYTES];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        channel.lock().pending.push(&buffer[..read]);
                        channel.signal.notify_all();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        // EIO on the master is the normal way a PTY reports
                        // that the last slave closed; it is an ending, not a
                        // failure worth surfacing.
                        tracing::debug!(%error, "pty reader stopped");
                        break;
                    }
                }
            }
            let exit = reap(child.as_mut());
            let mut state = channel.lock();
            state.exit = Some(exit);
            state.pending.finish();
            drop(state);
            channel.signal.notify_all();
        })
        .expect("spawn the pty reader thread");
}

/// Delivers coalesced batches, then the ending, exactly once.
fn spawn_flusher(channel: Arc<Channel>, sink: Arc<dyn TerminalSink>) {
    std::thread::Builder::new()
        .name("gritty-pty-flusher".into())
        .spawn(move || {
            loop {
                let mut state = channel.lock();
                // No timer while idle: an unused terminal costs nothing.
                state = channel
                    .signal
                    .wait_while(state, |shared| {
                        !shared.pending.has_output() && !shared.pending.is_finished()
                    })
                    .unwrap_or_else(PoisonError::into_inner);

                if state.pending.is_finished() {
                    let batch = state.pending.take_final();
                    let exit = state.exit.clone().unwrap_or_default();
                    drop(state);
                    // Output first, then the ending: nothing may follow it.
                    if let Some(batch) = batch {
                        sink.output(batch);
                    }
                    sink.exited(exit);
                    let mut state = channel.lock();
                    state.delivered = true;
                    drop(state);
                    channel.signal.notify_all();
                    return;
                }

                let batch = state.pending.take();
                drop(state);
                if let Some(batch) = batch {
                    sink.output(batch);
                }
                // Rate limit: whatever arrives during this sleep is coalesced
                // into the next batch.
                std::thread::sleep(FLUSH_INTERVAL);
            }
        })
        .expect("spawn the pty flusher thread");
}

/// Wait for the child and translate its ending into the domain vocabulary.
///
/// `portable-pty` renders a signal with `strsignal`, which is platform text:
/// macOS says `"Killed: 9"` where Linux says `"Killed"`. Raw platform output is
/// not domain state (`AGENTS.md`), and a protocol payload that changes wording
/// per operating system cannot be asserted on or displayed consistently. On
/// unix the child is a `std::process::Child` and `portable_pty::Child` is
/// declared downcastable, so the numeric signal is available; it is mapped to
/// its canonical `SIG*` name. If the downcast ever stops working the string
/// form is still reported rather than nothing.
///
/// The child is taken as `&mut dyn Child` on purpose: through a `Box`, method
/// resolution would find `Downcast` on the box itself and quietly hand back the
/// box's own `Any`, so every downcast would miss.
fn reap(child: &mut (dyn portable_pty::Child + Send + Sync)) -> TerminalExit {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if let Some(std_child) = child.as_any_mut().downcast_mut::<std::process::Child>() {
            return match std_child.wait() {
                Ok(status) => TerminalExit {
                    code: status.code(),
                    signal: status.signal().map(signal_name),
                },
                Err(error) => {
                    tracing::warn!(%error, "could not reap the pty child");
                    TerminalExit::default()
                }
            };
        }
    }
    match child.wait() {
        Ok(status) => TerminalExit {
            code: status.signal().is_none().then(|| status.exit_code() as i32),
            signal: status.signal().map(str::to_owned),
        },
        Err(error) => {
            tracing::warn!(%error, "could not reap the pty child");
            TerminalExit::default()
        }
    }
}

/// `9` becomes `"SIGKILL"`. An unknown number keeps its digits rather than
/// being dropped: an unexplained termination is still worth showing.
#[cfg(unix)]
fn signal_name(signal: i32) -> String {
    nix::sys::signal::Signal::try_from(signal)
        .map(|signal| signal.as_str().to_owned())
        .unwrap_or_else(|_| format!("signal {signal}"))
}

struct SystemSession {
    /// Shared with the killer thread so escalation can re-read which process
    /// group currently owns the terminal between two signals.
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Mutex<Box<dyn Write + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    pid: Option<u32>,
    channel: Arc<Channel>,
    kill_started: AtomicBool,
}

/// The process groups a termination signal should reach: the shell's own (it is
/// a session leader, so its pgid equals its pid) and the group that currently
/// owns the terminal, which is the running job when there is one.
#[cfg(unix)]
fn target_groups(master: &Mutex<Box<dyn MasterPty + Send>>, pid: Option<u32>) -> Vec<i32> {
    let mut groups = Vec::new();
    if let Some(pid) = pid {
        groups.push(pid as i32);
    }
    let foreground = master
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .process_group_leader();
    if let Some(group) = foreground
        && group > 0
        && !groups.contains(&group)
    {
        groups.push(group);
    }
    groups
}

impl PtySession for SystemSession {
    fn write(&self, data: &[u8]) -> Result<(), TerminalError> {
        if self.channel.is_delivered() {
            return Err(TerminalError::Exited);
        }
        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        writer
            .write_all(data)
            .and_then(|()| writer.flush())
            .map_err(|error| {
                // A write that races the shell's exit is an ending, not a fault.
                if self.channel.is_delivered() {
                    TerminalError::Exited
                } else {
                    TerminalError::io("writing to the pty", error)
                }
            })
    }

    fn resize(&self, size: PtyGrid) -> Result<(), TerminalError> {
        if self.channel.is_delivered() {
            return Err(TerminalError::Exited);
        }
        let size = size.sanitized();
        self.master
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .resize(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| {
                TerminalError::io("resizing the pty", std::io::Error::other(error.to_string()))
            })
    }

    fn kill(&self) {
        // Idempotent: a second close, or a close of an already-dead session,
        // must not start a second escalation or report an error.
        if self.kill_started.swap(true, Ordering::SeqCst) || self.channel.is_delivered() {
            return;
        }
        let channel = self.channel.clone();
        let master = self.master.clone();
        let pid = self.pid;
        let killer = self
            .killer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone_killer();
        // Escalation waits between signals, so it cannot run on the caller's
        // thread: closing a terminal tab must return immediately.
        std::thread::Builder::new()
            .name("gritty-pty-killer".into())
            .spawn(move || escalate(&channel, &master, pid, killer))
            .expect("spawn the pty killer thread");
    }

    fn wait_for_exit(&self, timeout: Duration) -> bool {
        self.channel.wait_delivered(timeout)
    }

    fn pid(&self) -> Option<u32> {
        self.pid
    }
}

/// SIGHUP, then SIGTERM, then SIGKILL, each given [`ESCALATION_STEP`] to work.
///
/// SIGHUP is first on purpose: an interactive shell forwards it to its jobs,
/// which is how closing a terminal window has always ended what was running in
/// it, and it gives programs like `vim` their chance to clean up.
#[cfg(unix)]
fn escalate(
    channel: &Arc<Channel>,
    master: &Mutex<Box<dyn MasterPty + Send>>,
    pid: Option<u32>,
    mut killer: Box<dyn ChildKiller + Send + Sync>,
) {
    use nix::sys::signal::Signal;
    use nix::unistd::Pid;

    for signal in [Signal::SIGHUP, Signal::SIGTERM, Signal::SIGKILL] {
        if channel.is_delivered() {
            return;
        }
        // Re-read the foreground group each time: the shell may have started or
        // finished a job since the previous signal.
        for group in target_groups(master, pid) {
            match nix::sys::signal::killpg(Pid::from_raw(group), signal) {
                // ESRCH only means the group is already gone.
                Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                Err(error) => {
                    tracing::warn!(group, ?signal, %error, "could not signal the process group");
                }
            }
        }
        if channel.wait_delivered(ESCALATION_STEP) {
            return;
        }
    }
    // The group would not go; fall back to the direct child so at least the
    // shell is reaped and the session stops being reported as running.
    tracing::warn!(
        ?pid,
        "process group survived SIGKILL; killing the child directly"
    );
    let _ = killer.kill();
}

/// Without `killpg` there is no process-group termination (ConPTY is an
/// explicit T090 non-goal): only the direct child can be reached.
#[cfg(not(unix))]
fn escalate(
    channel: &Arc<Channel>,
    _master: &Mutex<Box<dyn MasterPty + Send>>,
    _pid: Option<u32>,
    mut killer: Box<dyn ChildKiller + Send + Sync>,
) {
    if channel.is_delivered() {
        return;
    }
    let _ = killer.kill();
}

impl Drop for SystemSession {
    /// Losing the handle must not leak a shell: the registry drops sessions on
    /// close and on application exit.
    fn drop(&mut self) {
        if self.channel.is_delivered() {
            return;
        }
        self.kill();
        if !self.channel.wait_delivered(Duration::from_secs(1)) {
            tracing::warn!(pid = ?self.pid, "pty session did not exit before its handle was dropped");
        }
    }
}
