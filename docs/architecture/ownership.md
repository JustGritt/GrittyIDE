# Workspace ownership

Status: fixed by T325 (ADR-019; coordination plan §§12–23).
`gritty_core::ownership` owns the domain; `crates/persistence` stores it in the
same database as workstreams and the session (T060);
`apps/desktop/src-tauri/src/commands.rs` exposes it as `ownership.*`.

Several coding agents working in one checkout have a problem Git does not
solve: two of them editing the same file at the same time, neither knowing.
Git notices at commit, the editor notices when a buffer goes stale, and both
notice after the work is done. This is the orchestration primitive that lets
Gritty notice first.

## What this is, and what it is not

```text
ENFORCED ownership          OBSERVED ownership          ← Gritty is here
a write is refused          a second claim is refused
before it happens           before the work starts
```

**Nothing here stops a write.** Gritty knows who claimed what and refuses a
second claim on the same ground; an agent that edits an owned file anyway is
not blocked, and the UI says so where the claims are. §20 of the coordination
plan is explicit that filesystem observation must not be presented as
equivalent to pre-write interception, and this document holds that line.

Two things would change it, and each is a separate task:

- **T326** would notice a violation *after* it happened, by cross-checking the
  files an agent touched (T230's timeline already reports them) against the
  registry. It needs to know which workstream a session belongs to, which is
  T223's link.
- **T330** would stop the write. Claude Code's `PreToolUse` hook accepts a
  `permissionDecision: "deny"` reply — `scripts/agent-lease.py` uses exactly
  that — but `gritty-agent-hook` deliberately cannot answer one: ADR-018 §5,
  §6 and §7 make it a reporter with no opinion. Enforcement is an amendment to
  ADR-018 first and code second.

## Why not an operating-system lock

Every reason from §13, kept because each one is a thing somebody will propose:

| | |
|---|---|
| agents do not take them | a lock an agent ignores is a claim Gritty cannot honour but appears to |
| semantics differ per platform | and Gritty is dogfooded on one of them (ADR-002) |
| a directory or a glob cannot be locked | which is most of what a workstream actually owns |
| a lock is invisible | ownership has to be readable, in a UI, by a person |
| a lock carries no metadata | a claim belongs to a piece of work, with a reason |
| a lock blocks a syscall | a conflict needs an explicit resolution, not a hung process |
| worktrees change the picture | two trees cannot collide at all (T400) |

## The model

A lease is held by a **workstream**, never by an agent session: hard rule 6
keeps provider concepts out of the domain, and a durable row must not carry an
id that exists only while a process does (the same argument
`agent-sessions.md` makes about session ids).

| Field | Notes |
|---|---|
| `id` | UUID v7, **persisted**: a lease outlives the process that granted it, which is what makes a stale lease possible |
| `workspaceId` | The workspace as it is open right now; rows are keyed by its canonical root |
| `workstreamId` | The holder. Cascades: removing the workstream removes its leases |
| `agentRunId` | Always absent. The seam T300 fills, on the wire now so adding runs is a value rather than a shape change — the move T200 made with `Decision.raisedBy` |
| `target` | `FILE`, `DIRECTORY` or `GLOB` (below) |
| `mode` | `WRITE` only. Two workstreams reading one file is not a collision |
| `note` | ≤ 200 characters, the holder's reason |
| `acquiredAt`, `expiresAt` | Epoch milliseconds; an absent deadline holds until released |
| `state` | `ACTIVE`, `RELEASED`, `EXPIRED` |
| `stale` | The holder is `DONE` or `ABANDONED` — derived, reported, never repaired |

A conflict records a refusal: who asked, who holds, which lease is in the way,
what was asked for, and `WAITING | RESOLVED | CANCELLED | OVERRIDDEN`. It is
persisted, so a blocker is still in the attention queue after a restart.

## Targets, and one matcher

All three kinds compile to one list of segments — a file is all literals, a
directory is its own segments followed by `**` — so there is one routine to get
right instead of nine pairwise cases.

```text
*   any run of characters inside one segment; never crosses '/'
**  zero or more whole segments
?   exactly one character inside one segment
```

That is deliberately the language `AGENT-LEASES.md` documents for the sessions
sharing this checkout, so the convention and the product cannot disagree about
what a scope means.

**Where the matcher is unsure, it reports a collision.** `src/auth/*.ts`
against `src/auth/*.rs` is treated as overlapping, because deciding it properly
is not cheap and the two directions cost differently: a false conflict costs a
click, a missed one costs a file.

Paths are `WorkspacePath`s (ADR-010) — workspace-relative, `/`-separated, with
`..`, backslashes and absolute forms refused on deserialization. That is the
repository's one definition of path identity, the same one tabs, snapshots and
documents key on, so ownership cannot disagree with the rest of the product
about which file is which.

**It does no case folding.** On a case-insensitive volume `src/Auth.ts` and
`src/auth.ts` are one file on disk and two leases here. A test pins that
behaviour; it is a known limitation, not an accident, and fixing it means
teaching `WorkspacePath` about the filesystem, not teaching ownership a second
notion of identity.

## Acquisition is atomic

```text
ownership.acquire
        │
        ▼  one SQLite transaction
  workstream exists, in this workspace?
        ▼
  already holding this exact ground?  ──▶ the same lease, no second row
        ▼
  overlaps an active lease of another workstream?
        │                    │
       yes                   no
        ▼                    ▼
  conflict row          lease row
  (reused if the        GRANTED
   same refusal
   was already made)
   CONFLICTED
```

The check and the insert share one transaction, so two requests racing for
overlapping ground cannot both be granted: the loser's check has already seen
the winner's row. A refusal rolls back and writes only the conflict. This is
T200's trick — a `CoreError` smuggled through `PersistenceError::Io` and
downcast on the way out, because `Ok(Err(..))` inside `Database::write` would
commit — and the concurrency test is the one §37 calls mandatory.

Overlapping your *own* ground is not a collision: nothing is protected from its
holder. Asking twice for the identical target returns the same lease rather
than a second row.

## Expiry and staleness are derived, not stored

A lease past `expiresAt` reads `EXPIRED`; a lease whose workstream is finished
reads `stale`. Neither is written back. §21 asks for reconciliation on startup;
deriving on every read *is* reconciliation, and it cannot fall out of step with
the clock the way a stored verdict can — the same reason the hook reader treats
phase freshness this way (ADR-018).

**Nothing auto-releases.** A stale lease still holds and still refuses others.
It is reported, with a release beside it. "Never silently retain ownership
forever" is satisfied by making it visible, not by taking it away — a lease
that vanished on its own would be the worse failure.

## Commands and events

| Command | Answers |
|---|---|
| `ownership.list` | the whole map — leases that still hold or have expired, and conflicts newest first |
| `ownership.acquire` | `GRANTED` **or** `CONFLICTED`, never both and never silently |
| `ownership.release` | releases; conflicts waiting on that lease become `RESOLVED` |
| `ownership.check` | every lease overlapping a target — *who owns this file* |
| `ownership.resolve` | `CANCEL` (the requester withdraws) or `OVERRIDE` (the holder's lease is released, on the record) |

One event, `OwnershipChanged`, carrying the whole map. The map is small and
bounded (200 leases, 200 conflicts), so a consumer that missed an event
converges on the next one instead of accumulating a divergent copy — the
opposite trade to `AgentTimelineAppended`, which is a delta precisely because a
transcript is not bounded.

Every command goes through `WorkspaceManager::require`, so another workspace's
leases read as `NotFound` rather than as forbidden.

**An override does not transfer the ground.** It releases the holder's lease
and says who did it; the requester asks again, and that second ask goes through
the same check — which may be refused by a third lease nobody remembered.
Coding-agent rule 7 forbids overriding *silently*, and an automatic grant would
be a silent override one step later.

## The UI

Ownership belongs to the workstream surface, not to the agent rail: a claim is
made on behalf of a piece of work.

- **The island** gains an Ownership section under the roster — every claim,
  who holds it, what state it is in, a release, and a form to claim a file,
  folder or pattern. It ends with the sentence that says this is observation
  and not protection.
- **A roster row** shows `owns 4` beside its open-question count.
- **The attention queue** gains a fourth filter. T210 shipped three and its
  footer read "approvals, conflicts and failures arrive with agent runs and
  worktrees"; conflicts have arrived, so the footer now claims only approvals
  and failures.

No glob matcher is written in TypeScript. Overlap is decided in the core for
the same reason `Workstream.allowedTransitions` is published on the wire: a
second copy would drift, and this one would drift towards saying "no conflict".

Everything a lease displays is a path somebody typed. It is rendered as text —
never a link, never a path anything opens, never a command anything runs.

## Storage

`0003_ownership.sql`, schema version 3. Two tables keyed by the canonical
workspace root like every other table there, in the **same** database and the
same migration chain as workstreams: one integrity check, one moved-aside path,
one version.

Everything cascades from the workstream. A conflict references both parties and
the lease it waits on, so removing either workstream removes the refusal — a
conflict between two pieces of work is meaningless once one of them is gone.

## Bounds and security

- 200 active leases per workspace, 200 conflicts returned. A map nobody can
  read is not coordination, and an unbounded one is a way to make it useless.
- Glob patterns: ≤ 240 characters, ≤ 32 segments. The segment matcher
  backtracks over `**`, and the segment cap is what bounds it.
- Notes: ≤ 200 characters, trimmed, blank means absent.
- Targets are untrusted input (rule 13). Paths are validated by
  `WorkspacePath` on deserialization; glob patterns get the same rules written
  out — no NUL, no backslash, no absolute or drive-letter form, no `.` or `..`
  segment, no empty segment.
- A lease grants nothing on the filesystem. It is a record of intent, and no
  code path turns one into an open file handle.

## Deliberately not here

- **Enforcement** (T330), and **violation detection** (T326). See the top of
  this document.
- **Ownership-aware scheduling** (T405): comparing a workstream's intended
  scope against the registry *before* an agent starts, and recommending a wait
  or a worktree.
- **Worktrees** (T400, ADR-007). Two workstreams in separate trees cannot
  collide on the filesystem at all; what they can still do is collide on merge,
  which is a review conflict and belongs to the ChangeSet model.
- **Symbol-level ownership.** §38 lists it as an initial non-goal and the
  reason holds: it needs a language server per language to mean anything.
- **Read leases.** `OwnershipMode` has one variant so adding one is not a shape
  change, but a read lease that nothing enforces and nothing refuses would be
  decoration.
- **Ownership across machines** (§38). Nothing here leaves the session
  database.
