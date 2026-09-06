# Task T335 — Permissions: asked in place, answered in a queue (design artboards 12a/12b)

- **Phase:** 4
- **Status:** todo
- **Depends on:** T221 (hook bridge — the `PermissionRequest` event and the
  state file this extends), T232 (the timeline view the prompt is drawn in),
  T210 (the attention queue the background asks land in), **ADR-020**

## Objective

Make a permission prompt answerable from Gritty, and draw both halves of design
turn 12 — artboard **12a "In the console you're watching — the ask lands where
the work is"** and **12b "From agents you aren't watching — banner, then the
queue"** (Claude Design project `4a56ac43`, `Gritty Agent Layout.dc.html`).

The turn's thesis is the acceptance test for the whole task: *the agent you're
watching asks in place; the ones you aren't ask through a queue — never a modal
over the whole IDE.*

## Motivation

T221 earns `BLOCKED` and T232 draws it, so Gritty can already tell you `t090` is
stopped at a prompt. It cannot tell you *what* the prompt is about — the hook
records `Bash`, never which command — and it cannot answer. The user still walks
to another terminal, which is the failure the island exists to remove.

ADR-020 measured the way through: `PermissionRequest` fires exactly when Claude
Code is about to ask a human, carries `tool_input`, and honours a verdict
returned on stdout; overrunning the hook's `timeout` discards the output and
leaves the human's own prompt answerable. That last measurement is what makes a
waiting hook safe, and it is the reason this task is possible before T300.

## Dependencies

- **ADR-020**, in full. It is the decision this task implements, and its four
  measurements are the contract the hook is written against.
- **ADR-018 §§5–7**, because ADR-020 narrows §5 and leaves §6 and §7 standing —
  a file not a socket, and hook-reported *state* is still evidence.
- **T221** for `crates/agents/src/hook.rs`, the one place that owns both the
  writer and the reader of the state file, and for `phase_of_event`.
- **T232** for `AgentTimeline.pending`, `timelineRows()` and
  `.agent-feed__row--permission` — the row this task replaces with a card.
- **T210** for `AttentionItem`, `attentionItems()`, `waitedFor()` and the queue
  this task adds a fourth kind to.
- **ADR-016** for design tokens: every colour is an existing token.
- **ADR-010**: no unshortened path crosses the protocol. "Outside its worktree"
  is computed in the hook, which knows its own `cwd`, and crosses as a boolean.

## Required reading

- `docs/decisions/ADR-020-agent-permission-verdicts.md` (all of it)
- `docs/decisions/ADR-018-agent-integration-bridge.md` §§5–7 and Consequences
- Design project `4a56ac43`: `Gritty Agent Layout.dc.html`, artboards `12a` and
  `12b`; artboards `11a`/`11b` for the shell they sit in
- `crates/agents/src/hook.rs:1-27,119-144,248-271` — the format's doc comment,
  `HookState`, and the atomic write
- `crates/gritty-core/src/agent_session/wire.rs:283-292,320-400` — the timeline
  kinds and why `pending` is held beside the ring
- `apps/desktop/src/features/agents/agent-timeline-model.ts:49-93,219-320`
- `apps/desktop/src/features/workstreams/attention.ts:11-118`
- `docs/merge-queue.md` — claim the protocol, schema and migration numbers first
- `AGENTS.md` hard rules 4, 8, 10, 12, 13, 14

## Allowed scope

- `crates/agent-hook/**`, `crates/agents/**`
- `crates/gritty-core/src/agent_session/**`, and
  `crates/gritty-core/src/protocol/export.rs` (the export list only)
- `crates/persistence/**` (migration `0005` and its store module only)
- `apps/desktop/src-tauri/src/{commands.rs,lib.rs}` (command adapters and the
  handler registry only)
- `packages/protocol/src/{commands.ts,types.ts,client.ts}` and
  `packages/protocol/src/generated` (**written by `cargo test -p gritty-core`,
  never by hand**)
- `apps/desktop/src/features/agents/**`,
  `apps/desktop/src/features/workstreams/**`
