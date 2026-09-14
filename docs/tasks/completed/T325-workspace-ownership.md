# Task T325 — Workspace ownership registry

- **Phase:** 4
- **Status:** done
- **Depends on:** T010, T020, T060, T200, T210, ADR-019

## Objective

Give Gritty a first-class answer to two questions it currently cannot answer at
all: *who owns this file*, and *why can't another workstream modify it*.

A registry of logical write leases over files, directories and globs, held by
workstreams, persisted, atomic to acquire, and refusing a second claim with an
explicit conflict that reaches the attention queue. Coordination plan §§12–23
and §37; ADR-019 is the decision.

This is **observed** ownership. It refuses a claim; it does not stop a write,
and it must not be presented as if it did (§20).

## Dependencies

- **T200 (Gate C)** — a lease is held by a `WorkstreamId`, cascades with it,
  and reads `stale` from its holder's state.
- **T060** — the session database and its forward-only migration chain.
- **T020 / ADR-010** — `WorkspacePath` is the definition of path identity.
- **T010** — command, event and error envelopes; the operation model.

## Required reading

- `docs/decisions/ADR-019-workspace-ownership.md` (this task's decision)
- `docs/decisions/ADR-005-agent-isolation.md`, `ADR-010-workspace-paths.md`,
  `ADR-004-persistence.md`, `ADR-018-agent-integration-bridge.md` (§§5–7, why
  enforcement is not in scope)
- `docs/architecture/ownership.md`, `docs/architecture/workstreams.md`
- `docs/plans/Gritty Agent Runtime Coordination.md` §§12–23, §37, §40
- `crates/gritty-core/src/workstream/mod.rs` — the transaction pattern this
  copies (`domain()` / `from_write()`)
- `scripts/agent-lease.py` — the same idea, by hand, outside the product

## Allowed scope

- `crates/gritty-core/src/ownership/**` (new), plus the module, event, id and
  export lines that reach it
- `crates/persistence/src/migrations/0003_ownership.sql` (new),
  `migrations.rs`, the `ownership` module in `store.rs`, `tests/database.rs`
- `apps/desktop/src-tauri/src/{commands.rs,core.rs,lib.rs}`
- `packages/protocol/src/{client.ts,commands.ts,types.ts}` and the new
  `ownership-client.test.ts`; `src/generated` is written by the exporter
- `apps/desktop/src/features/workstreams/**`, `apps/desktop/src/app.css`
- `docs/decisions/ADR-019-*`, `docs/architecture/ownership.md`,
  `docs/tasks/README.md`, `AGENTS.md`, `docs/plans/roadmap.md`

## Explicit non-goals

- **Enforcement.** Nothing blocks a write. That is T330 and needs an ADR-018
  amendment first: the hook is forbidden from deciding anything.
- **Violation detection.** Cross-checking the files an agent touched against
  the registry is T326 and needs T223's session↔workstream link.
- **Ownership-aware scheduling** (T405) — comparing intended scope before a run
  starts.
- **Worktrees** (T400) and merge conflicts, which are review conflicts.
- **Symbol-level ownership** (§38), read leases, and anything across machines.
- **A path matcher in TypeScript.** Overlap is decided in the core.
- **Automatic re-acquisition** after a release or an override.

## Required behavior

1. A workstream may claim a `FILE`, a `DIRECTORY` subtree or a `GLOB`.
2. `ownership.acquire` answers `GRANTED` or `CONFLICTED` — never both, never a
   silent success, never an exception for an ordinary refusal.
3. The overlap check and the write share **one transaction**, so two concurrent
   requests for overlapping ground cannot both be granted.
4. Overlapping your own ground is allowed; asking twice for the identical
   target returns the same lease rather than a second row.
5. A refusal creates one persisted conflict naming requester, holder, the lease
   in the way and the target. The same refusal asked twice reuses it.
6. `ownership.release` frees the ground and resolves the conflicts waiting on
   that lease. Nothing is granted automatically.
7. `ownership.resolve` takes `CANCEL` or `OVERRIDE`. `OVERRIDE` releases the
   holder's lease and records it; it does not transfer the ground.
8. `ownership.check` names every lease overlapping a target, whoever holds it.
9. Expiry and staleness are derived on read; neither is stored, and nothing
   auto-releases.
10. Every mutation emits `OwnershipChanged` with the whole map, read inside the
    transaction that produced it.
11. Conflicts appear in the attention queue as a third kind, with a fourth
    filter; the island shows the map and a claim form and says, in words, that
    this is observation and not protection.

## Failure behavior

| Situation | Behaviour |
|---|---|
| Two concurrent acquisitions overlap | exactly one `GRANTED`, one `CONFLICTED`; one row in the table |
| Acquisition refused | transaction rolls back; only the conflict is written |
| Unknown workstream or lease | `NotFound` |
| Another workspace's lease or workstream | `NotFound` — missing, not forbidden |
| Lease already released | `Conflict`, naming its state |
| Conflict already closed | `Conflict` |
| Target with `..`, absolute, backslash, NUL, empty segment | `InvalidInput`; nothing is written |
| Pattern over 240 characters or 32 segments | `InvalidInput` |
| More than 200 active leases in a workspace | `Conflict`, asking for a release first |
| Session database unavailable | every command fails `Io`; no lease is granted that would vanish at exit |
| Stored row Gritty cannot parse | `Internal`, naming the field; the row is not guessed at |
| Holding workstream removed | leases and conflicts cascade away |
| Holding workstream finished | lease reads `stale`, still holds, still releasable |
| `OwnershipChanged` for another workspace | dropped by the reducer |

## Security

Every target is untrusted input (rule 13).

- Paths are `WorkspacePath`s, validated on deserialization: no `..`, no
  absolute or drive-letter form, no backslash, no NUL, no empty segment.
- Glob patterns are not paths, so they get the same rules written out, plus a
  length and a segment cap — the segment cap is what bounds the matcher's
  backtracking over `**`.
- A lease grants nothing on the filesystem. No code path turns one into a file
  handle; it is a record of intent.
- Targets render as text in the UI: never a link, never a path anything opens,
  never a command anything runs.
- Notes are trimmed and bounded before they cross the protocol.

## Acceptance criteria

The coordination plan's own criterion: *two simulated agent runs cannot obtain
conflicting write ownership without an explicit conflict.*

- Two workstreams claiming overlapping ground: the second is refused, the
  refusal names the holder, and it is in the attention queue.
- Releasing frees the ground; the waiting conflict reads `RESOLVED`; claiming
  again succeeds.
- Leases survive a restart and still hold.
- A lease held by finished work reads stale, still refuses others, and can be
  released.
- `pnpm check` green.

## Tests required

§37 lists them, and each has one: independent files, identical files,
directory/file overlap, glob/file overlap, glob/glob, several leases, release,
stale leases, crashed/finished holders, canonical path equivalence, traversal
attempts. **Concurrency is mandatory** — two threads racing `acquire`, and the
same race twenty times over, because a lock that usually works is not a lock.

Plus: schema 2 → 3 migrates an existing database forward with its rows; the
repository round-trips; conflicts close with their lease and cascade with their
workstream; the protocol client maps every command name and carries a refusal
as a value; the reducer replaces rather than merges and drops another
workspace's map.

## Commands to run

```
cargo test -p gritty-core         # also regenerates packages/protocol/src/generated
cargo test -p gritty-persistence
pnpm check
pnpm dev                          # the manual pass below
```

## Completion report

### Summary

`gritty_core::ownership` is a new domain module beside `workstream`, with the
same four-file shape. Three target kinds compile to **one** segment matcher, so
overlap is one routine rather than nine pairwise cases, and where it cannot
decide cheaply it reports a collision — a false conflict costs a click, a
missed one costs a file.

Acquisition is atomic because the overlap check and the insert share one
`Database::write` transaction, reusing T200's `domain()` / `from_write()`
smuggling. Expiry and staleness are derived on read and never stored, which
makes §21's startup reconciliation continuous instead of a pass that can be
skipped, and nothing auto-releases: a stale lease is reported with a release
beside it.

Five commands under a new `ownership.*` namespace, one event carrying the whole
map, migration `0003_ownership.sql`, and the UI in the workstream surface: an
Ownership section in the island, `owns N` on a roster row, and a fourth filter
in the attention queue.

### Files changed

**Core** — `crates/gritty-core/src/ownership/{mod,wire,target,tests}.rs` (new);
`protocol/ids.rs` (`LeaseId`, `ConflictId`); `protocol/export.rs`;
`protocol/mod.rs` (2.7.0 → **2.10.0**); `event/mod.rs` (`OwnershipChanged`);
`session/tap.rs`; `lib.rs`.

**Persistence** — `migrations/0003_ownership.sql` (new); `migrations.rs`
(`SCHEMA_VERSION` 2 → **3**); `store.rs` (`ownership` module); `lib.rs`
(re-exports `rusqlite::Connection` so a caller can name the handle without
depending on `rusqlite`); `tests/database.rs`.

**Desktop shell** — `src-tauri/src/{commands.rs,core.rs,lib.rs}`: five command
adapters, an `OwnershipManager` next to the workstream manager sharing its
database, and registration.

**Protocol** — `client.ts` (an `ownership` namespace), `commands.ts`,
`types.ts`, `ownership-client.test.ts` (new), 17 generated files.

**UI** — `features/workstreams/`: `ownership.ts`, `ownership-reducer.ts`,
`ownership-panel.tsx` and their tests (new); `use-workstreams.ts`,
`attention.ts`, `attention-queue.tsx`, `island.tsx`, and their tests;
`app.css`.

**Docs** — `ADR-019`, `docs/architecture/ownership.md`, this file,
`docs/tasks/README.md`, `AGENTS.md`, `docs/plans/roadmap.md`,
`docs/architecture/workstreams.md`, `docs/decisions/README.md`.

### Tests actually executed, and results

| Command | Result |
|---|---|
| `cargo test --workspace` | green — `gritty-core` 181 → **232** (+51 ownership), `gritty-persistence` `tests/database.rs` 15 → **18** |
| `pnpm test` (vitest) | green — **410** across 58 files, +24 new |
| `pnpm check` | green (lint, typecheck, vitest, `cargo fmt --check`, `clippy -D warnings`, `cargo test`, generated-types freshness) |

The two that matter most: `two_concurrent_requests_for_one_file_grant_exactly_one`
and `the_race_has_one_winner_every_time` (twenty rounds, four worker threads,
a directory against a file inside it — an overlap no unique index could have
caught).

**Not executed: the manual pass.** No agent session in this repository has a
screen, which is why every T2xx task is at `review`. See below.

### Architectural decisions

1. **The holder is a workstream, not a session.** Hard rule 6, and
   `agent-sessions.md`'s objection to a durable row carrying an ephemeral id.
   `agentRunId` is on the wire and always absent — T200's `Decision.raisedBy`
   move, paid for once.
2. **One matcher, biased towards collision.** Stated in ADR-019 §4 precisely
   because a later "optimisation" would quietly reverse it.
3. **Expiry derived, not stored.** A verdict a clock decides must not be able
   to disagree with the clock.
4. **A refusal is a value, not an exception.** `CONFLICTED` is a persisted row
   the queue draws; making it an error would put the one thing the user needs
   to see behind a dismiss button.
5. **An override does not transfer the ground.** Rule 7 forbids a *silent*
   override; an automatic re-acquisition is a silent override one step later.
6. **Protocol 2.10.0, skipping 2.8.0 and 2.9.0**, which the unmerged
   `t230-agent-timeline-contract` branch has already spent. A gap costs nothing
   (`assertCompatible` compares majors); two branches claiming one version for
   different contracts costs a merge.
7. **`OwnershipChanged` carries the whole map**, unlike `AgentTimelineAppended`.
   The map is bounded and a transcript is not, so a snapshot converges where a
   delta would have to be reconciled.
8. **The T330 renumbering** — see ADR-019's last section. Completion reports
   and the applied migration `0002_workstreams.sql` were deliberately left
   alone; the second because `migrations.rs` says *never edit an applied one*,
   and that rule has no comments-are-fine exception.

### Known limitations

- **Nothing is enforced.** An agent that writes to owned ground is not stopped
  and, until T326, not even noticed. The UI says so; this report says so twice.
- **No manual pass.** Every claim about the UI below the type level is reasoned
  from the code, not observed. The island section, the claim form, the fourth
  filter and the conflict card have never been on a screen.
- **`WorkspacePath` does no case folding**, so on a case-insensitive volume two
  spellings of one file are two leases. A test pins it.
- **Glob-against-glob overlap is conservative.** `src/*.ts` against `src/*.rs`
  reports a conflict that does not exist.
- **Nothing claims ownership automatically.** Every lease is a user action;
  a workstream's intended scope is not inferred from anything (T405).
- **Released leases leave the map.** They are stored but not returned, so there
  is no history view of who used to own what.
- **200 leases per workspace** is a guess at "more than anyone needs, less than
  makes the map unreadable", not a measurement.
- **`ownership.check` has no caller in the UI.** It is the command the rail's
  touched-file list and the editor will use once ownership can be shown beside
  a file; it is tested but not yet drawn.

### Follow-up tasks

- **T326 — observed violations (§20).** Cross-check `FILE_TOUCHED` timeline
  entries against active leases and raise an `OBSERVED` conflict. T230's
  timeline is now on `main`, so the only thing still missing is T223's
  session↔workstream link — without it a touch cannot be attributed to a
  workstream, and a violation nobody can attribute is not one.
- **T330 — write interception (§19).** `scripts/agent-lease.py` proves the
  mechanism: Claude Code's `PreToolUse` hook takes a
  `permissionDecision: "deny"`. `gritty-agent-hook` deliberately cannot answer
  one. **An ADR-018 amendment first, code second**, and the ENFORCED/OBSERVED
  distinction has to survive it.
- **T405 — ownership-aware scheduling.** Compare a workstream's intended scope
  against the registry before a run starts, and recommend waiting or a
  worktree.
- Annotate the rail's touched-file list and the editor with the owning
  workstream, through `ownership.check`.
- **T223's migration is now `0004`**, not `0003`; its task file was updated in
  the merge that brought T221–T232 in. It is unstarted, so nothing else moves.
- A history view of released leases, if anyone asks who used to own something.

### Remaining risks

- **The registry is believable, and it protects nothing.** The gap between what
  a user will assume from a list of claims and what Gritty actually does is the
  main risk in this task. It is mitigated with words in three places (the UI,
  the architecture doc, this report) and with the deliberate absence of any
  language suggesting a file is "locked" or "protected".
- **A conservative matcher trains people to ignore conflicts.** If false
  conflicts turn out to be common in practice, the fix is a better matcher, not
  a looser bias.
- **Two versions of the protocol are in flight.** This branch takes 2.10.0 off
  `main`'s 2.7.0 while `t230-agent-timeline-contract` holds 2.9.0. Whichever
  merges second must check `PROTOCOL_VERSION` and `SCHEMA_VERSION` rather than
  trusting the merge — the same trap HANDOFF.md records for stacked PRs.
