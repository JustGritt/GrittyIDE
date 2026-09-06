//! The whole chain, once, for real (T221).
//!
//! Every other test in this task exercises one link: the installer writes an
//! entry, the binary writes a state file, the reader reads one. This runs the
//! command string the installer *actually produced* through a shell, the way
//! Claude Code will, and then reads what came out with the core's reader.
//!
//! It is here because it needs three things at once: a built
//! `gritty-agent-hook` (`CARGO_BIN_EXE_*` only exists for this crate's own
//! tests), the installer, and the reader. What it catches is the failure mode
//! unit tests structurally cannot — a command string that parses fine and does
//! not *run*, because of a space in an application path or a flag the binary
//! does not accept.
//!
//! Everything happens under a scratch directory. Nothing here reads or writes a
//! real `~/.claude`, and nothing writes to the application-data directory the
//! running app uses.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use gritty_agents::hook::{AskVerb, HookPhase, HookScanOptions, scan};
use gritty_agents::integration::{HookTarget, IntegrationStatus, install, status, uninstall};
use gritty_agents::probe::Provider;
use gritty_agents::verdict::{Behavior, VERDICT_VERSION, Verdict, touch_marker};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        // A space in the path on purpose: the real one is
        // `~/Library/Application Support/…`, and an unquoted command would run
        // everywhere except where it matters.
        let path = std::env::temp_dir().join(format!(
            "gritty t221 e2e {name} {}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join(".claude")).expect("create scratch home");
        Self(path)
    }

    fn config(&self) -> PathBuf {
        self.0.join(".claude").join("settings.json")
    }

    fn state_dir(&self) -> PathBuf {
        self.0
            .join("Application Support")
            .join("gritty/agents/state")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The configuration this machine really has: another tool on every event.
fn foreign_config() -> String {
    let events = [
        "Notification",
        "PermissionDenied",
        "PermissionRequest",
        "PostToolUse",
        "PostToolUseFailure",
        "PreCompact",
        "PreToolUse",
        "SessionEnd",
        "SessionStart",
        "Stop",
        "StopFailure",
        "SubagentStart",
        "SubagentStop",
        "UserPromptSubmit",
    ];
    let hooks: Vec<String> = events
        .iter()
        .map(|event| {
            format!(
                r#""{event}": [{{"matcher":"*","hooks":[{{"type":"command","command":"'/Applications/OtherTool.app/bin/OtherToolHooks' --source claude"}}]}}]"#
            )
        })
        .collect();
    format!(
        r#"{{"model":"opus","hooks":{{{}}},"theme":"dark"}}"#,
        hooks.join(",")
    )
}

#[cfg_attr(not(unix), ignore = "the command is run through a POSIX shell")]
#[test]
fn the_installed_command_runs_and_the_reader_sees_the_phase() {
    let scratch = Scratch::new("chain");
    fs::write(scratch.config(), foreign_config()).expect("write config");
    let target = HookTarget::for_provider(Provider::ClaudeCode, &scratch.0);
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_gritty-agent-hook"));

    let change = install(&target, &binary, &scratch.state_dir(), true, 1_700).expect("install");
    assert!(change.applied);
    assert_eq!(status(&target).status, IntegrationStatus::Installed);

    // The exact string that is now in the user's configuration, run the way an
    // agent runs a hook: through a shell, with the payload on stdin.
    let command = change.command.clone();
    assert!(command.contains(' '), "the paths have spaces: {command}");
    let payload = r#"{"session_id":"e2e-abc","hook_event_name":"PermissionRequest","cwd":"/Users/a/p","tool_name":"Bash","tool_input":{"command":"rm -rf /"}}"#;
    let (status_code, stdout) = run_through_a_shell(&command, payload);
    assert_eq!(
        status_code,
        Some(0),
        "a hook must never be the reason an agent misbehaves (ADR-018 §5)"
    );
    assert!(
        stdout.trim().is_empty(),
        "no Gritty is running, so the hook says nothing and the agent keeps its          own prompt (ADR-020): {stdout}"
    );

    let states = scan(&HookScanOptions::rooted_at(scratch.state_dir()), 0);
    assert_eq!(states.len(), 1, "one session, one file");
    assert_eq!(states[0].session_id, "e2e-abc");
    assert_eq!(states[0].phase, HookPhase::Blocked);
    assert_eq!(states[0].tool.as_deref(), Some("Bash"));

    // The ask names what is waiting. T221 asserted the opposite here — that no
    // tool input was ever recorded — and ADR-020 is why that changed: a prompt
    // the user answers has to say what it would do.
    let ask = states[0].ask.clone().expect("the ask");
    assert_eq!(ask.target, "rm -rf /");
    assert_eq!(ask.verb, AskVerb::RunCommand);

    // Answering the prompt: the next hook overwrites the same file.
    let answered = r#"{"session_id":"e2e-abc","hook_event_name":"PostToolUse","tool_name":"Bash"}"#;
    assert_eq!(run_through_a_shell(&command, answered).0, Some(0));
    let states = scan(&HookScanOptions::rooted_at(scratch.state_dir()), 0);
    assert_eq!(states[0].phase, HookPhase::Active);
    assert_eq!(
        fs::read_dir(scratch.state_dir()).expect("read dir").count(),
        1,
        "overwritten, never appended"
    );

    // And the file comes back to what the other tool had, entry for entry.
    let backup = scratch
        .config()
        .with_file_name("settings.json.gritty-backup-1700");
    uninstall(&target, true, 1_800).expect("uninstall");
    let restored: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(scratch.config()).expect("read"))
            .expect("restored");
    let original: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(backup).expect("backup")).expect("original");
    assert_eq!(restored, original);
    assert_eq!(status(&target).status, IntegrationStatus::NotInstalled);
}

