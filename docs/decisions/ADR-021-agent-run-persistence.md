# ADR-021: A run Gritty owns is persisted as an account of itself, never as something to resume

- **Status:** Accepted
- **Date:** 2026-09-08
- **Accepted:** 2026-09-14 by Alexis, as drafted. Nothing implements it yet:
  `SCHEMA_VERSION` is still 4 and §8's migration `0005` and version 5 are still
  named rather than claimed, so the ticket that writes the migration claims them
  in `docs/merge-queue.md` in the ordinary way. Checked against the tree that
  day, after T310-ANSWER: the facts in *Context* still hold, and a run
  interrupted at shutdown with an ask outstanding has that ask denied first,
  which §2's write-back does not need to know about.
- **Motivated by:** T300-C, T300-CORE and T300-CMD, which each closed with the
  same known limitation in almost the same words, and T310, which turns it from
  a correct simplification into a real gap
- **Related:** ADR-004 (SQLite with explicit migrations), ADR-014 (the session
  database, its reconciliation and its failure policy), ADR-005 (agent
  isolation), ADR-010 (which paths may be opened)

## Context

An `AgentRun` lives entirely in memory. `session/tap.rs` turns document,
workspace and durable-operation events into persistence intents and turns
neither run event into anything; there is no `agent_run` table and no migration.

Three consecutive tickets recorded that as a limitation and all three argued it
was **correct rather than merely tolerable**, in the same terms: a row claiming
on startup that a run is `RUNNING`, when the process driving it died with the
app, is precisely the orphan the ten-state machine exists to prevent. Failing
closed is the right direction, and for a scripted fake there is nothing else to
lose. All three also said the same thing about when that stops being true:

> It becomes a real gap the moment T310 gives a run something worth losing — a
> real provider's process, its transcript, half an hour of work — and ADR-021
> is where that is decided.

T310 is that moment. Four facts about the shape of the problem, all of them
established rather than assumed:

1. **A clean quit leaves no orphan.** `Core::shutdown` ends every live run
   `INTERRUPTED` and kills every child before the session writer flushes, and it
   does so in that order deliberately, because those endings are events the
   writer still has to see. So "reattach after a normal restart" is not a case
   that exists.
2. **A crash may leave a child alive** — and that child is then a process Gritty
   does not supervise, holds no pipe to, and cannot cancel. That is the exact
   definition of the *observed* lane. It is not a hole in the run lane; it is
   the lane split working.
3. **The precedent is already in the tree.** `operation_record` is upserted on
   every durable operation's move, and `SessionStore::open` calls
   `operations::mark_interrupted` before the window shows, raising a
   `SessionNotice` that says how many operations from the previous process were
   interrupted. A run is the same shape of thing with a bigger tail.
4. **Memory is bounded and the database is not.** `MAX_RUN_EVENTS` = 512 per
   run and `FINISHED_RETENTION` = 50 runs bound what a session holds.
   `operation_record` has no retention at all — nothing prunes it, ever — and a
   run's log is two orders of magnitude larger per row. Inheriting that gap
   would be choosing it.

What is actually lost today, then, is not a resumable process. It is the
**account**: that a run happened, on which workstream, what it touched, what it
was told to do, and how it ended. That is what this decides.

## Decision

### 1. One row per run, written from the start and upserted on every move

An `agent_run` row is created when the run is registered, in state `STARTING`,
and upserted on every state or activity change — the shape `operation_record`
already has, for the same reason: a crash must not be able to lose the fact that
a run existed. The row carries the run's id, its workstream, the canonical
workspace root, the provider, the state, the timestamps, the correlation id and
the terminal error's code and message.

It is keyed by the canonical workspace root, like everything else in this
database (ADR-014). `WorkspaceId` and `AgentRunId` are per-process identifiers
(ADR-009); the run's id is stored as the primary key because it is the only
thing an event log can point at, and it is never presented as meaningful across
processes.

