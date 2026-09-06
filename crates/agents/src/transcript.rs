//! Transcript evidence: what each session is actually working on.
//!
//! Claude Code appends one JSON object per line to
//! `~/.claude/projects/<encoded-root>/<session-id>.jsonl`. Nearly every record
//! repeats `sessionId`, `cwd`, `gitBranch` and `version`, and the format also
//! carries records Gritty can use directly instead of guessing:
//!
//! - `{"type":"ai-title","aiTitle":…}` — the agent's own name for the session,
//!   which is the closest thing to "the task" that exists;
//! - `{"type":"last-prompt","lastPrompt":…}` — the standing user request;
//! - assistant `tool_use` blocks, closed by the matching user `tool_result`,
//!   which is what "a tool call is in flight" means.
//!
//! **Only the tail of each file is read.** The reference implementation this
//! was modelled on streams whole transcripts and carries a comment about
//! multi-GB startup peaks and OOMs on large ones; a fixed tail makes the cost
//! of a 400 MB transcript identical to a 4 KB one, and every field above is
//! re-emitted often enough to survive in it. The cost is the *first* prompt of
//! a long session, which `ai-title` covers better anyway.
//!
//! Last-active time comes from the file's modification time rather than the
//! records' ISO timestamps: it answers the same question, cannot disagree with
//! the filesystem ordering the scan already relies on, and needs no date
//! parser. Everything read here is untrusted (rule 13) — bounded, normalized to
//! plain text, never interpreted, never logged.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use serde_json::Value;

use crate::probe::Provider;

/// Longest session title kept.
pub const MAX_TITLE: usize = 200;
/// Longest prompt or reply preview kept.
pub const MAX_PREVIEW: usize = 300;
/// Longest tool-input preview kept.
pub const MAX_TOOL_DETAIL: usize = 160;
/// Longest touched-file path kept (T230).
pub const MAX_TRACKING_PATH: usize = 240;
/// Longest system notice kept (T230).
pub const MAX_NOTICE: usize = 300;

/// How much of an oversized field is even looked at before it is clipped.
///
/// Collapsing whitespace is a streaming transform, so the first `limit`
/// characters of the collapse of a prefix are the first `limit` characters of
/// the collapse of the whole — which means a megabyte of tool output can be
/// bounded without touching a megabyte. Any value comfortably above the
/// largest limit above gives the same answer.
const MAX_CLIP_SOURCE_CHARS: usize = 4 * 1024;

/// A tool call the agent has issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    /// `Bash`, `Edit`, `Read`, …
    pub name: String,
    /// A bounded preview of the input — a command, a path. Never re-executed,
    /// never parsed, only shown.
    pub detail: Option<String>,
}

/// One session, as its transcript describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptEvidence {
    pub provider: Provider,
    pub session_id: String,
    pub working_directory: PathBuf,
    pub transcript_path: PathBuf,
    /// The agent's own title for the session, when it has published one.
    pub title: Option<String>,
    pub last_prompt: Option<String>,
    pub last_reply: Option<String>,
    pub model: Option<String>,
    pub git_branch: Option<String>,
    /// The most recent `tool_use` with no matching `tool_result`.
    pub outstanding_tool: Option<ToolCall>,
    pub last_active_ms: i64,
}

/// Bounds on a scan. Every one of these exists to keep the cost of a poll flat
/// no matter how much history the machine has accumulated.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// `~/.claude/projects`.
    pub root: PathBuf,
    /// Ignore transcripts untouched for longer than this.
    pub max_age: Duration,
    /// Parse at most this many files, newest first.
    pub max_files: usize,
    /// Bytes read from the end of each transcript.
    pub tail_bytes: u64,
}

impl ScanOptions {
    pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
    pub const DEFAULT_MAX_FILES: usize = 40;
    pub const DEFAULT_TAIL_BYTES: u64 = 128 * 1024;

    /// Defaults for a root the caller resolved. The adapter deliberately does
    /// not go looking for the home directory itself: the core already has a
    /// resolver that answers correctly for an app launched without an
    /// environment, and two answers to one question is one too many.
    pub fn rooted_at(root: PathBuf) -> Self {
        Self {
            root,
            max_age: Self::DEFAULT_MAX_AGE,
            max_files: Self::DEFAULT_MAX_FILES,
            tail_bytes: Self::DEFAULT_TAIL_BYTES,
        }
    }
}

/// A transcript file worth parsing, and when it was last written.
struct Candidate {
    path: PathBuf,
    modified_ms: i64,
}

/// Every recent session under `options.root`, newest activity first.
///
/// A missing root, an unreadable directory or an unparseable file are all
/// normal: they cost that one session, never the scan.
pub fn scan(options: &ScanOptions, now_ms: i64) -> Vec<TranscriptEvidence> {
    let cutoff_ms = now_ms.saturating_sub(duration_to_ms(options.max_age));
    let mut candidates = collect_candidates(&options.root, cutoff_ms);
    // Newest first, so the per-root tie-break in the merge and the
    // `max_files` cap both keep the transcripts that matter.
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.modified_ms));
    candidates.truncate(options.max_files);

    candidates
        .into_iter()
        .filter_map(|candidate| {
            parse_transcript(&candidate.path, options.tail_bytes, candidate.modified_ms)
        })
        .collect()
}

/// `root/<project>/<session>.jsonl`, plus any transcript sitting directly in
/// the root. One level of nesting is the layout Claude Code writes; anything
/// deeper is not walked, so a stray deep tree cannot make a scan unbounded.
fn collect_candidates(root: &Path, cutoff_ms: i64) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return candidates;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {
                let Ok(files) = fs::read_dir(&path) else {
                    continue;
                };
                for file in files.flatten() {
                    push_candidate(&mut candidates, file.path(), cutoff_ms);
                }
            }
            Ok(_) => push_candidate(&mut candidates, path, cutoff_ms),
            Err(_) => {}
        }
    }
    candidates
}

fn push_candidate(into: &mut Vec<Candidate>, path: PathBuf, cutoff_ms: i64) {
    if path.extension().is_none_or(|ext| ext != "jsonl") {
        return;
    }
    let Some(modified_ms) = modified_ms(&path) else {
        return;
    };
    if modified_ms >= cutoff_ms {
        into.push(Candidate { path, modified_ms });
    }
}

fn modified_ms(path: &Path) -> Option<i64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    let millis = modified.duration_since(UNIX_EPOCH).ok()?.as_millis();
    i64::try_from(millis).ok()
}

