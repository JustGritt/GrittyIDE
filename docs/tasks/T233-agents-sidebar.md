# Task T233 — Agents in the left sidebar (design artboard 12a)

- **Phase:** 3
- **Status:** unspec
- **Depends on:** T210, T231, T232, T300-C, T300-WIRE

Split into `T233-M` (the row model), `T233-S` (the section and the sidebar
header) and `T233-C` (the full-height console, and retiring the rail). The
board carries the three; this file carries the argument they share.

## Objective

Move the agents from the right-hand rail into a pinned, collapsible section at
the top of the left sidebar, above the file tree, and give the selected agent
the editor area rather than a 320px column — artboard 12a's layout, arrived at
over design turns 10 and 11.

## The decision this settles

`docs/plans/execution-board.md:330` recommends **not** building this:

> `agents-panel.tsx:14` is explicit that the island keeps no roster of its own
> "so the two cannot report different counts", and a third surface reading the
> same roster is the duplication T231 argued against when it chose a rail over
> a tab.

The objection is answered by the artboard rather than argued with. **12a's body
has two columns — a 300px sidebar and the main area. There is no rail.** So this
is a move, not an addition: the surface count stays at three, and the case the
board named as the only one that would justify it is exactly what this is —

> a sidebar scoped to *this workspace's* agents against a rail that is
> machine-wide

— with the island keeping the machine-wide count. The sidebar also carries the
runs Gritty **owns**, which the rail never showed at all.

That reversal needs **ADR-026**, written by `T233-C` when the rail actually goes.

## Required reading

- Design project `4a56ac43`, `Gritty Agent Layout.dc.html`, artboards `10a`,
  `10b`, `11a`, `11b`, `12a`
- `docs/architecture/agent-runs.md` — the owned/observed table, and the rule
  that the two modules never import each other
- `docs/architecture/agent-sessions.md`, `docs/decisions/ADR-016-design-tokens.md`
- `docs/tasks/T231-agent-rail.md`, `T232-timeline-view.md`, `T335-agent-permissions.md`
  (its non-goals list is where this ticket came from), `docs/plans/execution-board.md`

## Allowed scope

- `apps/desktop/src/features/agents/**` (the section, the row model, the run
  status map, and — in `T233-C` — the console that replaces the rail)
- `apps/desktop/src/features/explorer/explorer-panel.tsx` (the header only)
- `apps/desktop/src/App.tsx` (composition only), `apps/desktop/src/app.css`
- `apps/desktop/src/features/session/persisted-layout.ts` and its test
- `apps/desktop/src/features/session/shell-restore.dom.test.tsx` (the two
  assertions the header rename breaks)
- `docs/decisions/ADR-026-*.md`, `docs/architecture/agent-runs.md`, this file,
  `docs/tasks/README.md`

## Explicit non-goals

Artboard elements with nothing behind them, named so the next reader does not
think they were forgotten.

- **The `CHANGES` tab.** 9a draws `Files / Changes`. `PROTO-W2-GIT` has just put
  `git.status` and `git.diff` on the wire; until a panel consumes them the tab
  would be one that can never have anything in it, which is what T210 refused
  when it cut 4b's five filters to three. The header stays an `<h2>` styled as
  the active tab — **not** a `role="tab"` in a tablist, because a lone tab that
  cannot be switched away from is an affordance that does not work. **T100-U**
  adds the second tab and the tablist with it.
- **"Run agent"** in the section header. The only `AgentRunProvider` is `FAKE`.
  A button that starts a fake agent where the artboard starts a real one is a
  claim this build cannot make; the DEV `RunsPanel` keeps the start form until
  T310 lands a real adapter.
- **The composer, `auto mode`, `step 4/6`, `3 files held`** — T335 already named
  these as needing T300 wiring and T223's opt-in link. Unchanged here.
- **The inline permission block** — built by T335.

## Required behavior

1. The section is **pinned above the tree, not a tab beside it**: an agent that
   stopped to ask must not be invisible because the tree is on screen.
2. Folded, it still shows a status mark per agent and the waiting count
   (artboard 11b). Folding may never hide an approval.
3. Owned runs and observed sessions appear in one list and are **visibly
   distinguished**. `owned` gates every control: a run Gritty started can be
   cancelled; a session another terminal owns cannot be touched.
