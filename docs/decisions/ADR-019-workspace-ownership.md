# ADR-019: Write ownership is a logical lease held by a workstream, and it is observed rather than enforced

- **Status:** Accepted (implemented by T325)
- **Date:** 2026-09-06

## Context

Gritty exists to supervise several coding agents working on one project at
once. That introduces a problem neither Git nor the editor solves: **two agents
modifying the same logical area at the same time, neither knowing about the
other**. Git notices at commit; the editor notices when a buffer goes stale;
both notice after the work is done.

`docs/plans/Gritty Agent Runtime Coordination.md` §§12–23 calls the answer a
Workspace Ownership Engine and argues it is an orchestration primitive rather
than a filesystem one. Nothing in the repository implemented it: a grep for
`lease` and `ownership` across `crates/` returned the Rust borrow-checker sense
of the word and variables named `release`.

The mechanism already existed here in another form. `scripts/agent-lease.py`
and `AGENT-LEASES.md` — file, directory and glob scopes, an eight-hour
staleness rule, a `PreToolUse` guard that fails open — were hand-built so that
the Claude sessions sharing this checkout stop overwriting each other, and
`AGENTS.md` hard rule 15 makes the convention binding. This ADR is that idea
made a domain object, with the differences named rather than assumed.

## Decision

**Ownership is a logical, persisted lease over a file, a directory subtree or a
glob, held by a workstream. Acquisition is atomic and either grants or refuses
with a conflict. Nothing here stops a write.**

### 1. Logical leases, not OS locks

Operating-system locks are the wrong primitive for every reason §13 lists:
agents do not take them, semantics differ per platform, a directory or a glob
cannot be locked at all, a lock is invisible in a UI, it carries no workstream
metadata, and a conflict needs an explicit resolution rather than a blocked
syscall. A lease is domain state, which is what makes it inspectable,
persistent, attributable and refusable.

### 2. The holder is a workstream, never an agent session

Hard rule 6 keeps provider concepts out of the domain, and
`docs/architecture/agent-sessions.md` already argues that a durable row must
not carry an id that exists only while a process does. So a lease belongs to a
`WorkstreamId`, cascading with it.

`OwnershipLease.agentRunId` is on the wire and in the schema, always absent:
nothing constructs an `AgentRunId` until Gritty owns runs (T300). This is the
same move T200 made with `Decision.raisedBy: Actor` — a seam paid for once, so
that adding runs later is a value rather than a shape change.

### 3. Acquisition is atomic, and a refusal is a value

The overlap check and the insert run inside one SQLite transaction. Two
requests racing for overlapping ground cannot both be granted, because the
loser's check has already seen the winner's row; a refusal rolls back and
writes only the conflict that explains it. This reuses the pattern T200 paid
for — a `CoreError` smuggled through `PersistenceError::Io` and downcast on the
way out — because `Ok(Err(..))` inside `Database::write` would commit.

`ownership.acquire` answers `GRANTED` or `CONFLICTED`. A refusal is not an
error the user dismisses: it is a persisted row naming who is in the way, which
is what the attention queue draws.

### 4. Where the matcher is unsure, it reports a collision

All three target kinds compile to one segment matcher (`**` crosses `/`, `*`
and `?` do not), so there is one routine to get right rather than nine pairwise
cases. Two globs whose intersection is not decidable cheaply — `*.ts` against
`*.rs` in the same position — are treated as overlapping.

**A false conflict costs a click; a missed one costs a file.** The bias is
stated here because it is the kind of thing a later "optimisation" would
quietly reverse.

The pattern language is deliberately the one `AGENT-LEASES.md` documents, so
the developer convention and the product cannot disagree about what a scope
means.

### 5. Path identity is `WorkspacePath`'s, including what it does not do

Ownership compares `WorkspacePath`s (ADR-010): workspace-relative,
`/`-separated, with `..`, backslashes and absolute forms refused on
deserialization. That is the repository's one definition of path identity —
tabs, snapshots and documents all key on it — and a second one here would
disagree with the rest of the product in ways nobody would predict.

It does no case folding and no Unicode normalization. On a case-insensitive
volume `src/Auth.ts` and `src/auth.ts` are one file on disk and two leases
here. That is a known limitation with a test pinning it, not an oversight.

### 6. Expiry and staleness are derived on read, never stored

A lease past its `expiresAt` reads as `EXPIRED`; a lease whose workstream is
`DONE` or `ABANDONED` reads as `stale`. Neither is written back. §21 asks for
startup reconciliation; deriving continuously *is* reconciliation, and it
cannot fall out of step with the clock the way a stored verdict can. The hook
reader (ADR-018) already treats phase freshness this way.

**Nothing auto-releases.** §21's "never silently retain ownership forever" is
satisfied by *reporting* a stale lease and offering a release, not by taking it
away — a lease that vanished on its own would be worse than one that is visibly
in the way.

### 7. Observed ownership, not enforced ownership

This is the constraint the rest of the ADR exists to protect. Gritty refuses a
second claim. **It does not stop a write.** An agent that edits an owned file
is not blocked, and §20 is explicit that filesystem observation must not be
presented as equivalent to pre-write interception. The UI says so in as many
words, next to the claims.

Two things would change that, and both are separate tasks:

- **T326, detecting a violation after the fact.** The timeline already reports
  the files an agent touched (T230); cross-checking them against the registry
  needs to know which workstream a session belongs to, which is T223's link.
