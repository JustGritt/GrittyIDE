//! Hook evidence: the third source, and the only one an agent has to
//! cooperate with (T221, ADR-018).
//!
//! `ps` and the transcript are read behind the agent's back and are complete
//! on their own. Neither can tell a pending permission prompt from a finished
//! turn — both are a live process with nothing outstanding — so `BLOCKED` is
//! earned instead by *asking*: the agent's own hook system runs
//! `gritty-agent-hook` when it blocks, and that binary writes one small file
//! here.
//!
//! This module owns both halves of that file, deliberately in one place:
//!
//! - [`write`] is what the hook binary calls. It is the only writer.
//! - [`scan`] is what the core's fast loop calls. It is the only reader.
//!
//! One definition of the format means the writer and the reader cannot drift,
//! and the truncation limits below are applied on both sides — by the hook so
//! an oversized field never reaches the state directory, and again by the
//! reader because what comes back off disk is untrusted input a second time
//! (rule 13).
//!
//! **A file rather than a socket** because Gritty is usually *not running*
//! when a hook fires (ADR-018 §6). **Overwritten rather than appended**
//! because only the current phase is ever asked for, which removes rotation,
//! truncation-tolerant parsing and an age policy from the design. **Atomic**
//! because the reader takes no lock: a rename either happened or did not, so a
//! concurrent read never sees half a file.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use serde_json::Value;

use crate::probe::Provider;
use crate::transcript::MAX_TOOL_DETAIL;

/// Format version of the state file.
///
/// A file carrying a *newer* version than this build understands is skipped
/// rather than guessed at: forward compatibility over a hard failure, and over
/// a confident misreading of fields that may have changed meaning.
///
/// **2** added [`HookState::ask`] (T335, ADR-020). An older file still parses:
/// every field the version added is optional, and a state file with no ask is
/// exactly what version 1 always wrote. The bump exists so a *newer* hook left
/// behind by an upgrade is skipped rather than half-read, not to strand an
/// older one.
pub const HOOK_STATE_VERSION: u32 = 2;

/// Longest session id kept. A UUID is 36 characters; anything appreciably
/// longer is not an id this build can join on anyway.
pub const MAX_HOOK_SESSION_ID: usize = 128;
/// Longest hook event name kept.
pub const MAX_HOOK_EVENT: usize = 64;
/// Longest tool name kept — `Bash`, `Edit`, an MCP tool's qualified name.
pub const MAX_HOOK_TOOL: usize = MAX_TOOL_DETAIL;
/// Longest working directory kept. Display only; never resolved as a path.
pub const MAX_HOOK_CWD: usize = 240;

/// Longest ask id kept. Minted by the hook, and long enough that a verdict
/// file cannot be guessed by another process on the machine.
pub const MAX_HOOK_ASK_ID: usize = 64;
/// Longest ask target kept — a path, or the command a `Bash` ask is about.
pub const MAX_HOOK_TARGET: usize = 400;
/// Longest blast-radius line kept: "network access · reaches crates.io".
pub const MAX_HOOK_DETAIL: usize = 240;
/// Longest single diff line kept. A permission prompt shows the shape of a
/// change, never the whole of it.
pub const MAX_HOOK_DIFF_LINE: usize = 200;
/// Most diff lines kept. Three either side of a hunk header is what the ask
/// needs to be judgeable; more is a diff view's job (T030), not a prompt's.
pub const MAX_HOOK_DIFF_LINES: usize = 8;

/// What the *agent* said it was doing. Not an activity.
///
/// The hook names phases and the core decides what they mean for a roster row
/// (ADR-018 §6): `BLOCKED` here is evidence that becomes
/// [`AgentActivity::Blocked`] only when the process table agrees the session is
/// alive, and never on its own.
///
/// [`AgentActivity::Blocked`]: https://docs.rs/gritty-core
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPhase {
    /// Stopped at a permission prompt. The one value this whole mechanism
    /// exists for.
    Blocked,
    /// Running: a turn started, a tool ran, a prompt was submitted.
    Active,
    /// A turn ended. The session is alive and it is the user's move.
    Settled,
    /// The session itself ended.
    Ended,
}

impl HookPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Blocked => "BLOCKED",
            Self::Active => "ACTIVE",
            Self::Settled => "SETTLED",
            Self::Ended => "ENDED",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "BLOCKED" => Some(Self::Blocked),
            "ACTIVE" => Some(Self::Active),
            "SETTLED" => Some(Self::Settled),
            "ENDED" => Some(Self::Ended),
            _ => None,
        }
    }
}