4. The count is `AgentRoster.needsYou`, taken from the core and never
   recomputed, so the section and the island cannot disagree.
5. Observed sessions are filtered to the open workspace. The island stays
   machine-wide.
6. Every colour is an ADR-016 token. No literal hex.
7. Every agent string is text: never `innerHTML`, never Markdown, never a
   resolved link or path (T232 §7).

## Failure behavior

A roster that has not arrived says "Looking for agents…", not "none". A run
state or session activity from a newer core is shown as it arrived and marked
unrecognised rather than folded into a status this build would be guessing at.
A cancel is cooperative: the row says `cancelling` until an event says
otherwise, and nothing is optimistic.

## Acceptance criteria

- `pnpm check` green.
- Light and dark both legible; 800×600 scrolls rather than clipping.
- A `LAYOUT_VERSION` 5 blob parses to the new defaults rather than throwing.
- The section is exercised in the running app, not only in tests.

## Commands to run

```
pnpm check
pnpm dev          # and look at it, in both colour schemes
```

---

## Completion report — T233-M (the row model)

### Summary

Two pure modules under `apps/desktop/src/features/agents/`, no UI.

`run-status.ts` is the sibling of `agent-status.ts` for the ten `AgentRunState`
values, and it exists because there was no mapping at all: `runs-panel.tsx`
wrote `ops__row--${state.toLowerCase()}` and inherited the operations panel's
four rules, so of ten states exactly three had a colour and `FINISHED` had none
— the operations CSS spells that one `completed`.

`roster-model.ts` projects owned runs and observed sessions onto one `RosterRow`
**for display only**: no common interface is imposed on the domain, no field is
copied from one type onto the other, and the ids a row carries are how a caller
gets back to whichever type it came from. `owned` is the discriminator.

### Files changed

| File | What |
|---|---|
| `features/agents/run-status.ts` | new — `RunStatus`, `RunStatusTone`, `runStatus()` |
| `features/agents/run-status.test.ts` | new — 6 tests |
| `features/agents/roster-model.ts` | new — `RosterRow`, `RosterBadge`, `rosterRows()`, `activityLine()` |
| `features/agents/roster-model.test.ts` | new — 25 tests |

### Architectural decisions

1. **`RunStatusTone = AgentStatusTone | 'failed'`,** rather than widening
   `AgentStatusTone`. A session has no failure state — a process that died is
   `ENDED` — so widening the session union would make every session row's type
   admit a value no session can hold. Drawing `FAILED` in the grey `ended` tone
   would have downgraded the one outcome the user most needs to see.
2. **Ten run states collapse onto six tones, in three bands** — in flight,
   stopped-and-it-is-your-move, terminal. `REVIEW` is toned `done` rather than
   `blocked`: nothing is being withheld from it.
3. **Ordering is three bands, not a score.** A finer ordering would invent
   precision the two sources cannot agree on — a run has `updatedAt`, a session
   may have no timestamp at all. The sort is stable, so `selectRuns`' newest-first
   and the core's own session ranking both survive inside their band.
4. **`needsAttention()` in `runs-reducer.ts` finally has a caller.** It was
   written and tested with no consumer, which is precisely the "correct view
   model with no caller" defect the review has sent back three times.
5. **`titleCase` is duplicated** from `runs-reducer.ts` rather than exported
   across a ticket boundary. `describeRun` composes state and activity into one
   line for the dev panel; a row draws the state as a coloured mark and a word,
   so it needs the activity half alone. Three lines of presentation, and the
   two agree on the shape rather than on the code.

### Tests actually executed

`npx vitest run --project desktop run-status.test.ts roster-model.test.ts` —
**31 passed**. Then `pnpm check:ts` — **80 files, 671 tests, all passed**.

---

## Completion report — T233-S (the section, and the sidebar header)

### Summary

The Agents section is mounted as the first child of `<aside className="shell__sidebar">`,
above the explorer. It follows the **dock's** controlled label-button +
`[hidden]` idiom rather than the `<details className="panel">` idiom the two DEV
panels use, because it has to keep reporting a count while folded.

The explorer header is restyled as artboard 9a's section tab and renamed
**Files** — "Explorer" named the whole column, and the column now has something
above it.

### Files changed

