# Agent session discovery (T220)

How Gritty answers *what is happening* for the coding agents the user is
already running outside it, and — just as importantly — what it refuses to
claim about them.

This is **observability, not orchestration**. Gritty reads evidence the machine
already produces. It starts nothing, stops nothing, answers nothing, approves
nothing, and writes to no agent's configuration. Runs that Gritty *owns* are
T300 and are a different type with a different lifecycle; nothing here may be
mistaken for one.

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

A stale transcript outranks an outstanding tool call on purpose: a call with no
result for half an hour is a session someone walked away from, not one that is
working.

There is **no `WAITING_FOR_APPROVAL`**. Poll-only discovery cannot distinguish a
pending permission prompt from a finished turn — both are a live process with
nothing outstanding. Only a hook integration could, and claiming the
distinction without one would violate `AGENTS.md` rule 12.

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
