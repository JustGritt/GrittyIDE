# Execution board

`roadmap.md` says what Gritty builds and in what order. This file says **who can
build it at the same time without two sessions editing the same file**, and what
each of them may touch. It is the scheduling layer over the roadmap, and it is
meant to be edited: a lane updates its own row and nothing else.

Read `AGENTS.md` § "Concurrent agent sessions" first. The lease registry is the
enforcement; this is the plan the leases implement.

---

## Where things stand — 2026-09-06

| | |
|---|---|
| `main` | `e32ee97` — everything below has landed on it |
| Protocol | **2.10.0** (on `main`) |
| Schema | **3** (`0003_ownership.sql`, on `main`) |
| Open PRs | none. #20–#24 are merged and the queue is empty |
| At `review`, merged but unreviewed | T049, T051, T096, T098, T200, T210, T220, T221, T230, T231, T232, T325 — **twelve tasks**, all of whose code is on `main`; what is missing is a reviewer, not a merge |
| `in-progress` | T045 drag and drop (holderless — see below) · T223 attention |
| Specified, not started | T097, T222, T224 |
| Not specified | T305, T310, T320, T330, T335, T400, T405, T410 |

Everything below is downstream of three things that are not code.

### The three blockers, in order

**1. ADR-001 is still `Proposed`, and only a human can close it.**
`docs/testing/phase-0-manual-pass.md` is ~105 minutes of clicking that no agent
on this machine can do: Monaco input in WKWebView, PTY under `vim`/`less`/Ctrl-C,
a `kill -9` with dirty buffers, typing latency, the packaged bundle. It fills the
`–` rows in `platform-matrix.md`, the perceptual rows of `phase-0-spike.md` §C,
and the §D go/no-go answers. Until it runs, every Phase 1–2 task starts against a
runtime decision that is formally open.

This is the single highest-leverage block of time in the project, and it is the
only work here that parallelises with everything else: agents write specs while
it happens.

**2. Twelve tasks sit at `review` and none of them has been driven by a pointer.**
The completion reports say so honestly. Reviewing twelve tasks is the real
bottleneck on this board — not the lease file, not build capacity. Merge before
adding.

**3. The lease system is uncommitted.** `scripts/agent-lease.py`, the `AGENTS.md`
section, the `.gitignore` line and `.claude/settings.json` are working-tree
changes. Concurrency has no backstop until they land. This is wave 0.

### The gate that was already crossed once

The roadmap says Phase 1–2 tasks start only after ADR-001 is recorded. T221 and
T230–T232 were built while it was `Proposed`; PR #22 records that as the
exception it was. This board does not silently repeat it. **Wave 2 starts when
ADR-001 is written**, and if the decision is to start earlier, that is an
amendment to the roadmap and not a thing a lane decides on its own.

---

## Lanes

A lane is a durable track, owned by one session at a time, with a scope that no
other lane may touch. Tasks inside a lane are sequential; lanes are parallel.

The scopes below are the lease claims. They are drawn so that no two intersect,
which is why they are the unit of parallelism rather than the tasks themselves —
T222 and T224 are both "agent" tasks and both touch
`crates/gritty-core/src/agent_session/**`, so they are one lane, not two.

### Lane W — filesystem truth · T070 → T080

Nothing in Gritty observes the filesystem. Document external-change detection
(ADR-011) is therefore reactive only, the explorer goes stale, and the two rows
`phase-0-spike.md` defers — A6 (watching, rapid changes do not lock the UI) and
B5 (filesystem storm) — are still open. T080's search wants T070's ignore rules
and its invalidation, so they are one lane in that order.

```
crates/workspace/**
crates/gritty-core/src/workspace/**
apps/desktop/src/features/explorer/**
crates/search/**                        (T080, new)
apps/desktop/src/features/search/**     (T080, new)
```

Reserved protocol minors: **2.11** (T070), **2.12** (T080).

### Lane G — git · T100 → T400

T100 is Phase 2's last dogfood piece, and it is the only path to T400. Worktree
isolation is the thing ADR-018's context says Herdr explicitly refuses to build;
it is the differentiator, and it cannot start before Gritty can talk to Git at
all. Both crates are new, so this lane collides with nothing.

```
crates/git/**                           (new)
crates/gritty-core/src/git/**           (new)
apps/desktop/src/features/git/**        (new)
```

Reserved protocol minors: **2.13** (T100), **2.14** (T400).
T400 needs a migration (workstream ↔ worktree); see the numbering rule below.

### Lane T — terminal · T097

Fully specified, ADR-017 is drafted and needs deciding first, and the scope does
not overlap another lane at any point. It is also the missing input for two
things already drawn: 8b's session subtitles and T047's deferred automatic tab
titles.

```
crates/terminal/**
crates/gritty-core/src/terminal/**
apps/desktop/src/features/terminal/**
docs/decisions/ADR-017-terminal-command-blocks.md
```

