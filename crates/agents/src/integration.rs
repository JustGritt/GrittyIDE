//! Writing Gritty's hook into another tool's configuration (T221, ADR-018).
//!
//! This is the one place in the repository that edits a file Gritty does not
//! own, and every rule here exists because of that. ADR-017 went out of its way
//! to reach a shell "never by editing the user's dotfiles"; T220's non-goals
//! said in as many words that Gritty does not write to `~/.claude/settings.json`.
//! ADR-018 is the deliberate, narrow exception, and this module is it made
//! executable.
//!
//! What that means concretely:
//!
//! - **Additive only.** One entry is appended to each event's existing array.
//!   No array is replaced, reordered or pruned of anything that is not
//!   Gritty's. The machine this was written on already has another tool
//!   registered on all fourteen events with `matcher: "*"`; an installer that
//!   wrote the array it wished to see would silently disable it, and the user
//!   would experience that as *the other app stopped working* (ADR-018 §2).
//! - **Refuse what is not understood.** A file that does not parse, or a hook
//!   section shaped in a way this code does not recognise, is reported and left
//!   alone. It is never repaired, never reformatted, never replaced from a
//!   template (§4). Losing a user's hook configuration to a confident parser
//!   would be unrecoverable.
//! - **Backed up first.** The prior contents are copied beside the original
//!   before anything is written, and if the backup cannot be written the change
//!   is abandoned before the original is touched (§3).
//! - **Recognised by its own binary.** Gritty's entries are the ones whose
//!   command names `gritty-agent-hook`, so uninstall needs no marker comment
//!   and no sidecar file — and an entry left pointing at a binary that has
//!   moved is still recognisably Gritty's, which is what makes `Broken`
//!   reportable rather than invisible.
//!
//! Formatting survives as far as the format allows: the file is parsed into
//! [`crate::json::Json`], which keeps object keys in the order they were
//! written, and re-emitted with the two-space indentation these files already
//! use.

use std::fs;
use std::path::{Path, PathBuf};

use crate::json::{self, Json};
use crate::probe::Provider;

/// The file name of the hook binary, which is also how Gritty's own entries
/// are recognised in a configuration full of other tools' entries.
pub const HOOK_BINARY_NAME: &str = "gritty-agent-hook";

/// Largest configuration file this code will parse.
///
/// A settings file is kilobytes. Anything past this is not one, and reading it
/// into memory to append a line would be the wrong trade.
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// The events Gritty registers on for Claude Code.
///
/// Exactly the events [`crate::hook::phase_of_event`] knows how to turn into a
/// phase, and no others: registering on an event whose payload Gritty would
/// discard would cost the agent a process launch for nothing.
pub const CLAUDE_CODE_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "PermissionDenied",
    "Stop",
    "StopFailure",
    "SessionEnd",
];

/// A provider is a config path, a parse and a set of event names — deliberately
/// not a trait, for the reason T220 gives about `Provider` itself: one match
/// arm is cheaper to read than an abstraction with a single implementation.
///
/// T222 adds Codex by adding one of these. Nothing else here changes.
#[derive(Debug, Clone)]
pub struct HookTarget {
    pub provider: Provider,
    /// The configuration file Gritty would edit.
    pub config_path: PathBuf,
    pub events: &'static [&'static str],
}

impl HookTarget {
    /// Where the given provider keeps its hooks, under `home`.
    pub fn for_provider(provider: Provider, home: &Path) -> Self {
        match provider {
            Provider::ClaudeCode => Self {
                provider,
                config_path: home.join(".claude").join("settings.json"),
                events: CLAUDE_CODE_EVENTS,
            },
        }
    }
}

/// What Gritty's entry looks like in that file today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationStatus {
    /// No entry of Gritty's anywhere in the file — including when the file does
    /// not exist, which is the default state and stays it.
    NotInstalled,
    /// Present, and pointing at a binary that is there.
    Installed,
    /// Present, and pointing at a binary that is not: the app moved, or was
    /// upgraded, or was deleted. The agent tolerates a hook that fails, so
    /// nothing breaks for the user — the roster just silently loses `BLOCKED`,
    /// and this is what says why.
    Broken,
    /// The file does not parse, or its hook section has a shape this code does
    /// not recognise. Nothing is written; the parse error is reported.
    UnsupportedConfig,
}

impl IntegrationStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "NOT_INSTALLED",
            Self::Installed => "INSTALLED",
            Self::Broken => "BROKEN",
            Self::UnsupportedConfig => "UNSUPPORTED_CONFIG",
        }
    }
}

/// The answer to "is it installed", with everything the UI needs to explain
/// itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationReport {
    pub status: IntegrationStatus,
    pub config_path: PathBuf,
    /// The events Gritty's entry covers today. Empty unless installed or
    /// broken.
    pub events: Vec<String>,
    /// Why it is broken or unsupported — the parse error, or the binary path
    /// that is not there. `None` when there is nothing to explain.
    pub detail: Option<String>,
}

/// One event's share of a proposed change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedEvent {
    pub event: String,
    /// Already carries a Gritty entry, so this event is left exactly as it is.
    /// An install over an existing one is a no-op that reports so; it never
    /// duplicates.
    pub already_present: bool,
    /// How many entries from *other* tools this event already carries. Shown
    /// because "we are appending to a list that has two other things in it" is
    /// the fact the user needs to trust the change (§2).
    pub foreign_entries: u32,
}

