# Review findings: the twelve tasks sitting at `review`

T049, T051, T096, T098, T200, T210, T220, T221, T230, T231, T232 and T325 are
all at `review` on the board, and all of their code is already on `main`
(`e32ee97`). What is missing is a person's sign-off. This document is the
paperwork half of that, done first, so the 105 minutes in
`docs/testing/phase-0-manual-pass.md` go to the checks only an eye can make.

## What was checked

Each task's `## Completion report` was read against the tree at `e32ee97`:
the commits and pull requests that landed it, the files it says it changed,
the tests it says it ran, the acceptance criteria and "Tests required" lists
in its own spec, the follow-ups it promised, and the claims it makes about
things on a screen.

## What was not checked, and why

**No build and no test was run.** `cargo test -p gritty-core` regenerates
`packages/protocol/src/generated/` as a side effect; a parallel implementation
branch is live in another checkout, and running it here would dirty this one
and collide with theirs. `pnpm check` reaches the same code path. Where
evidence that `main` compiles and passes is needed, CI is the citation: the
run for `e32ee97` (`Merge pull request #24`, id `34022386676`) is
`completed success`, as is the run for `7a4f580` (`#21`, id `34020924746`).
The two `cancelled` runs above them are the doc-only merges `#22` and `#23`,
superseded within seconds by the next push.

Test *counts* were reconciled arithmetically instead of by execution: Rust by
counting `#[test]` and `#[tokio::test]` attributes per crate at each commit,
TypeScript by counting `it(`/`test(` call sites and adding the 28 rows the two
`it.each` blocks in `packages/markdown/src/resolve-link.test.ts` expand to.
That method is exact against every headline number in the pile except one
(see the measurement note), which is itself the strongest evidence in this
document that the reports are written honestly.

**No verdicts are proposed.** An agent does not set a task `done` in this
repository, and nothing below decides whether a task passes.

## The one-line verdict

Seven discrepancies across twelve tasks, and none of them is a report claiming
work it did not do. Six of the seven are gaps in what is *tracked* — a feature
delivered by the core and drawn by nothing, a required behaviour demoted to a
suggestion, a runbook step that cannot be performed, three tasks landed in one
unreviewed commit alongside a fourth that has no ticket at all. Every "not
executed" admission in the twelve was checked and every one of them is true.

**Findings 1, 2 and 3 were fixed on 2026-09-06** and are stamped where they
stand. Findings 4 to 7 are about history and paperwork rather than code, and
are open.

---

# Discrepancies

Ordered by how much each would change a reviewer's decision.

## 1. The permission prompt is delivered by the core and drawn by nothing

> **Resolved 2026-09-06** — `T232: the feed draws the permission prompt it is
> already sent`. `applyAppended` carries `pending` through and `timelineRows`
> draws it as the last row. See T232's fix pass.

**What the reports claim.** T232's summary: "Selecting a session now draws the
files it touched and, under them, its feed — prompts, replies, thinking, tool
calls with their results attached, notices, **permission prompts** and subagent
work". Its known limitation 3 blames the core: "A `PERMISSION_PENDING` row
cannot appear yet. The core only produces it from hook evidence (T221)…
nothing in this build can make one arrive."

**What the tree shows.** The core does produce it, and T221 landed in the same
pull request (`#20`, commits `1eac44b` for T232 then `efb4ea1` for T221), so
the sentence is stale on `main`. T221's own decision was to hold the prompt
*beside* the ring rather than in it — `crates/gritty-core/src/agent_session/
timeline.rs:231` builds the entry, and `timeline.rs:171` and `:199` publish it
as `AgentTimeline.pending` and `AgentTimelineAppendedPayload.pending`
(`packages/protocol/src/generated/AgentTimeline.ts`, field `pending?:
AgentTimelineEntry`).