Reserved protocol minor: **2.15**.

### Lane A — agent observability · T223 → T326 → T222 → T224

T325 shipped a registry that knows a workstream owns a path. It cannot yet
notice that an agent wrote there anyway, because nothing connects a discovered
session to a workstream — that link is T223's second half. T326 is what turns the
registry from a table into a thing that detects something, and it is the cheapest
remaining step that makes T325 pay. T222 and T224 are additive and can slip.

```
crates/agents/**
crates/agent-hook/**
crates/gritty-core/src/agent_session/**
crates/gritty-core/src/ownership/**          (T326 — see the note below)
apps/desktop/src/features/agents/**
apps/desktop/src/features/workstreams/**
crates/platform/**                           (T224, new)
```

Reserved protocol minors: **2.16** (T223), **2.17** (T326), **2.18** (T222), **2.19** (T224).
T223 needs a migration. Both numbers are stale as planning hints: T223 is
spending **2.11.0** and migration **0004**, because `docs/merge-queue.md` is
the record and a version gap costs nothing.

**"Read side only" does not survive contact with T326.** The task file
(`docs/tasks/T326-ownership-violations.md`) records that an observed violation
is persisted as an ordinary `OwnershipConflict` row with a fourth
`ConflictState`, and inserting a row is a write. What the annotation was
protecting is intact — no new file, column or migration in
`crates/persistence/**` — with one deliberate exception the spec names: the
`WHERE` clause of `close_conflict` and `close_conflicts_for_lease`
(`crates/persistence/src/store.rs:917,930`) is hardcoded to `WAITING`, so
releasing a violated lease would otherwise leave its row open forever. Read
the task file's §§6–8 before disputing the boundary.

### Lane O — orchestration · T300 → T305 → T310 → T320

Gate D and the actual product thesis. This lane is parallel to Lane A because of
a distinction the codebase already makes: `agent_session` is an agent Gritty
**observes** and did not start; an agent run is one Gritty **owns**. They are
different modules with different lifetimes, and keeping them that way is what
makes these two lanes disjoint. If a task in this lane finds itself editing
`agent_session`, the lane boundary was wrong and it stops rather than reaching.

```
crates/gritty-core/src/agent_run/**     (new)
crates/agent-fake/**                    (new)
crates/agents/adapters/**               (T310 — coordinate with Lane A first)
apps/desktop/src/features/runs/**       (new)
```

Reserved protocol minors: **2.20** (T300), **2.21** (T305), **2.22** (T310), **2.23** (T320).

### Lane D — diagnostics · T110

Coordination plan §35 already specifies the content: adapter health, lease
counts, event accept/reject counts, degraded providers. Perfectly disjoint, low
priority, and a good lane for a session that would otherwise idle.

```
crates/gritty-core/src/diagnostics/**   (new)
apps/desktop/src/features/diagnostics/** (new)
```

Reserved protocol minor: **2.24**.

### Lane H — human · Alexis only

The manual pass, ADR-001, the twelve reviews, and any decision that is an ADR.
No agent claims this lane and no agent may write ADR-001's verdict.

---

## Waves

```
Wave 0  ── serial, one session ────────────────────────────────────────────
        commit the lease system · merge #21 and #22 · rebase every branch

Wave 1  ── parallel with Lane H ───────────────────────────────────────────
        six spec agents, one task file each        │  Alexis: manual pass
        T070 T080 T100 T110 T300 T326              │  → ADR-001 verdict
                                                    │  → the twelve reviews

Wave 2  ── four lanes, once ADR-001 is recorded ───────────────────────────
        W: T070      G: T100      T: T097      A: T223

Wave 3  ── the same four lanes, next task ─────────────────────────────────
        W: T080      G: T400      T: —         A: T326    O: T300

Wave 4  ── Gate D ─────────────────────────────────────────────────────────
        O: T305 → T310 → T320    A: T222, T224    D: T110

Wave 5  ── enforcement and supervision ────────────────────────────────────
        T330 (ADR-018 amendment first)  T335  T405  T410
```

Wave 1 is the one to start now, because spec-writing is the most parallel work in
the project and the least blocked: six new files in `docs/tasks/`, one shared
magnet touched for a minute each, and no dependency on ADR-001. It matches how
T221–T224 and T230–T232 were written — one batch, ahead of implementation.

**Run three or four lanes at once, not six.** The binding constraint is not the
lease file or the machine; it is that one person reviews everything and twelve
tasks are already queued.

---

## Reservations

**`docs/merge-queue.md` owns version and migration claims**, and already records
`PROTOCOL_VERSION 2.10.0`, `SCHEMA_VERSION 3`, migration `0003_ownership.sql` and
`0004_agent_links.sql` for T223. Claim there, not here. The per-lane protocol
minors above are a hint for planning; the queue is the record.