/// A proposed or performed change to a provider's configuration.
///
/// The same shape whether `apply` was false or true, so the UI shows the change
/// it is about to make and then the change it made, rather than a description
/// of one (ADR-018 §3).
///
/// Deliberately **not** the file's contents. The change is the command line and
/// the events it lands on; echoing a user's whole settings file back across the
/// protocol would put whatever else lives in it — tokens, private paths — into
/// an event payload for no gain (rule 13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationChange {
    pub config_path: PathBuf,
    /// Where the prior contents were copied, or would be. `None` when the file
    /// does not exist yet, because there is nothing to back up.
    pub backup_path: Option<PathBuf>,
    /// The file does not exist and would be created carrying only Gritty's
    /// entry.
    pub creates_config: bool,
    /// The exact command string that is being added or removed.
    pub command: String,
    pub events: Vec<PlannedEvent>,
    /// Whether this actually happened. `false` is the preview.
    pub applied: bool,
    /// What the status is now, or would be after applying.
    pub status: IntegrationStatus,
}

impl IntegrationChange {
    /// Whether applying this would change the file at all.
    pub fn is_noop(&self) -> bool {
        self.events.is_empty()
    }
}

/// Why an install or uninstall refused. Every one of these leaves the file
/// exactly as it was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IntegrationError {
    /// The file exists and does not parse, or its hook section is shaped in a
    /// way this code does not recognise. Never rewritten, never "repaired".
    #[error("{path} could not be understood: {reason}")]
    Unsupported { path: String, reason: String },
    /// The configuration, its directory, or the backup could not be written.
    #[error("{path} could not be written: {reason}")]
    PermissionDenied { path: String, reason: String },
    /// Reading failed for a reason that is not "it is not there".
    #[error("{path} could not be read: {reason}")]
    Unreadable { path: String, reason: String },
}

/// The command string Gritty registers.
///
/// Quoted, because both paths routinely contain spaces (`~/Library/Application
/// Support/…`), and single quotes because that is what a POSIX shell needs and
/// what the incumbent entry on this machine already uses. A path containing a
/// single quote is refused rather than escaped: the alternative is quoting
/// rules that differ per shell, and a hook command that half-works is worse
/// than one that was never installed.
///
/// The state directory is baked into the command rather than computed by the
/// hook binary, so that binary carries no knowledge of where Gritty's app data
/// lives on each platform — and `GRITTY_DATA_DIR` keeps working for free.
pub fn hook_command(hook_binary: &Path, state_dir: &Path, provider: Provider) -> Option<String> {
    let binary = quotable(hook_binary)?;
    let state_dir = quotable(state_dir)?;
    Some(format!(
        "'{binary}' --provider {} --state-dir '{state_dir}' --ask-timeout {ASK_WAIT_SECONDS}",
        provider.as_str().to_ascii_lowercase()
    ))
}

/// How long the hook may be held waiting for the user to answer (ADR-020).
///
/// Long enough to be worth having — a person who is looking at something else
/// takes minutes, not seconds — and finite, because the ceiling on how long an
/// agent can be held has to be a number somebody chose rather than an accident.
pub const ASK_WAIT_SECONDS: u64 = 300;

/// The `timeout` written onto the `PermissionRequest` entry itself.
///
/// **Strictly greater than [`ASK_WAIT_SECONDS`], and that ordering is the whole
/// point.** Claude Code discards a hook's output once the entry's timeout
/// passes (measured, ADR-020), so a wait that ran to its own deadline must
/// still have room to be heard. Written into the user's own configuration
/// rather than hidden in the binary, because it is the real bound on how long
/// their agent can sit waiting and they are entitled to see it.
pub const ASK_ENTRY_TIMEOUT_SECONDS: u64 = ASK_WAIT_SECONDS + 30;

// Asserted at compile time rather than in a test, the way the migration table
// asserts its own contiguity: if these two ever cross, every slowly-answered
// prompt would silently fall back to the terminal, and no test would have to
// be running for that to be true.
const _: () = assert!(ASK_ENTRY_TIMEOUT_SECONDS > ASK_WAIT_SECONDS);

/// The one event whose entry carries a timeout, because it is the one event
/// that waits.
pub const ASKING_EVENT: &str = "PermissionRequest";

/// A path that can be put inside single quotes without escaping.
fn quotable(path: &Path) -> Option<&str> {
    path.to_str().filter(|path| !path.contains('\''))
}

/// Whether a command string is one of Gritty's.
///
/// Identity is the *name* of the binary the command runs, not equality with the
/// binary this build would install. That is deliberate: after an upgrade moves
/// the app, the entry left behind still has to be recognisable as Gritty's, or
/// it could neither be reported as [`IntegrationStatus::Broken`] nor removed.
pub fn gritty_binary_in(command: &str) -> Option<&str> {
    let program = leading_token(command)?;
    Path::new(program)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| *name == HOOK_BINARY_NAME)
        .map(|_| program)
}

