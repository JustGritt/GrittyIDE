# Task T326 — Ownership notices a violation after it happens, not before

- **Phase:** 4
- **Status:** review
- **Depends on:** T230 (`FileTouched` timeline entries, on `main`), T325
  (the ownership registry, ADR-019), T223 (session→workstream link, landing
  in parallel — depend on its table's shape, not its code)

## Objective

T325 gave Gritty a registry that knows a workstream owns a path and refuses a
second *claim* on it, but nothing yet connects a discovered session to a
workstream (`docs/architecture/ownership.md:30`), so it cannot notice that an
agent wrote there anyway. T326 is that connection: cross-check the files an
agent touched (T230) against the registry (T325), attributed through T223's
link, and record what it finds as an observed violation — T325's own report
named the risk this shrinks: "the registry is believable, and it protects
nothing."

**Nothing here stops a write.** This adds detection, not enforcement: a
write into another workstream's ground is still not blocked or warned
beforehand, and the UI says so exactly as before (`ownership.md` lines
17–26, kept verbatim). Stopping the write is T330, gated on an ADR-018
amendment this task does not make.

## Dependencies

- **T230** — `AgentTimelineKind::FileTouched { path, display }`
  (`agent_session/wire.rs:283-292`) and the per-tick delta carrying it
  (`AgentTimelineAppendedPayload`, `mod.rs:566-611`, built by
  `OpenTimeline::absorb`/`take_batch` in `agent_session/timeline.rs`).
- **T325** — `crates/gritty-core/src/ownership/**`: `overlaps` (`target.rs:98`),
  `still_holds` (`mod.rs:523`), `ConflictState`, and `store.rs`'s `ownership`
  module (`insert_conflict:848`, `list_conflicts:902`, `close_conflict:917`,
  `close_conflicts_for_lease:930`).
- **T223** — table `workstream_agent_link(workstream_id, provider,
  session_id, linked_at)`, indexed on `session_id` (§8). Unstarted, building
  in parallel: depend on that shape, not a store function name (rule 2).

## Required reading

- `docs/architecture/ownership.md:17-26` (not to cross) and `:30` (this
  task's mandate, verbatim); ADR-019 §§7–8
- `docs/tasks/T325-workspace-ownership.md` — Follow-up tasks (line 277);
  `docs/tasks/T223-agent-attention.md` §§8–15 — link shape, cardinality
  (§9), cascade (§13)
- `crates/gritty-core/src/ownership/{mod.rs,wire.rs,target.rs}` — `check()`
  (`mod.rs:292-326`) is the closest analogue; `0003_ownership.sql:40-53` —
  `state` is unconstrained `TEXT`, no `CHECK`
- `docs/plans/execution-board.md:121-139` — Lane A, the "read side only"
  note §7 answers to; `ownership.ts:71-74`'s `waitingConflicts`

## Allowed scope

- `crates/gritty-core/src/ownership/{mod.rs,wire.rs}` — a fourth
  `ConflictState`, `Observed`, and the check-and-record pass. `target.rs` is
  read, not edited: reuse `overlaps`, never a second matcher.
- `crates/gritty-core/src/agent_session/**` and `apps/desktop/src-tauri/src/core.rs`
  — expose what the detector needs from a tick (provider, the entries just
  appended) and wire it to the ownership pass; no new command or capability
  line, `agents.timeline` unchanged.
- `crates/persistence/src/store.rs`, `ownership` module only — widen the two
  close functions' `WHERE` to also match `OBSERVED` (§8). No new file,
  column or migration.
- `apps/desktop/src/features/workstreams/{ownership.ts,ownership-reducer.ts,
  ownership-panel.tsx,attention.ts}` and tests — render/dismiss `OBSERVED`,
  no fifth filter (§12); `docs/architecture/ownership.md` — line 30 stops
  being a future conditional.

Nothing else moves: no file under `crates/persistence/src/migrations/**`.

## Explicit non-goals

- **No enforcement, no ownership-aware scheduling.** T330 needs an ADR-018
  amendment first (§§5–7 forbid `gritty-agent-hook` an opinion); T405
  compares intended scope before a run starts and needs no violation
  history to do it.
- **No new table, column, or migration** (§6 decides that; §7 argues it),
  **no fifth queue filter, resolution verb, or event** (§§10–12 reuse what
  T325 built), **no automatic attribution** — only T223's explicit link is
  consulted, no matching by working directory or title (rule 4).
- **No re-scan of the ring on a timer, no acting on the touched path beyond
  matching it.** Detection rides the per-tick delta already emitted (§1,
  rule 8); the path is never opened, linked, or run (rule 13).

## Required behavior

### Detection

1. Detection runs on the per-tick delta already produced for the agent rail
   — `AgentTimelineAppendedPayload`/`CoreEvent::AgentTimelineAppended`
   (`mod.rs:566-611`) — not a fresh scan of the ring, and not a subscriber
   to `agents.timeline`, a read for a human, not a feed for a checker. Only
   `FileTouched { path: Some(path), .. }` matters; every other kind, and a
   `FileTouched` with no `path` (outside the open workspace by construction
   — `wire.rs:285`), is ignored: nothing to check it against.
2. Attribution keys on `(provider, session_id)`, not `session_id` alone —
   the batch carries only `session_id`, the roster knows
   `AgentSession.provider`, and T223's primary key is the triple precisely
   so two providers reusing a session id cannot collide.
3. A session with no link resolves to zero workstreams: nothing is written
   and this is not a failure — T325's own follow-up note
   (`T325-workspace-ownership.md:281`) already says a violation nobody can
   attribute is not one — while one linked to several (T223's failure table
   allows it) is checked once per link.
4. For each candidate W, every lease the registry still holds — `list_held`
   filtered by the same `still_holds` test `ownership.check` already uses
   (`mod.rs:523`) — is compared to `path` with `OwnershipTarget::overlaps`
   (`target.rs:98`), the one matcher T325 built (rule 2).
5. `lease.workstream_id == W` is not a violation — `acquire` already treats
   your own overlapping ground as allowed; `!= W`, with an overlap, is.

### Representation — no new table

6. Persist a violation as an ordinary `OwnershipConflict`, through the
   existing `insert_conflict` (`store.rs:848`), with a fourth
   `ConflictState`, `Observed`: `requestedBy` = W, `ownedBy` = the lease's
   holder, `holdingLeaseId` = the lease, `target` = `File { path }`. The
   `state` column is unconstrained `TEXT` (no `CHECK`), so a fourth value
   costs one match arm, not a migration — this is the decision asked of
   this spec: not a new column or table, which would make the queue,
   island and panel learn a second shape for "someone is in someone else's
   way", reusing instead the shape rule 2 already owns. The variant still
   crosses the wire, so protocol version bumps MINOR (additive).
7. This is a write, and Lane A's line for this task says "read side only"
   (`execution-board.md:133`); §6 does not honour that literally, but it
   honours the boundary beneath it — no file, column or migration touches
   `crates/persistence/**` beyond §8's two lines, and every write goes
   through a function T325 already tested. Read "read side only" as "no
   new persistence surface" and correct the line — this spec cannot edit
   `execution-board.md` itself. If review rejects §6 in favor of a new
   column or table, it does need a migration: the next free number, claimed
   in `docs/merge-queue.md` immediately before that PR opens.
8. `close_conflict`/`close_conflicts_for_lease` (`store.rs:917,930`) close
   only `WAITING` rows today, so releasing a lease would leave any
   `OBSERVED` row on it forever. Widen both `WHERE` clauses to
   `state IN ('WAITING', 'OBSERVED')`.
9. Before inserting, skip a duplicate `(requestedBy, holdingLeaseId,
   targetKind, targetPattern, state = OBSERVED)` already unresolved among
   `list_conflicts`'s results — `waiting_conflict` (`store.rs:881`)
   hardcodes `state = WAITING` and cannot be reused as-is, so this dedup
   filters in Rust rather than add a second persistence query.
10. `OwnershipChanged` carries the new row; no second event, and a tick
    with no new violation emits nothing (rule 8) — free once §6 shares
    T325's per-transaction emit.

### Resolution and the UI

11. `ownership.resolve` keeps its two verbs. `CANCEL` on `Observed` means
    "acknowledged, stop asking" — no request to withdraw, but the effect is
    what the user wants. `OVERRIDE` is `InvalidInput`: it releases a lease
    to unblock a pending acquisition (ADR-019 §8), and there is none here
    to unblock — the toucher already wrote.
12. `waitingConflicts` (`ownership.ts:71-74`) widens to also return
    `Observed` rows; `attentionItems` needs no other change. Word the two
    states differently in the panel — a refusal reads "X asked, Y already
    holds it", a violation "X wrote here while Y held it" — never "locked"
    or "blocked", and never a fifth `AttentionFilter`.
13. `ownership.md:30`'s "T326 would notice a violation after it happened"
    becomes present tense, describing §§1–10; lines 17–26 are unchanged.

## Security

- `path` was already validated as a `WorkspacePath` by T230's fold; this
  adds no further filesystem interpretation, only the `overlaps` comparison
  `ownership.check` already runs (rule 13). `display` never enters matching
  or the conflict row, and no new command or capability is added (rule 1).
- A session's own report is the only evidence (rule 4); an `Observed` row
  is only as trustworthy as its source, since Gritty cannot verify a
  provider touched no more than it reported.

## Failure behavior

| Situation | Behavior |
|---|---|
| No link for the touching session, or its working directory matches no open workspace | No conflict; not an error, nothing to check against (§3) |
| The entry's `path` is absent | Ignored (§1) |
| Touch overlaps a lease the same workstream holds | Not a violation, no row (§5) |
| Touch overlaps several leases, or session links to several workstreams | One row per (workstream, lease) pair, deduped independently (§3, §9) |
| Overlapping lease is `EXPIRED`, or released before the check ran | Excluded by `still_holds`; ground nobody holds refuses nobody (§4) |
| The same violation recurs on every later tick | One row; later ticks match §9's dedup |
| A tick absorbs more entries than the ring holds | `absorb` truncates `undelivered` before the batch is emitted; a pruned touch is never seen — a real gap, and the report must say whether a test exercised it (rule 12) |
| No database this tick | Finds and writes nothing; costs the tick, not the loop |
| Violated lease released or overridden afterward | Resolved by §8's widened close, like an ordinary refusal |
| `ownership.resolve(OVERRIDE)` on `Observed` | `InvalidInput` (§11) |
| `MAX_CONFLICTS` reached | Bound shared with ordinary refusals, unmodified here — inherited risk, not new |

## Acceptance criteria

- A workstream holds a lease; a session linked to another workstream
  reports a `FileTouched` inside it; within that tick, exactly one
  `Observed` conflict exists, correctly attributed, `OwnershipChanged`
  fires once, and ten further identical ticks still leave that one row. A
  touch inside the toucher's own ground, or unlinked, or with `path`
  absent, never produces a row.
- Releasing or overriding the violated lease resolves its `Observed` row;
  `OVERRIDE` on one is refused with `InvalidInput`.
- Generated types show four `ConflictState` members, no new command; `pnpm
  check` green.

## Tests required

- **Unit (Rust):** each Failure-behavior row that is a pure function of
  leases, links and entries — no link, own ground, several leases, several
  links, an expired lease, idempotence, `OVERRIDE` rejected on `Observed`,
  the widened close functions clearing an `Observed` row while leaving an
  unrelated `WAITING` row untouched, and one tick over
  `MAX_TIMELINE_ENTRIES` asserting what the detector actually saw.
- **Unit (TS):** `waitingConflicts`/`attentionItems` surface `Observed`, the
  panel's distinct wording, and the `OVERRIDE` refusal rendering as a value.
- **Integration:** a fake transcript with one `FileTouched` inside a seeded
  lease held by another workstream, through one real tick, asserting one
  persisted `Observed` conflict and one `OwnershipChanged`.
- **Manual:** none claimed — as T325 recorded, no session here has a screen.

## Commands to run

```
cargo test -p gritty-core
cargo test -p gritty-persistence
cargo clippy --workspace --all-targets -- -D warnings
pnpm check
```

## Completion report

### Summary

The registry now notices a write it could not refuse. Every tick of the agent
rail already produces a delta of what each session did; the `FileTouched`
entries in it are cross-checked against the leases the registry still holds,
attributed through T223's `(provider, session_id)` link, and a write that
landed inside another workstream's lease is persisted as an ordinary
`OwnershipConflict` in a fourth `ConflictState`, `OBSERVED`.

**Nothing here stops a write, and the shipped words still say so.** The detector
runs after the fact, `ownership.md` lines 17–26 are unchanged, and the panel's
sentence about observation rather than protection is untouched. What changed in
that document is line 30, which was a future conditional and is now a
description.

No new command, no new event, no new capability line, no new table, no new
column, and **no migration**: `SCHEMA_VERSION` stays at 4. `agents.timeline` is
unchanged, and the registry does not subscribe to it — §1's point is that a
read for a human is not a feed for a checker, so detection rides the delta the
rail already emits.

### Files changed

| File | What |
|---|---|
| `crates/gritty-core/src/ownership/wire.rs` | `ConflictState::Observed`, and `is_open()` for the two states that are still the user's problem |
| `crates/gritty-core/src/ownership/mod.rs` | `OwnershipManager::observe_touches` — the whole detection pass, in one transaction; `OVERRIDE` on an `OBSERVED` row refused; module doc |
| `crates/gritty-core/src/ownership/tests.rs` | 16 tests for the pass (below) |
| `crates/gritty-core/src/agent_session/mod.rs` | `AgentDiscoveryConfig::ownership` + `with_ownership`; `touched_paths`; `observe_ownership`, called from `follow_once` *after* the rail's own batches are emitted |
| `crates/gritty-core/src/agent_session/tests.rs` | the `ViolationRig` (workspace + database + transcript), 4 whole-tick tests, and the `touched_paths` unit test |
| `crates/persistence/src/store.rs` | **exactly** the two widened `WHERE` clauses of §8, plus the `OBSERVED` constant they interpolate. Nothing else in `crates/persistence/` is touched: no file, no column, no migration |
| `apps/desktop/src-tauri/src/core.rs` | one builder call wiring the registry to the rail |
| `apps/desktop/src/features/workstreams/ownership.ts` | `waitingConflicts` widened; `isViolation`, `conflictHeadline`, `conflictSummary`, `conflictResolutions`, `conflictCancelLabel`, `conflictHint` |
| `apps/desktop/src/features/workstreams/attention-queue.tsx` | the row, the card and the fact list read those functions; the `OVERRIDE` button is absent on a violation rather than present and refused |
| `apps/desktop/src/features/workstreams/{ownership,attention}.test.ts` | the TS tests below |
| `crates/gritty-core/src/protocol/mod.rs` | `PROTOCOL_VERSION` 2.12.0 → **2.15.0**, claimed in `docs/merge-queue.md` |
| `packages/protocol/src/generated/{ConflictState,version}.ts` | written by `cargo test -p gritty-core`, never by hand |
| `docs/architecture/ownership.md` | line 30 stops being a conditional; the conflict states, the `resolve` row and "deliberately not here" follow it |

`apps/desktop/src/features/workstreams/attention-queue.tsx` is **one file
outside the spec's Allowed-scope list** (§ "Allowed scope" names
`ownership-panel.tsx`). The conflict row and card are drawn there, not in the
panel, so "render `OBSERVED`" is not satisfiable without it. Every word it
gained comes from a pure function in `ownership.ts`, which is in scope and is
what the tests assert; the `.tsx` edit is the call sites. `ownership-panel.tsx`
and `ownership-reducer.ts` needed no change at all — the reducer replaces a
whole map, and the panel draws leases.

### Tests actually executed

`cargo test -p gritty-core --lib` — **341 passed, 0 failed**.
`pnpm check:rust` (`cargo fmt --check`, `clippy --workspace --all-targets -D
warnings`, `cargo test --workspace`) — **exit 0**, every crate green,
`gritty-terminal` included (the known `pty.rs` flake did not fire).
`pnpm typecheck` — exit 0. `pnpm lint` — clean after `pnpm format`.
`pnpm vitest run apps/desktop/src/features/workstreams` — **82 passed**.

`pnpm check` as a whole is **red, on two failures that are not this work**:

- `packages/editor/src/monaco/model-eol.dom.test.ts` — `beforeAll` hook times
  out loading Monaco at 10 s. Verified pre-existing: stashed to the bare base
  commit `e8ada81` and it fails identically. Nothing here touches
  `packages/editor`, and the board already carries it as **`MONACO-JSDOM-FLAKE`**
  ("model-eol.dom.test.ts times out its 10s beforeAll under load").
- `packages/markdown/src/render.test.ts` "within budget" — a 2 000 ms wall-clock
  assertion, measured 2 530–2 900 ms with five agents building on this machine.
  Passes in isolation (`pnpm vitest run --project markdown`, 54 passed).

The desktop crate also needs T225's sidecar staged at
`apps/desktop/src-tauri/binaries/gritty-agent-hook-aarch64-apple-darwin` before
`cargo clippy --workspace` will build it at all. That path is git-ignored and
T225 is still in review, so it was built and copied by hand for this run.

#### Which failure-behavior rows a real test exercised

| Row | Test |
|---|---|
| No link for the touching session | `an_unlinked_session_touching_owned_ground_is_nobodys_violation`, and `attribution_is_by_session_and_not_by_anything_else` for the key |
| No open workspace | `no_open_workspace_finds_no_violation` |
| The entry's `path` is absent | `only_a_resolved_touched_path_is_checked` — over `touched_paths` directly, with a no-path entry, an unrelated kind, and a repeat |
| Touch overlaps a lease the same workstream holds | `a_write_on_your_own_ground_is_not_a_violation` |
| Touch overlaps several leases | `a_write_under_two_leases_of_one_holder_is_one_row_for_each` |
| Session links to several workstreams | `a_session_on_two_workstreams_is_checked_once_per_link` |
| Overlapping lease is `EXPIRED` | `an_expired_lease_is_not_violated` |
| Released before the check ran | `a_lease_released_before_the_check_is_not_violated` |
| The same violation recurs every tick | `a_write_on_another_workstreams_ground_is_recorded_once` — ten further ticks, one row, no event |
| **A tick absorbs more than the ring holds** | **`a_tick_over_the_ring_checks_only_what_survived_it` — a real test, not reasoning.** 540 touches into a 500-entry ring, through a real `absorb`: the batch reports `dropped > 0`, the detector receives exactly `MAX_TIMELINE_ENTRIES` paths, and the oldest (`src/auth/f0.ts`) is in neither the batch nor the rows. The gap §1 predicted is real and is now pinned |
| No database this tick | `no_database_finds_no_violation` |
| Violated lease released, or overridden | `releasing_a_violated_lease_resolves_its_observed_row` (which also asserts an unrelated `WAITING` row on another lease is untouched), `overriding_the_violated_lease_resolves_its_observed_row` |
| `resolve(OVERRIDE)` on `Observed` | `override_on_an_observed_violation_is_invalid_input` — and asserts the refusal changed nothing: the lease still holds, the row is still open |
| `MAX_CONFLICTS` reached | exercised, though not by a test written for it: the ring test's 500 surviving touches write 500 rows and the map returns 200. See Known limitations |

Also tested and not in the table: `cancel_dismisses_an_observed_violation` (and
that a later write raises it again — acknowledging is not silencing),
`a_tick_with_no_touched_files_does_nothing`,
`two_files_under_one_lease_are_two_rows`,
`removing_the_toucher_removes_its_observed_rows` (T325's cascade, inherited).

Whole-tick tests: `one_tick_records_a_violation_and_says_so_once`,
`a_touch_outside_every_lease_records_nothing`,
`without_a_registry_the_rail_is_unchanged`.

TS: `waitingConflicts` returns both open states; `attentionItems` files a
violation as the same `conflict` kind under the same filter (and
`attentionCount` counts it); the two headlines and summaries differ in tense;
neither says *locked* or *blocked*; `conflictResolutions` returns `['CANCEL']`
for a violation and both verbs for a refusal.

**Manual: none claimed.** As T325 recorded, no session here has a screen.

### Architectural decisions

**§6/§7 — a fourth `ConflictState` rather than a new table or column.** This is
the call most likely to bring the task back from review, so here is the case.

A violation and a refusal answer the same question — *who is in whose way, over
what ground, and what do you want to do about it* — with the same five facts:
who, whom, which lease, which target, when. They differ in one thing, tense:
one is a request that was stopped, the other a write that was not. A state is
exactly the right size for that difference. A second table would make the
attention queue, the island, the panel and the reducer each learn a second
shape for one concept, and every later change to "someone is in someone else's
way" would then have two places to be got right.

The `state` column is unconstrained `TEXT` with no `CHECK` (`0003_ownership.sql`
lines 40–53), so the fourth value costs one match arm and no migration. Every
write goes through `insert_conflict`, which T325 already tested. The cascade,
the bound and the ordering are inherited rather than re-derived.

The cost, stated plainly: `OwnershipConflict.requestedBy` now means "the
workstream that asked" *or* "the workstream that wrote", and `target` means
"what was asked for" *or* "the file that was written". Two readings of two
fields, disambiguated by `state`. That is real, and it is why the UI never
renders a conflict without branching on `state` first. The alternative price
was two shapes for one idea, and this seemed the cheaper of the two.

If review disagrees: a new column or table needs migration `0005`, which
`docs/merge-queue.md` still holds for T326 and which this branch has **not**
spent, so it is free for exactly that.

**Attribution is the link and nothing else.** No matching on working directory,
title or process. A session with no link resolves to zero workstreams and
writes nothing — T325's own note already ruled that a violation nobody can
attribute is not one, and guessing an attribution would put a workstream's name
on a row it may not have earned (rule 4).

**Direction of the dependency.** `agent_session` hands the ownership manager
what a tick produced; the registry does not read `agents.timeline`. That keeps
the *rail* the single place a transcript is followed, and it is why the check
cannot cost a second scan. The seam is `AgentDiscoveryConfig::with_ownership`,
so a build with no session database wires nothing and the rail is byte-for-byte
what T230 shipped (`without_a_registry_the_rail_is_unchanged`).

**Where in the tick.** The check runs *after* the rail's own batches are
emitted, so the agent view never waits on SQLite, and it is `spawn_blocking`
like every other ownership command. A failure is logged at `debug` and costs
that tick only.

**`Actor::System` on the event.** Not `Agent`: nothing constructs an
`AgentRunId` for a session Gritty merely observes (ADR-019 §2). Not `External`:
the write was external, but the row is Gritty's own record of having noticed.
The rail's other machine-observed events already use `System`.

**`OVERRIDE` is absent, not disabled.** The core answers `InvalidInput`; the UI
does not render a button whose only outcome is an error. `CANCEL` is relabelled
"Acknowledge", because withdrawing a request you never made is not what the
user is doing.

### §7's `execution-board.md` correction

Already made, before this branch was cut. `docs/plans/execution-board.md:185`
now reads "**'Read side only' does not survive contact with T326**", names the
`close_conflict`/`close_conflicts_for_lease` exception explicitly, and points at
§§6–8. Nothing was left for this task to correct there, so
`execution-board.md` is untouched by this branch.

### §2 against the shape T223 actually landed with

§2 guessed right, and the guess cost nothing. `workstream_agent_link
(workstream_id, provider, session_id, linked_at)` landed exactly as specified
in migration `0004_agent_links.sql`, with `agent_links::list_for_workspace`
joining through `workstream` for the workspace filter — so the detector reads
one function that already existed rather than adding a query. `provider` is
stored as `AgentProvider::as_str()`, which is the same spelling the roster
carries, so the `(provider, session_id)` comparison needs no translation.

Nothing in §2 had to be corrected.

### Persistence: exactly §8, and nothing else

`git diff crates/persistence/` is three hunks in one file, `src/store.rs`:
`close_conflict`'s `WHERE` and `close_conflicts_for_lease`'s `WHERE` each widen
from `state = '{WAITING}'` to `state IN ('{WAITING}', '{OBSERVED}')`, plus the
`pub const OBSERVED` those two clauses interpolate. No new file under
`src/migrations/`, no change to `migrations.rs`, `SCHEMA_VERSION` still 4.

### Known limitations

- **The conflict table can now grow without a user doing anything.** Before
  this, a row appeared only when somebody called `ownership.acquire`; now an
  agent writing 500 distinct files inside a lease writes 500 rows in one tick.
  Dedup stops repeats but not distinct files, and `list_conflicts` reads only
  the newest `MAX_CONFLICTS` (200) — so above 200 open rows the dedup window
  truncates and a violation already recorded can be recorded again. §"Failure
  behavior" ruled the bound "inherited risk, not new" and this honours that
  literally rather than inventing a second one; the ring test pins the
  behaviour. Follow-up below.
- **A violation is only as trustworthy as the agent's own report.** Gritty
  cannot verify a provider touched no more than it said. An absent row is not
  evidence of no write.
- **Case folding.** Inherited from `WorkspacePath` (ADR-019 §5): on a
  case-insensitive volume `src/Auth.ts` and `src/auth.ts` are one file and two
  targets, so a write to one does not violate a lease on the other.
- **Nothing is attributed without a link**, which in practice means a user has
  to have attached the session in the island first. Until they do, the registry
  protects nothing it can name.
- **No test drives the real desktop app.** The check is exercised through
  `follow_once` with a scratch transcript; no session here has a screen.

### Follow-up tasks

1. **Bound observed rows per workspace.** Either cap insertions against the
   open-row count, or age them out, or count opens in SQL instead of paging
   `list_conflicts`. Needs a decision about what to drop, which is why it is not
   in this task.
2. **Dedup without the 200-row read window.** A `state`-parameterised sibling of
   `waiting_conflict` would make §9 exact at any table size — a new persistence
   query, which §9 explicitly declined here.
3. **Group a violation burst in the queue.** Twelve files under one lease are
   twelve rows and one story; the queue has no notion of a group.
4. **`agents.timeline` could mark an entry as having violated something**, so
   the rail shows it where the user is already looking. Needs the rail to read
   ownership, which this task deliberately did not wire.
5. **T330 still needs the ADR-018 amendment** (ADR-018A is on the board).
   Detection makes the case for it concrete: every `OBSERVED` row is a write
   enforcement would have stopped.

### Remaining risks

- The §6 trade is a judgement, not a fact; a reviewer preferring a table has a
  real argument and migration `0005` is unspent for it.
- `pnpm check` is red on two failures verified as not this work, one of them by
  reproducing it on the bare base commit. A reviewer should confirm both
  independently rather than take this on trust.
- The detector's cost is one link read, one lease read and one conflict read per
  tick **that touched a file** — nothing when no file was written, which is the
  common case. Not measured under a real agent writing continuously.