fn duration_to_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// Reads the tail of one transcript and folds it into evidence.
///
/// Returns `None` when the file yields no session id or no working directory:
/// a session listed under a guessed root would be worse than one not listed.
fn parse_transcript(path: &Path, tail_bytes: u64, modified_ms: i64) -> Option<TranscriptEvidence> {
    let tail = read_tail(path, tail_bytes)?;
    let mut fold = Fold::default();
    for line in tail.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // A line that does not parse is expected, not exceptional: the agent
        // is appending to this file while we read it.
        if let Ok(Value::Object(record)) = serde_json::from_str::<Value>(line) {
            fold.absorb(&record);
        }
    }

    let session_id = fold
        .session_id
        .or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
        })
        .filter(|id| !id.is_empty())?;

    Some(TranscriptEvidence {
        provider: Provider::ClaudeCode,
        session_id,
        working_directory: fold.working_directory?,
        transcript_path: path.to_path_buf(),
        title: fold.title,
        last_prompt: fold.last_prompt,
        last_reply: fold.last_reply,
        model: fold.model,
        git_branch: fold.git_branch,
        outstanding_tool: fold.pending_tools.pop().map(|(_, call)| call),
        last_active_ms: modified_ms,
    })
}

/// The last `tail_bytes` of the file, with the leading partial line dropped.
fn read_tail(path: &Path, tail_bytes: u64) -> Option<String> {
    let length = file_len(path)?;
    let offset = length.saturating_sub(tail_bytes);
    let buffer = read_window(path, offset, tail_bytes)?;

    // Lossy on purpose: a transcript half-written at the moment we read it can
    // split a multi-byte character, and one mangled glyph must not cost the
    // whole session its row.
    let text = String::from_utf8_lossy(&buffer).into_owned();
    if offset == 0 {
        return Some(text);
    }
    // Seeking lands mid-record; that record's other half is already gone.
    match text.find('\n') {
        Some(index) => Some(text[index + 1..].to_owned()),
        None => Some(String::new()),
    }
}

/// The file's current length, or `None` when it cannot be read at all.
fn file_len(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|metadata| metadata.len())
}

/// At most `limit` bytes starting at `start`.
///
/// The one place a transcript is read, so the scanner's fixed tail and the
/// follower's incremental window cannot drift apart, and so the "never read a
/// whole file" property has exactly one place to hold.
fn read_window(path: &Path, start: u64, limit: u64) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    if start > 0 {
        file.seek(SeekFrom::Start(start)).ok()?;
    }
    // Capped so a caller's generous limit cannot become a generous allocation
    // on a file that is nowhere near that long; the vector grows if it must.
    let reserve = usize::try_from(limit.min(64 * 1024)).unwrap_or_default();
    let mut buffer = Vec::with_capacity(reserve);
    file.take(limit).read_to_end(&mut buffer).ok()?;
    Some(buffer)
}

/// Accumulates the fields as records stream past. Last writer wins for the
/// latched metadata, which is what "current" means in an append-only log.
#[derive(Default)]
struct Fold {
    session_id: Option<String>,
    working_directory: Option<PathBuf>,
    title: Option<String>,
    last_prompt: Option<String>,
    last_reply: Option<String>,
    model: Option<String>,
    git_branch: Option<String>,
    /// Insertion-ordered so the newest outstanding call is the last entry.
    pending_tools: Vec<(String, ToolCall)>,
}

impl Fold {
    fn absorb(&mut self, record: &serde_json::Map<String, Value>) {
        if let Some(value) = non_empty_str(record.get("sessionId")) {
            self.session_id = Some(value.to_ascii_lowercase());
        }
        if let Some(value) = non_empty_str(record.get("cwd")) {
            self.working_directory = Some(PathBuf::from(value));
        }
        if let Some(value) = non_empty_str(record.get("gitBranch")) {
            self.git_branch = Some(value.to_owned());
        }

        match record.get("type").and_then(Value::as_str) {
            Some("ai-title") => {
                self.title = clip(record.get("aiTitle"), MAX_TITLE);
            }
            Some("last-prompt") => {
                if let Some(prompt) = clip(record.get("lastPrompt"), MAX_PREVIEW) {
                    self.set_prompt(prompt);
                }
            }
            _ => {}
        }

        let Some(message) = record.get("message").and_then(Value::as_object) else {
            return;
        };
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => self.absorb_assistant(message),
            // A meta user record is Gritty-invisible plumbing the agent injects
            // into its own context, not something the user typed.
            Some("user") if record.get("isMeta") != Some(&Value::Bool(true)) => {
                self.absorb_user(message);
            }
            _ => {}
        }
    }

    fn absorb_assistant(&mut self, message: &serde_json::Map<String, Value>) {
        if let Some(model) = non_empty_str(message.get("model")) {
            self.model = Some(model.to_owned());
        }
        let Some(blocks) = message.get("content").and_then(Value::as_array) else {
            return;
        };
        for block in blocks.iter().filter_map(Value::as_object) {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = clip(block.get("text"), MAX_PREVIEW) {
                        self.last_reply = Some(text);
                    }
                }
                Some("tool_use") => {
                    let (Some(id), Some(name)) = (
                        non_empty_str(block.get("id")),
                        non_empty_str(block.get("name")),
                    ) else {
                        continue;
                    };
                    self.pending_tools.retain(|(known, _)| known != id);
                    self.pending_tools.push((
                        id.to_owned(),
                        ToolCall {
                            name: name.to_owned(),
                            detail: tool_detail(block.get("input")),
                        },
                    ));
                }
                _ => {}
            }
        }
    }

    fn set_prompt(&mut self, prompt: String) {
        if !is_command_echo(&prompt) {
            self.last_prompt = Some(prompt);
        }
    }

    fn absorb_user(&mut self, message: &serde_json::Map<String, Value>) {
        let content = message.get("content");
        if let Some(text) = content.and_then(Value::as_str) {
            if let Some(prompt) = clip(Some(&Value::String(text.to_owned())), MAX_PREVIEW) {
                self.set_prompt(prompt);
            }
            return;
        }
        let Some(blocks) = content.and_then(Value::as_array) else {
            return;
        };
        for block in blocks.iter().filter_map(Value::as_object) {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(prompt) = clip(block.get("text"), MAX_PREVIEW) {
                        self.set_prompt(prompt);
                    }
                }
                Some("tool_result") => {
                    if let Some(id) = non_empty_str(block.get("tool_use_id")) {
                        self.pending_tools.retain(|(known, _)| known != id);
                    }
                }
                _ => {}
            }
        }
    }
}

/// A tool's input as one short line: the command or path when there is an
/// obvious one, otherwise the compact JSON.
fn tool_detail(input: Option<&Value>) -> Option<String> {
    let input = input?;
    if let Some(object) = input.as_object() {
        for key in ["command", "file_path", "path", "pattern", "query"] {
            if let Some(value) = clip(object.get(key), MAX_TOOL_DETAIL) {
                return Some(value);
            }
        }
    }
    clip(Some(&Value::String(input.to_string())), MAX_TOOL_DETAIL)
}

fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// A slash command is the user talking to the CLI, not a description of the
/// work; showing `/clear` as "the task" would be actively misleading.
fn is_command_echo(text: &str) -> bool {
    text.starts_with("<command-name>") || text.starts_with("<local-command")
}

/// Collapses whitespace and truncates to `limit` characters. Both matter: the
/// result crosses the protocol boundary and lands in a one-line UI row, so an
/// oversized or newline-bearing field is bounded here rather than in the
/// renderer.
fn clip(value: Option<&Value>, limit: usize) -> Option<String> {
    clip_str(non_empty_str(value)?, limit)
}

/// [`clip`] for a string that is already in hand.
fn clip_str(text: &str, limit: usize) -> Option<String> {
    // Only ever a bounded prefix: a tool result can be megabytes, and
    // collapsing all of it to keep 160 characters is work nobody asked for.
    let source = match text.char_indices().nth(MAX_CLIP_SOURCE_CHARS) {
        Some((end, _)) => &text[..end],
        None => text,
    };
    let collapsed = source.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() <= limit {
        return Some(collapsed);
    }
    let kept: String = collapsed.chars().take(limit.saturating_sub(1)).collect();
    Some(format!("{kept}…"))
}

// ---------------------------------------------------------------------------
// Following one transcript forward (T230)
//
// `scan` above reads a fixed tail of every recent transcript to answer *which*
// agents are running. This answers *what one of them is doing*, and it is the
// same parser: the record vocabulary, the limits and `clip` are shared, and
// only the fold differs. The scanner latches the newest value of each field;
// the follower emits one entry per record it passes.
//
// The property that makes a live feed affordable is the one the scanner
// already bought. A call seeks to an offset and reads only the bytes after it,
// so following a 400 MB transcript costs what following a 4 KB one does.
// ---------------------------------------------------------------------------

/// Bytes read when a follower starts with no offset to resume from.
///
/// Larger than the scanner's window on purpose: the roster needs one line of
/// metadata, a timeline wants the recent conversation.
pub const DEFAULT_COLD_START_BYTES: u64 = 256 * 1024;

/// Largest window a single [`follow`] call reads once it is following.
///
/// A bound on the burst one tick can absorb. Anything older than it is
/// reported as a restart rather than spliced on silently, which is the same
/// rule the ring downstream follows: report the loss, never hide it.
pub const DEFAULT_MAX_READ_BYTES: u64 = 1024 * 1024;

/// Bounds on one [`follow`] call.
#[derive(Debug, Clone, Copy)]
pub struct FollowOptions {
    pub cold_start_bytes: u64,
    pub max_read_bytes: u64,
}

impl Default for FollowOptions {
    fn default() -> Self {
        Self {
            cold_start_bytes: DEFAULT_COLD_START_BYTES,
            max_read_bytes: DEFAULT_MAX_READ_BYTES,
        }
    }
}

/// How much a [`FollowedRecord::Notice`] matters, as the transcript labelled it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// One record the follower recognised, still in the adapter's vocabulary.
///
/// The core maps these onto its own provider-neutral kinds; nothing here
/// crosses the protocol (ADR-005, rule 4). Every string is already bounded by
/// [`clip`], and the core bounds it again at the boundary it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowedRecord {
    UserPrompt {
        text: String,
    },
    AssistantText {
        text: String,
    },
    Thinking {
        text: String,
    },
    ToolCall {
        tool_use_id: Option<String>,
        name: String,
        detail: Option<String>,
    },
    ToolResult {
        tool_use_id: Option<String>,
        ok: bool,
        detail: Option<String>,
    },
    /// A file the agent wrote, exactly as the transcript named it: relative to
    /// the session's working directory when it is inside it, absolute when it
    /// is not. Resolving it is the core's job — this crate does not know which
    /// workspace is open, and must not open anything either way.
    FileTouched {
        tracking_path: String,
    },
    Notice {
        level: NoticeLevel,
        text: String,
    },
}

/// One timeline entry as the transcript described it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowedEntry {
    /// When the record says it was written, in milliseconds since the epoch.
    /// `None` when it carries no parseable timestamp: an invented time would
    /// make a static timeline look like it was moving.
    pub at_ms: Option<i64>,
    /// Subagent work. Marked, never dropped and never merged into the parent's
    /// flow.
    pub sidechain: bool,
    pub record: FollowedRecord,
}

/// What one [`follow`] call read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Followed {
    pub entries: Vec<FollowedEntry>,
    /// Where the next call resumes. Never inside a partial final line: the
    /// agent is appending while we read, and half a record must be parsed on
    /// the next tick rather than dropped.
    pub next_offset: u64,
    /// The file was truncated, replaced, or had grown by more than one call
    /// may read: what follows does not continue what came before, so a
    /// consumer must reset its view instead of appending to it.
    pub restarted: bool,
    /// Whether the transcript could be read at all. A session may be process
    /// evidence only, and a transcript may be deleted while it is followed;
    /// neither is an error, and neither is "the agent did nothing".
    pub readable: bool,
}

/// Parse the records appended since `from_offset`.
///
/// A first call with offset `0` on a large file does **not** read the whole
/// file: it starts from a bounded tail and reports the offset it actually
/// started at, exactly as the scanner's fixed tail already does.
pub fn follow(path: &Path, from_offset: u64) -> Followed {
    follow_with(path, from_offset, FollowOptions::default())
}