/// The phase a hook event reports, with no interpretation whatsoever.
///
/// One event name in, one phase out. Everything this build has not been told
/// about returns `None` and **writes nothing**: a hook that fired for a reason
/// Gritty does not understand must not overwrite a phase that is still true.
pub fn phase_of_event(event: &str) -> Option<HookPhase> {
    match event {
        "PermissionRequest" => Some(HookPhase::Blocked),
        // A denial is the end of the block, not a new one: the agent carries
        // on (usually by telling the user it could not do the thing).
        "PermissionDenied" | "PostToolUse" | "PostToolUseFailure" | "PreToolUse"
        | "UserPromptSubmit" | "SessionStart" => Some(HookPhase::Active),
        "Stop" | "StopFailure" => Some(HookPhase::Settled),
        "SessionEnd" => Some(HookPhase::Ended),
        _ => None,
    }
}

/// What kind of thing an ask is about.
///
/// Four values, because four is what a bounded classifier can defend. The
/// point is not taxonomy: it is that "write file" and "run command" carry
/// different consequences, and a person answering in a hurry should not have
/// to read a path to tell which one they are looking at. Anything the
/// classifier is unsure of is [`AskVerb::Other`] and shows the tool's own
/// name — an honest shrug rather than a confident miscategorisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskVerb {
    WriteFile,
    RunCommand,
    InstallDependency,
    Other,
}

impl AskVerb {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WriteFile => "WRITE_FILE",
            Self::RunCommand => "RUN_COMMAND",
            Self::InstallDependency => "INSTALL_DEPENDENCY",
            Self::Other => "OTHER",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "WRITE_FILE" => Some(Self::WriteFile),
            "RUN_COMMAND" => Some(Self::RunCommand),
            "INSTALL_DEPENDENCY" => Some(Self::InstallDependency),
            "OTHER" => Some(Self::Other),
            _ => None,
        }
    }
}

/// The subject of a permission prompt: what, where, and how much.
///
/// T221 deliberately recorded none of this — its own test was called
/// `the_tool_input_is_never_recorded` — because a phase was all a roster row
/// needed and a tool's input is the most sensitive thing a hook ever sees.
/// ADR-020 changes the requirement, not the caution: an ask a person is asked
/// to *answer* has to say what it would do, so the input is now recorded
/// **bounded, and redacted where the target's content is presumed secret**.
///
/// Every field is clipped by the hook before it reaches the disk and clipped
/// again by [`scan`] on the way back, exactly as the rest of [`HookState`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAsk {
    /// Names the verdict file, so it must be unguessable by another process on
    /// the machine and unusable as a path (see [`sanitize_id`]).
    pub ask_id: String,
    pub verb: AskVerb,
    /// The path being written, or the command being run. Display only: like
    /// [`HookState::cwd`], it is never resolved, opened or executed.
    pub target: String,
    /// The blast radius in one line — "network access · reaches crates.io",
    /// "writes package.json + pnpm-lock.yaml". Absent when nothing beyond the
    /// target is worth saying.
    pub detail: Option<String>,
    /// A few sign-prefixed lines (`+`, `-`, or a leading space) showing the
    /// shape of the change. Empty when there is nothing to show, and empty
    /// *deliberately* when [`HookAsk::sensitive`] is set.
    pub diff: Vec<String>,
    pub added: u32,
    pub removed: u32,
    /// Whether the target lies outside the session's own working directory.
    ///
    /// Computed **in the hook**, which is the only place that knows both the
    /// session's real root and the real target. Neither ever crosses the
    /// protocol (ADR-010); this boolean is what 12a draws as "outside its
    /// worktree", and it costs no path.
    pub outside_root: bool,
    /// The target's content is presumed secret, so no diff was captured. Said
    /// out loud rather than left as an empty diff, because "we did not look"
    /// and "there was nothing to see" are different answers.
    pub sensitive: bool,
}

/// One session's current phase, as the agent reported it.
///
/// Every string in here has been through [`bounded`] twice: once in the hook
/// before it reached the disk, once in [`scan`] after it came back off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookState {
    pub version: u32,
    /// The agent's own session id — the join key, and the only exact one there
    /// is (the roster's other join is the working directory).
    pub session_id: String,
    pub provider: Provider,
    pub phase: HookPhase,
    /// When the agent reported this phase, in milliseconds since the epoch.
    ///
    /// The hook's own clock at the moment it wrote, which is what both callers
    /// actually want: the reader compares it to now to decide whether the
    /// phase is still worth believing, and a racing pair of hooks uses it to
    /// decide which write wins. It is compared, never trusted as a duration —
    /// a clock that moved backwards yields an age of zero, not a negative one.
    pub since_ms: i64,
    /// The hook event that produced the phase, kept so a `BLOCKED` row is
    /// attributable to something the agent actually sent.
    pub event: String,
    /// The tool the agent is asking about, when the event named one. This is
    /// what turns "something needs you" into "`Bash` needs you".
    pub tool: Option<String>,
    /// The session's working directory, for display and for nothing else.
    /// Never resolved, never opened, never joined on.
    pub cwd: Option<String>,
    /// What the session is asking permission to do, when the event was one
    /// that asks. Only ever set alongside [`HookPhase::Blocked`]: an ask
    /// attached to any other phase would be describing something that already
    /// happened (T335, ADR-020).
    pub ask: Option<HookAsk>,
}

