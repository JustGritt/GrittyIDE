# Agent session discovery (T220)

How Gritty answers *what is happening* for the coding agents the user is
already running outside it, and — just as importantly — what it refuses to
claim about them.

This is **observability, not orchestration**. Gritty reads evidence the machine
already produces. It starts nothing, stops nothing, answers nothing and approves
nothing. Runs that Gritty *owns* are T300 and are a different type with a
different lifecycle; nothing here may be mistaken for one.

It writes to exactly one thing outside itself, and only when the user asks: one
hook entry in an agent's own configuration (T221, ADR-018), which makes the
agent *tell* Gritty when it stops at a permission prompt. That is still not
control — a hook is a notification the agent emits, not a channel into it — and
discovery is complete without it, which stays the default.

## The layers

```
  ps -Ao pid=,ppid=,tty=,command=        ~/.claude/projects/**/<session>.jsonl
            │  one call per scan                    │  fixed tail per file
            ▼                                       ▼
     lsof -a -p <pid> -Fn                     transcript fold
     cwd, open transcript                 task, model, branch, tool in flight
            └───────────────┬───────────────────────┘
                            ▼
                  gritty-agents::discover        ← adapter, no domain types
                            ▼
              agent_session::AgentSessionManager  ← domain, ranking, activity
                            ▼
              agents.list / agents.refresh
              AgentSessionsChanged (only on change)
```

`crates/agents` (`gritty-agents`) is the adapter. `crates/gritty-core/src/agent_session`
is the domain. Nothing the adapter returns crosses the protocol: the core
decides what an observation *means* — the activity values, the ranking, the
display path — which is the split ADR-005 asks for.

## Why two sources

Each is blind where the other sees, so neither is trusted alone.

| Source | Sees | Blind to |
|---|---|---|
| Process (`ps`, `lsof`) | that a process is alive, its terminal, its root | what it is doing |
| Transcript (`.jsonl`) | the task, model, branch, the tool in flight | whether it is still running |

A session found by only one source is still listed, and `evidence`
(`PROCESS_ONLY` / `TRANSCRIPT_ONLY` / `BOTH`) says which. An honest partial
answer beats a confident invented one.

The join key is the **working directory**, not the open transcript path.
Claude Code appends to its transcript and closes it, so `lsof` usually does not
name the file; when it does, the session id it yields is preferred.

## Activity, and what it deliberately cannot say

Derived from evidence alone, in this order:

| Value | Evidence |
|---|---|
| `ENDED` | no live process, or the process table was unreadable |
| `IDLE` | live process, transcript quiet past the idle threshold |
| `WORKING` | live process, a `tool_use` with no matching `tool_result` |
| `WAITING` | live process, nothing outstanding — the user's move |

(T221 adds `BLOCKED` between `IDLE` and `WORKING`; see below.)

A stale transcript outranks an outstanding tool call on purpose: a call with no
result for half an hour is a session someone walked away from, not one that is
working.

There is **no `WAITING_FOR_APPROVAL` from these two sources**. Poll-only
discovery cannot distinguish a pending permission prompt from a finished turn —
both are a live process with nothing outstanding — so claiming the distinction
from them would violate `AGENTS.md` rule 12. `BLOCKED` exists, and T221's hook
integration below is the only thing that can produce it.

Likewise, an unreadable process table is not an empty one. Everything reads
`ENDED`, and `AgentRoster.processEvidence` is `false` so the UI explains itself
instead of asserting the machine is quiet.

## Cost, and why polling is acceptable

There is nothing to subscribe to — a process table has no change feed — so the
cost is bounded instead:

- one `ps` per tick (~90 ms), one `lsof` per *agent* process (400 ms timeout);
- a **fixed tail** of each transcript, capped at 40 files within a 24-hour
  window. Reading a 400 MB transcript costs the same as a 4 KB one;
- the scan runs on a blocking thread, so no command handler or renderer ever
  waits on it (rules 8 and 10);
- an unchanged roster emits **nothing at all**.

Measured on a machine with three live sessions and thirteen recent
transcripts: ~300 ms per scan at a 6-second interval.

`stdout` is drained by a thread while the child runs. `ps -A` writes more than a
pipe buffer holds, and waiting first then reading afterwards deadlocks until the
timeout — which looks exactly like "no agents are running".

## Why the tail, not the whole file

Nearly every record repeats `sessionId`, `cwd`, `gitBranch` and `version`, and
the format carries records that answer the question directly rather than by
inference:

- `{"type":"ai-title","aiTitle":…}` — the agent's own name for the session,
  which is the closest thing to "the task" that exists;
- `{"type":"last-prompt","lastPrompt":…}` — the standing user request.

So the tail contains everything needed. The cost is the *first* prompt of a long
session, which `ai-title` covers better anyway. Last-active time comes from the
file's modification time, which needs no date parser and cannot disagree with
the filesystem ordering the scan already relies on.

A session with no readable transcript carries **no** last-active time. Stamping
it with the scan time would make an unchanging roster look like it changed on
every tick.

## Security

Everything read here is untrusted input (rule 13):

- prompts, titles, tool inputs and paths are bounded and whitespace-collapsed
  **in the core**, so an oversized field never crosses the protocol boundary;
- they are rendered as text — never as markup, never as a path anything opens,
  never as a command anything runs;
- no transcript content is persisted: the roster is in-memory and rediscovered
  each launch, so Gritty never claims on startup that a session is running which
  ended while it was closed;
- tracing logs counts, providers and durations — never prompt or tool content.

Only the executable name classifies a process. Matching anywhere in the command
line would report every shell that merely *mentions* an agent — and Claude Code
sources a snapshot from `~/.claude/` in each of its own shells, so a substring
rule reports the agent's children as more agents.

## Platform

`ps` and `lsof` are Unix. On other platforms the probe returns nothing, every
session reads as `ENDED`, and `processEvidence` is `false`. Transcript discovery
still works. Nothing claims cross-platform support that was not tested (rule 11).

## Adding a provider

One variant on `Provider`, one arm in `provider_of_command`, and a transcript
reader for that agent's format. Deliberately not a trait: one match arm is
cheaper to read than an abstraction with a single implementation.

**A second provider is not quite free, and T222 says why.** Claude Code repeats
`sessionId`, `cwd` and `gitBranch` on nearly every record, which is what makes
the tail sufficient. Codex writes its identity once, in a `session_meta` record
on the **first** line, so its reader needs a bounded *head* read as well as the
bounded tail. Both bounds are constant, so the O(1)-in-file-size property
survives — but a provider added without checking where its identity lives will
quietly lose it.

## The timeline (T230)

The scan above reads a *fixed tail* of each transcript to produce one roster
line. That is the right shape for "which agents are running" and the wrong one
for "what is this one doing", so `gritty-agents` carries a **follower** beside
the scanner: given a session and a byte offset, `follow` parses only the bytes
appended since, which makes a live feed cost the same on a 400 MB transcript as
on a 4 KB one.

```
   scan()    fixed tail of every transcript   → roster      (6 s loop)
   follow()  new bytes of the open one        → timeline    (fast loop, 400 ms)
```

Nothing about this needs the agent's cooperation. Claude Code's transcript
already carries the whole vocabulary: `type=assistant` with `text`, `thinking`
and `tool_use` content blocks, `type=user` with `tool_result`, `type=system` for
notices, `file-history-delta` for files touched, and `isSidechain` marking
subagent work. The entries are folded into a provider-neutral
`AgentTimelineEntry` in the core (hard rule 4), bounded there (rule 13), held in
a ring of 500 that drops from the oldest end and *reports* the loss — the
terminal's rule, not a new one — and delivered in coalesced batches, one per
tick per session, with **nothing at all** emitted while a transcript is
unchanged (rule 8).

Four things the follower refuses to do, each because the alternative lies:

- a **partial final line** is not consumed and not counted into the next offset.
  The agent is appending while we read, so half a record is re-read on the next
  tick rather than parsed into a truncated entry;
- a file **shorter than the offset** was truncated or replaced, so the follower
  restarts from a bounded tail and sets `restarted`. The ring is cleared and the
  UI is told its view reset, instead of two unrelated files being spliced
  together. The same flag covers a burst larger than one call may read (1 MiB),
  which is a gap in the feed by another name;
- a line that does not parse is **skipped**, and non-UTF-8 costs a glyph rather
  than the batch around it;
- an entry's time comes from the record's own `timestamp` or is **absent**.
  Stamping it with the read time would make a static timeline look like it was
  moving.

`file-history-delta.trackingPath` is relative to the agent's working directory
when the file is inside it and absolute when it is not. Both are resolved
lexically — no `stat` per entry on a loop that runs twice a second — against the
**open workspace root**: inside it the entry carries a `WorkspacePath` the UI
may open, outside it the entry carries a display string and no openable path
(ADR-010). Because that resolution is fixed at read time, an open timeline
**closes** when the workspace changes rather than quietly re-pointing itself.

