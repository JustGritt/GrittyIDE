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
//! # One event answers back (T335, ADR-020)
//!
//! `PermissionRequest` — and nothing else — may wait. It records its phase
//! first, exactly as every other event does, and only then looks for a verdict
//! Gritty may have written. The constraint above survives that in the only form
//! that matters: the wait happens **only when Gritty is alive** (a liveness
//! marker says so) and **only when the agent was about to block on a human
//! anyway**, so it replaces a wait rather than adding one. Every way of not
//! getting an answer — no marker, a marker that goes stale mid-wait, a deadline,
//! an unreadable file — ends in silence, and silence leaves the agent's own
//! prompt exactly where it was.
//!
//! The binary is a shell around [`record_and_answer`], so every decision it
//! makes is tested against a scratch directory rather than by launching a
//! process — including "Gritty was killed while we waited", which is otherwise
//! only observable by killing something and waiting several seconds.

use std::hash::{BuildHasher, Hasher, RandomState};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gritty_agents::hook::{
    AskVerb, HOOK_STATE_VERSION, HookAsk, HookPhase, HookState, MAX_HOOK_DIFF_LINES, WriteOutcome,
    phase_of_event, state_dir_from_data_dir, write,
};
use gritty_agents::probe::Provider;
use gritty_agents::verdict::{WaitOptions, WaitOutcome, await_verdict};
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
    /// How long this hook may be held waiting for a verdict (T335, ADR-020).
    ///
    /// Supplied by the installer rather than fixed here, because the value that
    /// actually matters is the *hook entry's own* `timeout` in the agent's
    /// configuration — Claude Code discards a reply that arrives after it — and
    /// the two have to be chosen together. `None` means the module default.
    pub ask_timeout: Option<Duration>,
}

impl Args {
    /// `<app-data>/agents`, the parent of the state directory.
    ///
    /// Derived rather than passed as a fourth flag so that a hook command line
    /// written by an older Gritty keeps working: the installer has always
    /// pointed `--state-dir` at `<app-data>/agents/state`, so the verdict
    /// directory and the liveness marker are already reachable from it.
    fn agents_dir(&self) -> Option<&Path> {
        self.state_dir.as_deref().and_then(Path::parent)
    }