| File | What |
|---|---|
| `features/agents/agents-section.tsx` | new — the section, its rows, the folded dot cluster |
| `features/agents/agents-section-render.test.ts` | new — 19 render tests |
| `features/explorer/explorer-panel.tsx` | `<h2>Explorer</h2>` → `<h2 class="explorer__tab">Files</h2>` |
| `App.tsx` | `useRuns`, the `rosterRows` memo, the section mounted in the sidebar |
| `app.css` | `+277` — the `.agents-section` block, `.agent-status--failed`, `.explorer__tab` |
| `features/session/persisted-layout.ts` | `agents: { open }`; sidebar default 244 → 300; `LAYOUT_VERSION` 5 → 6 |
| `features/session/persisted-layout.test.ts` | the standard trio for the new section |
| `features/session/shell-restore.dom.test.tsx` | two heading assertions follow the rename |

### Architectural decisions

1. **A row is a button only when it has somewhere to go.** An observed session
   has a timeline, so its row opens one in the rail. An owned run has no console
   until `T233-C`, so its row is a plain span with its Cancel beside it. A row
   that looked clickable and did nothing would be worse than one that plainly
   is not.
2. **Cancel is a sibling of the pick button, never inside it** — a button within
   a button is not markup a browser honours.
3. **The section defaults to open**, unlike the rail. It is the surface 12a
   makes primary, and folding still leaves the dots and the count, so the
   default costs nothing a user cannot take back.
4. **The status mark is entered through a `RunStatus`, not an `AgentActivity`**,
   because a row may be either kind. The classes are deliberately identical to
   `agent-status-badge.tsx`'s so both draw from one set of tokens and marker
   shapes. Folding the two into one component is worth doing once `T233-C`
   deletes the rail and leaves the island as the other caller.
5. **No new colour value.** 12a's palette maps onto existing ADR-016 tokens;
   the one new rule, `.agent-status--failed`, is built from `--error` /
   `--error-text` and given a rotated open square so a failure reads without
   colour vision.
6. **`LAYOUT_VERSION` 6** is not claimed in `docs/merge-queue.md` — that file
   tracks `PROTOCOL_VERSION` and `SCHEMA_VERSION`, and `parseLayout` never reads
   the version anyway; every field falls back independently.

### Tests actually executed

- `pnpm check:ts` — **82 files, 696 tests, all passed** (Biome clean, four tsc
  projects clean).
- `shell-restore.dom.test.tsx` gained one assertion, because it is the only test
  that mounts the real `App`: it now asserts `.agents-section` is in the
  document after a startup restore. That is the evidence the section survives
  composition into the shell rather than only rendering in isolation — the
  render tests cannot show that, and "a correct component with no caller" is the
  defect this repo keeps catching late.
- `npx vite build` in `apps/desktop` — **built in 1.75s**, no error. The
  >500 kB chunk warning is Monaco's and pre-existing.
- `pnpm check:rust` and `check:generated` were **not** run, and the results
  would not have been mine: this change touches no Rust, and the working tree
  carried another session's in-flight `PROTO-W2-GIT` work while it was made.

### Known limitations

- **Not yet seen on a screen.** There is no visual-regression tooling in this
  repo and this machine gave the agent no screen access, so the light/dark and
  800×600 acceptance lines are **unverified**. They are the first thing a manual
  pass should look at.
- **Two `useRuns` controllers in a DEV build** — `RunsPanel` still creates its
  own. Two subscriptions that converge on the same events, so it is waste rather
  than a defect, and `T233-C` removes it with the panel.
- The section shows a run's short id where 12a shows a workstream handle like
  `t090`. The workstream's *title* is the row's title; there is no short handle
  on `Workstream` to use instead.

### Follow-up tasks

1. Fold `agent-status-badge.tsx` and `StatusMark` into one component (after
   `T233-C`).
2. Hoist `useRuns` to `App.tsx` and pass the controller to `RunsPanel`, or
   delete the panel.
3. `T100-U`: add the `Changes` tab and turn the header into a real tablist.
4. A shortcut for the section. **⌥⌘A is taken** by the permission queue
   (`permission-banner.tsx:17`, bound at `App.tsx:130`); `T233-C` should inherit
   the rail's ⌥G. This would be the sixth hand-gated `keydown` listener — the
   shortcut registry T335 asked for is overdue.

---

## Completion report — T233-C (the console as a tab, and retiring the rail)

### Summary

