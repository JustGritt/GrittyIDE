# Task T232 — The timeline view, and the files an agent touched

- **Phase:** 3
- **Status:** review
- **Depends on:** T030 (documents), T230 (timeline), T231 (rail)

## Objective

Render the selected session's feed inside the rail — prompts, replies, tool
calls, results — and list the files it has touched, so that clicking one opens
it in the editor next to the agent that changed it.

That last step is the point of the whole rail. A multiplexer can show you an
agent's output; only an IDE can put you in the file it just edited.

## Dependencies

- **T230** for `AgentTimelineEntry`, `agents.openTimeline` and
  `AgentTimelineAppended`, including the `FileTouched` entry that already
  carries a resolved `WorkspacePath` when the file is inside the open workspace.
- **T231** for the rail, the selection and the body this renders into.
- **T030** for `documents.open` and `documents.status`.

## Required reading

- `docs/architecture/documents.md` — external modification states, and that the
  check is **on demand** today
- `docs/architecture/agent-sessions.md`, `docs/architecture/security.md`
- `apps/desktop/src/features/documents/documents-reducer.ts` — `DocumentTab`,
  `findTabByPath`, and how an open document is reused
- `apps/desktop/src/features/editor/editor-pane.tsx` — how a file becomes visible
- `apps/desktop/src/features/markdown/` — the existing precedent for rendering
  untrusted text safely

## Allowed scope

- `apps/desktop/src/features/agents/**`
- `apps/desktop/src/{App.tsx,app.css}` (wiring the open-file callback only)
- `docs/architecture/agent-sessions.md`, `docs/tasks/README.md`, this file

## Explicit non-goals

- **No diff view.** The before-copies exist — `file-history-delta.backup`
  names one and it is on disk under `~/.claude/file-history/<session>/<hash>@v<n>`
  — but reading them means reading outside the workspace root, which is a
  boundary decision with its own security argument. Record it as a follow-up,
  with the observation that ADR-007's `ChangeSet` may be the right owner rather
  than this rail.
- **No editing, no reverting, no restoring a backup.**
- **No control of the session.** No prompting, answering or approving.
- **No transcript export, no "open the raw JSONL".** The rail shows a bounded
  view, not a file viewer.
- **No Markdown or HTML rendering of agent output.** See §7.
- **No Rust changes.** If the view needs a field the core does not send, that is
  a T230 gap to record, not to work around here.

## Required behavior

### The feed

1. Entries render newest-last, appended as `AgentTimelineAppended` arrives, with
   the list pinned to the bottom while the user is at the bottom and **not**
   yanked down when they have scrolled up to read something.
2. Each kind is visually distinct: prompt, assistant text, thinking, tool call,
   tool result, notice, and — when T221 is installed — a pending permission
   prompt, which is the one the user is meant to act on and reads as such.
3. A tool call shows its name and the core's bounded detail; its result shows
   success or failure. They are paired by tool use id so a result attaches to
   its call rather than floating.
4. Subagent entries (`sidechain`) are marked and visually subordinate. They are
   not hidden — T220's report lists dropping them as a known limitation, and
   this is where that is fixed.
5. `dropped` renders as an explicit gap ("earlier entries dropped"), never
   silently. `restarted` renders as an explicit reset. Both exist because the
   core reports its own losses instead of hiding them, and the UI must not undo
   that.
6. An empty timeline, a session with no transcript, and a session that has ended
   are three different states and read differently.

### Safety of what is rendered

7. **Every string is text.** Never `innerHTML`, never Markdown, never a link the
   view resolves, never a path anything opens by concatenation, never a command
   anything runs. Agent output and repository content are untrusted input
   (hard rule 13) and the core has already bounded every field; the renderer's
   only job is to not undo that.
8. Long single-line content wraps or scrolls inside its own container. The rail
   must never scroll horizontally as a whole.

### Files touched

9. A files-touched list for the selected session, deduplicated by path, most
   recent first, with the time it was last touched.