#[cfg(unix)]
fn run_through_a_shell(command: &str, payload: &str) -> (Option<i32>, String) {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        // Captured rather than discarded: stdout is the channel a verdict
        // travels back on (ADR-020), so "the hook said nothing" is now a thing
        // this test has to be able to assert.
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the hook");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write payload");
    let output = child.wait_with_output().expect("wait");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[cfg(not(unix))]
fn run_through_a_shell(_command: &str, _payload: &str) -> (Option<i32>, String) {
    let _ = Command::new("true");
    (None, String::new())
}

/// Everything above this line is T221's chain. Everything below is ADR-020's,
/// and it is here for the same reason: these two promises cannot be kept by a
/// unit test, because what they are about is a *process* being held and let go.
///
/// The command string is the installer's own, with a short `--ask-timeout`
/// appended so a broken build fails in seconds instead of five minutes. The
/// hook takes the last value it is given, so this overrides without pretending
/// to be a different command.
#[cfg(unix)]
fn asking_command(scratch: &Scratch, binary: &Path) -> String {
    let target = HookTarget::for_provider(Provider::ClaudeCode, &scratch.0);
    let change = install(&target, binary, &scratch.state_dir(), true, 1_700).expect("install");
    format!("{} --ask-timeout 5", change.command)
}

#[cfg(unix)]
fn agents_dir(scratch: &Scratch) -> PathBuf {
    scratch
        .state_dir()
        .parent()
        .expect("the state dir has a parent")
        .to_path_buf()
}

