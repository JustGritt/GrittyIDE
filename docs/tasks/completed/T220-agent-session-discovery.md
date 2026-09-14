# Task T220 — Agent session discovery: what is running, and on what

- **Phase:** 3
- **Status:** done
- **Depends on:** T010 (protocol), T020 (workspace), T210 (island)

## Objective

Answer the first question in `AGENTS.md`'s list — *what is happening* — for the
coding agents the user is already running outside Gritty. Discover local agent
CLI sessions from evidence on the machine (running processes and the transcripts
those processes write), translate them into a Gritty domain type, and list them
with the task each one is working on.

This is observability, not orchestration. Gritty reads; it never starts, stops,
answers or approves anything here. That distinction is what lets this land
before T300: nothing in this task defines a run lifecycle, a provider adapter
contract, a capability model, or any state an agent could mutate.

## Dependencies

- T010 for the error envelope, the event envelope, the operation model and
  `PROTOCOL_VERSION`.
- T020 for `WorkspaceManager` — a discovered session is ranked against the open
  workspace root, though the roster is machine-wide.
- T210 for the island, which is where the roster surfaces.

## Required reading

- `docs/decisions/ADR-005-agent-isolation.md` — agents are external processes
  behind provider-neutral adapters; raw transcripts are secondary
- `docs/architecture/overview.md`, `docs/architecture/security.md`
- `docs/architecture/workstreams.md` — the domain this must *not* pollute
- `crates/gritty-core/src/terminal/mod.rs` — the manager shape to copy for a
  non-database manager that owns a background loop and emits batched events
- `crates/terminal/src/system.rs` — how a platform adapter is isolated
- `apps/desktop/src/features/workstreams/` — the hook + pure reducer shape

## Allowed scope

- `crates/agents/**` (new crate, `gritty-agents`)
- `crates/gritty-core/src/agent_session/**` (new)
- `crates/gritty-core/src/{lib.rs,event/mod.rs,session/tap.rs}`,
  `crates/gritty-core/src/protocol/{mod.rs,export.rs}`
- `crates/gritty-core/src/workspace/recents.rs` — visibility only, to reuse its
  path-display helpers rather than duplicate them
- `crates/gritty-core/Cargo.toml`, root `Cargo.toml`
- `apps/desktop/src-tauri/src/{commands.rs,core.rs,lib.rs}`
- `packages/protocol/src/{client.ts,commands.ts,types.ts,index.ts}` + tests
- `apps/desktop/src/features/agents/**` (new),
  `apps/desktop/src/features/workstreams/island.tsx`
- `docs/architecture/agent-sessions.md`, `docs/tasks/README.md`, this file

## Explicit non-goals

- **No hook installation.** Gritty does not write to `~/.claude/settings.json`,
  `~/.codex/config.toml` or any other tool's configuration. Discovery is
  read-only and requires no cooperation from the agent. A hook bridge is a
  separate decision with a separate task (see follow-ups).
- **No control.** No starting, cancelling, answering, approving or writing to a
  discovered session. No keystroke injection, no terminal focus stealing.
- **No run lifecycle, capability model or provider adapter contract** — T300
  owns those, and must not be pre-empted by this task's shapes. The domain type
  here describes an *observation*, not a run.
- **No Workstream coupling.** A discovered session is not a workstream and does
  not create, transition or block one. Linking the two is follow-up work.
- **No transcript content surfaced beyond a bounded preview.** Gritty does not
  become a transcript viewer.
- **No cross-provider expansion beyond Claude Code** in this task. The probe is
  written so a second provider is an added match arm, but only Claude Code is
  implemented and tested.

## Required behavior

1. `agents.list` returns every discovered session, newest activity first, with
   sessions rooted inside the open workspace ranked ahead of the rest.
2. Discovery combines two independent sources and states which it used:
   - **process evidence** — the running agent processes, with pid, tty and
     working directory;
   - **transcript evidence** — the session files the agent writes, with the
     task, the model, and the tool call in flight.
   A session found by only one source is still listed, with `evidence` saying so.