The renderer never reads that field. `timelineRows()` at
`apps/desktop/src/features/agents/agent-timeline-model.ts:219` maps
`timeline.restarted`, `timeline.dropped` and `timeline.entries` only.
`applyAppended()` at
`apps/desktop/src/features/agents/agent-rail-reducer.ts:104` returns
`{ ...timeline, entries, dropped, restarted }` and drops `batch.pending` on
the floor. Grepping `.pending` across the same directory finds only
`pendingId` (the in-flight selection request) and a `pending: boolean` row
prop at `agent-rail.tsx:195`. The `case 'PERMISSION_PENDING':` arm at
`agent-timeline-model.ts:178` is reachable only from a ring entry, and the
core never appends one — `timeline.rs:728` marks that path
`unreachable!("T221 only")`.

So the feature the hook bridge exists to deliver arrives at the renderer and
stops there. T221 records this accurately as a follow-up ("T232's timeline view
should render `AgentTimeline.pending` — the entry exists and is delivered, but
nothing draws it yet"); nothing else does, and no ticket carries it.

**Time to confirm.** Five minutes: the two line numbers above and one grep.

## 2. T221 §24 is not implemented, and its report describes the state wrongly

> **Resolved 2026-09-06** — `T221: a row says where its verdict came from`. The
> rail row draws `activitySourceLabel`, and the follow-up sentence is corrected.
> See T221's fix pass.

**What the report claims.** Under Follow-up: "Consider surfacing
`activitySource` on the rail row; `activitySourceLabel` is exported and tested
but **only the settings surface is wired today**."

**What the spec required.** §24 of `docs/tasks/T221-agent-hook-bridge.md` is a
*Required behavior* item, not an option: "A row can say where its verdict came
from, so a `BLOCKED` row is attributable rather than magic."

**What the tree shows.** `activitySource` is on the wire
(`packages/protocol/src/generated/AgentSession.ts:21`) and is read by no
component: the only non-test reference in `apps/desktop/src` is the definition
of `activitySourceLabel` at
`apps/desktop/src/features/agents/agent-status.ts:103`, whose only caller is
`agent-status.test.ts`. `agent-integration-settings.tsx` never mentions
`activitySource`, so the settings surface is not wired to it either.

A required behaviour was therefore demoted to "consider", and the sentence
explaining where it currently lives is inaccurate. Nothing was claimed as done
that was not done — but a reviewer reading the follow-up would conclude the
label is on screen somewhere, and it is not.

**Time to confirm.** Two minutes.

## 3. Manual-pass step 7.7 cannot be performed: nothing raises a decision

> **Resolved 2026-09-06** — `T210: raise a question from the island and the
> queue`. The roster and the queue both raise questions, and steps 7.7a–7.7c
> put a decision on screen for 7.7. See T210's fix pass.

**What the runbook asks.** `docs/testing/phase-0-manual-pass.md`, Session 7,
step 7.7: "Open the island (⌥A) onto a decision with options, press ⌥1 | It
answers the decision. The dock does not switch."

**What the tree shows.** T210's report says it plainly, and it is true:
"**Raising a question has no UI at all yet**: decisions can be answered and
dismissed, but the only way to create one is the command." `raise` is defined
on the controller at
`apps/desktop/src/features/workstreams/use-workstreams.ts:191` and exported at
`:286`; no component calls it. `island.tsx` and `attention-queue.tsx` reference
only `decision.raisedAt` and `decision.raisedBy`.

The reviewer will reach 7.7 with no way to get a decision on screen. The step
also indirectly gates T096's chord arbitration ("with the island open, ⌥1…9
answer its decision and never reach the dock", a Failure-behavior item), so
skipping it leaves a T096 behaviour unexercised as well. T210's own first
follow-up is the missing form, and nothing tracks it (see the follow-ups
table).

**Time to confirm.** Three minutes. Working around it needs a way to call
`decisions.raise` outside the UI, which is longer.

## 4. T096, T098 and T220 landed in one unreviewed commit carrying a fourth,
unticketed feature

**What the reports imply.** Three separate task files, three separate scopes,
and T096 lists an acceptance criterion of its own: "Nothing in `crates/`,
`packages/protocol` or `src-tauri` changed."

**What the tree shows.** `cb9e08a`, "Checkpoint: T096/T098 terminal chrome,
T220 agent discovery, preview tabs" — 81 files, +6911/−407. It is the only
non-merge commit in `main`'s entire first-parent history
(`git log --first-parent origin/main`): every other change on `main` since the
initial commit arrived through a pull request. `gh pr list --state merged`
has no entry for it.

Two consequences for a reviewer:

- **T096's "nothing in `crates/` changed" cannot be checked against history.**
  That commit modifies `crates/gritty-core/src/terminal/{mod,wire}.rs`,
  `packages/protocol/src/{client,commands,types}.ts` and
  `apps/desktop/src-tauri/src/{commands,core,lib}.rs`. Those changes are
  T098's and T220's, and both claim them; the criterion is plausibly satisfied
  for T096's own diff, but nothing in the repository separates the three.
- **A fourth body of work rode along with no ticket.** The commit message names
  it: "Editor — preview tabs… Also: tab close-plan and its menu, editor binding
  and close-flow changes." In files that is
  `apps/desktop/src/features/editor/close-plan.ts` (new) and
  `close-plan.test.ts` (new, 107 lines), `use-close-flow.ts` (+92),
  `use-editor-binding.ts` (+44), `tab-strip.tsx` (+81),
  `features/documents/documents-reducer.ts` (+101) and its test (+151),
  `use-documents.ts` (+67), `packages/editor/src/monaco/editor-host.ts` (+8).
  There is no task file for it, no row in
  `scripts/agent-board.py list`, no line in `docs/tasks/README.md`, and no step
  in the manual pass. It changes user-visible editor behaviour — a single
  click now opens a preview tab that the next click replaces — that a reviewer
  walking Session 2 will meet without a specification to check it against.

  T096's report describes `tab-strip.tsx` as "the import"; the commit adds 81
  lines to it, and the excess belongs to this untracked work rather than to
  T096.

**Time to confirm.** Ten minutes: `git show --stat cb9e08a`, then the board and
the task index.

## 5. T096 and T098 report no tests at all

**What the reports contain.** Both completion reports run Summary → Files
changed → Architectural decisions → Known limitations → Follow-up tasks
(`docs/tasks/T096-terminal-chrome.md:129`,
`docs/tasks/T098-terminal-colours-and-rooting.md:123`). Neither has a "Tests
actually executed" section, which step 5 of `AGENTS.md`'s task workflow
requires: "tests actually executed with results".

**What the tree shows.** The tests themselves are complete, so this is a
reporting gap and not a coverage one:

- T096's spec asked for `terminal-naming.test.ts` covering every `pillLocation`
  branch, the three `pillTone` cases and `pillStatus`, and a
  `terminal-chords.test.ts` covering `Digit1`…`Digit9`, `Digit0`, the modifier
  combinations and the `code`-not-`key` point. Both files exist, with 25 and 6
  cases. No pre-existing terminal suite was edited, which was the spec's
  boundary test.
- T098's spec asked for five Rust behaviours and four Tauri ones. The five are
  covered by four tests at `crates/gritty-core/src/terminal/mod.rs:1434`,
  `:1455`, `:1479` and `:1501` — the "present in the `list` snapshot" case is
  asserted inside the first. The four Tauri behaviours are all asserted inside
  one test, `a_terminal_can_be_coloured_and_the_colour_survives_a_relist` in
  `apps/desktop/src-tauri/tests/commands.rs`: set, relist, clear, and `TEAL`
  refused as `InvalidInput`. `terminal-colors.test.ts` and
  `terminal-cwd.test.ts` carry 5 cases each.

The evidence for the suite as a whole exists only in `cb9e08a`'s commit
message: "cargo fmt/clippy clean, cargo test --workspace green, tsc clean, 386
vitest tests green, biome clean". That 386 is corroborated — the tree at
`cb9e08a` has 358 `it(`/`test(` call sites plus the 28 `it.each` rows — but a
commit message is not where a completion report goes.

**Time to confirm.** Five minutes.

## 6. Seven of the twelve have no session in the manual-pass runbook

**What the runbook claims.** `docs/testing/phase-0-manual-pass.md`: "This is
every outstanding human check in one ordered session. It replaces the 'Not
executed (human WKWebView pass)' lists spread across the T040, T045, T048,
T049, T050, T060, T090, T096 and T098 completion reports."

**What the tree shows.** That claim is true for the nine tasks it names.
T049 is Session 1B; T096 and T098 are Session 7. But T051, T210, T220, T221,
T231, T232 and T325 each end with an explicit "not executed" manual claim and
have no session, no step and no mention. T200 and T230 need none — both are
core-only and say so truthfully.

So the runbook is not the complete worklist it says it is, and a reviewer who
runs it end to end will still have seven tasks' perceptual claims outstanding.
The next section is that missing worklist, written so it can be folded in.

**Time to confirm.** Five minutes to verify the absence; drafting the sessions
is the real cost.

## 7. Files touched but not listed

Five cases, one of them substantial:

- **T325** added `docs/plans/Gritty Agent Runtime Coordination.md` — 1,726
  lines, status `A` in `9985978` — and does not list it under "Files changed".
  Its own Required reading cites the file ("§§12–23, §37, §40") as though it
  predated the task, and its Allowed scope does not include it. Also unlisted:
  `crates/gritty-core/src/workstream/wire.rs` (one line, renumbering a doc
  comment from T330 to T335) and the regenerated
  `packages/protocol/src/generated/Decision.ts` that follows from it.
- **T049** counts tests in three files it does not list:
  `apps/desktop/src-tauri/tests/commands.rs` (+75),
  `crates/persistence/tests/database.rs` (+94) and
  `packages/protocol/src/workspace-client.test.ts` (+33), all in `a45be54`.
- **T210** says "This is the only core change in T210" while `ee6627b` also
  modifies `crates/gritty-core/src/workstream/tests.rs` and
  `packages/protocol/src/workstream-client.test.ts`, neither listed.
- **T096** describes `tab-strip.tsx` as "the import" (see finding 4).
- **T221** does not list `Cargo.lock`, which `efb4ea1` modifies.

Nothing here is a claim of work not done. The T325 case is worth a minute of a
reviewer's attention because a 1,726-line planning document arriving inside an
implementation commit is the kind of thing that should be its own change.

**Time to confirm.** Two minutes.

## Measurement note: the numbers reconcile

This is not a finding. It is recorded so that a reviewer who spot-checks a
count and finds it off by one knows it has already been chased.

Every headline test total in the twelve reconciles exactly against the tree.
Rust, counting test attributes per crate at each commit and separating the
three `crates/gritty-core/tests/crash_recovery.rs` cases that cargo reports as
their own binary:

| Report | Claim | Tree |
|---|---|---|
| T200 (`dbf66cb`) | `gritty-core` 160 | 163 attrs − 3 = 160 |
| T051 (`c6ec1a2`) | 163 (3 new) | 166 − 3 = 163 |
| T220 + T098 (`cb9e08a`) | 177 + 4 new | 184 − 3 = 181 |
| T230 (`d2afd55`) | 209 + 3, agents 52 | 212 − 3 = 209, agents 52 |
| T221 (`efb4ea1`) | 229 + 3, agents 94, hook 13 + 1 | exact on all four |
| T232 (`1eac44b`) | 209 core, 52 agents | exact |
| T325 (`9985978`) | 181 → 232 (+51), persistence 15 → 18 | exact on both |

TypeScript, counting call sites and adding the 28 `it.each` rows: T049 293,
T200 297, T210 318, T051 320, T231 410, T325 410, T232 454 — all exact.
T220's 363-in-53-files is the only one that does not reconcile, and its own
report explains why: it was measured on a tree another session had mid-edit,
which is also why it reports `pnpm check` as not green.

Three per-file breakdowns drift, none of them into a total: T049 says "8
shortcut vitest cases" where `open-shortcuts.test.ts` has 7; T232 says
`agent-timeline-model.test.ts` has 21 cases where it has 22, says "9 new
cases" in `agent-rail-reducer.test.ts` in a sentence whose own parenthetical
says 5 + 6 and whose diff adds 11, and says "41 of those tests are new" where
the diff adds 44; T221's 424 is one above the 423 the tree implies.

---

# Claims requiring a human at the window

Sixty-six items across eleven tasks. T200 has none and says so correctly.
Where the check already exists in `docs/testing/phase-0-manual-pass.md` the
step number is given; where it does not, the task has no session at all
(finding 6) and a home is suggested.

## T049 — startup screen (Session 1B, all 21 steps)

1. The launcher appears on a first run, after Close workspace, and when the
   last root vanished — 1B.1, 1B.3, 1B.16.
2. It never flashes during a successful restore — 1B.21.
3. The native macOS picker, not an HTML dialog — 1B.2.
4. Clicking a recents row, and the Reopen tile — 1B.5, 1B.6.
5. Forget on hover *and* on keyboard focus; the row goes, and so does the
   session notice on the next launch — 1B.14, 1B.15, 1B.17.
6. ⌘O and ⌘⇧O everywhere, stealing nothing from Monaco or a live shell —
   1B.8 to 1B.10.
7. Light and dark both legible; the panel scrolls rather than clipping at
   800×600 — 1B.19, 1B.20.
8. Whether the dashed drop target is actually visible — 1B.11. The report is
   explicit that only `DropHoverChanged` was observed.

Note for the reviewer: T049 is the one task in the twelve whose report
contains visual observations — "Rendered in Chrome against a vite dev server
at 1400×900, 800×600 and 760×430, dark and light… Console clean", and three
defects "found by looking", including a reversed parent path. That was a
browser, not WKWebView, and the report says so; 1B is still first contact. The
one gesture a person did perform — dropping a folder onto the launcher twice on
2026-09-04 — is dated and attributed in the report and needs no repeat.

## T051 — launcher resume rail (no session; belongs after 1B.3)

1. Closing a workspace with tabs open puts the rail on the launcher.
2. Restore brings the same tabs back in the same order, with the same active
   tab, view modes and cursors.
3. Unsaved buffers are counted and named, and are still dirty after restoring.
4. Opening a workspace makes the rail disappear; closing again brings a fresh
   one.
5. The rail never appears for a workspace that had no tabs.
6. "Open without them" opens the folder only, and Dismiss hides without
   discarding.

## T096 — terminal chrome (Session 7, steps 7.1 to 7.16)

1. One 32px row collapsed or not, with the pills still readable collapsed —
   7.1, 7.2.
2. Three shells reading `zsh GrittyIDE`, `zsh crates/terminal`, `zsh ~` — 7.3.
3. ⌥2 from inside a focused terminal switching and printing nothing; ⌥7 with
   three sessions doing nothing — 7.4, 7.5.
4. ⌥1 during a rename typing into the field; ⌥1 with the island open answering
   the decision — 7.6, 7.7. **7.7 is currently unperformable (finding 3).**
5. ✕ leaving `exit N` behind, ✕ again removing the pill, right-click Close in
   one step — 7.8 to 7.10.
6. The rename field not making the row jump, and the ✕'s zero-opacity reserved
   width not reflowing the row under the pointer — 7.11. The reserved width is
   reasoned in the report, never seen.
7. The caret listing only `available` profiles, the choice surviving a restart,
   and `⋯` opening upward and fully on screen — 7.12 to 7.14.
8. The dock at its 120px minimum, and the whole bar in the Light ramp — 7.15,
   7.16.

## T098 — session colours and rooting (Session 7, steps 7.17 to 7.27)

1. Right-click → Blue drawing a bar on the pill's left edge while the dot still
   says whether the shell is alive — 7.17.
2. Terminating that shell: the bar stays, the dot turns red — 7.18.
3. The radio tick on the current colour, and "No colour" removing the bar —
   7.19, 7.20.
4. Colours surviving a renderer reload, which is what makes them core state —
   7.21.
5. "New terminal" rooting at the active document's folder, and the tooltip
   saying where before the shell exists — 7.22, 7.23.
6. The root and `~` fallbacks, including the silent fallback when the folder
   was deleted under the tab — 7.24 to 7.26.
7. All six hues visible against the bar in the Light ramp — 7.27.

## T200 — workstream model

None. The report's "Not executed: nothing in the running app — T200 adds no
UI" is accurate; the task ships no renderer code.

## T210 — island and attention queue (no session)

1. A workstream taken from `DRAFT` to `DONE` entirely through the island.
2. ⌥A toggling; Escape and a click outside closing.
3. The sheet never pushing the tab strip down or occupying a layout track.
4. A question showing in the island within one event, and answering it
   resuming the work without a reload.
5. The queue and the island agreeing at all times, in both directions.
6. The roster's transition buttons appearing on hover *and* on keyboard focus.
7. A refused transition leaving the row where it was and showing the core's
   message rather than moving optimistically.

Items 1 and 4 depend on a decision existing, which finding 3 blocks.

## T220 — agent session discovery (no session)

1. The island's agents section rendering at all — the report says its
   "rendering of the section has not been seen in the running app".
2. A live Claude Code session in another terminal appearing with its task and
   the tool in flight.
3. Quitting that session flipping the row to `ENDED` within one poll interval.
4. With the process table unreadable, the surface explaining itself rather
   than showing everything as Ended.

The report's "Manual, against real machine state (three live Claude Code
sessions)" is a core-level observation, not a screen one, and it is
unrepeatable: no fixture pins that machine state, so a reviewer cannot
reproduce it and does not need to.