**Two loops, deliberately.** Discovery stays at 6 s because `ps` and `lsof` are
its cost. The fast loop follows only the timelines that are open (at most eight;
the rail opens one), which is what makes 400 ms affordable. Measured in a
release build on a 400 MB transcript: **1.4–3.4 ms** for a cold open, **~25 µs**
for a tick that finds nothing appended, and **~55 µs** for one that finds a
turn's worth of records. A cold open of a 5 KB transcript costs ~85 µs, so the
cost tracks the 256 KiB window and not the file. A tick with nothing open does
no I/O at all.

T221 puts the hook state directory on this same loop, for the same reason: a
permission prompt reported six seconds late is not worth reporting.

## The third source (T221)

The `WAITING_FOR_APPROVAL` value this document used to refuse is earned by
asking the agent instead of guessing at it. Every agent CLI in scope already
fires a hook event precisely when it blocks on the user, so Gritty registers a
small binary on it:

```
agent process                         gritty (usually not running)
  │ PermissionRequest hook fires
  ▼
gritty-agent-hook  ──▶  <app-data>/agents/state/<session-id>.json
  (writes atomically, exits 0)   one small file, overwritten, never appended
                                          │
                                          ▼  next fast-loop tick (~400 ms)
                             merged as a third source, joined on session id
```

A **file** rather than a socket because Gritty is usually closed when a hook
fires, and a hook must never stall the agent. **Overwritten** rather than
appended because only the current phase is ever asked for, which removes
rotation, truncation-tolerant parsing and an age policy from the design.
**Atomic**, so the reader takes no lock and never sees half a file.

The hook binary is deliberately the smallest thing in the repository: it reads
one JSON payload on stdin, names a *phase* — `BLOCKED`, `ACTIVE`, `SETTLED`,
`ENDED` — writes one file, and **exits `0` on every path**, including a payload
that never arrives, a state directory it cannot write and a panic. A non-zero
exit from a hook is the agent's problem, and Gritty must never be the reason
another tool misbehaves. It links no `gritty-core`, knows nothing of the
protocol, and does not decide what any phase *means*: the core does, which is
the same adapter/domain split `ps` and the transcripts already go through.

It also does not record the tool's *input*. A `Bash` command line or an `Edit`
payload is exactly the kind of thing that carries a secret, and it is not needed:
the transcript already has the core's bounded preview of the same call.

**Read on the fast loop, not the discovery tick.** "Pending validation" arriving
six seconds after the agent stopped defeats the value. The scan is a directory
listing plus a stat and a few hundred bytes per file, bounded exactly as
transcripts are (64 files, 6 hours, 8 KiB each), and files past the cutoff are
swept. With nothing installed it is one failed `read_dir` and nothing else,
which is what keeps the roster identical to T220's for anyone who never opted
in. A tick whose hook state changed re-derives the roster from the **cached**
discovery — no second `ps`, no second `lsof` — and still publishes only when a
derived value actually differs.

The derivation gains one row, at a defined precedence:

| Value | Evidence | `activitySource` |
|---|---|---|
| `ENDED` | no live process, or the process table was unreadable | `PROCESS` |
| `IDLE` | live process, transcript quiet past the idle threshold | `TRANSCRIPT` |
| **`BLOCKED`** | live process, hook phase `BLOCKED`, not stale | `HOOK` |
| `WORKING` | live process, a `tool_use` with no matching `tool_result` | `TRANSCRIPT` |
| `WAITING` | live process, nothing outstanding | `PROCESS` |

`ENDED` outranks `BLOCKED` — an agent killed at a prompt is gone, not blocked —
and so does `IDLE`, for the reason already given above. A hook phase older than
the idle threshold is discarded outright, so a state file cannot hold a row at
`BLOCKED` forever even when its transcript is still moving. Hook state is
*evidence, not authority*: it joins onto a session the other two sources already
found, on **session id** (the one exact key; the process join falls back to the
working directory), and a state file matching nothing creates no row.

`BLOCKED` is only ever set from hook evidence. No transcript pattern, process
state or elapsed-time threshold may produce it, which satisfies rule 12 by
construction, and `activitySource` says which of the three sources produced a
verdict so a row is attributable rather than magic. `AgentEvidence` gains the
hook combinations (`PROCESS_AND_HOOK`, `TRANSCRIPT_AND_HOOK`, `ALL`) — but no
`HOOK_ONLY`, because that combination cannot occur.

