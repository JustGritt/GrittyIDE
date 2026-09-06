//! `gritty-agent-hook`: the whole of Gritty inside somebody else's process
//! (T221, ADR-018).
//!
//! An agent CLI runs this when it blocks on a permission prompt, when a tool
//! starts or finishes, and when a turn or a session ends. It reads one JSON
//! payload on stdin, writes one small state file, and exits `0`.
//!
//! Everything about it is shaped by one constraint: **an agent must be no
//! slower and no less reliable for having Gritty installed** (ADR-018 §5). So
//! it performs no network access, starts no process, holds no lock, reads no
//! configuration, links nothing of `gritty-core`, and — most importantly —
//! **always exits `0`**. A non-zero exit from a hook is the agent's problem to
//! handle, and Gritty must never be the reason another tool misbehaves. Every
//! failure here is silent to the agent and visible only inside Gritty, as a
//! stale or missing state file.
//!
//! It also knows nothing of the Gritty protocol (§6). It names a *phase* — what
//! the agent said — and the core decides what that means for a roster row. The
//! state directory arrives on the command line rather than being computed here,
//! so this binary carries no platform knowledge about where Gritty's app data
//! lives and honours `GRITTY_DATA_DIR` for free.
//!
//! The binary is a shell around [`run`], so every decision it makes is tested
//! against a scratch directory rather than by launching a process.

use std::path::{Path, PathBuf};

use gritty_agents::hook::{
    HOOK_STATE_VERSION, HookState, WriteOutcome, phase_of_event, state_dir_from_data_dir, write,
};
use gritty_agents::probe::Provider;
use serde_json::Value;

/// What the hook did. Only ever reported to a test: the process exits `0`
/// whichever of these it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A state file was written.
    Recorded(WriteOutcome),
    /// The event is not one this build turns into a phase, so nothing was
    /// written. A phase Gritty does not understand must not overwrite one it
    /// does.
    UnknownEvent(String),
    /// The payload was not a JSON object, or carried no session id, or named no
    /// event. Nothing to record.
    UnusablePayload,
    /// No `--state-dir`, or an argument this build does not know.
    Misconfigured,
}

/// The command line, parsed. Deliberately hand-rolled: three flags do not
/// justify an argument-parsing dependency in a binary whose whole virtue is
/// that it does almost nothing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    pub provider: Option<Provider>,
    /// Where the state files go. Gritty writes its own app-data path here at
    /// install time, so this binary never has to guess at one.
    pub state_dir: Option<PathBuf>,
}

/// `--provider <name>` and one of `--state-dir <dir>` / `--data-dir <dir>`.
///
/// Unknown arguments are ignored rather than refused: a future Gritty may add
/// one, and an old hook binary left behind by an upgrade must keep working
/// rather than start failing in the middle of somebody's session.
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Args {
    let mut parsed = Args::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--provider" => {
                parsed.provider = args.next().as_deref().and_then(provider_of_name);
            }
            "--state-dir" => {
                parsed.state_dir = args.next().map(PathBuf::from);
            }
            "--data-dir" => {
                parsed.state_dir = args
                    .next()
                    .map(|dir| state_dir_from_data_dir(Path::new(&dir)));
            }
            _ => {}
        }
    }
    parsed
}

fn provider_of_name(name: &str) -> Option<Provider> {
    match name {
        "claude-code" | "claude_code" => Some(Provider::ClaudeCode),
        _ => None,
    }
}