## T221 — hook bridge (no session; the highest-stakes group here)

1. Installing from the settings surface, with the proposed change shown before
   it is applied.
2. A real permission prompt flipping the row to `BLOCKED` with
   `activitySource: HOOK` within one interval, and answering it clearing.
3. Killing the agent while it sits at the prompt reading `ENDED`, not
   `BLOCKED`.
4. Uninstalling and diffing against the backup — every foreign entry
   byte-identical.
5. **The whole path against a real `~/.claude/settings.json`.** The report's
   own remaining risk: "None of them has been exercised against a real
   `~/.claude/settings.json`." The installer is the only thing in the
   repository that edits a file Gritty does not own, and the worst case is a
   configuration nobody can recover. The fixture coverage is exhaustive —
   `crates/agents/src/integration.rs` carries 22 tests including
   `installing_beside_another_tool_leaves_every_one_of_its_entries_intact`,
   `key_order_and_indentation_survive`,
   `a_malformed_config_is_refused_and_left_exactly_as_it_was` and
   `a_read_only_config_is_refused_before_anything_is_written` — but a fixture
   is not the user's file.
6. Where the settings control actually sits. §25 says "in settings, not in the
   roster"; it renders in the island's agents-section header
   (`apps/desktop/src/features/agents/agents-panel.tsx:38`), which the code
   argues is settings. Worth an eye rather than an argument.