The agent console is now a tab in the document strip, filling the editor area
when it is the active one, and the rail is deleted. **ADR-026** records the
reversal of T231's "no agent entries in the document tab strip" and — the part
that made it affordable — why the migration T231 priced it at does not apply.

The report was that a split-view markdown file was unreadable with the rail
beside it, and that an agent should behave "like a regular file". Both follow
from one change: the editor gets the rail's ~320px back, and selecting an agent
obeys T041's preview rule rather than a second one invented for it.

### Files changed

| File | What |
|---|---|
| `features/agents/agent-tabs-reducer.ts` (+test) | new — the tab state; `applyAppended` moved from `agent-rail-reducer.ts` unchanged |
| `features/agents/use-agent-console.ts` | new — the follower lifecycle, moved from `use-agent-rail.ts` |
| `features/agents/agent-console.tsx` (+render test) | new — the header over `AgentTimelinePanel`, reused verbatim |
| `features/agents/agents-section-chords.ts` | new — ⌥G, inherited from the rail |
| `features/editor/tab-strip.tsx` | agent tabs after the documents, from a second item source |
| `features/editor/editor-pane.tsx` | the console replaces the surfaces and the per-document toolbar |
| `features/editor/status-bar.tsx` | an agent branch beside `DocumentFacts` |
| `features/agents/roster-model.ts` | `retired`, `elsewhere`, `sourceLabel`; `partitionRows` |
| `features/agents/agents-section.tsx` | renamed `SidebarAgents`; the reveal, the envelope, the note, ⌥G |
| `App.tsx`, `app.css` | the rail's wiring and ~290 lines of its CSS removed |
| `persisted-layout.ts` (+test) | `agentRail` and `agentRailWidth` dropped |
| `docs/decisions/ADR-026-*.md`, `docs/architecture/agent-sessions.md` | the decision, and "## The rail" rewritten as "## The console" |
| **deleted** | `agent-rail.tsx`, `agent-rail-chords.ts`, `agent-rail-reducer.ts`, `use-agent-rail.ts` and their three tests |

### Architectural decisions

1. **The console never enters `documents.state`.** Verified end to end:
   `buildSessionTabs` is the only producer of `SessionTab`, walks only
   `documents.state.order`, and drops any id without a `DocumentTab`;
   `request.tabs` is assigned once; `tabs::replace` has one non-test caller. So
   there is no path to `TabRow.path`'s non-nullable column — no wire change, no
   ts-rs regeneration, no migration.
2. **`documents.state.activeId` is never cleared while a console shows.** That
   is what keeps restore correct: the last active document is still flagged
   `active: true`, so quitting with a console focused reopens the file you were
   on rather than falling back to the leftmost tab.
3. **Agent tabs are grouped after the documents**, not interleaved — the legible
   half of T231's objection, kept.
4. **A row is a button only if it has somewhere to go.** Owned runs carry an
   `AgentRunEventLog`, a different shape from `AgentTimeline`; the console does
   not draw it and the only provider is `FAKE`, so owned rows keep Cancel and
   nothing else. See *Known limitations*.
