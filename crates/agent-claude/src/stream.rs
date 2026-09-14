//! Bytes in, [`ClaudeObservation`]s out — with no process, no filesystem and
//! no clock.
//!
//! This is the half of the adapter a **recorded fixture** proves. Everything
//! that needs a child process lives in [`crate::process`]; everything that
//! decides what the child *said* lives here, and takes a `&[u8]` so a test can
//! feed it `fixtures/two-turns-denied.jsonl` and get the same answer CI gets
//! with neither `claude` nor an API key present. That separation is the fake's
//! determinism argument applied one layer out (task §"Tests required").
//!
//! # Framing
//!
//! The CLI writes one JSON object per line, but a pipe delivers arbitrary
//! chunks: a line can arrive in forty pieces, and two lines can arrive in one.
//! [`ClaudeStream::ingest`] holds the partial tail in a buffer that lives on
//! `self`, which is also what makes the async reader around it cancel-safe —
//! a `select!` that drops a read mid-line loses nothing.
//!
//! # What is bounded here, and what is deliberately not
//!
//! Nothing in this module clips or tidies a *meaning*. `AgentRunEvent::bounded`
//! is the one ingest seam that does that, applied once in the core, "never in
//! the adapter, which is the untrusted side" (task §"Failure behavior"). A
//! prompt, a reply, a command and a path all pass through exactly as the child
//! wrote them.
//!
//! What *is* bounded is resources, because an adapter that let a hostile child
//! allocate without limit would not survive to be sanitised:
//!
//! | Bound | Why |
//! |---|---|
//! | [`MAX_LINE_BYTES`] | one line cannot grow the buffer without limit; an oversized line is dropped whole and counted, never truncated into a half-event |
//! | [`MAX_OUTSTANDING_TOOLS`] | the table pairing a `tool_result` to its `tool_use` has a ceiling, so a child that opens calls and never closes them is bounded |
//! | [`MAX_KIND_BYTES`] | the `type` of an unmodelled line is the child's string too, and it is carried into an observation |

use std::collections::VecDeque;

use serde_json::Value;

use crate::message::{
    AssistantBlock, AssistantLine, ControlRequestLine, ControlResponseLine, InitLine, ResultLine,
    UserBlock, UserContent, UserLine,
};
use crate::observation::{
    ClaudeObservation, ClaudePermissionAsk, ClaudeReady, ClaudeToolDone, ClaudeToolIntent,
    ClaudeToolUse, ClaudeTurnEnd,
};

/// Longest line accepted, in bytes.
///
/// A `tool_use` carrying a whole file as its `content` is the largest thing the
/// CLI plausibly writes, and a megabyte covers that with room to spare; the
/// recorded fixture's biggest line is under 3 KB. A line past this is dropped
/// **whole** rather than truncated, because half a JSON object parsed into a
/// half-event is exactly the "truncated event" the failure table forbids.
///
/// The cost is named rather than hidden: a genuinely enormous write would be
/// dropped, so the run would show the intent and not the completion. That is
/// the safe direction — a lost event, never a wrong one.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// How many unclosed `tool_use` calls are remembered for pairing.
///
/// Claude Code issues tool calls a few at a time, so this is generous; it
/// exists so a child that opens calls and never closes them cannot grow the
/// table. The oldest is evicted, and a result whose call was evicted still
/// reports — with `call: None` — rather than vanishing.
pub const MAX_OUTSTANDING_TOOLS: usize = 64;

/// Longest `type` remembered for an unmodelled line.
///
/// The discriminator is the child's string like everything else, and it is the
/// one untrusted value this module copies into an observation, so it is capped.
/// Bounding an identifier is not the same as clipping a message: nothing
/// downstream displays this as prose.
pub const MAX_KIND_BYTES: usize = 64;

/// What the framing had to throw away.
///
/// Counters rather than events on purpose: a malformed line is not something a
/// user can act on, and one bad line should not push a real one out of the
/// bounded ring. They are read by the process layer, which logs them once, by
/// count and never by payload.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClaudeStreamStats {
    /// Lines that parsed and were dispatched.
    pub lines: u64,
    /// Lines longer than [`MAX_LINE_BYTES`], discarded to the next newline.
    pub oversized: u64,
    /// Lines that were not a JSON object with a string `type`.
    pub malformed: u64,
    /// Lines whose kind this crate does not model. Not a problem — the CLI is
    /// a moving target — but worth a number, because a *rising* count is how
    /// a release that changed the format announces itself.
    pub unknown: u64,
}