### 2. Nothing is ever restored as live

`SessionStore::open` rewrites every non-terminal `agent_run` row to
`INTERRUPTED` before the window shows, in the same transaction as the existing
durable-operation reconciliation, and raises a `SessionNotice` of a new kind
saying how many. `INTERRUPTED` is the honest word and it is already the word:
"Gritty stopped being able to watch it" is a different answer from "it broke"
and from "someone asked it to stop", and a user deciding whether to try again
needs to know which.

A restored run is therefore always terminal, always. There is no code path that
puts a persisted run back into the registry as something a driver could still
be attached to.

### 3. Gritty never re-adopts a child

A `claude` that outlived a crash is not a run any more. Gritty holds no stdin to
write to it, no stdout to read from it and no way to cancel it, and a lane whose
whole promise is *start, cancel, and account for it* must not claim a process it
can do none of those to.

It belongs to the **observed roster**, which is the lane for processes someone
else owns, and it will appear there on the next scan without anything being
built: T310-B's exclusion set is derived from live registry entries, so a run
that no longer exists stops excluding its child and the child correctly
reappears as a session.

**This is not a limitation to be lifted later. It is the thing this ADR
refuses.** Taking over an existing conversation is a different operation with
different rules — a new run, started with the provider's own resume flag against
a session id, deciding for itself what to do about a session that may still be
live elsewhere. That is `T310-RESUME`, and it is a *start*, not a *restore*.

### 4. The event ring is not persisted; a bounded, typed summary is

This is the substantive call, and it goes against the obvious move.

The ring is up to 512 records per run and up to 50 runs per session, growing
without limit across sessions. Every record in it is already bounded and
whitespace-collapsed by `AgentRunEvent::bounded`, so the risk is not malformed
text — it is *durable* text: a `PROMPT_FINISHED` quotes whatever the agent read,
and an agent reads whatever it is pointed at. The database already holds full
document text in `buffer_snapshot`, so this is not a new class of risk; but a
snapshot has explicit removal rules and a log has none, which is the difference
that matters.

Against that, what a person actually asks a week later is: *what did it do, what
did it touch, why did it stop.* That is answerable from a summary. So what is
written when a run reaches a terminal state is:

- **counts by event kind** — enough to say "it wrote four files and ran nine
  commands" without keeping what was in them;
- **the distinct paths it touched**, and **only those inside the workspace**.
  `AgentRunPath.path` is `Some` exactly when the target is in the workspace
  (ADR-010); an outside path is display-only and is **not written at all**,
  which also drops most of the ways an absolute path becomes a disclosure;
- **the commands it started**, already bounded at `MAX_COMMAND`;
- **the first prompt only**, clipped to `MAX_TEXT`. It is what the user typed to
  begin the run and it is the run's title in any list of runs. Every later turn
  and every agent reply is the *conversation*, and the conversation already has
  a durable home;
- the terminal error, which the row carries anyway.

The conversation's durable home is the provider's own transcript. T310 §6
decided the *live* feed comes from the run's ring rather than that transcript,
because a supervisor that renders a file instead of the process it supervises
can disagree with itself. Once a run is terminal there is no process, no
disagreement is possible, and the transcript is the better record precisely
because it is richer. **The live feed is the ring; the durable record is a
summary, and the detail is where the provider already keeps it.**

### 5. The provider session id is persisted; a path derived from it is not

The run row carries the provider's session id — the string Gritty minted and
passed to the child. It is the single highest-value durable field: it is what a
resume needs, and it is the key a transcript is found by.

A transcript **path** is not stored. Gritty would have to derive it from the
provider's directory convention, and a derived path written into a database goes
stale the moment that convention changes, while the id it was derived from does
not. `crates/agents` already locates a transcript from a session id for observed
sessions; that is where the knowledge belongs and it stays there.