5. **Four things were moved out of the rail rather than deleted with it** —
   `activitySourceLabel` (T221 §24, whose only caller and only test were the
   rail's), the `processEvidence` note, the roster's error envelope, and
   out-of-workspace sessions, which `rosterRows` now flags rather than filters.

### Tests actually executed

- `npx vitest run --project desktop` — **51 files, 539 tests, all passed.**
- `pnpm typecheck` — all four projects clean. `biome check` clean.
- `npx vite build` — succeeded; the >500 kB warning is Monaco's and pre-existing.
- `pnpm test` (whole workspace) — **715 passed, 3 skipped, 1 failed**: the
  failure is `packages/editor`'s `model-eol` DOM test timing out its
  `beforeAll`, which is the board's own `MONACO-JSDOM-FLAKE`. Confirmed not mine
  by reproducing it with this work stashed; `W-flakes` holds that file and is
  fixing it now.
- `pnpm check:rust` was **not** run: no Rust is touched, and the working tree
  carries two other sessions' in-flight work.

Three tests exist specifically to stop this decision rotting:

- `persisted-layout.test.ts` — "an agent console is never persisted (ADR-026)":
  a console id parked in `order` produces no `SessionTab`; the active document
  stays flagged; the layout blob contains no session id.
- `agent-console-render.test.ts` — the §24 assertions, inherited from
  `agent-rail-render.test.ts` so the required behaviour cannot be lost silently.
- `shell-restore.dom.test.tsx` — the only test that mounts the real `App`, now
  asserting the section is present *and* the rail is gone.

### Known limitations

- **Not seen on a screen.** No visual-regression tooling, no screen access on
  this machine, and the chrome-devtools MCP server would not connect. The
  light/dark and 800×600 lines are unverified and are the first thing a manual
  pass should check.
- **A run Gritty owns still has no console.** Named in ADR-026; T310 earns it.
- **Agent tabs cannot be reordered or dragged among the documents.**
- One test-only sharpening: `agent-console-render.test.ts` blanks attribute
  *values* before its tag scan, because the console puts a working directory in
  a `title` and an escaped `src=` inside one is the safe outcome, not a finding.

### Follow-up tasks

1. A shortcut registry. ⌥G is the sixth hand-gated `keydown` listener; T335
   asked for this and it is now more overdue.
2. Fold `agent-status-badge.tsx` and the console's `StatusMark` into one
   component — the rail is gone, so the island is the only other caller.
3. Hoist `useRuns` to `App` and pass the controller to the DEV `RunsPanel`, or
   delete the panel; there are two controllers in a dev build.
4. `T100-U`: the `Changes` tab, and turning the sidebar header into a real
   tablist.

---

## Feed legibility pass — 2026-09-07

Reported after looking at the console in the running app: *"missing color … a bit
hard to read between thoughts, prompts and used tools."*

### What was actually wrong

The model was not the problem. `agent-timeline-model.ts` already assigns one of
ten `TimelineTone`s to every row. Three things stopped any of it reaching the
screen:

1. **Every label was `--fg-muted`.** `YOU`, `AGENT`, `TOOL` and `THINKING` were
   rendered in the same grey, so the one word that names a row's kind carried no
   information.
2. **A tool row put its *name* in the prominent slot and its *command* in a
   well.** `text` is `kind.name` ("Bash") and `detail` is the command — so the
   least informative part was bright and the part the reader came for was
   recessed. Worse, the command and its output were both `.agent-feed__detail`
   with the same fill, which is what made a run of calls read as one grey wall.
3. **`.agent-feed__row--tool` used `--accent-fill`** (`#22303f`), which against
   `--surface-editor` is very nearly no edge at all — so tool rows read as
   unmarked whatever their outcome.

### What changed

- **A tool's name moved into the head** beside its label (`TOOL · Bash · ok`),
  and the command became the row's subject: bright, `--fg`, on the page rather
  than in a well. The result stays recessed and drops to `--fg-faint`, so input
  and output are finally distinguishable.
- **The row's left edge carries the outcome** — `--ok`, `--error`, `--accent`
  while running — which is artboard 12a's own pattern for a command block. A
  *failed* call additionally gets `--error-wash` behind it; success does not,
  because most calls succeed and a feed where every row is tinted has no
  emphasis left to spend.
- **Labels are toned per kind**: prompt `--accent-text`, file `--ok-text`,
  thinking and tool `--fg-faint`, notice and permission `--warn-text`.
- **The prompt anchors a turn** — `--accent-wash` behind it and space above it,
  the same treatment the permission row already had, because it is the row a
  long feed is skimmed *for*.
- **The agent's prose reads as prose**: `--text-code` at 1.6 line-height, a step
  larger and looser than the machine lines around it.
- **Thinking is subordinate on three channels at once** — dimmer, italic, and a
  dashed edge where an answer has a solid one — because it is the thing most
  often skipped and it has to be skippable.

Colour is never the only channel (T231 §10): the badge still says `ok` /
`failed` / `running` in words, and the edge style — solid, dashed — differs
alongside the hue.

### Tests actually executed

`agent-timeline-render.test.ts` gained five cases, because the existing ones
asserted escaping and gaps and would have passed against any of this: the tool
name is in the head and not the body, input and result are distinguishable, the
outcome is on the row and not only in the badge, every kind emits its tone
class, and the outcome is still stated in a word.

`npx vitest run --project desktop` — **51 files, 544 tests, all passed.**
`pnpm typecheck` and `biome check` clean. `npx vite build` succeeded.

Every value is an ADR-016 token, so the light ramp follows without a second
palette. **Still not seen on a screen** — the same limitation as the rest of
T233-C.
