# Task T230 — Agent timeline: what a session is actually doing

- **Phase:** 3
- **Status:** done
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

### Summary

The follower and the domain behind the timeline commands. `gritty-agents` gained
`follow(path, from_offset)` beside `scan()`, sharing the same parser, limits and
reader; `gritty_core::agent_session` gained a bounded ring per open timeline, a
fold onto the provider-neutral wire kinds, workspace-relative resolution of
touched files, and a second loop that follows only what is open and emits one
coalesced batch per tick — or nothing.

The wire surface (`AgentTimelineEntry`, `AgentTimelineKind`, `AgentTimeline`,
`AgentTimelineAppendedPayload`, the three commands, the event, `PROTOCOL_VERSION`
2.8.0) was already landed before this session and was **not** changed: no type,
field, tag or doc comment in `wire.rs` was touched, so `packages/protocol/src/generated`
is byte-identical and the T231 session's UI kept building against it.

### Files changed

- `crates/agents/src/transcript.rs` — `follow` / `follow_with`, `FollowOptions`,
  `Followed`, `FollowedEntry`, `FollowedRecord`, `NoticeLevel`, a strict
  ISO-8601 → epoch-millis parser, and three refactors that keep one parser
  rather than two: `read_tail` now sits on a shared `read_window` / `file_len`,
  `clip` delegates to a new `clip_str` that only ever collapses a bounded
  prefix, and the slash-command rule is a shared `is_command_echo`. `MAX_TITLE`,
  `MAX_PREVIEW` and `MAX_TOOL_DETAIL` became `pub`, joined by `MAX_TRACKING_PATH`
  and `MAX_NOTICE`, so the core bounds with the adapter's own limits.
- `crates/agents/src/lib.rs` — re-exports for the follower.
- `crates/gritty-core/src/agent_session/timeline.rs` — **new**: `OpenTimeline`
  (the ring, the pending delta, `take_batch`, `snapshot`), the fold onto
  `AgentTimelineKind`, `fold_file_touched`, `lexical_absolute`, `bounded`.
- `crates/gritty-core/src/agent_session/mod.rs` — the real `open_timeline`,
  `close_timeline` and `timeline`; the fast loop (`follow`, `follow_once`);
  `FollowMetrics`; `TimelineSource` populated by every scan; `MAX_OPEN_TIMELINES`;
  `DEFAULT_TIMELINE_INTERVAL`; `shutdown_and_join`; `domain_session_id` extracted
  so the roster id and the timeline key cannot drift.
- `crates/gritty-core/src/agent_session/tests.rs` — the manager-level T230 tests.
- `docs/architecture/agent-sessions.md`, `docs/tasks/README.md`, this file.

### Tests actually executed, with results

```
cargo test -p gritty-agents                              52 passed, 0 failed
cargo test -p gritty-core                                209 + 3 passed, 0 failed
cargo clippy --workspace --all-targets -- -D warnings    clean
cargo fmt --all                                          applied
```

New tests, by the list this task asked for:

- *follower* (`crates/agents/src/transcript.rs`): every record type to its own
  entry; call/result paired by `tool_use_id`; `isSidechain` marked; a failed
  result; timestamps read and never invented; a following call reads only what
  was appended and does not move on an unchanged file; a partial final line held
  back and parsed on the next call; a file shorter than the offset restarting; a
  burst larger than one window reported as a gap; a malformed line skipped;
  non-UTF-8 tolerated; an unreadable and an empty transcript; oversized fields
  bounded; slash commands not reported as prompts; the timestamp parser's
  accepted and rejected shapes; a **cold open on a fixture larger than twice the
  window** proving only the tail is read.