3. Each session carries: a stable id, the provider, the working directory, a
   `title` (the agent's own session title when it publishes one), the last
   prompt, the tool in flight, the model, the git branch, an `activity`, and
   when it was last active.
4. `activity` is derived from evidence only, and each value has exactly one
   meaning: `WORKING` (a tool call is outstanding), `WAITING` (a live process
   with no outstanding tool call — the user's move), `IDLE` (a live process
   whose transcript has been quiet past the threshold), `ENDED` (no live
   process). Nothing infers a permission prompt: poll-only discovery cannot see
   one, and claiming otherwise would violate rule 12.
5. The core polls on an interval and emits `AgentSessionsChanged` **only when
   the roster actually differs** from the last snapshot (rule 8). One event
   carries the whole roster; it is small and a consumer that missed one still
   converges.
6. `agents.refresh` forces a scan out of band and returns the fresh roster.
7. Transcript reads are bounded: a fixed tail per file, a cap on files scanned,
   and an age cutoff. Reading a session file must cost the same whether it is
   4 KB or 400 MB.
8. Discovery never blocks a command handler or the renderer: the scan runs on a
   blocking-safe thread, and the poll loop stops on shutdown.
9. Protocol version bumps MINOR (additive).

## Failure behavior

- **No agent tooling installed** (`~/.claude/projects` absent, no processes) →
  an empty roster and no error. This is the normal state for most users.
- **`ps`/`lsof` missing, failing, or slow** → per-command timeouts; the scan
  degrades to transcript-only evidence and records a warning rather than
  failing. `lsof` is expected to fail for processes owned by another user.
- **Unreadable, truncated, non-UTF-8 or partially written transcript** → that
  file is skipped; every other session still lists. A line that does not parse
  is skipped, not fatal: the agent is appending while we read.
- **Transcript with no recoverable session id or working directory** → skipped
  rather than listed under a guess.
- **Non-Unix platform** → the process probe is a stub returning no evidence;
  transcript discovery still works. The roster degrades, it does not error, and
  nothing claims cross-platform support that was not tested (rule 11).
- **Shutdown** → the poll loop is cancelled and joined; no scan outlives the app.

## Security

- Everything read here is untrusted input (rule 13). Prompts, titles, tool
  inputs and paths are treated as data: bounded in length, never interpreted,
  never logged in full, and rendered as text by the UI.
- Previews are truncated in the core, not the renderer, so an oversized field
  never crosses the protocol boundary.
- No transcript content is persisted by Gritty; the roster is in-memory only.
- Tracing logs counts, providers and durations — never prompt or tool content.

## Acceptance criteria

- With Claude Code running in another terminal, the island lists that session,
  names its task and shows the tool in flight; quitting it flips the session to
  `ENDED` within one poll interval.
- With no agent running and no transcripts, the roster is empty and no error is
  raised anywhere in the UI.
- A 400 MB transcript is scanned in the same bounded time as a small one, with
  no memory spike.
- No `AgentSessionsChanged` event is emitted while the roster is unchanged.
- `pnpm check` is green; generated protocol types are fresh and committed.

## Tests required

- Unit (Rust): transcript parsing against fixture jsonl — task title, last
  prompt, model, branch, outstanding tool call, tool call closed by its result,
  malformed line skipped, truncated final line tolerated, non-UTF-8 tolerated.
- Unit (Rust): activity derivation for each of the four values.
- Unit (Rust): the merge — process-only, transcript-only, and both; dedup by
  session id; ranking against the open workspace root.
- Unit (Rust): process probe parsing against captured `ps`/`lsof` output through
  an injected command runner, so no test spawns a subprocess.
- Unit (Rust): the roster diff emits nothing when unchanged.
- Unit (TS): the client method and the UI reducer.
- Manual: run Claude Code in a second terminal and watch the island.

## Commands to run

```
cargo test -p gritty-agents
cargo test -p gritty-core
pnpm check
```

## Completion report

### Summary

Gritty now lists the coding-agent sessions already running on the machine, with
the task each one is working on, in the island. Two independent sources are
merged — the process table and the agents' own transcripts — and the roster is
published on a bounded poll that emits only when it actually changes.

Nothing here acts on a session. The pid and tty are shown so the user can go and
find the terminal themselves; that is the whole interaction model, and it is
what keeps this task clear of T300.

### Files changed

New crate `crates/agents` (`gritty-agents`), the adapter:
- `src/probe.rs` — `ps` / `lsof` behind an injected `CommandRunner`
- `src/transcript.rs` — bounded tail-read and fold of Claude Code `.jsonl`
- `src/lib.rs` — the merge, `Evidence`, `Discovery`
- `Cargo.toml` — **no new third-party dependencies** (serde_json, tracing only)

Core domain:
- `crates/gritty-core/src/agent_session/{mod,wire,tests}.rs` — manager, poll
  loop, activity derivation, ranking, wire types
- `crates/gritty-core/src/{lib.rs,event/mod.rs,session/tap.rs}`,
  `protocol/{mod.rs,export.rs}` — module, `AgentSessionsChanged`, version 2.6.0
- `crates/gritty-core/src/workspace/recents.rs` — `display_name` / `shorten`
  made `pub(crate)` (visibility only)
- `crates/gritty-core/Cargo.toml`, root `Cargo.toml`

Shell and protocol:
- `apps/desktop/src-tauri/src/{core.rs,commands.rs,lib.rs}` — manager wiring,
  `agents.list` / `agents.refresh`, shutdown
- `packages/protocol/src/{client.ts,commands.ts,types.ts}` + `agents-client.test.ts`
- `packages/protocol/src/generated/Agent*.ts` (generated, must be committed)

UI:
- `apps/desktop/src/features/agents/{agents-reducer.ts,agents-reducer.test.ts,use-agents.ts,agents-panel.tsx}`
- `apps/desktop/src/features/workstreams/island.tsx`, `App.tsx`, `app.css`

Docs: `docs/architecture/agent-sessions.md`, `docs/tasks/README.md`, this file.

### Tests actually executed, with results

| Command | Result |
|---|---|
| `cargo test -p gritty-agents` | **35 passed** |
| `cargo test -p gritty-core` | **177 passed** (14 new) + 3 crash-recovery |
| `cargo test --workspace` | **passed** |
| `cargo clippy --workspace --all-targets -- -D warnings` | **clean** |
| `cargo fmt --all --check` | **clean** |
| `pnpm typecheck` | **passed** |
| `pnpm test` (vitest) | **363 passed** in 53 files (12 new) |
| `pnpm check` | **fails on three pre-existing Biome errors in files this task did not touch** — see limitations |

Manual, against real machine state (three live Claude Code sessions):
- the adapter reported all three with correct pid, tty, cwd, branch and model;
- the core roster reported `Working` (with the tool in flight), `Waiting` and
  `Idle` correctly, plus ten `Ended` sessions from transcripts;
- one scan took ~300 ms; a second scan emitted **no** event.

### Architectural decisions

1. **Adapter/domain split.** `gritty-agents` reports evidence; `gritty-core`
   decides what it means. No adapter type crosses the protocol (ADR-005).
2. **Tail-read, not whole-file.** The reference implementation streams entire
   transcripts and carries a comment about multi-GB peaks and OOMs. A fixed
   128 KB tail makes a 400 MB transcript cost the same as a 4 KB one, and
   Claude Code's `ai-title` / `last-prompt` records mean nothing is lost.
3. **File mtime for last-active**, not the records' ISO timestamps: same
   question, no date parser, cannot disagree with the scan's own ordering.
4. **A session with no transcript carries no last-active time.** Stamping it
   with the scan time made an unchanging roster emit an event every tick — this
   was caught by a test and is why the field is optional.
5. **No `WAITING_FOR_APPROVAL`.** Poll-only discovery cannot distinguish a
   permission prompt from a finished turn. Claiming it would violate rule 12.
6. **An unreadable process table is not an empty one.** `processEvidence: false`
   travels to the UI so it explains itself instead of asserting all is quiet.
7. **Only the executable name classifies a process.** Claude Code sources a
   snapshot from `~/.claude/` in each of its own shells, so a substring match
   over the command line reports the agent's children as more agents.
8. **Machine-wide roster, workspace-first ranking.** Sorting is
   (this workspace, then blocked-on-you, then running, then recency).

### Known limitations

- **macOS/Unix only** for process evidence. On other platforms every session
  reads `ENDED` with `processEvidence: false`; transcript discovery still works.
  Not tested on Linux (no machine available) or Windows.
- **Claude Code only.** Codex, Cursor, Gemini and the rest are a match arm and a
  transcript reader away, but none is implemented or tested.
- **No permission-prompt visibility**, by construction — see decision 5.
- **A ~6-second worst-case lag** between a session changing and the island
  showing it, which is the poll interval.
- **The UI has not been visually verified.** The core path was exercised against
  real machine state and every layer is unit-tested, but the island's rendering
  of the section has not been seen in the running app.
- **`pnpm check` is not green on this working tree**, for reasons outside this
  task: `apps/desktop/src/features/terminal/terminal-pill.tsx` (untracked) and
  `apps/desktop/src/features/editor/tab-strip.tsx` (modified) carry three Biome
  errors from concurrent in-progress work. Every check this task's own files are
  subject to passes. `check:generated` also reports the new
  `packages/protocol/src/generated/Agent*.ts` as untracked until they are
  committed, which is the intended workflow.

### Follow-up tasks

- **Hook bridge (needs an ADR).** Installing a hook CLI into
  `~/.claude/settings.json` would give real-time phase changes and, more
  importantly, actual permission-prompt visibility and approval brokering. It
  writes into the user's global agent configuration, so it is a deliberate
  decision, not an increment of this task.
- **More providers**: Codex (`~/.codex/sessions`), Cursor, Gemini CLI.
- **Link a session to a workstream**, so an observed agent can be attached to
  the work it belongs to (T200 domain, T300 semantics).
- **Jump to the terminal.** The tty and tmux target are discoverable; focusing
  the right window needs the platform abstraction and a capability decision.
- **Subagents** are currently dropped (their worktrees are skipped). They could
  be shown as metadata on the parent session.
- **`AgentSessionsChanged` carries the whole roster.** If the roster ever grows
  past a few dozen sessions, this should become a delta.

### Remaining risks

- `lsof` is the slowest part of a scan and its cost grows with the number of
  agent processes, not with the machine's total. A user running many agents at
  once would want the interval raised; it is a constant, not yet a setting.
- Claude Code's transcript format is not a published contract. The parser
  tolerates unknown records and malformed lines, and every field is optional, so
  a format change degrades the roster rather than breaking it — but a rename of
  `ai-title` or `last-prompt` would silently cost the task line.