- `apps/desktop/src/App.tsx` (the banner's composition only),
  `apps/desktop/src/app.css` (the permission card, banner and queue sections),
  `apps/desktop/src/features/editor/status-bar.tsx` (one span)
- `docs/decisions/ADR-020-*`, `docs/decisions/README.md`,
  `docs/architecture/agent-sessions.md`, `docs/merge-queue.md`,
  `docs/tasks/README.md`, this file

## Explicit non-goals

Artboard elements with nothing behind them, named so the next reader does not
think they were forgotten:

- **The composer.** 12a's greyed `❯ paused until you answer — typing denies and
  replies instead` implies an input that types into the agent. There is none
  anywhere in Gritty, and typing into an agent is T300. The footer is drawn as
  static, disabled text or not at all — it never accepts a keystroke.
- **`▶▶ auto mode on · shift+tab to cycle`.** That is Claude Code's own mode
  cycling. Gritty neither reads nor sets it. `permission_mode` arrives in the
  hook payload and is *reported* if anything, never changed.
- **`t090 running · step 4/6` and `3 files held`** in 12b's status bar. Step
  counts belong to runs Gritty owns (T300); `files held` is `OwnershipMap`,
  which is held by a workstream and needs T223's opt-in link. Only
  `N permissions waiting` is earned.
- **The Agents section in the left sidebar.** 12a/12b draw it because turn 10
  introduced it; it is unbuilt and unticketed. This task lands in the existing
  agent rail.
- **Enforcement.** Nothing here stops a write. A denied ask is honoured because
  the agent asked. Blocking a write nobody asked about is T330.
- **Cross-workspace aggregation** — T410.
- **A second provider.** Claude Code only. The verdict file, the marker and the
  bounded request are provider-neutral; the event name and reply shape live in
  the adapter, and Codex is T222.
- **Answering a workstream `Decision`.** Unrelated surface, already built.

## Required behavior

1. `PermissionRequest` makes the hook write its state file **first** — so
   `BLOCKED` is earned exactly as today whether or not Gritty is listening —
   and only then consider waiting.
2. The state file carries a bounded `ask`: an id, the classified verb
   (`WRITE_FILE`, `RUN_COMMAND`, `INSTALL_DEPENDENCY`, `OTHER`), the target, a
   blast-radius summary, an optional short diff, and whether the target is
   outside the session's own root. Every field is truncated by the hook on
   write and again by the core on read.
3. The hook waits only while Gritty's liveness marker is fresh, polls for a
   verdict, and emits it as
   `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{…}}}`.
   Absent marker, stale marker, timeout, unreadable or malformed verdict: emit
   nothing, exit 0.
4. `AgentSession` carries at most one pending ask, so a background agent's
   prompt is visible without its timeline being followed. `AgentTimeline.pending`
   is derived from the same value — one source of truth, drawn twice.
5. `agents.answerPermission` takes an ask id and a verdict of `ALLOW_ONCE`,
   `ALLOW_FOR_RUN` or `DENY`, writes the verdict file, and records the answer.
   `ALLOW_FOR_RUN` records a grant that is matched against later asks from the
   same session so the prompt does not return.
6. **12a**: the followed session's ask is drawn as a card at the foot of the
   feed — dot, `<id> needs permission`, verb, target, `outside its worktree ·
   asked 8s ago`, the diff, then Allow once (`⏎`) / Allow … for this run (`⌘⏎`)
   / Deny (`esc`), plus `Open diff` and `Ask again next time`.
7. **12b**: an ask from a session that is *not* being followed never steals
   focus. The row goes amber, the header count rises, a one-line banner offers
   the queue, and the queue answers in place with the verb, target and blast
   radius on each card. `Deny all`, `Answered just now … Undo` and the
   `Standing rules … Edit` row are part of it.
8. `⌥⌘A` opens the queue on its permission filter. It must not disturb `⌥A`.
9. Answering removes the ask everywhere at once: card, rail row, banner, queue,
   header count and status bar all come from the same value.

## Failure behavior

| Condition | Expected |
|---|---|
| Gritty not running | Hook writes its file, returns immediately, emits nothing. Agent prompts as it always did. |
| Gritty killed mid-wait | Marker goes stale; the hook releases within one marker interval and emits nothing. |
| Hook overruns its `timeout` | Claude Code discards the output (measured); the human's prompt is still live. |
| Verdict file malformed, truncated or half-written | Treated as absent. Written by atomic rename so a reader never sees half. |
| User answers in the terminal first | The ask disappears without a verdict. Normal, not an error; the card and queue row go. |
| Verdict written for an ask that has ended | Ignored on read; never resurrected. |
| Session dies while an ask is open | Process evidence wins, as ADR-018 §7 already requires: the row reports `ENDED` and the ask is dropped. |
| Database unavailable | Answering still reaches the agent; the grant is not recorded and the UI says so rather than claiming a standing rule it did not store. |
| Two asks from one session | Only the newest is held. A ring would keep answered prompts claiming to be waiting — the reason `pending` sits beside the ring at all. |
| Sensitive target (`.env`, `*.pem`, key material) | The ask names the path; the diff is omitted, not truncated. |

## Acceptance criteria

- `pnpm check` green.
- A real Claude Code session stopped at a prompt shows the 12a card naming the
  verb and the target; Allow once lets the tool run and the transcript records
  `Allowed by PermissionRequest hook`; Deny refuses it and the reason reaches
  the agent.
- With Gritty closed, the same session prompts in its own terminal, unchanged.
- Killing Gritty while a prompt waits releases the agent rather than hanging it.
- Two background sessions asking produce a banner, not a modal; `⌥⌘A` opens the
  queue; answering one leaves the other.
- `Allow for this run` stops the same ask returning; `Undo` makes it return.
- Light and dark both legible; 800×600 scrolls rather than clipping.
- No secret ever appears in a log, an event or the state file.

## Tests required

- **Hook (Rust):** no marker → instant return, empty stdout; stale marker →
  same; verdict appears → exact JSON; marker goes stale mid-wait → releases;
  malformed verdict → empty stdout, exit 0; oversized `tool_input` → truncated;
  sensitive path → no diff. The mid-wait release is **executed**, not reasoned
  (rule 12).
- **Core (Rust):** classification of each verb; a v1 state file still parses;
  an ask id is unguessable and cannot choose where the verdict is written (the
  sibling of `a_session_id_cannot_choose_where_the_hook_writes`); a grant
  suppresses a later matching ask; answering an unknown ask is `NotFound`;
  answering twice is not two verdicts.
- **Renderer:** the hostile-string sweep `agent-timeline-render.test.ts` already
  applies, extended to the target, summary, blast radius and diff lines. A
  render test that the three buttons exist **and** call the controller — a
  view-model test cannot catch a missing caller, which is what sent T210, T221
  and T232 back. Pure tests for the queue's fourth kind, ordering and counts.
- **Manual:** the four ADR-020 measurements, re-run against the shipped hook
  rather than a probe script.

## Commands to run

```
cargo build -p gritty-agent-hook     # before any manual pass, or T221 reads "unavailable"
cargo test -p gritty-core            # also regenerates packages/protocol/src/generated
pnpm check
GRITTY_LOG=debug pnpm dev
```

Claim `PROTOCOL_VERSION 2.12.0`, `SCHEMA_VERSION 5` and migration
`0005_agent_permissions.sql` in `docs/merge-queue.md` before spending them.

## Completion report

_To be filled in._