impl HookState {
    /// How long ago the agent reported this phase, in milliseconds.
    ///
    /// Never negative: a `since` in the future means the two clocks disagree
    /// (or the file was written on a machine whose time then moved back), and
    /// "zero milliseconds ago" is the honest reading of a stamp we cannot
    /// place in the past.
    pub fn age_ms(&self, now_ms: i64) -> i64 {
        now_ms.saturating_sub(self.since_ms).max(0)
    }

    /// Whether this phase is too old to still describe the session.
    pub fn is_stale(&self, now_ms: i64, after: Duration) -> bool {
        self.age_ms(now_ms) > i64::try_from(after.as_millis()).unwrap_or(i64::MAX)
    }

    /// The same state with every field clipped to its limit.
    ///
    /// Applied by [`write`] rather than left to the caller, so an oversized
    /// field cannot reach the state directory however the hook binary was
    /// invoked (§4). The reader applies the same bounds again, because a file
    /// on disk is untrusted input a second time.
    fn clipped(&self) -> Self {
        Self {
            version: self.version,
            session_id: bounded(&self.session_id, MAX_HOOK_SESSION_ID),
            provider: self.provider,
            phase: self.phase,
            since_ms: self.since_ms,
            event: bounded(&self.event, MAX_HOOK_EVENT),
            tool: self
                .tool
                .as_deref()
                .map(|tool| bounded(tool, MAX_HOOK_TOOL)),
            cwd: self.cwd.as_deref().map(|cwd| bounded(cwd, MAX_HOOK_CWD)),
            // An ask only ever describes a session that is *stopped*. Dropping
            // it on any other phase is not tidying: a `PostToolUse` carrying an
            // ask would leave a prompt on screen for something already done.
            ask: self
                .ask
                .as_ref()
                .filter(|_| self.phase == HookPhase::Blocked)
                .map(HookAsk::clipped),
        }
    }

    fn to_json(&self) -> String {
        let mut object = serde_json::Map::new();
        object.insert("version".to_owned(), Value::from(self.version));
        object.insert("sessionId".to_owned(), Value::from(self.session_id.clone()));
        object.insert("provider".to_owned(), Value::from(self.provider.as_str()));
        object.insert("phase".to_owned(), Value::from(self.phase.as_str()));
        object.insert("since".to_owned(), Value::from(self.since_ms));
        object.insert("event".to_owned(), Value::from(self.event.clone()));
        if let Some(tool) = &self.tool {
            object.insert("tool".to_owned(), Value::from(tool.clone()));
        }
        if let Some(cwd) = &self.cwd {
            object.insert("cwd".to_owned(), Value::from(cwd.clone()));
        }
        if let Some(ask) = &self.ask {
            object.insert("ask".to_owned(), ask.to_json());
        }
        Value::Object(object).to_string()
    }

    /// The reverse, applying every bound a second time.
    ///
    /// Returns `None` for anything this build cannot read with confidence: a
    /// non-object, a missing id, an unknown phase, an unknown provider, or a
    /// `version` from a newer build. None of those is an error — one unreadable
    /// file costs that session's hook evidence and nothing else.
    fn from_json(raw: &str) -> Option<Self> {
        let Ok(Value::Object(object)) = serde_json::from_str::<Value>(raw) else {
            return None;
        };
        let version = u32::try_from(object.get("version")?.as_u64()?).ok()?;
        if version > HOOK_STATE_VERSION {
            return None;
        }
        let session_id = bounded(object.get("sessionId")?.as_str()?, MAX_HOOK_SESSION_ID);
        if session_id.is_empty() {
            return None;
        }
        let provider = match object.get("provider")?.as_str()? {
            "CLAUDE_CODE" => Provider::ClaudeCode,
            _ => return None,
        };
        Some(Self {
            version,
            session_id,
            provider,
            phase: HookPhase::parse(object.get("phase")?.as_str()?)?,
            since_ms: object.get("since")?.as_i64()?,
            event: bounded(object.get("event")?.as_str().unwrap_or(""), MAX_HOOK_EVENT),
            tool: object
                .get("tool")
                .and_then(Value::as_str)
                .map(|tool| bounded(tool, MAX_HOOK_TOOL)),
            cwd: object
                .get("cwd")
                .and_then(Value::as_str)
                .map(|cwd| bounded(cwd, MAX_HOOK_CWD)),
            // Absent in every version-1 file, and absent in a version-2 file
            // for every phase but `BLOCKED`. Unreadable is the same as absent:
            // losing an ask costs a prompt its detail, never the phase that
            // says a session is waiting at all.
            ask: object.get("ask").and_then(HookAsk::from_json),
        })
    }
}

