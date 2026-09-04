//! Integration tests against a real pseudo-terminal.
//!
//! These are the T090 spike evidence: everything here spawns an actual shell,
//! writes to it, resizes it and kills it. The domain-level rules (ids, registry
//! bookkeeping) are tested in `gritty-core` against the fake host; what is
//! proven here is that the operating system behaves as the design assumes.
//!
//! Every wait is a poll against a deadline rather than a fixed sleep, so a slow
//! machine makes the tests slower, never flaky.

#![cfg(unix)]

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use gritty_terminal::batch::MAX_PENDING_BYTES;
use gritty_terminal::host::{
    PtyGrid, PtyHost, PtySession, SpawnRequest, TerminalExit, TerminalSink,
};
use gritty_terminal::shell::platform_default_shell;
use gritty_terminal::testing::TempDir;
use gritty_terminal::{OutputBatch, SystemPtyHost, TerminalError};

/// Generous: a login shell sources the user's profile before it answers.
const DEADLINE: Duration = Duration::from_secs(20);

/// Output kept for assertions. Bounds what a flood test holds in memory.
const MAX_RECORDED_TEXT: usize = 256 * 1024;

/// How much of the recorded output a failure prints.
const FAILURE_EXCERPT: usize = 2048;

#[derive(Default)]
struct Recorded {
    text: String,
    truncated_bytes: u64,
    batches: usize,
    largest_batch: usize,
    /// Output delivered after `exited`, which must never happen.
    output_after_exit: usize,
    exits: Vec<TerminalExit>,
}

#[derive(Default)]
struct TestSink {
    recorded: Mutex<Recorded>,
    exited: AtomicBool,
    output_calls: AtomicUsize,
    /// Time spent inside `output`, simulating a consumer that cannot keep up.
    consumer_delay: Duration,
}

impl TestSink {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A sink that is slower than the shell writing into it. Overflow then
    /// depends on the *consumer* falling behind, which is the situation the
    /// cap exists for, rather than on the machine being fast enough to outrun
    /// a 60 Hz drain — which a developer's Mac is and a CI runner is not.
    fn slow() -> Arc<Self> {
        Arc::new(Self {
            consumer_delay: Duration::from_millis(50),
            ..Self::default()
        })
    }

