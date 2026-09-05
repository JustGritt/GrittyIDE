# Task T230 — Agent timeline: what a session is actually doing

- **Phase:** 3
- **Status:** not yet started
- **Depends on:** T020 (workspace), T220 (agent session discovery)

## Objective

T220 answers *which agents are running*. This answers *what one of them is
doing*: follow a discovered session's transcript forward from a byte offset and
publish a bounded, provider-neutral timeline of its prompts, replies, tool
calls, results and the files it touched.

This is the core half of the agent rail. It has **no UI** and is fully testable
in Rust; T231 renders it.

The mechanism needs no cooperation from the agent and writes into nobody's
configuration. Claude Code's transcript is an append-only JSONL file that
already carries everything the rail needs; reading it forward from an offset is
a live feed. The hook bridge (T221) adds one thing this cannot see — a pending
permission prompt — and nothing else.

## Dependencies

- **T220** for `gritty-agents`, `AgentSessionManager`, the roster, the poll loop
  and the record-shape knowledge already encoded in `transcript.rs`.
- **T020** for `WorkspaceManager` and `WorkspacePath`, used to decide which
  touched files the UI may open.
- **T010** for the operation model, the event envelope and `PROTOCOL_VERSION`.

## Required reading

- `docs/architecture/agent-sessions.md` — the two evidence sources, the bounds,
  and the "specified but not built" section this task implements half of
- `docs/decisions/ADR-005-agent-isolation.md` — the adapter/domain split
- `docs/architecture/security.md`, `docs/decisions/ADR-010-workspace-paths.md`
- `crates/agents/src/transcript.rs` — `scan`, `parse_transcript`, `read_tail`,
  `clip`, `MAX_PREVIEW`, `MAX_TOOL_DETAIL`. **Extend this; do not fork it.**
- `crates/gritty-core/src/terminal/mod.rs` and `crates/terminal/src/` — the
  bounded-buffer + coalescing-flusher pattern this task copies
- `crates/gritty-core/src/agent_session/{mod,wire}.rs`

## Allowed scope

- `crates/agents/src/{transcript.rs,lib.rs}` and new modules there
- `crates/gritty-core/src/agent_session/**`
- `crates/gritty-core/src/protocol/{mod.rs,export.rs}`
- `apps/desktop/src-tauri/src/{commands.rs,core.rs,lib.rs}`
- `packages/protocol/src/{client.ts,commands.ts,types.ts,index.ts}` + tests
- `docs/architecture/agent-sessions.md`, `docs/tasks/README.md`, this file

## Explicit non-goals

- **No UI.** T231 and T232 own the rail. This task ships commands, events and
  types, and is judged by Rust tests.
- **No control.** Nothing starts, stops, answers, approves or writes to a
  session. The timeline is a read.
- **No configuration written.** The hook bridge is T221; this task must work
  fully with no integration installed.
- **No run lifecycle.** T300 owns `AgentRun`. An observed timeline is not a run
  and its types must not be reusable as one.
- **No persistence.** The timeline is in-memory and rediscovered from the
  transcript; nothing reaches the session database.
- **No diff, no file content.** `FileTouched` names a file. Reading the
  before-copies under `~/.claude/file-history/` is a separate decision (T232
  follow-up).
- **No new providers.** Claude Code only, as T220 shipped. T222 adds Codex.
- **No whole-file reads.** The O(1)-in-file-size property T220 bought is the
  main thing this task must not lose.

## Required behavior

### The follower (`gritty-agents`)

1. `follow(path, from_offset) -> Followed { entries, next_offset, restarted }`
   seeks to `from_offset` and parses only the bytes after it. A first call with
   offset `0` on a large file must **not** read the whole file: it starts from a
   bounded tail and reports the offset it actually started at, exactly as
   `read_tail` already does for the roster.
2. A partial final line is **not** consumed and not counted into `next_offset` —
   the agent is appending while we read, and a half-written record must be
   parsed on the next tick, not dropped.
3. If the file is shorter than `from_offset`, it was truncated or replaced:
   restart from a bounded tail and set `restarted`, so the domain can tell the
   UI its view was reset rather than silently splicing unrelated records.
4. Parsing is shared with `scan()`. The record vocabulary is already known:
   `type=assistant` with `message.content[]` blocks (`text`, `thinking`,
   `tool_use`), `type=user` with `tool_result` blocks and `toolUseResult`,
   `type=system` (with `level`, `hookErrors`), `file-history-delta` (with
   `trackingPath`), plus the metadata records `ai-title`, `last-prompt`, `mode`
   and `permission-mode`.
5. Records with `isSidechain: true` are subagent activity. Mark them; do not
   drop them and do not merge them into the parent's flow.
6. A line that does not parse is skipped, not fatal. Non-UTF-8 is tolerated.

### The domain (`gritty_core::agent_session`)

7. `AgentTimelineEntry { seq, at, kind, sidechain }`, where `seq` is strictly
   increasing per session so the renderer can append without reconciling.
   `kind` is provider-neutral (hard rule 4):

   | Kind | Carries |
   |---|---|
   | `UserPrompt` | bounded text |
   | `AssistantText` | bounded text |
   | `Thinking` | bounded text |
   | `ToolCall` | tool use id, name, bounded detail |
   | `ToolResult` | tool use id, ok/failed, bounded detail |
   | `FileTouched` | see §8 |
   | `Notice` | level, bounded text (system records, hook errors) |
   | `PermissionPending` | tool name, bounded detail — **T221 only** |