/// Waits for the hook to record its ask and hands back the id it minted.
///
/// Polling, because the point is to answer a hook that is *already waiting* —
/// which is the only condition under which the verdict path is exercised at
/// all.
#[cfg(unix)]
fn ask_id_once_written(state_dir: &Path) -> String {
    for _ in 0..200 {
        let states = scan(&HookScanOptions::rooted_at(state_dir.to_path_buf()), 0);
        if let Some(ask) = states.first().and_then(|state| state.ask.clone()) {
            return ask.ask_id;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("the hook never recorded an ask");
}

#[test]
#[cfg(unix)]
fn a_verdict_written_while_the_hook_waits_is_what_the_agent_reads() {
    let scratch = Scratch::new("verdict");
    fs::write(scratch.config(), foreign_config()).expect("write config");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_gritty-agent-hook"));
    let command = asking_command(&scratch, &binary);
    let agents = agents_dir(&scratch);
    touch_marker(&agents.join("live")).expect("marker");

    let payload = r#"{"session_id":"verdict-abc","hook_event_name":"PermissionRequest","cwd":"/Users/a/p","tool_name":"Write","tool_input":{"file_path":"/Users/a/p/src/pty.rs","content":"fn main() {}"}}"#;
    let hook = std::thread::spawn({
        let command = command.clone();
        move || run_through_a_shell(&command, payload)
    });

    let ask_id = ask_id_once_written(&scratch.state_dir());
    gritty_agents::verdict::write(
        &agents.join("verdicts"),
        &Verdict {
            version: VERDICT_VERSION,
            ask_id: ask_id.clone(),
            behavior: Behavior::Deny,
            message: Some("pty.rs is held by your editor session".to_owned()),
        },
    )
    .expect("write the verdict");

    let (code, stdout) = hook.join().expect("the hook thread");
    assert_eq!(code, Some(0), "still exit 0, whatever the answer was");
    assert_eq!(
        stdout.trim(),
        r#"{"hookSpecificOutput":{"decision":{"behavior":"deny","message":"pty.rs is held by your editor session"},"hookEventName":"PermissionRequest"}}"#,
        "the exact shape measured in ADR-020"
    );
    assert!(
        !agents
            .join("verdicts")
            .join(format!("{ask_id}.json"))
            .exists(),
        "a verdict answers one ask, once, and is consumed as it is read"
    );
}

#[test]
#[cfg(unix)]
fn killing_gritty_mid_wait_releases_the_agent_rather_than_hanging_it() {
    // The load-bearing promise of ADR-020, and the one that cannot be reasoned
    // about (rule 12): a hook that is *already waiting* when Gritty dies must
    // let go by itself, long before its own deadline, and say nothing — so the
    // agent falls back to asking its own user.
    let scratch = Scratch::new("went-away");
    fs::write(scratch.config(), foreign_config()).expect("write config");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_gritty-agent-hook"));
    let command = asking_command(&scratch, &binary);
    let agents = agents_dir(&scratch);
    let marker = agents.join("live");
    touch_marker(&marker).expect("marker");

    let payload = r#"{"session_id":"gone-abc","hook_event_name":"PermissionRequest","cwd":"/Users/a/p","tool_name":"Bash","tool_input":{"command":"cargo publish --dry-run"}}"#;
    let started = std::time::Instant::now();
    let hook = std::thread::spawn({
        let command = command.clone();
        move || run_through_a_shell(&command, payload)
    });

    // Wait until it is genuinely waiting, then take Gritty away. Removing the
    // marker is what a force-kill looks like from inside the hook: nothing is
    // refreshing it any more.
    ask_id_once_written(&scratch.state_dir());
    fs::remove_file(&marker).expect("take Gritty away");

    let (code, stdout) = hook.join().expect("the hook thread");
    let waited = started.elapsed();
    assert_eq!(code, Some(0));
    assert!(
        stdout.trim().is_empty(),
        "silence is what leaves the agent's own prompt standing: {stdout}"
    );
    assert!(
        waited < std::time::Duration::from_secs(5),
        "released by the marker going away, not by the deadline running out \
         (waited {waited:?})"
    );
    // And the phase is still on disk: the ask was recorded before any of this,
    // so the roster still knows the session is stopped at a prompt.
    let states = scan(&HookScanOptions::rooted_at(scratch.state_dir()), 0);
    assert_eq!(states[0].phase, HookPhase::Blocked);
}