10. A file inside the open workspace is a button that opens it in the editor.
    A file outside it is shown as text and is **not** openable — T230 already
    sends no path for those, so this is enforced by the protocol, not by the
    renderer's restraint.
11. **Opening calls `documents.status` first.** External modification is checked
    on demand today (T070 does not exist), so a file the agent rewrote a second
    ago would otherwise open from a stale buffer, or reuse an already-open tab
    whose contents are old. Checking first turns that into the normal
    external-change flow the document contract already defines, and hard rule 7
    is satisfied because nothing is resolved silently: a dirty buffer with a
    changed file surfaces the conflict the same way it does everywhere else.
12. Opening reuses an already-open tab for that path rather than creating a
    second one, using the existing `findTabByPath`.

## Failure behavior

| Situation | Behavior |
|---|---|
| `documents.status` reports the file changed under a dirty buffer | The existing conflict flow, unchanged. This task adds no new resolution path |
| The touched file was deleted since the agent wrote it | The row stays, opening reports `NotFound` in the rail's error envelope; the list is history, not a live directory |
| The touched file is outside the open workspace | Text only, not clickable |
| `documents.open` fails for any other reason | Error envelope; the feed keeps updating |
| A very long tool result | Already bounded by the core; the view scrolls it inside its own container |
| Thousands of entries | The core's ring bounds the count; the view renders what it is given without its own unbounded growth |
| Timeline reset (`restarted`) while scrolled up | The gap is shown in place; the scroll position is not stolen |
| The session ends mid-render | The feed stops growing and says so; nothing is removed |

## Acceptance criteria

- With Claude Code working in another terminal, the rail's feed shows its
  prompts, replies and tool calls live, and pairs each result with its call.
- Scrolling up and staying there is not interrupted by new entries.
- A file the agent just edited appears in files-touched and opens in the editor
  showing the **current** contents, not a stale buffer — verified by editing a
  file in the agent while it is open in Gritty.
- A file the agent touched outside the open workspace is visible and not
  clickable.
- A dropped or restarted timeline renders an explicit gap.
- Subagent entries are visible and marked.
- No agent-supplied string is ever interpreted as markup — verified by a test
  with a fixture entry containing HTML and a path-like string.
- `pnpm check` green.

## Tests required

- **Unit (TS):** the entry → view-model mapping is total over the entry kinds,
  including one added later, so an unknown kind renders as text rather than
  vanishing.
- **Unit (TS):** tool calls pair with results by id; an unpaired call renders as
  still running.
- **Unit (TS):** files-touched dedup and ordering; an external file yields no
  openable target.
- **Unit (TS):** a fixture entry containing `<script>` and `../../etc/passwd`
  renders as text and produces no markup and no path anything opens.
- **Unit (TS):** the open-file flow calls `documents.status` before
  `documents.open`, and reuses an existing tab for the same path.
- **Unit (TS):** `dropped` and `restarted` render as gaps; the reducer does not
  splice around them.
- **Manual:** with a file open in Gritty, have Claude Code edit it in another
  terminal, then open it from files-touched and confirm the contents are current
  and any dirty-buffer conflict surfaces normally.

## Commands to run

```
pnpm check
```

## Completion report

**Status:** implemented, UI only. No Rust, no protocol bump, no schema change.
`packages/protocol/src/generated` is byte-identical to the base commit.

### Summary

The rail's placeholder body is gone. Selecting a session now draws the files it
touched and, under them, its feed — prompts, replies, thinking, tool calls with
their results attached, notices, permission prompts and subagent work — growing
live from `AgentTimelineAppended`. A touched file inside the open workspace is a
button that opens it in the editor beside the agent that changed it; a file
outside it is text, because the core sent no path for it.

Four decisions carry most of the weight, and each is a place where the obvious
implementation would have been wrong:

1. **The event is a delta, so the client accumulates.** `applyAppended` in
   `agent-rail-reducer.ts` appends by `seq` onto whatever `openTimeline`
   returned. Nothing treats a batch as a snapshot, because a timeline grows
   without bound and the core deliberately never resends it (rule 8).
2. **`dropped` and `restarted` are drawn, never repaired.** Each becomes an
   explicit row at the head of the feed. A restart discards what was held rather
   than splicing two transcripts together. The core reports its own losses
   precisely so the UI does not have to guess.
3. **A result attaches to its call by `toolUseId`.** An unpaired call reads as
   still running rather than being given an outcome nobody observed (rule 12).
4. **Opening calls `documents.status` before `documents.open`.** §11's reason
   holds exactly: nothing watches the disk yet, so a file the agent rewrote a
   second ago would open from a stale buffer, or reuse an open tab whose text
   predates the write. Checking first hands the case to the external-change flow
   the document contract already defines, and nothing is resolved silently
   (rule 7).

### Files changed

| File | What |
|---|---|
| `apps/desktop/src/features/agents/agent-timeline-model.ts` | new — the pure view model: entry to row (total over the kinds, with an `unknown` fallthrough), tool pairing, the gap rows, files-touched dedup and ordering, the scroll-pin predicate, the mirror of the core's ring bound |
| `apps/desktop/src/features/agents/agent-timeline-model.test.ts` | new — 21 cases |
| `apps/desktop/src/features/agents/agent-timeline.tsx` | new — `AgentTimelinePanel`: the files-touched list, the feed, one row component, the three empty states and the ended note |
| `apps/desktop/src/features/agents/agent-timeline-render.test.ts` | new — 6 cases, rendering the real component to static markup and asserting on its tags |
| `apps/desktop/src/features/agents/open-touched-file.ts` | new — the status-then-open flow over a two-method port |
| `apps/desktop/src/features/agents/open-touched-file.test.ts` | new — 5 cases, against a fake store that reuses tabs through the real `findTabByPath` |
| `apps/desktop/src/features/agents/agent-rail-reducer.ts` | `applyAppended`, the `appended` action, and the `openingPath` the file rows read |
| `apps/desktop/src/features/agents/agent-rail-reducer.test.ts` | 9 new cases (5 on the reducer, 6 on `applyAppended`; 1 existing assertion extended) |
| `apps/desktop/src/features/agents/use-agent-rail.ts` | subscribes to `AgentTimelineAppended`, exposes `openFile` / `openingPath` / `canOpenFiles` |
| `apps/desktop/src/features/agents/agent-rail.tsx` | placeholder replaced by the panel; the body gains a `--following` modifier so the roster cannot be pushed off screen |
| `apps/desktop/src/App.tsx` | passes `documents` and `workspaceId` into `useAgentRail` — the open-file wiring, and nothing else |
| `apps/desktop/src/app.css` | the feed, the files list, the gap and permission rows, the two-pane split of a followed rail |
| `docs/architecture/agent-sessions.md` | the rail section moved above the "specified but not built" line and now says what T232 shipped |
| `docs/tasks/README.md` | status to review |

`agent-status.ts` was deliberately **not** touched: T221 is editing it in
parallel to add `BLOCKED`.

No new dependency. `react-dom/server` is used by one test and is already a
dependency of `apps/desktop`.

### Tests actually executed

All of these were run in this session, in an isolated worktree with nothing else
editing the tree.

- `pnpm install` — clean.
- `pnpm typecheck` — clean across all four projects.
- `pnpm test` — **61 files, 454 tests, all passing** (vitest 4.1.11). 41 of
  those tests are new.
