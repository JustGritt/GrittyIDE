//! The child's lifetime, against real processes.
//!
//! Everything here spawns something. The stubs are `/bin/sh` scripts that
//! behave like the CLI in the one way each test is about — a slow start, a
//! death mid-turn, a refusal to honour SIGTERM — because the real binary needs
//! credentials CI does not have, and because a stub can be made to fail on
//! demand in ways a real agent cannot.
//!
//! Rule 12 is the reason this file exists at all: recovery behaviour is not
//! claimed without a force-kill test, and "no orphan survives" is asserted
//! against a live process id rather than against the absence of a log line.
//!
//! Unix only, like `gritty-terminal`'s pty tests. `killpg` has no equivalent
//! here on Windows, nothing in this crate has been run there, and a test that
//! did not compile on a platform would be a worse claim than an honest gap.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use gritty_agent_claude::{
    ClaudeError, ClaudeExitStatus, ClaudeLaunch, ClaudeObservation, ClaudeProcess, ClaudeVerdict,
};

/// Generous next to every wait in this file, and never reached on a healthy
/// machine: a poll against a deadline makes a slow machine slower, not flaky.
const DEADLINE: Duration = Duration::from_secs(20);

const SESSION: &str = "3f8a1c2e-9b40-4d17-8e55-6a2b7c4d9f10";

