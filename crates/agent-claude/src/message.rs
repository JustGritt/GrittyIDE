//! The shapes on the CLI's stdout, exactly as it writes them.
//!
//! Private to the crate, and deliberately so: these are *raw provider output*
//! (hard rule 4), and nothing outside this module is allowed to see a
//! `snake_case` field of somebody else's JSON. They exist to be turned into
//! [`crate::ClaudeObservation`] and then dropped.
//!
//! # How a line is dispatched, and why not with one `#[serde(tag = "type")]`
//!
//! An internally-tagged enum fails the *whole line* when the tag is a variant
//! it does not know, and it cannot say what the tag was. Task §2 asks for the
//! opposite: "An unrecognised `type` on the wire is **recorded as unknown and
//! skipped**, not an error: the CLI is a moving target and a new message kind
//! must not fail a run." So a line is parsed to a `Value` first, its `type` is
//! read as a string, and only then is the matching struct deserialized. A known
//! type whose *body* is unexpected fails only itself, and reports as unknown
//! with its own name.
//!
//! Every field is `#[serde(default)]` for the same reason. The recorded fixture
//! has thirty-odd keys this crate ignores — `usage`, `total_cost_usd`,
//! `permission_suggestions`, `parent_tool_use_id`, `request_id`, `uuid` — and a
//! field that becomes optional in a later release must not turn a run into a
//! failure.

use serde::Deserialize;

/// `{"type":"system","subtype":"init", …}` — the session is up.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct InitLine {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Names ignored; only the count is reported. The list is this machine's
    /// inventory and nothing downstream has a use for it.
    #[serde(default)]
    pub tools: Vec<serde_json::Value>,
}

/// `{"type":"assistant","message":{…}}`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct AssistantLine {
    #[serde(default)]
    pub message: AssistantMessage,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct AssistantMessage {
    #[serde(default)]
    pub content: Vec<AssistantBlock>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum AssistantBlock {
    #[serde(rename = "text")]
    Text {
        #[serde(default)]
        text: String,
    },
    /// Present without `--include-partial-messages`: a thinking block arrives
    /// whole, as its own assistant message, and its `signature` is never read.
    #[serde(rename = "thinking")]
    Thinking,
    #[serde(rename = "tool_use")]
    ToolUse {
        #[serde(default)]
        id: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    /// `redacted_thinking`, `server_tool_use`, and whatever comes next.
    #[serde(other)]
    Other,
}

/// `{"type":"user","message":{…},"isReplay":true}`.
///
/// Two very different things wear this type, and `isReplay` is what tells them
/// apart: the echo of what the user typed (`--replay-user-messages`), and the
/// `tool_result` the CLI feeds back to the model after a tool ran. Reading the
/// first as the second would put tool output in the console as if the user had
/// said it.
///
/// Note the spelling: `isReplay` is camelCase while its neighbours
/// (`session_id`, `parent_tool_use_id`) are snake_case. That is the CLI's, not
/// a mistake here, and it is why the rename is explicit.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct UserLine {
    #[serde(default)]
    pub message: UserMessage,
    #[serde(rename = "isReplay", default)]
    pub is_replay: bool,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct UserMessage {
    #[serde(default)]
    pub content: UserContent,
}

/// A user message's content is a bare string in the short form and a list of
/// blocks in the long one. Both are accepted; the CLI has used both.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum UserContent {
    Text(String),
    Blocks(Vec<UserBlock>),
}

impl Default for UserContent {
    fn default() -> Self {
        Self::Blocks(Vec::new())
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum UserBlock {
    #[serde(rename = "text")]
    Text {
        #[serde(default)]
        text: String,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(default)]
        tool_use_id: String,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(other)]
    Other,
}

/// `{"type":"result","subtype":"success", …}` — one turn is over.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ResultLine {
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub is_error: bool,
    /// The CLI's final answer for the turn.
    #[serde(default)]
    pub result: Option<String>,
    /// Counted, not carried: each denial already arrived as its own
    /// `control_request` and was recorded then.
    #[serde(default)]
    pub permission_denials: Vec<serde_json::Value>,
}

/// `{"type":"control_request","request_id":…,"request":{…}}`.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ControlRequestLine {
    #[serde(default)]
    pub request_id: String,
    #[serde(default)]
    pub request: ControlRequest,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ControlRequest {
    #[serde(default)]
    pub subtype: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    /// The call's arguments, exactly as the model issued them.
    ///
    /// Read for what the call is *about* — a path, a command — and never
    /// echoed back: an allowance is sent without `updatedInput`, which the CLI
    /// honours as "run it as issued" (T310-ANSWER, verified against 2.1.270),
    /// so nothing Gritty holds can alter what runs.
    #[serde(default)]
    pub input: serde_json::Value,
    /// The CLI's own one-line rendering of the call. For `Bash` this is the
    /// command; it is shown and never run.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub blocked_path: Option<String>,
    #[serde(default)]
    pub tool_use_id: Option<String>,
}

/// `{"type":"control_response","response":{…}}`, echoed back on stdout when the
/// CLI has taken an answer.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ControlResponseLine {
    #[serde(default)]
    pub response: ControlResponseBody,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ControlResponseBody {
    #[serde(default)]
    pub request_id: String,
    #[serde(default)]
    pub response: Option<ControlVerdict>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ControlVerdict {
    /// `allow` or `deny`.
    ///
    /// A `Value` rather than a `String`, and that is not laziness. A verdict
    /// body of an unexpected *shape* — `null`, a number, a future object —
    /// would fail the whole `control_response` line if this were typed, and a
    /// failed line leaves the ask outstanding: the run would sit in `BLOCKED`
    /// forever having already been answered, which is the inspectability
    /// failure the run machine exists to prevent. Read loosely, every shape
    /// that is not literally the string `allow` is a denial, the run resumes,
    /// and the one direction it is unsafe to guess in stays closed.
    #[serde(default)]
    pub behavior: serde_json::Value,
}