    fn text(&self) -> String {
        self.lock().text.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Recorded> {
        self.recorded.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn has_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    /// Poll until `predicate` holds or the deadline passes.
    fn wait_for(&self, what: &str, predicate: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + DEADLINE;
        while Instant::now() < deadline {
            if predicate(self) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // A flood test's output is megabytes of `y`; print enough to diagnose.
        let text = self.text();
        let excerpt: String = text.chars().take(FAILURE_EXCERPT).collect();
        let elided = if text.len() > excerpt.len() {
            " […]"
        } else {
            ""
        };
        panic!("timed out waiting for {what}; output so far:\n{excerpt}{elided}");
    }

    fn wait_for_text(&self, needle: &str) {
        self.wait_for(&format!("{needle:?} in the output"), |sink| {
            sink.text().contains(needle)
        });
    }

    fn wait_for_exit(&self) {
        self.wait_for("the session to exit", Self::has_exited);
    }
}

impl TerminalSink for TestSink {
    fn output(&self, batch: OutputBatch) {
        if !self.consumer_delay.is_zero() {
            std::thread::sleep(self.consumer_delay);
        }
        self.output_calls.fetch_add(1, Ordering::SeqCst);
        let exited = self.exited.load(Ordering::SeqCst);
        let mut recorded = self.lock();
        if exited {
            recorded.output_after_exit += 1;
        }
        recorded.batches += 1;
        recorded.largest_batch = recorded.largest_batch.max(batch.text.len());
        recorded.truncated_bytes += batch.truncated_bytes;
        // A flood test would otherwise hoard every byte `yes` produced. Every
        // marker the tests look for is printed before the flood starts, so
        // keeping the head is enough.
        if recorded.text.len() < MAX_RECORDED_TEXT {
            recorded.text.push_str(&batch.text);
        }
    }

    fn exited(&self, exit: TerminalExit) {
        self.lock().exits.push(exit);
        self.exited.store(true, Ordering::SeqCst);
    }
}

/// Marker prompt the generated rc files print. Waiting for it is how a test
/// knows the shell is ready for input.
const PROMPT: &str = "GRITTY> ";

/// A home directory with rc files that make the shell deterministic.
///
/// Without this the tests run the developer's own login shell: a framework like
/// oh-my-zsh asks about updates, and an interactive line editor with
/// autosuggestions rewrites input as it arrives, so a line written in one go
/// comes back mangled. `HOME`/`ZDOTDIR` point here instead, the rc files set a
/// known prompt, and zsh's line editor is turned off so what is written is what
/// the shell reads.
fn pristine_home() -> TempDir {
    let home = TempDir::new("home");
    let zshrc = format!(
        "unsetopt zle\nunsetopt prompt_cr\nunsetopt prompt_sp\nPS1='{PROMPT}'\nPROMPT='{PROMPT}'\n"
    );
    std::fs::write(home.join(".zshrc"), &zshrc).expect("write .zshrc");
    std::fs::write(home.join(".zshenv"), &zshrc).expect("write .zshenv");
    let bashrc = format!("PS1='{PROMPT}'\n");
    std::fs::write(home.join(".bashrc"), &bashrc).expect("write .bashrc");
    std::fs::write(home.join(".bash_profile"), &bashrc).expect("write .bash_profile");
    std::fs::write(home.join(".profile"), &bashrc).expect("write .profile");
    home
}

fn request(cwd: &std::path::Path, home: &TempDir) -> SpawnRequest {
    SpawnRequest {
        cwd: cwd.to_path_buf(),
        // Pinned rather than `$SHELL`: the same shell must run on the
        // developer's machine and in CI.
        shell: Some(platform_default_shell().into()),
        env: vec![
            ("HOME".into(), home.path().display().to_string()),
            ("ZDOTDIR".into(), home.path().display().to_string()),
        ],
        size: PtyGrid { cols: 80, rows: 24 },
    }
}

/// A live session in a fresh directory, already showing its first prompt.
struct Fixture {
    _dir: TempDir,
    _home: TempDir,
    session: Box<dyn PtySession>,
    sink: Arc<TestSink>,
}

impl Fixture {
    fn new() -> Self {
        Self::with(TempDir::new("pty"), TestSink::new())
    }

    fn in_dir(dir: TempDir) -> Self {
        Self::with(dir, TestSink::new())
    }

    fn with_sink(sink: Arc<TestSink>) -> Self {
        Self::with(TempDir::new("pty"), sink)
    }

    fn with(dir: TempDir, sink: Arc<TestSink>) -> Self {
        let home = pristine_home();
        let session = SystemPtyHost::new()
            .spawn(request(dir.path(), &home), sink.clone())
            .expect("spawn a shell");
        // Input written before the shell is ready is simply lost; a user types
        // after seeing a prompt, and so does this test.
        sink.wait_for_text(PROMPT);
        Self {
            _dir: dir,
            _home: home,
            session,
            sink,
        }
    }

    fn session(&self) -> &dyn PtySession {
        self.session.as_ref()
    }

    fn run(&self, line: &str) {
        self.session
            .write(format!("{line}\n").as_bytes())
            .expect("write to the pty");
    }
}

/// `Box<dyn PtySession>` is not `Debug`, so `expect_err` cannot be used on a
/// spawn result; this says the same thing.
fn spawn_error(result: Result<Box<dyn PtySession>, TerminalError>, what: &str) -> TerminalError {
    match result {
        Ok(_) => panic!("expected {what} to fail, but a session was opened"),
        Err(error) => error,
    }
}

#[test]
fn spawns_a_shell_streams_output_and_exits_cleanly() {
    let fixture = Fixture::new();
    assert!(fixture.session().pid().is_some(), "the shell has a pid");

    fixture.run("echo gritty-marker-ok");
    fixture.sink.wait_for_text("gritty-marker-ok");

    fixture.run("exit 0");
    fixture.sink.wait_for_exit();

    let recorded = fixture.sink.lock();
    assert_eq!(recorded.exits.len(), 1, "TerminalExited lands exactly once");
    assert_eq!(recorded.exits[0].code, Some(0));
    assert_eq!(recorded.exits[0].signal, None);
    assert_eq!(
        recorded.output_after_exit, 0,
        "no output may follow the exit"
    );
}

#[test]
fn reports_a_non_zero_exit_code() {
    let fixture = Fixture::new();
    fixture.run("exit 3");
    fixture.sink.wait_for_exit();
    assert_eq!(fixture.sink.lock().exits[0].code, Some(3));
}

#[test]
fn the_users_own_login_shell_starts() {
    // The other tests pin a shell for determinism; this one proves the default
    // resolution path works with whatever the developer or CI actually uses.
    let dir = TempDir::new("pty-default-shell");
    let sink = TestSink::new();
    let session = SystemPtyHost::new()
        .spawn(
            SpawnRequest {
                cwd: dir.path().to_path_buf(),
                shell: None,
                env: Vec::new(),
                size: PtyGrid { cols: 80, rows: 24 },
            },
            sink.clone(),
        )
        .expect("the user's login shell starts");
    assert!(session.pid().is_some());
    // Only that it produces something: the contents belong to whoever's
    // configuration this is.
    sink.wait_for("the shell to write something", |sink| {
        !sink.text().is_empty()
    });
    session.kill();
    sink.wait_for_exit();
}

#[test]
fn the_child_starts_in_the_requested_directory() {
    // The temp dir may be reached through a symlink (/tmp on macOS); compare
    // against what the shell itself will report.
    let dir = TempDir::new("pty-cwd");
    let expected = std::fs::canonicalize(dir.path()).expect("canonicalize");
    let fixture = Fixture::in_dir(dir);

    fixture.run("pwd");
    fixture.sink.wait_for_text(&expected.display().to_string());
    fixture.session().kill();
}

#[test]
fn the_environment_carries_term_and_the_caller_additions() {
    let fixture = Fixture::new();
    fixture.run("echo term=$TERM colorterm=$COLORTERM");
    fixture
        .sink
        .wait_for_text("term=xterm-256color colorterm=truecolor");
    fixture.session().kill();
}

#[test]
fn resize_reaches_the_child() {
    let fixture = Fixture::new();

    fixture.run("stty size");
    fixture.sink.wait_for_text("24 80");

    fixture
        .session()
        .resize(PtyGrid {
            cols: 100,
            rows: 30,
        })
        .expect("resize");
    fixture.run("stty size");
    fixture.sink.wait_for_text("30 100");

    fixture.session().kill();
}

#[test]
fn an_absurd_size_is_clamped_rather_than_rejected() {
    let fixture = Fixture::new();
    fixture
        .session()
        .resize(PtyGrid { cols: 0, rows: 0 })
        .expect("a zero size is clamped, not an error");
    fixture.run("stty size");
    fixture.sink.wait_for_text("1 1");
    fixture.session().kill();
}

#[test]
fn kill_terminates_the_whole_process_tree() {
    let dir = TempDir::new("pty-tree");
    let pid_file = dir.join("child.pid");
    let fixture = Fixture::in_dir(dir);

    // A background job lives in its own process group under job control; it is
    // reached because SIGHUP goes to the shell, which forwards it to its jobs.
    fixture.run(&format!(
        "sleep 300 & echo $! > {}; echo started",
        pid_file.display()
    ));
    fixture.sink.wait_for_text("started");
    fixture
        .sink
        .wait_for("the child pid file", |_| pid_file.exists());

    let child_pid: i32 = std::fs::read_to_string(&pid_file)
        .expect("read the pid file")
        .trim()
        .parse()
        .expect("a pid");
    assert!(
        is_alive(child_pid),
        "the grandchild is running before the kill"
    );

    fixture.session().kill();
    fixture.sink.wait_for_exit();

    // The shell is reaped first; give the grandchild the escalation window.
    let deadline = Instant::now() + DEADLINE;
    while is_alive(child_pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !is_alive(child_pid),
        "the grandchild ({child_pid}) survived the kill"
    );
}

#[test]
fn an_output_storm_is_bounded_and_reports_what_it_dropped() {
    // The consumer is deliberately slower than the shell, so the buffer fills
    // whatever the machine's speed. What is asserted is the mechanism: the cap
    // holds, the oldest bytes go, and the count of what went is reported.
    let fixture = Fixture::with_sink(TestSink::slow());
    fixture.run("echo storm-ready");
    fixture.sink.wait_for_text("storm-ready");

    fixture.run("yes gritty");
    fixture
        .sink
        .wait_for("the buffer to overflow and report it", |sink| {
            sink.lock().truncated_bytes > 0
        });

    fixture.session().kill();
    fixture.sink.wait_for_exit();

    let recorded = fixture.sink.lock();
    assert!(
        recorded.largest_batch <= MAX_PENDING_BYTES + 4,
        "a batch never exceeds the cap (was {})",
        recorded.largest_batch
    );
    assert_eq!(
        recorded.output_after_exit, 0,
        "no output may follow the exit, flood or not"
    );
}

#[test]
fn output_delivery_is_rate_limited_and_does_not_starve_the_control_path() {
    // No assertion here depends on the machine being fast: a slower machine
    // produces *fewer* batches, which still satisfies the ceiling.
    let fixture = Fixture::new();
    fixture.run("echo storm-ready");
    fixture.sink.wait_for_text("storm-ready");

    let started = Instant::now();
    fixture.run("yes gritty");
    std::thread::sleep(Duration::from_millis(1500));
    let flooding_for = started.elapsed();

    // A resize still gets through while the child writes as fast as it can.
    fixture
        .session()
        .resize(PtyGrid { cols: 90, rows: 30 })
        .expect("resize during a flood");

    fixture.session().kill();
    fixture.sink.wait_for_exit();

    let recorded = fixture.sink.lock();
    assert!(
        recorded.batches > 0,
        "the flood produced no output at all, which means the test proved nothing"
    );
    // ~60 Hz plus slack; the point is that the sink is not called once per read.
    let ceiling = (flooding_for.as_secs_f64() * 70.0).ceil() as usize + 20;
    assert!(
        recorded.batches <= ceiling,
        "delivery must be rate limited: {} batches in {:?} (ceiling {ceiling})",
        recorded.batches,
        flooding_for
    );
}

#[test]
fn killing_the_child_unexpectedly_reports_termination() {
    // Addendum §11.2: the child dies without Gritty asking.
    let fixture = Fixture::new();
    fixture.run("echo alive");
    fixture.sink.wait_for_text("alive");

    let pid = fixture.session().pid().expect("a pid") as i32;
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("kill the shell");

    fixture.sink.wait_for_exit();
    let recorded = fixture.sink.lock();
    assert_eq!(recorded.exits.len(), 1);
    assert_eq!(
        recorded.exits[0].signal.as_deref(),
        Some("SIGKILL"),
        "the exit reason names the signal"
    );
    assert_eq!(recorded.exits[0].code, None);
}

#[test]
fn kill_is_idempotent_and_writes_after_the_exit_are_refused() {
    let fixture = Fixture::new();
    fixture.run("echo ready");
    fixture.sink.wait_for_text("ready");

    fixture.session().kill();
    fixture.session().kill();
    fixture.session().kill();
    fixture.sink.wait_for_exit();

    assert_eq!(fixture.sink.lock().exits.len(), 1, "still exactly one exit");
    assert!(fixture.session().wait_for_exit(Duration::from_secs(1)));

    let error = fixture
        .session()
        .write(b"echo late\n")
        .expect_err("write after exit");
    assert!(matches!(error, TerminalError::Exited));
    let error = fixture
        .session()
        .resize(PtyGrid { cols: 10, rows: 10 })
        .expect_err("resize after exit");
    assert!(matches!(error, TerminalError::Exited));
}

#[test]
fn dropping_the_handle_kills_the_shell() {
    let fixture = Fixture::new();
    fixture.run("echo ready");
    fixture.sink.wait_for_text("ready");
    let pid = fixture.session().pid().expect("a pid") as i32;

    let Fixture {
        _dir,
        _home,
        session,
        sink,
    } = fixture;
    drop(session);

    sink.wait_for_exit();
    let deadline = Instant::now() + DEADLINE;
    while is_alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!is_alive(pid), "the shell survived losing its handle");
}

#[test]
fn several_sessions_run_at_once_without_crossing_output() {
    let first = Fixture::new();
    let second = Fixture::new();

    first.run("echo from-session-a");
    second.run("echo from-session-b");
    first.sink.wait_for_text("from-session-a");
    second.sink.wait_for_text("from-session-b");

    assert!(!first.sink.text().contains("from-session-b"));
    assert!(!second.sink.text().contains("from-session-a"));
    assert_ne!(first.session().pid(), second.session().pid());

    first.session().kill();
    second.session().kill();
}

#[test]
fn a_missing_shell_is_a_spawn_error_with_no_session() {
    let dir = TempDir::new("pty-missing-shell");
    let home = pristine_home();
    let mut with_missing_shell = request(dir.path(), &home);
    with_missing_shell.shell = Some(dir.join("no-such-shell"));
    let error = spawn_error(
        SystemPtyHost::new().spawn(with_missing_shell, TestSink::new()),
        "a missing shell",
    );
    assert!(matches!(error, TerminalError::Spawn { .. }));
    assert!(error.is_caller_fault());
}

#[test]
fn a_working_directory_that_is_not_a_directory_is_rejected() {
    let dir = TempDir::new("pty-bad-cwd");
    let home = pristine_home();
    let file = dir.join("a-file");
    std::fs::write(&file, b"x").expect("write");

    let mut on_a_file = request(dir.path(), &home);
    on_a_file.cwd = file;
    let error = spawn_error(
        SystemPtyHost::new().spawn(on_a_file, TestSink::new()),
        "a file as the working directory",
    );
    assert!(matches!(error, TerminalError::Cwd { .. }));

    let mut on_nothing = request(dir.path(), &home);
    on_nothing.cwd = dir.join("missing");
    let error = spawn_error(
        SystemPtyHost::new().spawn(on_nothing, TestSink::new()),
        "a missing working directory",
    );
    assert!(matches!(error, TerminalError::Cwd { .. }));
}

/// `kill(pid, 0)` succeeds while the process exists.
fn is_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}
