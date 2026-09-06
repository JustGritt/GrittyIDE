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
//! A shell running a job puts that job in a *different* process group in the
//! same session, and there is no system call that signals a session, so the
//! groups have to be enumerated: [`session_groups`] reads the process table
//! once, while the shell is still alive, and keeps every group whose members
//! share the shell's session id. That set, plus the group that currently owns
//! the terminal, is what each signal goes to. SIGHUP first — it gives programs
//! like `vim` their chance to clean up, and it is what closing a terminal
//! window has always sent — then SIGTERM, then SIGKILL, and the ladder runs
//! until every one of those groups is empty rather than stopping at the shell.
//! So *ignoring* SIGHUP no longer outlives a close, which is a deliberate
//! narrowing of what `nohup` buys; *leaving the session* still does (`setsid`,
//! a daemon), exactly as it survives closing any terminal.
//!
//! Enumerating is not an optimisation of the old behaviour, it is the
//! correctness of it. Before `PTY-FLAKE` only the shell's own group and the
//! terminal's foreground group were signalled, and a backgrounded job is in
//! neither: reaching it depended entirely on the shell forwarding SIGHUP to its
//! jobs on the way out. That forwarding is not a guarantee — under load `zsh`
//! was measured exiting from SIGHUP in 9 ms without having forwarded, and a job
//! it did not forward to was orphaned permanently — and a job the shell never
//! knew about (`( sleep 300 & )`, which detaches from job control while staying
//! in the session) was leaked every single time.
//!
//! Enumerating alone was still not enough, which is the other half of the same
//! ticket: escalation used to stop the moment the shell was reaped, so a group
//! that swallowed its SIGHUP never saw a second signal. See [`escalate`].

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::batch::{FLUSH_INTERVAL, MAX_PENDING_BYTES, PendingOutput, READ_BUFFER_BYTES};
use crate::error::TerminalError;
use crate::host::{PtyGrid, PtyHost, PtySession, SpawnRequest, TerminalExit, TerminalSink};
use crate::shell::{
    COLORTERM_VALUE, ShellProfile, TERM_VALUE, check_cwd, resolve_shell, system_shells,
};

/// How long each termination signal is given before escalating.
const ESCALATION_STEP: Duration = Duration::from_millis(250);

/// Absolute path, so `PATH` cannot decide what a kill runs.
#[cfg(unix)]
const PS_PATH: &str = "/bin/ps";

/// Bound on the one process-table read a kill costs. Exceeding it costs the
/// enumeration, not the kill: escalation carries on with the groups it already
/// knows, which is what it had before this existed.
#[cfg(unix)]
const PS_TIMEOUT: Duration = Duration::from_millis(1_500);

/// How often the `ps` wait wakes.
#[cfg(unix)]
const PS_POLL_STEP: Duration = Duration::from_millis(10);

/// State shared by the reader, the flusher and the public handle.
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
    fn new(max_pending_bytes: usize) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(Shared {
                pending: PendingOutput::with_cap(max_pending_bytes),
                exit: None,
                delivered: false,
            }),
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
#[derive(Debug, Clone, Copy)]
pub struct SystemPtyHost {
    max_pending_bytes: usize,
}

impl Default for SystemPtyHost {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemPtyHost {
    pub fn new() -> Self {
        Self {
            max_pending_bytes: MAX_PENDING_BYTES,
        }
    }