/// Read one payload, write one state file.
///
/// `now_ms` is injected so the ordering rule ("an older `since` never moves the
/// phase backwards") is a test rather than a claim.
pub fn run(args: &Args, payload: &str, now_ms: i64) -> Outcome {
    let Some(state_dir) = args.state_dir.as_deref() else {
        return Outcome::Misconfigured;
    };
    // No provider named is Claude Code: the only one this build supports, and
    // an older installed command line may not carry the flag at all.
    let provider = args.provider.unwrap_or(Provider::ClaudeCode);

    let Ok(Value::Object(payload)) = serde_json::from_str::<Value>(payload) else {
        return Outcome::UnusablePayload;
    };
    let Some(session_id) = string_of(&payload, &["session_id", "sessionId"]) else {
        return Outcome::UnusablePayload;
    };
    let Some(event) = string_of(&payload, &["hook_event_name", "hookEventName", "event"]) else {
        return Outcome::UnusablePayload;
    };
    let Some(phase) = phase_of_event(&event) else {
        return Outcome::UnknownEvent(event);
    };

    // Fields are read, never interpreted: the tool name is a label, the working
    // directory is a display string, and neither is resolved, opened or run.
    // `write` clips both again before they reach the disk (§4).
    let state = HookState {
        version: HOOK_STATE_VERSION,
        session_id,
        provider,
        phase,
        since_ms: now_ms,
        event,
        tool: string_of(&payload, &["tool_name", "toolName"]),
        cwd: string_of(&payload, &["cwd", "workingDirectory"]),
    };
    Outcome::Recorded(write(state_dir, &state))
}