**The persisted session id does not feed the observed-roster exclusion set.**
This is the one place the two lanes could quietly be rejoined and it is
forbidden: a set that survived a restart would be a permanent blacklist, and the
day the user runs the provider's own resume against that id in their own
terminal, a real observed session would be invisible. Exclusion follows live
supervision and nothing else. The visible consequence — after a restart, a
finished owned run's transcript is inside the observed scan's window and appears
as a transcript-only session — is **correct**: Gritty no longer owns anything
there.

### 6. Retention is decided here, not deferred

At most 200 ended runs per workspace root, oldest first, swept in the same
`SessionStore::open` transaction as the reconciliation in §2. Two hundred
matches the operation registry's own in-memory ceiling and is affordable
precisely because §4 made a run's row small; the in-memory `FINISHED_RETENTION`
of 50 is unchanged and independent, because it bounds something else (the rings)
for a different reason.

`session.discard` removes a workspace's runs along with its snapshots. Nothing
else removes them: closing a workspace does not, because the account of what ran
there is what a user comes back for.

### 7. Persistence never fails a run

A write that fails is logged and surfaced as a notice, and it changes no run's
state, refuses no `runs.start`, and blocks no `runs.prompt`. This is ADR-014's
rule for `session.restore` — *it never rejects for persistence reasons* — applied
to the lane where the cost of getting it wrong is a real process that did not
start because a database was busy.

Every write goes through the existing tap-and-writer path on the blocking pool.
No command handler and no driver touches SQLite.

### 8. Schema

Migration `0005_agent_runs.sql`, `SCHEMA_VERSION` **5**. Both are free, and
`docs/merge-queue.md` records that this ADR **names** them without claiming
them: an ADR at `Proposed` has not been decided, so it must not hold a number a
decided ticket may need first. Whoever writes the migration claims it there in
the ordinary way, and if that turns out to be a different ticket, this section
follows the number rather than the other way round.

## Consequences

- A crash loses the conversation and keeps the account of it, plus the one
  string a deliberate resume would need.
- A run's durable record is deliberately smaller than its live one. Anyone who
  wants the full detail reads the provider's transcript, which is where it
  already is — and Gritty does not duplicate a file it does not own.
- `SessionStore::open` does one more sweep before the window shows. It is
  already synchronous and already does exactly this for operations, so the shape
  is not new; the startup cost is one indexed `UPDATE` and one `DELETE`.
- **`operation_record`'s unbounded growth becomes visibly inconsistent** with an
  `agent_run` table that sweeps. That is a real gap in the older code and this
  ADR does not fix it; it is filed rather than absorbed.
- A restored run can never be cancelled, resumed or answered, because it is
  terminal by construction. Every `runs.*` command that takes a live run
  continues to answer `Conflict` naming the state, with no new case to handle.
- Two things become possible that are not built here: a run history surface
  (`T300-PANEL`'s question of where a run belongs), and `T310-RESUME`.

## What this deliberately does not decide

- **When a run may be resumed.** Persisting a session id makes `T310-RESUME`
  possible; it does not decide its rules, and the hard one — what to do about a
  session that is still live in someone else's terminal — is untouched here.
- **Where a run's history is shown.** `runs.list` is scoped to a workspace
  because that is the boundary a run is cancelled at; a durable history is a
  different query with a different surface, and inventing one here would be
  taking `T300-PANEL`'s scope.
- **Whether the summary is enough.** If it is not, the honest fix is to say so
  and amend this, not to quietly start writing the ring.
- **Persisting anything for an *observed* session.** Different lane, different
  lifetime, and nothing about this reaches it.

## Revisit when

A provider ships without an on-disk transcript, so §4's summary loses the
escape hatch that justifies it and the ring may have to be persisted after all;
runs become long enough that "started and never finished" is an ordinary
outcome rather than a crash artefact, which would make §2's write-back the
common path instead of the rare one; T400's worktrees make the canonical
workspace root a poor key; or a run's durable record is wanted on another
machine, at which point "per-process ids, never presented as meaningful across
processes" stops being sufficient.