impl HookAsk {
    fn clipped(&self) -> Self {
        Self {
            ask_id: bounded(&self.ask_id, MAX_HOOK_ASK_ID),
            verb: self.verb,
            target: bounded(&self.target, MAX_HOOK_TARGET),
            detail: self
                .detail
                .as_deref()
                .map(|detail| bounded(detail, MAX_HOOK_DETAIL))
                .filter(|detail| !detail.is_empty()),
            // Sensitive targets carry no diff at all — the bound is not what
            // protects them, the refusal to look is.
            diff: if self.sensitive {
                Vec::new()
            } else {
                self.diff
                    .iter()
                    .take(MAX_HOOK_DIFF_LINES)
                    .map(|line| clipped_diff_line(line))
                    .collect()
            },
            added: self.added,
            removed: self.removed,
            outside_root: self.outside_root,
            sensitive: self.sensitive,
        }
    }

    fn to_json(&self) -> Value {
        let mut object = serde_json::Map::new();
        object.insert("id".to_owned(), Value::from(self.ask_id.clone()));
        object.insert("verb".to_owned(), Value::from(self.verb.as_str()));
        object.insert("target".to_owned(), Value::from(self.target.clone()));
        if let Some(detail) = &self.detail {
            object.insert("detail".to_owned(), Value::from(detail.clone()));
        }
        if !self.diff.is_empty() {
            object.insert("diff".to_owned(), Value::from(self.diff.clone()));
        }
        if self.added > 0 || self.removed > 0 {
            object.insert("added".to_owned(), Value::from(self.added));
            object.insert("removed".to_owned(), Value::from(self.removed));
        }
        if self.outside_root {
            object.insert("outsideRoot".to_owned(), Value::from(true));
        }
        if self.sensitive {
            object.insert("sensitive".to_owned(), Value::from(true));
        }
        Value::Object(object)
    }

    /// An ask that cannot name itself is no ask at all: without an id there is
    /// nothing to write a verdict against, so the whole value is dropped
    /// rather than half-kept.
    fn from_json(raw: &Value) -> Option<Self> {
        let object = raw.as_object()?;
        let ask_id = bounded(object.get("id")?.as_str()?, MAX_HOOK_ASK_ID);
        if sanitize_id(&ask_id).as_deref() != Some(ask_id.as_str()) {
            return None;
        }
        let sensitive = object
            .get("sensitive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Some(Self {
            ask_id,
            verb: AskVerb::parse(object.get("verb")?.as_str()?)?,
            target: bounded(object.get("target")?.as_str()?, MAX_HOOK_TARGET),
            detail: object
                .get("detail")
                .and_then(Value::as_str)
                .map(|detail| bounded(detail, MAX_HOOK_DETAIL))
                .filter(|detail| !detail.is_empty()),
            diff: if sensitive {
                Vec::new()
            } else {
                object
                    .get("diff")
                    .and_then(Value::as_array)
                    .map(|lines| {
                        lines
                            .iter()
                            .filter_map(Value::as_str)
                            .take(MAX_HOOK_DIFF_LINES)
                            .map(clipped_diff_line)
                            .collect()
                    })
                    .unwrap_or_default()
            },
            added: object
                .get("added")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0),
            removed: object
                .get("removed")
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0),
            outside_root: object
                .get("outsideRoot")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            sensitive,
        })
    }
}

/// Clips a diff line while keeping the sign that gives it meaning.
///
/// [`bounded`] trims, which would eat the leading space that marks a context
/// line and turn it into something the renderer draws as an addition. So the
/// sign is taken off, the text is bounded, and the sign goes back on.
fn clipped_diff_line(line: &str) -> String {
    // A line that arrived without a sign is context, and saying so is safer
    // than letting the renderer guess from the first character.
    let (sign, rest) = match line.split_at_checked(1) {
        Some((sign @ ("+" | "-" | " "), rest)) => (sign, rest),
        _ => (" ", line),
    };
    format!("{sign}{}", bounded(rest, MAX_HOOK_DIFF_LINE))
}