/// [`follow`] with explicit bounds, which is what the core injects and what
/// makes the bounded-window behaviour testable without a 400 MB fixture.
pub fn follow_with(path: &Path, from_offset: u64, options: FollowOptions) -> Followed {
    let unreadable = || Followed {
        entries: Vec::new(),
        next_offset: from_offset,
        restarted: false,
        readable: false,
    };
    let Some(length) = file_len(path) else {
        return unreadable();
    };

    // Shorter than where we stopped: the file was truncated or replaced.
    // Splicing what is there now onto what was already published would invent
    // a conversation that never happened.
    let replaced = from_offset > length;
    let mut start = if replaced { 0 } else { from_offset };
    let cold = start == 0;
    let window = if cold {
        options.cold_start_bytes
    } else {
        options.max_read_bytes
    }
    .max(1);

    // More has arrived than one call may read. On a cold start that is the
    // bounded tail working as designed; mid-follow it is a gap in the feed.
    let skipped = length.saturating_sub(start) > window;
    if skipped {
        start = length - window;
    }
    // `start` sits on a record boundary when it is where we stopped, or the
    // beginning of the file. Anywhere else, the seek landed mid-record.
    let aligned = start == from_offset || start == 0;

    let Some(bytes) = read_window(path, start, window) else {
        return unreadable();
    };
    let restarted = replaced || (skipped && !cold);

    // Stopping at the last newline leaves a half-written final record for the
    // next call instead of parsing — and losing — half of it.
    let Some(last_newline) = bytes.iter().rposition(|byte| *byte == b'\n') else {
        return Followed {
            entries: Vec::new(),
            next_offset: start,
            restarted,
            readable: true,
        };
    };
    let consumed = u64::try_from(last_newline)
        .unwrap_or(u64::MAX)
        .saturating_add(1);

    let mut entries = Vec::new();
    let mut lines = bytes[..last_newline].split(|byte| *byte == b'\n');
    if !aligned {
        // Whatever the seek cut in half; its other end is already behind us.
        lines.next();
    }
    for line in lines {
        // Lossy decoding costs a glyph rather than the record around it, and a
        // line that does not parse is expected rather than exceptional: the
        // agent is appending to this file while we read it.
        let text = String::from_utf8_lossy(line);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(Value::Object(record)) = serde_json::from_str::<Value>(trimmed) {
            absorb_entries(&record, &mut entries);
        }
    }

    Followed {
        entries,
        next_offset: start.saturating_add(consumed),
        restarted,
        readable: true,
    }
}

/// One record to zero or more entries. The vocabulary is the same one the
/// scanner's [`Fold`] reads; only what is done with it differs.
fn absorb_entries(record: &serde_json::Map<String, Value>, into: &mut Vec<FollowedEntry>) {
    let at_ms = record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp_ms);
    let sidechain = record.get("isSidechain") == Some(&Value::Bool(true));
    let entry = |record| FollowedEntry {
        at_ms,
        sidechain,
        record,
    };

    // A hook that failed is attached to whatever record was being written, so
    // it is looked for before the type is dispatched on.
    if let Some(text) = hook_errors(record.get("hookErrors")) {
        into.push(entry(FollowedRecord::Notice {
            level: NoticeLevel::Error,
            text,
        }));
    }

    match record.get("type").and_then(Value::as_str) {
        Some("file-history-delta") => {
            if let Some(tracking_path) = clip(record.get("trackingPath"), MAX_TRACKING_PATH) {
                into.push(entry(FollowedRecord::FileTouched { tracking_path }));
            }
            return;
        }
        Some("system") => {
            let level = match record.get("level").and_then(Value::as_str) {
                Some("error") => NoticeLevel::Error,
                Some("warn" | "warning") => NoticeLevel::Warning,
                _ => NoticeLevel::Info,
            };
            if let Some(text) = clip(record.get("content"), MAX_NOTICE)
                && !is_command_echo(&text)
            {
                into.push(entry(FollowedRecord::Notice { level, text }));
            }
            return;
        }
        // Session fields, not events in the session: `ai-title` and
        // `last-prompt` restate what the roster row already shows, the modes
        // are configuration, and the snapshot is the agent's own bookkeeping.
        // Named so they are skipped deliberately rather than by falling
        // through into the message arm below.
        Some(
            "ai-title"
            | "last-prompt"
            | "mode"
            | "permission-mode"
            | "file-history-snapshot"
            | "summary",
        ) => return,
        _ => {}
    }

    let Some(message) = record.get("message").and_then(Value::as_object) else {
        return;
    };
    match message.get("role").and_then(Value::as_str) {
        Some("assistant") => absorb_assistant_entries(message, &entry, into),
        // A meta user record is Gritty-invisible plumbing the agent injects
        // into its own context, not something the user typed.
        Some("user") if record.get("isMeta") != Some(&Value::Bool(true)) => {
            absorb_user_entries(message, &entry, into);
        }
        _ => {}
    }
}

fn absorb_assistant_entries(
    message: &serde_json::Map<String, Value>,
    entry: &dyn Fn(FollowedRecord) -> FollowedEntry,
    into: &mut Vec<FollowedEntry>,
) {
    let Some(blocks) = message.get("content").and_then(Value::as_array) else {
        return;
    };
    for block in blocks.iter().filter_map(Value::as_object) {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = clip(block.get("text"), MAX_PREVIEW) {
                    into.push(entry(FollowedRecord::AssistantText { text }));
                }
            }
            Some("thinking") => {
                if let Some(text) = clip(block.get("thinking"), MAX_PREVIEW) {
                    into.push(entry(FollowedRecord::Thinking { text }));
                }
            }
            Some("tool_use") => {
                let Some(name) = non_empty_str(block.get("name")) else {
                    continue;
                };
                into.push(entry(FollowedRecord::ToolCall {
                    tool_use_id: non_empty_str(block.get("id")).map(str::to_owned),
                    name: name.to_owned(),
                    detail: tool_detail(block.get("input")),
                }));
            }
            _ => {}
        }
    }
}

fn absorb_user_entries(
    message: &serde_json::Map<String, Value>,
    entry: &dyn Fn(FollowedRecord) -> FollowedEntry,
    into: &mut Vec<FollowedEntry>,
) {
    let content = message.get("content");
    if let Some(text) = clip(content, MAX_PREVIEW) {
        if !is_command_echo(&text) {
            into.push(entry(FollowedRecord::UserPrompt { text }));
        }
        return;
    }
    let Some(blocks) = content.and_then(Value::as_array) else {
        return;
    };
    for block in blocks.iter().filter_map(Value::as_object) {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = clip(block.get("text"), MAX_PREVIEW)
                    && !is_command_echo(&text)
                {
                    into.push(entry(FollowedRecord::UserPrompt { text }));
                }
            }
            Some("tool_result") => {
                into.push(entry(FollowedRecord::ToolResult {
                    tool_use_id: non_empty_str(block.get("tool_use_id")).map(str::to_owned),
                    ok: block.get("is_error") != Some(&Value::Bool(true)),
                    detail: result_detail(block),
                }));
            }
            _ => {}
        }
    }
}

/// A tool result's output as one short line. The content is either a string or
/// a list of blocks, and either way only a bounded preview is kept: a result
/// is routinely a whole file.
fn result_detail(block: &serde_json::Map<String, Value>) -> Option<String> {
    let content = block.get("content")?;
    if let Some(text) = clip(Some(content), MAX_TOOL_DETAIL) {
        return Some(text);
    }
    content
        .as_array()?
        .iter()
        .filter_map(Value::as_object)
        .find_map(|inner| clip(inner.get("text"), MAX_TOOL_DETAIL))
}