    /// A host whose sessions hold less undelivered output before dropping the
    /// oldest. Tests use it to make overflow certain: whether a real flood
    /// overflows a megabyte before the flusher drains it depends on the
    /// machine, and that is not something worth asserting.
    pub fn with_max_pending_bytes(max_pending_bytes: usize) -> Self {
        Self { max_pending_bytes }
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

        let channel = Channel::new(self.max_pending_bytes);
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

    fn shells(&self) -> Vec<ShellProfile> {
        system_shells()
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

/// Every process group in the pty's session, read from the live process table.
///
/// This is the only way to reach a job the shell put in its own process group
/// without making it the foreground one — `sleep 300 &`, or anything a build
/// tool leaves running. There is no `killsid`, so the members have to be
/// enumerated and their groups collected.
///
/// It must be called *before* the first signal. Once the shell is gone its
/// children are reparented to init and the session they belonged to is no
/// longer discoverable from the shell's pid.
///
/// Failure is never fatal: an empty answer leaves escalation exactly where it
/// was before, signalling the shell's group and the foreground group.
///
/// # Safety of the answer
///
/// Every group returned is `killpg`ed, so a wrong answer signals innocent
/// processes, and widening the blast radius from one group to a whole session
/// is what makes that worth spelling out. Four guards, in the order they run:
///
/// 1. The pid must still be a session leader (`getsid(pid) == pid`), which is
///    what `setsid` made it. A pid recycled onto an unrelated process is
///    overwhelmingly unlikely to be one, so this is what stops a stale pid from
///    nominating a stranger's session for termination.
/// 2. That session must not be this process's own, or the enumeration would
///    collect Gritty's process groups and the caller would kill the application.
/// 3. Membership is asked of the kernel per process (`getsid`) rather than
///    parsed out of `ps`, so a malformed row can only be skipped, never
///    mis-attributed.
/// 4. Group 0 is never returned: `killpg(0, …)` means *the caller's own group*.
#[cfg(unix)]
fn session_groups(shell: u32) -> Vec<i32> {
    use nix::unistd::{Pid, getpgid, getpgrp, getsid};

    let shell = Pid::from_raw(shell as i32);
    let Ok(session) = getsid(Some(shell)) else {
        // The shell is already gone; there is nothing to enumerate.
        return Vec::new();
    };
    // `portable-pty` calls `setsid` in the child, so the shell leads its own
    // session and `sid == pid == pgid`. If that does not hold, this pid is not
    // the shell we spawned — most plausibly it was reaped and recycled in the
    // window between the reader reaping it and the flusher marking the session
    // delivered — and enumerating "its" session would sentence a stranger's
    // process tree.
    if session != shell {
        tracing::warn!(
            pid = shell.as_raw(),
            session = session.as_raw(),
            "the pty's pid no longer leads its own session; not enumerating it"
        );
        return Vec::new();
    }
    // Belt and braces behind the check above: never enumerate our own session.
    if getsid(None).is_ok_and(|own| own == session) {
        tracing::error!(
            session = session.as_raw(),
            "the pty shares this process's session; not enumerating it"
        );
        return Vec::new();
    }
    let own_group = getpgrp();

    let Some(table) = process_table() else {
        return Vec::new();
    };
    let mut groups = Vec::new();
    for pid in table {
        let pid = Pid::from_raw(pid);
        // `getsid` fails with EPERM for another user's processes on some
        // systems, and with ESRCH for one that exited between `ps` and here.
        // Both mean "not ours to signal".
        if getsid(Some(pid)).ok() != Some(session) {
            continue;
        }
        let Ok(group) = getpgid(Some(pid)) else {
            continue;
        };
        let group = group.as_raw();
        if group <= 1 || group == own_group.as_raw() || groups.contains(&group) {
            continue;
        }
        groups.push(group);
    }
    groups
}

/// Whether any of `groups` still has a member.
///
/// `killpg` with no signal is the existence check without the signal: `ESRCH`
/// is the one answer that means the group is empty. `EPERM` — a group we may
/// look at but not signal — counts as alive, because giving up on it silently
/// would be the same mistake this whole path exists to stop making.
#[cfg(unix)]
fn any_group_alive(groups: &[i32]) -> bool {
    groups.iter().any(|group| {
        !matches!(
            nix::sys::signal::killpg(nix::unistd::Pid::from_raw(*group), None),
            Err(nix::errno::Errno::ESRCH)
        )
    })
}

/// Every pid on the machine, from `ps`.
///
/// A subprocess rather than a system call because there is no portable one:
/// Linux has `/proc`, macOS has a `sysctl` that needs `unsafe`, and this crate
/// denies `unsafe_code` like the rest of the workspace. Only the pid column is
/// read — the session filter in [`session_groups`] is asked of the kernel, not
/// of this output — so there is nothing here for a hostile process name to
/// influence.
///
/// stdout is drained by a thread rather than after the wait: `ps -A` on a busy
/// machine can outwrite a pipe buffer, and a child blocked on a full pipe never
/// exits.
#[cfg(unix)]
fn process_table() -> Option<Vec<i32>> {
    use std::process::{Command, Stdio};

    let started = Instant::now();
    let mut child = Command::new(PS_PATH)
        .args(["-Ao", "pid="])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| tracing::warn!(%error, "could not read the process table"))
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let reader = std::thread::Builder::new()
        .name("gritty-pty-ps".into())
        .spawn(move || {
            let mut buffer = Vec::new();
            let _ = stdout.read_to_end(&mut buffer);
            buffer
        })
        .ok()?;

    let deadline = Instant::now() + PS_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            // Killing closes the pipe, which is what lets the reader thread
            // finish instead of leaking. `kill` is only ever reached while the
            // child is unreaped, so it cannot land on a recycled pid.
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!("reading the process table timed out");
                return None;
            }
            Ok(None) => std::thread::sleep(PS_POLL_STEP),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                tracing::warn!(%error, "could not wait for the process table read");
                return None;
            }
        }
    }

    let output = reader.join().ok()?;
    let pids: Vec<i32> = String::from_utf8_lossy(&output)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();
    tracing::debug!(
        pids = pids.len(),
        took_ms = started.elapsed().as_millis(),
        "read the process table for a kill"
    );
    Some(pids)
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