- **T330, stopping the write before it happens.** `scripts/agent-lease.py`
  proves the mechanism exists: Claude Code's `PreToolUse` hook accepts a
  `permissionDecision: "deny"` reply. `gritty-agent-hook` deliberately cannot
  do this — ADR-018 §5 says the hook never blocks the agent, §6 says it knows
  nothing of the protocol and decides nothing, and §7 says hook state is
  "evidence, never authority". Enforcement is therefore an **amendment to
  ADR-018 first and code second**, and the ENFORCED/OBSERVED distinction must
  survive it.

### 8. An override is explicit, attributed, and does not transfer

Coding-agent rule 7 forbids overriding another workstream's ownership
*silently*, not at all. `ownership.resolve` with `OVERRIDE` releases the
holder's lease and records it; the event carries the `Actor` who did it. It
does **not** grant the ground to the requester, who asks again — a lease handed
over by an automatic re-acquisition is a silent override one step later.

## Consequences

- **A new namespace, `ownership.*`**, five commands and one event
  (`OwnershipChanged`, carrying the whole map). Protocol 2.7.0 → 2.10.0,
  additive. `2.8.0` and `2.9.0` are skipped: the unmerged
  `t230-agent-timeline-contract` branch has already spent them, and two
  branches claiming one version for different contracts is worse than a gap.
- **Schema 2 → 3**, migration `0003_ownership.sql`. T223's specification also
  claims `0003`; it is unstarted, so it moves to `0004`.
- **The attention queue gains a third kind.** T210 shipped three filters and
  its footer said "approvals, conflicts and failures arrive with agent runs and
  worktrees". Conflicts have arrived; the footer now claims only approvals and
  failures, which is still true.
- **The registry is bounded** at 200 leases and 200 returned conflicts per
  workspace. A map nobody can read is not coordination.
- **Nothing in the renderer matches paths.** Overlap is decided in the core for
  the same reason `Workstream.allowedTransitions` is published on the wire: a
  second copy would drift, and this one would drift towards "no conflict".

### The T330 renumbering

This work adopts the numbering in `docs/plans/Gritty Agent Runtime
Coordination.md` §36, which inserts T305/T315/T325/T335/T405 and **changes what
`T330` means**: write interception, with decision interruption moving to
`T335`. Live references in `AGENTS.md`, `docs/tasks/README.md`,
`docs/plans/roadmap.md`, `docs/architecture/workstreams.md` and
`crates/gritty-core/src/workstream/wire.rs` were retargeted.

Two places were deliberately **not** changed:

- **Completion reports**, including T200's "the only seam T330 needs". They
  record what was true when they were written; editing history to match a later
  decision is how a repository stops being trustworthy.
- **`0003_ownership.sql`'s neighbour, `0002_workstreams.sql`**, whose comment
  still reads "AGENT once runs exist (T330)". `migrations.rs` says *append
  only; never edit an applied one*, and that rule does not have a
  comments-are-fine exception.

`T315` — "live agent activity" in that document — is **already built**, by
T220, T230, T231 and T232. Its row says superseded rather than re-specifying
work that shipped.

That document was written green-field: it references none of T220–T232, the
rail, the island or ADR-018, and three of its type names collide with contracts
already frozen on the wire — its `AgentSession` is a run where the shipped one
is an observation, its `AgentActivity` is an operation kind where the shipped
one is a roster status, and its `agents.list` is narrower than the shipped one.
None of that touches this task, which adds only `ownership.*`. It is recorded
here so that whoever writes T300 and T305 meets it on paper rather than in a
merge.

## Alternatives rejected

**A filesystem lock, or a lock file per claimed path.** Nothing would take it.
Agents write with ordinary file APIs, and a lock an agent ignores is a claim
Gritty cannot honour but would appear to. It also cannot express a directory or
a glob, which is most of what a workstream actually owns.

**Ownership as a field on `Workstream`.** A list of paths on the workstream row
would be simpler to read and impossible to make atomic: two concurrent claims
would be two read-modify-writes of one JSON column, and the second would
silently win. It also has nowhere to put a conflict.

**Leasing to an agent session rather than a workstream.** It is what the
coordination document's `agentRunId` suggests, and it is wrong for sessions
Gritty does not own: a discovered session's id is in memory only and gone at
exit, so every lease would be stale on the next launch. The workstream is the
durable thing; the run attaches to it later.

**Granting the requester the ground when a conflict is overridden.** Fewer
clicks, and it turns a deliberate act into an implicit one. The requester's
second ask goes through the same check as the first, which also means it can be
refused by a *third* lease the user had forgotten about.

**Deciding overlap in TypeScript so the UI can grey out a doomed claim.** A
second matcher would drift from the first, and the direction it would drift —
towards saying "no conflict" — is the one that loses a file. The core answers,
and the answer is a row the user can read.

**A `mode: "read"` lease.** Two workstreams reading one file is not a
collision, and a read lease that nothing enforces and nothing refuses would be
decoration. `OwnershipMode` is an enum with one variant so that adding one
later is not a shape change.

## Revisit when

An agent Gritty supports offers a pre-write hook it may answer, at which point
§7's ENFORCED/OBSERVED distinction becomes real and ADR-018 needs the
amendment. Or when T400 lands worktrees: two workstreams in separate trees
cannot collide on the filesystem at all, so ownership becomes advisory within a
tree and the interesting conflict moves to merge time — a review conflict, not
a live one.