/// The program a hook command runs: the first single- or double-quoted run, or
/// everything up to the first space.
fn leading_token(command: &str) -> Option<&str> {
    let command = command.trim();
    let mut chars = command.char_indices();
    match chars.next() {
        Some((_, quote @ ('\'' | '"'))) => {
            let end = command[1..].find(quote)? + 1;
            Some(&command[1..end])
        }
        Some(_) => Some(command.split(char::is_whitespace).next().unwrap_or(command)),
        None => None,
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// What the file says today.
///
/// A missing file is [`IntegrationStatus::NotInstalled`], not an error: nothing
/// installed is the default state and stays it.
pub fn status(target: &HookTarget) -> IntegrationReport {
    let config = match read_config(&target.config_path) {
        Ok(config) => config,
        // A file that cannot be read and one that cannot be understood are the
        // same answer to the user: Gritty will not touch it, and here is why.
        Err(error) => {
            return IntegrationReport {
                status: IntegrationStatus::UnsupportedConfig,
                config_path: target.config_path.clone(),
                events: Vec::new(),
                detail: Some(error.to_string()),
            };
        }
    };

    let Some(root) = config else {
        return IntegrationReport {
            status: IntegrationStatus::NotInstalled,
            config_path: target.config_path.clone(),
            events: Vec::new(),
            detail: None,
        };
    };

    let hooks = match hooks_section(&root, &target.config_path) {
        Ok(hooks) => hooks,
        Err(error) => {
            return IntegrationReport {
                status: IntegrationStatus::UnsupportedConfig,
                config_path: target.config_path.clone(),
                events: Vec::new(),
                detail: Some(error.to_string()),
            };
        }
    };

    let mut events = Vec::new();
    let mut missing_binary: Option<String> = None;
    if let Some(hooks) = hooks {
        for (event, value) in hooks {
            let groups = match matcher_groups(value, event, &target.config_path) {
                Ok(groups) => groups,
                Err(error) => {
                    return IntegrationReport {
                        status: IntegrationStatus::UnsupportedConfig,
                        config_path: target.config_path.clone(),
                        events: Vec::new(),
                        detail: Some(error.to_string()),
                    };
                }
            };
            let mut covered = false;
            for group in groups {
                for command in group {
                    if let Some(program) = gritty_binary_in(command) {
                        covered = true;
                        if !Path::new(program).is_file() {
                            missing_binary.get_or_insert_with(|| program.to_owned());
                        }
                    }
                }
            }
            if covered {
                events.push((*event).to_owned());
            }
        }
    }

    let status = if events.is_empty() {
        IntegrationStatus::NotInstalled
    } else if missing_binary.is_some() {
        IntegrationStatus::Broken
    } else {
        IntegrationStatus::Installed
    };
    IntegrationReport {
        status,
        config_path: target.config_path.clone(),
        events,
        detail: missing_binary.map(|path| format!("the hook binary is not at {path}")),
    }
}

/// `None` when the file does not exist. `Err` when it exists and cannot be
/// understood — which is refusal, not repair.
fn read_config(path: &Path) -> Result<Option<Vec<(String, Json)>>, IntegrationError> {
    let shown = path.display().to_string();
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(IntegrationError::Unreadable {
                path: shown,
                reason: error.to_string(),
            });
        }
    };
    if !metadata.is_file() {
        return Err(IntegrationError::Unsupported {
            path: shown,
            reason: "it is not a file".to_owned(),
        });
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(IntegrationError::Unsupported {
            path: shown,
            reason: format!("it is larger than {MAX_CONFIG_BYTES} bytes"),
        });
    }
    let raw = fs::read_to_string(path).map_err(|error| IntegrationError::Unreadable {
        path: shown.clone(),
        reason: error.to_string(),
    })?;
    // An empty file is a file someone truncated, not an empty object. Treating
    // it as `{}` and writing a fresh config over it would be exactly the
    // "repair" §4 forbids.
    match serde_json::from_str::<Json>(&raw) {
        Ok(Json::Object(object)) => Ok(Some(object)),
        Ok(_) => Err(IntegrationError::Unsupported {
            path: shown,
            reason: "the top level is not a JSON object".to_owned(),
        }),
        Err(error) => Err(IntegrationError::Unsupported {
            path: shown,
            reason: error.to_string(),
        }),
    }
}

/// The `hooks` object, or `None` when the file has no hook section at all.
fn hooks_section<'a>(
    root: &'a [(String, Json)],
    path: &Path,
) -> Result<Option<&'a [(String, Json)]>, IntegrationError> {
    match root
        .iter()
        .find(|(key, _)| key == "hooks")
        .map(|(_, value)| value)
    {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(hooks)) => Ok(Some(hooks)),
        Some(_) => Err(IntegrationError::Unsupported {
            path: path.display().to_string(),
            reason: "`hooks` is not an object".to_owned(),
        }),
    }
}