Note: the hook binary is not bundled as a sidecar (recorded honestly as a
known limitation), so a packaged build reports `hookAvailable: false`. Session
6's packaged run cannot exercise any of the above.

## T230 — agent timeline (no session needed of its own)

1. Opening a timeline against a real running session and watching it grow.
   The report defers this to T231's session, which is the right place for it.

Everything else in T230 is Rust and is covered: the follower's
offset/partial-line/truncation/bounded-tail cases, the fold, path resolution,
the ring, the lifecycle including
`closing_a_timeline_stops_its_follower` asserted against `FollowMetrics`, and
a measured cost table for a 400 MB transcript.

## T231 — the agent rail (no session)

1. The rail listing a live session and its colour changing as the session
   does.
2. Dragging the gutter to resize, and the width surviving a restart.
3. A layout blob written before the rail existed loading at the default
   (`LAYOUT_VERSION` is 5 at
   `apps/desktop/src/features/session/persisted-layout.ts:11`).
4. Collapsing the rail keeping the needs-you count visible.
5. **⌥G from inside a focused terminal switching and inserting no `©`.** The
   report is explicit: "The chord from inside a focused terminal is reasoned,
   not observed" — source-read from xterm 6.0.0, not run.
6. ⌥G not firing while the island is open, and not fighting the dock's ⌥1…⌥9
   or the island's ⌥A.