/// Hook failures, which arrive as a list of strings or as one string.
fn hook_errors(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(text) = clip(Some(value), MAX_NOTICE) {
        return Some(text);
    }
    let joined = value
        .as_array()?
        .iter()
        .filter_map(|item| non_empty_str(Some(item)))
        .collect::<Vec<_>>()
        .join(" ");
    clip_str(&joined, MAX_NOTICE)
}

/// `2026-09-05T13:49:47.770Z` to milliseconds since the epoch.
///
/// Strict on purpose: the one shape these transcripts write, UTC only.
/// Anything else yields `None`, so an entry carries no time rather than a
/// wrong one — and the crate keeps needing no date/time dependency for one
/// fixed format (dependency policy).
fn parse_timestamp_ms(raw: &str) -> Option<i64> {
    let raw = raw.strip_suffix('Z')?;
    let (date, rest) = raw.split_once('T')?;
    let (time, fraction) = match rest.split_once('.') {
        Some((time, fraction)) => (time, Some(fraction)),
        None => (rest, None),
    };

    let mut date_parts = date.split('-');
    let year = parse_field(date_parts.next())?;
    let month = parse_field(date_parts.next())?;
    let day = parse_field(date_parts.next())?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let mut time_parts = time.split(':');
    let hour = parse_field(time_parts.next())?;
    let minute = parse_field(time_parts.next())?;
    let second = parse_field(time_parts.next())?;
    // 60 is a leap second, which some writers do emit.
    if time_parts.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let millis = match fraction {
        None => 0,
        Some(digits) if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) => {
            let mut millis = 0;
            for index in 0..3 {
                let digit = digits
                    .as_bytes()
                    .get(index)
                    .map_or(0, |byte| i64::from(byte - b'0'));
                millis = millis * 10 + digit;
            }
            millis
        }
        Some(_) => return None,
    };

    let seconds = days_from_civil(year, month, day)
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)?;
    seconds.checked_mul(1_000)?.checked_add(millis)
}

fn parse_field(raw: Option<&str>) -> Option<i64> {
    let raw = raw?;
    (!raw.is_empty() && raw.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| raw.parse().ok())
        .flatten()
}