- `pnpm check` — run four times end to end; **exit 0** twice, and each of the
  other two runs hit a *different* pre-existing timing-sensitive Rust test (see
  follow-up 3). A green run covers biome ("Checked 220 files. No fixes
  applied.", no warnings), tsc, vitest, `cargo fmt --check`, `cargo clippy -D
  warnings`, `cargo test --workspace` (209 in `gritty-core`, 52 in
  `gritty-agents`, the rest as before) and the generated-types freshness check.
  The final run before committing was green.
- `cargo test -p gritty-core --lib` x6 and
  `cargo test -p gritty-terminal --test pty kill_terminates_the_whole_process_tree`
  x3, in isolation — all passing, while characterising those flakes.

The security case §7 asks for is `agent-timeline-render.test.ts`. It renders the
real `AgentTimelinePanel` with `<script>alert(1)</script><img src=x
onerror=alert(2)>` and `../../etc/passwd` in every text-bearing field — prompt,
reply, thinking, tool name, tool detail, result, notice, permission prompt,
sidechain entry, file display and the session title — and asserts that the
output contains the escaped script tag as characters, that no tag in the markup
is a `script`/`img`/`iframe`/`object`/`embed`/`a`, that no tag carries `href`,
`src=` or an `on…=` handler, and that the traversal string produces no
`<button>`. The tag scan matters: `onerror` *does* appear in the output, inside
an escaped text node, which is exactly the safe outcome — a naive substring
assertion would have failed for the right reason and passed for the wrong one.

### Architectural decisions

- **The view model is a separate pure module.** `apps/desktop`'s vitest project
  is a node one (`environment: 'node'`, `include: src/**/*.test.ts`), and
  changing that is outside this task's scope, so everything decidable was put
  where it can be tested without a DOM: the mapping, the pairing, the gaps, the
  dedup, and even the "am I at the bottom" predicate. The component is left with
  turning rows into elements.
- **The markup test renders to a string rather than to a DOM.** It needs no
  jsdom, runs in the existing project, and is a stronger assertion than querying
  a tree: escaped output is escaped whatever parses it next.
- **The feed owns its own scroll, and the rail body splits in two when
  following.** A long tool result must not push the files-touched list out of
  reach, and the pin needs one element whose bottom means "the newest entry" —
  which a body with the session list above it does not have. `.agent-rail__body`
  is `overflow-x: hidden` so the rail never scrolls sideways as a whole (§8);
  long lines wrap with `overflow-wrap: anywhere` and tool details bound their
  own height and scroll inside it.
- **The pin is a ref, and a session change is a remount.** Re-rendering on every
  scroll frame to store a boolean nothing draws would be the opposite of the
  point; and `<Feed key={session.id}>` resets the pin for a new conversation
  more honestly than an effect that reaches into a ref.
- **The files list is above the feed, not below it.** It is the actionable half
  and the reason the rail exists; a conversation that never stops growing must
  not be able to push it away.
- **Ordered by `seq`, not by `at`.** `at` is absent whenever the record carried
  no timestamp, and sorting by a missing value would reorder the list around
  nothing. `seq` is the order the core guarantees.
- **The client mirrors the core's ring bound (500).** Not a second policy: in
  the ordinary case the client's overflow is exactly the `dropped` the core
  reported for the same entries, which is why the larger of the two is taken
  rather than a sum. It exists so a client that somehow ran ahead cannot grow
  forever.
- **One open at a time.** A second click while `documents.status` is in flight
  is ignored rather than racing a second check.
- **`quiet: true` on `openFile`.** The refusal belongs in the rail's own error
  envelope; nothing was opened, so there is no pane for the pane's banner.

### Known limitations

1. **No manual pass was possible in this session** (no display, no ability to
   drive the app). Every acceptance criterion that needs the running app is
   unverified: the feed updating live beside a real Claude Code session, the
   scroll-up not being interrupted, and — the important one — the "open a file
   the agent just edited and see current contents" check, including the
   dirty-buffer conflict surfacing. The logic each depends on is unit-tested,
   but that is not the same claim (rule 12), and the **Manual** row of "Tests
   required" is therefore outstanding.
2. **`restarted` and `dropped` gaps were exercised from fixtures, not from a
   real truncation.** The core's own tests cover producing them; these tests
   cover drawing them.
3. **A `PERMISSION_PENDING` row cannot appear yet.** The core only produces it
   from hook evidence (T221). The row is written, styled and tested against a
   fixture; nothing in this build can make one arrive.
4. **The files-touched list does not mark a subagent's writes.** Deduplication
   is by path, and splitting the key by `sidechain` would show the same file
   twice. The feed marks who wrote it; the list answers "what changed".
5. **A file the agent touched before the ring overflowed disappears from the
   list.** The list is derived from the entries the core still holds, so it
   inherits their bound. That is history the core no longer has, not a directory
   the view failed to refresh — the `dropped` gap in the feed is what says so.
6. **`documents.status` re-checks every open document, not one.** The command
   takes an optional `documentId`, but the file being opened usually has no tab
   yet, and the controller's `checkStatus` is the whole-store call. For a
   handful of open tabs this is cheaper than adding a second path.

### Follow-ups

1. **A diff view for a touched file — recommended for ADR-007's `ChangeSet`,
   not for this rail.** The before-copies exist (`file-history-delta.backup`
   names one, under `~/.claude/file-history/<session>/<hash>@v<n>`), but reading
   them means reading outside the workspace root, which is a boundary decision
   with its own security argument (ADR-010 exists precisely to stop the renderer
   doing this). Beyond the boundary, three reasons put it on `ChangeSet` rather
   than here: the backup path is provider-specific, and folding it into
   `AgentTimelineKind` would put a Claude Code detail in a provider-neutral type
   (rule 4); a diff is a *review* surface with its own lifecycle — accept,
   revert, stage — none of which a read-only observation rail should grow; and
   `ChangeSet` is already the contract that owns "what changed", so a second
   answer here would be the duplicate abstraction rule 2 forbids. The rail's
   right-sized version is a link *into* that surface once it exists.
2. **`AgentTimelineAppendedPayload` carries no `transcriptEvidence` (T230).** A
   session whose transcript only becomes readable after `openTimeline` keeps the
   `false` from its snapshot until it is reopened. The UI is unaffected today
   because the "no transcript" note is only shown for an *empty* timeline, which
   is what the wire's own documentation prescribes — so this is left alone
   rather than reconstructed locally, and recorded here instead.
3. **Two pre-existing Rust tests are flaky under a loaded `pnpm check`.**
   Neither is in code this task touched (it makes no Rust changes), and both
   passed on re-run.
   - `agent_session::tests::a_changed_roster_emits_once_more` (T230) failed
     once with 3 rosters observed against 2 expected, then passed 6/6 runs of
     the whole crate and 5/5 in isolation. The cause looks precise:
     `AgentSessionManager::poll` uses `tokio::time::interval(poll_interval)` in
     `crates/gritty-core/src/agent_session/mod.rs`, which fires its **first tick
     immediately** regardless of the interval, so the background loop can scan
     and publish while the test is making its own two calls. The timeline loop
     beside it already avoids exactly this with
     `interval_at(now + interval, interval)`. Making the poll loop match would
     remove the race.
   - `crates/terminal/tests/pty.rs::kill_terminates_the_whole_process_tree`
     failed once at line 377 — the "the grandchild is running before the kill"
     precondition, not the kill itself — then passed 3/3 in isolation. It waits
     for a pid file and assumes the grandchild is still alive when it reads it,
     which a loaded machine can falsify.
4. **No component-level test infrastructure exists** (`apps/desktop`'s vitest
   project is node-only and its `include` is `*.test.ts`). The static-markup
   approach used here works and needs nothing new, but interaction — clicking a
   file row, scrolling the feed — is untestable until someone decides whether
   jsdom and a `*.test.tsx` include belong in that project.
5. **A very long feed re-derives its rows on every batch.** `timelineRows` is
   O(entries) inside a `useMemo` keyed on the timeline object, so a 400 ms tick
   re-maps up to 500 entries. Measured cost was not a problem at this bound; if
   the ring ever grows, incremental mapping is the fix rather than a smaller
   view.