/// A directory that removes itself, the way `gritty-agent-hook`'s tests do it.
/// No `tempfile` dependency for six scripts.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "gritty-agent-claude-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or_default(),
        ));
        std::fs::create_dir_all(&path).expect("a temp directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// Write an executable `/bin/sh` stub and return its path.
    fn stub(&self, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = self.0.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write the stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make it executable");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn launch(dir: &TempDir, program: &Path) -> ClaudeLaunch {
    ClaudeLaunch::new(SESSION, dir.path())
        .expect("a valid uuid")
        .with_program(program)
}

/// Drain to the end and return everything, including the final `Exited`.
async fn drain(process: &mut ClaudeProcess) -> Vec<ClaudeObservation> {
    let mut out = Vec::new();
    while let Ok(Some(observation)) = tokio::time::timeout(DEADLINE, process.next()).await {
        out.push(observation);
    }
    out
}

fn exit_of(observations: &[ClaudeObservation]) -> &gritty_agent_claude::ClaudeExit {
    match observations.last() {
        Some(ClaudeObservation::Exited(exit)) => exit,
        other => panic!("the stream must end with an Exited, got {other:?}"),
    }
}

/// `kill(pid, 0)`: the existence check without the signal, as
/// `gritty-terminal` spells it.
fn alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

async fn until(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + DEADLINE;
    while tokio::time::Instant::now() < deadline {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    condition()
}

/// A stub that answers turns the way the real CLI does, and records the argv it
/// was given.
fn conversational(dir: &TempDir) -> PathBuf {
    let argv_log = dir.path().join("argv.txt");
    dir.stub(
        "agent.sh",
        &format!(
            r#"
: > '{log}'
for arg in "$@"; do printf '%s\n' "$arg" >> '{log}'; done
printf '%s\n' '{{"type":"system","subtype":"init","session_id":"{SESSION}","tools":[]}}'
while IFS= read -r line; do
  printf '%s\n' '{{"type":"user","message":{{"content":[{{"type":"text","text":"echo"}}]}},"isReplay":true}}'
  printf '%s\n' '{{"type":"result","subtype":"success","is_error":false,"result":"done"}}'
done
exit 0
"#,
            log = argv_log.display()
        ),
    )
}

// ---------------------------------------------------------------------------
// The happy path
// ---------------------------------------------------------------------------

/// Two turns into one process, then a clean ending. This is the acceptance
/// criterion's shape, with a stub standing in for the credentials CI lacks;
/// the real two-turn conversation is what `fixtures/two-turns-denied.jsonl`
/// recorded.
#[tokio::test]
async fn a_child_takes_many_turns_and_ends_cleanly() {
    let dir = TempDir::new("turns");
    let program = conversational(&dir);
    let mut process = ClaudeProcess::spawn(&launch(&dir, &program)).expect("spawn the stub");

    let ready = process
        .wait_ready(DEADLINE)
        .await
        .expect("the stub announces itself");
    assert!(process.is_ready());
    assert!(matches!(ready.last(), Some(ClaudeObservation::Ready(_))));

    process.prompt("first").await.expect("write a turn");
    process.prompt("second").await.expect("write another turn");
    process.close_input();

    let rest = drain(&mut process).await;
    let prompts = rest
        .iter()
        .filter(|o| matches!(o, ClaudeObservation::PromptEchoed { .. }))
        .count();
    let turns = rest
        .iter()
        .filter(|o| matches!(o, ClaudeObservation::TurnFinished(_)))
        .count();
    assert_eq!(prompts, 2, "both turns reached the same process");
    assert_eq!(turns, 2);

    let exit = exit_of(&rest);
    assert_eq!(exit.status, ClaudeExitStatus::Ok);
    assert!(!exit.unfinished);
    assert_eq!(exit.run_state(), "FINISHED");
    assert_eq!(process.stats().malformed, 0);
}

/// The launch line reaches the child unchanged. Asserting it inside the
/// process is the only way to know that nothing between here and `execve`
/// added a flag.
#[tokio::test]
async fn the_child_is_given_exactly_the_fixed_launch_line() {
    let dir = TempDir::new("argv");
    let program = conversational(&dir);
    let spec = launch(&dir, &program);
    let mut process = ClaudeProcess::spawn(&spec).expect("spawn the stub");
    process.wait_ready(DEADLINE).await.expect("ready");
    process.close_input();
    drain(&mut process).await;

    let recorded = std::fs::read_to_string(dir.path().join("argv.txt")).expect("the argv log");
    let argv: Vec<&str> = recorded.lines().collect();
    assert_eq!(argv, spec.argv());
    assert!(!recorded.contains("dangerously"), "{recorded}");
}

/// The prompt and the answer travel the same pipe, which is the economy
/// `--permission-prompt-tool stdio` buys.
#[tokio::test]
async fn a_permission_answer_goes_back_on_the_same_pipe() {
    let dir = TempDir::new("answer");
    let program = dir.stub(
        "gate.sh",
        r#"
printf '%s\n' '{"type":"system","subtype":"init","tools":[]}'
IFS= read -r prompt
printf '%s\n' '{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","description":"touch x"}}'
IFS= read -r answer
printf '%s\n' "$answer"
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"refused"}'
exit 0
"#,
    );
    let mut process = ClaudeProcess::spawn(&launch(&dir, &program)).expect("spawn the stub");
    process.wait_ready(DEADLINE).await.expect("ready");
    process.prompt("do it").await.expect("write a turn");

    let ask = tokio::time::timeout(DEADLINE, process.next())
        .await
        .expect("not stuck")
        .expect("an observation");
    let ClaudeObservation::PermissionRequested(ask) = ask else {
        panic!("expected a permission ask, got {ask:?}");
    };
    assert_eq!(ask.tool, "Bash");
    process
        .answer(&ask.request_id, &ClaudeVerdict::deny_by_default())
        .await
        .expect("write the answer");
    process.close_input();

    let rest = drain(&mut process).await;
    let answered = rest.iter().find_map(|o| match o {
        ClaudeObservation::PermissionAnswered {
            request_id,
            allowed,
        } => Some((request_id.clone(), *allowed)),
        _ => None,
    });
    assert_eq!(
        answered,
        Some(("req-1".to_owned(), false)),
        "the stub echoed the answer back and it was read as a denial"
    );
}

// ---------------------------------------------------------------------------
// Failure behaviour
// ---------------------------------------------------------------------------

/// "`claude` not on `PATH`, or not executable → the run never leaves
/// `STARTING` and ends `FAILED` with an error naming the binary. Not
/// `INTERRUPTED`: nothing was ever being supervised."
#[tokio::test]
async fn a_missing_binary_fails_before_anything_is_supervised() {
    let dir = TempDir::new("missing");
    let missing = dir.path().join("definitely-not-here");
    let error = ClaudeProcess::spawn(&launch(&dir, &missing)).expect_err("nothing to spawn");
    assert!(matches!(error, ClaudeError::Spawn { .. }));
    assert_eq!(error.core_error_code(), "Io");
    assert!(error.to_string().contains("definitely-not-here"), "{error}");
}

/// A file that exists and is not executable is the other half of the same
/// failure, and must not be reported as a working directory problem.
#[tokio::test]
async fn a_binary_that_is_not_executable_fails_the_same_way() {
    let dir = TempDir::new("noexec");
    let path = dir.path().join("not-executable");
    std::fs::write(&path, "#!/bin/sh\ntrue\n").expect("write it");
    let error = ClaudeProcess::spawn(&launch(&dir, &path)).expect_err("cannot execute it");
    assert!(matches!(error, ClaudeError::Spawn { .. }), "{error}");
}

/// A missing working directory is reported as itself. `posix_spawn` answers
/// `ENOENT` for a missing binary and a missing cwd alike, and the user fixes
/// the two differently.
#[tokio::test]
async fn a_missing_working_directory_says_so() {
    let dir = TempDir::new("nocwd");
    let program = conversational(&dir);
    let spec = ClaudeLaunch::new(SESSION, dir.path().join("gone"))
        .expect("a valid uuid")
        .with_program(&program);
    let error = ClaudeProcess::spawn(&spec).expect_err("no such directory");
    assert!(
        matches!(error, ClaudeError::WorkingDirectory { .. }),
        "{error}"
    );
    assert_eq!(error.core_error_code(), "InvalidInput");
}

/// §3: "A child that never emits `init` within a bounded wait fails the run
/// with an error that says so, rather than sitting in `STARTING`."
#[tokio::test]
async fn a_child_that_never_reports_ready_gives_up_rather_than_hanging() {
    let dir = TempDir::new("slow");
    let program = dir.stub("slow.sh", "sleep 120\n");
    let mut process = ClaudeProcess::spawn(&launch(&dir, &program)).expect("spawn the stub");
    let pid = i32::try_from(process.pid().expect("a pid")).expect("a small pid");

    let error = process
        .wait_ready(Duration::from_millis(250))
        .await
        .expect_err("it never says init");
    assert!(matches!(error, ClaudeError::NotReady { .. }), "{error}");
    assert_eq!(error.core_error_code(), "Timeout");
    assert!(!process.is_ready());

    // Giving up must not leave it running.
    process.terminate().await;
    assert!(until(|| !alive(pid)).await, "the stub survived giving up");
}

/// "Child exits non-zero, or its stdout closes mid-turn → `FAILED`, carrying
/// the exit status and a bounded tail of stderr."
#[tokio::test]
async fn a_child_that_dies_mid_turn_fails_and_carries_its_stderr() {
    let dir = TempDir::new("dies");
    let program = dir.stub(
        "dies.sh",
        r#"
printf '%s\n' '{"type":"system","subtype":"init","tools":[]}'
IFS= read -r prompt
printf 'the agent fell over\n' >&2
printf '%s' '{"type":"assistant","message":{"cont'
exit 3
"#,
    );
    let mut process = ClaudeProcess::spawn(&launch(&dir, &program)).expect("spawn the stub");
    process.wait_ready(DEADLINE).await.expect("ready");
    process.prompt("go").await.expect("write a turn");

    let rest = drain(&mut process).await;
    let exit = exit_of(&rest);
    assert_eq!(exit.status, ClaudeExitStatus::Code(3));
    assert!(exit.unfinished, "the turn never got its result");
    assert_eq!(exit.run_state(), "FAILED");
    assert!(
        exit.stderr_tail.contains("the agent fell over"),
        "{:?}",
        exit.stderr_tail
    );
    // The half-written line is malformed, and counted rather than parsed into
    // a truncated event.
    assert_eq!(process.stats().malformed, 1);
}

/// A clean exit with the work never started is still a failure: it looked like
/// it worked and nothing happened.
#[tokio::test]
async fn a_child_that_exits_zero_without_ever_starting_is_a_failure() {
    let dir = TempDir::new("instant");
    let program = dir.stub("instant.sh", "exit 0\n");
    let mut process = ClaudeProcess::spawn(&launch(&dir, &program)).expect("spawn the stub");

    let seen = process
        .wait_ready(DEADLINE)
        .await
        .expect("it exits, quickly");
    assert!(!process.is_ready());
    let exit = exit_of(&seen);
    assert_eq!(exit.status, ClaudeExitStatus::Ok);
    assert!(exit.unfinished);
    assert_eq!(exit.run_state(), "FAILED");
}

/// "Child killed … → `INTERRUPTED`, which is the state that exists for exactly
/// this." Killed from outside, by something that is not Gritty.
#[tokio::test]
async fn a_child_killed_from_outside_is_interrupted_and_not_failed() {
    let dir = TempDir::new("killed");
    let program = conversational(&dir);
    let mut process = ClaudeProcess::spawn(&launch(&dir, &program)).expect("spawn the stub");
    process.wait_ready(DEADLINE).await.expect("ready");
    let pid = i32::try_from(process.pid().expect("a pid")).expect("a small pid");

    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    )
    .expect("kill it");

    let rest = drain(&mut process).await;
    let exit = exit_of(&rest);
    assert_eq!(exit.status, ClaudeExitStatus::Signal(9));
    assert_eq!(exit.run_state(), "INTERRUPTED");
}