/// The directory the state files live in, under Gritty's own app-data
/// directory — never `/tmp`, never anywhere world-writable.
///
/// One function, called by the core when it starts the reader and by the hook
/// binary when the installer handed it a data directory, so the two cannot
/// disagree about where the files are.
pub fn state_dir_from_data_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("agents").join("state")
}

/// Where one session's file lives.
///
/// The session id is a file *name*, never a path: an id carrying separators or
/// `..` would otherwise choose where the hook writes, and the hook's input is
/// a payload from a process Gritty does not control (rule 13).
pub fn state_path(state_dir: &Path, session_id: &str) -> Option<PathBuf> {
    let name = sanitize_id(session_id)?;
    Some(state_dir.join(format!("{name}.json")))
}

/// Keeps the characters an agent session id is actually made of and refuses
/// everything else, so nothing in a payload can steer the write.
pub(crate) fn sanitize_id(session_id: &str) -> Option<String> {
    let kept: String = session_id
        .chars()
        .take(MAX_HOOK_SESSION_ID)
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    (!kept.is_empty()).then_some(kept)
}

/// What went wrong writing a state file. The hook binary maps every one of
/// these onto "exit 0 and say nothing to the agent" (ADR-018 §5); it exists so
/// tests can assert *which* refusal happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Written.
    Written,
    /// A newer `since` is already on disk, so this write would move the phase
    /// backwards. Two hooks racing cannot undo each other (§7).
    Superseded,
    /// The session id had nothing usable in it.
    UnusableSessionId,
    /// The directory or the file could not be written. The roster degrades to
    /// what T220 shipped, which is the whole point of the integration being
    /// optional.
    Failed(String),
}

/// Record one session's phase.
///
/// Overwrites, never appends. Atomic, so the reader needs no lock. Refuses to
/// move a phase backwards in time, so two hooks that fire together cannot
/// leave the older one's answer on disk.
pub fn write(state_dir: &Path, state: &HookState) -> WriteOutcome {
    let state = state.clipped();
    let Some(path) = state_path(state_dir, &state.session_id) else {
        return WriteOutcome::UnusableSessionId;
    };
    // Read-before-write is not a lock and does not pretend to be one: it is
    // the cheap half of the ordering guarantee, and the atomic rename is the
    // other half. The worst case is that two simultaneous hooks both pass this
    // check and the later rename wins, which is the outcome we want anyway.
    if let Some(existing) = read_one(&path, DEFAULT_MAX_BYTES)
        && existing.since_ms > state.since_ms
    {
        return WriteOutcome::Superseded;
    }
    if let Err(error) = ensure_dir(state_dir) {
        return WriteOutcome::Failed(error);
    }
    match gritty_workspace::write_file_atomic(
        &path,
        state.to_json().as_bytes(),
        gritty_workspace::AtomicWriteFault::None,
    ) {
        Ok(_) => {
            restrict(&path);
            WriteOutcome::Written
        }
        Err(error) => WriteOutcome::Failed(error.to_string()),
    }
}