7. The five status markers — filled disc, diamond, square, ring, bar —
   distinguishable at rail size without colour vision.
8. A narrow window shrinking the rail toward its collapsed strip rather than
   squeezing the editor below its minimum.

## T232 — timeline view (no session)

1. The feed growing live beside a real Claude Code session.
2. Scrolling up and staying there not being interrupted by new entries.
3. **A file the agent just edited opening with current contents, and a
   dirty-buffer conflict surfacing normally.** This is the criterion the rail
   exists for; the flow is unit-tested
   (`open-touched-file.test.ts`, "checks the disk before it opens anything",
   "reuses the tab that is already open for that path") but never performed.
4. A file the agent touched outside the workspace being visible and not
   clickable.
5. A dropped or restarted timeline drawing an explicit gap. Exercised from
   fixtures only; the report says so.
6. The rail never scrolling sideways as a whole, and a long tool result not
   pushing the files-touched list out of reach.

## T325 — ownership registry (no session)

1. The island's Ownership section and its claim form.
2. `owns N` on a roster row.
3. The fourth filter and the conflict card in the attention queue.
4. A refusal reaching the queue and naming the holder.
5. **Whether the wording actually stops a user believing a lease protects a
   file.** The report names this as its main risk: "The registry is
   believable, and it protects nothing." It is a perceptual judgement no test
   can make, and it is the single most useful thing a person can decide about
   this task.

