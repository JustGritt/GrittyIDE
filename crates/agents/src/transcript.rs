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
const MAX_TITLE: usize = 200;
/// Longest prompt or reply preview kept.
const MAX_PREVIEW: usize = 300;
/// Longest tool-input preview kept.
const MAX_TOOL_DETAIL: usize = 160;

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
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let offset = length.saturating_sub(tail_bytes);
    if offset > 0 {
        file.seek(SeekFrom::Start(offset)).ok()?;
    }

    let mut buffer =
        Vec::with_capacity(usize::try_from(length.min(tail_bytes)).unwrap_or_default());
    file.take(tail_bytes).read_to_end(&mut buffer).ok()?;

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

    /// A slash command is the user talking to the CLI, not a description of
    /// the work; showing `/clear` as "the task" would be actively misleading.
    fn set_prompt(&mut self, prompt: String) {
        if !prompt.starts_with("<command-name>") && !prompt.starts_with("<local-command") {
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

/// Collapses whitespace and truncates to `limit` characters. Both matter: the
/// result crosses the protocol boundary and lands in a one-line UI row, so an
/// oversized or newline-bearing field is bounded here rather than in the
/// renderer.
fn clip(value: Option<&Value>, limit: usize) -> Option<String> {
    let text = non_empty_str(value)?;
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    if collapsed.chars().count() <= limit {
        return Some(collapsed);
    }
    let kept: String = collapsed.chars().take(limit.saturating_sub(1)).collect();
    Some(format!("{kept}…"))
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

    #[test]
    fn tool_detail_prefers_a_readable_field() {
        let input = serde_json::json!({ "command": "cargo test", "description": "run" });
        assert_eq!(tool_detail(Some(&input)).as_deref(), Some("cargo test"));

        let opaque = serde_json::json!({ "unknown": 1 });
        assert!(tool_detail(Some(&opaque)).is_some_and(|text| text.contains("unknown")));
        assert_eq!(tool_detail(None), None);
    }
}
