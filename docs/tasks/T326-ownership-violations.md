# Task T326 — Ownership notices a violation after it happens, not before

- **Phase:** 4
- **Status:** todo
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

_(filled in by the implementing session)_ — with obligations beyond the
template: state which Failure-behavior rows a real test exercised versus
reasoned about only, naming the ring-overflow row either way (rule 12);
flag where §2 guessed wrong against the shape T223 actually landed with;
confirm persistence changed by exactly §8's two widened clauses and nothing
else; and argue the §6/§7 `ConflictState`-reuse trade in Architectural
decisions — a reviewer disagreeing with that call is the most likely reason
this task returns from review.