/// Days between 1970-01-01 and a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`). Twelve lines of arithmetic instead of a calendar crate.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// A scratch directory that cleans itself up.
    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "gritty-agents-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp root");
            Self(path)
        }

        fn write(&self, relative: &str, contents: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
            let mut file = File::create(&path).expect("create file");
            file.write_all(contents.as_bytes()).expect("write file");
            path
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const SESSION: &str = "eca001ac-5b6f-40ff-b79a-9ae79ccc6483";

    /// Shapes taken from a real transcript.
    fn transcript_lines() -> String {
        [
            format!(r#"{{"type":"mode","mode":"normal","sessionId":"{SESSION}"}}"#),
            format!(
                r#"{{"type":"user","cwd":"/Users/a/p","sessionId":"{SESSION}","gitBranch":"main","message":{{"role":"user","content":[{{"type":"text","text":"rename  the\nterminal"}}]}}}}"#
            ),
            format!(
                r#"{{"type":"assistant","cwd":"/Users/a/p","sessionId":"{SESSION}","message":{{"role":"assistant","model":"claude-opus-5","content":[{{"type":"text","text":"On it."}},{{"type":"tool_use","id":"toolu_1","name":"Bash","input":{{"command":"cargo test","description":"run tests"}}}}]}}}}"#
            ),
            format!(r#"{{"type":"ai-title","aiTitle":"IDE terminal implementation","sessionId":"{SESSION}"}}"#),
        ]
        .join("\n")
    }

    fn parse(contents: &str) -> Option<TranscriptEvidence> {
        let root = TempRoot::new("parse");
        let path = root.write(&format!("-Users-a-p/{SESSION}.jsonl"), contents);
        parse_transcript(&path, ScanOptions::DEFAULT_TAIL_BYTES, 1_700_000_000_000)
    }

    #[test]
    fn reads_the_task_the_model_and_the_branch() {
        let found = parse(&transcript_lines()).expect("evidence");
        assert_eq!(found.session_id, SESSION);
        assert_eq!(found.working_directory, PathBuf::from("/Users/a/p"));
        assert_eq!(found.title.as_deref(), Some("IDE terminal implementation"));
        assert_eq!(found.model.as_deref(), Some("claude-opus-5"));
        assert_eq!(found.git_branch.as_deref(), Some("main"));
        assert_eq!(found.last_reply.as_deref(), Some("On it."));
        // Whitespace, including the newline, is collapsed before it crosses out.
        assert_eq!(found.last_prompt.as_deref(), Some("rename the terminal"));
    }

    #[test]
    fn a_tool_call_is_outstanding_until_its_result_arrives() {
        let opened = parse(&transcript_lines()).expect("evidence");
        let call = opened.outstanding_tool.expect("outstanding tool");
        assert_eq!(call.name, "Bash");
        assert_eq!(call.detail.as_deref(), Some("cargo test"));

        let closed = parse(&format!(
            "{}\n{}",
            transcript_lines(),
            format_args!(
                r#"{{"type":"user","cwd":"/Users/a/p","sessionId":"{SESSION}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_1"}}]}}}}"#
            )
        ))
        .expect("evidence");
        assert_eq!(closed.outstanding_tool, None);
    }

    #[test]
    fn the_newest_outstanding_call_wins() {
        let extra = format!(
            r#"{{"type":"assistant","cwd":"/Users/a/p","sessionId":"{SESSION}","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"toolu_2","name":"Edit","input":{{"file_path":"/Users/a/p/main.rs"}}}}]}}}}"#
        );
        let found = parse(&format!("{}\n{extra}", transcript_lines())).expect("evidence");
        let call = found.outstanding_tool.expect("outstanding tool");
        assert_eq!(call.name, "Edit");
        assert_eq!(call.detail.as_deref(), Some("/Users/a/p/main.rs"));
    }

    #[test]
    fn the_prompt_record_is_preferred_over_message_text() {
        let contents = format!(
            "{}\n{}",
            transcript_lines(),
            format_args!(
                r#"{{"type":"last-prompt","lastPrompt":"ship the island","sessionId":"{SESSION}"}}"#
            )
        );
        let found = parse(&contents).expect("evidence");
        assert_eq!(found.last_prompt.as_deref(), Some("ship the island"));
    }

    #[test]
    fn meta_user_records_are_not_prompts() {
        let contents = format!(
            "{}\n{}",
            transcript_lines(),
            format_args!(
                r#"{{"type":"user","isMeta":true,"cwd":"/Users/a/p","sessionId":"{SESSION}","message":{{"role":"user","content":[{{"type":"text","text":"injected plumbing"}}]}}}}"#
            )
        );
        let found = parse(&contents).expect("evidence");
        assert_eq!(found.last_prompt.as_deref(), Some("rename the terminal"));
    }

    #[test]
    fn malformed_and_truncated_lines_are_skipped_not_fatal() {
        let contents = format!(
            "not json at all\n{}\n{{\"type\":\"assistant\",\"message\":{{\"role\"",
            transcript_lines()
        );
        let found = parse(&contents).expect("evidence despite the damage");
        assert_eq!(found.title.as_deref(), Some("IDE terminal implementation"));
    }

    #[test]
    fn a_transcript_without_a_working_directory_is_skipped() {
        let contents = format!(r#"{{"type":"mode","mode":"normal","sessionId":"{SESSION}"}}"#);
        assert_eq!(parse(&contents), None);
    }

    #[test]
    fn invalid_utf8_costs_a_glyph_not_the_session() {
        let root = TempRoot::new("utf8");
        let path = root.write(&format!("-Users-a-p/{SESSION}.jsonl"), "");
        let mut file = File::create(&path).expect("create");
        file.write_all(&[0xff, 0xfe, b'\n']).expect("write bytes");
        file.write_all(transcript_lines().as_bytes())
            .expect("write lines");

        let found = parse_transcript(&path, ScanOptions::DEFAULT_TAIL_BYTES, 0).expect("evidence");
        assert_eq!(found.title.as_deref(), Some("IDE terminal implementation"));
    }

    #[test]
    fn only_the_tail_is_read_so_size_does_not_matter() {
        let root = TempRoot::new("tail");
        let filler = format!(
            "{}\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"old"}]}}"#
        )
        .repeat(4_000);
        let path = root.write(
            &format!("-Users-a-p/{SESSION}.jsonl"),
            &format!("{filler}{}", transcript_lines()),
        );
        assert!(
            fs::metadata(&path).expect("metadata").len() > 256 * 1024,
            "fixture must exceed the tail window"
        );

        let found = parse_transcript(&path, ScanOptions::DEFAULT_TAIL_BYTES, 0).expect("evidence");
        // The filler's replies are outside the window; the tail's survive.
        assert_eq!(found.last_reply.as_deref(), Some("On it."));
        assert_eq!(found.title.as_deref(), Some("IDE terminal implementation"));
    }

    #[test]
    fn the_partial_first_line_of_a_tail_is_dropped() {
        let root = TempRoot::new("partial");
        let path = root.write(
            &format!("-Users-a-p/{SESSION}.jsonl"),
            &format!("GARBAGE-HALF-RECORD\n{}", transcript_lines()),
        );
        // A window small enough to land inside the first line.
        let found = parse_transcript(&path, 400, 0);
        assert!(
            found.is_none_or(|evidence| evidence.working_directory == Path::new("/Users/a/p")),
            "a mid-record seek never yields a bogus root"
        );
    }

    #[test]
    fn scan_orders_by_recency_and_honors_the_cutoff() {
        let root = TempRoot::new("scan");
        root.write(&format!("-Users-a-p/{SESSION}.jsonl"), &transcript_lines());
        root.write(
            "-Users-a-q/11111111-2222-3333-4444-555555555555.jsonl",
            &transcript_lines(),
        );
        root.write("-Users-a-p/notes.txt", "ignored");

        let options = ScanOptions::rooted_at(root.0.clone());
        let now_ms = modified_ms(&root.0.join(format!("-Users-a-p/{SESSION}.jsonl")))
            .expect("mtime")
            + 1_000;

        let found = scan(&options, now_ms);
        assert_eq!(found.len(), 2, "both transcripts, no .txt");
        assert!(found[0].last_active_ms >= found[1].last_active_ms);

        // Everything is older than a zero-length window.
        let stale = ScanOptions {
            max_age: Duration::from_millis(0),
            ..options.clone()
        };
        assert!(scan(&stale, now_ms).is_empty());

        let capped = ScanOptions {
            max_files: 1,
            ..options
        };
        assert_eq!(scan(&capped, now_ms).len(), 1);
    }

    #[test]
    fn a_missing_root_is_empty_not_an_error() {
        let options = ScanOptions::rooted_at(PathBuf::from("/nonexistent/gritty/agents"));
        assert!(scan(&options, 1_700_000_000_000).is_empty());
    }

    #[test]
    fn clip_collapses_whitespace_and_bounds_length() {
        let value = Value::String("  a \n b\tc  ".to_owned());
        assert_eq!(clip(Some(&value), 100).as_deref(), Some("a b c"));

        let long = Value::String("x".repeat(500));
        let clipped = clip(Some(&long), 10).expect("clipped");
        assert_eq!(clipped.chars().count(), 10);
        assert!(clipped.ends_with('…'));

        assert_eq!(clip(Some(&Value::String("   ".to_owned())), 10), None);
        assert_eq!(clip(None, 10), None);
    }

    // ------------------------------------------------------------- follower

    /// The record shapes the follower has to recognise, in the order a real
    /// session writes them.
    fn timeline_lines() -> String {
        [
            format!(
                r#"{{"type":"user","timestamp":"2026-09-05T13:49:47.770Z","cwd":"/Users/a/p","sessionId":"{SESSION}","message":{{"role":"user","content":[{{"type":"text","text":"ship  the\nisland"}}]}}}}"#
            ),
            format!(
                r#"{{"type":"assistant","timestamp":"2026-09-05T13:49:48.000Z","cwd":"/Users/a/p","sessionId":"{SESSION}","message":{{"role":"assistant","model":"claude-opus-5","content":[{{"type":"thinking","thinking":"weighing it up","signature":"x"}},{{"type":"text","text":"On it."}},{{"type":"tool_use","id":"toolu_1","name":"Bash","input":{{"command":"cargo test"}}}}]}}}}"#
            ),
            format!(
                r#"{{"type":"user","timestamp":"2026-09-05T13:49:49.000Z","cwd":"/Users/a/p","sessionId":"{SESSION}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_1","content":"ok, 41 passed"}}]}},"toolUseResult":{{"stdout":"…"}}}}"#
            ),
            format!(
                r#"{{"type":"file-history-delta","timestamp":"2026-09-05T13:49:50.000Z","sessionId":"{SESSION}","trackingPath":"crates/agents/src/transcript.rs","backup":{{"version":1}}}}"#
            ),
            format!(
                r#"{{"type":"system","subtype":"hook","level":"error","timestamp":"2026-09-05T13:49:51.000Z","sessionId":"{SESSION}","content":"a hook refused the edit"}}"#
            ),
            format!(
                r#"{{"type":"assistant","isSidechain":true,"timestamp":"2026-09-05T13:49:52.000Z","sessionId":"{SESSION}","message":{{"role":"assistant","content":[{{"type":"text","text":"subagent speaking"}}]}}}}"#
            ),
            format!(r#"{{"type":"ai-title","aiTitle":"the rail","sessionId":"{SESSION}"}}"#),
        ]
        .join("\n")
            + "\n"
    }

    /// One assistant reply as a whole transcript line, newline included.
    fn assistant_line(text: &str) -> String {
        format!(
            "{}\n",
            format_args!(
                r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"{text}"}}]}}}}"#
            )
        )
    }

    fn followed(contents: &str) -> Followed {
        let root = TempRoot::new("follow");
        let path = root.write(&format!("-Users-a-p/{SESSION}.jsonl"), contents);
        follow(&path, 0)
    }

    #[test]
    fn every_record_type_becomes_its_own_entry() {
        let read = followed(&timeline_lines());
        assert!(read.readable);
        assert!(!read.restarted);
        assert_eq!(
            read.next_offset,
            timeline_lines().len() as u64,
            "a file ending in a newline is fully consumed"
        );

        let kinds: Vec<&FollowedRecord> = read.entries.iter().map(|e| &e.record).collect();
        assert!(matches!(
            kinds[0],
            FollowedRecord::UserPrompt { text } if text == "ship the island"
        ));
        assert!(matches!(
            kinds[1],
            FollowedRecord::Thinking { text } if text == "weighing it up"
        ));
        assert!(matches!(
            kinds[2],
            FollowedRecord::AssistantText { text } if text == "On it."
        ));
        assert!(matches!(
            kinds[3],
            FollowedRecord::ToolCall { name, detail, tool_use_id }
                if name == "Bash"
                    && detail.as_deref() == Some("cargo test")
                    && tool_use_id.as_deref() == Some("toolu_1")
        ));
        assert!(matches!(
            kinds[4],
            FollowedRecord::ToolResult { tool_use_id, ok, detail }
                if *ok
                    && tool_use_id.as_deref() == Some("toolu_1")
                    && detail.as_deref() == Some("ok, 41 passed")
        ));
        assert!(matches!(
            kinds[5],
            FollowedRecord::FileTouched { tracking_path }
                if tracking_path == "crates/agents/src/transcript.rs"
        ));
        assert!(matches!(
            kinds[6],
            FollowedRecord::Notice { level: NoticeLevel::Error, text }
                if text == "a hook refused the edit"
        ));
        assert!(matches!(kinds[7], FollowedRecord::AssistantText { .. }));
        assert_eq!(
            kinds.len(),
            8,
            "the metadata records carry no entry of their own"
        );
    }

    #[test]
    fn a_tool_call_and_its_result_share_an_id_so_the_ui_can_pair_them() {
        let read = followed(&timeline_lines());
        let call = read.entries.iter().find_map(|entry| match &entry.record {
            FollowedRecord::ToolCall { tool_use_id, .. } => tool_use_id.clone(),
            _ => None,
        });
        let result = read.entries.iter().find_map(|entry| match &entry.record {
            FollowedRecord::ToolResult { tool_use_id, .. } => tool_use_id.clone(),
            _ => None,
        });
        assert_eq!(call, result);
        assert!(call.is_some());
    }

    #[test]
    fn subagent_records_are_marked_rather_than_dropped_or_merged() {
        let read = followed(&timeline_lines());
        let sidechained: Vec<bool> = read.entries.iter().map(|entry| entry.sidechain).collect();
        assert_eq!(sidechained.iter().filter(|flag| **flag).count(), 1);
        assert!(
            sidechained.last().copied().unwrap_or_default(),
            "the subagent's line is the marked one"
        );
    }

    #[test]
    fn a_failed_tool_result_says_so() {
        let read = followed(concat!(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_9","is_error":true,"content":[{"type":"text","text":"exit 1"}]}]}}"#,
            "\n"
        ));
        assert!(matches!(
            &read.entries[0].record,
            FollowedRecord::ToolResult { ok: false, detail, .. } if detail.as_deref() == Some("exit 1")
        ));
    }

    #[test]
    fn timestamps_come_from_the_record_and_are_never_invented() {
        let read = followed(&timeline_lines());
        // 2026-09-05T13:49:47.770Z
        assert_eq!(read.entries[0].at_ms, Some(1_788_616_187_770));
        assert!(
            read.entries
                .windows(2)
                .all(|pair| pair[0].at_ms <= pair[1].at_ms),
            "entries arrive in the order they were written"
        );

        let undated = followed(
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"no clock\"}]}}\n",
        );
        assert_eq!(undated.entries[0].at_ms, None);
    }

    #[test]
    fn a_following_call_reads_only_what_was_appended() {
        let root = TempRoot::new("append");
        let name = format!("-Users-a-p/{SESSION}.jsonl");
        let first = timeline_lines();
        let path = root.write(&name, &first);

        let opened = follow(&path, 0);
        assert_eq!(opened.next_offset, first.len() as u64);

        let more = assistant_line("later");
        root.write(&name, &format!("{first}{more}"));

        let next = follow(&path, opened.next_offset);
        assert_eq!(next.entries.len(), 1, "only the appended record");
        assert!(matches!(
            &next.entries[0].record,
            FollowedRecord::AssistantText { text } if text == "later"
        ));
        assert_eq!(next.next_offset, (first.len() + more.len()) as u64);
        assert!(!next.restarted);

        // Nothing new: nothing read, and the offset does not move.
        let quiet = follow(&path, next.next_offset);
        assert!(quiet.entries.is_empty());
        assert_eq!(quiet.next_offset, next.next_offset);
    }

    #[test]
    fn a_partial_final_line_is_held_back_for_the_next_call() {
        let root = TempRoot::new("partial-follow");
        let name = format!("-Users-a-p/{SESSION}.jsonl");
        let complete = assistant_line("first");
        let half = r#"{"type":"assistant","message":{"role":"assis"#;
        let path = root.write(&name, &format!("{complete}{half}"));

        let read = follow(&path, 0);
        assert_eq!(read.entries.len(), 1, "the half record is not parsed");
        assert_eq!(
            read.next_offset,
            complete.len() as u64,
            "and its bytes are not consumed"
        );

        // The agent finishes writing it.
        let rest = "tant\",\"content\":[{\"type\":\"text\",\"text\":\"second\"}]}}\n";
        root.write(&name, &format!("{complete}{half}{rest}"));
        let read = follow(&path, read.next_offset);
        assert!(matches!(
            &read.entries[0].record,
            FollowedRecord::AssistantText { text } if text == "second"
        ));
    }

    #[test]
    fn a_file_shorter_than_the_offset_restarts_instead_of_splicing() {
        let root = TempRoot::new("truncate");
        let name = format!("-Users-a-p/{SESSION}.jsonl");
        let path = root.write(&name, &timeline_lines());
        let opened = follow(&path, 0);
        assert!(!opened.restarted);

        // A new session reused the file, or the user truncated it.
        let replacement = assistant_line("a different file");
        root.write(&name, &replacement);

        let read = follow(&path, opened.next_offset);
        assert!(read.restarted, "the view was reset, not continued");
        assert_eq!(read.entries.len(), 1);
        assert_eq!(read.next_offset, replacement.len() as u64);
    }

    #[test]
    fn a_cold_open_on_a_huge_transcript_reads_a_bounded_tail() {
        let root = TempRoot::new("huge");
        let filler = format!(
            "{}\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"old"}]}}"#
        )
        .repeat(20_000);
        let path = root.write(
            &format!("-Users-a-p/{SESSION}.jsonl"),
            &format!("{filler}{}", timeline_lines()),
        );
        let length = fs::metadata(&path).expect("metadata").len();
        assert!(
            length > 2 * DEFAULT_COLD_START_BYTES,
            "fixture must exceed the window"
        );

        let read = follow(&path, 0);
        assert!(
            read.next_offset > length - DEFAULT_COLD_START_BYTES,
            "the follower started near the end, not at the beginning"
        );
        assert_eq!(read.next_offset, length, "and consumed to the last newline");
        assert!(
            !read.restarted,
            "a bounded cold start is the design, not a reset"
        );
        assert!(
            read.entries.len() < 20_000,
            "the history before the window was never read"
        );
        assert!(
            read.entries
                .iter()
                .any(|entry| matches!(&entry.record, FollowedRecord::FileTouched { .. })),
            "the newest records survive"
        );
    }

    #[test]
    fn a_burst_larger_than_one_call_may_read_is_reported_as_a_gap() {
        let root = TempRoot::new("burst");
        let line = format!(
            "{}\n",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"burst"}]}}"#
        );
        let path = root.write(&format!("-Users-a-p/{SESSION}.jsonl"), &line.repeat(400));
        let options = FollowOptions {
            cold_start_bytes: 64,
            max_read_bytes: 512,
        };

        // Mid-follow, with far more than `max_read_bytes` waiting.
        let read = follow_with(&path, 1, options);
        assert!(read.restarted, "a gap is a reset, not a silent splice");
        assert!(!read.entries.is_empty());
        assert!(
            read.entries.len() < 400,
            "only the tail of the burst was read"
        );
    }

    #[test]
    fn a_malformed_line_costs_itself_and_nothing_else() {
        let contents = format!(
            "not json at all\n{}{}\n",
            timeline_lines(),
            r#"{"type":"assistant","message":{"role":"#
        );
        let read = followed(&contents);
        assert!(
            read.entries
                .iter()
                .any(|entry| matches!(&entry.record, FollowedRecord::FileTouched { .. })),
            "every other record still parses"
        );
    }

    #[test]
    fn invalid_utf8_costs_a_glyph_not_the_batch() {
        let root = TempRoot::new("follow-utf8");
        let path = root.write(&format!("-Users-a-p/{SESSION}.jsonl"), "");
        let mut file = File::create(&path).expect("create");
        file.write_all(&[0xff, 0xfe, b'\n']).expect("write bytes");
        file.write_all(timeline_lines().as_bytes())
            .expect("write lines");

        let read = follow(&path, 0);
        assert!(read.readable);
        assert!(
            read.entries
                .iter()
                .any(|entry| matches!(&entry.record, FollowedRecord::FileTouched { .. }))
        );
    }

    #[test]
    fn a_transcript_that_cannot_be_read_is_not_an_error() {
        let read = follow(Path::new("/nonexistent/gritty/session.jsonl"), 128);
        assert!(!read.readable);
        assert!(read.entries.is_empty());
        assert_eq!(read.next_offset, 128, "the offset is left where it was");
        assert!(!read.restarted);
    }

    #[test]
    fn an_empty_transcript_yields_nothing_and_stays_at_zero() {
        let root = TempRoot::new("empty");
        let path = root.write(&format!("-Users-a-p/{SESSION}.jsonl"), "");
        let read = follow(&path, 0);
        assert!(read.readable);
        assert!(read.entries.is_empty());
        assert_eq!(read.next_offset, 0);
    }

    #[test]
    fn oversized_fields_are_bounded_before_they_leave_the_adapter() {
        let long = "x".repeat(5_000);
        let read = followed(&format!(
            "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"{long}\"}},{{\"type\":\"tool_use\",\"id\":\"t\",\"name\":\"Bash\",\"input\":{{\"command\":\"{long}\"}}}}]}}}}\n"
        ));
        let FollowedRecord::AssistantText { text } = &read.entries[0].record else {
            panic!("assistant text");
        };
        assert_eq!(text.chars().count(), MAX_PREVIEW);
        let FollowedRecord::ToolCall { detail, .. } = &read.entries[1].record else {
            panic!("tool call");
        };
        assert_eq!(
            detail.as_deref().map(|d| d.chars().count()),
            Some(MAX_TOOL_DETAIL)
        );
    }

    #[test]
    fn slash_commands_are_not_reported_as_prompts() {
        let read = followed(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/clear</command-name>\"}}\n",
        );
        assert!(read.entries.is_empty());
    }

    #[test]
    fn timestamps_parse_only_the_shape_the_agents_write() {
        assert_eq!(parse_timestamp_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_timestamp_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_timestamp_ms("2026-09-05T13:49:47.770Z"),
            Some(1_788_616_187_770)
        );
        // A leap day, which a naive month table gets wrong.
        assert_eq!(
            parse_timestamp_ms("2024-02-29T00:00:00Z"),
            Some(1_709_164_800_000)
        );
        // Sub-millisecond precision is truncated, not rejected.
        assert_eq!(
            parse_timestamp_ms("2026-09-05T13:49:47.770999Z"),
            Some(1_788_616_187_770)
        );

        for rejected in [
            "2026-09-05T13:49:47.770+02:00",
            "2026-09-05 13:49:47Z",
            "2026-13-05T00:00:00Z",
            "2026-09-05T24:00:00Z",
            "not a date",
            "",
        ] {
            assert_eq!(parse_timestamp_ms(rejected), None, "{rejected}");
        }
    }

    #[test]
    fn tool_detail_prefers_a_readable_field() {
        let input = serde_json::json!({ "command": "cargo test", "description": "run" });
        assert_eq!(tool_detail(Some(&input)).as_deref(), Some("cargo test"));

        let opaque = serde_json::json!({ "unknown": 1 });
        assert!(tool_detail(Some(&opaque)).is_some_and(|text| text.contains("unknown")));
        assert_eq!(tool_detail(None), None);
    }
}