/// Garbage on stdout is dropped and counted, and the session still works. One
/// bad line must never poison a run.
#[tokio::test]
async fn garbage_on_stdout_is_survivable() {
    let dir = TempDir::new("garbage");
    let program = dir.stub(
        "garbage.sh",
        r#"
printf '%s\n' 'this is not json'
printf '%s\n' '{"no":"type"}'
printf '%s\n' '{"type":"system","subtype":"init","tools":[]}'
printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"fine"}'
exit 0
"#,
    );
    let mut process = ClaudeProcess::spawn(&launch(&dir, &program)).expect("spawn the stub");
    process
        .wait_ready(DEADLINE)
        .await
        .expect("it still gets ready");
    assert!(process.is_ready());
    let rest = drain(&mut process).await;
    assert_eq!(process.stats().malformed, 2);
    assert!(
        rest.iter()
            .any(|o| matches!(o, ClaudeObservation::TurnFinished(_))),
        "the good lines still arrived"
    );
}

// ---------------------------------------------------------------------------
// Cancellation, and the orphan guarantee
// ---------------------------------------------------------------------------

/// A stub that ignores SIGTERM and has started a child of its own, so both
/// rungs of the ladder and the whole process group are exercised.
fn stubborn(dir: &TempDir) -> (PathBuf, PathBuf) {
    let pidfile = dir.path().join("grandchild.pid");
    let program = dir.stub(
        "stubborn.sh",
        &format!(
            r#"
trap '' TERM
sleep 120 &
printf '%s\n' "$!" > '{pid}'
printf '%s\n' '{{"type":"system","subtype":"init","tools":[]}}'
while : ; do sleep 1 ; done
"#,
            pid = pidfile.display()
        ),
    );
    (program, pidfile)
}