/// Every command string one event registers, grouped by matcher entry.
///
/// The shape both Claude Code and Codex use:
///
/// ```json
/// "PreToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": "…" } ] } ]
/// ```
///
/// Anything else is refused rather than guessed at.
fn matcher_groups<'a>(
    value: &'a Json,
    event: &str,
    path: &Path,
) -> Result<Vec<Vec<&'a str>>, IntegrationError> {
    let refuse = |reason: String| IntegrationError::Unsupported {
        path: path.display().to_string(),
        reason: format!("`hooks.{event}` {reason}"),
    };
    let Json::Array(entries) = value else {
        return Err(refuse("is not an array".to_owned()));
    };
    let mut groups = Vec::new();
    for entry in entries {
        let Json::Object(_) = entry else {
            return Err(refuse(
                "contains something that is not an object".to_owned(),
            ));
        };
        let commands = match entry.get("hooks") {
            None | Some(Json::Null) => Vec::new(),
            Some(Json::Array(hooks)) => {
                let mut commands = Vec::new();
                for hook in hooks {
                    let Json::Object(_) = hook else {
                        return Err(refuse("contains a hook that is not an object".to_owned()));
                    };
                    // A hook with no `command` is not one this code can
                    // recognise as anyone's, and leaving it alone is correct.
                    if let Some(Json::String(command)) = hook.get("command") {
                        commands.push(command.as_str());
                    }
                }
                commands
            }
            Some(_) => {
                return Err(refuse(
                    "has a `hooks` field that is not an array".to_owned(),
                ));
            }
        };
        groups.push(commands);
    }
    Ok(groups)
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Plan (`apply == false`) or perform (`apply == true`) an install.
///
/// The two are one function on purpose: a preview the UI shows and an apply
/// that then does something else would be a diff nobody can trust.
pub fn install(
    target: &HookTarget,
    hook_binary: &Path,
    state_dir: &Path,
    apply: bool,
    now_ms: i64,
) -> Result<IntegrationChange, IntegrationError> {
    let command = hook_command(hook_binary, state_dir, target.provider).ok_or_else(|| {
        IntegrationError::Unsupported {
            path: hook_binary.display().to_string(),
            reason: "the hook binary's path cannot be quoted for a shell".to_owned(),
        }
    })?;
    let existing = read_config(&target.config_path)?;
    let creates_config = existing.is_none();
    let mut root = existing.unwrap_or_default();

    // Parse everything before writing anything: a file that is half understood
    // must not be half edited.
    let mut planned = Vec::new();
    for event in target.events {
        let current = root
            .iter()
            .find(|(key, _)| key == "hooks")
            .map(|(_, value)| value)
            .and_then(|hooks| hooks.get(event));
        let groups = match current {
            Some(value) => matcher_groups(value, event, &target.config_path)?,
            None => Vec::new(),
        };
        let already_present = groups
            .iter()
            .flatten()
            .any(|command| gritty_binary_in(command).is_some());
        let foreign_entries = groups
            .iter()
            .flatten()
            .filter(|command| gritty_binary_in(command).is_none())
            .count();
        if !already_present {
            planned.push(PlannedEvent {
                event: (*event).to_owned(),
                already_present: false,
                foreign_entries: u32::try_from(foreign_entries).unwrap_or(u32::MAX),
            });
        }
    }
    // Refuse a shape we do not recognise even on events we are not changing:
    // the file is re-emitted whole, so anything unparsed would be at risk.
    check_every_event(&root, &target.config_path)?;

    let backup_path = (!creates_config).then(|| backup_path(&target.config_path, now_ms));
    let change = IntegrationChange {
        config_path: target.config_path.clone(),
        backup_path: backup_path.clone(),
        creates_config,
        command: command.clone(),
        events: planned.clone(),
        applied: apply,
        status: IntegrationStatus::Installed,
    };
    if !apply || planned.is_empty() {
        // An install over an existing one is a no-op that says so rather than
        // duplicating the entry, and a preview writes nothing by definition.
        return Ok(IntegrationChange {
            applied: apply && planned.is_empty(),
            ..change
        });
    }

    for event in &planned {
        append_entry(&mut root, &event.event, &command);
    }
    commit(&target.config_path, backup_path.as_deref(), &root)?;
    Ok(change)
}

/// Remove exactly the entries whose command runs Gritty's binary.
///
/// Everything else is left byte-identical modulo the re-emitted JSON's own
/// formatting: no other entry is touched, no array is reordered, and a matcher
/// group is dropped only when removing Gritty's entry emptied it.
pub fn uninstall(
    target: &HookTarget,
    apply: bool,
    now_ms: i64,
) -> Result<IntegrationChange, IntegrationError> {
    let Some(mut root) = read_config(&target.config_path)? else {
        return Ok(IntegrationChange {
            config_path: target.config_path.clone(),
            backup_path: None,
            creates_config: false,
            command: String::new(),
            events: Vec::new(),
            applied: apply,
            status: IntegrationStatus::NotInstalled,
        });
    };
    check_every_event(&root, &target.config_path)?;

    let mut planned = Vec::new();
    let mut command = String::new();
    if let Some(Json::Object(hooks)) = root
        .iter()
        .find(|(key, _)| key == "hooks")
        .map(|(_, value)| value)
    {
        for (event, value) in hooks {
            let groups = matcher_groups(value, event, &target.config_path)?;
            let mine: Vec<&str> = groups
                .iter()
                .flatten()
                .filter(|entry| gritty_binary_in(entry).is_some())
                .copied()
                .collect();
            if mine.is_empty() {
                continue;
            }
            if command.is_empty() {
                command = mine[0].to_owned();
            }
            let foreign = groups
                .iter()
                .flatten()
                .filter(|entry| gritty_binary_in(entry).is_none())
                .count();
            planned.push(PlannedEvent {
                event: (*event).to_owned(),
                already_present: true,
                foreign_entries: u32::try_from(foreign).unwrap_or(u32::MAX),
            });
        }
    }

    let backup_path = backup_path(&target.config_path, now_ms);
    let change = IntegrationChange {
        config_path: target.config_path.clone(),
        backup_path: Some(backup_path.clone()),
        creates_config: false,
        command,
        events: planned.clone(),
        applied: apply,
        status: IntegrationStatus::NotInstalled,
    };
    if !apply || planned.is_empty() {
        return Ok(change);
    }

    for event in &planned {
        remove_entry(&mut root, &event.event);
    }
    commit(&target.config_path, Some(&backup_path), &root)?;
    Ok(change)
}

/// Every event in the file, not only the ones we register on, must parse: the
/// whole file is re-emitted, so a section this code cannot read is a section it
/// could silently mangle.
fn check_every_event(root: &[(String, Json)], path: &Path) -> Result<(), IntegrationError> {
    if let Some(hooks) = hooks_section(root, path)? {
        for (event, value) in hooks {
            matcher_groups(value, event, path)?;
        }
    }
    Ok(())
}