    fn wait_options(&self) -> Option<WaitOptions> {
        let agents = self.agents_dir()?;
        Some(WaitOptions {
            verdict_dir: agents.join("verdicts"),
            marker: agents.join("live"),
            deadline: self
                .ask_timeout
                .unwrap_or(gritty_agents::verdict::DEFAULT_WAIT),
            poll: gritty_agents::verdict::POLL_INTERVAL,
        })
    }
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
            "--ask-timeout" => {
                parsed.ask_timeout = args
                    .next()
                    .and_then(|seconds| seconds.parse::<u64>().ok())
                    .filter(|seconds| *seconds > 0)
                    .map(Duration::from_secs);
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
///
/// This is the whole of what T221 did and the whole of what every event but
/// `PermissionRequest` still does. Answering is [`record_and_answer`].
pub fn run(args: &Args, payload: &str, now_ms: i64) -> Outcome {
    record(args, payload, now_ms).0
}

/// Record the phase, then — for an ask, and only for an ask — wait for the
/// user's verdict and return the reply the agent should read on stdout.
///
/// The order is the safety property (ADR-020 §2): the state file is written
/// **first**, so `BLOCKED` is earned whether or not anyone is listening, and
/// only then does the hook consider waiting. Every way of not getting an answer
/// returns `None`, and `None` means the agent prompts its own user exactly as
/// it did before Gritty existed.
///
/// `now` and `sleep` are injected for the same reason `now_ms` is: so that
/// "a killed Gritty releases the agent" is a test and not a paragraph.
pub fn record_and_answer<N, S>(
    args: &Args,
    payload: &str,
    now_ms: i64,
    now: N,
    sleep: S,
) -> (Outcome, Option<String>)
where
    N: FnMut() -> i64,
    S: FnMut(Duration),
{
    let (outcome, ask_id) = record(args, payload, now_ms);
    let (Some(ask_id), Some(options)) = (ask_id, args.wait_options()) else {
        return (outcome, None);
    };
    let reply = match await_verdict(&ask_id, &options, now, sleep) {
        WaitOutcome::Answered(verdict) => Some(verdict.to_claude_code_reply()),
        // Not running, went away, or nobody answered in time. All three are the
        // same answer to the agent — silence — and silence is what leaves its
        // own prompt standing.
        WaitOutcome::NotRunning | WaitOutcome::WentAway | WaitOutcome::TimedOut => None,
    };
    (outcome, reply)
}

/// The recording half, and the ask id if this event was one worth waiting on.
fn record(args: &Args, payload: &str, now_ms: i64) -> (Outcome, Option<String>) {
    let Some(state_dir) = args.state_dir.as_deref() else {
        return (Outcome::Misconfigured, None);
    };
    // No provider named is Claude Code: the only one this build supports, and
    // an older installed command line may not carry the flag at all.
    let provider = args.provider.unwrap_or(Provider::ClaudeCode);

    let Ok(Value::Object(payload)) = serde_json::from_str::<Value>(payload) else {
        return (Outcome::UnusablePayload, None);
    };
    let Some(session_id) = string_of(&payload, &["session_id", "sessionId"]) else {
        return (Outcome::UnusablePayload, None);
    };
    let Some(event) = string_of(&payload, &["hook_event_name", "hookEventName", "event"]) else {
        return (Outcome::UnusablePayload, None);
    };
    let Some(phase) = phase_of_event(&event) else {
        return (Outcome::UnknownEvent(event), None);
    };

    let tool = string_of(&payload, &["tool_name", "toolName"]);
    let cwd = string_of(&payload, &["cwd", "workingDirectory"]);
    // Only a blocking event describes something that has not happened yet, and
    // only something that has not happened yet is worth asking about.
    let ask = (phase == HookPhase::Blocked)
        .then(|| classify(tool.as_deref(), payload.get("tool_input"), cwd.as_deref()))
        .flatten();
    let ask_id = ask.as_ref().map(|ask| ask.ask_id.clone());

    // Fields are read, never interpreted: the tool name is a label, the working
    // directory is a display string, and neither is resolved, opened or run.
    // `write` clips all of them again before they reach the disk (§4).
    let state = HookState {
        version: HOOK_STATE_VERSION,
        session_id,
        provider,
        phase,
        since_ms: now_ms,
        event,
        tool,
        cwd,
        ask,
    };
    let outcome = write(state_dir, &state);
    // An ask whose state file did not land has nowhere to be answered from, so
    // there is nothing to wait for. Waiting anyway would hold the agent on a
    // prompt Gritty cannot draw.
    let ask_id = ask_id.filter(|_| outcome == WriteOutcome::Written);
    (Outcome::Recorded(outcome), ask_id)
}

/// Turn a tool name and its input into something a person can answer.
///
/// This is the one place in the hook that *interprets* rather than copies, and
/// it is deliberately shallow. It answers three questions — what kind of thing,
/// on what, and roughly how much — and refuses to answer anything else. Where
/// it is unsure it returns [`AskVerb::Other`] with the tool's own name, which
/// draws as "Claude Code wants to use `WebFetch`": less informative than a
/// guess, and never wrong.
///
/// `None` when there is nothing to name at all, which leaves the phase exactly
/// as T221 wrote it: still `BLOCKED`, just without a subject.
fn classify(tool: Option<&str>, input: Option<&Value>, cwd: Option<&str>) -> Option<HookAsk> {
    let tool = tool?;
    let input = input.and_then(Value::as_object);
    let field = |key: &str| {
        input
            .and_then(|input| input.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };

    let (verb, target) = match tool {
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            (AskVerb::WriteFile, field("file_path")?.to_owned())
        }
        "Bash" | "BashOutput" => {
            let command = field("command")?.to_owned();
            let verb = if is_install(&command) {
                AskVerb::InstallDependency
            } else {
                AskVerb::RunCommand
            };
            (verb, command)
        }
        other => (AskVerb::Other, other.to_owned()),
    };

    let sensitive = verb == AskVerb::WriteFile && is_sensitive(&target);
    let (diff, added, removed) = if sensitive {
        (Vec::new(), 0, 0)
    } else {
        diff_of(
            tool,
            field("content"),
            field("old_string"),
            field("new_string"),
        )
    };

    Some(HookAsk {
        ask_id: mint_ask_id(),
        verb,
        outside_root: verb == AskVerb::WriteFile && is_outside(&target, cwd),
        detail: detail_for(verb, &target),
        target,
        diff,
        added,
        removed,
        sensitive,
    })
}

/// Whether a shell command installs something.
///
/// A prefix match on a short list, because the question being answered is
/// "should this card say *install dependency* rather than *run command*" — a
/// labelling decision with no consequences if it is wrong in either direction.
/// It is not a security boundary and must never be mistaken for one; the
/// command itself is shown either way.
fn is_install(command: &str) -> bool {
    const INSTALLERS: &[&str] = &[
        "npm i",
        "npm install",
        "npm add",
        "pnpm add",
        "pnpm install",
        "yarn add",
        "yarn install",
        "bun add",
        "bun install",
        "cargo add",
        "cargo install",
        "pip install",
        "pip3 install",
        "uv add",
        "uv pip install",
        "gem install",
        "go get",
        "brew install",
        "apt install",
        "apt-get install",
    ];
    let normalised = command.trim_start().to_ascii_lowercase();
    INSTALLERS
        .iter()
        .any(|installer| normalised.starts_with(installer))
}

/// Whether a file's *content* is presumed secret.
///
/// Matched on the name alone, and deliberately over-broad: the cost of a false
/// positive is a prompt without a diff, and the cost of a false negative is a
/// secret drawn on screen and written to a file on disk (hard rule 13). The
/// path is still shown — what is withheld is only what is inside it.
fn is_sensitive(path: &str) -> bool {
    const SECRET_NAMES: &[&str] = &[
        ".env",
        ".envrc",
        ".npmrc",
        ".netrc",
        ".pgpass",
        "credentials",
        "id_rsa",
        "id_ed25519",
        ".htpasswd",
        "secrets",
    ];
    const SECRET_SUFFIXES: &[&str] = &[".pem", ".key", ".p12", ".pfx", ".keystore", ".jks"];

    let name = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    SECRET_NAMES
        .iter()
        .any(|secret| name.starts_with(secret) || name.contains(secret))
        || SECRET_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

/// Whether a write lands outside the session's own working directory.
///
/// Textual, on purpose. The hook does not canonicalise, does not stat and does
/// not follow a symlink: it is running inside somebody else's process and must
/// not touch the filesystem to answer a question that is only ever used to
/// *label* a prompt. A relative path is inside by definition — it is resolved
/// against the very directory being compared to.
fn is_outside(target: &str, cwd: Option<&str>) -> bool {
    let Some(cwd) = cwd else {
        // Without a root there is nothing to be outside of, and claiming
        // "outside its worktree" on no evidence is the sort of confident
        // invention rule 12 exists to prevent.
        return false;
    };
    if !target.starts_with('/') {
        return false;
    }
    let root = cwd.trim_end_matches('/');
    !(target == root || target.starts_with(&format!("{root}/")))
}

/// The blast-radius line: what this ask reaches beyond its target.
///
/// Only said when it is derivable from the target itself. An empty line beats
/// a plausible one.
fn detail_for(verb: AskVerb, target: &str) -> Option<String> {
    match verb {
        AskVerb::InstallDependency => Some("installs a dependency · changes a lockfile".to_owned()),
        AskVerb::RunCommand if reaches_network(target) => {
            Some("runs a command that reaches the network".to_owned())
        }
        _ => None,
    }
}

fn reaches_network(command: &str) -> bool {
    const NETWORKED: &[&str] = &[
        "curl",
        "wget",
        "ssh",
        "scp",
        "rsync",
        "nc ",
        "git push",
        "git pull",
        "git fetch",
        "git clone",
        "cargo publish",
        "npm publish",
        "gh ",
    ];
    let normalised = command.trim_start().to_ascii_lowercase();
    NETWORKED.iter().any(|tool| normalised.starts_with(tool))
}

/// A few sign-prefixed lines showing the shape of a change, and its size.
///
/// `Write` has only new content, so every line is an addition. `Edit` has both
/// halves, so the removed lines come first — the order a diff is read in. The
/// counts describe the *whole* change even though the lines shown are clipped,
/// which is the point of showing them: "+64 −11" beside three lines says both
/// what is happening and that you are not seeing all of it.
fn diff_of(
    tool: &str,
    content: Option<&str>,
    old_string: Option<&str>,
    new_string: Option<&str>,
) -> (Vec<String>, u32, u32) {
    let count = |text: Option<&str>| -> u32 {
        text.map(|text| u32::try_from(text.lines().count()).unwrap_or(u32::MAX))
            .unwrap_or(0)
    };
    let (removed_text, added_text) = match tool {
        "Write" => (None, content),
        _ => (old_string, new_string),
    };
    let mut lines = Vec::new();
    let half = MAX_HOOK_DIFF_LINES / 2;
    for line in removed_text.into_iter().flat_map(str::lines).take(half) {
        lines.push(format!("-{line}"));
    }
    for line in added_text
        .into_iter()
        .flat_map(str::lines)
        .take(MAX_HOOK_DIFF_LINES - lines.len())
    {
        lines.push(format!("+{line}"));
    }
    (lines, count(added_text), count(removed_text))
}

/// An identifier no other process on the machine can guess.
///
/// It names the verdict file, so guessing it would let another local process
/// answer a prompt on the user's behalf — which is exactly the authority
/// ADR-020 is careful to say belongs only to a person. Two independently seeded
/// [`RandomState`] hashers give the OS entropy behind them without adding a
/// dependency to a binary whose whole virtue is having almost none.
fn mint_ask_id() -> String {
    let mut first = RandomState::new().build_hasher();
    first.write_u64(std::process::id().into());
    let mut second = RandomState::new().build_hasher();
    second.write_i64(gritty_agents::now_ms());
    format!("{:016x}{:016x}", first.finish(), second.finish())
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
                ask_timeout: None,
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

    /// T221's `the_tool_input_is_never_recorded` used to live here, and it was
    /// right for as long as the state file was only ever *read*. ADR-020 makes
    /// it answerable, and an ask a person has to answer must say what it would
    /// do. What replaces the old test is not "the input is recorded" — that
    /// would be a weaker promise — but the three narrower ones below: it is
    /// recorded only for a blocking event, only bounded, and never for a target
    /// whose content is presumed secret.
    #[test]
    fn the_tool_input_is_recorded_only_for_an_event_that_is_asking() {
        for (event, expected) in [
            ("PermissionRequest", true),
            ("PreToolUse", false),
            ("PostToolUse", false),
            ("Stop", false),
        ] {
            let dir = TempDir::new(&format!("ask-{event}"));
            run(&dir.args(), &payload(event), 1_000);
            let states = dir.states();
            assert_eq!(
                states[0].ask.is_some(),
                expected,
                "{event} should{} carry an ask",
                if expected { "" } else { " not" }
            );
            let raw = fs::read_to_string(dir.0.join("abc-123.json")).expect("read state");
            assert_eq!(raw.contains("rm -rf"), expected, "{event}: {raw}");
        }
    }

    #[test]
    fn an_ask_names_the_command_and_never_the_raw_payload_shape() {
        let dir = TempDir::new("ask-shape");
        run(&dir.args(), &payload("PermissionRequest"), 1_000);
        let ask = dir.states()[0].ask.clone().expect("an ask");
        assert_eq!(ask.verb, AskVerb::RunCommand);
        assert_eq!(ask.target, "rm -rf /");
        assert!(!ask.sensitive);
        // The *payload's* own key names are the agent's business, not ours: we
        // copy values out of it, never the envelope they arrived in.
        let raw = fs::read_to_string(dir.0.join("abc-123.json")).expect("read state");
        assert!(!raw.contains("tool_input"), "{raw}");
    }

    #[test]
    fn a_secret_file_is_named_but_never_quoted() {
        // The whole point of the redaction: the user still sees *which* file is
        // being written — they have to, to judge it — and never what would go
        // into it. A prompt that leaked the secret it asked about would be
        // worse than no prompt at all (rule 13).
        for path in [
            "/home/me/app/.env",
            "/home/me/.ssh/id_rsa",
            "/home/me/certs/server.pem",
            "/home/me/.npmrc",
        ] {
            let dir = TempDir::new("secret");
            let payload = format!(
                r#"{{"session_id":"abc-123","hook_event_name":"PermissionRequest","cwd":"/home/me/app","tool_name":"Write","tool_input":{{"file_path":"{path}","content":"AWS_SECRET_ACCESS_KEY=hunter2"}}}}"#
            );
            run(&dir.args(), &payload, 1_000);
            let ask = dir.states()[0].ask.clone().expect("an ask");
            assert!(ask.sensitive, "{path}");
            assert!(ask.diff.is_empty(), "{path}");
            assert_eq!(ask.target, path);
            let raw = fs::read_to_string(dir.0.join("abc-123.json")).expect("read state");
            assert!(!raw.contains("hunter2"), "{path}: {raw}");
        }
    }

    #[test]
    fn an_ordinary_write_carries_a_bounded_diff() {
        let dir = TempDir::new("write-diff");
        let content = (0..40)
            .map(|n| format!("line {n} {}", "x".repeat(400)))
            .collect::<Vec<_>>()
            .join("\\n");
        let payload = format!(
            r#"{{"session_id":"abc-123","hook_event_name":"PermissionRequest","cwd":"/home/me/app","tool_name":"Write","tool_input":{{"file_path":"/home/me/app/src/main.rs","content":"{content}"}}}}"#
        );
        run(&dir.args(), &payload, 1_000);
        let ask = dir.states()[0].ask.clone().expect("an ask");
        assert_eq!(ask.verb, AskVerb::WriteFile);
        assert!(!ask.outside_root);
        assert!(
            ask.diff.len() <= MAX_HOOK_DIFF_LINES,
            "{:?}",
            ask.diff.len()
        );
        assert!(ask.diff.iter().all(|line| line.starts_with('+')));
        assert!(
            ask.diff.iter().all(|line| line.chars().count() <= 202),
            "every line is clipped: {:?}",
            ask.diff.iter().map(|l| l.chars().count()).max()
        );
        // The counts describe the whole change even though the lines do not.
        assert_eq!(ask.added, 40);
    }

    #[test]
    fn a_write_outside_the_session_root_says_so() {
        for (target, cwd, expected) in [
            ("/home/me/app/src/main.rs", "/home/me/app", false),
            ("/home/me/other/src/main.rs", "/home/me/app", true),
            // A prefix that is not a path boundary is not containment.
            ("/home/me/app-two/x.rs", "/home/me/app", true),
            // Relative paths resolve against the very root being compared to.
            ("src/main.rs", "/home/me/app", false),
        ] {
            let dir = TempDir::new("outside");
            let payload = format!(
                r#"{{"session_id":"abc-123","hook_event_name":"PermissionRequest","cwd":"{cwd}","tool_name":"Write","tool_input":{{"file_path":"{target}","content":"x"}}}}"#
            );
            run(&dir.args(), &payload, 1_000);
            let ask = dir.states()[0].ask.clone().expect("an ask");
            assert_eq!(ask.outside_root, expected, "{target} in {cwd}");
        }
    }

    #[test]
    fn an_install_is_told_apart_from_an_ordinary_command() {
        for (command, expected) in [
            ("pnpm add @scene7/protocol", AskVerb::InstallDependency),
            ("cargo add serde", AskVerb::InstallDependency),
            ("  npm install", AskVerb::InstallDependency),
            ("cargo test -p gritty-core", AskVerb::RunCommand),
            ("cargo publish --dry-run", AskVerb::RunCommand),
        ] {
            let dir = TempDir::new("install");
            let payload = format!(
                r#"{{"session_id":"abc-123","hook_event_name":"PermissionRequest","cwd":"/home/me/app","tool_name":"Bash","tool_input":{{"command":"{command}"}}}}"#
            );
            run(&dir.args(), &payload, 1_000);
            assert_eq!(
                dir.states()[0].ask.clone().expect("an ask").verb,
                expected,
                "{command}"
            );
        }
    }

    #[test]
    fn a_tool_the_classifier_does_not_know_shrugs_rather_than_guesses() {
        let dir = TempDir::new("unknown-tool");
        let payload = r#"{"session_id":"abc-123","hook_event_name":"PermissionRequest","cwd":"/home/me/app","tool_name":"WebFetch","tool_input":{"url":"https://example.com"}}"#;
        run(&dir.args(), payload, 1_000);
        let ask = dir.states()[0].ask.clone().expect("an ask");
        assert_eq!(ask.verb, AskVerb::Other);
        assert_eq!(ask.target, "WebFetch");
        assert!(ask.diff.is_empty());
    }

    #[test]
    fn two_asks_never_share_an_id() {
        // The id names the verdict file. Two asks sharing one would let an
        // answer to the first silently answer the second.
        let dir = TempDir::new("ids");
        let mut seen = std::collections::HashSet::new();
        for n in 0..16 {
            run(&dir.args(), &payload("PermissionRequest"), 1_000 + n);
            let ask = dir.states()[0].ask.clone().expect("an ask");
            assert!(
                seen.insert(ask.ask_id.clone()),
                "repeated id {}",
                ask.ask_id
            );
        }
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
            ..Args::default()
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
            ..Args::default()
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