/// The CLI's stdout, framed and translated.
///
/// Holds the small amount of state translation genuinely needs: the partial
/// line, the unclosed tool calls, and the outstanding permission asks. It does
/// **not** hold a copy of the run's state — that is the core's, moved only by
/// a transition it accepted, and an adapter keeping its own would be a second
/// answer to a question that already has one.
#[derive(Debug, Default)]
pub struct ClaudeStream {
    buffer: Vec<u8>,
    /// Discarding the remainder of a line that went past [`MAX_LINE_BYTES`].
    discarding: bool,
    outstanding: VecDeque<ClaudeToolUse>,
    /// `request_id`s of asks the CLI has raised and not yet had answered.
    /// Bounded by the same ceiling: an ask is a tool call that has not run.
    asks: VecDeque<String>,
    ready: bool,
    turn_in_flight: bool,
    stats: ClaudeStreamStats,
}

impl ClaudeStream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the next chunk of stdout. Returns everything it completed.
    ///
    /// A chunk boundary means nothing: the same bytes split any way at all
    /// produce the same observations in the same order, which
    /// `framing_does_not_depend_on_chunk_boundaries` asserts one byte at a
    /// time.
    pub fn ingest(&mut self, chunk: &[u8]) -> Vec<ClaudeObservation> {
        let mut out = Vec::new();
        for &byte in chunk {
            if byte == b'\n' {
                if self.discarding {
                    self.discarding = false;
                    self.buffer.clear();
                    continue;
                }
                let line = std::mem::take(&mut self.buffer);
                self.dispatch(&line, &mut out);
                continue;
            }
            if self.discarding {
                continue;
            }
            self.buffer.push(byte);
            if self.buffer.len() > MAX_LINE_BYTES {
                self.stats.oversized += 1;
                self.discarding = true;
                self.buffer.clear();
            }
        }
        out
    }

    /// Flush a final line the child wrote without a trailing newline.
    ///
    /// Called once, when stdout is at end of file. A child killed mid-write
    /// leaves a partial line here, and that line is malformed rather than
    /// interesting — which is why this exists at all: without it the partial
    /// bytes would just be forgotten, and the counter would not know.
    pub fn flush(&mut self) -> Vec<ClaudeObservation> {
        let mut out = Vec::new();
        if self.discarding {
            self.discarding = false;
            self.buffer.clear();
            return out;
        }
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            self.dispatch(&line, &mut out);
        }
        out
    }

    pub fn stats(&self) -> ClaudeStreamStats {
        self.stats
    }

    /// Whether `system`/`init` has been seen. What ends `STARTING` (task §3).
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Whether a turn has been asked for and not answered.
    ///
    /// Read by [`crate::process`] when the child goes away, because "stdout
    /// closed mid-turn" is a failure and "stdout closed with nothing
    /// outstanding" is an ordinary ending.
    pub fn turn_in_flight(&self) -> bool {
        self.turn_in_flight
    }

    /// A prompt was just written to the child.
    ///
    /// The stream only ever sees stdout, so without this a child that died
    /// before echoing the prompt back would look idle rather than
    /// interrupted mid-turn.
    pub fn note_prompt_sent(&mut self) {
        self.turn_in_flight = true;
    }

    fn dispatch(&mut self, line: &[u8], out: &mut Vec<ClaudeObservation>) {
        if line.iter().all(u8::is_ascii_whitespace) {
            return;
        }
        let Ok(value) = serde_json::from_slice::<Value>(line) else {
            self.stats.malformed += 1;
            return;
        };
        let Some(kind) = value.get("type").and_then(Value::as_str) else {
            self.stats.malformed += 1;
            return;
        };
        self.stats.lines += 1;
        let kind = kind.to_owned();
        match kind.as_str() {
            "system" => self.system(&value, out),
            "assistant" => self.assistant(value, out),
            "user" => self.user(value, out),
            "result" => self.result(value, out),
            "control_request" => self.control_request(value, out),
            "control_response" => self.control_response(value, out),
            _ => out.push(self.unknown(&kind)),
        }
    }

    fn unknown(&mut self, kind: &str) -> ClaudeObservation {
        self.stats.unknown += 1;
        ClaudeObservation::Unknown {
            kind: clip_kind(kind),
        }
    }

    fn system(&mut self, value: &Value, out: &mut Vec<ClaudeObservation>) {
        let subtype = value
            .get("subtype")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if subtype != "init" {
            // `hook_started`, `hook_response`, `thinking_tokens`, and whatever
            // the next release adds. All four of those are in the recorded
            // fixture, and none of them means anything to a run.
            let kind = format!("system/{subtype}");
            out.push(self.unknown(&kind));
            return;
        }
        let init: InitLine = serde_json::from_value(value.clone()).unwrap_or_default();
        // Verified to arrive once per *turn*, not once per process, so this is
        // not guarded against repeats: a second `init` is the run going from
        // `WAITING` back to `RUNNING`, which is what it says.
        self.ready = true;
        out.push(ClaudeObservation::Ready(ClaudeReady {
            session_id: init.session_id,
            model: init.model,
            cwd: init.cwd,
            tools: init.tools.len(),
        }));
    }

    fn assistant(&mut self, value: Value, out: &mut Vec<ClaudeObservation>) {
        let Ok(line) = serde_json::from_value::<AssistantLine>(value) else {
            out.push(self.unknown("assistant"));
            return;
        };
        for block in line.message.content {
            match block {
                AssistantBlock::Text { text } => out.push(ClaudeObservation::Said { text }),
                AssistantBlock::Thinking => out.push(ClaudeObservation::Thought),
                AssistantBlock::ToolUse { id, name, input } => {
                    let call = ClaudeToolUse {
                        intent: intent_for(&name, &input),
                        id,
                        name,
                    };
                    self.remember(call.clone());
                    out.push(ClaudeObservation::ToolStarted(call));
                }
                AssistantBlock::Other => out.push(self.unknown("assistant/block")),
            }
        }
    }

    fn user(&mut self, value: Value, out: &mut Vec<ClaudeObservation>) {
        let Ok(line) = serde_json::from_value::<UserLine>(value) else {
            out.push(self.unknown("user"));
            return;
        };
        if line.is_replay {
            // `--replay-user-messages`: the acknowledgement the console's
            // prompt box waits for. Blocks are joined rather than emitted one
            // at a time, because one turn is one `PROMPT_STARTED`.
            let text = match line.message.content {
                UserContent::Text(text) => text,
                UserContent::Blocks(blocks) => blocks
                    .into_iter()
                    .filter_map(|block| match block {
                        UserBlock::Text { text } => Some(text),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            };
            self.turn_in_flight = true;
            out.push(ClaudeObservation::PromptEchoed { text });
            return;
        }
        let UserContent::Blocks(blocks) = line.message.content else {
            // A non-replay user message with bare text: not something this
            // flow produces, and reporting it as a prompt would put words in
            // the user's mouth.
            out.push(self.unknown("user"));
            return;
        };
        let before = out.len();
        for block in blocks {
            if let UserBlock::ToolResult {
                tool_use_id,
                is_error,
            } = block
            {
                let call = self.close(&tool_use_id);
                out.push(ClaudeObservation::ToolFinished(ClaudeToolDone {
                    call,
                    id: tool_use_id,
                    ok: !is_error,
                }));
            }
        }
        if out.len() == before {
            out.push(self.unknown("user"));
        }
    }

    fn result(&mut self, value: Value, out: &mut Vec<ClaudeObservation>) {
        let Ok(line) = serde_json::from_value::<ResultLine>(value) else {
            out.push(self.unknown("result"));
            return;
        };
        self.turn_in_flight = false;
        // The turn is over, so a call still open at this point is never going
        // to be closed. Dropping them keeps the table from carrying one turn's
        // debris into the next.
        self.outstanding.clear();
        out.push(ClaudeObservation::TurnFinished(ClaudeTurnEnd {
            ok: !line.is_error && line.subtype == "success",
            subtype: line.subtype,
            text: line.result,
            denials: line.permission_denials.len(),
        }));
    }

    fn control_request(&mut self, value: Value, out: &mut Vec<ClaudeObservation>) {
        let Ok(line) = serde_json::from_value::<ControlRequestLine>(value) else {
            out.push(self.unknown("control_request"));
            return;
        };
        // A request that names no id cannot be answered, whatever it asks: a
        // response has to quote the id back. Recorded, and left for the run's
        // own cancellation rather than guessed at.
        if line.request_id.is_empty() {
            let kind = format!("control_request/{}", line.request.subtype);
            out.push(self.unknown(&kind));
            return;
        }
        if line.request.subtype != "can_use_tool" {
            out.push(ClaudeObservation::RequestUnsupported {
                request_id: line.request_id,
                subtype: clip_kind(&line.request.subtype),
            });
            return;
        }
        if self.asks.len() >= MAX_OUTSTANDING_TOOLS {
            self.asks.pop_front();
        }
        self.asks.push_back(line.request_id.clone());
        let tool = line.request.tool_name.unwrap_or_default();
        let intent = intent_for(&tool, &line.request.input);
        // Only for a call nothing more specific can be said about; a classified
        // call already names its path or command, and a write's content is
        // not what the user is deciding on (see `ClaudePermissionAsk`).
        let input = matches!(intent, ClaudeToolIntent::Other)
            .then(|| line.request.input.to_string())
            .filter(|rendered| rendered != "null");
        out.push(ClaudeObservation::PermissionRequested(
            ClaudePermissionAsk {
                request_id: line.request_id,
                tool,
                intent,
                input,
                detail: line.request.description,
                blocked_path: line.request.blocked_path,
                tool_use_id: line.request.tool_use_id,
            },
        ));
    }

    fn control_response(&mut self, value: Value, out: &mut Vec<ClaudeObservation>) {
        let Ok(line) = serde_json::from_value::<ControlResponseLine>(value) else {
            out.push(self.unknown("control_response"));
            return;
        };
        let Some(at) = self
            .asks
            .iter()
            .position(|id| *id == line.response.request_id)
        else {
            // An echo for an ask this stream never saw. Reported rather than
            // dropped, and *not* treated as a resume: resuming a run on a
            // request nobody outstanding would be the adapter inventing a
            // state change.
            out.push(self.unknown("control_response"));
            return;
        };
        self.asks.remove(at);
        let allowed = line
            .response
            .response
            .is_some_and(|verdict| verdict.behavior.as_str() == Some("allow"));
        out.push(ClaudeObservation::PermissionAnswered {
            request_id: line.response.request_id,
            allowed,
        });
    }

    fn remember(&mut self, call: ClaudeToolUse) {
        if self.outstanding.len() >= MAX_OUTSTANDING_TOOLS {
            self.outstanding.pop_front();
        }
        self.outstanding.push_back(call);
    }

    fn close(&mut self, id: &str) -> Option<ClaudeToolUse> {
        let at = self.outstanding.iter().position(|call| call.id == id)?;
        self.outstanding.remove(at)
    }
}

/// A discriminator the child chose, cut to [`MAX_KIND_BYTES`] on a character
/// boundary. A resource bound, not a tidying: the string is carried into an
/// observation and a hostile child picks its length.
fn clip_kind(kind: &str) -> String {
    let mut kind = kind.to_owned();
    if kind.len() > MAX_KIND_BYTES {
        kind.truncate(
            (0..=MAX_KIND_BYTES)
                .rev()
                .find(|&at| kind.is_char_boundary(at))
                .unwrap_or(0),
        );
    }
    kind
}

/// What a tool call means to Gritty, by the names Claude Code ships.
///
/// This is the one piece of provider knowledge that genuinely belongs on this
/// side of the seam: the core must never learn that a tool called `Edit`
/// writes a file. The list is deliberately short — a name this crate does not
/// know is [`ClaudeToolIntent::Other`], which still reports work happening,
/// and every MCP tool (`mcp__server__tool`) lands there by design.
///
/// A recognised name whose input does not carry the field it should is `Other`
/// too. Guessing a path out of a differently-shaped input would put a string
/// the core then decides is openable in front of the user.
fn intent_for(name: &str, input: &Value) -> ClaudeToolIntent {
    let field = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .map(std::borrow::ToOwned::to_owned)
    };
    match name {
        "Read" => field("file_path").map(|path| ClaudeToolIntent::ReadFile { path }),
        "Write" | "Edit" | "MultiEdit" => {
            field("file_path").map(|path| ClaudeToolIntent::WriteFile { path })
        }
        "NotebookEdit" => field("notebook_path").map(|path| ClaudeToolIntent::WriteFile { path }),
        "Bash" => field("command").map(|command| ClaudeToolIntent::RunCommand { command }),
        "Glob" | "Grep" => field("pattern").map(|query| ClaudeToolIntent::Search { query }),
        _ => None,
    }
    .unwrap_or(ClaudeToolIntent::Other)
}

#[cfg(test)]
mod tests;