/// Append Gritty's entry to one event, creating the shape around it only where
/// it is missing.
fn append_entry(root: &mut Vec<(String, Json)>, event: &str, command: &str) {
    let hooks = json::entry(root, "hooks", Json::Object(Vec::new()));
    let Some(hooks) = hooks.as_object_mut() else {
        return;
    };
    let entries = json::entry(hooks, event, Json::Array(Vec::new()));
    let Some(entries) = entries.as_array_mut() else {
        return;
    };
    let mut hook = vec![
        ("type".to_owned(), Json::String("command".to_owned())),
        ("command".to_owned(), Json::String(command.to_owned())),
    ];
    // Only the asking event waits, so only the asking event needs room to. A
    // timeout on the others would be a number with nothing behind it.
    if event == ASKING_EVENT {
        hook.push((
            "timeout".to_owned(),
            Json::Number(ASK_ENTRY_TIMEOUT_SECONDS.into()),
        ));
    }
    let hook = Json::Object(hook);
    let group = Json::Object(vec![
        ("matcher".to_owned(), Json::String("*".to_owned())),
        ("hooks".to_owned(), Json::Array(vec![hook])),
    ]);
    // Appended as its own matcher group rather than pushed into somebody
    // else's: their group is theirs, and an agent runs every matching hook, so
    // a separate group costs the incumbent nothing (§2).
    entries.push(group);
}

/// Drop Gritty's entries from one event, and drop what removing them emptied.
fn remove_entry(root: &mut Vec<(String, Json)>, event: &str) {
    let Some(hooks) = root
        .iter_mut()
        .find(|(key, _)| key == "hooks")
        .and_then(|(_, value)| value.as_object_mut())
    else {
        return;
    };
    let Some(entries) = hooks
        .iter_mut()
        .find(|(key, _)| key == event)
        .and_then(|(_, value)| value.as_array_mut())
    else {
        return;
    };
    entries.retain_mut(|entry| {
        let Some(entry) = entry.as_object_mut() else {
            return true;
        };
        let Some(commands) = entry
            .iter_mut()
            .find(|(key, _)| key == "hooks")
            .and_then(|(_, value)| value.as_array_mut())
        else {
            return true;
        };
        commands.retain(|hook| {
            hook.get("command")
                .and_then(Json::as_str)
                .is_none_or(|command| gritty_binary_in(command).is_none())
        });
        // A matcher group that held nothing but Gritty's entry goes with it;
        // one that still holds someone else's stays exactly as it was.
        !commands.is_empty()
    });
    if entries.is_empty() {
        json::remove(hooks, event);
    }
    if hooks.is_empty() {
        json::remove(root, "hooks");
    }
}

/// `<config>.gritty-backup-<epoch-ms>`, beside the original.
///
/// Epoch milliseconds rather than a formatted date: it sorts correctly, it is
/// unambiguous across time zones, and it needs no date library in a crate whose
/// whole job is reading other people's files.
fn backup_path(config_path: &Path, now_ms: i64) -> PathBuf {
    let name = config_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_owned());
    config_path.with_file_name(format!("{name}.gritty-backup-{now_ms}"))
}