- *fold, paths, bounding, ring*
  (`crates/gritty-core/src/agent_session/timeline.rs`): each record to its
  provider-neutral kind; strictly increasing `seq`; sidechain marked; a file
  inside the open root yielding a `WorkspacePath`; one outside it yielding
  nothing openable; an absolute path inside the root; no workspace open; a
  relative path with no working directory; a path climbing out of the root;
  lexical normalisation; **every kind's strings clipped at the core's limits**;
  the ring dropping oldest and reporting `dropped`; no batch when nothing
  changed; fifty records in one batch; a restart resetting rather than splicing;
  an unreadable transcript reported rather than treated as silence.
- *lifecycle* (`crates/gritty-core/src/agent_session/tests.rs`): open returns
  what the transcript already says; following delivers only what was appended as
  one batch; an unchanged transcript emits nothing across three ticks; a
  truncated transcript restarts; **closing stops the follower, asserted against
  the loop's own `FollowMetrics.reads`**; opening twice is idempotent and starts
  no second follower; an unknown session is `NotFound` on all three commands;
  closing what was never open is fine; `agents.timeline` reads a session nobody
  is following without starting one; a process-only session reports
  `transcriptEvidence: false`; the open-timeline bound; `shutdown_and_join`
  cancelling and joining both loops.

**Not executed:** `pnpm check`. Another session had the JS tree mid-edit for
T231 throughout, and this task changed no TypeScript and no generated type. The
Rust half of `pnpm check` (`fmt --check`, `clippy -D warnings`, `cargo test`) was
run directly and is green.

**Manual:** the acceptance criterion "watch a real running session" was **not**
performed — see Known limitations.

### Measured fast-loop cost (§14)

Release build, 400 MB synthetic transcript (400 KB/record-block, ~1.05 M
records), Apple Silicon, warm page cache:

| Call | Cost |
|---|---|
| cold open, 400 MB file | 1.4 – 3.4 ms (696 entries from the 256 KiB tail) |
| cold open, 5 KB file | 57 – 90 µs |
| tick, nothing appended | ~25 µs |
| tick, 20 records appended | ~50 – 60 µs |

So the cost tracks the *window*, not the file: a 400 MB transcript opens in
single-digit milliseconds and is then followed for tens of microseconds a tick.
At 400 ms that is well under a thousandth of a core for the one timeline the
rail opens, which is what makes the second loop affordable. A tick with nothing
open does no I/O at all and only bumps a counter.

### Architectural decisions

1. **The follower lives in `transcript.rs`, not a new module.** The record
   vocabulary, `clip`, `tool_detail` and the reader are shared with `scan`
   verbatim. A sibling module would have needed all of them `pub(crate)` and
   would have been the first step towards two parsers that disagree.
2. **`read_tail` was rebuilt on a shared `read_window`.** One place reads a
   transcript, so the "never read a whole file" property has one place to hold.
   Behaviour is unchanged; the existing T220 tests still pass untouched.
3. **`clip` now collapses only a bounded prefix (4096 chars).** Whitespace
   collapsing is a streaming transform, so the answer is identical for every
   limit in use, but a megabyte tool result no longer costs a megabyte of work
   per entry on a loop that runs twice a second.
4. **The adapter has its own record enum (`FollowedRecord`); the core maps it
   onto `AgentTimelineKind`.** Rule 4 and ADR-005: raw provider output never
   becomes domain state, and a second provider adds a match arm rather than a
   wire variant.
5. **Path resolution is lexical, never physical.** A `stat` per touched file on
   the fast loop buys nothing: the renderer cannot open the result without
   `WorkspaceRoot::resolve` checking it physically again, and a path that climbs
   out of its base is refused rather than clamped. `WorkspaceRoot::relativize`
   is reused rather than reimplemented (rule 2).
6. **The fast loop *is* the flusher.** The terminal needs a separate flusher
   because its PTY reader must never block; here the reader is the tick, so one
   tick produces at most one batch per session by construction. No new mechanism
   was introduced (rule 8).