The one thing worth repeating here, because a lane plans against it: a
`PROTOCOL_VERSION` gap is free, and a **migration gap is not** — migrations must
stay contiguous, so take the next free number immediately before opening the PR
and renumber if you lost the race. `docs/merge-queue.md` carries the reasoning
and the assertion that enforces it.

**New crates are pre-assigned** too, because two lanes inventing
`crates/git` differently is worse than a rebase: `gritty-search` (W),
`gritty-git` (G), `gritty-platform` (A/T224), `gritty-agent-fake` (O). The root
`Cargo.toml` says `members = ["apps/desktop/src-tauri", "crates/*"]`, so a new
crate directory joins the workspace without touching the magnet — only a new
entry in `[workspace.dependencies]` does, and that needs the one-line
justification the dependency policy asks for.

---

## Shared magnets

These belong to no lane. Claim, edit, commit, release — inside a minute, never
for the length of a task.

```
crates/gritty-core/src/protocol/{mod.rs,export.rs}
packages/protocol/src/generated/**
packages/protocol/src/{types.ts,client.ts}
apps/desktop/src-tauri/src/{commands.rs,core.rs,lib.rs}
apps/desktop/src/App.tsx
apps/desktop/src/app.css
crates/persistence/src/migrations.rs
Cargo.toml · pnpm-workspace.yaml · lockfiles
docs/tasks/README.md
```

`commands.rs` is 1040 lines and every lane adds to it. It is the single most
contended file in the repository, and the reason a lane should write its command
adapters in one commit at the end rather than growing them file by file.

Two more files belong to no lane and are not on the list above, which T070's
spec found the hard way: `crates/gritty-core/src/event/mod.rs`, where every
lane eventually adds a `CoreEvent` variant, and
`apps/desktop/src/features/documents/use-documents.ts`, which holds ADR-011's
own named watcher seam. Treat both as magnets — claimed for the minute, not
for the task — until a lane is given them deliberately.

### Why lanes should run in separate worktrees

`cargo test -p gritty-core` **regenerates `packages/protocol/src/generated/`** as
a side effect, and `check:generated` is literally
`test -z "$(git status --porcelain -- packages/protocol/src/generated)"`. Two
sessions running `pnpm check` in one checkout therefore write each other's
protocol types mid-run, and the check fails against a diff neither of them made.
Leases do not help: this is not an edit, it is a build.

One `git worktree` per active lane. `.claude/worktrees/` is where they go. The
lease file remains the backstop for the shared tree and for docs.

---

## Merge discipline

Learned the expensive way in `HANDOFF.md`, and it still applies:

- **Merge stacks top-down, or delete each base branch as it merges.** GitHub
  retargets a stacked PR only when its base is deleted; merging bottom-up lands
  each PR *in its parent branch* while `gh pr list` says `MERGED`.
- **Verify against `main`, never against the PR state:**
  `git rev-list --count origin/main..origin/<branch>` should be `0`.
- Rebase every open lane onto `main` after each merge, run
  `cargo test -p gritty-core`, and **commit the regenerated types** with it.
- One PR per task, named for it, completion report pasted in, status → `review`.
  The reviewer flips it to `done` — a lane never marks its own task done.

---

## The board

There is no table here. A status written into a planning document is stale by
the time a second session reads it, which is how this file, `roadmap.md` and
`HANDOFF.md` came to disagree about T221–T325.

```
scripts/agent-board.py list              # who holds what, right now
scripts/agent-board.py list --lane W     # one lane
```

The board is a live, cross-worktree record; its committed projection is the
generated status column of `docs/tasks/README.md`. The lane definitions above
are what this file owns — they change deliberately, not hourly — and
`agent-board.py` mirrors these scopes in a hardcoded `LANE_SCOPES` dict
(`scripts/agent-board.py:72`); it does **not** parse this file. The two copies
have already drifted — the script's Lane T omits ADR-017, its Lane A omits
`crates/gritty-core/src/ownership/**`, its Lane O omits
`crates/agents/adapters/**`, and Lane H does not exist there at all — so a
`take` that relies on the lane default can claim less than the task needs.
Pass `--scope` explicitly from the task file's "Allowed scope" when the two
disagree, and treat reconciling them as its own change.

---

## What this board deliberately does not do

- **It does not schedule T330.** Write interception is an amendment to ADR-018
  before it is code: `gritty-agent-hook` is a reporter with no opinion by
  construction (§5, §6, §7), and ownership stays *observed* until that changes.
  `docs/architecture/ownership.md` holds that line and this board does not move it.
- **It does not parallelise inside a lane.** Two agents on T070 and T080 at once
  would both be in `crates/workspace`.
- **It does not promise the waves are the right size.** They are the disjoint
  partition of work that exists today; a spec written in wave 1 may split a lane
  or merge two. Record that as a follow-up, do not widen a lane to absorb it.