---

# Follow-ups promised but not tracked

"Tracked" means a board ticket (`scripts/agent-board.py list`), a task file in
`docs/tasks/`, or a recorded limitation in an ADR. "Task file only" means the
item exists nowhere but the completion report that promised it.

| Follow-up | Promised by | Tracked as |
|---|---|---|
| Perform the manual pass for the startup screen | T049 | Session 1B of the manual pass |
| Git branch badge on a recents row | T049 | Task file only (T100 is an `unspec` ticket with no file) |
| Reopening a recent workspace restores its persisted tabs | T049 | Task file only |
| Snapshots under a forgotten root are never offered or collected | T049 | `docs/decisions/ADR-014-session-persistence.md:124` |
| Persist terminal sessions per workspace | T051 | Task file only |
| The rail could name *why* the last session did not come back | T051 | Task file only |
| Shell integration and command blocks | T096 | T097 (`todo`, task file, ADR-017) |
| A jsdom vitest project so `.tsx` can be tested at all | T096 **and** T232 | Task files only — promised twice, tracked neither time |
| Reordering terminal pills by drag | T096 | Task file only |
| Artboard 8b's session rail | T096 | Task file only |
| Colour the terminal surface's edge | T098 | Task file only |
| A colour for a workstream | T098 | Task file only (T300 is `unspec`) |
| Workstream UI | T200 | T210 (`review`, task file) |
| A `change_set` table referencing `workstream_id` | T200 | Task file only — ADR-007 does not record it |
| Cross-workspace attention | T200 | T410 (`unspec` ticket, no file) |
| `session.restore` reporting workstreams needing attention | T200 | Task file only. T051 shipped the resume rail afterwards without it |
| **Raise a question from the UI** | T210 — its own "largest gap and the first follow-up" | **Nothing.** Blocks manual-pass step 7.7 (finding 3) |
| Edit title and goal, and remove a workstream, from the roster | T210 | Task file only |
| Hook bridge | T220 | T221 (`review`), ADR-018 |
| More providers (Codex, Cursor, Gemini) | T220, T221, T230 | T222 (`todo`, task file) |
| Link a session to a workstream | T220 | T223 (`doing`, task file) |
| Jump to the terminal a session runs in | T220 | T224 (`todo`, task file) |
| `AgentSessionsChanged` should become a delta if the roster grows | T220 | Task file only |
| **Bundle `gritty-agent-hook` as a sidecar resource** | T221 | **Nothing.** T221's own Allowed scope named `tauri.conf.json` for exactly this; without it a packaged build reports `hookAvailable: false` |
| **Render `AgentTimeline.pending`** | T221 | **Nothing.** This is finding 1 |
| **Surface `activitySource` on a rail row** | T221 | **Nothing.** This is finding 2, and §24 required it |
| A signal for "your timeline closed because the workspace changed" | T230 | Task file only |
| A keys-only summary for `tool_detail`'s fallback | T230 | Task file only |
| Timeline view and files touched | T230, T231 | T232 (`review`, task file) |
| Extend `attachCustomKeyEventHandler` to refuse the rail's chord | T231 | Task file only |
| Decide where the rail lives with no workspace open | T231 | Task file only. Its own Failure-behavior row ("Workspace closed → the rail keeps listing") does not hold, and the report says so |
| Name the rail's collapsed strip width as a token | T231 | Task file only |
| A diff view for a touched file, on ADR-007's `ChangeSet` | T232 | Task file only — ADR-007 does not record it |
| `AgentTimelineAppendedPayload` carries no `transcriptEvidence` | T232 | Task file only |
| `crates/terminal/tests/pty.rs::kill_terminates_the_whole_process_tree` is flaky under load | T232 | **Nothing.** Its sibling flake, `a_changed_roster_emits_once_more`, was fixed after the report by `2b3a45e` (`interval_at` at `crates/gritty-core/src/agent_session/mod.rs:483` and `:683`); the PTY one was not |
| Incremental row mapping for a very long feed | T232 | Task file only |
| Observed ownership violations | T325 | T326 (`unspec` ticket, no file) |
| Write interception, after an ADR-018 amendment | T325 | T330 (`unspec` ticket, no file) |
| Ownership-aware scheduling | T325 | T405 (`unspec` ticket, no file) |
| Annotate the touched-file list and the editor with the owning workstream | T325 | Task file only |
| A history view of released leases | T325 | Task file only |