/// Creates the state directory, owner-only.
///
/// The directory carries the permission rather than each file, because a file
/// created by an atomic rename is only chmod-able *after* it lands: a 0700
/// directory closes that window for everything inside it.
fn ensure_dir(state_dir: &Path) -> Result<(), String> {
    if state_dir.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(state_dir).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Owner-only on the file too, once it exists. Best effort: a filesystem that
/// refuses the change still gets the content, and the 0700 directory above is
/// what actually keeps it private.
fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Bounds on a scan of the state directory.
///
/// Exactly the shape [`crate::transcript::ScanOptions`] has, for exactly the
/// same reason: this runs on the fast loop (~400 ms), so its cost must be flat
/// no matter how much the directory has accumulated.
#[derive(Debug, Clone)]
pub struct HookScanOptions {
    /// `<app-data>/agents/state`.
    pub dir: PathBuf,
    /// Files older than this are ignored *and swept*. Long enough to cover a
    /// working day's session, short enough that the directory does not become
    /// a history nobody asked for.
    pub max_age: Duration,
    /// Files parsed per scan, newest first.
    pub max_files: usize,
    /// A state file is a few hundred bytes. Anything larger is not one.
    pub max_bytes: u64,
}

pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(6 * 60 * 60);
pub const DEFAULT_MAX_FILES: usize = 64;
pub const DEFAULT_MAX_BYTES: u64 = 8 * 1024;

impl HookScanOptions {
    pub fn rooted_at(dir: PathBuf) -> Self {
        Self {
            dir,
            max_age: DEFAULT_MAX_AGE,
            max_files: DEFAULT_MAX_FILES,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

/// Every state file worth believing, newest first, sweeping the rest.
///
/// A missing directory is the normal case — it means nothing is installed —
/// and costs one failed `read_dir`. That is what keeps the roster identical to
/// T220's for a user who never opted in.
///
/// Age is taken from the file's modification time rather than from its own
/// `since`, so a file cannot keep itself alive by claiming a future timestamp.
pub fn scan(options: &HookScanOptions, now_ms: i64) -> Vec<HookState> {
    let cutoff_ms =
        now_ms.saturating_sub(i64::try_from(options.max_age.as_millis()).unwrap_or(i64::MAX));
    let Ok(entries) = fs::read_dir(&options.dir) else {
        return Vec::new();
    };

    let mut fresh: Vec<(i64, PathBuf)> = Vec::new();
    let mut swept = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > options.max_bytes {
            continue;
        }
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .and_then(|since| i64::try_from(since.as_millis()).ok())
            .unwrap_or_default();
        if modified_ms < cutoff_ms {
            // A session that ended hours ago is not evidence about anything,
            // and leaving the file would make the directory grow forever.
            if fs::remove_file(&path).is_ok() {
                swept += 1;
            }
            continue;
        }
        fresh.push((modified_ms, path));
    }

    if swept > 0 {
        tracing::debug!(swept, "stale agent hook state files removed");
    }
    fresh.sort_by_key(|(modified_ms, _)| std::cmp::Reverse(*modified_ms));
    fresh.truncate(options.max_files);
    fresh
        .into_iter()
        .filter_map(|(_, path)| read_one(&path, options.max_bytes))
        .collect()
}

/// One file, or `None` for anything unreadable, oversized, non-UTF-8,
/// malformed, or written by a newer build.
fn read_one(path: &Path, max_bytes: u64) -> Option<HookState> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return None;
    }
    // Bytes, then a checked decode: a truncated multi-byte sequence is a
    // skipped file, never a panic and never a lossy guess at what it said.
    let bytes = fs::read(path).ok()?;
    let raw = std::str::from_utf8(&bytes).ok()?;
    HookState::from_json(raw)
}

/// Clip to `limit` characters, marking that something was removed. Character
/// counts, not bytes, so a multi-byte string is never cut mid-glyph.
pub fn bounded(text: &str, limit: usize) -> String {
    let collapsed: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let collapsed = collapsed.trim();
    if collapsed.chars().count() <= limit {
        return collapsed.to_owned();
    }
    let kept: String = collapsed.chars().take(limit.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch state directory that cleans itself up. Nothing in this file's
    /// tests ever touches a real app-data directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gritty-hook-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn state(session_id: &str, phase: HookPhase, since_ms: i64) -> HookState {
        HookState {
            version: HOOK_STATE_VERSION,
            session_id: session_id.to_owned(),
            provider: Provider::ClaudeCode,
            phase,
            since_ms,
            event: "PermissionRequest".to_owned(),
            tool: Some("Bash".to_owned()),
            cwd: Some("/Users/someone/code/gritty".to_owned()),
            ask: None,
        }
    }

    // ------------------------------------------------------------- the map

    #[test]
    fn every_event_maps_to_the_phase_it_reports() {
        assert_eq!(
            phase_of_event("PermissionRequest"),
            Some(HookPhase::Blocked)
        );
        for active in [
            "PermissionDenied",
            "PostToolUse",
            "PostToolUseFailure",
            "PreToolUse",
            "UserPromptSubmit",
            "SessionStart",
        ] {
            assert_eq!(phase_of_event(active), Some(HookPhase::Active), "{active}");
        }
        assert_eq!(phase_of_event("Stop"), Some(HookPhase::Settled));
        assert_eq!(phase_of_event("StopFailure"), Some(HookPhase::Settled));
        assert_eq!(phase_of_event("SessionEnd"), Some(HookPhase::Ended));
    }

    #[test]
    fn an_unrecognised_event_names_no_phase_at_all() {
        // Which is what makes "the file is not written" the hook's answer: a
        // phase Gritty does not understand must not overwrite one it does.
        assert_eq!(phase_of_event("Notification"), None);
        assert_eq!(phase_of_event("PreCompact"), None);
        assert_eq!(phase_of_event("SubagentStop"), None);
        assert_eq!(phase_of_event(""), None);
    }

    // ------------------------------------------------------ write and read

    #[test]
    fn a_written_state_reads_back_identically() {
        let dir = TempDir::new("roundtrip");
        let written = state("abc-123", HookPhase::Blocked, 1_000);
        assert_eq!(write(&dir.0, &written), WriteOutcome::Written);

        let found = scan(&HookScanOptions::rooted_at(dir.0.clone()), 1_000);
        assert_eq!(found, vec![written]);
    }

    #[test]
    fn the_file_is_overwritten_rather_than_grown() {
        let dir = TempDir::new("overwrite");
        write(&dir.0, &state("abc", HookPhase::Blocked, 1_000));
        write(&dir.0, &state("abc", HookPhase::Active, 2_000));

        let found = scan(&HookScanOptions::rooted_at(dir.0.clone()), 2_000);
        assert_eq!(found.len(), 1, "one session, one file, one phase");
        assert_eq!(found[0].phase, HookPhase::Active);
        assert_eq!(
            fs::read_dir(&dir.0).expect("read dir").count(),
            1,
            "nothing accumulates"
        );
    }

    #[test]
    fn an_older_since_never_moves_the_phase_backwards() {
        let dir = TempDir::new("race");
        write(&dir.0, &state("abc", HookPhase::Blocked, 2_000));
        assert_eq!(
            write(&dir.0, &state("abc", HookPhase::Active, 1_000)),
            WriteOutcome::Superseded,
            "the losing half of a race leaves the winner alone (§7)"
        );

        let found = scan(&HookScanOptions::rooted_at(dir.0.clone()), 2_000);
        assert_eq!(found[0].phase, HookPhase::Blocked);
    }

    #[test]
    fn oversized_fields_never_reach_the_state_directory() {
        let dir = TempDir::new("bounded");
        let mut huge = state("abc", HookPhase::Blocked, 1_000);
        huge.tool = Some("x".repeat(2_000));
        huge.cwd = Some("y".repeat(2_000));
        huge.event = "z".repeat(2_000);
        assert_eq!(write(&dir.0, &huge), WriteOutcome::Written);

        // On disk, not merely on the way back: the file itself is small (§4).
        let raw = fs::read_to_string(dir.0.join("abc.json")).expect("read state");
        assert!(raw.len() < 1_024, "{} bytes", raw.len());

        let found = scan(&HookScanOptions::rooted_at(dir.0.clone()), 1_000);
        assert_eq!(
            found[0].tool.as_ref().map(|t| t.chars().count()),
            Some(MAX_HOOK_TOOL)
        );
        assert_eq!(
            found[0].cwd.as_ref().map(|c| c.chars().count()),
            Some(MAX_HOOK_CWD)
        );
        assert_eq!(found[0].event.chars().count(), MAX_HOOK_EVENT);
    }

    #[test]
    fn a_hand_written_oversized_state_file_is_bounded_on_the_way_back() {
        // Nothing stops a user, or another tool, putting a file here. What
        // comes off disk is untrusted a second time (rule 13).
        let dir = TempDir::new("hand-written");
        let raw = format!(
            r#"{{"version":1,"sessionId":"a","provider":"CLAUDE_CODE","phase":"BLOCKED","since":1,"event":"E","tool":"{}"}}"#,
            "x".repeat(1_000)
        );
        fs::write(dir.0.join("a.json"), raw).expect("write");

        let found = scan(&HookScanOptions::rooted_at(dir.0.clone()), 1);
        assert_eq!(
            found[0].tool.as_ref().map(|t| t.chars().count()),
            Some(MAX_HOOK_TOOL)
        );
    }

    #[test]
    fn a_session_id_cannot_choose_where_the_hook_writes() {
        let dir = TempDir::new("traversal");
        let escaping = state("../../../etc/passwd", HookPhase::Blocked, 1_000);
        write(&dir.0, &escaping);

        assert!(
            dir.0.join("etcpasswd.json").is_file(),
            "the separators and dots are dropped, not followed"
        );
        assert_eq!(
            write(&dir.0, &state("///", HookPhase::Blocked, 1_000)),
            WriteOutcome::UnusableSessionId,
            "an id with nothing usable in it writes nothing at all"
        );
    }

    #[test]
    fn a_state_directory_that_cannot_be_written_is_not_fatal() {
        // A path whose parent is a *file*: the directory can never be created.
        let dir = TempDir::new("unwritable");
        let blocker = dir.0.join("a-file");
        fs::write(&blocker, b"not a directory").expect("write blocker");
        let outcome = write(&blocker.join("state"), &state("abc", HookPhase::Blocked, 1));
        assert!(
            matches!(outcome, WriteOutcome::Failed(_)),
            "reported, never panicked: {outcome:?}"
        );
    }

    // ----------------------------------------------------- what is skipped

    #[test]
    fn malformed_truncated_and_non_utf8_files_are_skipped_not_fatal() {
        let dir = TempDir::new("malformed");
        fs::write(
            dir.0.join("good.json"),
            state("good", HookPhase::Blocked, 5).to_json(),
        )
        .expect("good");
        fs::write(dir.0.join("garbage.json"), b"{not json").expect("garbage");
        fs::write(
            dir.0.join("truncated.json"),
            br#"{"version":1,"sessionId":"#,
        )
        .expect("trunc");
        fs::write(dir.0.join("binary.json"), [0xff, 0xfe, 0xfd]).expect("binary");
        fs::write(dir.0.join("array.json"), b"[]").expect("array");
        fs::write(dir.0.join("ignored.txt"), b"{}").expect("txt");

        let found = scan(&HookScanOptions::rooted_at(dir.0.clone()), 5);
        assert_eq!(
            found.len(),
            1,
            "one bad file costs one session, not the scan"
        );
        assert_eq!(found[0].session_id, "good");
    }

    #[test]
    fn a_file_from_a_newer_build_is_skipped_rather_than_guessed_at() {
        let dir = TempDir::new("future");
        let mut ahead = state("ahead", HookPhase::Blocked, 5);
        ahead.version = HOOK_STATE_VERSION + 1;
        fs::write(dir.0.join("ahead.json"), ahead.to_json()).expect("write");

        assert!(scan(&HookScanOptions::rooted_at(dir.0.clone()), 5).is_empty());
    }

    #[test]
    fn an_unknown_phase_or_provider_is_skipped() {
        let dir = TempDir::new("unknown");
        fs::write(
            dir.0.join("a.json"),
            br#"{"version":1,"sessionId":"a","provider":"CLAUDE_CODE","phase":"THINKING","since":1,"event":"X"}"#,
        )
        .expect("write");
        fs::write(
            dir.0.join("b.json"),
            br#"{"version":1,"sessionId":"b","provider":"SOMETHING_ELSE","phase":"BLOCKED","since":1,"event":"X"}"#,
        )
        .expect("write");

        assert!(scan(&HookScanOptions::rooted_at(dir.0.clone()), 1).is_empty());
    }

    #[test]
    fn an_oversized_file_is_not_read_at_all() {
        let dir = TempDir::new("huge");
        fs::write(dir.0.join("huge.json"), vec![b'x'; 32 * 1024]).expect("write");
        let mut options = HookScanOptions::rooted_at(dir.0.clone());
        options.max_bytes = 1_024;
        assert!(scan(&options, 1).is_empty());
    }

    #[test]
    fn the_file_cap_keeps_the_newest() {
        let dir = TempDir::new("cap");
        for index in 0..5 {
            let id = format!("s{index}");
            write(&dir.0, &state(&id, HookPhase::Active, 1_000));
            // Distinct modification times, which is what the cap sorts by.
            std::thread::sleep(Duration::from_millis(12));
        }
        let mut options = HookScanOptions::rooted_at(dir.0.clone());
        options.max_files = 2;
        let found = scan(&options, 1_000);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].session_id, "s4", "newest first");
        assert_eq!(found[1].session_id, "s3");
    }

    #[test]
    fn a_file_past_the_age_cutoff_is_ignored_and_swept() {
        let dir = TempDir::new("stale");
        write(&dir.0, &state("old", HookPhase::Blocked, 1_000));
        let mut options = HookScanOptions::rooted_at(dir.0.clone());
        options.max_age = Duration::from_millis(1);

        // `now` far in the future of the file's modification time.
        let found = scan(&options, now_ms_of(&dir.0.join("old.json")) + 10_000);
        assert!(found.is_empty(), "too old to be evidence about anything");
        assert!(
            !dir.0.join("old.json").exists(),
            "and swept, so the directory does not grow forever"
        );
    }

    fn now_ms_of(path: &Path) -> i64 {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .and_then(|since| i64::try_from(since.as_millis()).ok())
            .unwrap_or_default()
    }

    #[test]
    fn a_missing_state_directory_is_the_normal_case() {
        let options = HookScanOptions::rooted_at(PathBuf::from("/nonexistent/gritty/state"));
        assert!(scan(&options, 1).is_empty(), "nothing installed, no cost");
    }

    // --------------------------------------------------------- staleness

    #[test]
    fn age_is_compared_and_never_trusted_as_a_duration() {
        let blocked = state("a", HookPhase::Blocked, 10_000);
        assert_eq!(blocked.age_ms(12_000), 2_000);
        assert_eq!(
            blocked.age_ms(9_000),
            0,
            "a clock that moved backwards reads as zero, never negative"
        );
        assert!(!blocked.is_stale(12_000, Duration::from_secs(5)));
        assert!(blocked.is_stale(30_000, Duration::from_secs(5)));
    }
}