8. `FileTouched` resolves `file-history-delta.trackingPath` — which the agent
   already writes relative to its project root — against the **open workspace
   root**. Inside it, the entry carries a `WorkspacePath` the UI may open.
   Outside it, the entry is marked external, carries a display string only, and
   **no path the UI can open** (ADR-010: the protocol does not put absolute
   paths on the wire, and nothing outside the root may be opened from the
   renderer).
9. **Every string is bounded in the core**, reusing `transcript.rs`'s existing
   limits. An oversized field must never cross the protocol boundary.
10. A bounded ring per open timeline. When it overflows it drops from the
    **oldest** end and reports the count as `dropped` — the terminal's rule:
    report the loss, never hide it.
11. Only sessions with an **open** timeline are followed. Opening one returns
    the current contents and starts following; closing one stops and frees the
    ring. A timeline for a session absent from the roster is `NotFound`.
12. Delivery is batched and rate limited, following the terminal's flusher: one
    coalesced batch per tick, not one event per record (hard rule 8). An
    unchanged timeline emits **nothing**.
13. The follower runs on a blocking-safe thread and never blocks a command
    handler or the renderer (rules 8 and 10). The loop stops on shutdown.
14. **Cadence.** The 6 s discovery loop is unchanged. Open timelines are
    followed on a **fast loop** (~300–500 ms), which is affordable precisely
    because it seeks to an offset and reads only new bytes, and because it runs
    only for what is open. Record the measured cost in the completion report.

### Protocol

15. `agents.openTimeline { sessionId }` → the current entries plus a cursor.
    `agents.closeTimeline { sessionId }`. `agents.timeline { sessionId }`
    rehydrates after a renderer reload without reopening (the T010 convention:
    subscribe, then call the domain's list command).
16. Event `AgentTimelineAppended { sessionId, entries, dropped?, restarted? }`.
17. Protocol version bumps MINOR (`2.7.0 → 2.8.0`): additive.

## Failure behavior

| Situation | Behavior |
|---|---|
| Transcript missing or unreadable | `openTimeline` succeeds with an empty timeline and a notice; the session may be process-only evidence |
| Transcript deleted while followed | Timeline stops growing, no error; the roster already reports the session's activity |
| Transcript truncated or replaced | Restart from a bounded tail, set `restarted`, so the UI can say the view reset |
| Partial final line | Not consumed, re-read next tick |
| Malformed or non-UTF-8 line | Skipped; every other record still parses |
| A 400 MB transcript opened cold | Bounded tail only; same cost as a 4 KB file, no memory spike |
| Agent writing faster than the UI drains | Ring drops oldest, `dropped` reported |
| Session ends while followed | Following stops at EOF; entries stay readable until the timeline is closed |
| Workspace closed or replaced | Open timelines close; `FileTouched` paths are no longer resolvable, which is why the resolution happens at parse time |
| Timeline opened for an unknown session | `NotFound` |
| Same timeline opened twice | Idempotent; returns the current contents, does not start a second follower |
| Shutdown | The fast loop is cancelled and joined; no follower outlives the app |

## Security

- Everything read is untrusted input (hard rule 13): prompts, replies, tool
  inputs, tool outputs, paths and titles are bounded **in the core**, never
  interpreted, never resolved as a path, never run, and rendered as text by T231.
- Tracing records counts, providers, offsets and durations — never prompt,
  reply or tool content.
- No timeline content is persisted.
- A touched file outside the open workspace root is never openable from the
  renderer.

## Acceptance criteria

- With Claude Code working in another terminal, opening its timeline shows the
  live feed and new entries appear within one fast-loop interval.
- A 400 MB transcript opens in the same bounded time as a small one, with no
  memory spike, and following it afterwards reads only the appended bytes.
- Truncating the transcript under a live follower sets `restarted` instead of
  splicing unrelated records together.
- No `AgentTimelineAppended` is emitted while the transcript is unchanged.
- A `trackingPath` outside the open workspace produces an entry with no openable
  path.
- Closing a timeline stops its follower — verified by the loop's own counters.
- `pnpm check` green; generated protocol types fresh and committed.

## Tests required

- **Unit (Rust):** follower against fixture JSONL — offset advance, partial
  final line held back, truncation restart, file shorter than offset, malformed
  line skipped, non-UTF-8 tolerated, cold open on a large fixture reads a
  bounded tail.
- **Unit (Rust):** the fold — each record type to its entry kind; `tool_use`
  paired with its `tool_result`; `isSidechain` marked; `file-history-delta` to
  `FileTouched`.
- **Unit (Rust):** path resolution — inside the root yields a `WorkspacePath`,
  outside yields an external entry with nothing openable.
- **Unit (Rust):** bounding — every entry kind's strings clipped at the core's
  limits.
- **Unit (Rust):** the ring drops oldest and reports `dropped`.
- **Unit (Rust):** an unchanged transcript emits no event.
- **Unit (Rust):** open/close lifecycle — idempotent open, close stops the
  follower, unknown session is `NotFound`, shutdown joins the loop.
- **Unit (TS):** the client methods and the event's type.
- **Manual:** open a timeline against a real running session and watch it.

## Commands to run

```
cargo test -p gritty-agents
cargo test -p gritty-core
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
```

## Completion report

_(filled in by the implementing session — including the measured fast-loop cost
required by §14)_