Two entries deserve the reviewer's attention above the rest: **"Raise a
question from the UI"**, because T210's report calls the island not usable end
to end without it and it has no home at all, and **the sidecar bundling**,
because it is the difference between T221 working in `pnpm dev` and doing
nothing in a packaged application.

---

# Tasks that check out clean

- **T200 — workstream model.** Files listed match `dbf66cb` exactly; the
  state-machine, wire, manager and persistence tests all exist; protocol 2.2.0
  → 2.3.0 and schema 1 → 2 are what the commit does; the counts reconcile
  exactly; and "Not executed: nothing in the running app — T200 adds no UI" is
  true.
- **T230 — agent timeline.** The claim that the wire surface was frozen
  earlier and untouched here holds: `cd33f78` carries the protocol change and
  `d2afd55` touches no file under `packages/protocol`. Every test named in the
  report exists, the counts are exact, and both the un-run `pnpm check` and the
  un-run live watch are declared rather than glossed.
- **T231 — agent rail.** Every file, every per-file case count (12 reducer, 5
  status, 6 chord) and the 410-test total match the tree, and the two things it
  could not observe — the chord from a focused terminal, and the rail's absence
  with no workspace open — are both written down as such.
- **T051 — launcher resume rail.** Files match `c6ec1a2` and `1a00106`,
  protocol 2.4.0 → 2.5.0 is what the commit does, the three new Rust and two
  new vitest cases reconcile, and the manual round trip is declared not run.
- **T210 — island and attention queue.** The report's shortfalls are its own
  candid limitations rather than discrepancies; the only tree-level notes are
  two unlisted test files (finding 7) and the missing raise form, which the
  report itself names as its largest gap.
- **T220 — agent session discovery.** Every test its spec required exists by
  name, including the bounded-tail and roster-diff cases, and the report
  declares `pnpm check` as not green on its tree — an admission worth
  crediting, since CI on `main` is green now.
- **T325 — ownership registry.** Apart from the unlisted planning document
  (finding 7) this is the most thoroughly evidenced task in the pile: every
  §37 test topic has a test named after it, including
  `two_concurrent_requests_for_one_file_grant_exactly_one` and
  `the_race_has_one_winner_every_time`, and the protocol 2.10.0 / schema 3 /
  migration 0003 / T223-renumbered-to-0004 claims are all exactly what `main`
  holds.