async fn grandchild_of(pidfile: &Path) -> i32 {
    assert!(
        until(|| pidfile.exists()).await,
        "the stub never wrote its grandchild's pid"
    );
    std::fs::read_to_string(pidfile)
        .expect("the pid file")
        .trim()
        .parse()
        .expect("a pid")
}

/// The acceptance criterion, asserted the way rule 12 demands: a child that
/// **ignores SIGTERM** is force-killed, and the `sleep` it started dies with
/// it. Without `killpg` the grandchild would survive — which is the orphaned
/// agent the task calls the worst outcome available here.
#[tokio::test]
async fn cancellation_kills_the_whole_process_group_even_when_sigterm_is_ignored() {
    let dir = TempDir::new("stubborn");
    let (program, pidfile) = stubborn(&dir);
    let mut process = ClaudeProcess::spawn(&launch(&dir, &program)).expect("spawn the stub");
    process.wait_ready(DEADLINE).await.expect("ready");

    let child = i32::try_from(process.pid().expect("a pid")).expect("a small pid");
    let grandchild = grandchild_of(&pidfile).await;
    assert!(alive(child) && alive(grandchild), "both should be running");

    let exit = process.terminate().await;
    // SIGTERM was ignored, so the second rung is what ended it.
    assert_eq!(exit.status, ClaudeExitStatus::Signal(9));

    assert!(until(|| !alive(child)).await, "the child survived");
    assert!(
        until(|| !alive(grandchild)).await,
        "an orphaned grandchild survived the cancellation"
    );
}