7. **A "transcript missing" timeline carries no synthesised `Notice`.** The
   failure table asks for "an empty timeline and a notice"; the notice is
   `transcriptEvidence: false`, which the wire already documents as "nothing to
   read, not the agent did nothing". Manufacturing a `Notice` entry would put a
   line in the feed that no record produced, and `Notice` means *the run*
   reported something.
8. **`dropped` means two related things, deliberately.** On the snapshot it is
   every entry evicted since the timeline opened; on a batch it is the entries
   evicted since the previous batch, which is exactly what a consumer mirroring
   the ring must trim from its own head. Both are documented in `timeline.rs`.
9. **A restart clears the ring rather than counting the loss as `dropped`.**
   `restarted` already says "your view was reset"; adding the cleared entries to
   `dropped` would double-report one event.
10. **`agents.timeline` on a session nobody is following performs one bounded
    read** instead of answering with a confident empty timeline. It starts no
    follower and keeps no state, so it stays a read.
11. **`close_timeline` no longer requires the session for a timeline that is
    open.** An open ring is freed unconditionally; only a close for a session
    that is neither open nor in the roster is still `NotFound`, which keeps the
    stub's contract while making the idempotence its doc comment promises real.
12. **The fast loop's first tick is delayed by one interval** (`interval_at`),
    unlike discovery's, which fires immediately: nothing is open at startup, so
    an immediate tick would be a wakeup with no work in it.
13. **An open timeline closes when the workspace changes.** Its `FileTouched`
    entries were resolved against the old root and cannot be re-resolved without
    re-reading; closing is what the failure table asks for and the honest move.
14. **No new dependency.** The ISO-8601 timestamps are parsed by a strict
    twenty-line function (Howard Hinnant's `days_from_civil`) rather than a date
    crate, because exactly one format appears in these files and anything else
    must yield "no time" rather than a guess.

### Known limitations

- **`MAX_OPEN_TIMELINES = 8` is new and enforced with `Conflict`.** T231 opens
  exactly one, so nothing today can reach it, but a future surface that opens a
  timeline per visible row will need to close as it scrolls.
- **Sub-second bursts can be lost by the ring.** 500 entries is roughly a long
  turn; an agent that writes more than that between two ticks reports the
  overflow as `dropped` rather than growing.
- **The manual acceptance check was not run.** This session has no way to open
  the app's window and watch a live rail; the equivalent behaviour is covered by
  `following_delivers_only_what_was_appended_as_one_batch` against a file being
  appended to under a live follower. T231's session should confirm it in the
  running app.
- **The timestamp parser accepts UTC only.** Claude Code writes `…Z`; an offset
  timestamp yields `at: None` rather than a wrong time. T222 should check what
  Codex writes.
- **`file-history-delta` is the only file evidence.** A tool call that edits a
  file without producing one (or an agent configured without file history) shows
  as a `ToolCall`, not a `FileTouched`.
- **Sidechain entries are marked but not grouped.** They interleave with the
  parent's flow in `seq` order; grouping them is a rendering decision for T232.
- **The touched-file ordering is the transcript's**, so a file written twice
  appears twice. Collapsing duplicates is a view concern (T232).

### Follow-up tasks

- **T232** should decide how `sidechain` and repeated `FileTouched` entries are
  drawn, and whether the rail wants a de-duplicated "files touched" list beside
  the feed.
- **T221** puts the hook-state directory on this same fast loop and is the only
  thing that may produce `AgentTimelineKind::PermissionPending`; the loop, the
  metrics and the batching are already there for it.
- **T222 (Codex)** needs a `FollowedRecord` mapping for its own record shapes and
  must re-check the timestamp format; the fold and the ring are provider-neutral
  already.
- A renderer-visible signal for "your open timeline was closed because the
  workspace changed" would be kinder than the current silent close. It needs a
  wire addition, so it is deliberately not in this task.
- `tool_detail`'s fallback still serialises an opaque tool input to JSON before
  clipping it. It is bounded by the read window (1 MiB), so it is not a leak,
  but a keys-only summary would be cheaper.