**What the prompt is about.** The state file's bounded `tool` field becomes a
`PermissionPending` timeline entry, so a rail showing a blocked session names
*what* is waiting rather than only that something is. Its `detail` comes from
the transcript's outstanding tool call, and only when the two name the same
tool: they are describing one call, so pairing them is not a guess, while
pairing a different call's input to this prompt would be. The entry is held
**beside** the ring rather than in it — it is a state that goes away, and a ring
can only ever append, so an answered prompt would otherwise sit in the feed
forever claiming to be waiting. A batch is emitted when it changes even with no
records appended, and not otherwise.

### Writing into another tool's configuration

This is the only thing in the repository that edits a file Gritty does not own,
and ADR-018 is what constrains it. Nothing is written without an explicit user
action for that provider; the entry is **appended** to each event's existing
array and no other tool's entry is replaced, reordered or removed; the exact
change is shown before it is applied and the prior file is copied to a
timestamped backup beside it; and a configuration that does not parse — or whose
hook section has a shape the installer does not recognise — is **refused with
the parse error**, never rewritten, never repaired, never replaced from a
template. If the backup cannot be written the change is abandoned before the
original is touched.

Gritty's own entries are the ones whose command runs `gritty-agent-hook`, so
uninstall needs no marker comment and an entry left pointing at a binary that
moved is still recognisably Gritty's — which is what makes `BROKEN` reportable.
A broken entry breaks nothing for the user, because the agent tolerates a hook
that fails; the only symptom is that `BLOCKED` quietly stops appearing, and the
status line is what says why.

Key order survives because the file is parsed into an order-preserving JSON
model local to the installer, not `serde_json::Value` (which sorts) and not
`serde_json`'s `preserve_order` feature (which is workspace-wide, enlarges every
`Value` in the tree, and would push `CoreError` past clippy's `result_large_err`
threshold — a repository-wide representation change to append one line to one
file). Indentation is two spaces, which is what these files already use.

The state directory is under Gritty's own application data, never `/tmp`, and is
created owner-only. The `agents.integrationStatus`, `agents.integrationInstall`
and `agents.integrationUninstall` commands carry the command line and the events
it lands on — never the configuration's contents, which would put whatever else
lives in that file into an event payload for no gain.

## Specified but not built (T222, T232)

Everything above this line is implemented. Everything below is written down and
not yet built; nothing here may be described as behaviour Gritty has.

**The rail (T231, built; T232, not yet).** The roster and the timeline surface
in a third region of the shell, to the right of the editor, with its own
splitter: a list of sessions coloured by status, and beneath it the selected
session's feed and the files it touched. Clicking a touched file opens it in the
editor — the thing a terminal multiplexer structurally cannot do.

T231 shipped the region and the list: the third child of `shell__body`, a
right-anchored splitter driving `--agent-rail-width`, `agentRailWidth` and the
collapsed flag in the layout blob, one row per session in the core's ranking,
the five status labels below, and selection that opens exactly one timeline
follower (`⌥G` toggles the rail; collapsing it closes the follower, because an
invisible one is waste). The body under the list is a placeholder until T232
draws the feed and the files touched.

It is deliberately **not** a tab in the document strip. Agent sessions and open
files are different kinds of thing, and `SessionTab.path` is a required
`WorkspacePath` in a non-nullable column, so a non-document tab would cost a
wire change, a ts-rs regeneration and a migration. The rail's width lives in the
UI-owned layout blob and costs none of that.

The rail and the island read **one** `useAgents` controller, following T210's
rule that two surfaces over one controller cannot disagree; the island's agents
section reduces to a count and its most urgent row. The rail's five labels map
onto the wire as Working=`WORKING`, Pending validation=`BLOCKED`, Done=`WAITING`,
Idle=`IDLE`, Ended=`ENDED` — `WAITING` keeps its name because it means *the
user's move*, and renaming the wire for a word is not worth the ripple.

**The rest.** T222 adds Codex. T223 notifies on entering `BLOCKED`/`WAITING`
and links a session to a workstream — through a separate table, never a field
on `Workstream`, because the domain model takes no provider-specific fields
(rule 6) and a durable row must not carry an ephemeral session id. T224 turns
the pid and tty into a "reveal the terminal" action behind a new platform
crate.

**Still deliberately absent afterwards:** any control (prompting, answering,
approving, keystroke injection), any screen-scraping of another terminal, a
daemon that outlives the app, and a socket or plugin surface. Runs Gritty owns
are T300 and remain a different type with a different lifecycle.