/// The last line of the guarantee: a driver that panicked, or a future that
/// was abandoned, still takes the group with it. Nothing calls `terminate`
/// here — the handle is simply dropped.
#[tokio::test]
async fn dropping_the_handle_still_leaves_no_orphan() {
    let dir = TempDir::new("dropped");
    let (program, pidfile) = stubborn(&dir);
    let (child, grandchild) = {
        let mut process = ClaudeProcess::spawn(&launch(&dir, &program)).expect("spawn the stub");
        process.wait_ready(DEADLINE).await.expect("ready");
        let child = i32::try_from(process.pid().expect("a pid")).expect("a small pid");
        let grandchild = grandchild_of(&pidfile).await;
        assert!(alive(child) && alive(grandchild));
        (child, grandchild)
    };
    assert!(until(|| !alive(child)).await, "the child survived a drop");
    assert!(
        until(|| !alive(grandchild)).await,
        "an orphaned grandchild survived a drop"
    );
}

/// Writing to a run that has ended is a refusal that names the state, never a
/// panic and never a silent success.
#[tokio::test]
async fn writing_to_a_finished_run_is_refused() {
    let dir = TempDir::new("closed");
    let program = conversational(&dir);
    let mut process = ClaudeProcess::spawn(&launch(&dir, &program)).expect("spawn the stub");
    process.wait_ready(DEADLINE).await.expect("ready");
    process.close_input();
    drain(&mut process).await;

    let error = process.prompt("too late").await.expect_err("the run ended");
    assert!(matches!(error, ClaudeError::Closed { .. }), "{error}");
    assert_eq!(error.core_error_code(), "Conflict");

    let error = process
        .answer("req-1", &ClaudeVerdict::deny_by_default())
        .await
        .expect_err("the run ended");
    assert!(matches!(error, ClaudeError::Closed { .. }), "{error}");
}

/// The stream ends exactly once. A caller that drained to `None` has been told
/// how the child ended, and asking again does not produce a second ending.
#[tokio::test]
async fn the_stream_reports_one_ending_and_then_stops() {
    let dir = TempDir::new("once");
    let program = dir.stub(
        "quick.sh",
        "printf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\"}'\nexit 0\n",
    );
    let mut process = ClaudeProcess::spawn(&launch(&dir, &program)).expect("spawn the stub");
    let seen = drain(&mut process).await;
    assert_eq!(
        seen.iter()
            .filter(|o| matches!(o, ClaudeObservation::Exited(_)))
            .count(),
        1
    );
    assert!(process.next().await.is_none());
    assert!(process.next().await.is_none());
}