/// The first of several spellings that is a non-empty string.
///
/// Claude Code sends `snake_case`; accepting `camelCase` beside it costs one
/// lookup and means a provider that spells it the other way (T222) needs no
/// change here.
fn string_of(payload: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gritty_agents::hook::{HookPhase, HookScanOptions, scan};

    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gritty-agent-hook-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }

        fn args(&self) -> Args {
            Args {
                provider: Some(Provider::ClaudeCode),
                state_dir: Some(self.0.clone()),
            }
        }

        fn states(&self) -> Vec<HookState> {
            scan(&HookScanOptions::rooted_at(self.0.clone()), 0)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn payload(event: &str) -> String {
        format!(
            r#"{{"session_id":"abc-123","hook_event_name":"{event}","cwd":"/Users/someone/code/gritty","tool_name":"Bash","tool_input":{{"command":"rm -rf /"}}}}"#
        )
    }

    #[test]
    fn each_event_writes_the_phase_it_names() {
        for (event, expected) in [
            ("PermissionRequest", HookPhase::Blocked),
            ("PermissionDenied", HookPhase::Active),
            ("PreToolUse", HookPhase::Active),
            ("PostToolUse", HookPhase::Active),
            ("PostToolUseFailure", HookPhase::Active),
            ("UserPromptSubmit", HookPhase::Active),
            ("SessionStart", HookPhase::Active),
            ("Stop", HookPhase::Settled),
            ("StopFailure", HookPhase::Settled),
            ("SessionEnd", HookPhase::Ended),
        ] {
            let dir = TempDir::new(&format!("event-{event}"));
            let outcome = run(&dir.args(), &payload(event), 1_000);
            assert_eq!(outcome, Outcome::Recorded(WriteOutcome::Written), "{event}");
            let states = dir.states();
            assert_eq!(states.len(), 1, "{event}");
            assert_eq!(states[0].phase, expected, "{event}");
            assert_eq!(states[0].event, event);
            assert_eq!(states[0].session_id, "abc-123");
        }
    }

    #[test]
    fn a_permission_request_names_the_tool_it_is_waiting_on() {
        let dir = TempDir::new("tool");
        run(&dir.args(), &payload("PermissionRequest"), 1_000);
        assert_eq!(dir.states()[0].tool.as_deref(), Some("Bash"));
    }

    #[test]
    fn the_tool_input_is_never_recorded() {
        // The state file is a phase, not a transcript. A tool input is exactly
        // the kind of thing that carries a secret (rule 13), and the core has
        // the transcript's own bounded preview for the detail it shows.
        let dir = TempDir::new("no-input");
        run(&dir.args(), &payload("PermissionRequest"), 1_000);
        let raw = fs::read_to_string(dir.0.join("abc-123.json")).expect("read state");
        assert!(!raw.contains("rm -rf"), "{raw}");
        assert!(!raw.contains("tool_input"), "{raw}");
    }

    #[test]
    fn an_event_this_build_does_not_understand_writes_nothing() {
        let dir = TempDir::new("unknown-event");
        let outcome = run(&dir.args(), &payload("Notification"), 1_000);
        assert_eq!(outcome, Outcome::UnknownEvent("Notification".to_owned()));
        assert!(
            dir.states().is_empty(),
            "a phase we do not know stays unwritten"
        );
        assert!(!dir.0.exists(), "and the directory is not even created");
    }

    #[test]
    fn malformed_stdin_records_nothing_and_says_nothing() {
        let dir = TempDir::new("malformed");
        for raw in ["", "not json", "[]", "null", "{}", r#"{"session_id":"a"}"#] {
            assert_eq!(
                run(&dir.args(), raw, 1_000),
                Outcome::UnusablePayload,
                "{raw:?}"
            );
        }
        assert!(dir.states().is_empty());
    }

    #[test]
    fn an_unwritable_state_directory_is_reported_and_never_fatal() {
        let dir = TempDir::new("unwritable");
        fs::create_dir_all(&dir.0).expect("create");
        let blocker = dir.0.join("blocker");
        fs::write(&blocker, b"a file where a directory would go").expect("write");
        let args = Args {
            provider: Some(Provider::ClaudeCode),
            state_dir: Some(blocker.join("state")),
        };
        let outcome = run(&args, &payload("PermissionRequest"), 1_000);
        assert!(
            matches!(outcome, Outcome::Recorded(WriteOutcome::Failed(_))),
            "{outcome:?}"
        );
    }

    #[test]
    fn no_state_directory_means_nothing_happens_at_all() {
        let args = Args {
            provider: Some(Provider::ClaudeCode),
            state_dir: None,
        };
        assert_eq!(
            run(&args, &payload("PermissionRequest"), 1_000),
            Outcome::Misconfigured
        );
    }

    #[test]
    fn an_older_hook_never_moves_the_phase_backwards() {
        let dir = TempDir::new("race");
        run(&dir.args(), &payload("PermissionRequest"), 2_000);
        let outcome = run(&dir.args(), &payload("PostToolUse"), 1_000);
        assert_eq!(outcome, Outcome::Recorded(WriteOutcome::Superseded));
        assert_eq!(dir.states()[0].phase, HookPhase::Blocked);
    }

    #[test]
    fn oversized_payload_fields_never_reach_the_state_directory() {
        let dir = TempDir::new("huge");
        let raw = format!(
            r#"{{"session_id":"abc","hook_event_name":"PermissionRequest","tool_name":"{}","cwd":"{}"}}"#,
            "x".repeat(5_000),
            "y".repeat(5_000)
        );
        run(&dir.args(), &raw, 1_000);
        let written = fs::read_to_string(dir.0.join("abc.json")).expect("read state");
        assert!(written.len() < 1_024, "{} bytes", written.len());
    }

    #[test]
    fn camel_case_payload_keys_are_accepted_too() {
        let dir = TempDir::new("camel");
        let raw = r#"{"sessionId":"abc","hookEventName":"Stop"}"#;
        assert_eq!(
            run(&dir.args(), raw, 1_000),
            Outcome::Recorded(WriteOutcome::Written)
        );
        assert_eq!(dir.states()[0].phase, HookPhase::Settled);
    }

    // --------------------------------------------------------- command line

    #[test]
    fn the_state_directory_can_be_given_directly_or_derived_from_the_data_directory() {
        let direct =
            parse_args(["--provider", "claude-code", "--state-dir", "/tmp/x"].map(str::to_owned));
        assert_eq!(direct.provider, Some(Provider::ClaudeCode));
        assert_eq!(direct.state_dir, Some(PathBuf::from("/tmp/x")));

        let derived = parse_args(["--data-dir", "/tmp/data"].map(str::to_owned));
        assert_eq!(
            derived.state_dir,
            Some(PathBuf::from("/tmp/data/agents/state"))
        );
    }

    #[test]
    fn an_argument_this_build_does_not_know_is_ignored_rather_than_refused() {
        // An upgrade may add a flag; the hook binary an agent's configuration
        // still points at must not start failing mid-session because of it.
        let parsed =
            parse_args(["--future-flag", "yes", "--state-dir", "/tmp/x"].map(str::to_owned));
        assert_eq!(parsed.state_dir, Some(PathBuf::from("/tmp/x")));
    }

    #[test]
    fn an_unknown_provider_name_leaves_the_default() {
        let parsed = parse_args(
            ["--provider", "something-else", "--state-dir", "/tmp/x"].map(str::to_owned),
        );
        assert_eq!(parsed.provider, None);
    }
}