/// SIGHUP, then SIGTERM, then SIGKILL, each given [`ESCALATION_STEP`] to work,
/// and each sent to every process group in the pty's session.
///
/// SIGHUP is first on purpose: it is what closing a terminal window has always
/// sent, and it gives programs like `vim` their chance to clean up. What it is
/// *not* is a request for the shell to pass the signal on — the session is
/// enumerated up front precisely so that a job survives or dies on its own
/// signal rather than on the shell's cooperation.
///
/// The ladder runs until the shell has been reaped **and** every group it knew
/// about is empty. Stopping at the shell alone is not enough, and that is the
/// second half of `PTY-FLAKE`: signalling the right group is worth nothing if
/// nothing follows up when the signal does not take. A job forked moments
/// earlier can still be between `fork` and `execve`, where the shell's own
/// signal mask and dispositions are what is installed — a SIGHUP that lands
/// there can be blocked or ignored, and both states survive the `execve` into
/// `sleep`. Measured on a loaded machine, that swallowed about one SIGHUP in
/// twenty, and the job then lived forever because escalation had already
/// returned. SIGTERM and finally SIGKILL now reach it.
///
/// Nothing user-facing waits for this: the ladder runs on the killer thread and
/// the session reports its exit when the shell is reaped, so closing a tab is
/// as quick as it ever was. Only a job that shrugs off a signal pays the
/// [`ESCALATION_STEP`].
///
/// The cost is a deliberate narrowing of what survives a close. Ignoring SIGHUP
/// — what `nohup` does — is no longer enough on its own; leaving the session is
/// (`setsid`, a daemon), which is the escape hatch this module has always
/// documented. For a cockpit that supervises agents, a dev server an agent left
/// running is a thing to end with its terminal, not to inherit.
#[cfg(unix)]
fn escalate(
    channel: &Arc<Channel>,
    master: &Mutex<Box<dyn MasterPty + Send>>,
    pid: Option<u32>,
    mut killer: Box<dyn ChildKiller + Send + Sync>,
) {
    use nix::sys::signal::Signal;
    use nix::unistd::Pid;

    // Read once, before anything is signalled: a job is only attributable to
    // this session while the shell that owns it is still alive.
    let session = pid.map(session_groups).unwrap_or_default();
    tracing::debug!(?pid, ?session, "terminating the pty session");

    for signal in [Signal::SIGHUP, Signal::SIGTERM, Signal::SIGKILL] {
        // Re-read the foreground group each time: the shell may have started or
        // finished a job since the previous signal.
        let mut groups = target_groups(master, pid);
        for group in &session {
            if !groups.contains(group) {
                groups.push(*group);
            }
        }
        for group in &groups {
            match nix::sys::signal::killpg(Pid::from_raw(*group), signal) {
                // ESRCH only means the group is already gone.
                Ok(()) | Err(nix::errno::Errno::ESRCH) => {}
                Err(error) => {
                    tracing::warn!(group, ?signal, %error, "could not signal the process group");
                }
            }
        }
        if channel.wait_delivered(ESCALATION_STEP) && !any_group_alive(&groups) {
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
