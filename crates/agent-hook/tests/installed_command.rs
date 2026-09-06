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
use std::path::PathBuf;
use std::process::Command;

use gritty_agents::hook::{HookPhase, HookScanOptions, scan};
use gritty_agents::integration::{HookTarget, IntegrationStatus, install, status, uninstall};
use gritty_agents::probe::Provider;

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
    let status_code = run_through_a_shell(&command, payload);
    assert_eq!(
        status_code,
        Some(0),
        "a hook must never be the reason an agent misbehaves (ADR-018 §5)"
    );

    let states = scan(&HookScanOptions::rooted_at(scratch.state_dir()), 0);
    assert_eq!(states.len(), 1, "one session, one file");
    assert_eq!(states[0].session_id, "e2e-abc");
    assert_eq!(states[0].phase, HookPhase::Blocked);
    assert_eq!(states[0].tool.as_deref(), Some("Bash"));

    // The tool input never left the agent's process.
    let raw = fs::read_to_string(scratch.state_dir().join("e2e-abc.json")).expect("state file");
    assert!(!raw.contains("rm -rf"), "{raw}");

    // Answering the prompt: the next hook overwrites the same file.
    let answered = r#"{"session_id":"e2e-abc","hook_event_name":"PostToolUse","tool_name":"Bash"}"#;
    assert_eq!(run_through_a_shell(&command, answered), Some(0));
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
fn run_through_a_shell(command: &str, payload: &str) -> Option<i32> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the hook");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(payload.as_bytes())
        .expect("write payload");
    child.wait().expect("wait").code()
}

#[cfg(not(unix))]
fn run_through_a_shell(_command: &str, _payload: &str) -> Option<i32> {
    let _ = Command::new("true");
    None
}