/// Back up, then replace. In that order, and never the other way round.
fn commit(
    config_path: &Path,
    backup_path: Option<&Path>,
    root: &[(String, Json)],
) -> Result<(), IntegrationError> {
    let shown = config_path.display().to_string();
    // A read-only file would still be replaced by an atomic rename — the rename
    // needs write permission on the *directory*, not the file — so refusing
    // here is what makes "the user marked this file read-only" mean something.
    if let Ok(metadata) = fs::metadata(config_path)
        && metadata.permissions().readonly()
    {
        return Err(IntegrationError::PermissionDenied {
            path: shown,
            reason: "the file is read-only".to_owned(),
        });
    }
    if let Some(backup_path) = backup_path {
        fs::copy(config_path, backup_path).map_err(|error| {
            // Before the original is touched: a change that cannot be undone is
            // not a change this code makes (§3).
            IntegrationError::PermissionDenied {
                path: backup_path.display().to_string(),
                reason: error.to_string(),
            }
        })?;
    }
    if let Some(parent) = config_path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.is_dir()
    {
        fs::create_dir_all(parent).map_err(|error| IntegrationError::PermissionDenied {
            path: parent.display().to_string(),
            reason: error.to_string(),
        })?;
    }
    // Two-space indentation, which is what these files already use, and the
    // file's own key order, which the ordered `Json` model is what keeps.
    let mut serialized =
        serde_json::to_string_pretty(&Json::Object(root.to_vec())).map_err(|error| {
            IntegrationError::Unsupported {
                path: shown.clone(),
                reason: error.to_string(),
            }
        })?;
    serialized.push('\n');
    gritty_workspace::write_file_atomic(
        config_path,
        serialized.as_bytes(),
        gritty_workspace::AtomicWriteFault::None,
    )
    .map_err(|error| IntegrationError::PermissionDenied {
        path: shown,
        reason: error.to_string(),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    /// A scratch home. Every test in this file writes into one of these and
    /// never into a real `~/.claude`.
    struct TempHome(PathBuf);

    impl TempHome {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gritty-integration-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(path.join(".claude")).expect("create temp home");
            Self(path)
        }

        fn target(&self) -> HookTarget {
            HookTarget::for_provider(Provider::ClaudeCode, &self.0)
        }

        fn config(&self) -> PathBuf {
            self.0.join(".claude").join("settings.json")
        }

        fn write_config(&self, raw: &str) {
            fs::write(self.config(), raw).expect("write config");
        }

        fn read_config(&self) -> String {
            fs::read_to_string(self.config()).expect("read config")
        }

        /// A hook binary that exists, so `Installed` and `Broken` can be told
        /// apart without the real application being anywhere.
        fn hook_binary(&self) -> PathBuf {
            let path = self.0.join(HOOK_BINARY_NAME);
            fs::write(&path, b"#!/bin/sh\n").expect("write hook binary");
            path
        }

        /// Where the installed command line would tell the hook to write. Under
        /// the scratch home, never under a real application-data directory.
        fn state_dir(&self) -> PathBuf {
            self.0.join("data").join("agents").join("state")
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The configuration this machine actually has: another tool on every
    /// event, with `matcher: "*"`. The important fixture, not the edge one.
    fn foreign_on_every_event() -> String {
        let events: Vec<String> = CLAUDE_CODE_EVENTS
            .iter()
            .map(|event| {
                format!(
                    r#"    "{event}": [
      {{
        "matcher": "*",
        "hooks": [
          {{
            "type": "command",
            "command": "'/Applications/OtherTool.app/bin/OtherToolHooks' --source claude"
          }}
        ]
      }}
    ]"#
                )
            })
            .collect();
        format!(
            "{{\n  \"model\": \"opus\",\n  \"hooks\": {{\n{}\n  }},\n  \"theme\": \"dark\"\n}}\n",
            events.join(",\n")
        )
    }

    // ---------------------------------------------------------------- status

    #[test]
    fn an_absent_config_is_not_installed_and_is_not_an_error() {
        let home = TempHome::new("absent");
        let report = status(&home.target());
        assert_eq!(report.status, IntegrationStatus::NotInstalled);
        assert_eq!(report.detail, None);
        assert!(report.events.is_empty());
    }

    #[test]
    fn a_config_with_only_other_tools_is_not_installed() {
        let home = TempHome::new("foreign");
        home.write_config(&foreign_on_every_event());
        assert_eq!(
            status(&home.target()).status,
            IntegrationStatus::NotInstalled
        );
    }

    #[test]
    fn a_malformed_config_is_unsupported_and_names_the_parse_error() {
        let home = TempHome::new("malformed");
        home.write_config("{ \"hooks\": { oops }");
        let report = status(&home.target());
        assert_eq!(report.status, IntegrationStatus::UnsupportedConfig);
        assert!(report.detail.is_some(), "the parse error is reported");
    }

    #[test]
    fn a_hook_section_in_an_unrecognised_shape_is_unsupported() {
        let home = TempHome::new("shape");
        home.write_config(r#"{"hooks": {"PreToolUse": "run-this"}}"#);
        assert_eq!(
            status(&home.target()).status,
            IntegrationStatus::UnsupportedConfig
        );

        home.write_config(r#"{"hooks": ["PreToolUse"]}"#);
        assert_eq!(
            status(&home.target()).status,
            IntegrationStatus::UnsupportedConfig
        );
    }

    #[test]
    fn an_entry_pointing_at_a_binary_that_is_not_there_is_broken() {
        let home = TempHome::new("broken");
        let binary = home.hook_binary();
        install(&home.target(), &binary, &home.state_dir(), true, 1).expect("install");
        assert_eq!(status(&home.target()).status, IntegrationStatus::Installed);

        fs::remove_file(&binary).expect("remove binary");
        let report = status(&home.target());
        assert_eq!(
            report.status,
            IntegrationStatus::Broken,
            "the app moved; the entry is still ours"
        );
        assert!(report.detail.expect("a reason").contains(HOOK_BINARY_NAME));
    }

    // --------------------------------------------------------------- install

    #[test]
    fn a_preview_writes_nothing() {
        let home = TempHome::new("preview");
        home.write_config(&foreign_on_every_event());
        let before = home.read_config();

        let change = install(
            &home.target(),
            &home.hook_binary(),
            &home.state_dir(),
            false,
            1_700,
        )
        .expect("plan");
        assert!(!change.applied);
        assert_eq!(change.events.len(), CLAUDE_CODE_EVENTS.len());
        assert!(change.command.contains(HOOK_BINARY_NAME));
        assert_eq!(
            change.backup_path,
            Some(
                home.config()
                    .with_file_name("settings.json.gritty-backup-1700")
            ),
            "the backup it would create is named before it is made"
        );
        assert_eq!(home.read_config(), before, "the file is untouched");
        assert!(
            !change.backup_path.expect("a path").exists(),
            "and no backup was written either"
        );
    }

    #[test]
    fn installing_beside_another_tool_leaves_every_one_of_its_entries_intact() {
        let home = TempHome::new("additive");
        home.write_config(&foreign_on_every_event());
        let before: Value = serde_json::from_str(&home.read_config()).expect("before");

        install(
            &home.target(),
            &home.hook_binary(),
            &home.state_dir(),
            true,
            1_700,
        )
        .expect("install");
        let after: Value = serde_json::from_str(&home.read_config()).expect("after");

        let before_hooks = before["hooks"].as_object().expect("before hooks");
        let after_hooks = after["hooks"].as_object().expect("after hooks");
        for (event, value) in before_hooks {
            let after_entries = after_hooks[event].as_array().expect("after entries");
            let before_entries = value.as_array().expect("before entries");
            assert_eq!(
                &after_entries[..before_entries.len()],
                &before_entries[..],
                "{event}: the incumbent's entries are byte-identical and still first"
            );
            assert_eq!(after_entries.len(), before_entries.len() + 1);
        }
        assert_eq!(
            after["model"], before["model"],
            "nothing outside `hooks` moved"
        );
    }

    #[test]
    fn key_order_and_indentation_survive() {
        let home = TempHome::new("formatting");
        home.write_config(&foreign_on_every_event());
        install(
            &home.target(),
            &home.hook_binary(),
            &home.state_dir(),
            true,
            1_700,
        )
        .expect("install");
        let after = home.read_config();

        let keys: Vec<&str> = after
            .lines()
            .filter(|line| line.starts_with("  \""))
            .map(|line| line.trim())
            .collect();
        assert_eq!(
            keys,
            vec![
                "\"model\": \"opus\",",
                "\"hooks\": {",
                "\"theme\": \"dark\""
            ],
            "the file's own key order, not the alphabet"
        );
        assert!(
            after
                .lines()
                .any(|line| line.starts_with("    \"SessionStart\"")),
            "two-space indentation, nested"
        );
    }

    #[test]
    fn installing_over_an_existing_entry_is_a_no_op_that_says_so() {
        let home = TempHome::new("idempotent");
        let binary = home.hook_binary();
        install(&home.target(), &binary, &home.state_dir(), true, 1).expect("first install");
        let after_first = home.read_config();

        let second =
            install(&home.target(), &binary, &home.state_dir(), true, 2).expect("second install");
        assert!(second.is_noop());
        assert_eq!(home.read_config(), after_first, "nothing was duplicated");
        assert!(
            !home
                .config()
                .with_file_name("settings.json.gritty-backup-2")
                .exists(),
            "a no-op writes no backup either"
        );
    }

    #[test]
    fn an_absent_config_is_created_carrying_only_grittys_entry() {
        let home = TempHome::new("create");
        let change = install(
            &home.target(),
            &home.hook_binary(),
            &home.state_dir(),
            true,
            1,
        )
        .expect("install");
        assert!(change.creates_config);
        assert_eq!(change.backup_path, None, "nothing to back up");

        let written: Value = serde_json::from_str(&home.read_config()).expect("parse");
        let hooks = written["hooks"].as_object().expect("hooks");
        assert_eq!(hooks.len(), CLAUDE_CODE_EVENTS.len());
        assert_eq!(status(&home.target()).status, IntegrationStatus::Installed);
    }

    #[test]
    fn a_malformed_config_is_refused_and_left_exactly_as_it_was() {
        let home = TempHome::new("refuse");
        let raw = "{ \"hooks\": { oops }";
        home.write_config(raw);

        let error = install(
            &home.target(),
            &home.hook_binary(),
            &home.state_dir(),
            true,
            1,
        )
        .expect_err("refused");
        assert!(
            matches!(error, IntegrationError::Unsupported { .. }),
            "{error}"
        );
        assert_eq!(home.read_config(), raw, "never rewritten, never repaired");
    }

    #[test]
    fn an_unrecognised_shape_is_refused_even_on_an_event_gritty_does_not_register_on() {
        let home = TempHome::new("foreign-shape");
        home.write_config(r#"{"hooks": {"PreCompact": {"weird": true}}}"#);
        let error = install(
            &home.target(),
            &home.hook_binary(),
            &home.state_dir(),
            true,
            1,
        )
        .expect_err("refused");
        assert!(
            matches!(error, IntegrationError::Unsupported { .. }),
            "{error}"
        );
    }

    #[test]
    fn a_read_only_config_is_refused_before_anything_is_written() {
        let home = TempHome::new("read-only");
        home.write_config(&foreign_on_every_event());
        let before = home.read_config();
        let mut permissions = fs::metadata(home.config()).expect("metadata").permissions();
        permissions.set_readonly(true);
        fs::set_permissions(home.config(), permissions).expect("chmod");

        let error = install(
            &home.target(),
            &home.hook_binary(),
            &home.state_dir(),
            true,
            1_700,
        )
        .expect_err("refused");
        assert!(
            matches!(error, IntegrationError::PermissionDenied { .. }),
            "{error}"
        );
        assert_eq!(home.read_config(), before);
        assert!(
            !home
                .config()
                .with_file_name("settings.json.gritty-backup-1700")
                .exists(),
            "nothing half-written: the refusal came before the backup"
        );

        // Owner-writable again, so the scratch home can be removed. Set by
        // mode rather than `set_readonly(false)`, which on Unix would make the
        // file world-writable.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(home.config(), fs::Permissions::from_mode(0o600))
                .expect("chmod back");
        }
    }

    #[test]
    fn a_backup_is_written_beside_the_original_before_it_changes() {
        let home = TempHome::new("backup");
        home.write_config(&foreign_on_every_event());
        let before = home.read_config();

        install(
            &home.target(),
            &home.hook_binary(),
            &home.state_dir(),
            true,
            1_700,
        )
        .expect("install");
        let backup = home
            .config()
            .with_file_name("settings.json.gritty-backup-1700");
        assert_eq!(
            fs::read_to_string(&backup).expect("backup"),
            before,
            "the prior contents, byte for byte"
        );
    }

    // ------------------------------------------------------------- uninstall

    #[test]
    fn uninstalling_returns_the_file_to_the_backup_except_for_grittys_entries() {
        let home = TempHome::new("uninstall");
        home.write_config(&foreign_on_every_event());
        install(
            &home.target(),
            &home.hook_binary(),
            &home.state_dir(),
            true,
            1_700,
        )
        .expect("install");
        let backup = fs::read_to_string(
            home.config()
                .with_file_name("settings.json.gritty-backup-1700"),
        )
        .expect("backup");

        let change = uninstall(&home.target(), true, 1_800).expect("uninstall");
        assert_eq!(change.events.len(), CLAUDE_CODE_EVENTS.len());

        // Modulo the re-emitted JSON's own formatting, which is why this
        // compares values rather than bytes.
        let restored: Value = serde_json::from_str(&home.read_config()).expect("restored");
        let original: Value = serde_json::from_str(&backup).expect("original");
        assert_eq!(restored, original);
        assert_eq!(
            status(&home.target()).status,
            IntegrationStatus::NotInstalled
        );
    }

    #[test]
    fn uninstalling_from_a_config_gritty_never_touched_changes_nothing() {
        let home = TempHome::new("uninstall-clean");
        home.write_config(&foreign_on_every_event());
        let before = home.read_config();
        let change = uninstall(&home.target(), true, 1).expect("uninstall");
        assert!(change.is_noop());
        assert_eq!(home.read_config(), before);
    }

    #[test]
    fn uninstalling_removes_the_event_key_it_created_and_keeps_the_ones_it_did_not() {
        let home = TempHome::new("uninstall-keys");
        home.write_config(r#"{"hooks":{"Stop":[{"matcher":"*","hooks":[{"type":"command","command":"'/other/tool' x"}]}]}}"#);
        install(
            &home.target(),
            &home.hook_binary(),
            &home.state_dir(),
            true,
            1,
        )
        .expect("install");
        uninstall(&home.target(), true, 2).expect("uninstall");

        let after: Value = serde_json::from_str(&home.read_config()).expect("parse");
        let hooks = after["hooks"].as_object().expect("hooks");
        assert_eq!(
            hooks.keys().collect::<Vec<_>>(),
            vec!["Stop"],
            "the events Gritty created are gone; the one it shared survives"
        );
        assert_eq!(
            hooks["Stop"].as_array().expect("entries").len(),
            1,
            "and the incumbent's group is still there, alone again"
        );
    }

    #[test]
    fn uninstall_removes_an_entry_whose_binary_has_moved() {
        let home = TempHome::new("uninstall-moved");
        home.write_config(
            r#"{"hooks":{"Stop":[{"matcher":"*","hooks":[{"type":"command","command":"'/gone/away/gritty-agent-hook' --provider claude-code"}]}]}}"#,
        );
        assert_eq!(status(&home.target()).status, IntegrationStatus::Broken);
        uninstall(&home.target(), true, 1).expect("uninstall");
        assert_eq!(
            status(&home.target()).status,
            IntegrationStatus::NotInstalled
        );
    }

    // ------------------------------------------------------ command identity

    #[test]
    fn grittys_entries_are_recognised_by_the_binary_they_run() {
        assert_eq!(
            gritty_binary_in(
                "'/Applications/Gritty.app/Contents/MacOS/gritty-agent-hook' --provider claude-code"
            ),
            Some("/Applications/Gritty.app/Contents/MacOS/gritty-agent-hook")
        );
        assert_eq!(
            gritty_binary_in("/usr/local/bin/gritty-agent-hook"),
            Some("/usr/local/bin/gritty-agent-hook")
        );
        assert_eq!(
            gritty_binary_in("\"/with spaces/gritty-agent-hook\" --provider claude-code"),
            Some("/with spaces/gritty-agent-hook")
        );
        assert_eq!(
            gritty_binary_in("'/other/tool/OtherToolHooks' --source claude"),
            None
        );
        assert_eq!(gritty_binary_in("gritty-agent-hook-extra"), None);
        assert_eq!(gritty_binary_in(""), None);
    }

    #[test]
    fn a_path_that_cannot_be_quoted_is_refused_rather_than_escaped() {
        let state = Path::new("/data/agents/state");
        assert_eq!(
            hook_command(
                Path::new("/it's/awkward/gritty-agent-hook"),
                state,
                Provider::ClaudeCode
            ),
            None
        );
        assert_eq!(
            hook_command(
                Path::new("/fine/gritty-agent-hook"),
                Path::new("/it's/awkward"),
                Provider::ClaudeCode
            ),
            None
        );
    }

    #[test]
    fn only_the_asking_event_carries_a_timeout_and_it_outlasts_the_wait() {
        // Claude Code discards a hook's output once the entry's own timeout
        // passes (measured, ADR-020). That the entry outlasts the wait is
        // asserted at compile time beside the two constants; this is about
        // which events carry it.
        let home = TempHome::new("entry-timeout");
        home.write_config("{}");
        install(
            &home.target(),
            &home.hook_binary(),
            &home.state_dir(),
            true,
            1_700,
        )
        .expect("install");

        let raw = home.read_config();
        let written: serde_json::Value = serde_json::from_str(&raw).expect("valid json");
        let hooks = &written["hooks"];
        for event in CLAUDE_CODE_EVENTS {
            let ours = hooks[event]
                .as_array()
                .expect("an array")
                .iter()
                .find(|group| {
                    group["hooks"][0]["command"]
                        .as_str()
                        .is_some_and(|command| command.contains("gritty-agent-hook"))
                })
                .expect("our group");
            let timeout = ours["hooks"][0].get("timeout").and_then(|t| t.as_u64());
            if *event == ASKING_EVENT {
                assert_eq!(timeout, Some(ASK_ENTRY_TIMEOUT_SECONDS), "{event}");
            } else {
                assert_eq!(timeout, None, "{event} does not wait, so it needs no room");
            }
        }
    }

    #[test]
    fn the_command_carries_the_state_directory_so_the_hook_guesses_at_nothing() {
        let command = hook_command(
            Path::new("/Applications/Gritty.app/Contents/MacOS/gritty-agent-hook"),
            Path::new("/Users/a/Library/Application Support/gritty/agents/state"),
            Provider::ClaudeCode,
        )
        .expect("a command");
        assert_eq!(
            command,
            "'/Applications/Gritty.app/Contents/MacOS/gritty-agent-hook' --provider claude_code \
             --state-dir '/Users/a/Library/Application Support/gritty/agents/state' \
             --ask-timeout 300"
        );
        assert_eq!(
            gritty_binary_in(&command),
            Some("/Applications/Gritty.app/Contents/MacOS/gritty-agent-hook"),
            "and it is still recognisable as ours"
        );
    }
}
